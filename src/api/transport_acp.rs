use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::api::export::{AcpExportRunRequest, AcpExportRunResponse};
use crate::api::llm::{
    LlmDecisionMakingSrtSimilarSerachRequest, LlmDecisionMakingSrtSimilarSerachResponse,
};
use crate::api::media_pool::{
    ClearMediaPoolRequest, ClearMediaPoolResponse, ListMediaPoolMetadataRequest,
    RemoveMediaPoolByIdRequest, RemoveMediaPoolByIdResponse, list_media_metadata_from_pool_items,
};
use crate::api::timeline::{
    AudioSilenceCutPlanRequest, AudioSilenceCutPlanResponse, AudioSilenceMapRequest,
    AudioSilenceMapResponse, AutonomousEditPlanRequest, AutonomousEditPlanResponse,
    SubtitleGapCutPlanRequest, SubtitleGapCutPlanResponse, SubtitleGapMapRequest,
    SubtitleGapMapResponse, SubtitleSemanticRepeatsRequest, SubtitleSemanticRepeatsResponse,
    TimelineEditApplyResponse, TimelineEditPlanRequest, TimelineEditValidationResponse,
    TimelineSnapshotRequest, TimelineSnapshotResponse, TranscriptLowConfidenceCutPlanRequest,
    TranscriptLowConfidenceCutPlanResponse, TranscriptLowConfidenceMapRequest,
    TranscriptLowConfidenceMapResponse,
};
use crate::core::global_state::MediaPoolItem;
use agent_client_protocol::{
    Agent, AuthenticateRequest, CancelNotification, Client, ClientCapabilities,
    ClientSideConnection, ContentBlock, ContentChunk, Error as AcpError, ExtRequest, ExtResponse,
    FileSystemCapability, Implementation, InitializeRequest, NewSessionRequest,
    PermissionOptionKind, PromptRequest, ProtocolVersion, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome, SessionId,
    SessionNotification, SessionUpdate,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStderr, Command as TokioCommand};
use tokio::task::LocalSet;
use tokio::time::timeout;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

#[derive(Debug)]
enum AcpWorkerCommand {
    Connect {
        command: String,
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
    },
    SendPrompt {
        prompt: String,
    },
    UpdateMediaPoolSnapshot {
        items: Vec<MediaPoolItem>,
        ffmpeg_available: bool,
        ffprobe_available: bool,
        ffprobe_command: String,
    },
    Disconnect,
    Shutdown,
}

#[derive(Debug)]
pub enum AcpToolBridgeRequest {
    LlmDecisionMakingSrtSimilarSerach {
        request: LlmDecisionMakingSrtSimilarSerachRequest,
        reply_tx: Sender<Result<LlmDecisionMakingSrtSimilarSerachResponse, String>>,
    },
    BuildAutonomousEditPlan {
        request: AutonomousEditPlanRequest,
        reply_tx: Sender<Result<AutonomousEditPlanResponse, String>>,
    },
    GetTimelineSnapshot {
        request: TimelineSnapshotRequest,
        reply_tx: Sender<Result<TimelineSnapshotResponse, String>>,
    },
    GetAudioSilenceMap {
        request: AudioSilenceMapRequest,
        reply_tx: Sender<Result<AudioSilenceMapResponse, String>>,
    },
    BuildAudioSilenceCutPlan {
        request: AudioSilenceCutPlanRequest,
        reply_tx: Sender<Result<AudioSilenceCutPlanResponse, String>>,
    },
    GetTranscriptLowConfidenceMap {
        request: TranscriptLowConfidenceMapRequest,
        reply_tx: Sender<Result<TranscriptLowConfidenceMapResponse, String>>,
    },
    BuildTranscriptLowConfidenceCutPlan {
        request: TranscriptLowConfidenceCutPlanRequest,
        reply_tx: Sender<Result<TranscriptLowConfidenceCutPlanResponse, String>>,
    },
    GetSubtitleGapMap {
        request: SubtitleGapMapRequest,
        reply_tx: Sender<Result<SubtitleGapMapResponse, String>>,
    },
    BuildSubtitleGapCutPlan {
        request: SubtitleGapCutPlanRequest,
        reply_tx: Sender<Result<SubtitleGapCutPlanResponse, String>>,
    },
    GetSubtitleSemanticRepeats {
        request: SubtitleSemanticRepeatsRequest,
        reply_tx: Sender<Result<SubtitleSemanticRepeatsResponse, String>>,
    },
    ValidateEditPlan {
        request: TimelineEditPlanRequest,
        reply_tx: Sender<Result<TimelineEditValidationResponse, String>>,
    },
    ApplyEditPlan {
        request: TimelineEditPlanRequest,
        reply_tx: Sender<Result<TimelineEditApplyResponse, String>>,
    },
    RunExport {
        request: AcpExportRunRequest,
        reply_tx: Sender<Result<AcpExportRunResponse, String>>,
    },
    RemoveMediaPoolById {
        request: RemoveMediaPoolByIdRequest,
        reply_tx: Sender<Result<RemoveMediaPoolByIdResponse, String>>,
    },
    ClearMediaPool {
        request: ClearMediaPoolRequest,
        reply_tx: Sender<Result<ClearMediaPoolResponse, String>>,
    },
}

