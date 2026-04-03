// =========================================
// =========================================
// src/core/media_tools.rs
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;

use crate::runtime_paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostPlatform {
    MacOS,
    Windows,
    Linux,
    Other,
}

impl HostPlatform {
    pub fn detect() -> Self {
        match std::env::consts::OS {
            "macos" => HostPlatform::MacOS,
            "windows" => HostPlatform::Windows,
            "linux" => HostPlatform::Linux,
            _ => HostPlatform::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            HostPlatform::MacOS => "macOS",
            HostPlatform::Windows => "Windows",
            HostPlatform::Linux => "Linux",
            HostPlatform::Other => "Other",
        }
    }

    pub fn install_commands(self) -> &'static [(&'static str, &'static str)] {
        match self {
            HostPlatform::MacOS => &[("Homebrew", "brew install ffmpeg")],
            HostPlatform::Windows => &[("winget", "winget install --id Gyan.FFmpeg -e")],
            HostPlatform::Linux => &[
                ("Debian/Ubuntu", "sudo apt install ffmpeg"),
                ("Fedora", "sudo dnf install ffmpeg"),
                ("Arch", "sudo pacman -S ffmpeg"),
            ],
            HostPlatform::Other => &[],
        }
    }

    pub fn gstreamer_install_commands(self) -> &'static [(&'static str, &'static str)] {
        match self {
            HostPlatform::MacOS => &[(
                "Latest version",
                "brew install gstreamer gst-plugins-base gst-plugins-good gst-editing-services",
            )],
            HostPlatform::Windows => &[("winget", "winget install --id GStreamer.GStreamer -e")],
            HostPlatform::Linux => &[(
                "Debian/Ubuntu",
                "sudo apt install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-editing-services libges-1.0-dev",
            )],
            HostPlatform::Other => &[],
        }
    }
}

#[derive(Debug, Clone)]
pub struct MediaDependencyStatus {
    pub host: HostPlatform,
    pub ffmpeg_command: String,
    pub ffprobe_command: String,
    pub ffmpeg_available: bool,
    pub ffprobe_available: bool,
    pub ffmpeg_version: Option<String>,
    pub ffprobe_version: Option<String>,
}

impl Default for MediaDependencyStatus {
    fn default() -> Self {
        Self {
            host: HostPlatform::detect(),
            ffmpeg_command: "ffmpeg".to_string(),
            ffprobe_command: "ffprobe".to_string(),
            ffmpeg_available: false,
            ffprobe_available: false,
            ffmpeg_version: None,
            ffprobe_version: None,
        }
    }
}

impl MediaDependencyStatus {
    pub fn all_available(&self) -> bool {
        self.ffmpeg_available && self.ffprobe_available
    }

    pub fn missing_tools(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.ffmpeg_available {
            missing.push("ffmpeg");
        }
        if !self.ffprobe_available {
            missing.push("ffprobe");
        }
        missing
    }
}

#[derive(Debug, Error)]
pub enum MediaBootstrapError {
    #[error("Failed to launch bootstrap script: {source}")]
    LaunchBootstrapScript { source: std::io::Error },
    #[error("Bootstrap script failed (status {status}): {reason}")]
    BootstrapScriptFailed { status: String, reason: String },
    #[error("Bootstrap script not found: {path}")]
    BootstrapScriptNotFound { path: PathBuf },
    #[error("Windows bootstrap failed via pwsh/powershell. {details}")]
    WindowsBootstrapFailed { details: String },
    #[error("Unsupported host platform for runtime bootstrap.")]
    UnsupportedHostPlatform,
}

fn first_non_empty_line(raw: &str) -> Option<String> {
    raw.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToString::to_string)
}

fn bundle_tool_candidate(tool_name: &str) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }

    let resources_dir = runtime_paths::bundle_resources_dir()?;
    let runtime_root = runtime_paths::bundle_runtime_root();
    let candidates = [
        resources_dir.join(tool_name),
        runtime_root
            .as_ref()
            .map(|root| root.join("ffmpeg").join("bin").join(tool_name))
            .unwrap_or_default(),
        runtime_root
            .as_ref()
            .map(|root| root.join("gstreamer").join("bin").join(tool_name))
            .unwrap_or_default(),
    ];
    for candidate in candidates {
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

fn ffmpeg_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

fn gst_launch_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "gst-launch-1.0.exe"
    } else {
        "gst-launch-1.0"
    }
}

