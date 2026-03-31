// =========================================
// =========================================
// src/app/menu.rs
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{
    collections::HashSet,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use gpui::{
    App, BorrowAppContext, Global, KeyBinding, Menu, MenuItem, OsAction, PathPromptOptions,
    PromptLevel, Subscription, Timer, actions,
};

use crate::core::global_state::GlobalState;
use crate::core::media_tools::detect_media_dependencies;
use crate::core::project_state::{
    ProjectState, autosave_dir, clear_all_recovery_drafts, clear_recovery_draft,
    default_project_dir, find_missing_media, latest_recovery_draft, load_project_from_path,
    remap_project_paths, save_project_snapshot, save_project_to_path, save_recovery_draft,
};

actions!(
    anica_menu,
    [
        UndoAction,
        RedoAction,
        SaveAction,
        SaveAsAction,
        OpenAction,
        NewAction,
        RevealInFinderExplorerAction,
        OpenMemoryPreferencesAction,
        TogglePreviewRenderModeAction
    ]
);

pub struct AppMenuState {
    pub global: gpui::Entity<GlobalState>,
    pub last_project_path: Option<PathBuf>,
    pub last_saved_signature: Option<u64>,
    pub last_recovery_signature: Option<u64>,
    pub quit_cleanup_sub: Option<Subscription>,
}

impl Global for AppMenuState {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PostSaveAction {
    None,
    CreateNewProject,
}

fn project_signature(gs: &GlobalState) -> u64 {
    let state = ProjectState::from_global(gs);
    project_state_signature(&state)
}

fn project_state_signature(state: &ProjectState) -> u64 {
    let mut state = state.clone();
    // Dirty-check should track timeline/content edits, not volatile metadata.
    state.meta.created_at = 0;
    state.meta.last_opened = 0;
    state.ui = None;

    let mut hasher = DefaultHasher::new();
    match serde_json::to_vec(&state) {
        Ok(bytes) => bytes.hash(&mut hasher),
        Err(err) => {
            eprintln!("[Project] Signature serialization failed: {err}");
            state.canvas.width.to_bits().hash(&mut hasher);
            state.canvas.height.to_bits().hash(&mut hasher);
            state.timeline.v1.len().hash(&mut hasher);
            state.timeline.audio_tracks.len().hash(&mut hasher);
            state.timeline.video_tracks.len().hash(&mut hasher);
            state.timeline.subtitle_tracks.len().hash(&mut hasher);
            state.timeline.next_clip_id.hash(&mut hasher);
            state.timeline.next_subtitle_group_id.hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn create_new_project(cx: &mut App, global: gpui::Entity<GlobalState>) {
    let previous_project_path = global.read(cx).project_file_path.clone();
    // Creating a fresh project intentionally discards the current draft recovery entry.
    if let Err(err) = clear_recovery_draft(previous_project_path.as_deref()) {
        eprintln!("[Project] Failed to clear recovery draft: {err}");
    }

    let preferred_ffmpeg = global.read(cx).ffmpeg_path.clone();
    let media_status = detect_media_dependencies(Some(&preferred_ffmpeg));
    let signature = global.update(cx, |gs, cx| {
        *gs = GlobalState::default();
        gs.apply_media_dependency_status(media_status.clone(), false);
        gs.set_project_file_path(None);
        cx.notify();
        project_signature(gs)
    });

    cx.update_global(|state: &mut AppMenuState, _| {
        state.last_project_path = None;
        state.last_saved_signature = Some(signature);
        state.last_recovery_signature = None;
    });

    println!("[Project] Created new project.");
}

fn save_to_path(cx: &mut App, global: gpui::Entity<GlobalState>, path: PathBuf) -> bool {
    let previous_project_path = global.read(cx).project_file_path.clone();
    let path_for_save = path.clone();
    let saved_signature = global.update(cx, |gs, cx| {
        if let Err(err) = save_project_to_path(gs, &path_for_save) {
            eprintln!("[Project] Save failed: {err}");
            return None;
        }

        println!("[Project] Saved to {}", path_for_save.display());
        gs.set_project_file_path(Some(path_for_save.clone()));
        if let Some(parent) = path_for_save.parent()
            && let Err(err) = save_project_snapshot(gs, autosave_dir(parent), None)
        {
            eprintln!("[Project] Snapshot failed: {err}");
        }
        cx.notify();
        Some(project_signature(gs))
    });

    if let Some(signature) = saved_signature {
        // Successful saves retire both the previous draft key and the current one.
        if let Err(err) = clear_recovery_draft(previous_project_path.as_deref()) {
            eprintln!("[Project] Failed to clear previous recovery draft: {err}");
        }
        if let Err(err) = clear_recovery_draft(Some(path.as_path())) {
            eprintln!("[Project] Failed to clear current recovery draft: {err}");
        }
        cx.update_global(|state: &mut AppMenuState, _| {
            state.last_project_path = Some(path.clone());
            state.last_saved_signature = Some(signature);
            state.last_recovery_signature = None;
        });
        true
    } else {
        false
    }
}

pub fn init_app_menus(cx: &mut App, global: gpui::Entity<GlobalState>) {
    let global_for_init = global.clone();
    cx.on_action(undo_action);
    cx.on_action(redo_action);
    cx.on_action(save_action);
    cx.on_action(save_as_action);
    cx.on_action(open_action);
    cx.on_action(new_action);
    cx.on_action(reveal_in_finder_explorer_action);
    cx.on_action(open_memory_preferences_action);
    cx.on_action(toggle_preview_render_mode_action);

    #[cfg(target_os = "macos")]
    cx.bind_keys([KeyBinding::new("cmd-s", SaveAction, None)]);
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    cx.bind_keys([KeyBinding::new("ctrl-s", SaveAction, None)]);

    // Start with unsaved-project cache scope (shared default cache directory).
    let initial_signature = global_for_init.update(cx, |gs, _| {
        gs.set_project_file_path(None);
        project_signature(gs)
    });

    cx.set_global(AppMenuState {
        global,
        last_project_path: None,
        last_saved_signature: Some(initial_signature),
        last_recovery_signature: None,
        quit_cleanup_sub: None,
    });

    let quit_cleanup_sub = cx.on_app_quit(move |_cx| {
        // Recovery drafts are crash-only safety nets, so graceful quits should retire them all.
        let clear_result = clear_all_recovery_drafts();
        async move {
            if let Err(err) = clear_result {
                eprintln!("[Project] Failed to clear recovery drafts on quit: {err}");
            }
        }
    });
    cx.update_global(|state: &mut AppMenuState, _| {
        state.quit_cleanup_sub = Some(quit_cleanup_sub);
    });

    schedule_recovery_autosave(cx);
    schedule_recovery_prompt(cx);

    cx.set_menus(vec![
        Menu {
            name: "Anica".into(),
            items: vec![MenuItem::action("Open…", OpenAction)],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New", NewAction),
                MenuItem::action("Open…", OpenAction),
                MenuItem::separator(),
                MenuItem::action("Save", SaveAction),
                MenuItem::action("Save As…", SaveAsAction),
                MenuItem::separator(),
                MenuItem::action("Reveal in Finder/Explorer", RevealInFinderExplorerAction),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Undo", UndoAction, OsAction::Undo),
                MenuItem::os_action("Redo", RedoAction, OsAction::Redo),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![MenuItem::action(
                "Switch NV12/BGRA (Cmd/Ctrl+B)",
                TogglePreviewRenderModeAction,
            )],
        },
        Menu {
            name: "Preferences".into(),
            items: vec![MenuItem::action("Memory…", OpenMemoryPreferencesAction)],
        },
    ]);
}

fn schedule_recovery_autosave(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            // Save a recovery draft at a low cadence so editing stays responsive.
            Timer::after(Duration::from_secs(12)).await;
            let _ = cx.update(|cx| {
                sync_recovery_draft(cx);
            });
        }
    })
    .detach();
}

fn sync_recovery_draft(cx: &mut App) {
    let Some((global, last_saved_signature, last_recovery_signature)) =
        cx.try_global::<AppMenuState>().map(|state| {
            (
                state.global.clone(),
                state.last_saved_signature,
                state.last_recovery_signature,
            )
        })
    else {
        return;
    };

    let (current_signature, current_project_path) = {
        let gs = global.read(cx);
        (project_signature(gs), gs.project_file_path.clone())
    };
    let has_unsaved_changes = last_saved_signature
        .map(|saved| saved != current_signature)
        .unwrap_or(true);

    if !has_unsaved_changes {
        if let Err(err) = clear_recovery_draft(current_project_path.as_deref()) {
            eprintln!("[Project] Failed to clear recovery draft: {err}");
        }
        cx.update_global(|state: &mut AppMenuState, _| {
            state.last_recovery_signature = None;
        });
        return;
    }

    if last_recovery_signature == Some(current_signature) {
        return;
    }

    let save_result = {
        let gs = global.read(cx);
        save_recovery_draft(gs)
    };
    match save_result {
        Ok(path) => {
            println!("[Project] Recovery draft saved to {}", path.display());
            cx.update_global(|state: &mut AppMenuState, _| {
                state.last_recovery_signature = Some(current_signature);
            });
        }
        Err(err) => {
            eprintln!("[Project] Recovery draft save failed: {err}");
        }
    }
}

fn schedule_recovery_prompt(cx: &mut App) {
    cx.spawn(async move |cx| {
        // Delay the first recovery prompt until the primary window has rendered.
        Timer::after(Duration::from_millis(180)).await;
        let _ = cx.update(|cx| {
            prompt_recovery_draft_if_needed(cx, 0);
        });
    })
    .detach();
}

fn prompt_recovery_draft_if_needed(cx: &mut App, attempt: u8) {
    let Some(global) = cx
        .try_global::<AppMenuState>()
        .map(|state| state.global.clone())
    else {
        return;
    };

    let Some((draft_path, draft)) = (match latest_recovery_draft() {
        Ok(result) => result,
        Err(err) => {
            eprintln!("[Project] Failed to scan recovery drafts: {err}");
            None
        }
    }) else {
        return;
    };

    let saved_project_exists = draft
        .project_path
        .as_ref()
        .is_some_and(|path| path.exists());
    let detail = if let Some(project_path) = draft.project_path.as_ref() {
        format!(
            "Anica found an unsaved recovery draft.\n\nProject: {}\nSaved path: {}\n\nRecovering will not overwrite the saved project file.",
            draft.project_name,
            project_path.display()
        )
    } else {
        format!(
            "Anica found an unsaved recovery draft for an unnamed project.\n\nDraft: {}",
            draft.project_name
        )
    };

    let mut windows = Vec::new();
    let mut seen = HashSet::new();
    if let Some(active) = cx.active_window() {
        seen.insert(active.window_id());
        windows.push(active);
    }
    if let Some(stack) = cx.window_stack() {
        for handle in stack {
            if seen.insert(handle.window_id()) {
                windows.push(handle);
            }
        }
    }
    for handle in cx.windows() {
        if seen.insert(handle.window_id()) {
            windows.push(handle);
        }
    }

    let answers: Vec<&str> = if saved_project_exists {
        vec!["Recover Draft", "Open Saved", "Discard"]
    } else {
        vec!["Recover Draft", "Discard"]
    };
    for handle in windows {
        let detail_for_window = detail.clone();
        let answers_for_window = answers.clone();
        let prompt = handle.update(cx, |_, window, cx| {
            window.prompt(
                PromptLevel::Info,
                "Recovery Draft Found",
                Some(detail_for_window.as_str()),
                &answers_for_window,
                cx,
            )
        });
        let Ok(rx) = prompt else {
            continue;
        };

        let global_for_prompt = global.clone();
        cx.spawn(async move |cx| {
            let Ok(choice) = rx.await else { return };
            match choice {
                0 => {
                    let saved_signature = draft
                        .project_path
                        .as_ref()
                        .and_then(|path| load_project_from_path(path).ok())
                        .map(|state| project_state_signature(&state));

                    let recovered_signature = match global_for_prompt.update(cx, |gs, cx| {
                        draft.project.apply_to(gs);
                        gs.set_project_file_path(draft.project_path.clone());
                        cx.notify();
                        project_signature(gs)
                    }) {
                        Ok(signature) => signature,
                        Err(err) => {
                            eprintln!("[Project] Failed to recover draft: {err}");
                            return;
                        }
                    };

                    let _ = cx.update_global(|state: &mut AppMenuState, _| {
                        state.last_project_path = draft.project_path.clone();
                        state.last_saved_signature = saved_signature;
                        state.last_recovery_signature = Some(recovered_signature);
                    });
                }
                1 if saved_project_exists => {
                    if let Some(path) = draft.project_path.clone()
                        && let Ok(project) = load_project_from_path(&path)
                    {
                        let loaded_signature = match global_for_prompt.update(cx, |gs, cx| {
                            project.apply_to(gs);
                            gs.set_project_file_path(Some(path.clone()));
                            cx.notify();
                            project_signature(gs)
                        }) {
                            Ok(signature) => signature,
                            Err(err) => {
                                eprintln!("[Project] Failed to load saved project: {err}");
                                return;
                            }
                        };
                        let _ = cx.update_global(|state: &mut AppMenuState, _| {
                            state.last_project_path = Some(path.clone());
                            state.last_saved_signature = Some(loaded_signature);
                            state.last_recovery_signature = None;
                        });
                    }
                    if let Err(err) = std::fs::remove_file(&draft_path) {
                        eprintln!("[Project] Failed to remove recovery draft: {err}");
                    }
                }
                _ => {
                    if let Err(err) = std::fs::remove_file(&draft_path) {
                        eprintln!("[Project] Failed to remove recovery draft: {err}");
                    }
                    let _ = cx.update_global(|state: &mut AppMenuState, _| {
                        state.last_recovery_signature = None;
                    });
                }
            }
        })
        .detach();
        return;
    }

    if attempt < 8 {
        cx.defer(move |cx| {
            prompt_recovery_draft_if_needed(cx, attempt + 1);
        });
    }
}

fn undo_action(_: &UndoAction, cx: &mut App) {
    let Some(state) = cx.try_global::<AppMenuState>() else {
        return;
    };
    let global = state.global.clone();
    global.update(cx, |gs, cx| {
        gs.undo();
        cx.notify();
    });
}

fn redo_action(_: &RedoAction, cx: &mut App) {
    let Some(state) = cx.try_global::<AppMenuState>() else {
        return;
    };
    let global = state.global.clone();
    global.update(cx, |gs, cx| {
        gs.redo();
        cx.notify();
    });
}

fn save_action(_: &SaveAction, cx: &mut App) {
    let Some(state) = cx.try_global::<AppMenuState>() else {
        return;
    };
    let global = state.global.clone();
    let last_path = state
        .last_project_path
        .clone()
        .or_else(|| global.read(cx).project_file_path.clone());

    if let Some(path) = last_path {
        let _ = save_to_path(cx, global, path);
        return;
    }

    prompt_save_as(cx, global, None, PostSaveAction::None);
}

fn save_as_action(_: &SaveAsAction, cx: &mut App) {
    let Some(state) = cx.try_global::<AppMenuState>() else {
        return;
    };
    let global = state.global.clone();
    let suggested = state.last_project_path.as_ref().and_then(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
    });
    prompt_save_as(cx, global, suggested, PostSaveAction::None);
}

fn prompt_save_as(
    cx: &mut App,
    global: gpui::Entity<GlobalState>,
    suggested: Option<String>,
    post_save_action: PostSaveAction,
) {
    let dir = default_project_dir();
    let suggested_name = suggested.unwrap_or_else(|| {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs();
        format!("Untitled_{ts}.anica.json")
    });
    let rx = cx.prompt_for_new_path(&dir, Some(&suggested_name));

    cx.spawn(async move |cx| {
        let Ok(result) = rx.await else { return };
        let mut path = match result {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(err) => {
                eprintln!("[Project] Save picker error: {err}");
                return;
            }
        };

        if !path
            .to_string_lossy()
            .to_lowercase()
            .ends_with(".anica.json")
        {
            path.set_extension("anica.json");
        }

        let path_for_save = path.clone();
        let saved_signature = global.update(cx, |gs, cx| {
            if let Err(err) = save_project_to_path(gs, &path_for_save) {
                eprintln!("[Project] Save failed: {err}");
                return None;
            }

            println!("[Project] Saved to {}", path_for_save.display());
            gs.set_project_file_path(Some(path_for_save.clone()));
            if let Some(parent) = path_for_save.parent()
                && let Err(err) = save_project_snapshot(gs, autosave_dir(parent), None)
            {
                eprintln!("[Project] Snapshot failed: {err}");
            }
            cx.notify();
            Some(project_signature(gs))
        });

        let Ok(Some(saved_signature)) = saved_signature else {
            return;
        };

        let _ = cx.update_global(|state: &mut AppMenuState, _| {
            state.last_project_path = Some(path.clone());
            state.last_saved_signature = Some(saved_signature);
        });

        if matches!(post_save_action, PostSaveAction::CreateNewProject) {
            let _ = cx.update(|cx| {
                create_new_project(cx, global.clone());
            });
        }
    })
    .detach();
}

fn open_action(_: &OpenAction, cx: &mut App) {
    let Some(state) = cx.try_global::<AppMenuState>() else {
        return;
    };
    let global = state.global.clone();
    let rx = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some("Open Project".into()),
    });

    cx.spawn(async move |cx| {
        let Ok(result) = rx.await else { return };
        let paths = match result {
            Ok(Some(paths)) => paths,
            Ok(None) => return,
            Err(err) => {
                eprintln!("[Project] File picker error: {err}");
                return;
            }
        };

        let Some(path) = paths.into_iter().next() else {
            return;
        };
        let mut project = match load_project_from_path(&path) {
            Ok(project) => project,
            Err(err) => {
                eprintln!("[Project] Failed to load {}: {err}", path.display());
                return;
            }
        };

        // Check for missing media files and offer to relink.
        let missing = find_missing_media(&project);
        if !missing.is_empty() {
            let missing_names: Vec<String> = missing
                .iter()
                .map(|p| {
                    std::path::Path::new(p)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| p.clone())
                })
                .collect();
            let detail = format!(
                "{} media file(s) not found:\n\n{}\n\nWould you like to locate them?",
                missing.len(),
                missing_names.join("\n")
            );

            // Show prompt on the active window asking whether to relink.
            let prompt_rx = cx.update(|cx| {
                let windows = cx.windows();
                for handle in windows {
                    if let Ok(rx) = handle.update(cx, |_, window, cx| {
                        window.prompt(
                            PromptLevel::Warning,
                            "Missing Media Files",
                            Some(&detail),
                            &["Locate Files", "Skip"],
                            cx,
                        )
                    }) {
                        return Some(rx);
                    }
                }
                None
            });

            if let Ok(Some(rx)) = prompt_rx
                && let Ok(choice) = rx.await
            {
                // User chose "Locate Files".
                if choice == 0 {
                    let mut mapping: Vec<(String, String)> = Vec::new();
                    for old_path in &missing {
                        let file_name = std::path::Path::new(old_path)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| old_path.clone());

                        // Prompt user to locate each missing file.
                        let pick_rx = cx.update(|cx| {
                            cx.prompt_for_paths(PathPromptOptions {
                                files: true,
                                directories: false,
                                multiple: false,
                                prompt: Some(format!("Locate: {file_name}").into()),
                            })
                        });
                        let Ok(pick_rx) = pick_rx else { continue };
                        let Ok(pick_result) = pick_rx.await else {
                            continue;
                        };
                        if let Ok(Some(picked)) = pick_result
                            && let Some(new_path) = picked.into_iter().next()
                        {
                            mapping
                                .push((old_path.clone(), new_path.to_string_lossy().to_string()));
                        }
                    }
                    // Apply the path remapping to the project before loading.
                    remap_project_paths(&mut project, &mapping);
                }
            }
        }

        let loaded_signature = match global.update(cx, |gs, cx| {
            project.apply_to(gs);
            // Loaded projects always use project-local cache scope.
            gs.set_project_file_path(Some(path.clone()));
            cx.notify();
            project_signature(gs)
        }) {
            Ok(signature) => signature,
            Err(err) => {
                eprintln!(
                    "[Project] Failed to update app state after loading {}: {err}",
                    path.display()
                );
                return;
            }
        };

        let _ = cx.update_global(|menu_state: &mut AppMenuState, _| {
            menu_state.last_project_path = Some(path.clone());
            menu_state.last_saved_signature = Some(loaded_signature);
            menu_state.last_recovery_signature = None;
        });
    })
    .detach();
}