#[derive(Debug)]
pub enum AcpUiEvent {
    Status(String),
    Connected {
        session_id: String,
        agent_label: String,
    },
    Disconnected {
        reason: String,
    },
    AssistantChunk(String),
    PromptFinished {
        stop_reason: String,
    },
    Error(String),
    ToolBridgeRequest(AcpToolBridgeRequest),
}

pub struct AcpWorker {
    cmd_tx: Sender<AcpWorkerCommand>,
    evt_rx: Receiver<AcpUiEvent>,
}

#[derive(Default)]
struct AcpSharedState {
    media_pool: Vec<MediaPoolItem>,
    ffmpeg_available: bool,
    ffprobe_available: bool,
    ffprobe_command: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct DocsListRequest {
    #[serde(default)]
    subdir: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct DocsListResponse {
    root: String,
    subdir: String,
    files: Vec<String>,
    total_files: usize,
}

#[derive(Debug, Clone, Deserialize)]
struct DocsReadRequest {
    path: String,
    #[serde(default = "default_docs_max_chars")]
    max_chars: usize,
}

#[derive(Debug, Clone, Serialize)]
struct DocsReadResponse {
    root: String,
    path: String,
    exists: bool,
    is_text: bool,
    truncated: bool,
    content: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Error)]
enum AcpInternalError {
    #[error("docs root not found. expected ./docs or ../docs")]
    DocsRootNotFound,
    #[error("failed to read docs dir {dir}: {source}")]
    ReadDocsDir { dir: String, source: std::io::Error },
    #[error("failed to read docs entry: {source}")]
    ReadDocsEntry { source: std::io::Error },
    #[error("failed to read file type for {path}: {source}")]
    ReadDocsFileType {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid docs subdir path")]
    InvalidDocsSubdirPath,
    #[error("docs subdir does not exist: {path}")]
    DocsSubdirMissing { path: String },
    #[error("docs subdir is not a directory: {path}")]
    DocsSubdirNotDirectory { path: String },
    #[error("invalid docs file path")]
    InvalidDocsFilePath,
    #[error("docs read path is empty")]
    DocsReadPathEmpty,
    #[error("failed to read {path}: {source}")]
    ReadDocsFile {
        path: String,
        source: std::io::Error,
    },
    #[error("ACP command is empty")]
    EmptyAgentCommand,
    #[error(
        "ACP command not found: `{bin}`. Bundle `anica-acp` (or `codex-acp`) in app Resources, or set ANICA_ACP_AGENT_CMD to a valid command/path."
    )]
    AgentCommandNotFound { bin: String },
    #[error("Failed to spawn ACP agent process: {source}")]
    SpawnAgentProcess { source: std::io::Error },
    #[error("Failed to open agent stdin")]
    MissingAgentStdin,
    #[error("Failed to open agent stdout")]
    MissingAgentStdout,
    #[error("Failed to open agent stderr")]
    MissingAgentStderr,
    #[error("ACP initialize failed: {message}")]
    Initialize { message: String },
    #[error("ACP authenticate failed: {message}")]
    Authenticate { message: String },
    #[error("ACP session/new failed: {message}")]
    NewSession { message: String },
    #[error("ACP session/prompt failed: {message}")]
    Prompt { message: String },
}

fn default_docs_max_chars() -> usize {
    40_000
}