fn common_host_gstreamer_candidates() -> &'static [&'static str] {
    #[cfg(target_os = "macos")]
    {
        &[
            "/opt/homebrew/bin/gst-launch-1.0",
            "/usr/local/bin/gst-launch-1.0",
        ]
    }
    #[cfg(target_os = "linux")]
    {
        &["/usr/bin/gst-launch-1.0", "/usr/local/bin/gst-launch-1.0"]
    }
    #[cfg(target_os = "windows")]
    {
        &[]
    }
}

fn configured_tools_home() -> Option<PathBuf> {
    std::env::var("ANICA_TOOLS_HOME").ok().map(PathBuf::from)
}

fn default_tools_home() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            return Some(PathBuf::from(local).join("Anica").join("tools"));
        }
        if let Ok(profile) = std::env::var("USERPROFILE") {
            return Some(PathBuf::from(profile).join(".anica").join("tools"));
        }
        None
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME")
            .ok()
            .map(|home| PathBuf::from(home).join(".anica").join("tools"))
    }
}

fn local_ffmpeg_tool_candidate(root: &PathBuf) -> Option<String> {
    let direct = root.join("ffmpeg").join("bin").join(ffmpeg_binary_name());
    if direct.is_file() {
        return Some(direct.to_string_lossy().to_string());
    }
    let current = root
        .join("ffmpeg")
        .join("current")
        .join("bin")
        .join(ffmpeg_binary_name());
    if current.is_file() {
        return Some(current.to_string_lossy().to_string());
    }
    None
}

fn local_gstreamer_tool_candidate(root: &PathBuf) -> Option<String> {
    let direct = root
        .join("gstreamer")
        .join("bin")
        .join(gst_launch_binary_name());
    if direct.is_file() {
        return Some(direct.to_string_lossy().to_string());
    }
    let current = root
        .join("gstreamer")
        .join("current")
        .join("bin")
        .join(gst_launch_binary_name());
    if current.is_file() {
        return Some(current.to_string_lossy().to_string());
    }
    None
}

fn workspace_ffmpeg_tool_candidate() -> Option<String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let platform = format!("{os}-{arch}");
    let runtime_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tools")
        .join("runtime");
    let roots = [
        runtime_root.join(&platform),
        runtime_root.join(os),
        runtime_root.join("current").join(&platform),
        runtime_root.join("current").join(os),
        runtime_root.join("current"),
    ];
    for root in roots {
        let direct = root.join("ffmpeg").join("bin").join(ffmpeg_binary_name());
        if direct.is_file() {
            return Some(direct.to_string_lossy().to_string());
        }
    }
    None
}

fn workspace_gstreamer_tool_candidate() -> Option<String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let platform = format!("{os}-{arch}");
    let runtime_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tools")
        .join("runtime");
    let roots = [
        runtime_root.join(&platform),
        runtime_root.join(os),
        runtime_root.join("current").join(&platform),
        runtime_root.join("current").join(os),
        runtime_root.join("current"),
    ];
    for root in roots {
        let direct = root
            .join("gstreamer")
            .join("bin")
            .join(gst_launch_binary_name());
        if direct.is_file() {
            return Some(direct.to_string_lossy().to_string());
        }
    }
    None
}

fn workspace_runtime_current_home() -> PathBuf {
    let os = std::env::consts::OS;
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tools")
        .join("runtime")
        .join("current")
        .join(os)
}

fn prepend_env_path_var(name: &str, path: PathBuf) {
    if !path.is_dir() {
        return;
    }
    let mut values = vec![path.to_string_lossy().to_string()];
    if let Some(existing) = std::env::var_os(name)
        && !existing.is_empty()
    {
        values.push(existing.to_string_lossy().to_string());
    }
    // SAFETY: This is called during process startup on the main thread before
    // media runtime initialization, so mutating process env is safe here.
    unsafe {
        std::env::set_var(name, values.join(":"));
    }
}

fn set_env_if_missing(name: &str, value: &str) {
    if std::env::var_os(name).is_some() {
        return;
    }
    // SAFETY: Startup-only env mutation before worker threads/media init.
    unsafe {
        std::env::set_var(name, value);
    }
}

