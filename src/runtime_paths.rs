#![allow(dead_code)]

use std::path::PathBuf;

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if path.as_os_str().is_empty() || paths.iter().any(|existing| existing == &path) {
        return;
    }
    paths.push(path);
}

pub fn current_exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
}

pub fn bundle_resources_dir() -> Option<PathBuf> {
    let exe_dir = current_exe_dir()?;
    let exe_name = exe_dir.file_name().and_then(|v| v.to_str());
    if exe_name == Some("Resources") {
        let parent = exe_dir.parent()?;
        if parent.file_name().and_then(|v| v.to_str()) == Some("Contents") {
            return Some(exe_dir);
        }
    }
    if let Some(contents_dir) = exe_dir.parent()
        && contents_dir.file_name().and_then(|v| v.to_str()) == Some("Contents")
    {
        return Some(contents_dir.join("Resources"));
    }
    None
}

pub fn bundle_runtime_root() -> Option<PathBuf> {
    let resources = bundle_resources_dir()?;
    let os = std::env::consts::OS;
    let candidate = resources.join("runtime").join("current").join(os);
    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

pub fn candidate_asset_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Some(from_env) = std::env::var_os("ANICA_ASSETS_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
    {
        push_unique(&mut roots, from_env);
    }

    if let Some(exe_dir) = current_exe_dir() {
        push_unique(&mut roots, exe_dir.join("assets"));
    }
    if let Some(resources_dir) = bundle_resources_dir() {
        push_unique(&mut roots, resources_dir.join("assets"));
    }

    push_unique(
        &mut roots,
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets"),
    );

    if let Ok(cwd) = std::env::current_dir() {
        push_unique(&mut roots, cwd.join("assets"));
        push_unique(&mut roots, cwd.join("anica").join("assets"));
    }

    roots
}

pub fn resolve_asset_root() -> PathBuf {
    candidate_asset_roots()
        .into_iter()
        .find(|path| path.is_dir())
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets"))
}

pub fn candidate_font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(from_env) = std::env::var_os("ANICA_FONTS_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
    {
        push_unique(&mut dirs, from_env);
    }

    for root in candidate_asset_roots() {
        push_unique(&mut dirs, root.join("fonts"));
    }

    dirs
}

pub fn candidate_twemoji_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(from_env) = std::env::var_os("ANICA_TWEMOJI_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
    {
        push_unique(&mut dirs, from_env);
    }

    for root in candidate_asset_roots() {
        push_unique(&mut dirs, root.join("twemoji").join("72x72"));
    }

    push_unique(
        &mut dirs,
        std::env::temp_dir().join("anica_twemoji").join("72x72"),
    );
    dirs
}

pub fn candidate_docs_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Some(from_env) = std::env::var_os("ANICA_DOCS_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
    {
        push_unique(&mut roots, from_env);
    }

    if let Some(exe_dir) = current_exe_dir() {
        push_unique(&mut roots, exe_dir.join("docs"));
    }
    if let Some(resources_dir) = bundle_resources_dir() {
        push_unique(&mut roots, resources_dir.join("docs"));
    }

    push_unique(
        &mut roots,
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs"),
    );
    push_unique(
        &mut roots,
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs"),
    );

    if let Ok(cwd) = std::env::current_dir() {
        push_unique(&mut roots, cwd.join("docs"));
        push_unique(&mut roots, cwd.join("anica").join("docs"));
        if let Some(parent) = cwd.parent() {
            push_unique(&mut roots, parent.join("docs"));
        }
    }

    roots
}