fn sanitize_relative_path(path: &str) -> Option<PathBuf> {
    let raw = path.trim();
    if raw.is_empty() {
        return Some(PathBuf::new());
    }
    let p = Path::new(raw);
    if p.is_absolute() {
        return None;
    }

    let mut out = PathBuf::new();
    for component in p.components() {
        match component {
            Component::Normal(seg) => out.push(seg),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

fn docs_path_candidates(rel: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    // Try requested path first.
    candidates.push(rel.to_path_buf());

    // Backward/forward compatibility:
    // - If request is "export/...", also try "acp/export/..."
    // - If request is "acp/export/...", also try "export/..."
    let mut comps = rel.components();
    let first = comps.next();
    let second = comps.next();
    let is_export = matches!(first, Some(Component::Normal(seg)) if seg == "export");
    let is_acp_export = matches!(first, Some(Component::Normal(seg)) if seg == "acp")
        && matches!(second, Some(Component::Normal(seg)) if seg == "export");

    if is_export {
        candidates.push(PathBuf::from("acp").join(rel));
    } else if is_acp_export {
        let mut stripped = PathBuf::new();
        let mut skip_first = true;
        for component in rel.components() {
            if skip_first {
                skip_first = false;
                continue;
            }
            stripped.push(component.as_os_str());
        }
        candidates.push(stripped);
    }

    candidates
}

fn candidate_docs_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Some(from_env) = std::env::var_os("ANICA_DOCS_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
    {
        roots.push(from_env);
    }

    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.join("docs"));
        if let Some(parent) = cwd.parent() {
            roots.push(parent.join("docs"));
        }
    }

    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs"));

    let mut unique = Vec::new();
    for root in roots {
        if !unique.iter().any(|p| p == &root) {
            unique.push(root);
        }
    }
    unique
}

fn resolve_docs_root() -> Result<PathBuf, AcpInternalError> {
    candidate_docs_roots()
        .into_iter()
        .find(|root| root.is_dir())
        .ok_or(AcpInternalError::DocsRootNotFound)
}

fn collect_files_recursive(
    dir: &Path,
    base: &Path,
    out: &mut Vec<String>,
) -> Result<(), AcpInternalError> {
    let entries = fs::read_dir(dir).map_err(|source| AcpInternalError::ReadDocsDir {
        dir: dir.display().to_string(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| AcpInternalError::ReadDocsEntry { source })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| AcpInternalError::ReadDocsFileType {
                path: path.display().to_string(),
                source,
            })?;

        if file_type.is_dir() {
            collect_files_recursive(&path, base, out)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        let Ok(rel) = path.strip_prefix(base) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        out.push(rel);
    }
    Ok(())
}

fn list_docs_files(request: DocsListRequest) -> Result<DocsListResponse, AcpInternalError> {
    let root = resolve_docs_root()?;
    let subdir_rel = request.subdir.unwrap_or_default();
    let subdir =
        sanitize_relative_path(&subdir_rel).ok_or(AcpInternalError::InvalidDocsSubdirPath)?;
    let mut target = root.join(&subdir);
    if (!target.exists() || !target.is_dir())
        && let Some(found) = docs_path_candidates(&subdir)
            .into_iter()
            .map(|candidate| root.join(candidate))
            .find(|candidate| candidate.exists() && candidate.is_dir())
    {
        target = found;
    }
    if !target.exists() {
        return Err(AcpInternalError::DocsSubdirMissing {
            path: target.display().to_string(),
        });
    }
    if !target.is_dir() {
        return Err(AcpInternalError::DocsSubdirNotDirectory {
            path: target.display().to_string(),
        });
    }

    let mut files = Vec::new();
    collect_files_recursive(&target, &root, &mut files)?;
    files.sort();

    Ok(DocsListResponse {
        root: root.to_string_lossy().to_string(),
        subdir: if subdir.as_os_str().is_empty() {
            ".".to_string()
        } else {
            subdir.to_string_lossy().replace('\\', "/")
        },
        total_files: files.len(),
        files,
    })
}

fn read_docs_file(request: DocsReadRequest) -> Result<DocsReadResponse, AcpInternalError> {
    let root = resolve_docs_root()?;
    let rel = sanitize_relative_path(&request.path).ok_or(AcpInternalError::InvalidDocsFilePath)?;
    if rel.as_os_str().is_empty() {
        return Err(AcpInternalError::DocsReadPathEmpty);
    }

    let mut full_path = root.join(&rel);
    if (!full_path.exists() || !full_path.is_file())
        && let Some(found) = docs_path_candidates(&rel)
            .into_iter()
            .map(|candidate| root.join(candidate))
            .find(|candidate| candidate.exists() && candidate.is_file())
    {
        full_path = found;
    }
    if !full_path.exists() {
        return Ok(DocsReadResponse {
            root: root.to_string_lossy().to_string(),
            path: rel.to_string_lossy().replace('\\', "/"),
            exists: false,
            is_text: false,
            truncated: false,
            content: None,
            error: Some("file not found".to_string()),
        });
    }
    if !full_path.is_file() {
        return Ok(DocsReadResponse {
            root: root.to_string_lossy().to_string(),
            path: rel.to_string_lossy().replace('\\', "/"),
            exists: false,
            is_text: false,
            truncated: false,
            content: None,
            error: Some("path is not a file".to_string()),
        });
    }

    let bytes = fs::read(&full_path).map_err(|source| AcpInternalError::ReadDocsFile {
        path: full_path.display().to_string(),
        source,
    })?;
    let is_text = std::str::from_utf8(&bytes).is_ok();
    let mut content = String::from_utf8_lossy(&bytes).to_string();

    let max_chars = request.max_chars.clamp(256, 200_000);
    let char_count = content.chars().count();
    let truncated = char_count > max_chars;
    if truncated {
        content = content.chars().take(max_chars).collect::<String>();
    }

    Ok(DocsReadResponse {
        root: root.to_string_lossy().to_string(),
        path: rel.to_string_lossy().replace('\\', "/"),
        exists: true,
        is_text,
        truncated,
        content: Some(content),
        error: None,
    })
}

fn first_command_token(command: &str) -> Option<String> {
    let trimmed = command.trim_start();
    if trimmed.is_empty() {
        return None;
    }

    let mut token = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    for ch in trimmed.chars() {
        if escaped {
            token.push(ch);
            escaped = false;
            continue;
        }

        if in_single_quote {
            if ch == '\'' {
                in_single_quote = false;
            } else {
                token.push(ch);
            }
            continue;
        }

        if in_double_quote {
            if ch == '"' {
                in_double_quote = false;
            } else if ch == '\\' {
                escaped = true;
            } else {
                token.push(ch);
            }
            continue;
        }

        match ch {
            '\'' => in_single_quote = true,
            '"' => in_double_quote = true,
            '\\' => escaped = true,
            c if c.is_whitespace() => break,
            c => token.push(c),
        }
    }

    if escaped {
        token.push('\\');
    }

    if token.trim().is_empty() {
        None
    } else {
        Some(token)
    }
}

fn command_exists(bin: &str) -> bool {
    if bin.is_empty() {
        return false;
    }

    if bin.contains('/') || bin.contains('\\') {
        return Path::new(bin).is_file();
    }

    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };

    std::env::split_paths(&path_var).any(|dir| {
        let candidate = dir.join(bin);
        if candidate.is_file() {
            return true;
        }

        if cfg!(windows) {
            for ext in [".exe", ".bat", ".cmd"] {
                if dir.join(format!("{bin}{ext}")).is_file() {
                    return true;
                }
            }
        }

        false
    })
}

fn preflight_agent_command(command: &str) -> Result<(), AcpInternalError> {
    let Some(bin) = first_command_token(command) else {
        return Err(AcpInternalError::EmptyAgentCommand);
    };

    if command_exists(&bin) {
        Ok(())
    } else {
        Err(AcpInternalError::AgentCommandNotFound { bin })
    }
}

#[cfg(test)]
mod tests {
    use super::first_command_token;

    #[test]
    fn first_command_token_handles_plain_token() {
        assert_eq!(
            first_command_token("anica-acp --stdio"),
            Some("anica-acp".to_string())
        );
    }

    #[test]
    fn first_command_token_handles_single_quoted_path_with_spaces() {
        assert_eq!(
            first_command_token("'/tmp/untitled folder/anica-acp' --stdio"),
            Some("/tmp/untitled folder/anica-acp".to_string())
        );
    }

    #[test]
    fn first_command_token_handles_escaped_space_path() {
        assert_eq!(
            first_command_token("/tmp/untitled\\ folder/anica-acp --stdio"),
            Some("/tmp/untitled folder/anica-acp".to_string())
        );
    }
}

impl AcpWorker {
    pub fn spawn() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<AcpWorkerCommand>();
        let (evt_tx, evt_rx) = mpsc::channel::<AcpUiEvent>();

        thread::spawn(move || worker_loop(cmd_rx, evt_tx));

        Self { cmd_tx, evt_rx }
    }

    pub fn connect(&self, command: String, cwd: Option<PathBuf>, env: Vec<(String, String)>) {
        let _ = self
            .cmd_tx
            .send(AcpWorkerCommand::Connect { command, cwd, env });
    }

    pub fn send_prompt(&self, prompt: String) {
        let _ = self.cmd_tx.send(AcpWorkerCommand::SendPrompt { prompt });
    }

    pub fn disconnect(&self) {
        let _ = self.cmd_tx.send(AcpWorkerCommand::Disconnect);
    }

    pub fn update_media_pool_snapshot(
        &self,
        items: Vec<MediaPoolItem>,
        ffmpeg_available: bool,
        ffprobe_available: bool,
        ffprobe_command: String,
    ) {
        let _ = self.cmd_tx.send(AcpWorkerCommand::UpdateMediaPoolSnapshot {
            items,
            ffmpeg_available,
            ffprobe_available,
            ffprobe_command,
        });
    }

    pub fn try_recv(&self) -> Option<AcpUiEvent> {
        self.evt_rx.try_recv().ok()
    }
}

