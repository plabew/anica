use std::collections::HashMap;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, atomic::AtomicBool, mpsc};
use std::time::Duration;

use gpui::{
    ClipboardItem, Context, Entity, Focusable, MouseButton, Render, SharedString, Subscription,
    Timer, Window, div, prelude::*, px, rgb,
};
use gpui_component::{
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    text::TextView,
    white,
};
use motionloom::{GraphScope, compile_runtime_program, is_graph_script, parse_graph_script};
use regex::Regex;
use serde_json::Value;

use crate::api::export::{
    AcpExportRunRequest, AcpExportRunResponse, resolve_acp_export_run_request,
};
use crate::api::llm::llm_decision_making_srt_similar_serach;
use crate::api::media_pool::{clear_media_pool, remove_media_pool_by_id};
use crate::api::motionloom::{
    AcpMotionLoomGetSceneScriptResponse, AcpMotionLoomRenderSceneResponse,
    AcpMotionLoomSetSceneScriptResponse,
};
use crate::api::timeline::{
    apply_edit_plan, build_audio_silence_cut_plan, build_autonomous_edit_plan,
    build_subtitle_gap_cut_plan, build_transcript_low_confidence_cut_plan, get_audio_silence_map,
    get_subtitle_gap_map, get_subtitle_semantic_repeats, get_timeline_snapshot,
    get_transcript_low_confidence_map, validate_edit_plan,
};
use crate::api::transport_acp::{AcpToolBridgeRequest, AcpUiEvent, AcpWorker};
use crate::core::export::{FfmpegExporter, is_cancelled_export_error};
use crate::core::global_state::{
    AiChatMessage, AiChatRole, AppPage, GlobalState, MediaPoolItem, MediaPoolUiEvent,
    SilencePreviewCandidate, SilencePreviewModalState,
};
use crate::core::user_settings::{
    SettingsScope, load_settings, resolve_workspace_root, save_acp_cli_paths, save_auto_connect,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AcpAgentProvider {
    Codex,
    Gemini,
    Claude,
}

impl AcpAgentProvider {
    fn label(self) -> &'static str {
        match self {
            AcpAgentProvider::Codex => "Codex",
            AcpAgentProvider::Gemini => "Gemini",
            AcpAgentProvider::Claude => "Claude",
        }
    }

    fn default_command(self) -> String {
        match self {
            AcpAgentProvider::Codex => resolve_default_agent_command(),
            AcpAgentProvider::Gemini => resolve_default_agent_command(),
            AcpAgentProvider::Claude => resolve_default_agent_command(),
        }
    }

    fn infer_from_command(command: &str) -> Self {
        let lower = command.trim().to_ascii_lowercase();
        if lower.starts_with("claude ")
            || lower == "claude"
            || lower.contains("/claude ")
            || lower.contains("claude-agent-acp")
        {
            AcpAgentProvider::Claude
        } else if lower.starts_with("gemini ") || lower == "gemini" || lower.contains("/gemini ") {
            AcpAgentProvider::Gemini
        } else {
            AcpAgentProvider::Codex
        }
    }
}

const AI_CHAT_MAX_CONVERSATION_MESSAGES: usize = 1000;

#[derive(Debug, Default)]
struct MotionLoomScriptNormalization {
    script: String,
    patched_fps: bool,
    patched_size: bool,
    patched_duration: bool,
    patched_text_value: Vec<String>,
    patched_animate_opacity: Vec<String>,
}

#[derive(Debug)]
struct AnimateOpacityCompat {
    target_id: String,
    from: f64,
    to: f64,
    start_ms: f64,
    end_ms: f64,
}

fn is_counted_ai_chat_role(role: AiChatRole) -> bool {
    !matches!(role, AiChatRole::System)
}

fn rebase_ai_chat_index(
    idx: Option<usize>,
    removed_prefix: usize,
    remaining_len: usize,
) -> Option<usize> {
    idx.and_then(|old| old.checked_sub(removed_prefix))
        .filter(|new_idx| *new_idx < remaining_len)
}

fn prune_ai_chat_history(messages: &mut Vec<AiChatMessage>) -> usize {
    let mut counted = messages
        .iter()
        .filter(|msg| is_counted_ai_chat_role(msg.role))
        .count();
    if counted <= AI_CHAT_MAX_CONVERSATION_MESSAGES {
        return 0;
    }

    let mut remove_upto = 0usize;
    while counted > AI_CHAT_MAX_CONVERSATION_MESSAGES && remove_upto < messages.len() {
        while remove_upto < messages.len()
            && matches!(messages[remove_upto].role, AiChatRole::System)
        {
            remove_upto += 1;
        }
        if remove_upto >= messages.len() {
            break;
        }

        let first_role = messages[remove_upto].role;
        remove_upto += 1;
        counted = counted.saturating_sub(1);

        while remove_upto < messages.len()
            && matches!(messages[remove_upto].role, AiChatRole::System)
        {
            remove_upto += 1;
        }

        if matches!(first_role, AiChatRole::User)
            && remove_upto < messages.len()
            && matches!(messages[remove_upto].role, AiChatRole::Assistant)
        {
            remove_upto += 1;
            counted = counted.saturating_sub(1);

            while remove_upto < messages.len()
                && matches!(messages[remove_upto].role, AiChatRole::System)
            {
                remove_upto += 1;
            }
        }
    }

    if remove_upto > 0 {
        messages.drain(0..remove_upto);
    }
    remove_upto
}

#[derive(Clone, Debug)]
enum AgentLoginStatus {
    LoggedIn {
        detail: String,
        source: &'static str,
    },
    LoggedOut {
        reason: String,
        source: &'static str,
    },
    CliMissing {
        detail: String,
    },
    Error(String),
}

impl AgentLoginStatus {
    fn title(&self, provider: AcpAgentProvider) -> String {
        match self {
            AgentLoginStatus::LoggedIn { .. } => format!("{}: Ready", provider.label()),
            AgentLoginStatus::LoggedOut { .. } => format!("{}: Setup Needed", provider.label()),
            AgentLoginStatus::CliMissing { .. } => format!("{} CLI Not Found", provider.label()),
            AgentLoginStatus::Error(_) => format!("{} Status Check Error", provider.label()),
        }
    }

    fn color(&self) -> gpui::Hsla {
        match self {
            AgentLoginStatus::LoggedIn { .. } => rgb(0x22c55e).into(),
            AgentLoginStatus::LoggedOut { .. } => rgb(0xf59e0b).into(),
            AgentLoginStatus::CliMissing { .. } => rgb(0xef4444).into(),
            AgentLoginStatus::Error(_) => rgb(0xf97316).into(),
        }
    }

    fn detail_text(&self) -> String {
        match self {
            AgentLoginStatus::LoggedIn { detail, source } => {
                format!("{detail} (source: {source})")
            }
            AgentLoginStatus::LoggedOut { reason, source } => {
                format!("{reason} (source: {source})")
            }
            AgentLoginStatus::CliMissing { detail } => detail.clone(),
            AgentLoginStatus::Error(err) => err.clone(),
        }
    }

    fn action_hint(&self, provider: AcpAgentProvider) -> &'static str {
        match provider {
            AcpAgentProvider::Codex => match self {
                AgentLoginStatus::LoggedIn { .. } => "Ready for ACP chat.",
                AgentLoginStatus::LoggedOut { .. } => "Run: codex login",
                AgentLoginStatus::CliMissing { .. } => {
                    "Install Codex CLI (or codex-acp), then run: codex login"
                }
                AgentLoginStatus::Error(_) => {
                    "Check terminal logs and verify Codex CLI auth state."
                }
            },
            AcpAgentProvider::Gemini => match self {
                AgentLoginStatus::LoggedIn { .. } => {
                    "Ready for ACP chat. API key is optional; CLI login is supported."
                }
                AgentLoginStatus::LoggedOut { .. } => {
                    "Set GEMINI_API_KEY or login in Gemini CLI (`gemini`, then `/auth`)."
                }
                AgentLoginStatus::CliMissing { .. } => {
                    "Install Gemini CLI or point agent command to a Gemini ACP-capable command."
                }
                AgentLoginStatus::Error(_) => {
                    "Check Gemini CLI install path and auth state (`gemini`, then `/auth`)."
                }
            },
            AcpAgentProvider::Claude => match self {
                AgentLoginStatus::LoggedIn { .. } => {
                    "Ready for ACP chat. Claude CLI login is supported."
                }
                AgentLoginStatus::LoggedOut { .. } => "Run `claude auth login`.",
                AgentLoginStatus::CliMissing { .. } => {
                    "Install Claude Code CLI, then run `claude auth login`."
                }
                AgentLoginStatus::Error(_) => {
                    "Check Claude CLI install path and auth state (`claude auth status`)."
                }
            },
        }
    }
}

fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToString::to_string)
}