pub fn configure_bundled_media_runtime_environment() {
    let Some(runtime_root) = runtime_paths::bundle_runtime_root() else {
        return;
    };

    let ffmpeg_root = [
        runtime_root.join("ffmpeg").join("current"),
        runtime_root.join("ffmpeg"),
    ]
    .into_iter()
    .find(|candidate| candidate.join("bin").join(ffmpeg_binary_name()).is_file())
    .unwrap_or_else(|| runtime_root.join("ffmpeg"));
    let ffmpeg_bin = ffmpeg_root.join("bin").join(ffmpeg_binary_name());
    let ffprobe_bin = ffmpeg_root
        .join("bin")
        .join(if cfg!(target_os = "windows") {
            "ffprobe.exe"
        } else {
            "ffprobe"
        });
    let ffmpeg_lib = ffmpeg_root.join("lib");
    let gst_bin = runtime_root.join("gstreamer").join("bin");
    let gst_lib = runtime_root.join("gstreamer").join("lib");
    let gst_plugins = gst_lib.join("gstreamer-1.0");
    let gst_typelib = gst_lib.join("girepository-1.0");
    let gst_scanner = runtime_root
        .join("gstreamer")
        .join("libexec")
        .join("gstreamer-1.0")
        .join("gst-plugin-scanner");

    set_env_if_missing("ANICA_MEDIA_RUNTIME_STRICT", "1");
    set_env_if_missing("ANICA_ALLOW_SYSTEM_MEDIA", "0");
    // Keep ANICA_TOOLS_HOME aligned with the runtime root layout used elsewhere.
    // SAFETY: Startup-only env mutation before media initialization.
    unsafe {
        std::env::set_var("ANICA_TOOLS_HOME", &runtime_root);
    }

    if ffmpeg_bin.is_file() {
        // SAFETY: Startup-only env mutation before media initialization.
        unsafe {
            std::env::set_var("ANICA_FFMPEG_PATH", &ffmpeg_bin);
            if ffprobe_bin.is_file() {
                std::env::set_var("ANICA_FFPROBE_PATH", &ffprobe_bin);
            }
        }
        prepend_env_path_var(
            "PATH",
            ffmpeg_bin.parent().unwrap_or(&ffmpeg_root).to_path_buf(),
        );
        if ffprobe_bin.is_file() {
            prepend_env_path_var(
                "PATH",
                ffprobe_bin.parent().unwrap_or(&ffmpeg_root).to_path_buf(),
            );
        }
    }

    if gst_bin.is_dir() {
        prepend_env_path_var("PATH", gst_bin.clone());
    }
    let gst_launch = gst_bin.join(gst_launch_binary_name());
    if gst_launch.is_file() {
        // SAFETY: Startup-only env mutation before media initialization.
        unsafe {
            std::env::set_var("ANICA_GSTREAMER_PATH", &gst_launch);
        }
    }

    if gst_plugins.is_dir() {
        let plugin_path = gst_plugins.to_string_lossy().to_string();
        for name in [
            "GST_PLUGIN_PATH",
            "GST_PLUGIN_PATH_1_0",
            "GST_PLUGIN_SYSTEM_PATH_1_0",
            "GST_PLUGIN_SYSTEM_PATH",
        ] {
            // SAFETY: Startup-only env mutation before media initialization.
            unsafe {
                std::env::set_var(name, &plugin_path);
            }
        }
    }

    if gst_scanner.is_file() {
        // SAFETY: Startup-only env mutation before media initialization.
        unsafe {
            std::env::set_var("GST_PLUGIN_SCANNER", &gst_scanner);
            std::env::set_var("GST_PLUGIN_SCANNER_1_0", &gst_scanner);
        }
    }

    if gst_typelib.is_dir() {
        prepend_env_path_var("GI_TYPELIB_PATH", gst_typelib);
    }

    if runtime_root.join("gstreamer").is_dir() {
        let cache_dir = runtime_root.join("gstreamer").join("cache");
        let _ = std::fs::create_dir_all(&cache_dir);
        // SAFETY: Startup-only env mutation before media initialization.
        unsafe {
            std::env::set_var("GST_REGISTRY_1_0", cache_dir.join("registry.bin"));
        }
    }

    if cfg!(target_os = "macos") {
        let mut dyld_values = Vec::new();
        if gst_lib.is_dir() {
            dyld_values.push(gst_lib.to_string_lossy().to_string());
        }
        if ffmpeg_lib.is_dir() {
            dyld_values.push(ffmpeg_lib.to_string_lossy().to_string());
        }
        if let Some(existing) = std::env::var_os("DYLD_FALLBACK_LIBRARY_PATH")
            && !existing.is_empty()
        {
            dyld_values.push(existing.to_string_lossy().to_string());
        }
        if !dyld_values.is_empty() {
            // SAFETY: Startup-only env mutation before media initialization.
            unsafe {
                std::env::set_var("DYLD_FALLBACK_LIBRARY_PATH", dyld_values.join(":"));
            }
        }
    }
}