fn new_action(_: &NewAction, cx: &mut App) {
    let Some(state) = cx.try_global::<AppMenuState>() else {
        return;
    };
    let global = state.global.clone();
    let last_saved_signature = state.last_saved_signature;
    let maybe_saved_path = state
        .last_project_path
        .clone()
        .or_else(|| global.read(cx).project_file_path.clone());

    let current_signature = {
        let gs = global.read(cx);
        project_signature(gs)
    };

    let has_unsaved_changes = last_saved_signature
        .map(|saved| saved != current_signature)
        .unwrap_or(true);

    if !has_unsaved_changes {
        create_new_project(cx, global);
        return;
    }

    let global_for_prompt = global.clone();
    cx.defer(move |cx| {
        prompt_new_project_with_unsaved(cx, global_for_prompt.clone(), maybe_saved_path.clone());
    });
}

fn reveal_in_file_manager(path: &Path) -> Result<(), String> {
    let status = {
        #[cfg(target_os = "macos")]
        {
            Command::new("open").arg(path).status()
        }
        #[cfg(target_os = "windows")]
        {
            Command::new("explorer").arg(path).status()
        }
        #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
        {
            Command::new("xdg-open").arg(path).status()
        }
    }
    .map_err(|err| format!("Failed to launch file manager: {err}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "File manager exited with status {}.",
            status
                .code()
                .map_or_else(|| "unknown".to_string(), |code| code.to_string())
        ))
    }
}