fn normalize_cli_override(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn resolve_cli_bin_for_ui(override_value: &str, env_var: &str, bin: &str) -> Option<PathBuf> {
    if let Some(explicit) = normalize_cli_override(override_value) {
        let explicit_path = PathBuf::from(&explicit);
        if explicit_path.components().count() > 1 {
            return explicit_path.is_file().then_some(explicit_path);
        }
        return crate::runtime_paths::resolve_cli_bin(env_var, &explicit);
    }

    crate::runtime_paths::resolve_cli_bin(env_var, bin)
}

fn codex_install_hint() -> String {
    "Codex CLI not found.\nInstall first, then restart Anica:\n1) npm i -g @openai/codex\n2) codex login\n3) codex --version\nOptional: set ANICA_CODEX_CLI_BIN=/absolute/path/to/codex if PATH is not visible to GUI app."
        .to_string()
}

fn gemini_install_hint() -> String {
    "Gemini CLI not found.\nInstall Gemini CLI (`npm install -g @google/gemini-cli`) so bundled `anica-acp` can route prompts through Gemini when Gemini provider is selected.\nYou can authenticate with GEMINI_API_KEY, or use CLI login (`gemini`, then `/auth`)."
        .to_string()
}

fn claude_install_hint() -> String {
    "Claude CLI not found.\nInstall Claude Code CLI, then login:\n1) curl -fsSL https://claude.ai/install.sh | bash\n2) claude auth login\n3) claude auth status\nAlternative npm path (legacy): npm install -g @anthropic-ai/claude-code"
        .to_string()
}

fn has_gemini_api_key(gemini_api_key: &str) -> bool {
    if !gemini_api_key.trim().is_empty() {
        return true;
    }

    env::var("GEMINI_API_KEY")
        .ok()
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
        || env::var("GOOGLE_API_KEY")
            .ok()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
}

fn detect_gemini_cli_auth_from_files() -> Option<AgentLoginStatus> {
    let home = env::var_os("HOME")?;
    let gemini_dir = PathBuf::from(home).join(".gemini");
    let settings_path = gemini_dir.join("settings.json");
    let oauth_path = gemini_dir.join("oauth_creds.json");

    let settings_json = fs::read_to_string(&settings_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());

    let selected_auth_type = settings_json
        .as_ref()
        .and_then(|json| json.get("security"))
        .and_then(|v| v.get("auth"))
        .and_then(|v| v.get("selectedType"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string);

    let oauth_has_token = || -> bool {
        let Ok(raw) = fs::read_to_string(&oauth_path) else {
            return false;
        };
        let Ok(json) = serde_json::from_str::<Value>(&raw) else {
            return false;
        };

        let access = json
            .get("access_token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let refresh = json
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();

        !access.is_empty() || !refresh.is_empty()
    };

    if let Some(auth_type) = selected_auth_type {
        return Some(match auth_type.as_str() {
            "oauth-personal" => {
                if oauth_has_token() {
                    AgentLoginStatus::LoggedIn {
                        detail: "Gemini CLI OAuth credentials detected".to_string(),
                        source: "~/.gemini/oauth_creds.json",
                    }
                } else {
                    AgentLoginStatus::LoggedOut {
                        reason: "Gemini CLI auth is `oauth-personal`, but OAuth credentials are missing or invalid".to_string(),
                        source: "~/.gemini/oauth_creds.json",
                    }
                }
            }
            "compute-default-credentials" | "cloud-shell" => AgentLoginStatus::LoggedIn {
                detail: format!("Gemini CLI auth type is `{auth_type}`"),
                source: "~/.gemini/settings.json",
            },
            "vertex-ai" => {
                let has_google_api_key = env::var("GOOGLE_API_KEY")
                    .ok()
                    .map(|v| !v.trim().is_empty())
                    .unwrap_or(false);
                let has_cloud_project = env::var("GOOGLE_CLOUD_PROJECT")
                    .ok()
                    .or_else(|| env::var("GOOGLE_CLOUD_PROJECT_ID").ok())
                    .map(|v| !v.trim().is_empty())
                    .unwrap_or(false);
                let has_cloud_location = env::var("GOOGLE_CLOUD_LOCATION")
                    .ok()
                    .map(|v| !v.trim().is_empty())
                    .unwrap_or(false);

                if has_google_api_key || (has_cloud_project && has_cloud_location) {
                    AgentLoginStatus::LoggedIn {
                        detail: "Gemini CLI Vertex auth config detected".to_string(),
                        source: "~/.gemini/settings.json",
                    }
                } else {
                    AgentLoginStatus::LoggedOut {
                        reason: "Gemini CLI auth is `vertex-ai`, but required GOOGLE_API_KEY or GOOGLE_CLOUD_PROJECT/GOOGLE_CLOUD_LOCATION is missing".to_string(),
                        source: "~/.gemini/settings.json",
                    }
                }
            }
            "gemini-api-key" => AgentLoginStatus::LoggedOut {
                reason: "Gemini CLI auth is `gemini-api-key`, but no key is set in Anica/env"
                    .to_string(),
                source: "~/.gemini/settings.json",
            },
            _ => AgentLoginStatus::LoggedOut {
                reason: format!(
                    "Gemini CLI auth type `{auth_type}` is configured but could not be verified from Anica"
                ),
                source: "~/.gemini/settings.json",
            },
        });
    }

    if oauth_has_token() {
        return Some(AgentLoginStatus::LoggedIn {
            detail: "Gemini CLI OAuth credentials detected".to_string(),
            source: "~/.gemini/oauth_creds.json",
        });
    }

    None
}

fn detect_from_auth_file() -> Option<AgentLoginStatus> {
    let home = env::var_os("HOME")?;
    let auth_path = PathBuf::from(home).join(".codex").join("auth.json");

    if !auth_path.exists() {
        return Some(AgentLoginStatus::LoggedOut {
            reason: format!("Auth file not found: {}", auth_path.display()),
            source: "~/.codex/auth.json",
        });
    }

    let raw = match fs::read_to_string(&auth_path) {
        Ok(v) => v,
        Err(err) => {
            return Some(AgentLoginStatus::Error(format!(
                "Failed to read {}: {err}",
                auth_path.display()
            )));
        }
    };

    let json: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(err) => {
            return Some(AgentLoginStatus::Error(format!(
                "Failed to parse {}: {err}",
                auth_path.display()
            )));
        }
    };

    let token = json
        .get("tokens")
        .and_then(|v| v.get("access_token"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    let api_key = json
        .get("OPENAI_API_KEY")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    let auth_mode = json
        .get("auth_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    if !token.is_empty() || !api_key.is_empty() {
        let account_hint = json
            .get("tokens")
            .and_then(|v| v.get("account_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown-account");

        return Some(AgentLoginStatus::LoggedIn {
            detail: format!("Credentials detected (auth_mode={auth_mode}, account={account_hint})"),
            source: "~/.codex/auth.json",
        });
    }

    Some(AgentLoginStatus::LoggedOut {
        reason: "Auth file exists but no usable access token/API key found".to_string(),
        source: "~/.codex/auth.json",
    })
}

fn detect_codex_login_status_with_override(codex_cli_bin_override: &str) -> AgentLoginStatus {
    let codex_bin = resolve_cli_bin_for_ui(codex_cli_bin_override, "ANICA_CODEX_CLI_BIN", "codex")
        .unwrap_or_else(|| PathBuf::from("codex"));
    match Command::new(&codex_bin).args(["login", "status"]).output() {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let combined_lc = format!("{}\n{}", stdout, stderr).to_lowercase();

            if out.status.success() {
                if combined_lc.contains("not logged")
                    || combined_lc.contains("login required")
                    || combined_lc.contains("unauthorized")
                {
                    return AgentLoginStatus::LoggedOut {
                        reason: first_non_empty_line(&stdout)
                            .or_else(|| first_non_empty_line(&stderr))
                            .unwrap_or_else(|| "CLI reports not logged in".to_string()),
                        source: "codex login status",
                    };
                }

                return AgentLoginStatus::LoggedIn {
                    detail: first_non_empty_line(&stdout)
                        .or_else(|| first_non_empty_line(&stderr))
                        .unwrap_or_else(|| "codex login status succeeded".to_string()),
                    source: "codex login status",
                };
            }

            if combined_lc.contains("not logged")
                || combined_lc.contains("login required")
                || combined_lc.contains("unauthorized")
            {
                return AgentLoginStatus::LoggedOut {
                    reason: first_non_empty_line(&stderr)
                        .or_else(|| first_non_empty_line(&stdout))
                        .unwrap_or_else(|| "CLI reports not logged in".to_string()),
                    source: "codex login status",
                };
            }

            detect_from_auth_file().unwrap_or_else(|| {
                AgentLoginStatus::Error(format!(
                    "`codex login status` failed with code {:?}",
                    out.status.code()
                ))
            })
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            if let Some(status) = detect_from_auth_file() {
                status
            } else {
                AgentLoginStatus::CliMissing {
                    detail: "Cannot find `codex`. Install Codex CLI, or set ANICA_CODEX_CLI_BIN to the absolute path of the codex executable.".to_string(),
                }
            }
        }
        Err(err) => detect_from_auth_file().unwrap_or_else(|| {
            AgentLoginStatus::Error(format!("Failed to execute `codex login status`: {err}"))
        }),
    }
}

fn detect_gemini_login_status(
    gemini_api_key: &str,
    gemini_cli_bin_override: &str,
) -> AgentLoginStatus {
    if resolve_cli_bin_for_ui(gemini_cli_bin_override, "ANICA_GEMINI_CLI_BIN", "gemini").is_none() {
        return AgentLoginStatus::CliMissing {
            detail: "Cannot find `gemini`. Install Gemini CLI, or set a Gemini CLI path in AI Agents settings.".to_string(),
        };
    }

    if has_gemini_api_key(gemini_api_key) {
        let source = if !gemini_api_key.trim().is_empty() {
            "AI Agents page"
        } else if env::var("GEMINI_API_KEY")
            .ok()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
        {
            "GEMINI_API_KEY"
        } else {
            "GOOGLE_API_KEY"
        };

        return AgentLoginStatus::LoggedIn {
            detail: "Gemini API key present".to_string(),
            source,
        };
    }

    if let Some(status) = detect_gemini_cli_auth_from_files() {
        return status;
    }

    AgentLoginStatus::LoggedOut {
        reason: "Gemini CLI is available, but no API key and no CLI auth credentials were detected"
            .to_string(),
        source: "environment + ~/.gemini",
    }
}

fn detect_claude_login_status_with_override(claude_cli_bin_override: &str) -> AgentLoginStatus {
    if resolve_cli_bin_for_ui(claude_cli_bin_override, "ANICA_CLAUDE_CLI_BIN", "claude").is_none() {
        return AgentLoginStatus::CliMissing {
            detail: "Cannot find `claude`. Install Claude Code CLI, or set a Claude CLI path in AI Agents settings.".to_string(),
        };
    }

    let claude_bin =
        resolve_cli_bin_for_ui(claude_cli_bin_override, "ANICA_CLAUDE_CLI_BIN", "claude")
            .unwrap_or_else(|| PathBuf::from("claude"));

    let parse_status_output = |out: std::process::Output, source: &'static str| {
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        let combined_lc = format!("{}\n{}", stdout, stderr).to_lowercase();
        if out.status.success() {
            return AgentLoginStatus::LoggedIn {
                detail: first_non_empty_line(&stdout)
                    .or_else(|| first_non_empty_line(&stderr))
                    .unwrap_or_else(|| "claude auth status succeeded".to_string()),
                source,
            };
        }

        if combined_lc.contains("not logged")
            || combined_lc.contains("login required")
            || combined_lc.contains("auth required")
            || combined_lc.contains("unauthorized")
        {
            return AgentLoginStatus::LoggedOut {
                reason: first_non_empty_line(&stderr)
                    .or_else(|| first_non_empty_line(&stdout))
                    .unwrap_or_else(|| "Claude CLI reports not logged in".to_string()),
                source,
            };
        }

        AgentLoginStatus::LoggedOut {
            reason: first_non_empty_line(&stderr)
                .or_else(|| first_non_empty_line(&stdout))
                .unwrap_or_else(|| format!("`{source}` failed with code {:?}", out.status.code())),
            source,
        }
    };

    match Command::new(&claude_bin)
        .args(["auth", "status", "--text"])
        .output()
    {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
            let stdout = String::from_utf8_lossy(&out.stdout).to_lowercase();
            let unknown_text_flag = (!out.status.success())
                && (stderr.contains("unknown option '--text'")
                    || stderr.contains("unknown option: --text")
                    || stderr.contains("unrecognized option '--text'")
                    || stdout.contains("unknown option '--text'")
                    || stdout.contains("unknown option: --text")
                    || stdout.contains("unrecognized option '--text'"));
            if unknown_text_flag {
                return match Command::new(&claude_bin).args(["auth", "status"]).output() {
                    Ok(fallback) => parse_status_output(fallback, "claude auth status"),
                    Err(err) => AgentLoginStatus::Error(format!(
                        "Failed to execute `claude auth status`: {err}"
                    )),
                };
            }
            parse_status_output(out, "claude auth status --text")
        }
        Err(err) if err.kind() == ErrorKind::NotFound => AgentLoginStatus::CliMissing {
            detail: "Cannot find `claude` in PATH. Install Claude Code CLI or launch app from a shell where `claude` is available.".to_string(),
        },
        Err(err) => AgentLoginStatus::Error(format!(
            "Failed to execute `claude auth status --text`: {err}"
        )),
    }
}

fn bundled_acp_command() -> Option<String> {
    if let Ok(exe) = env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        // Dev/bundled sidecar next to main executable.
        for name in ["anica-acp", "codex-acp"] {
            let candidate = exe_dir.join(name);
            if candidate.exists() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }

    // macOS app bundle: .../Contents/MacOS/anica -> .../Contents/Resources/codex-acp
    if let Ok(exe) = env::current_exe()
        && let Some(macos_dir) = exe.parent()
        && let Some(contents_dir) = macos_dir.parent()
    {
        for name in ["anica-acp", "codex-acp"] {
            let candidate = contents_dir.join("Resources").join(name);
            if candidate.exists() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }

    // Dev fallback: local bundled helper in repo.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for rel in [
        "assets/bin/anica-acp",
        "assets/bin/codex-acp",
        "bin/anica-acp",
        "bin/codex-acp",
    ] {
        let p = manifest.join(rel);
        if p.exists() {
            return Some(p.to_string_lossy().to_string());
        }
    }

    None
}

fn split_first_token(command: &str) -> Option<(&str, &str)> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }
    let idx = trimmed
        .find(|c: char| c.is_whitespace())
        .unwrap_or(trimmed.len());
    Some((&trimmed[..idx], &trimmed[idx..]))
}

fn sidecar_candidates_for_name(name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        out.push(exe_dir.join(name));
        // macOS app bundle: .../Contents/MacOS/anica -> .../Contents/Resources/anica-acp
        if let Some(contents_dir) = exe_dir.parent()
            && contents_dir
                .file_name()
                .and_then(|v| v.to_str())
                .is_some_and(|v| v == "Contents")
        {
            out.push(contents_dir.join("Resources").join(name));
        }
    }
    out
}

fn sidecar_candidate_for_token(token: &str) -> Option<String> {
    let path = PathBuf::from(token);
    let sidecar_name = path.file_name().and_then(|v| v.to_str()).and_then(|name| {
        if name == "anica-acp" || name == "codex-acp" {
            Some(name)
        } else {
            None
        }
    });

    if path.is_absolute() {
        if let Some(name) = sidecar_name {
            for candidate in sidecar_candidates_for_name(name) {
                if candidate.is_file() {
                    return Some(candidate.to_string_lossy().to_string());
                }
            }
        }
        if path.is_file() {
            return Some(path.to_string_lossy().to_string());
        }
        return None;
    }

    if token.contains('/') || token.contains('\\') {
        if let Some(name) = sidecar_name {
            for candidate in sidecar_candidates_for_name(name) {
                if candidate.is_file() {
                    return Some(candidate.to_string_lossy().to_string());
                }
            }
        }
        if let Ok(exe) = env::current_exe()
            && let Some(exe_dir) = exe.parent()
        {
            let candidate = exe_dir.join(&path);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
        return None;
    }

    if token == "anica-acp" || token == "codex-acp" {
        for candidate in sidecar_candidates_for_name(token) {
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }
    None
}

fn resolve_agent_command_for_spawn(command: &str) -> String {
    let trimmed = command.trim();
    if PathBuf::from(trimmed).is_file() {
        return shell_quote_token(trimmed);
    }
    let Some((token, rest)) = split_first_token(command) else {
        return trimmed.to_string();
    };
    if let Some(resolved_token) = sidecar_candidate_for_token(token) {
        format!("{}{}", shell_quote_token(&resolved_token), rest)
    } else {
        trimmed.to_string()
    }
}

fn shell_quote_token(token: &str) -> String {
    if cfg!(windows) {
        return token.to_string();
    }
    if !token.chars().any(char::is_whitespace) && !token.contains('\'') {
        return token.to_string();
    }
    format!("'{}'", token.replace('\'', "'\"'\"'"))
}

fn agent_command_env_override() -> Option<String> {
    env::var("ANICA_ACP_AGENT_CMD")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn resolve_default_agent_command() -> String {
    if let Some(cmd) = agent_command_env_override() {
        return cmd;
    }

    if bundled_acp_command().is_some() {
        // Keep UI command portable/user-friendly. Connect-time resolution maps this
        // short command to a sidecar binary next to the executable when available.
        return "anica-acp".to_string();
    }

    "anica-acp".to_string()
}

fn normalize_agent_command_for_ui(command: String) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return "anica-acp".to_string();
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute()
        && let Some(name) = path.file_name().and_then(|s| s.to_str())
        && (name == "anica-acp" || name == "codex-acp")
    {
        return name.to_string();
    }
    trimmed.to_string()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProviderModelOption {
    slug: String,
    label: String,
}

fn provider_model_option(slug: impl Into<String>, label: impl Into<String>) -> ProviderModelOption {
    ProviderModelOption {
        slug: slug.into(),
        label: label.into(),
    }
}

fn fallback_codex_model_options() -> Vec<ProviderModelOption> {
    [
        ("gpt-5.5", "GPT-5.5"),
        ("gpt-5.4", "gpt-5.4"),
        ("gpt-5.4-mini", "GPT-5.4-Mini"),
        ("gpt-5.3-codex", "gpt-5.3-codex"),
        ("gpt-5.2", "gpt-5.2"),
    ]
    .into_iter()
    .map(|(slug, label)| provider_model_option(slug, label))
    .collect()
}

fn fallback_gemini_model_options() -> Vec<ProviderModelOption> {
    [
        ("auto", "Auto"),
        ("pro", "Pro"),
        ("flash", "Flash"),
        ("flash-lite", "Flash Lite"),
        ("gemini-3.1-pro-preview", "Gemini 3.1 Pro Preview"),
        ("gemini-3-pro-preview", "Gemini 3 Pro Preview"),
        ("gemini-3-flash-preview", "Gemini 3 Flash Preview"),
        ("gemini-2.5-pro", "Gemini 2.5 Pro"),
        ("gemini-2.5-flash", "Gemini 2.5 Flash"),
        ("gemini-2.5-flash-lite", "Gemini 2.5 Flash Lite"),
    ]
    .into_iter()
    .map(|(slug, label)| provider_model_option(slug, label))
    .collect()
}

fn fallback_claude_model_options() -> Vec<ProviderModelOption> {
    [
        ("sonnet", "Sonnet"),
        ("opus", "Opus"),
        ("claude-sonnet-4-6", "claude-sonnet-4-6"),
    ]
    .into_iter()
    .map(|(slug, label)| provider_model_option(slug, label))
    .collect()
}

fn codex_config_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".codex"))
}

fn codex_models_cache_path() -> Option<PathBuf> {
    codex_config_dir().map(|dir| dir.join("models_cache.json"))
}

fn read_codex_config_model() -> Option<String> {
    let path = codex_config_dir()?.join("config.toml");
    let raw = fs::read_to_string(path).ok()?;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        if key.trim() != "model" {
            continue;
        }
        let model = value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim()
            .to_string();
        if !model.is_empty() {
            return Some(model);
        }
    }
    None
}

fn load_codex_model_options() -> (Vec<ProviderModelOption>, String) {
    let Some(path) = codex_models_cache_path() else {
        return (
            fallback_codex_model_options(),
            "fallback list (no HOME detected)".to_string(),
        );
    };
    let Ok(raw) = fs::read_to_string(&path) else {
        return (
            fallback_codex_model_options(),
            format!("fallback list (missing {})", path.display()),
        );
    };
    let Ok(json) = serde_json::from_str::<Value>(&raw) else {
        return (
            fallback_codex_model_options(),
            format!("fallback list (invalid {})", path.display()),
        );
    };

    let mut options = Vec::<ProviderModelOption>::new();
    if let Some(models) = json.get("models").and_then(Value::as_array) {
        for item in models {
            let Some(slug) = item.get("slug").and_then(Value::as_str) else {
                continue;
            };
            if slug.trim().is_empty() || slug == "codex-auto-review" {
                continue;
            }
            if item
                .get("visibility")
                .and_then(Value::as_str)
                .is_some_and(|visibility| visibility != "list")
            {
                continue;
            }
            if options.iter().any(|option| option.slug == slug) {
                continue;
            }
            let label = item
                .get("display_name")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(slug);
            options.push(ProviderModelOption {
                slug: slug.to_string(),
                label: label.to_string(),
            });
        }
    }

    if options.is_empty() {
        return (
            fallback_codex_model_options(),
            format!("fallback list (empty {})", path.display()),
        );
    }

    (options, format!("Codex cache: {}", path.display()))
}

fn choose_codex_model(options: &mut Vec<ProviderModelOption>, preferred: Option<&str>) -> String {
    let selected = preferred
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(read_codex_config_model)
        .or_else(|| options.first().map(|option| option.slug.clone()))
        .unwrap_or_else(|| "gpt-5.5".to_string());

    if !options.iter().any(|option| option.slug == selected) {
        options.insert(
            0,
            ProviderModelOption {
                slug: selected.clone(),
                label: selected.clone(),
            },
        );
    }

    selected
}