fn probe_tool_version(command: &str) -> Option<String> {
    let out = Command::new(command).arg("-version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    first_non_empty_line(&stdout).or_else(|| first_non_empty_line(&stderr))
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if value.trim().is_empty() || values.iter().any(|v| v == &value) {
        return;
    }
    values.push(value);
}

static AUTO_BOOTSTRAP_ATTEMPTED: AtomicBool = AtomicBool::new(false);

fn env_flag_true(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|raw| {
            let normalized = raw.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

fn env_flag_false(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|raw| {
            let normalized = raw.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "0" | "false" | "no" | "off")
        })
        .unwrap_or(false)
}

fn auto_bootstrap_enabled() -> bool {
    if env_flag_false("ANICA_RUNTIME_AUTO_DOWNLOAD") {
        return false;
    }
    if env_flag_true("ANICA_DISABLE_RUNTIME_AUTO_DOWNLOAD") {
        return false;
    }
    true
}

fn runtime_strict_pinned_enabled() -> bool {
    if env_flag_false("ANICA_MEDIA_RUNTIME_STRICT") {
        return false;
    }
    true
}

fn system_media_fallback_enabled() -> bool {
    if !runtime_strict_pinned_enabled() {
        return true;
    }
    env_flag_true("ANICA_ALLOW_SYSTEM_MEDIA")
}

fn setup_media_tools_script_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("setup_media_tools.sh")
}

fn setup_media_tools_windows_script_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("setup_media_tools.ps1")
}

fn extract_bootstrap_error_reason(stderr: &str, stdout: &str) -> String {
    // Prefer a line that looks like an actual error over a warning.
    let is_error_line = |l: &str| -> bool {
        let ll = l.to_ascii_lowercase();
        (ll.contains("error:") || ll.contains("fatal:") || ll.contains("failed"))
            && !ll.contains("warning:")
    };
    let primary = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && is_error_line(l))
        .last()
        .or_else(|| {
            // Fall back to last non-empty stderr line (avoids showing first compiler warning).
            stderr
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .last()
        })
        .map(ToString::to_string)
        .or_else(|| first_non_empty_line(stdout))
        .unwrap_or_else(|| "unknown bootstrap error".to_string());

    // Append last few non-empty stderr lines for context (skip duplicates of primary).
    let tail: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != primary.as_str())
        .rev()
        .take(4)
        .collect();
    if !tail.is_empty() {
        let mut reversed = tail;
        reversed.reverse();
        format!("{primary} | stderr: {}", reversed.join(" | "))
    } else {
        primary
    }
}

fn run_bootstrap_command(mut cmd: Command) -> Result<(), MediaBootstrapError> {
    let output = cmd
        .output()
        .map_err(|source| MediaBootstrapError::LaunchBootstrapScript { source })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let reason = extract_bootstrap_error_reason(&stderr, &stdout);
        return Err(MediaBootstrapError::BootstrapScriptFailed {
            status: output.status.to_string(),
            reason,
        });
    }
    Ok(())
}

fn run_posix_lgpl_bootstrap_script(
    _host: HostPlatform,
    tools_home: &PathBuf,
    skip_ffmpeg: bool,
) -> Result<(), MediaBootstrapError> {
    let script = setup_media_tools_script_path();
    if !script.is_file() {
        return Err(MediaBootstrapError::BootstrapScriptNotFound { path: script });
    }
    let mut cmd = Command::new("bash");
    cmd.arg(script.as_os_str())
        .arg("--mode")
        .arg("local-lgpl")
        .arg("--yes")
        .env("ANICA_TOOLS_HOME", tools_home.as_os_str());
    if skip_ffmpeg {
        cmd.arg("--skip-ffmpeg");
    }
    run_bootstrap_command(cmd)
}