impl Drop for AcpWorker {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(AcpWorkerCommand::Shutdown);
    }
}

struct AcpSession {
    command: String,
    conn: ClientSideConnection,
    session_id: SessionId,
    child: Child,
}

impl AcpSession {
    async fn connect(
        command: String,
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
        evt_tx: Sender<AcpUiEvent>,
        shared_state: Arc<Mutex<AcpSharedState>>,
    ) -> Result<Self, AcpInternalError> {
        preflight_agent_command(&command)?;

        let mut cmd = if cfg!(target_os = "windows") {
            let mut c = TokioCommand::new("cmd");
            c.arg("/C").arg(&command);
            c
        } else {
            let mut c = TokioCommand::new("/bin/sh");
            c.arg("-lc").arg(&command);
            c
        };

        if let Some(path) = cwd.clone() {
            cmd.current_dir(path);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }

        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .map_err(|source| AcpInternalError::SpawnAgentProcess { source })?;

        let stdin = child
            .stdin
            .take()
            .ok_or(AcpInternalError::MissingAgentStdin)?;
        let stdout = child
            .stdout
            .take()
            .ok_or(AcpInternalError::MissingAgentStdout)?;
        let stderr = child
            .stderr
            .take()
            .ok_or(AcpInternalError::MissingAgentStderr)?;

        let handler = AcpClientHandler {
            evt_tx: evt_tx.clone(),
            shared_state,
        };

        let (conn, io_task) =
            ClientSideConnection::new(handler, stdin.compat_write(), stdout.compat(), |fut| {
                tokio::task::spawn_local(fut);
            });

        let io_evt_tx = evt_tx.clone();
        tokio::task::spawn_local(async move {
            if let Err(err) = io_task.await {
                let _ = io_evt_tx.send(AcpUiEvent::Error(format!("ACP I/O error: {err}")));
            }
        });

        tokio::task::spawn_local(stream_stderr(stderr, evt_tx.clone()));

        let init_req = InitializeRequest::new(ProtocolVersion::LATEST)
            .client_capabilities(
                ClientCapabilities::new()
                    .fs(FileSystemCapability::new()
                        .read_text_file(false)
                        .write_text_file(false))
                    .terminal(false),
            )
            .client_info(Implementation::new("anica", env!("CARGO_PKG_VERSION")).title("Anica"));

        let init_rsp =
            conn.initialize(init_req)
                .await
                .map_err(|err| AcpInternalError::Initialize {
                    message: err.to_string(),
                })?;

        let agent_label = init_rsp
            .agent_info
            .as_ref()
            .map(|info| {
                let title = info.title.clone().unwrap_or_else(|| info.name.clone());
                format!("{title} {}", info.version)
            })
            .unwrap_or_else(|| "Unknown Agent".to_string());

        if let Some(auth_method) = init_rsp.auth_methods.first() {
            let _ = evt_tx.send(AcpUiEvent::Status(format!(
                "Authenticating via {}",
                auth_method.name
            )));

            conn.authenticate(AuthenticateRequest::new(auth_method.id.clone()))
                .await
                .map_err(|err| AcpInternalError::Authenticate {
                    message: err.to_string(),
                })?;
        }

        let working_dir =
            cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let session_rsp = conn
            .new_session(NewSessionRequest::new(working_dir))
            .await
            .map_err(|err| AcpInternalError::NewSession {
                message: err.to_string(),
            })?;

        let session_id = session_rsp.session_id.clone();

        let _ = evt_tx.send(AcpUiEvent::Connected {
            session_id: session_id.0.to_string(),
            agent_label,
        });

        Ok(Self {
            command,
            conn,
            session_id,
            child,
        })
    }