fn reveal_in_finder_explorer_action(_: &RevealInFinderExplorerAction, cx: &mut App) {
    let Some(global) = cx
        .try_global::<AppMenuState>()
        .map(|menu_state| menu_state.global.clone())
    else {
        return;
    };

    let reveal_dir = global.read(cx).generated_media_root_dir();
    if let Err(err) = std::fs::create_dir_all(&reveal_dir) {
        global.update(cx, |gs, cx| {
            gs.ui_notice = Some(format!(
                "Failed to prepare generated media folder '{}': {err}",
                reveal_dir.display()
            ));
            cx.notify();
        });
        return;
    }

    let notice = match reveal_in_file_manager(&reveal_dir) {
        Ok(()) => format!("Opened generated media folder: {}", reveal_dir.display()),
        Err(err) => format!(
            "Failed to open generated media folder '{}': {err}",
            reveal_dir.display()
        ),
    };

    global.update(cx, |gs, cx| {
        gs.ui_notice = Some(notice);
        cx.notify();
    });
}

fn open_memory_preferences_action(_: &OpenMemoryPreferencesAction, cx: &mut App) {
    let Some(global) = cx
        .try_global::<AppMenuState>()
        .map(|menu_state| menu_state.global.clone())
    else {
        return;
    };

    global.update(cx, |gs, cx| {
        gs.show_preview_memory_budget_modal();
        cx.notify();
    });
}