fn run_windows_lgpl_bootstrap_script(tools_home: &PathBuf) -> Result<(), MediaBootstrapError> {
    let script = setup_media_tools_windows_script_path();
    if !script.is_file() {
        return Err(MediaBootstrapError::BootstrapScriptNotFound { path: script });
    }
    let mut tried = Vec::new();
    for shell in ["pwsh", "powershell"] {
        let mut cmd = Command::new(shell);
        cmd.arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(script.as_os_str())
            .arg("-Mode")
            .arg("local-lgpl")
            .arg("-InstallGStreamer")
            .arg("-Yes")
            .arg("-ToolsHome")
            .arg(tools_home.as_os_str());
        match run_bootstrap_command(cmd) {
            Ok(()) => return Ok(()),
            Err(err) => tried.push(format!("{shell}: {err}")),
        }
    }
    Err(MediaBootstrapError::WindowsBootstrapFailed {
        details: tried.join(" | "),
    })
}

fn bootstrap_missing_media_runtime(
    host: HostPlatform,
    tools_home: &PathBuf,
    skip_ffmpeg: bool,
) -> Result<(), MediaBootstrapError> {
    match host {
        HostPlatform::MacOS | HostPlatform::Linux => {
            run_posix_lgpl_bootstrap_script(host, tools_home, skip_ffmpeg)
        }
        HostPlatform::Windows => run_windows_lgpl_bootstrap_script(tools_home),
        HostPlatform::Other => Err(MediaBootstrapError::UnsupportedHostPlatform),
    }
}

pub fn ffprobe_from_ffmpeg(ffmpeg_command: &str) -> String {
    let path = PathBuf::from(ffmpeg_command);
    let lower = ffmpeg_command.to_ascii_lowercase();
    if lower.ends_with("ffmpeg.exe") {
        return path
            .with_file_name("ffprobe.exe")
            .to_string_lossy()
            .to_string();
    }
    if lower.ends_with("ffmpeg") {
        return path.with_file_name("ffprobe").to_string_lossy().to_string();
    }
    "ffprobe".to_string()
}

pub fn detect_media_dependencies(preferred_ffmpeg: Option<&str>) -> MediaDependencyStatus {
    let mut status = MediaDependencyStatus::default();
    status.host = HostPlatform::detect();
    let strict_pinned = runtime_strict_pinned_enabled();

    let preferred = preferred_ffmpeg
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    let mut ffmpeg_candidates = Vec::new();
    if let Some(pref) = preferred.clone() {
        push_unique(&mut ffmpeg_candidates, pref);
    }
    if let Ok(env_ffmpeg) = std::env::var("ANICA_FFMPEG_PATH") {
        push_unique(&mut ffmpeg_candidates, env_ffmpeg);
    }
    if let Some(workspace_ffmpeg) = workspace_ffmpeg_tool_candidate() {
        push_unique(&mut ffmpeg_candidates, workspace_ffmpeg);
    }
    if let Some(configured_home) = configured_tools_home()
        && let Some(local_ffmpeg) = local_ffmpeg_tool_candidate(&configured_home)
    {
        push_unique(&mut ffmpeg_candidates, local_ffmpeg);
    }
    if !strict_pinned
        && let Some(default_home) = default_tools_home()
        && let Some(local_ffmpeg) = local_ffmpeg_tool_candidate(&default_home)
    {
        push_unique(&mut ffmpeg_candidates, local_ffmpeg);
    }
    if let Some(bundle_ffmpeg) = bundle_tool_candidate("ffmpeg") {
        push_unique(&mut ffmpeg_candidates, bundle_ffmpeg);
    }
    if let Some(first_candidate) = ffmpeg_candidates.first() {
        status.ffmpeg_command = first_candidate.clone();
    }

    for candidate in ffmpeg_candidates {
        if let Some(version) = probe_tool_version(&candidate) {
            status.ffmpeg_command = candidate;
            status.ffmpeg_available = true;
            status.ffmpeg_version = Some(version);
            break;
        }
    }

    let mut ffprobe_candidates = Vec::new();
    if let Ok(env_ffprobe) = std::env::var("ANICA_FFPROBE_PATH") {
        push_unique(&mut ffprobe_candidates, env_ffprobe);
    }
    if status.ffmpeg_available {
        let ffprobe_from_selected = ffprobe_from_ffmpeg(&status.ffmpeg_command);
        push_unique(&mut ffprobe_candidates, ffprobe_from_selected);
    }
    if let Some(bundle_ffprobe) = bundle_tool_candidate("ffprobe") {
        push_unique(&mut ffprobe_candidates, bundle_ffprobe);
    }
    if let Some(first_candidate) = ffprobe_candidates.first() {
        status.ffprobe_command = first_candidate.clone();
    }

    for candidate in ffprobe_candidates {
        if let Some(version) = probe_tool_version(&candidate) {
            status.ffprobe_command = candidate;
            status.ffprobe_available = true;
            status.ffprobe_version = Some(version);
            break;
        }
    }

    status
}