fn choose_provider_model(
    options: &mut Vec<ProviderModelOption>,
    preferred: Option<&str>,
    fallback: &str,
) -> String {
    let selected = preferred
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| options.first().map(|option| option.slug.clone()))
        .unwrap_or_else(|| fallback.to_string());

    if !options.iter().any(|option| option.slug == selected) {
        options.insert(0, provider_model_option(selected.clone(), selected.clone()));
    }

    selected
}

fn read_gemini_settings_model() -> Option<String> {
    let path = env::var_os("HOME")
        .map(PathBuf::from)?
        .join(".gemini")
        .join("settings.json");
    let raw = fs::read_to_string(path).ok()?;
    let json = serde_json::from_str::<Value>(&raw).ok()?;
    json.get("model")
        .and_then(|model| model.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            json.get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
}

fn gemini_cli_models_path(gemini_cli_bin: &str) -> Option<PathBuf> {
    let cli_path = resolve_cli_bin_for_ui(gemini_cli_bin, "ANICA_GEMINI_CLI_BIN", "gemini")?;
    let canonical = fs::canonicalize(&cli_path).unwrap_or(cli_path);
    for parent in canonical.ancestors() {
        let candidate = parent
            .join("..")
            .join("node_modules")
            .join("@google")
            .join("gemini-cli-core")
            .join("dist")
            .join("src")
            .join("config")
            .join("models.js");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn js_exported_const_string(raw: &str, name: &str) -> Option<String> {
    let prefix = format!("export const {name} = ");
    raw.lines().find_map(|line| {
        let value = line.trim().strip_prefix(&prefix)?.trim();
        value
            .strip_prefix('\'')
            .and_then(|rest| rest.split_once('\'').map(|(value, _)| value.to_string()))
            .or_else(|| {
                value
                    .strip_prefix('"')
                    .and_then(|rest| rest.split_once('"').map(|(value, _)| value.to_string()))
            })
    })
}

fn gemini_model_label(slug: &str) -> String {
    match slug {
        "auto" => "Auto".to_string(),
        "pro" => "Pro".to_string(),
        "flash" => "Flash".to_string(),
        "flash-lite" => "Flash Lite".to_string(),
        "auto-gemini-3" => "Auto (Gemini 3)".to_string(),
        "auto-gemini-2.5" => "Auto (Gemini 2.5)".to_string(),
        "gemini-3.1-pro-preview" => "Gemini 3.1 Pro Preview".to_string(),
        "gemini-3-pro-preview" => "Gemini 3 Pro Preview".to_string(),
        "gemini-3-flash-preview" => "Gemini 3 Flash Preview".to_string(),
        "gemini-2.5-pro" => "Gemini 2.5 Pro".to_string(),
        "gemini-2.5-flash" => "Gemini 2.5 Flash".to_string(),
        "gemini-2.5-flash-lite" => "Gemini 2.5 Flash Lite".to_string(),
        _ => slug.to_string(),
    }
}

fn load_gemini_model_options(gemini_cli_bin: &str) -> (Vec<ProviderModelOption>, String) {
    let Some(path) = gemini_cli_models_path(gemini_cli_bin) else {
        return (
            fallback_gemini_model_options(),
            "fallback list (Gemini CLI model constants not found)".to_string(),
        );
    };
    let Ok(raw) = fs::read_to_string(&path) else {
        return (
            fallback_gemini_model_options(),
            format!("fallback list (failed to read {})", path.display()),
        );
    };

    let mut options = Vec::<ProviderModelOption>::new();
    for const_name in [
        "GEMINI_MODEL_ALIAS_AUTO",
        "GEMINI_MODEL_ALIAS_PRO",
        "GEMINI_MODEL_ALIAS_FLASH",
        "GEMINI_MODEL_ALIAS_FLASH_LITE",
        "PREVIEW_GEMINI_MODEL_AUTO",
        "DEFAULT_GEMINI_MODEL_AUTO",
        "PREVIEW_GEMINI_3_1_MODEL",
        "PREVIEW_GEMINI_MODEL",
        "PREVIEW_GEMINI_FLASH_MODEL",
        "DEFAULT_GEMINI_MODEL",
        "DEFAULT_GEMINI_FLASH_MODEL",
        "DEFAULT_GEMINI_FLASH_LITE_MODEL",
    ] {
        let Some(slug) = js_exported_const_string(&raw, const_name) else {
            continue;
        };
        if options.iter().any(|option| option.slug == slug) {
            continue;
        }
        options.push(provider_model_option(
            slug.clone(),
            gemini_model_label(&slug),
        ));
    }

    if options.is_empty() {
        return (
            fallback_gemini_model_options(),
            format!("fallback list (empty {})", path.display()),
        );
    }
    (options, format!("Gemini CLI constants: {}", path.display()))
}

fn read_claude_settings_model() -> Option<String> {
    let path = env::var_os("HOME")
        .map(PathBuf::from)?
        .join(".claude")
        .join("settings.json");
    let raw = fs::read_to_string(path).ok()?;
    let json = serde_json::from_str::<Value>(&raw).ok()?;
    ["model", "modelName", "defaultModel"]
        .iter()
        .find_map(|key| json.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn load_claude_model_options(_claude_cli_bin: &str) -> (Vec<ProviderModelOption>, String) {
    (
        fallback_claude_model_options(),
        "Claude CLI aliases/full model names".to_string(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CodexReasoningMode {
    Low,
    Medium,
    High,
    ExtraHigh,
}

impl CodexReasoningMode {
    fn label(self) -> &'static str {
        match self {
            CodexReasoningMode::Low => "Low",
            CodexReasoningMode::Medium => "Medium",
            CodexReasoningMode::High => "High",
            CodexReasoningMode::ExtraHigh => "Extra High",
        }
    }

    fn env_value(self) -> &'static str {
        match self {
            CodexReasoningMode::Low => "low",
            CodexReasoningMode::Medium => "medium",
            CodexReasoningMode::High => "high",
            CodexReasoningMode::ExtraHigh => "xhigh",
        }
    }

    fn from_setting_value(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "low" => Some(CodexReasoningMode::Low),
            "medium" => Some(CodexReasoningMode::Medium),
            "high" => Some(CodexReasoningMode::High),
            "xhigh" | "extra_high" | "extra-high" | "extra high" => {
                Some(CodexReasoningMode::ExtraHigh)
            }
            _ => None,
        }
    }
}

pub struct AiAgentsPage {
    pub global: Entity<GlobalState>,
    agent_provider: AcpAgentProvider,
    agent_status: AgentLoginStatus,

    worker: AcpWorker,
    connected: bool,
    // Tracks which provider is actually connected, so the UI can highlight it.
    connected_provider: Option<AcpAgentProvider>,
    busy: bool,
    codex_model: String,
    codex_model_options: Vec<ProviderModelOption>,
    codex_models_source: String,
    gemini_model: String,
    gemini_model_options: Vec<ProviderModelOption>,
    gemini_models_source: String,
    claude_model: String,
    claude_model_options: Vec<ProviderModelOption>,
    claude_models_source: String,
    codex_reasoning_mode: CodexReasoningMode,
    auto_connect_enabled: bool,
    auto_connect_scope: SettingsScope,

    agent_command: String,
    codex_cli_bin: String,
    gemini_cli_bin: String,
    claude_cli_bin: String,
    prompt_text: String,
    last_status: String,

    command_input: Option<Entity<InputState>>,
    command_input_sub: Option<Subscription>,
    codex_cli_input: Option<Entity<InputState>>,
    codex_cli_input_sub: Option<Subscription>,
    gemini_cli_input: Option<Entity<InputState>>,
    gemini_cli_input_sub: Option<Subscription>,
    claude_cli_input: Option<Entity<InputState>>,
    claude_cli_input_sub: Option<Subscription>,

    gemini_api_key: String,
    gemini_key_input: Option<Entity<InputState>>,
    gemini_key_input_sub: Option<Subscription>,

    prompt_input: Option<Entity<InputState>>,
    prompt_input_sub: Option<Subscription>,

    active_assistant_idx: Option<usize>,
    media_pool_signature: u64,
    last_chat_status: String,
    prompt_send_on_next_render: bool,
    show_system_messages: bool,

    poll_running: bool,
    poll_token: u64,
    pending_input_resync: bool,
}

impl AiAgentsPage {
    fn reload_codex_model_options(&mut self, preserve_current: bool) {
        let preferred = if preserve_current {
            Some(self.codex_model.as_str())
        } else {
            None
        };
        let (mut options, source) = load_codex_model_options();
        let selected = choose_codex_model(&mut options, preferred);
        self.codex_model = selected;
        self.codex_model_options = options;
        self.codex_models_source = source;
    }

    fn reload_gemini_model_options(&mut self, preserve_current: bool) {
        let preferred_owned = if preserve_current {
            None
        } else {
            env::var("GEMINI_MODEL")
                .ok()
                .or_else(read_gemini_settings_model)
        };
        let preferred = if preserve_current {
            Some(self.gemini_model.as_str())
        } else {
            preferred_owned.as_deref()
        };
        let (mut options, source) = load_gemini_model_options(&self.gemini_cli_bin);
        let selected = choose_provider_model(&mut options, preferred, "auto");
        self.gemini_model = selected;
        self.gemini_model_options = options;
        self.gemini_models_source = source;
    }

    fn reload_claude_model_options(&mut self, preserve_current: bool) {
        let preferred_owned = if preserve_current {
            None
        } else {
            read_claude_settings_model()
        };
        let preferred = if preserve_current {
            Some(self.claude_model.as_str())
        } else {
            preferred_owned.as_deref()
        };
        let (mut options, source) = load_claude_model_options(&self.claude_cli_bin);
        let selected = choose_provider_model(&mut options, preferred, "sonnet");
        self.claude_model = selected;
        self.claude_model_options = options;
        self.claude_models_source = source;
    }

    fn refresh_agent_status(&mut self) {
        self.agent_status = match self.agent_provider {
            AcpAgentProvider::Codex => detect_codex_login_status_with_override(&self.codex_cli_bin),
            AcpAgentProvider::Gemini => {
                detect_gemini_login_status(&self.gemini_api_key, &self.gemini_cli_bin)
            }
            AcpAgentProvider::Claude => {
                detect_claude_login_status_with_override(&self.claude_cli_bin)
            }
        };
    }

    fn set_agent_command_value(
        &mut self,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.agent_command = value.clone();
        if let Some(input) = self.command_input.as_ref() {
            input.update(cx, |this, cx| {
                this.set_value(value.clone(), window, cx);
            });
        }
    }

    fn rebuild_command_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.command_input = None;
        self.command_input_sub = None;

        let input = cx.new(|cx| InputState::new(window, cx).placeholder("ACP agent command"));
        let current = self.agent_command.clone();
        input.update(cx, |this, cx| {
            this.set_value(current.clone(), window, cx);
        });

        let sub = cx.subscribe(&input, |this, input, ev, cx| {
            if !matches!(ev, InputEvent::Change) {
                return;
            }
            this.agent_command = input.read(cx).value().to_string();
        });

        self.command_input = Some(input);
        self.command_input_sub = Some(sub);
    }

    fn select_agent_provider(
        &mut self,
        provider: AcpAgentProvider,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.agent_provider == provider {
            return;
        }
        self.agent_provider = provider;
        let default_command = normalize_agent_command_for_ui(provider.default_command());
        self.agent_command = default_command.clone();
        self.rebuild_command_input(window, cx);
        self.set_agent_command_value(default_command, window, cx);
        match provider {
            AcpAgentProvider::Codex => self.reload_codex_model_options(true),
            AcpAgentProvider::Gemini => self.reload_gemini_model_options(true),
            AcpAgentProvider::Claude => self.reload_claude_model_options(true),
        }
        self.pending_input_resync = true;
        self.refresh_agent_status();
        self.push_system_message(
            format!(
                "ACP provider switched to {}. Command reset to: {}",
                provider.label(),
                self.agent_command
            ),
            cx,
        );
    }

    pub fn new(global: Entity<GlobalState>, cx: &mut Context<Self>) -> Self {
        let workspace_root = resolve_workspace_root(global.read(cx).project_file_path.as_deref());
        let loaded_settings = load_settings(&workspace_root);

        let mut agent_command = resolve_default_agent_command();
        if agent_command_env_override().is_none()
            && let Some(saved_command) = loaded_settings.effective.acp_agent_command.clone()
        {
            agent_command = saved_command;
        }
        if agent_command_env_override().is_none() {
            agent_command = normalize_agent_command_for_ui(agent_command);
        }
        let agent_provider = AcpAgentProvider::infer_from_command(&agent_command);

        let reasoning_mode = loaded_settings
            .effective
            .acp_reasoning_mode
            .as_deref()
            .and_then(CodexReasoningMode::from_setting_value)
            .unwrap_or(CodexReasoningMode::Medium);
        let (mut codex_model_options, codex_models_source) = load_codex_model_options();
        let codex_model = choose_codex_model(&mut codex_model_options, None);
        let auto_connect_enabled = loaded_settings.effective.acp_auto_connect;
        let auto_connect_scope = loaded_settings.auto_connect_source.preferred_scope();
        let gemini_api_key = env::var("GEMINI_API_KEY")
            .ok()
            .or_else(|| env::var("GOOGLE_API_KEY").ok())
            .unwrap_or_default();
        let codex_cli_bin = loaded_settings
            .effective
            .acp_codex_cli_bin
            .clone()
            .unwrap_or_default();
        let gemini_cli_bin = loaded_settings
            .effective
            .acp_gemini_cli_bin
            .clone()
            .unwrap_or_default();
        let claude_cli_bin = loaded_settings
            .effective
            .acp_claude_cli_bin
            .clone()
            .unwrap_or_default();
        let gemini_preferred_model = env::var("GEMINI_MODEL")
            .ok()
            .or_else(read_gemini_settings_model);
        let (mut gemini_model_options, gemini_models_source) =
            load_gemini_model_options(&gemini_cli_bin);
        let gemini_model = choose_provider_model(
            &mut gemini_model_options,
            gemini_preferred_model.as_deref(),
            "auto",
        );
        let claude_preferred_model = read_claude_settings_model();
        let (mut claude_model_options, claude_models_source) =
            load_claude_model_options(&claude_cli_bin);
        let claude_model = choose_provider_model(
            &mut claude_model_options,
            claude_preferred_model.as_deref(),
            "sonnet",
        );

        cx.observe(&global, |this, _global, cx| {
            this.sync_worker_media_pool_snapshot(cx);
            cx.notify();
        })
        .detach();
        cx.subscribe(&global, |this, _global, evt: &MediaPoolUiEvent, cx| {
            if matches!(evt, MediaPoolUiEvent::StateChanged) {
                this.sync_worker_media_pool_snapshot(cx);
                cx.notify();
            }
        })
        .detach();

        let mut this = Self {
            global,
            agent_provider,
            agent_status: AgentLoginStatus::Error("Status not initialized".to_string()),

            worker: AcpWorker::spawn(),
            connected: false,
            connected_provider: None,
            busy: false,
            codex_model,
            codex_model_options,
            codex_models_source,
            gemini_model,
            gemini_model_options,
            gemini_models_source,
            claude_model,
            claude_model_options,
            claude_models_source,
            codex_reasoning_mode: reasoning_mode,
            auto_connect_enabled,
            auto_connect_scope,

            agent_command,
            codex_cli_bin,
            gemini_cli_bin,
            claude_cli_bin,
            prompt_text: String::new(),
            last_status: "Idle".to_string(),

            command_input: None,
            command_input_sub: None,
            codex_cli_input: None,
            codex_cli_input_sub: None,
            gemini_cli_input: None,
            gemini_cli_input_sub: None,
            claude_cli_input: None,
            claude_cli_input_sub: None,

            gemini_api_key,
            gemini_key_input: None,
            gemini_key_input_sub: None,

            prompt_input: None,
            prompt_input_sub: None,

            active_assistant_idx: None,
            media_pool_signature: 0,
            last_chat_status: String::new(),
            prompt_send_on_next_render: false,
            show_system_messages: false,

            poll_running: false,
            poll_token: 0,
            pending_input_resync: true,
        };
        this.refresh_agent_status();

        this.sync_worker_media_pool_snapshot(cx);

        this.push_system_message(
            format!(
                "ACP chat ready. Provider: {}. Set agent command, click Connect, then send prompt.",
                this.agent_provider.label()
            ),
            cx,
        );
        if matches!(this.agent_status, AgentLoginStatus::CliMissing { .. }) {
            match this.agent_provider {
                AcpAgentProvider::Codex => this.push_system_message(codex_install_hint(), cx),
                AcpAgentProvider::Gemini => this.push_system_message(gemini_install_hint(), cx),
                AcpAgentProvider::Claude => this.push_system_message(claude_install_hint(), cx),
            }
        }
        this.ensure_worker_poller(cx);
        if this.auto_connect_enabled {
            this.push_system_message(
                format!(
                    "Auto Connect is enabled ({} settings). Connecting...",
                    this.auto_connect_scope.label()
                ),
                cx,
            );
            this.on_connect(cx);
        }

        this
    }

    fn action_button(label: impl Into<String>) -> gpui::Div {
        div()
            .h(px(30.0))
            .px_3()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.14))
            .bg(white().opacity(0.05))
            .text_color(white().opacity(0.9))
            .hover(|s| s.bg(white().opacity(0.1)))
            .cursor_pointer()
            .flex()
            .items_center()
            .justify_center()
            .child(label.into())
    }

    fn normalize_motionloom_scene_script(script: &str) -> MotionLoomScriptNormalization {
        let (script, patched_fps, patched_size) = Self::ensure_motionloom_graph_defaults(script);
        let (script, patched_duration) = Self::ensure_motionloom_graph_duration(&script);
        let (script, patched_text_value) = Self::rewrite_text_value_compat(&script);
        let (script, patched_animate_opacity) = Self::rewrite_animate_opacity_compat(&script);
        MotionLoomScriptNormalization {
            script,
            patched_fps,
            patched_size,
            patched_duration,
            patched_text_value,
            patched_animate_opacity,
        }
    }

    fn ensure_motionloom_graph_defaults(script: &str) -> (String, bool, bool) {
        let Some(graph_start) = script.find("<Graph") else {
            return (script.to_string(), false, false);
        };
        let after = &script[graph_start..];
        let Some(rel_end) = after.find('>') else {
            return (script.to_string(), false, false);
        };
        let graph_end = graph_start + rel_end;
        let open_tag = &script[graph_start..=graph_end];
        let has_fps = open_tag.contains("fps=");
        let has_size = open_tag.contains("size=");
        if has_fps && has_size {
            return (script.to_string(), false, false);
        }

        let mut attrs = String::new();
        if !has_fps {
            attrs.push_str(" fps={60}");
        }
        if !has_size {
            attrs.push_str(" size={[1920,1080]}");
        }

        let patched_open = if let Some(prefix) = open_tag.strip_suffix("/>") {
            format!("{prefix}{attrs}/>")
        } else if let Some(prefix) = open_tag.strip_suffix('>') {
            format!("{prefix}{attrs}>")
        } else {
            return (script.to_string(), false, false);
        };

        let mut out = String::with_capacity(script.len() + attrs.len());
        out.push_str(&script[..graph_start]);
        out.push_str(&patched_open);
        out.push_str(&script[graph_end + 1..]);
        (out, !has_fps, !has_size)
    }

    fn find_tag_attr(block: &str, attr: &str) -> Option<String> {
        let pattern = [
            "(?is)\\b",
            &regex::escape(attr),
            "\\s*=\\s*(?:\"([^\"]*)\"|\\{([^}]*)\\}|([^\\s/>]+))",
        ]
        .concat();
        let re = Regex::new(&pattern).ok()?;
        let caps = re.captures(block)?;
        caps.get(1)
            .or_else(|| caps.get(2))
            .or_else(|| caps.get(3))
            .map(|m| m.as_str().trim().to_string())
    }

    fn remove_tag_attr(block: &str, attr: &str) -> String {
        let pattern = [
            "(?is)\\s",
            &regex::escape(attr),
            "\\s*=\\s*(?:\"[^\"]*\"|\\{[^}]*\\}|[^\\s/>]+)",
        ]
        .concat();
        match Regex::new(&pattern) {
            Ok(re) => re.replace_all(block, "").into_owned(),
            Err(_) => block.to_string(),
        }
    }

    fn format_motionloom_text_value_attr(value: &str) -> String {
        if !value.contains('"') {
            format!(r#""{value}""#)
        } else if !value.contains('}') && !value.contains('\n') {
            format!("{{{value}}}")
        } else {
            format!(r#""{}""#, value.replace('"', "'").replace('\n', " "))
        }
    }

    fn text_value_alias(block: &str) -> Option<String> {
        ["text", "content", "children", "label"]
            .iter()
            .find_map(|attr| Self::find_tag_attr(block, attr))
            .filter(|value| !value.trim().is_empty())
    }

    fn text_patch_label(tag: &str) -> String {
        Self::find_tag_attr(tag, "id")
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| "Text".to_string())
    }

    fn rewrite_text_tag_value(tag: &str, fallback_value: Option<&str>) -> Option<String> {
        if Self::find_tag_attr(tag, "value").is_some() {
            return None;
        }
        let value = Self::text_value_alias(tag).or_else(|| {
            fallback_value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })?;
        let value_attr = Self::format_motionloom_text_value_attr(&value);
        if let Some(close_ix) = tag.rfind("/>") {
            let head = tag[..close_ix].trim_end();
            Some(format!("{head} value={value_attr} />"))
        } else if let Some(close_ix) = tag.rfind('>') {
            let head = tag[..close_ix].trim_end();
            Some(format!("{head} value={value_attr} />"))
        } else {
            None
        }
    }

    fn rewrite_text_value_compat(script: &str) -> (String, Vec<String>) {
        let mut patched = Vec::<String>::new();
        let paired_re = match Regex::new(r#"(?is)<Text\b([^>]*)>(.*?)</Text>"#) {
            Ok(re) => re,
            Err(_) => return (script.to_string(), patched),
        };
        let paired_rewritten = paired_re
            .replace_all(script, |caps: &regex::Captures<'_>| {
                let full = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
                let attrs = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
                let body = caps.get(2).map(|m| m.as_str()).unwrap_or_default();
                let tag = format!("<Text{attrs}>");
                if let Some(rewritten) = Self::rewrite_text_tag_value(&tag, Some(body)) {
                    patched.push(Self::text_patch_label(&tag));
                    rewritten
                } else if Self::find_tag_attr(&tag, "value").is_some() {
                    format!("<Text{attrs} />")
                } else {
                    full.to_string()
                }
            })
            .into_owned();

        let self_closing_re = match Regex::new(r#"(?is)<Text\b[^>]*\/>"#) {
            Ok(re) => re,
            Err(_) => return (paired_rewritten, patched),
        };
        let rewritten = self_closing_re
            .replace_all(&paired_rewritten, |caps: &regex::Captures<'_>| {
                let full = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
                if let Some(rewritten) = Self::rewrite_text_tag_value(full, None) {
                    patched.push(Self::text_patch_label(full));
                    rewritten
                } else {
                    full.to_string()
                }
            })
            .into_owned();
        patched.sort();
        patched.dedup();
        (rewritten, patched)
    }

    fn ensure_motionloom_graph_duration(script: &str) -> (String, bool) {
        let Some(graph_start) = script.find("<Graph") else {
            return (script.to_string(), false);
        };
        let after = &script[graph_start..];
        let Some(rel_end) = after.find('>') else {
            return (script.to_string(), false);
        };
        let graph_end = graph_start + rel_end;
        let open_tag = &script[graph_start..=graph_end];
        if open_tag.contains("duration=") {
            return (script.to_string(), false);
        }

        let present_re = match Regex::new(r#"(?is)<Present\b([^>]*)\/>"#) {
            Ok(re) => re,
            Err(_) => return (script.to_string(), false),
        };
        let Some(caps) = present_re.captures(script) else {
            return (script.to_string(), false);
        };
        let attrs = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        let Some(duration_raw) = Self::find_tag_attr(attrs, "duration_ms") else {
            return (script.to_string(), false);
        };
        let Ok(duration_ms) = duration_raw.parse::<f64>() else {
            return (script.to_string(), false);
        };
        let duration_ms = duration_ms.max(1.0).round() as u64;
        let patched_open = if let Some(prefix) = open_tag.strip_suffix("/>") {
            format!(r#"{prefix} duration="{duration_ms}ms"/>"#)
        } else if let Some(prefix) = open_tag.strip_suffix('>') {
            format!(r#"{prefix} duration="{duration_ms}ms">"#)
        } else {
            return (script.to_string(), false);
        };

        let mut out = String::with_capacity(script.len() + 24);
        out.push_str(&script[..graph_start]);
        out.push_str(&patched_open);
        out.push_str(&script[graph_end + 1..]);
        (out, true)
    }

    fn parse_animate_opacity_compat(attrs: &str) -> Option<AnimateOpacityCompat> {
        let target = Self::find_tag_attr(attrs, "target")?;
        let target_id = target.strip_suffix(".opacity")?.trim().to_string();
        if target_id.is_empty() {
            return None;
        }
        let from = Self::find_tag_attr(attrs, "from")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0);
        let to = Self::find_tag_attr(attrs, "to")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(1.0);
        let start_ms = Self::find_tag_attr(attrs, "start_ms")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0);
        let end_ms = Self::find_tag_attr(attrs, "end_ms")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or((start_ms + 1000.0).max(1.0));
        Some(AnimateOpacityCompat {
            target_id,
            from,
            to,
            start_ms,
            end_ms,
        })
    }

    fn build_fade_expr_from_animate(spec: &AnimateOpacityCompat) -> String {
        let start_sec = (spec.start_ms / 1000.0).max(0.0);
        let duration_sec = ((spec.end_ms - spec.start_ms).abs() / 1000.0).max(0.001);
        format!(
            "{:.6}+({:.6}-{:.6})*min(max(($time.sec-{:.6})/{:.6},0),1)",
            spec.from, spec.to, spec.from, start_sec, duration_sec
        )
    }

    fn rewrite_tag_opacity(tag: &str, opacity_expr: &str) -> String {
        let cleaned = Self::remove_tag_attr(tag, "opacity");
        if let Some(close_ix) = cleaned.rfind("/>") {
            let head = cleaned[..close_ix].trim_end();
            return format!(r#"{head} opacity="{opacity_expr}" />"#);
        }
        cleaned
    }

    fn rewrite_animate_opacity_compat(script: &str) -> (String, Vec<String>) {
        let animate_re = match Regex::new(r#"(?is)<Animate\b([^>]*)\/>"#) {
            Ok(re) => re,
            Err(_) => return (script.to_string(), Vec::new()),
        };
        let mut opacity_by_id = HashMap::<String, String>::new();
        for caps in animate_re.captures_iter(script) {
            let attrs = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            if let Some(spec) = Self::parse_animate_opacity_compat(attrs) {
                opacity_by_id.insert(
                    spec.target_id.clone(),
                    Self::build_fade_expr_from_animate(&spec),
                );
            }
        }
        if opacity_by_id.is_empty() {
            return (script.to_string(), Vec::new());
        }

        let script_without_animate = animate_re.replace_all(script, "").into_owned();
        let node_re = match Regex::new(r#"(?is)<(Text|Image|Svg|SVG)\b[^>]*\/>"#) {
            Ok(re) => re,
            Err(_) => return (script_without_animate, Vec::new()),
        };
        let mut patched_ids = Vec::<String>::new();
        let rewritten = node_re
            .replace_all(&script_without_animate, |caps: &regex::Captures<'_>| {
                let full = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
                let Some(id) = Self::find_tag_attr(full, "id") else {
                    return full.to_string();
                };
                let Some(expr) = opacity_by_id.get(&id) else {
                    return full.to_string();
                };
                patched_ids.push(id);
                Self::rewrite_tag_opacity(full, expr)
            })
            .into_owned();
        patched_ids.sort();
        patched_ids.dedup();
        (rewritten, patched_ids)
    }

    fn append_motionloom_normalization_message(
        mut base: String,
        normalization: &MotionLoomScriptNormalization,
    ) -> String {
        let mut parts = Vec::<String>::new();
        if normalization.patched_fps {
            parts.push("fps=60".to_string());
        }
        if normalization.patched_size {
            parts.push("size=[1920,1080]".to_string());
        }
        if normalization.patched_duration {
            parts.push("duration from Present.duration_ms".to_string());
        }
        if !normalization.patched_text_value.is_empty() {
            parts.push(format!(
                "Text value for {}",
                normalization.patched_text_value.join(",")
            ));
        }
        if !normalization.patched_animate_opacity.is_empty() {
            parts.push(format!(
                "Animate->opacity for {}",
                normalization.patched_animate_opacity.join(",")
            ));
        }
        if !parts.is_empty() {
            base.push_str(" Auto-normalized: ");
            base.push_str(&parts.join(", "));
            base.push('.');
        }
        base
    }

    fn media_pool_signature(
        items: &[MediaPoolItem],
        ffmpeg_available: bool,
        ffprobe_available: bool,
    ) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        items.len().hash(&mut hasher);
        ffmpeg_available.hash(&mut hasher);
        ffprobe_available.hash(&mut hasher);
        for item in items {
            item.path.hash(&mut hasher);
            item.name.hash(&mut hasher);
            item.duration.as_millis().hash(&mut hasher);
        }
        hasher.finish()
    }

    fn sync_worker_media_pool_snapshot(&mut self, cx: &mut Context<Self>) {
        let (media_pool, ffmpeg_available, ffprobe_available, ffprobe_command) = {
            let gs = self.global.read(cx);
            (
                gs.media_pool.clone(),
                gs.media_dependency.ffmpeg_available,
                gs.media_dependency.ffprobe_available,
                gs.ffprobe_path.clone(),
            )
        };
        let sig = Self::media_pool_signature(&media_pool, ffmpeg_available, ffprobe_available);
        if sig == self.media_pool_signature {
            return;
        }
        self.media_pool_signature = sig;
        self.worker.update_media_pool_snapshot(
            media_pool,
            ffmpeg_available,
            ffprobe_available,
            ffprobe_command,
        );
    }

    fn current_workspace_root(&self, cx: &mut Context<Self>) -> PathBuf {
        resolve_workspace_root(self.global.read(cx).project_file_path.as_deref())
    }

    fn acp_runtime_cwd(&self, cx: &mut Context<Self>) -> PathBuf {
        let workspace_root = self.current_workspace_root(cx);
        let api_root = workspace_root.join("src").join("api");
        let repo_api_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("api");

        // Keep ACP chat scoped to API sources so runtime guidance stays separate from coding-only folders.
        if api_root.is_dir() {
            api_root
        } else if repo_api_root.is_dir() {
            repo_api_root
        } else {
            workspace_root
        }
    }

    fn acp_docs_dir(&self, cx: &mut Context<Self>) -> PathBuf {
        let workspace_docs = self.current_workspace_root(cx).join("docs").join("acp");
        if workspace_docs.is_dir() {
            workspace_docs
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("docs")
                .join("acp")
        }
    }

    fn on_toggle_auto_connect(&mut self, cx: &mut Context<Self>) {
        let next_value = !self.auto_connect_enabled;
        self.auto_connect_enabled = next_value;

        let workspace_root = self.current_workspace_root(cx);
        match save_auto_connect(self.auto_connect_scope, &workspace_root, next_value) {
            Ok(path) => {
                self.push_system_message(
                    format!(
                        "Auto Connect {} (saved to {} settings: {}).",
                        if next_value { "enabled" } else { "disabled" },
                        self.auto_connect_scope.label(),
                        path.display()
                    ),
                    cx,
                );
            }
            Err(err) => {
                self.push_system_message(format!("Failed to save Auto Connect setting: {err}"), cx);
            }
        }

        cx.notify();
    }

    fn on_save_cli_paths(&mut self, cx: &mut Context<Self>) {
        let workspace_root = self.current_workspace_root(cx);
        let saved_provider = self.agent_provider;
        let active_override = match saved_provider {
            AcpAgentProvider::Codex => normalize_cli_override(&self.codex_cli_bin),
            AcpAgentProvider::Gemini => normalize_cli_override(&self.gemini_cli_bin),
            AcpAgentProvider::Claude => normalize_cli_override(&self.claude_cli_bin),
        };
        match save_acp_cli_paths(
            SettingsScope::User,
            &workspace_root,
            Some(&self.codex_cli_bin),
            Some(&self.gemini_cli_bin),
            Some(&self.claude_cli_bin),
        ) {
            Ok(path) => {
                let action_summary = if active_override.is_some() {
                    format!("Saved {} CLI path override", saved_provider.label())
                } else {
                    format!(
                        "Cleared {} CLI path override and reverted to auto-detect/default path",
                        saved_provider.label()
                    )
                };
                self.push_system_message(
                    format!("{action_summary} in user settings: {}.", path.display()),
                    cx,
                );
                self.refresh_agent_status();
            }
            Err(err) => {
                self.push_system_message(format!("Failed to save CLI path overrides: {err}"), cx);
            }
        }

        cx.notify();
    }

    fn chat_bubble(
        msg: &AiChatMessage,
        bubble_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let (title, border, bg): (&str, gpui::Hsla, gpui::Hsla) = match msg.role {
            AiChatRole::User => ("You", rgb(0x2563eb).into(), rgb(0x172554).into()),
            AiChatRole::Assistant => ("Agent", rgb(0x10b981).into(), rgb(0x052e2b).into()),
            AiChatRole::System => ("System", rgb(0xf59e0b).into(), rgb(0x3f2a05).into()),
        };

        let text = if msg.pending && msg.text.trim().is_empty() {
            "...".to_string()
        } else {
            msg.text.clone()
        };
        let copy_text = text.clone();

        div()
            .w_full()
            .rounded_md()
            .border_1()
            .border_color(border.opacity(0.55))
            .bg(bg.opacity(0.45))
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_xs().text_color(border.opacity(0.9)).child(title))
                    .child(
                        div()
                            .h(px(20.0))
                            .px_2()
                            .rounded_md()
                            .border_1()
                            .border_color(white().opacity(0.18))
                            .bg(white().opacity(0.04))
                            .text_xs()
                            .text_color(white().opacity(0.72))
                            .hover(|s| s.bg(white().opacity(0.09)))
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child("Copy")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |_this, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        copy_text.clone(),
                                    ));
                                }),
                            ),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.92))
                    .whitespace_normal()
                    .child(
                        TextView::markdown(("ai-agents-msg-body", bubble_index), text, window, cx)
                            .selectable(true)
                            .w_full(),
                    ),
            )
    }

    fn push_chat_message(
        &mut self,
        role: AiChatRole,
        text: impl Into<String>,
        pending: bool,
        cx: &mut Context<Self>,
    ) -> usize {
        let text = text.into();
        let mut removed_prefix = 0usize;
        let mut remaining_len = 0usize;
        let mut idx = 0usize;
        self.global.update(cx, |gs, cx| {
            gs.ai_chat_messages.push(AiChatMessage {
                role,
                text: text.clone(),
                pending,
            });
            removed_prefix = prune_ai_chat_history(&mut gs.ai_chat_messages);
            remaining_len = gs.ai_chat_messages.len();
            idx = gs.ai_chat_messages.len().saturating_sub(1);
            cx.notify();
        });
        self.active_assistant_idx =
            rebase_ai_chat_index(self.active_assistant_idx, removed_prefix, remaining_len);
        idx
    }

    fn push_system_message(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        let text = text.into();
        let active_idx_before = self.active_assistant_idx;
        let mut inserted_before_assistant = false;
        let mut removed_prefix = 0usize;
        let mut remaining_len = 0usize;

        self.global.update(cx, |gs, cx| {
            if let Some(active_idx) = active_idx_before
                && let Some(active_msg) = gs.ai_chat_messages.get(active_idx)
                && matches!(active_msg.role, AiChatRole::Assistant)
                && active_msg.pending
            {
                gs.ai_chat_messages.insert(
                    active_idx,
                    AiChatMessage {
                        role: AiChatRole::System,
                        text: text.clone(),
                        pending: false,
                    },
                );
                inserted_before_assistant = true;
            } else {
                gs.ai_chat_messages.push(AiChatMessage {
                    role: AiChatRole::System,
                    text: text.clone(),
                    pending: false,
                });
            }

            removed_prefix = prune_ai_chat_history(&mut gs.ai_chat_messages);
            remaining_len = gs.ai_chat_messages.len();
            cx.notify();
        });

        let rebased_active = if inserted_before_assistant {
            active_idx_before.map(|active_idx| active_idx.saturating_add(1))
        } else {
            self.active_assistant_idx
        };
        self.active_assistant_idx =
            rebase_ai_chat_index(rebased_active, removed_prefix, remaining_len);
    }

    fn ensure_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.command_input.is_none() {
            self.rebuild_command_input(window, cx);
        }

        if self.codex_cli_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Optional absolute path to codex executable")
            });
            let current = self.codex_cli_bin.clone();
            input.update(cx, |this, cx| {
                this.set_value(current.clone(), window, cx);
            });

            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                this.codex_cli_bin = input.read(cx).value().to_string();
                if this.agent_provider == AcpAgentProvider::Codex {
                    this.refresh_agent_status();
                    cx.notify();
                }
            });

            self.codex_cli_input = Some(input);
            self.codex_cli_input_sub = Some(sub);
        }

        if self.gemini_cli_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Optional absolute path to gemini executable")
            });
            let current = self.gemini_cli_bin.clone();
            input.update(cx, |this, cx| {
                this.set_value(current.clone(), window, cx);
            });

            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                this.gemini_cli_bin = input.read(cx).value().to_string();
                if this.agent_provider == AcpAgentProvider::Gemini {
                    this.refresh_agent_status();
                    cx.notify();
                }
            });

            self.gemini_cli_input = Some(input);
            self.gemini_cli_input_sub = Some(sub);
        }

        if self.claude_cli_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Optional absolute path to claude executable")
            });
            let current = self.claude_cli_bin.clone();
            input.update(cx, |this, cx| {
                this.set_value(current.clone(), window, cx);
            });

            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                this.claude_cli_bin = input.read(cx).value().to_string();
                if this.agent_provider == AcpAgentProvider::Claude {
                    this.refresh_agent_status();
                    cx.notify();
                }
            });

            self.claude_cli_input = Some(input);
            self.claude_cli_input_sub = Some(sub);
        }

        if self.gemini_key_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("GEMINI_API_KEY (optional, forwarded on connect)")
            });
            let current = self.gemini_api_key.clone();
            input.update(cx, |this, cx| {
                this.set_value(current.clone(), window, cx);
            });

            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                this.gemini_api_key = input.read(cx).value().to_string();
                if this.agent_provider == AcpAgentProvider::Gemini {
                    this.refresh_agent_status();
                    cx.notify();
                }
            });

            self.gemini_key_input = Some(input);
            self.gemini_key_input_sub = Some(sub);
        }

        if self.prompt_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx).placeholder("Type a message to the AI agent")
            });
            let sub = cx.subscribe(&input, |this, input, ev, cx| match ev {
                InputEvent::Change => {
                    this.prompt_text = input.read(cx).value().to_string();
                }
                InputEvent::PressEnter { .. } => {
                    this.prompt_text = input.read(cx).value().to_string();
                    this.prompt_send_on_next_render = true;
                    cx.notify();
                }
                _ => {}
            });

            self.prompt_input = Some(input);
            self.prompt_input_sub = Some(sub);
        }
    }

    fn sync_input_entity_value(
        input: &Entity<InputState>,
        desired: &str,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let should_sync = {
            let state = input.read(cx);
            let focused = state.focus_handle(cx).is_focused(window);
            let current = state.value().to_string();
            force || (!focused && current != desired)
        };

        if should_sync {
            let desired = desired.to_string();
            input.update(cx, |this, cx| {
                this.set_value(desired.clone(), window, cx);
            });
        }
    }

    fn sync_visible_input_values(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let force = self.pending_input_resync;

        if let Some(input) = self.command_input.as_ref() {
            Self::sync_input_entity_value(input, &self.agent_command, force, window, cx);
        }

        match self.agent_provider {
            AcpAgentProvider::Codex => {
                if let Some(input) = self.codex_cli_input.as_ref() {
                    Self::sync_input_entity_value(input, &self.codex_cli_bin, force, window, cx);
                }
            }
            AcpAgentProvider::Gemini => {
                if let Some(input) = self.gemini_key_input.as_ref() {
                    Self::sync_input_entity_value(input, &self.gemini_api_key, force, window, cx);
                }
                if let Some(input) = self.gemini_cli_input.as_ref() {
                    Self::sync_input_entity_value(input, &self.gemini_cli_bin, force, window, cx);
                }
            }
            AcpAgentProvider::Claude => {
                if let Some(input) = self.claude_cli_input.as_ref() {
                    Self::sync_input_entity_value(input, &self.claude_cli_bin, force, window, cx);
                }
            }
        }

        self.pending_input_resync = false;
    }

    fn clear_prompt_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_text.clear();
        if let Some(input) = self.prompt_input.as_ref() {
            input.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
        }
    }

    fn ensure_worker_poller(&mut self, cx: &mut Context<Self>) {
        if self.poll_running {
            return;
        }

        self.poll_running = true;
        self.poll_token = self.poll_token.wrapping_add(1);
        let token = self.poll_token;

        cx.spawn(async move |view, cx| {
            loop {
                Timer::after(Duration::from_millis(120)).await;
                let keep = view
                    .update(cx, |this, cx| {
                        if this.poll_token != token {
                            this.poll_running = false;
                            return false;
                        }

                        if this.drain_worker_events(cx) {
                            cx.notify();
                        }

                        true
                    })
                    .unwrap_or(false);

                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    fn append_assistant_chunk(&mut self, chunk: String, cx: &mut Context<Self>) {
        if chunk.is_empty() {
            return;
        }

        if let Some(idx) = self.active_assistant_idx {
            let mut appended = false;
            self.global.update(cx, |gs, cx| {
                if let Some(msg) = gs.ai_chat_messages.get_mut(idx) {
                    msg.text.push_str(&chunk);
                    appended = true;
                    cx.notify();
                }
            });
            if appended {
                return;
            }
        }

        let idx = self.push_chat_message(AiChatRole::Assistant, chunk, true, cx);
        self.active_assistant_idx = Some(idx);
    }

    fn mark_assistant_done(&mut self, cx: &mut Context<Self>) {
        if let Some(idx) = self.active_assistant_idx.take() {
            self.global.update(cx, |gs, cx| {
                if let Some(msg) = gs.ai_chat_messages.get_mut(idx) {
                    msg.pending = false;
                    cx.notify();
                }
            });
        }
    }

    fn start_export_from_acp_request(
        &mut self,
        request: AcpExportRunRequest,
        cx: &mut Context<Self>,
    ) -> Result<AcpExportRunResponse, String> {
        let tools_ready = self.global.read(cx).media_tools_ready_for_export();
        if !tools_ready {
            self.global.update(cx, |gs, cx| {
                gs.show_media_dependency_modal();
                gs.ui_notice = Some(
                    "ACP export requires FFmpeg and FFprobe. Install tools first.".to_string(),
                );
                cx.notify();
            });
            self.push_system_message(
                "ACP export blocked: missing FFmpeg/FFprobe. Install tools and retry.".to_string(),
                cx,
            );
            return Err(
                "MISSING_FFMPEG_FFPROBE: anica.export/run requires ffmpeg and ffprobe. Install FFmpeg package and retry."
                    .to_string(),
            );
        }

        let resolved = {
            let gs = self.global.read(cx);
            resolve_acp_export_run_request(request, gs).map_err(|err| err.to_string())?
        };
        let out_path = resolved.out_path.clone();
        let mode_id = resolved.export_mode.id().to_string();
        let preset_id = resolved.export_preset.id().to_string();
        let export_resolution = resolved.export_resolution.clone();
        let layout_resolution = resolved.layout_resolution.clone();
        let resolution_source = resolved.resolution_source.clone();
        let export_width = resolved.export_w.round().max(1.0) as u32;
        let export_height = resolved.export_h.round().max(1.0) as u32;

        self.push_system_message(
            format!(
                "ACP export started: {} [{} | layout {}]",
                out_path, export_resolution, layout_resolution
            ),
            cx,
        );
        self.busy = true;

        let cancel_signal = Arc::new(AtomicBool::new(false));
        self.global.update(cx, |gs, cx| {
            gs.export_begin(out_path.clone(), resolved.export_total);
            cx.notify();
        });

        enum AcpExportWorkerEvent {
            Progress(crate::core::export::ExportProgress),
            Finished(Result<String, String>),
        }

        let (tx, rx) = mpsc::channel::<AcpExportWorkerEvent>();
        let out_path_for_thread = resolved.out_path.clone();
        let ffmpeg_path = resolved.ffmpeg_path.clone();
        std::thread::spawn(move || {
            let tx_progress = tx.clone();
            let result = FfmpegExporter::export(
                &ffmpeg_path,
                &resolved.v1,
                &resolved.audio_tracks,
                &resolved.video_tracks,
                &resolved.subtitle_tracks,
                &resolved.subtitle_groups,
                &out_path_for_thread,
                resolved.layout_canvas_w,
                resolved.layout_canvas_h,
                resolved.export_w,
                resolved.export_h,
                resolved.layer_effects,
                &resolved.layer_effect_clips,
                resolved.export_color_mode,
                resolved.export_mode,
                resolved.export_preset,
                resolved.export_settings,
                resolved.export_range,
                cancel_signal,
                move |progress| {
                    let _ = tx_progress.send(AcpExportWorkerEvent::Progress(progress));
                },
            );
            let result = match result {
                Ok(_) => Ok(out_path_for_thread),
                Err(err) => {
                    eprintln!("[ACP Export] Failed: {err}");
                    Err(err.to_string())
                }
            };
            let _ = tx.send(AcpExportWorkerEvent::Finished(result));
        });

        let global_for_finish = self.global.clone();
        cx.spawn(async move |view, cx| {
            loop {
                Timer::after(Duration::from_millis(120)).await;

                let mut latest_progress = None;
                let mut finished = None;
                loop {
                    match rx.try_recv() {
                        Ok(AcpExportWorkerEvent::Progress(p)) => latest_progress = Some(p),
                        Ok(AcpExportWorkerEvent::Finished(result)) => {
                            finished = Some(result);
                            break;
                        }
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            finished = Some(Err("Export worker disconnected.".to_string()));
                            break;
                        }
                    }
                }

                let has_finished = finished.is_some();
                let updated = view.update(cx, |this, cx| {
                    let mut chat_notice: Option<String> = None;
                    global_for_finish.update(cx, |gs, cx| {
                        if let Some(p) = latest_progress {
                            gs.export_update_progress(p.rendered, p.total, p.speed);
                        }
                        if let Some(result) = finished {
                            match result {
                                Ok(path) => {
                                    gs.export_done();
                                    gs.export_last_out_path = Some(path.clone());
                                    gs.ui_notice = Some(format!("Export saved: {}", path));
                                    chat_notice = Some(format!("ACP export saved: {}", path));
                                }
                                Err(err) => {
                                    if is_cancelled_export_error(&err) {
                                        gs.export_cancelled();
                                        gs.ui_notice = Some("Export stopped.".to_string());
                                        chat_notice = Some("ACP export stopped.".to_string());
                                    } else {
                                        gs.export_fail(err.clone());
                                        chat_notice = Some(format!("ACP export failed: {err}"));
                                    }
                                }
                            }
                            this.busy = false;
                        }
                        cx.notify();
                    });
                    if let Some(msg) = chat_notice {
                        this.push_system_message(msg, cx);
                    }
                });

                if has_finished || updated.is_err() {
                    break;
                }
            }
        })
        .detach();

        let response_message = format!(
            "Export job started at {export_resolution} (layout {layout_resolution}, source {resolution_source})."
        );
        Ok(AcpExportRunResponse {
            ok: true,
            started: true,
            mode: mode_id,
            preset: preset_id,
            out_path: Some(out_path),
            layout_resolution,
            export_resolution: export_resolution.clone(),
            export_width,
            export_height,
            resolution_source: resolution_source.clone(),
            message: response_message,
        })
    }

    fn drain_worker_events(&mut self, cx: &mut Context<Self>) -> bool {
        let mut changed = false;

        while let Some(ev) = self.worker.try_recv() {
            changed = true;
            match ev {
                AcpUiEvent::Status(line) => {
                    self.last_status = line.clone();
                    let status_for_chat = if let Some(raw) = line.strip_prefix("ACP_STATUS:") {
                        Some(raw.trim().to_string())
                    } else if line.starts_with("Tool call:")
                        || line.contains("timed out")
                        || line.to_lowercase().contains("error")
                    {
                        Some(line.clone())
                    } else {
                        None
                    };
                    if let Some(status_text) = status_for_chat
                        && !status_text.is_empty()
                        && status_text != self.last_chat_status
                    {
                        self.last_chat_status = status_text.clone();
                        self.push_system_message(status_text, cx);
                    }
                }
                AcpUiEvent::Connected {
                    session_id,
                    agent_label,
                } => {
                    self.connected = true;
                    self.connected_provider = Some(self.agent_provider);
                    self.busy = false;
                    self.last_status = format!("Connected ({agent_label})");
                    self.push_system_message(
                        format!("Connected to agent. Session: {session_id}"),
                        cx,
                    );
                }
                AcpUiEvent::Disconnected { reason } => {
                    self.connected = false;
                    self.connected_provider = None;
                    self.busy = false;
                    self.mark_assistant_done(cx);
                    self.last_status = reason.clone();
                    self.push_system_message(reason, cx);
                }
                AcpUiEvent::AssistantChunk(chunk) => {
                    self.append_assistant_chunk(chunk, cx);
                }
                AcpUiEvent::PromptFinished { stop_reason } => {
                    self.busy = false;
                    self.mark_assistant_done(cx);
                    self.last_status = format!("Prompt finished: {stop_reason}");
                }
                AcpUiEvent::Error(err) => {
                    self.busy = false;
                    self.mark_assistant_done(cx);
                    self.last_status = format!("Error: {err}");
                    self.push_system_message(format!("Error: {err}"), cx);
                    if err.to_lowercase().contains("codex cli not found")
                        && self.agent_provider == AcpAgentProvider::Codex
                    {
                        self.push_system_message(codex_install_hint(), cx);
                    }
                    if err.to_lowercase().contains("gemini")
                        && self.agent_provider == AcpAgentProvider::Gemini
                    {
                        self.push_system_message(gemini_install_hint(), cx);
                    }
                    if err.to_lowercase().contains("claude")
                        && self.agent_provider == AcpAgentProvider::Claude
                    {
                        self.push_system_message(claude_install_hint(), cx);
                    }
                }
                AcpUiEvent::ToolBridgeRequest(request) => match request {
                    AcpToolBridgeRequest::LlmDecisionMakingSrtSimilarSerach {
                        request,
                        reply_tx,
                    } => {
                        let response = llm_decision_making_srt_similar_serach(request);
                        let _ = reply_tx.send(Ok(response));
                    }
                    AcpToolBridgeRequest::BuildAutonomousEditPlan { request, reply_tx } => {
                        let response = {
                            let gs = self.global.read(cx);
                            build_autonomous_edit_plan(gs, request)
                        };
                        let _ = reply_tx.send(Ok(response));
                    }
                    AcpToolBridgeRequest::GetTimelineSnapshot { request, reply_tx } => {
                        let response = {
                            let gs = self.global.read(cx);
                            get_timeline_snapshot(gs, request)
                        };
                        let _ = reply_tx.send(Ok(response));
                    }
                    AcpToolBridgeRequest::GetAudioSilenceMap { request, reply_tx } => {
                        let response = {
                            let gs = self.global.read(cx);
                            get_audio_silence_map(gs, request)
                        };
                        // Show silence preview modal so user can pick which ranges to cut.
                        // Candidates with confidence data are shown as checkbox rows.
                        if !response.cut_candidates.is_empty() {
                            let modal_state = SilencePreviewModalState {
                                timeline_revision: response.timeline_revision.clone(),
                                candidates: response
                                    .cut_candidates
                                    .iter()
                                    .map(|c| SilencePreviewCandidate {
                                        start_ms: c.start_ms,
                                        end_ms: c.end_ms,
                                        confidence: c.confidence,
                                        reason: c.reason.clone(),
                                        selected: true, // pre-select all by default
                                    })
                                    .collect(),
                            };
                            self.global.update(cx, |gs, cx| {
                                gs.show_silence_preview_modal(modal_state);
                                cx.notify();
                            });
                        }
                        let _ = reply_tx.send(Ok(response));
                    }
                    AcpToolBridgeRequest::BuildAudioSilenceCutPlan { request, reply_tx } => {
                        let response = {
                            let gs = self.global.read(cx);
                            build_audio_silence_cut_plan(gs, request)
                        };
                        let _ = reply_tx.send(Ok(response));
                    }
                    AcpToolBridgeRequest::GetTranscriptLowConfidenceMap { request, reply_tx } => {
                        let response = {
                            let gs = self.global.read(cx);
                            get_transcript_low_confidence_map(gs, request)
                        };
                        let _ = reply_tx.send(Ok(response));
                    }
                    AcpToolBridgeRequest::BuildTranscriptLowConfidenceCutPlan {
                        request,
                        reply_tx,
                    } => {
                        let response = {
                            let gs = self.global.read(cx);
                            build_transcript_low_confidence_cut_plan(gs, request)
                        };
                        let _ = reply_tx.send(Ok(response));
                    }
                    AcpToolBridgeRequest::GetSubtitleGapMap { request, reply_tx } => {
                        let response = {
                            let gs = self.global.read(cx);
                            get_subtitle_gap_map(gs, request)
                        };
                        let _ = reply_tx.send(Ok(response));
                    }
                    AcpToolBridgeRequest::BuildSubtitleGapCutPlan { request, reply_tx } => {
                        let response = {
                            let gs = self.global.read(cx);
                            build_subtitle_gap_cut_plan(gs, request)
                        };
                        let _ = reply_tx.send(Ok(response));
                    }
                    AcpToolBridgeRequest::GetSubtitleSemanticRepeats { request, reply_tx } => {
                        let response = {
                            let gs = self.global.read(cx);
                            get_subtitle_semantic_repeats(gs, request)
                        };
                        let _ = reply_tx.send(Ok(response));
                    }
                    AcpToolBridgeRequest::ValidateEditPlan { request, reply_tx } => {
                        let response = {
                            let gs = self.global.read(cx);
                            validate_edit_plan(gs, &request)
                        };
                        let _ = reply_tx.send(Ok(response));
                    }
                    AcpToolBridgeRequest::ApplyEditPlan { request, reply_tx } => {
                        let mut applied_ok = false;
                        let response = self.global.update(cx, |gs, cx| {
                            let result = apply_edit_plan(gs, &request);
                            if result.ok {
                                applied_ok = true;
                                cx.notify();
                            }
                            result
                        });
                        let _ = reply_tx.send(Ok(response));
                        if applied_ok {
                            self.sync_worker_media_pool_snapshot(cx);
                        }
                    }
                    AcpToolBridgeRequest::RunExport { request, reply_tx } => {
                        let response = self.start_export_from_acp_request(request, cx);
                        let _ = reply_tx.send(response);
                    }
                    AcpToolBridgeRequest::RemoveMediaPoolById { request, reply_tx } => {
                        let mut removed = false;
                        let response = self.global.update(cx, |gs, cx| {
                            let result = remove_media_pool_by_id(gs, request);
                            if result.removed {
                                removed = true;
                                cx.emit(MediaPoolUiEvent::StateChanged);
                                cx.notify();
                            }
                            result
                        });
                        let _ = reply_tx.send(Ok(response));
                        if removed {
                            self.sync_worker_media_pool_snapshot(cx);
                        }
                    }
                    AcpToolBridgeRequest::ClearMediaPool { request, reply_tx } => {
                        let mut removed_count = 0usize;
                        let response = self.global.update(cx, |gs, cx| {
                            let result = clear_media_pool(gs, request);
                            removed_count = result.removed_count;
                            if result.removed_count > 0 {
                                cx.emit(MediaPoolUiEvent::StateChanged);
                                cx.notify();
                            }
                            result
                        });
                        let _ = reply_tx.send(Ok(response));
                        if removed_count > 0 {
                            self.sync_worker_media_pool_snapshot(cx);
                        }
                    }
                    AcpToolBridgeRequest::GetMotionLoomSceneScript { request, reply_tx } => {
                        let _ = request;
                        let response = {
                            let gs = self.global.read(cx);
                            let script = gs.motionloom_scene_script().to_string();
                            AcpMotionLoomGetSceneScriptResponse {
                                ok: true,
                                script_length: script.chars().count(),
                                script,
                                script_revision: gs.motionloom_scene_script_revision(),
                                apply_revision: gs.motionloom_scene_apply_revision(),
                                message: "Fetched MotionLoom scene script.".to_string(),
                            }
                        };
                        let _ = reply_tx.send(Ok(response));
                    }
                    AcpToolBridgeRequest::SetMotionLoomSceneScript { request, reply_tx } => {
                        let script = request.script;
                        let apply_now = request.apply_now;
                        let focus_vfx_page = request.focus_vfx_page;
                        let normalization = Self::normalize_motionloom_scene_script(&script);
                        let trimmed = normalization.script.trim().to_string();
                        if trimmed.is_empty() {
                            let _ = reply_tx.send(Err(
                                "motionloom/set_scene_script requires a non-empty <Graph ...> script."
                                    .to_string(),
                            ));
                            continue;
                        }
                        if !is_graph_script(&trimmed) {
                            let _ = reply_tx.send(Err(
                                "motionloom/set_scene_script expects MotionLoom Graph DSL (<Graph ...>...</Graph>)."
                                    .to_string(),
                            ));
                            continue;
                        }
                        let graph = match parse_graph_script(&trimmed) {
                            Ok(graph) => graph,
                            Err(err) => {
                                let _ = reply_tx.send(Err(format!(
                                    "motionloom/set_scene_script parse error at line {}: {}",
                                    err.line, err.message
                                )));
                                continue;
                            }
                        };
                        let runtime = match compile_runtime_program(graph.clone()) {
                            Ok(runtime) => runtime,
                            Err(err) => {
                                let _ = reply_tx.send(Err(format!(
                                    "motionloom/set_scene_script compile error: {}",
                                    err.message
                                )));
                                continue;
                            }
                        };
                        if !runtime.unsupported_kernels().is_empty() {
                            let _ = reply_tx.send(Err(format!(
                                "motionloom/set_scene_script unsupported kernel(s): {}",
                                runtime.unsupported_kernels().join(", ")
                            )));
                            continue;
                        }

                        let response = self.global.update(cx, |gs, cx| {
                            let script_length = trimmed.chars().count();
                            let (updated, apply_requested) =
                                gs.set_motionloom_scene_script(trimmed.clone(), apply_now);
                            if focus_vfx_page {
                                gs.active_page = AppPage::MotionLoom;
                            }
                            cx.notify();
                            AcpMotionLoomSetSceneScriptResponse {
                                ok: true,
                                updated,
                                apply_requested,
                                focus_vfx_page,
                                script_length,
                                script_revision: gs.motionloom_scene_script_revision(),
                                apply_revision: gs.motionloom_scene_apply_revision(),
                                message: if updated {
                                    if apply_requested {
                                        let base = format!(
                                            "MotionLoom script updated, validated (scope={:?}), and apply requested.",
                                            graph.scope
                                        );
                                        Self::append_motionloom_normalization_message(
                                            base,
                                            &normalization,
                                        )
                                    } else {
                                        let base = format!(
                                            "MotionLoom script updated and validated (scope={:?}).",
                                            graph.scope
                                        );
                                        Self::append_motionloom_normalization_message(
                                            base,
                                            &normalization,
                                        )
                                    }
                                } else if apply_requested {
                                    "MotionLoom script unchanged, apply requested (validated)."
                                        .to_string()
                                } else {
                                    "MotionLoom script unchanged (validated).".to_string()
                                },
                            }
                        });
                        let _ = reply_tx.send(Ok(response));
                    }
                    AcpToolBridgeRequest::RenderMotionLoomScene { request, reply_tx } => {
                        let mode = request.mode.trim().to_ascii_lowercase();
                        let focus_vfx_page = request.focus_vfx_page;

                        let current_script = {
                            let gs = self.global.read(cx);
                            gs.motionloom_scene_script().to_string()
                        };
                        if current_script.trim().is_empty() {
                            let _ = reply_tx.send(Err(
                                "motionloom/render_scene requires a scene script. Set script first."
                                    .to_string(),
                            ));
                            continue;
                        }
                        let graph = match parse_graph_script(&current_script) {
                            Ok(graph) => graph,
                            Err(err) => {
                                let _ = reply_tx.send(Err(format!(
                                    "motionloom/render_scene parse error at line {}: {}",
                                    err.line, err.message
                                )));
                                continue;
                            }
                        };
                        if graph.scope != GraphScope::Scene {
                            let _ = reply_tx.send(Err(
                                "motionloom/render_scene requires <Graph scope=\"scene\" ...>."
                                    .to_string(),
                            ));
                            continue;
                        }
                        if !graph.has_scene_nodes() {
                            let _ = reply_tx.send(Err(
                                "motionloom/render_scene requires at least one scene node: <Scene>, <Solid>, <Text>, <Image>, <Svg>, <Rect>, <Circle>, <Line>, <Polyline>, <Path>, <Camera>, or <Group>."
                                    .to_string(),
                            ));
                            continue;
                        }

                        let response = match self.global.update(cx, |gs, cx| {
                            let revision = gs.request_motionloom_scene_render(&mode)?;
                            if focus_vfx_page {
                                gs.active_page = AppPage::MotionLoom;
                            }
                            cx.notify();
                            Ok::<AcpMotionLoomRenderSceneResponse, String>(
                                AcpMotionLoomRenderSceneResponse {
                                    ok: true,
                                    queued: true,
                                    mode: gs
                                        .motionloom_scene_render_mode()
                                        .unwrap_or("gpu")
                                        .to_string(),
                                    focus_vfx_page,
                                    render_revision: revision,
                                    message: "MotionLoom render queued. VFX page will execute matching render button flow."
                                        .to_string(),
                                },
                            )
                        }) {
                            Ok(response) => response,
                            Err(err) => {
                                let _ = reply_tx.send(Err(err));
                                continue;
                            }
                        };
                        let _ = reply_tx.send(Ok(response));
                    }
                },
            }
        }

        changed
    }

    // Pure disconnect — stops the agent, no reconnect.
    fn on_disconnect(&mut self, cx: &mut Context<Self>) {
        if !self.connected {
            return;
        }
        self.worker.disconnect();
        self.connected = false;
        self.connected_provider = None;
        self.busy = false;
        self.last_status = "Disconnecting...".to_string();
        cx.notify();
    }

    // Connect to the currently selected provider.
    // If already connected to a different provider, disconnect first then connect.
    fn on_connect(&mut self, cx: &mut Context<Self>) {
        if self.connected {
            self.worker.disconnect();
            self.connected = false;
            self.connected_provider = None;
            self.busy = false;
            self.push_system_message(
                format!(
                    "Disconnected previous provider. Connecting to {}...",
                    self.agent_provider.label()
                ),
                cx,
            );
        }

        let cmd = self.agent_command.trim().to_string();
        if cmd.is_empty() {
            self.push_system_message("Agent command is empty.", cx);
            cx.notify();
            return;
        }
        let resolved_cmd = resolve_agent_command_for_spawn(&cmd);
        if self.agent_provider == AcpAgentProvider::Gemini
            && !has_gemini_api_key(&self.gemini_api_key)
            && !matches!(self.agent_status, AgentLoginStatus::LoggedIn { .. })
        {
            self.push_system_message(
                "No API key detected. Trying Gemini CLI auth state. If connect fails, run `gemini` then `/auth`, or set GEMINI_API_KEY."
                    .to_string(),
                cx,
            );
        }

        // Force ACP session cwd to API folder so runtime AGENTS policy is applied first.
        let runtime_cwd = self.acp_runtime_cwd(cx);
        let acp_docs_dir = self.acp_docs_dir(cx);
        let api_scoped = runtime_cwd.ends_with("src/api");
        let docs_scoped = acp_docs_dir.is_dir();

        self.last_status = format!(
            "Connecting with: {resolved_cmd} (cwd: {})",
            runtime_cwd.display()
        );
        let mut acp_env = vec![(
            "ANICA_ACP_PROVIDER".to_string(),
            match self.agent_provider {
                AcpAgentProvider::Codex => "codex",
                AcpAgentProvider::Gemini => "gemini",
                AcpAgentProvider::Claude => "claude",
            }
            .to_string(),
        )];
        if self.agent_provider == AcpAgentProvider::Codex {
            if !self.codex_model.trim().is_empty() {
                acp_env.push(("ANICA_CODEX_MODEL".to_string(), self.codex_model.clone()));
            }
            acp_env.push((
                "ANICA_CODEX_REASONING_EFFORT".to_string(),
                self.codex_reasoning_mode.env_value().to_string(),
            ));
        }
        if self.agent_provider == AcpAgentProvider::Gemini && !self.gemini_model.trim().is_empty() {
            acp_env.push(("ANICA_GEMINI_MODEL".to_string(), self.gemini_model.clone()));
        }
        if self.agent_provider == AcpAgentProvider::Claude && !self.claude_model.trim().is_empty() {
            acp_env.push(("ANICA_CLAUDE_MODEL".to_string(), self.claude_model.clone()));
        }
        if let Some(path) = normalize_cli_override(&self.codex_cli_bin) {
            acp_env.push(("ANICA_CODEX_CLI_BIN".to_string(), path));
        }
        if let Some(path) = normalize_cli_override(&self.gemini_cli_bin) {
            acp_env.push(("ANICA_GEMINI_CLI_BIN".to_string(), path));
        }
        if let Some(path) = normalize_cli_override(&self.claude_cli_bin) {
            acp_env.push(("ANICA_CLAUDE_CLI_BIN".to_string(), path));
        }
        if self.agent_provider == AcpAgentProvider::Gemini && !self.gemini_api_key.trim().is_empty()
        {
            acp_env.push(("GEMINI_API_KEY".to_string(), self.gemini_api_key.clone()));
            acp_env.push(("GOOGLE_API_KEY".to_string(), self.gemini_api_key.clone()));
        }
        // Pin ACP docs lookups to docs/acp so runtime guidance stays isolated from development docs.
        if docs_scoped {
            acp_env.push((
                "ANICA_DOCS_DIR".to_string(),
                acp_docs_dir.to_string_lossy().to_string(),
            ));
        }
        self.worker
            .connect(resolved_cmd.clone(), Some(runtime_cwd.clone()), acp_env);
        self.busy = true;
        let provider_detail = match self.agent_provider {
            AcpAgentProvider::Codex => format!(
                " (model: {}, thinking: {})",
                self.codex_model,
                self.codex_reasoning_mode.label()
            ),
            AcpAgentProvider::Gemini => format!(" (model: {})", self.gemini_model),
            AcpAgentProvider::Claude => format!(" (model: {})", self.claude_model),
        };
        self.push_system_message(
            format!(
                "Connecting {} ACP agent: {resolved_cmd}{provider_detail}",
                self.agent_provider.label(),
            ),
            cx,
        );
        if api_scoped {
            self.push_system_message(
                format!(
                    "ACP runtime scope locked to API folder: {}",
                    runtime_cwd.display()
                ),
                cx,
            );
        } else {
            self.push_system_message(
                format!(
                    "API folder not found at {}/src/api. Fallback scope: {}",
                    self.current_workspace_root(cx).display(),
                    runtime_cwd.display()
                ),
                cx,
            );
        }
        if docs_scoped {
            self.push_system_message(
                format!("ACP docs scope locked to: {}", acp_docs_dir.display()),
                cx,
            );
        } else {
            self.push_system_message(
                format!(
                    "ACP docs folder not found at {}. Agent will use default docs discovery.",
                    acp_docs_dir.display()
                ),
                cx,
            );
        }
        cx.notify();
    }

    fn submit_prompt(
        &mut self,
        prompt: String,
        clear_main_prompt: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if prompt.is_empty() {
            return false;
        }

        if !self.connected {
            self.push_system_message("Not connected. Connect an ACP agent first.", cx);
            cx.notify();
            return false;
        }

        let _ = self.push_chat_message(AiChatRole::User, prompt.clone(), false, cx);
        let pending_idx = self.push_chat_message(AiChatRole::Assistant, String::new(), true, cx);
        self.active_assistant_idx = Some(pending_idx);

        self.worker.send_prompt(prompt);
        self.busy = true;
        if clear_main_prompt {
            self.clear_prompt_input(window, cx);
        }
        cx.notify();
        true
    }

    fn on_send_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = self.prompt_text.trim().to_string();
        let _ = self.submit_prompt(prompt, true, window, cx);
    }

    pub fn send_prompt_from_external(
        &mut self,
        prompt: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.submit_prompt(prompt.trim().to_string(), false, window, cx)
    }
}

impl Render for AiAgentsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.global.read(cx).active_page;

        if self.prompt_send_on_next_render {
            self.prompt_send_on_next_render = false;
            if self.connected && !self.busy {
                self.on_send_prompt(window, cx);
            }
        }

        self.sync_worker_media_pool_snapshot(cx);
        self.ensure_inputs(window, cx);
        self.sync_visible_input_values(window, cx);
        self.ensure_worker_poller(cx);

        let command_input_elem = if let Some(input) = self.command_input.as_ref() {
            Input::new(input).h(px(32.0)).w_full().into_any_element()
        } else {
            div()
                .h(px(32.0))
                .w_full()
                .rounded_sm()
                .bg(white().opacity(0.05))
                .into_any_element()
        };

        let codex_cli_input_elem = if let Some(input) = self.codex_cli_input.as_ref() {
            Input::new(input).h(px(32.0)).w_full().into_any_element()
        } else {
            div()
                .h(px(32.0))
                .w_full()
                .rounded_sm()
                .bg(white().opacity(0.05))
                .into_any_element()
        };

        let gemini_cli_input_elem = if let Some(input) = self.gemini_cli_input.as_ref() {
            Input::new(input).h(px(32.0)).w_full().into_any_element()
        } else {
            div()
                .h(px(32.0))
                .w_full()
                .rounded_sm()
                .bg(white().opacity(0.05))
                .into_any_element()
        };

        let claude_cli_input_elem = if let Some(input) = self.claude_cli_input.as_ref() {
            Input::new(input).h(px(32.0)).w_full().into_any_element()
        } else {
            div()
                .h(px(32.0))
                .w_full()
                .rounded_sm()
                .bg(white().opacity(0.05))
                .into_any_element()
        };

        let gemini_key_input_elem = if let Some(input) = self.gemini_key_input.as_ref() {
            Input::new(input).h(px(32.0)).w_full().into_any_element()
        } else {
            div()
                .h(px(32.0))
                .w_full()
                .rounded_sm()
                .bg(white().opacity(0.05))
                .into_any_element()
        };

        let prompt_input_elem = if let Some(input) = self.prompt_input.as_ref() {
            Input::new(input).h(px(46.0)).w_full().into_any_element()
        } else {
            div()
                .h(px(46.0))
                .w_full()
                .rounded_sm()
                .bg(white().opacity(0.05))
                .into_any_element()
        };

        let chat_messages = self.global.read(cx).ai_chat_messages.clone();
        let system_count = chat_messages
            .iter()
            .filter(|m| matches!(m.role, AiChatRole::System))
            .count();
        let visible_chat_messages: Vec<AiChatMessage> = chat_messages
            .iter()
            .filter(|m| self.show_system_messages || !matches!(m.role, AiChatRole::System))
            .cloned()
            .collect();
        let mut message_list = div().flex().flex_col().gap_2();
        if visible_chat_messages.is_empty() {
            message_list =
                message_list.child(div().text_xs().text_color(white().opacity(0.55)).child(
                    if system_count > 0 {
                        format!(
                            "System logs are hidden ({}). Toggle to view them.",
                            system_count
                        )
                    } else {
                        "No messages yet.".to_string()
                    },
                ));
        } else {
            for (idx, msg) in visible_chat_messages.iter().enumerate() {
                message_list = message_list.child(Self::chat_bubble(msg, idx, window, cx));
            }
        }

        let is_logged_in = matches!(self.agent_status, AgentLoginStatus::LoggedIn { .. });
        let can_send = self.connected && !self.busy;
        let mut system_toggle_button = Self::action_button(if self.show_system_messages {
            format!("Hide System ({})", system_count)
        } else {
            format!("Show System ({})", system_count)
        })
        .h(px(26.0))
        .text_xs();
        if self.show_system_messages {
            system_toggle_button = system_toggle_button
                .bg(gpui::Hsla::from(rgb(0x7c2d12)).opacity(0.35))
                .border_color(gpui::Hsla::from(rgb(0xf59e0b)).opacity(0.45));
        }
        let mut auto_connect_button = Self::action_button(if self.auto_connect_enabled {
            "Auto Connect: On"
        } else {
            "Auto Connect: Off"
        });
        if self.auto_connect_enabled {
            auto_connect_button = auto_connect_button
                .bg(gpui::Hsla::from(rgb(0x14532d)).opacity(0.45))
                .border_color(gpui::Hsla::from(rgb(0x22c55e)).opacity(0.45))
                .text_color(white().opacity(0.95));
        }
        let mode_button = |label: &'static str, mode: CodexReasoningMode, active: bool| {
            let mut chip = div()
                .h(px(28.0))
                .px_2()
                .rounded_md()
                .border_1()
                .border_color(if active {
                    gpui::Hsla::from(rgb(0x3b82f6)).opacity(0.55)
                } else {
                    white().opacity(0.14)
                })
                .bg(if active {
                    gpui::Hsla::from(rgb(0x1e3a8a)).opacity(0.35)
                } else {
                    white().opacity(0.05)
                })
                .text_xs()
                .text_color(white().opacity(if active { 0.95 } else { 0.75 }))
                .flex()
                .items_center()
                .justify_center()
                .child(label);
            if !active {
                chip = chip.hover(|s| s.bg(white().opacity(0.1))).cursor_pointer();
            }
            chip.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.codex_reasoning_mode = mode;
                    this.push_system_message(
                        format!(
                            "Thinking mode set to {} (applies on next connect).",
                            mode.label()
                        ),
                        cx,
                    );
                    cx.notify();
                }),
            )
        };
        let model_button =
            |provider: AcpAgentProvider, option: ProviderModelOption, active: bool| {
                let slug = option.slug.clone();
                let display_label: SharedString = option.label.into();
                let mut chip = div()
                    .h(px(28.0))
                    .px_2()
                    .rounded_md()
                    .border_1()
                    .border_color(if active {
                        gpui::Hsla::from(rgb(0x3b82f6)).opacity(0.55)
                    } else {
                        white().opacity(0.14)
                    })
                    .bg(if active {
                        gpui::Hsla::from(rgb(0x1e3a8a)).opacity(0.35)
                    } else {
                        white().opacity(0.05)
                    })
                    .text_xs()
                    .text_color(white().opacity(if active { 0.95 } else { 0.75 }))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().max_w(px(150.0)).truncate().child(display_label));
                if !active {
                    chip = chip.hover(|s| s.bg(white().opacity(0.1))).cursor_pointer();
                }
                chip.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        match provider {
                            AcpAgentProvider::Codex => this.codex_model = slug.clone(),
                            AcpAgentProvider::Gemini => this.gemini_model = slug.clone(),
                            AcpAgentProvider::Claude => this.claude_model = slug.clone(),
                        }
                        this.push_system_message(
                            format!(
                                "{} model set to {} (applies on next connect).",
                                provider.label(),
                                slug
                            ),
                            cx,
                        );
                        cx.notify();
                    }),
                )
            };
        let connected_prov = self.connected_provider;
        let provider_button = |label: &'static str, provider: AcpAgentProvider, active: bool| {
            let is_connected = connected_prov == Some(provider);
            // Show a green dot next to the label of the provider that is actually connected.
            let display_label: SharedString = if is_connected {
                format!("{label} \u{25cf}").into()
            } else {
                label.into()
            };
            let mut chip = div()
                .h(px(30.0))
                .px_3()
                .rounded_md()
                .border_1()
                .border_color(if active {
                    gpui::Hsla::from(rgb(0x10b981)).opacity(0.55)
                } else {
                    white().opacity(0.14)
                })
                .bg(if active {
                    gpui::Hsla::from(rgb(0x064e3b)).opacity(0.45)
                } else {
                    white().opacity(0.05)
                })
                .text_xs()
                .text_color(if is_connected {
                    gpui::Hsla::from(rgb(0x22c55e)).opacity(0.95)
                } else {
                    white().opacity(if active { 0.95 } else { 0.75 })
                })
                .flex()
                .items_center()
                .justify_center()
                .child(display_label);
            if !active {
                chip = chip.hover(|s| s.bg(white().opacity(0.1))).cursor_pointer();
            }
            chip.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.select_agent_provider(provider, window, cx);
                    cx.notify();
                }),
            )
        };
        let provider_name = self.agent_provider.label();
        let status_title = self.agent_status.title(self.agent_provider);
        let status_detail = self.agent_status.detail_text();
        let status_hint = self.agent_status.action_hint(self.agent_provider);
        let mut provider_specific_help = div()
            .w_full()
            .min_w_0()
            .overflow_x_hidden()
            .flex()
            .flex_col()
            .gap_2();
        if self.agent_provider == AcpAgentProvider::Codex {
            let mut codex_model_buttons = div().flex().flex_wrap().gap_2();
            for option in self.codex_model_options.iter().cloned() {
                let active = self.codex_model == option.slug;
                codex_model_buttons = codex_model_buttons.child(model_button(
                    AcpAgentProvider::Codex,
                    option,
                    active,
                ));
            }
            let refresh_models_button = Self::action_button("Refresh Models")
                .h(px(26.0))
                .text_xs()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.reload_codex_model_options(true);
                        this.push_system_message(
                            format!(
                                "Codex models refreshed. Selected: {}. Source: {}",
                                this.codex_model, this.codex_models_source
                            ),
                            cx,
                        );
                        cx.notify();
                    }),
                );
            provider_specific_help = provider_specific_help
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(white().opacity(0.55))
                                        .child("Codex Model"),
                                )
                                .child(refresh_models_button),
                        )
                        .child(codex_model_buttons)
                        .child(
                            div()
                                .w_full()
                                .truncate()
                                .text_xs()
                                .text_color(white().opacity(0.45))
                                .child(self.codex_models_source.clone()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(white().opacity(0.55))
                                .child("Codex Thinking Mode"),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(mode_button(
                                    "Low",
                                    CodexReasoningMode::Low,
                                    self.codex_reasoning_mode == CodexReasoningMode::Low,
                                ))
                                .child(mode_button(
                                    "Medium",
                                    CodexReasoningMode::Medium,
                                    self.codex_reasoning_mode == CodexReasoningMode::Medium,
                                ))
                                .child(mode_button(
                                    "High",
                                    CodexReasoningMode::High,
                                    self.codex_reasoning_mode == CodexReasoningMode::High,
                                ))
                                .child(mode_button(
                                    "Extra High",
                                    CodexReasoningMode::ExtraHigh,
                                    self.codex_reasoning_mode == CodexReasoningMode::ExtraHigh,
                                )),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.55))
                        .child("If Codex reply fails with \"not found\": install CLI (`npm i -g @openai/codex`), then run `codex login`."),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.5))
                        .child(
                            "Model and thinking mode are passed to anica-acp at connect time.",
                        ),
                );
        } else if self.agent_provider == AcpAgentProvider::Gemini {
            let mut gemini_model_buttons = div().flex().flex_wrap().gap_2();
            for option in self.gemini_model_options.iter().cloned() {
                let active = self.gemini_model == option.slug;
                gemini_model_buttons = gemini_model_buttons.child(model_button(
                    AcpAgentProvider::Gemini,
                    option,
                    active,
                ));
            }
            let refresh_models_button = Self::action_button("Refresh Models")
                .h(px(26.0))
                .text_xs()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.reload_gemini_model_options(true);
                        this.push_system_message(
                            format!(
                                "Gemini models refreshed. Selected: {}. Source: {}",
                                this.gemini_model, this.gemini_models_source
                            ),
                            cx,
                        );
                        cx.notify();
                    }),
                );
            provider_specific_help = provider_specific_help
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(white().opacity(0.55))
                                        .child("Gemini Model"),
                                )
                                .child(refresh_models_button),
                        )
                        .child(gemini_model_buttons)
                        .child(
                            div()
                                .w_full()
                                .truncate()
                                .text_xs()
                                .text_color(white().opacity(0.45))
                                .child(self.gemini_models_source.clone()),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.55))
                        .child("Gemini API Key (optional)"),
                )
                .child(div().w_full().min_w_0().child(gemini_key_input_elem))
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.5))
                        .whitespace_normal()
                        .child(
                            "Gemini uses the same bundled anica-acp tool bridge as Codex. Use CLI login (`gemini`, then `/auth`) or provide an API key, which is forwarded on connect.",
                        ),
                );
        } else {
            let mut claude_model_buttons = div().flex().flex_wrap().gap_2();
            for option in self.claude_model_options.iter().cloned() {
                let active = self.claude_model == option.slug;
                claude_model_buttons = claude_model_buttons.child(model_button(
                    AcpAgentProvider::Claude,
                    option,
                    active,
                ));
            }
            let refresh_models_button = Self::action_button("Refresh Models")
                .h(px(26.0))
                .text_xs()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.reload_claude_model_options(true);
                        this.push_system_message(
                            format!(
                                "Claude models refreshed. Selected: {}. Source: {}",
                                this.claude_model, this.claude_models_source
                            ),
                            cx,
                        );
                        cx.notify();
                    }),
                );
            provider_specific_help = provider_specific_help
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(white().opacity(0.55))
                                        .child("Claude Model"),
                                )
                                .child(refresh_models_button),
                        )
                        .child(claude_model_buttons)
                        .child(
                            div()
                                .w_full()
                                .truncate()
                                .text_xs()
                                .text_color(white().opacity(0.45))
                                .child(self.claude_models_source.clone()),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.55))
                        .child("Claude Login"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.5))
                        .whitespace_normal()
                        .child(
                            "Claude mode uses the same bundled anica-acp tool bridge as Codex/Gemini. Login in terminal: `claude auth login`, then verify with `claude auth status`.",
                        ),
                  );
        }
        let (active_cli_label, active_cli_input_elem, active_cli_save_label) =
            match self.agent_provider {
                AcpAgentProvider::Codex => (
                    "Codex CLI Path",
                    codex_cli_input_elem,
                    "Save Codex CLI Path",
                ),
                AcpAgentProvider::Gemini => (
                    "Gemini CLI Path",
                    gemini_cli_input_elem,
                    "Save Gemini CLI Path",
                ),
                AcpAgentProvider::Claude => (
                    "Claude CLI Path",
                    claude_cli_input_elem,
                    "Save Claude CLI Path",
                ),
            };
        let cli_paths_section = div()
            .w_full()
            .min_w_0()
            .overflow_x_hidden()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.55))
                    .child("CLI Path Override (optional, saved locally)"),
            )
            .child(div().text_xs().text_color(white().opacity(0.45)).child(
                "Use this only when the GUI app cannot auto-detect the selected provider CLI.",
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.45))
                    .child("Leave it blank and save to go back to auto-detect/default path."),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.55))
                    .child(active_cli_label),
            )
            .child(div().w_full().min_w_0().child(active_cli_input_elem))
            .child(div().flex().gap_2().child(
                Self::action_button(active_cli_save_label).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.on_save_cli_paths(cx);
                    }),
                ),
            ));

        div()
            .size_full()
            .bg(gpui::rgb(0x09090b))
            .p_5()
            .flex()
            .flex_col()
            .overflow_y_scrollbar()
            .gap_4()
            .child(
                div()
                    .text_lg()
                    .text_color(white().opacity(0.92))
                    .child("AI Agents (ACP Chat)"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(white().opacity(0.62))
                    .child("Connect an ACP agent command and chat directly from Anica."),
            )
            .child(
                div()
                    .rounded_lg()
                    .border_1()
                    .border_color(self.agent_status.color().opacity(0.5))
                    .bg(self.agent_status.color().opacity(0.08))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(self.agent_status.color())
                            .child(status_title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.72))
                            .whitespace_normal()
                            .child(status_detail),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.6))
                            .whitespace_normal()
                            .child(status_hint),
                    ),
            )
            .child(
                div()
                    .rounded_lg()
                    .min_w_0()
                    .overflow_x_hidden()
                    .border_1()
                    .border_color(white().opacity(0.14))
                    .bg(white().opacity(0.03))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.55))
                            .child("ACP Provider"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(provider_button(
                                "Codex",
                                AcpAgentProvider::Codex,
                                self.agent_provider == AcpAgentProvider::Codex,
                            ))
                            .child(provider_button(
                                "Gemini",
                                AcpAgentProvider::Gemini,
                                self.agent_provider == AcpAgentProvider::Gemini,
                            ))
                            .child(provider_button(
                                "Claude",
                                AcpAgentProvider::Claude,
                                self.agent_provider == AcpAgentProvider::Claude,
                            )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.55))
                            .child("ACP Agent Command"),
                    )
                    .child(div().w_full().min_w_0().child(command_input_elem))
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.45))
                            .child(
                                "Default: resolved automatically at connect time. You can still enter an absolute path to your ACP binary.",
                            ),
                    )
                    .child(provider_specific_help)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Self::action_button("Refresh Login").on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.refresh_agent_status();
                                        cx.notify();
                                    }),
                                ),
                            )
                            .child(
                                Self::action_button("Connect")
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.on_connect(cx);
                                    }),
                                ),
                            )
                            .child(
                                Self::action_button("Disconnect")
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.on_disconnect(cx);
                                    }),
                                ),
                            )
                            .child(
                                auto_connect_button.on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.on_toggle_auto_connect(cx);
                                    }),
                                ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(if self.connected {
                                        gpui::Hsla::from(rgb(0x22c55e)).opacity(0.9)
                                    } else if is_logged_in {
                                        white().opacity(0.65)
                                    } else {
                                        gpui::Hsla::from(rgb(0xf59e0b)).opacity(0.9)
                                    })
                                    .child(if self.connected {
                                        format!("Connected ({provider_name}) | {}", self.last_status)
                                    } else {
                                        format!("Disconnected ({provider_name}) | {}", self.last_status)
                                    }),
                            ),
                    )
                    .child(cli_paths_section)
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.45))
                            .child("Tip: Codex, Gemini, and Claude default to bundled anica-acp (same tool-bridge pipeline). You can still override ACP Agent Command manually."),
                    )
            )
            .child(
                div().flex().items_center().justify_between().child(system_toggle_button.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.show_system_messages = !this.show_system_messages;
                        cx.notify();
                    }),
                )),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .rounded_lg()
                    .border_1()
                    .border_color(white().opacity(0.14))
                    .bg(white().opacity(0.02))
                    .p_3()
                    .overflow_y_scrollbar()
                    .child(message_list),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(prompt_input_elem))
                    .child(
                        div()
                            .w(px(86.0))
                            .flex_shrink_0()
                            .child(
                                if can_send {
                                    Self::action_button("Send")
                                } else {
                                    Self::action_button(if self.busy { "Sending..." } else { "Send" })
                                        .opacity(0.55)
                                }
                                .h(px(46.0))
                                .w_full()
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, window, cx| {
                                        if this.connected && !this.busy {
                                            this.on_send_prompt(window, cx);
                                        }
                                    }),
                                ),
                        ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{AI_CHAT_MAX_CONVERSATION_MESSAGES, AiAgentsPage, prune_ai_chat_history};
    use crate::core::global_state::{AiChatMessage, AiChatRole};
    use motionloom::parse_graph_script;

    fn msg(role: AiChatRole, text: &str) -> AiChatMessage {
        AiChatMessage {
            role,
            text: text.to_string(),
            pending: false,
        }
    }

    #[test]
    fn prune_history_ignores_system_messages_for_limit() {
        let mut messages = vec![
            msg(AiChatRole::System, "boot"),
            msg(AiChatRole::User, "u1"),
            msg(AiChatRole::System, "tool"),
            msg(AiChatRole::Assistant, "a1"),
        ];

        let removed = prune_ai_chat_history(&mut messages);

        assert_eq!(removed, 0);
        assert_eq!(messages.len(), 4);
    }

    #[test]
    fn prune_history_removes_oldest_user_assistant_turn() {
        let mut messages = Vec::new();
        for idx in 0..(AI_CHAT_MAX_CONVERSATION_MESSAGES / 2 + 1) {
            messages.push(msg(AiChatRole::User, &format!("u{idx}")));
            messages.push(msg(AiChatRole::System, &format!("s{idx}")));
            messages.push(msg(AiChatRole::Assistant, &format!("a{idx}")));
        }

        let removed = prune_ai_chat_history(&mut messages);

        assert!(removed >= 3);
        assert_eq!(
            messages
                .iter()
                .filter(|msg| !matches!(msg.role, AiChatRole::System))
                .count(),
            AI_CHAT_MAX_CONVERSATION_MESSAGES
        );
        assert_eq!(messages.first().map(|msg| msg.text.as_str()), Some("u1"));
    }

    #[test]
    fn normalize_motionloom_scene_defaults_fps_and_size() {
        let input = r##"
<Graph scope="scene" duration="3s">
  <Solid color="#000000" />
  <Present from="scene" />
</Graph>
"##;
        let out = AiAgentsPage::normalize_motionloom_scene_script(input);
        assert!(out.patched_fps);
        assert!(out.patched_size);
        assert!(out.script.contains("fps={60}"));
        assert!(out.script.contains("size={[1920,1080]}"));
        parse_graph_script(&out.script).expect("normalized graph should parse");
    }

    #[test]
    fn normalize_motionloom_scene_rewrites_animate_opacity_and_duration_ms() {
        let input = r##"
<Graph scope="scene" fps={60} size={[1920,1080]}>
  <Solid color="#000000" />
  <Text id="title" value="HELLO WORLD" color="#FFFFFF" fontSize={120} x={960} y={540} opacity={0} />
  <Animate target="title.opacity" from={0} to={1} start_ms={0} end_ms={1200} />
  <Present from="scene" duration_ms={3000} />
</Graph>
"##;
        let out = AiAgentsPage::normalize_motionloom_scene_script(input);
        assert!(out.patched_duration);
        assert_eq!(out.patched_animate_opacity, vec!["title".to_string()]);
        assert!(out.script.contains(r#"duration="3000ms""#));
        assert!(out.script.contains(
            r#"opacity="0.000000+(1.000000-0.000000)*min(max(($time.sec-0.000000)/1.200000,0),1)""#
        ));
        assert!(!out.script.contains("<Animate"));
        parse_graph_script(&out.script).expect("normalized graph should parse");
    }

    #[test]
    fn normalize_motionloom_scene_rewrites_text_alias_to_value() {
        let input = r##"
<Graph scope="scene" fps={60} duration="3s" size={[1920,1080]}>
  <Solid color="#000000" />
  <Text id="title" text="HELLO WORLD" color="#FFFFFF" fontSize={120} x="center" y="center" opacity="1" />
  <Present from="scene" />
</Graph>
"##;
        let out = AiAgentsPage::normalize_motionloom_scene_script(input);
        assert_eq!(out.patched_text_value, vec!["title".to_string()]);
        assert!(out.script.contains(r#"value="HELLO WORLD""#));
        parse_graph_script(&out.script).expect("normalized graph should parse");
    }

    #[test]
    fn normalize_motionloom_scene_rewrites_text_body_to_value() {
        let input = r##"
<Graph scope="scene" fps={60} duration="3s" size={[1920,1080]}>
  <Solid color="#000000" />
  <Text id="title" color="#FFFFFF" fontSize={120} x="center" y="center" opacity="1">HELLO WORLD</Text>
  <Present from="scene" />
</Graph>
"##;
        let out = AiAgentsPage::normalize_motionloom_scene_script(input);
        assert_eq!(out.patched_text_value, vec!["title".to_string()]);
        assert!(out.script.contains(r#"value="HELLO WORLD""#));
        assert!(!out.script.contains("</Text>"));
        parse_graph_script(&out.script).expect("normalized graph should parse");
    }
}