    async fn prompt(&self, prompt: String) -> Result<String, AcpInternalError> {
        let req = PromptRequest::new(self.session_id.clone(), vec![ContentBlock::from(prompt)]);
        let rsp = self
            .conn
            .prompt(req)
            .await
            .map_err(|err| AcpInternalError::Prompt {
                message: err.to_string(),
            })?;
        Ok(format!("{:?}", rsp.stop_reason))
    }

    async fn cancel_prompt(&self) {
        let _ = self
            .conn
            .cancel(CancelNotification::new(self.session_id.clone()))
            .await;
    }

    async fn shutdown(&mut self) {
        let _ = self
            .conn
            .cancel(CancelNotification::new(self.session_id.clone()))
            .await;
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

async fn stream_stderr(stderr: ChildStderr, evt_tx: Sender<AcpUiEvent>) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            eprintln!("[ACP STDERR] {trimmed}");
            let _ = evt_tx.send(AcpUiEvent::Status(trimmed.to_string()));
        }
    }
}

struct AcpClientHandler {
    evt_tx: Sender<AcpUiEvent>,
    shared_state: Arc<Mutex<AcpSharedState>>,
}

fn json_raw_from_value<T: Serialize>(value: &T) -> agent_client_protocol::Result<Arc<RawValue>> {
    let json = serde_json::to_string(value).map_err(|err| {
        AcpError::internal_error().data(format!("serialize ext response failed: {err}"))
    })?;
    let raw = RawValue::from_string(json).map_err(|err| {
        AcpError::internal_error().data(format!("build raw ext response failed: {err}"))
    })?;
    Ok(raw.into())
}

fn call_tool_bridge<T, F>(
    evt_tx: &Sender<AcpUiEvent>,
    timeout_secs: u64,
    method: &'static str,
    make_request: F,
) -> agent_client_protocol::Result<T>
where
    F: FnOnce(Sender<Result<T, String>>) -> AcpToolBridgeRequest,
{
    let (reply_tx, reply_rx) = mpsc::channel();
    evt_tx
        .send(AcpUiEvent::ToolBridgeRequest(make_request(reply_tx)))
        .map_err(|err| {
            AcpError::internal_error().data(format!("tool bridge send failed: {err}"))
        })?;
    let response = reply_rx
        .recv_timeout(Duration::from_secs(timeout_secs))
        .map_err(|err| {
            AcpError::internal_error().data(format!("tool bridge timeout for {method}: {err}"))
        })?;
    response.map_err(|err| AcpError::internal_error().data(err))
}