pub fn detect_gstreamer_cli(preferred_gstreamer: Option<&str>) -> Option<String> {
    let allow_system_fallback = system_media_fallback_enabled();
    let strict_pinned = runtime_strict_pinned_enabled();

    let preferred = preferred_gstreamer
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    let mut candidates = Vec::new();
    if let Some(pref) = preferred {
        push_unique(&mut candidates, pref);
    }
    if let Ok(env_gst) = std::env::var("ANICA_GSTREAMER_PATH") {
        push_unique(&mut candidates, env_gst);
    }
    if let Some(workspace_gst) = workspace_gstreamer_tool_candidate() {
        push_unique(&mut candidates, workspace_gst);
    }
    if let Some(configured_home) = configured_tools_home()
        && let Some(local_gst) = local_gstreamer_tool_candidate(&configured_home)
    {
        push_unique(&mut candidates, local_gst);
    }
    if !strict_pinned
        && let Some(default_home) = default_tools_home()
        && let Some(local_gst) = local_gstreamer_tool_candidate(&default_home)
    {
        push_unique(&mut candidates, local_gst);
    }
    if let Some(bundle_gst) = bundle_tool_candidate(gst_launch_binary_name()) {
        push_unique(&mut candidates, bundle_gst);
    }
    for candidate in common_host_gstreamer_candidates() {
        push_unique(&mut candidates, (*candidate).to_string());
    }
    if allow_system_fallback {
        push_unique(&mut candidates, gst_launch_binary_name().to_string());
    }

    for candidate in candidates {
        let looks_like_path =
            candidate.contains('/') || candidate.contains('\\') || candidate.ends_with(".exe");
        if looks_like_path && PathBuf::from(&candidate).is_file() {
            return Some(candidate);
        }
        if probe_tool_version(&candidate).is_some() {
            return Some(candidate);
        }
    }
    None
}

pub fn detect_or_bootstrap_media_dependencies(
    preferred_ffmpeg: Option<&str>,
) -> MediaDependencyStatus {
    let status = detect_media_dependencies(preferred_ffmpeg);
    let gst_available = detect_gstreamer_cli(None).is_some();
    if status.all_available() {
        if !gst_available {
            eprintln!(
                "[System Check] GStreamer runtime bootstrap is disabled. Using host GStreamer only."
            );
        }
        return status;
    }
    if !auto_bootstrap_enabled() {
        return status;
    }
    if AUTO_BOOTSTRAP_ATTEMPTED.swap(true, Ordering::SeqCst) {
        return status;
    }
    let tools_home = configured_tools_home().unwrap_or_else(workspace_runtime_current_home);

    let skip_ffmpeg = false;
    eprintln!(
        "[System Check] ffmpeg/ffprobe missing. Starting first-run LGPL runtime bootstrap..."
    );
    if let Err(err) = bootstrap_missing_media_runtime(status.host, &tools_home, skip_ffmpeg) {
        let refreshed = detect_media_dependencies(preferred_ffmpeg);
        if refreshed.all_available() {
            // Tools are usable despite the script exit error (e.g. -version probe crash
            // after a successful build).
            eprintln!(
                "[System Check] Bootstrap script exited with error but required tools are available. ffmpeg: {}",
                refreshed.ffmpeg_command
            );
            eprintln!("[System Check] Bootstrap error detail: {err}");
            return refreshed;
        }
        eprintln!("[System Check] Runtime bootstrap failed: {err}");
        return status;
    }

    let refreshed = detect_media_dependencies(preferred_ffmpeg);
    let refreshed_gst = detect_gstreamer_cli(None);
    if refreshed.all_available() {
        eprintln!(
            "[System Check] Runtime bootstrap completed. ffmpeg: {}",
            refreshed.ffmpeg_command
        );
        if let Some(gst_cli) = refreshed_gst {
            eprintln!(
                "[System Check] GStreamer detected from host/system: {}",
                gst_cli
            );
        } else {
            eprintln!(
                "[System Check] GStreamer not detected after bootstrap (expected: host/system-managed)."
            );
        }
    } else {
        eprintln!(
            "[System Check] Runtime bootstrap finished but ffmpeg/ffprobe are still missing."
        );
    }
    refreshed
}
