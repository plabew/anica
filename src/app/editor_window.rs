// =========================================
// =========================================
// src/app/editor_window.rs
use gpui::{App, AppContext, Bounds, Context, WindowBounds, WindowOptions, size};

use crate::core::global_state::{GlobalState, MediaPoolUiEvent};
use crate::ui::ai_agents_page::AiAgentsPage;
use crate::ui::ai_srt_page::AiSrtPage;
use crate::ui::app_root::AppRoot;
use crate::ui::editor_shell::EditorShell;
use crate::ui::inspector_panel::InspectorPanel;
use crate::ui::motionloom_page::MotionLoomPage;
use crate::ui::timeline_panel::TimelinePanel;
use crate::ui::video_preview::VideoPreview;
use gpui_component::Root;

// Open the editor with a fresh in-memory project state.
pub fn open_editor_window(cx: &mut App) -> gpui::Entity<GlobalState> {
    // Use the primary display to center the editor window.
    let binding = cx.displays();
    let display = binding.first().expect("No display detected");

    // Derive window size from the current screen dimensions.
    let screen_size = display.bounds().size;

    let width = screen_size.width * 0.85;
    let height = screen_size.height * 0.90;

    // Keep a large default window while leaving room around edges.
    let bounds = Bounds::centered(Some(display.id()), size(width, height), cx);

    let global = cx.new(|_cx| GlobalState::default());
    let global_for_window = global.clone();

    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            focus: true,
            ..Default::default()
        },
        move |window, cx| {
            let global = global_for_window.clone();

            let global_for_editor = global.clone();
            let editor = cx.new(move |cx| EditorShell::new(global_for_editor.clone(), cx));

            let global_for_timeline = global.clone();
            let timeline = cx.new(move |cx: &mut Context<TimelinePanel>| {
                TimelinePanel::new(global_for_timeline.clone(), cx)
            });

            let global_for_preview = global.clone();
            let preview = cx.new(move |cx: &mut Context<VideoPreview>| {
                // Let the preview handle playback synchronization during render.
                cx.observe(&global_for_preview, |_this, _global, cx| {
                    cx.notify();
                })
                .detach();

                VideoPreview::new(global_for_preview.clone(), cx)
            });

            let global_for_inspector = global.clone();
            let inspector = cx.new(move |cx: &mut Context<InspectorPanel>| {
                InspectorPanel::new(global_for_inspector, cx)
            });

            let global_for_ai = global.clone();
            let ai_srt_page = cx.new(move |_cx| AiSrtPage::new(global_for_ai.clone()));
            let global_for_ai_agents = global.clone();
            let ai_agents_page =
                cx.new(move |cx| AiAgentsPage::new(global_for_ai_agents.clone(), cx));
            let global_for_motionloom = global.clone();
            let motionloom_page =
                cx.new(move |cx| MotionLoomPage::new(global_for_motionloom.clone(), cx));

            let global_for_app_root = global.clone();
            let app_root = cx.new(|cx| {
                cx.subscribe(
                    &global_for_app_root,
                    |_, _, event: &MediaPoolUiEvent, cx| {
                        if matches!(
                            event,
                            MediaPoolUiEvent::StateChanged | MediaPoolUiEvent::DragCursorChanged
                        ) {
                            cx.notify();
                        }
                    },
                )
                .detach();

                AppRoot {
                    global,
                    editor,
                    preview,
                    inspector,
                    timeline,
                    ai_srt_page,
                    ai_agents_page,
                    motionloom_page,
                    ai_chat_widget_open: false,
                    ai_chat_input_text: String::new(),
                    ai_chat_input: None,
                    ai_chat_input_sub: None,
                    ai_chat_send_on_next_render: false,
                    ai_chat_show_system_messages: false,
                    ai_chat_expand_modal_open: false,
                    inspector_expand_modal_open: false,
                }
            });

            cx.new(|cx| Root::new(app_root, window, cx))
        },
    )
    .unwrap();

    global
}