#[async_trait(?Send)]
impl Client for AcpClientHandler {
    async fn request_permission(
        &self,
        args: RequestPermissionRequest,
    ) -> agent_client_protocol::Result<RequestPermissionResponse> {
        let selected = args
            .options
            .iter()
            .find(|opt| {
                matches!(
                    opt.kind,
                    PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
                )
            })
            .or_else(|| args.options.first());

        if let Some(option) = selected {
            let _ = self.evt_tx.send(AcpUiEvent::Status(format!(
                "Permission: {} ({:?})",
                option.name, option.kind
            )));
            Ok(RequestPermissionResponse::new(
                RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                    option.option_id.clone(),
                )),
            ))
        } else {
            Ok(RequestPermissionResponse::new(
                RequestPermissionOutcome::Cancelled,
            ))
        }
    }

    async fn session_notification(
        &self,
        args: SessionNotification,
    ) -> agent_client_protocol::Result<()> {
        match args.update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                if let Some(text) = text_from_chunk(&chunk) {
                    let _ = self.evt_tx.send(AcpUiEvent::AssistantChunk(text));
                }
            }
            SessionUpdate::ToolCall(call) => {
                let _ = self
                    .evt_tx
                    .send(AcpUiEvent::Status(format!("Tool call: {}", call.title)));
            }
            SessionUpdate::ToolCallUpdate(_) => {
                let _ = self
                    .evt_tx
                    .send(AcpUiEvent::Status("Tool update received".to_string()));
            }
            SessionUpdate::Plan(_) => {
                let _ = self
                    .evt_tx
                    .send(AcpUiEvent::Status("Plan update received".to_string()));
            }
            _ => {}
        }
        Ok(())
    }

    async fn ext_method(&self, args: ExtRequest) -> agent_client_protocol::Result<ExtResponse> {
        match args.method.as_ref() {
            "anica.media_pool/list_metadata" => {
                let request =
                    serde_json::from_str::<ListMediaPoolMetadataRequest>(args.params.get())
                        .map_err(|err| {
                            AcpError::invalid_params()
                                .data(format!("invalid list_metadata request: {err}"))
                        })?;
                let shared = self
                    .shared_state
                    .lock()
                    .map_err(|_| AcpError::internal_error().data("shared state poisoned"))?;
                if request.include_media_probe && !shared.ffprobe_available {
                    return Err(AcpError::internal_error().data(
                        "MISSING_FFPROBE: anica.media_pool/list_metadata media probe requires ffprobe. Install FFmpeg package and retry.",
                    ));
                }
                let items = shared.media_pool.clone();
                let response = list_media_metadata_from_pool_items(
                    &items,
                    request,
                    Some(shared.ffprobe_command.as_str()),
                );
                let raw = json_raw_from_value(&response)?;
                Ok(ExtResponse::new(raw))
            }
            "anica.media_pool/remove_by_id" => {
                let request = serde_json::from_str::<RemoveMediaPoolByIdRequest>(args.params.get())
                    .map_err(|err| {
                        AcpError::invalid_params()
                            .data(format!("invalid media_pool/remove_by_id request: {err}"))
                    })?;
                let response = call_tool_bridge(
                    &self.evt_tx,
                    8,
                    "media_pool/remove_by_id",
                    move |reply_tx| AcpToolBridgeRequest::RemoveMediaPoolById { request, reply_tx },
                )?;
                let raw = json_raw_from_value(&response)?;
                Ok(ExtResponse::new(raw))
            }
            "anica.media_pool/clear_all" => {
                let request = serde_json::from_str::<ClearMediaPoolRequest>(args.params.get())
                    .map_err(|err| {
                        AcpError::invalid_params()
                            .data(format!("invalid media_pool/clear_all request: {err}"))
                    })?;
                let response =
                    call_tool_bridge(&self.evt_tx, 12, "media_pool/clear_all", move |reply_tx| {
                        AcpToolBridgeRequest::ClearMediaPool { request, reply_tx }
                    })?;
                let raw = json_raw_from_value(&response)?;
                Ok(ExtResponse::new(raw))
            }
            "anica.llm/decision_making_srt_similar_serach" => {
                let request = serde_json::from_str::<LlmDecisionMakingSrtSimilarSerachRequest>(
                    args.params.get(),
                )
                .map_err(|err| {
                    AcpError::invalid_params().data(format!(
                        "invalid llm/decision_making_srt_similar_serach request: {err}"
                    ))
                })?;
                let response = call_tool_bridge(
                    &self.evt_tx,
                    15,
                    "llm/decision_making_srt_similar_serach",
                    move |reply_tx| AcpToolBridgeRequest::LlmDecisionMakingSrtSimilarSerach {
                        request,
                        reply_tx,
                    },
                )?;
                let raw = json_raw_from_value(&response)?;
                Ok(ExtResponse::new(raw))
            }
            "anica.timeline/get_snapshot" => {
                let request = serde_json::from_str::<TimelineSnapshotRequest>(args.params.get())
                    .map_err(|err| {
                        AcpError::invalid_params()
                            .data(format!("invalid timeline/get_snapshot request: {err}"))
                    })?;
                let response =
                    call_tool_bridge(&self.evt_tx, 8, "timeline/get_snapshot", move |reply_tx| {
                        AcpToolBridgeRequest::GetTimelineSnapshot { request, reply_tx }
                    })?;
                let raw = json_raw_from_value(&response)?;
                Ok(ExtResponse::new(raw))
            }
            "anica.timeline/build_autonomous_edit_plan" => {
                let request = serde_json::from_str::<AutonomousEditPlanRequest>(args.params.get())
                    .map_err(|err| {
                        AcpError::invalid_params().data(format!(
                            "invalid timeline/build_autonomous_edit_plan request: {err}"
                        ))
                    })?;
                let response = call_tool_bridge(
                    &self.evt_tx,
                    90,
                    "timeline/build_autonomous_edit_plan",
                    move |reply_tx| AcpToolBridgeRequest::BuildAutonomousEditPlan {
                        request,
                        reply_tx,
                    },
                )?;
                let raw = json_raw_from_value(&response)?;
                Ok(ExtResponse::new(raw))
            }
            "anica.timeline/get_audio_silence_map" => {
                let request = serde_json::from_str::<AudioSilenceMapRequest>(args.params.get())
                    .map_err(|err| {
                        AcpError::invalid_params().data(format!(
                            "invalid timeline/get_audio_silence_map request: {err}"
                        ))
                    })?;
                let response = call_tool_bridge(
                    &self.evt_tx,
                    90,
                    "timeline/get_audio_silence_map",
                    move |reply_tx| AcpToolBridgeRequest::GetAudioSilenceMap { request, reply_tx },
                )?;
                let raw = json_raw_from_value(&response)?;
                Ok(ExtResponse::new(raw))
            }
            "anica.timeline/build_audio_silence_cut_plan" => {
                let request = serde_json::from_str::<AudioSilenceCutPlanRequest>(args.params.get())
                    .map_err(|err| {
                        AcpError::invalid_params().data(format!(
                            "invalid timeline/build_audio_silence_cut_plan request: {err}"
                        ))
                    })?;
                let response = call_tool_bridge(
                    &self.evt_tx,
                    90,
                    "timeline/build_audio_silence_cut_plan",
                    move |reply_tx| AcpToolBridgeRequest::BuildAudioSilenceCutPlan {
                        request,
                        reply_tx,
                    },
                )?;
                let raw = json_raw_from_value(&response)?;
                Ok(ExtResponse::new(raw))
            }
            "anica.timeline/get_transcript_low_confidence_map" => {
                let request =
                    serde_json::from_str::<TranscriptLowConfidenceMapRequest>(args.params.get())
                        .map_err(|err| {
                            AcpError::invalid_params().data(format!(
                                "invalid timeline/get_transcript_low_confidence_map request: {err}"
                            ))
                        })?;
                let response = call_tool_bridge(
                    &self.evt_tx,
                    30,
                    "timeline/get_transcript_low_confidence_map",
                    move |reply_tx| AcpToolBridgeRequest::GetTranscriptLowConfidenceMap {
                        request,
                        reply_tx,
                    },
                )?;
                let raw = json_raw_from_value(&response)?;
                Ok(ExtResponse::new(raw))
            }
            "anica.timeline/build_transcript_low_confidence_cut_plan" => {
                let request = serde_json::from_str::<TranscriptLowConfidenceCutPlanRequest>(
                    args.params.get(),
                )
                .map_err(|err| {
                    AcpError::invalid_params().data(format!(
                        "invalid timeline/build_transcript_low_confidence_cut_plan request: {err}"
                    ))
                })?;
                let response = call_tool_bridge(
                    &self.evt_tx,
                    45,
                    "timeline/build_transcript_low_confidence_cut_plan",
                    move |reply_tx| AcpToolBridgeRequest::BuildTranscriptLowConfidenceCutPlan {
                        request,
                        reply_tx,
                    },
                )?;
                let raw = json_raw_from_value(&response)?;
                Ok(ExtResponse::new(raw))
            }
            "anica.timeline/get_subtitle_gap_map" => {
                let request = serde_json::from_str::<SubtitleGapMapRequest>(args.params.get())
                    .map_err(|err| {
                        AcpError::invalid_params().data(format!(
                            "invalid timeline/get_subtitle_gap_map request: {err}"
                        ))
                    })?;
                let response = call_tool_bridge(
                    &self.evt_tx,
                    10,
                    "timeline/get_subtitle_gap_map",
                    move |reply_tx| AcpToolBridgeRequest::GetSubtitleGapMap { request, reply_tx },
                )?;
                let raw = json_raw_from_value(&response)?;
                Ok(ExtResponse::new(raw))
            }
            "anica.timeline/build_subtitle_gap_cut_plan" => {
                let request = serde_json::from_str::<SubtitleGapCutPlanRequest>(args.params.get())
                    .map_err(|err| {
                        AcpError::invalid_params().data(format!(
                            "invalid timeline/build_subtitle_gap_cut_plan request: {err}"
                        ))
                    })?;
                let response = call_tool_bridge(
                    &self.evt_tx,
                    90,
                    "timeline/build_subtitle_gap_cut_plan",
                    move |reply_tx| AcpToolBridgeRequest::BuildSubtitleGapCutPlan {
                        request,
                        reply_tx,
                    },
                )?;
                let raw = json_raw_from_value(&response)?;
                Ok(ExtResponse::new(raw))
            }
            "anica.timeline/get_subtitle_semantic_repeats" => {
                let request =
                    serde_json::from_str::<SubtitleSemanticRepeatsRequest>(args.params.get())
                        .map_err(|err| {
                            AcpError::invalid_params().data(format!(
                                "invalid timeline/get_subtitle_semantic_repeats request: {err}"
                            ))
                        })?;
                let response = call_tool_bridge(
                    &self.evt_tx,
                    12,
                    "timeline/get_subtitle_semantic_repeats",
                    move |reply_tx| AcpToolBridgeRequest::GetSubtitleSemanticRepeats {
                        request,
                        reply_tx,
                    },
                )?;
                let raw = json_raw_from_value(&response)?;
                Ok(ExtResponse::new(raw))
            }
            "anica.timeline/validate_edit_plan" => {
                let request = serde_json::from_str::<TimelineEditPlanRequest>(args.params.get())
                    .map_err(|err| {
                        AcpError::invalid_params().data(format!(
                            "invalid timeline/validate_edit_plan request: {err}"
                        ))
                    })?;
                let response = call_tool_bridge(
                    &self.evt_tx,
                    8,
                    "timeline/validate_edit_plan",
                    move |reply_tx| AcpToolBridgeRequest::ValidateEditPlan { request, reply_tx },
                )?;
                let raw = json_raw_from_value(&response)?;
                Ok(ExtResponse::new(raw))
            }
            "anica.timeline/apply_edit_plan" => {
                let request = serde_json::from_str::<TimelineEditPlanRequest>(args.params.get())
                    .map_err(|err| {
                        AcpError::invalid_params()
                            .data(format!("invalid timeline/apply_edit_plan request: {err}"))
                    })?;
                let response = call_tool_bridge(
                    &self.evt_tx,
                    15,
                    "timeline/apply_edit_plan",
                    move |reply_tx| AcpToolBridgeRequest::ApplyEditPlan { request, reply_tx },
                )?;
                let raw = json_raw_from_value(&response)?;
                Ok(ExtResponse::new(raw))
            }
            "anica.export/run" => {
                let request = serde_json::from_str::<AcpExportRunRequest>(args.params.get())
                    .map_err(|err| {
                        AcpError::invalid_params()
                            .data(format!("invalid export/run request: {err}"))
                    })?;
                let response = call_tool_bridge(&self.evt_tx, 20, "export/run", move |reply_tx| {
                    AcpToolBridgeRequest::RunExport { request, reply_tx }
                })?;
                let raw = json_raw_from_value(&response)?;
                Ok(ExtResponse::new(raw))
            }
            "anica.docs/list_files" => {
                let request =
                    serde_json::from_str::<DocsListRequest>(args.params.get()).map_err(|err| {
                        AcpError::invalid_params()
                            .data(format!("invalid docs/list_files request: {err}"))
                    })?;
                let response = list_docs_files(request)
                    .map_err(|err| AcpError::internal_error().data(err.to_string()))?;
                let raw = json_raw_from_value(&response)?;
                Ok(ExtResponse::new(raw))
            }
            "anica.docs/read_file" => {
                let request =
                    serde_json::from_str::<DocsReadRequest>(args.params.get()).map_err(|err| {
                        AcpError::invalid_params()
                            .data(format!("invalid docs/read_file request: {err}"))
                    })?;
                let response = read_docs_file(request)
                    .map_err(|err| AcpError::internal_error().data(err.to_string()))?;
                let raw = json_raw_from_value(&response)?;
                Ok(ExtResponse::new(raw))
            }
            _ => {
                let raw = RawValue::from_string("null".to_string()).map_err(|err| {
                    AcpError::internal_error().data(format!("ext null build failed: {err}"))
                })?;
                Ok(ExtResponse::new(raw.into()))
            }
        }
    }
}