fn toggle_preview_render_mode_action(_: &TogglePreviewRenderModeAction, cx: &mut App) {
    let Some(global) = cx
        .try_global::<AppMenuState>()
        .map(|menu_state| menu_state.global.clone())
    else {
        return;
    };

    global.update(cx, |gs, cx| {
        #[cfg(target_os = "macos")]
        {
            let next_mode = gs.toggle_mac_preview_render_mode();
            gs.ui_notice = Some(format!("macOS preview render mode: {}", next_mode.label()));
        }
        #[cfg(not(target_os = "macos"))]
        {
            gs.ui_notice = Some("Preview render mode switch is available on macOS only.".into());
        }
        cx.notify();
    });
}

fn prompt_new_project_with_unsaved(
    cx: &mut App,
    global: gpui::Entity<GlobalState>,
    maybe_saved_path: Option<PathBuf>,
) {
    prompt_new_project_with_unsaved_retry(cx, global, maybe_saved_path, 0);
}

fn prompt_new_project_with_unsaved_retry(
    cx: &mut App,
    global: gpui::Entity<GlobalState>,
    maybe_saved_path: Option<PathBuf>,
    attempt: u8,
) {
    let mut show_prompt = |title: &str, detail: Option<String>, answers: &[&str]| -> Option<_> {
        let mut windows = Vec::new();
        let mut seen = HashSet::new();

        if let Some(active) = cx.active_window() {
            seen.insert(active.window_id());
            windows.push(active);
        }
        if let Some(stack) = cx.window_stack() {
            for handle in stack {
                if seen.insert(handle.window_id()) {
                    windows.push(handle);
                }
            }
        }
        for handle in cx.windows() {
            if seen.insert(handle.window_id()) {
                windows.push(handle);
            }
        }

        eprintln!(
            "[Project] New confirmation candidates: active={}, total={}",
            cx.active_window().is_some(),
            windows.len()
        );

        for handle in windows {
            let detail_for_window = detail.clone();
            match handle.update(cx, |_, window, cx| {
                window.prompt(
                    PromptLevel::Warning,
                    title,
                    detail_for_window.as_deref(),
                    answers,
                    cx,
                )
            }) {
                Ok(rx) => return Some(rx),
                Err(err) => {
                    eprintln!(
                        "[Project] Failed to show New confirmation on window {:?}: {err}",
                        handle.window_id()
                    );
                }
            }
        }
        None
    };

    let notify_prompt_unavailable =
        |cx: &mut App, global: gpui::Entity<GlobalState>, maybe_saved_path: Option<PathBuf>| {
            eprintln!("[Project] Cannot open New confirmation dialog: no available window.");
            global.update(cx, |gs, cx| {
                gs.ui_notice = Some("Cannot show unsaved-changes dialog right now.".to_string());
                cx.notify();
            });
            // Safety fallback: keep project intact (no destructive action) and just log context.
            if let Some(path) = maybe_saved_path {
                eprintln!(
                    "[Project] Pending unsaved project path when prompt unavailable: {}",
                    path.display()
                );
            } else {
                eprintln!("[Project] Pending unsaved project had no save path yet.");
            }
        };

    if let Some(existing_path) = maybe_saved_path {
        let detail = format!(
            "Current project has unsaved changes.\nOverwrite current file before creating a new project?\n\n{}",
            existing_path.display()
        );
        let Some(rx) = show_prompt(
            "Unsaved Changes",
            Some(detail),
            &["Overwrite", "Don't Save", "Cancel"],
        ) else {
            if attempt < 8 {
                let global_for_retry = global.clone();
                let path_for_retry = Some(existing_path.clone());
                eprintln!(
                    "[Project] New confirmation retry {}/8 (saved-path flow).",
                    attempt + 1
                );
                cx.defer(move |cx| {
                    prompt_new_project_with_unsaved_retry(
                        cx,
                        global_for_retry.clone(),
                        path_for_retry.clone(),
                        attempt + 1,
                    );
                });
                return;
            }
            notify_prompt_unavailable(cx, global.clone(), Some(existing_path.clone()));
            return;
        };

        let global_for_prompt = global.clone();
        cx.spawn(async move |cx| {
            let Ok(choice) = rx.await else { return };
            match choice {
                0 => {
                    let _ = cx.update(|cx| {
                        if save_to_path(cx, global_for_prompt.clone(), existing_path.clone()) {
                            create_new_project(cx, global_for_prompt.clone());
                        }
                    });
                }
                1 => {
                    let _ = cx.update(|cx| {
                        create_new_project(cx, global_for_prompt.clone());
                    });
                }
                _ => {}
            }
        })
        .detach();
        return;
    }

    let Some(rx) = show_prompt(
        "Unsaved Changes",
        Some("Current project has unsaved changes.\nSave before creating a new project?".into()),
        &["Save…", "Don't Save", "Cancel"],
    ) else {
        if attempt < 8 {
            let global_for_retry = global.clone();
            eprintln!(
                "[Project] New confirmation retry {}/8 (no-path flow).",
                attempt + 1
            );
            cx.defer(move |cx| {
                prompt_new_project_with_unsaved_retry(
                    cx,
                    global_for_retry.clone(),
                    None,
                    attempt + 1,
                );
            });
            return;
        }
        notify_prompt_unavailable(cx, global.clone(), None);
        return;
    };

    cx.spawn(async move |cx| {
        let Ok(choice) = rx.await else { return };
        match choice {
            0 => {
                let _ = cx.update(|cx| {
                    prompt_save_as(cx, global.clone(), None, PostSaveAction::CreateNewProject);
                });
            }
            1 => {
                let _ = cx.update(|cx| {
                    create_new_project(cx, global.clone());
                });
            }
            _ => {}
        }
    })
    .detach();
}