fn text_from_chunk(chunk: &ContentChunk) -> Option<String> {
    match &chunk.content {
        ContentBlock::Text(text) => {
            let s = text.text.clone();
            if s.is_empty() { None } else { Some(s) }
        }
        _ => None,
    }
}

fn worker_loop(cmd_rx: Receiver<AcpWorkerCommand>, evt_tx: Sender<AcpUiEvent>) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            let _ = evt_tx.send(AcpUiEvent::Error(format!(
                "Failed to start async runtime: {err}"
            )));
            return;
        }
    };
    let local = LocalSet::new();
    let shared_state = Arc::new(Mutex::new(AcpSharedState::default()));

    let mut session: Option<AcpSession> = None;

    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            AcpWorkerCommand::Connect { command, cwd, env } => {
                if let Some(mut current) = session.take() {
                    local.block_on(&runtime, current.shutdown());
                }

                let _ = evt_tx.send(AcpUiEvent::Status(format!(
                    "Connecting ACP agent: {command}"
                )));

                match local.block_on(
                    &runtime,
                    AcpSession::connect(command, cwd, env, evt_tx.clone(), shared_state.clone()),
                ) {
                    Ok(new_session) => {
                        session = Some(new_session);
                    }
                    Err(err) => {
                        let _ = evt_tx.send(AcpUiEvent::Error(err.to_string()));
                        let _ = evt_tx.send(AcpUiEvent::Disconnected {
                            reason: "Not connected".to_string(),
                        });
                    }
                }
            }
            AcpWorkerCommand::SendPrompt { prompt } => {
                let Some(current) = session.as_ref() else {
                    let _ = evt_tx.send(AcpUiEvent::Error(
                        "ACP agent is not connected. Connect first.".to_string(),
                    ));
                    continue;
                };

                let prompt_timeout_secs = std::env::var("ANICA_ACP_PROMPT_TIMEOUT_SEC")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .filter(|v| *v > 0)
                    .unwrap_or(600);

                match local.block_on(&runtime, async {
                    timeout(
                        Duration::from_secs(prompt_timeout_secs),
                        current.prompt(prompt),
                    )
                    .await
                }) {
                    Ok(Ok(stop_reason)) => {
                        let _ = evt_tx.send(AcpUiEvent::PromptFinished { stop_reason });
                    }
                    Ok(Err(err)) => {
                        let _ = evt_tx.send(AcpUiEvent::Error(err.to_string()));
                        let _ = evt_tx.send(AcpUiEvent::PromptFinished {
                            stop_reason: "Error".to_string(),
                        });
                    }
                    Err(_) => {
                        local.block_on(&runtime, current.cancel_prompt());
                        let timeout_msg = format!(
                            "Prompt timed out after {}s and was cancelled.",
                            prompt_timeout_secs
                        );
                        let _ = evt_tx.send(AcpUiEvent::Status(timeout_msg.clone()));
                        let _ = evt_tx.send(AcpUiEvent::Error(timeout_msg));
                        let _ = evt_tx.send(AcpUiEvent::PromptFinished {
                            stop_reason: "Timeout".to_string(),
                        });
                    }
                }
            }
            AcpWorkerCommand::UpdateMediaPoolSnapshot {
                items,
                ffmpeg_available,
                ffprobe_available,
                ffprobe_command,
            } => {
                if let Ok(mut state) = shared_state.lock() {
                    state.media_pool = items;
                    state.ffmpeg_available = ffmpeg_available;
                    state.ffprobe_available = ffprobe_available;
                    state.ffprobe_command = ffprobe_command;
                }
            }
            AcpWorkerCommand::Disconnect => {
                if let Some(mut current) = session.take() {
                    let command = current.command.clone();
                    local.block_on(&runtime, current.shutdown());
                    let _ = evt_tx.send(AcpUiEvent::Disconnected {
                        reason: format!("Disconnected: {command}"),
                    });
                } else {
                    let _ = evt_tx.send(AcpUiEvent::Disconnected {
                        reason: "Already disconnected".to_string(),
                    });
                }
            }
            AcpWorkerCommand::Shutdown => {
                if let Some(mut current) = session.take() {
                    local.block_on(&runtime, current.shutdown());
                }
                break;
            }
        }
    }
}
