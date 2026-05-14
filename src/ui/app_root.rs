// =========================================
// =========================================
// src/ui/app_root.rs
use gpui::{
    ClipboardItem, Context, Entity, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Render, ScrollWheelEvent, Subscription, Window, div, prelude::*, px, rgb, rgba,
    svg,
};

use crate::core::global_state::{
    AiChatMessage, AiChatRole, AppPage, GlobalState, MediaPoolUiEvent,
};
use crate::core::media_tools::{detect_gstreamer_cli, detect_or_bootstrap_media_dependencies};
use crate::ui::ai_agents_page::AiAgentsPage;
use crate::ui::ai_srt_page::AiSrtPage;
use crate::ui::display_settings_modal::render_display_settings_modal_overlay;
use crate::ui::editor_shell::EditorShell;
use crate::ui::export_modal::render_export_modal_overlay;
use crate::ui::inspector_panel::InspectorPanel;
use crate::ui::motionloom_page::MotionLoomPage;
use crate::ui::timeline_panel::TimelinePanel;
use crate::ui::vector_lab_page::VectorLabPage;
use crate::ui::video_preview::VideoPreview;
use gpui_component::{
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    text::TextView,
    white,
};

// Keep this in sync with the timeline panel fixed height so global mouse-up
// logic does not cancel drag/drop before timeline handlers run.
const TIMELINE_PANEL_HEIGHT_PX: f32 = 364.0;

pub struct AppRoot {
    pub global: Entity<GlobalState>,
    pub editor: Entity<EditorShell>,       // left
    pub preview: Entity<VideoPreview>,     // right
    pub inspector: Entity<InspectorPanel>, // ✅ Right Sidebar
    pub timeline: Entity<TimelinePanel>,   // bottom
    pub ai_srt_page: Entity<AiSrtPage>,
    pub ai_agents_page: Entity<AiAgentsPage>,
    pub motionloom_page: Entity<MotionLoomPage>,
    pub vector_lab_page: Entity<VectorLabPage>,
    pub ai_chat_widget_open: bool,
    pub ai_chat_input_text: String,
    pub ai_chat_input: Option<Entity<InputState>>,
    pub ai_chat_input_sub: Option<Subscription>,
    pub ai_chat_send_on_next_render: bool,
    pub ai_chat_show_system_messages: bool,
    pub ai_chat_expand_modal_open: bool,
    pub inspector_expand_modal_open: bool,
}

impl AppRoot {
    fn ensure_chat_widget_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai_chat_input.is_some() {
            return;
        }

        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Type a message to the AI agent"));
        let sub = cx.subscribe(&input, |this, input, ev, cx| match ev {
            InputEvent::Change => {
                this.ai_chat_input_text = input.read(cx).value().to_string();
            }
            InputEvent::PressEnter { .. } => {
                this.ai_chat_input_text = input.read(cx).value().to_string();
                this.ai_chat_send_on_next_render = true;
                cx.notify();
            }
            _ => {}
        });
        self.ai_chat_input = Some(input);
        self.ai_chat_input_sub = Some(sub);
    }

    fn clear_chat_widget_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ai_chat_input_text.clear();
        if let Some(input) = self.ai_chat_input.as_ref() {
            input.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
        }
    }

    fn send_chat_widget_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = self.ai_chat_input_text.trim().to_string();
        if prompt.is_empty() {
            return;
        }

        let sent = self.ai_agents_page.update(cx, |page, cx| {
            page.send_prompt_from_external(prompt, window, cx)
        });

        if sent {
            self.clear_chat_widget_input(window, cx);
            cx.notify();
        }
    }

    fn open_inspector_expand_modal(&mut self, cx: &mut Context<Self>) {
        if self.inspector_expand_modal_open {
            return;
        }
        self.inspector_expand_modal_open = true;
        self.global.update(cx, |gs, cx| {
            gs.set_inspector_panel_expanded(true);
            cx.notify();
        });
    }

    fn close_inspector_expand_modal(&mut self, cx: &mut Context<Self>) {
        if !self.inspector_expand_modal_open {
            return;
        }
        self.inspector_expand_modal_open = false;
        self.global.update(cx, |gs, cx| {
            gs.set_inspector_panel_expanded(false);
            cx.notify();
        });
    }

    fn open_ai_chat_expand_modal(&mut self, cx: &mut Context<Self>) {
        if self.ai_chat_expand_modal_open {
            return;
        }
        self.ai_chat_expand_modal_open = true;
        cx.notify();
    }

    fn close_ai_chat_expand_modal(&mut self, cx: &mut Context<Self>) {
        if !self.ai_chat_expand_modal_open {
            return;
        }
        self.ai_chat_expand_modal_open = false;
        cx.notify();
    }

    fn render_chat_bubble(
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
            .px_2()
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
                        TextView::markdown(("ai-widget-msg-body", bubble_index), text, window, cx)
                            .selectable(true)
                            .w_full(),
                    ),
            )
    }

    fn render_chat_widget_card(
        &self,
        panel_w: f32,
        panel_h: f32,
        allow_expand: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let chat_input_elem = if let Some(input) = self.ai_chat_input.as_ref() {
            Input::new(input).h(px(34.0)).w_full().into_any_element()
        } else {
            div()
                .h(px(34.0))
                .w_full()
                .rounded_sm()
                .bg(white().opacity(0.05))
                .into_any_element()
        };
        let can_send_widget = !self.ai_chat_input_text.trim().is_empty();
        let chat_messages = self.global.read(cx).ai_chat_messages.clone();
        let system_count = chat_messages
            .iter()
            .filter(|m| matches!(m.role, AiChatRole::System))
            .count();
        let visible_chat_messages: Vec<AiChatMessage> = chat_messages
            .iter()
            .filter(|m| self.ai_chat_show_system_messages || !matches!(m.role, AiChatRole::System))
            .cloned()
            .collect();
        let mut chat_message_list = div().flex().flex_col().gap_2();
        if visible_chat_messages.is_empty() {
            chat_message_list =
                chat_message_list.child(div().text_xs().text_color(white().opacity(0.5)).child(
                    if system_count > 0 {
                        format!(
                            "System logs are hidden ({}). Toggle to view them.",
                            system_count
                        )
                    } else {
                        "No chat yet. Connect in AI Agents page, then chat here.".to_string()
                    },
                ));
        } else {
            for (idx, msg) in visible_chat_messages
                .iter()
                .rev()
                .take(60)
                .rev()
                .enumerate()
            {
                chat_message_list =
                    chat_message_list.child(Self::render_chat_bubble(msg, idx, window, cx));
            }
        }

        div()
            .w(px(panel_w))
            .h(px(panel_h))
            .rounded_lg()
            .border_1()
            .border_color(white().opacity(0.18))
            .bg(rgb(0x05080f))
            .shadow_2xl()
            .overflow_hidden()
            .flex()
            .flex_col()
            // Prevent scroll and click events from passing through to panels underneath.
            .on_scroll_wheel(cx.listener(|_this, _evt: &ScrollWheelEvent, _win, cx| {
                cx.stop_propagation();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, _evt: &MouseDownEvent, _win, cx| {
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .h(px(44.0))
                    .px_3()
                    .border_b_1()
                    .border_color(white().opacity(0.12))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.9))
                            .child("AI Chat"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(allow_expand, |row| {
                                row.child(
                                    div()
                                        .h(px(26.0))
                                        .px_2()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(white().opacity(0.14))
                                        .bg(white().opacity(0.05))
                                        .text_xs()
                                        .text_color(white().opacity(0.82))
                                        .hover(|s| s.bg(white().opacity(0.1)))
                                        .cursor_pointer()
                                        .child("Expand")
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(|this, _, _, cx| {
                                                this.open_ai_chat_expand_modal(cx);
                                            }),
                                        ),
                                )
                            })
                            .child(
                                div()
                                    .h(px(26.0))
                                    .px_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(if self.ai_chat_show_system_messages {
                                        gpui::Hsla::from(rgb(0xf59e0b)).opacity(0.45)
                                    } else {
                                        white().opacity(0.14)
                                    })
                                    .bg(if self.ai_chat_show_system_messages {
                                        gpui::Hsla::from(rgb(0x7c2d12)).opacity(0.35)
                                    } else {
                                        white().opacity(0.05)
                                    })
                                    .text_xs()
                                    .text_color(white().opacity(0.82))
                                    .hover(|s| s.bg(white().opacity(0.1)))
                                    .cursor_pointer()
                                    .child(if self.ai_chat_show_system_messages {
                                        format!("Hide System ({})", system_count)
                                    } else {
                                        format!("Show System ({})", system_count)
                                    })
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            this.ai_chat_show_system_messages =
                                                !this.ai_chat_show_system_messages;
                                            cx.notify();
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .h(px(26.0))
                                    .px_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(white().opacity(0.14))
                                    .bg(white().opacity(0.05))
                                    .text_xs()
                                    .text_color(white().opacity(0.78))
                                    .hover(|s| s.bg(white().opacity(0.1)))
                                    .cursor_pointer()
                                    .child("Close")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            if allow_expand {
                                                this.ai_chat_widget_open = false;
                                                this.ai_chat_expand_modal_open = false;
                                            } else {
                                                this.close_ai_chat_expand_modal(cx);
                                            }
                                            cx.notify();
                                        }),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .p_2()
                    .overflow_y_scrollbar()
                    .child(chat_message_list),
            )
            .child(
                div()
                    .p_2()
                    .border_t_1()
                    .border_color(white().opacity(0.12))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(chat_input_elem))
                    .child(
                        div().w(px(82.0)).flex_shrink_0().child(
                            div()
                                .h(px(34.0))
                                .px_3()
                                .rounded_md()
                                .border_1()
                                .border_color(white().opacity(0.14))
                                .bg(if can_send_widget {
                                    gpui::Hsla::from(rgb(0x1d4ed8)).opacity(0.9)
                                } else {
                                    white().opacity(0.06)
                                })
                                .text_sm()
                                .text_color(white().opacity(if can_send_widget {
                                    0.95
                                } else {
                                    0.45
                                }))
                                .hover(|s| s.bg(white().opacity(0.1)))
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child("Send")
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, window, cx| {
                                        if !this.ai_chat_input_text.trim().is_empty() {
                                            this.send_chat_widget_prompt(window, cx);
                                        }
                                    }),
                                ),
                        ),
                    ),
            )
    }

    fn render_media_dependency_modal(&self, cx: &mut Context<Self>) -> gpui::Div {
        let (show_modal, status, gstreamer_available, gstreamer_path) = {
            let gs = self.global.read(cx);
            (
                gs.show_media_dependency_modal,
                gs.media_dependency.clone(),
                gs.gstreamer_available,
                gs.gstreamer_path.clone(),
            )
        };
        if !show_modal || (status.all_available() && gstreamer_available) {
            return div();
        }

        let mut missing = status
            .missing_tools()
            .into_iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if !gstreamer_available {
            missing.push("gstreamer".to_string());
        }
        let missing_tools = missing.join(", ");
        let install_rows = status
            .host
            .install_commands()
            .iter()
            .chain(status.host.gstreamer_install_commands().iter())
            .fold(div().flex().flex_col().gap_2(), |rows, (label, command)| {
                let command_text = (*command).to_string();
                rows.child(
                    div()
                        .rounded_md()
                        .border_1()
                        .border_color(white().opacity(0.14))
                        .bg(white().opacity(0.03))
                        .px_2()
                        .py_2()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(white().opacity(0.72))
                                        .child(*label),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(white().opacity(0.9))
                                        .truncate()
                                        .child(command_text.clone()),
                                ),
                        )
                        .child(
                            div()
                                .h(px(24.0))
                                .px_2()
                                .rounded_sm()
                                .border_1()
                                .border_color(white().opacity(0.16))
                                .bg(white().opacity(0.06))
                                .text_xs()
                                .text_color(white().opacity(0.9))
                                .hover(|s| s.bg(white().opacity(0.12)))
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child("Copy")
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |_this, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            command_text.clone(),
                                        ));
                                    }),
                                ),
                        ),
                )
            });
        let ffmpeg_line = if status.ffmpeg_available {
            status
                .ffmpeg_version
                .clone()
                .unwrap_or_else(|| "ffmpeg detected".to_string())
        } else {
            format!("ffmpeg missing (using `{}`)", status.ffmpeg_command)
        };
        let ffprobe_line = if status.ffprobe_available {
            status
                .ffprobe_version
                .clone()
                .unwrap_or_else(|| "ffprobe detected".to_string())
        } else {
            format!("ffprobe missing (using `{}`)", status.ffprobe_command)
        };
        let gstreamer_line = if gstreamer_available {
            format!("gstreamer detected (using `{}`)", gstreamer_path)
        } else {
            format!("gstreamer missing (using `{}`)", gstreamer_path)
        };

        let global_for_close = self.global.clone();
        let global_for_recheck = self.global.clone();
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right_0()
            .bg(rgba(0x05080fc7))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(620.0))
                    .rounded_lg()
                    .border_1()
                    .border_color(white().opacity(0.14))
                    .bg(rgb(0x111827))
                    .shadow_2xl()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.96))
                            .child("Install Media Runtime"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.72))
                            .child(format!(
                                "Detected platform: {}. Missing: {}.",
                                status.host.label(),
                                missing_tools
                            )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.6))
                            .child(
                                "Basic import remains available. Preview playback depends on GStreamer. Export and ACP deep media analysis depend on FFmpeg/FFprobe.",
                            ),
                    )
                    .child(
                        div()
                            .rounded_md()
                            .border_1()
                            .border_color(white().opacity(0.1))
                            .bg(rgb(0x0b1220))
                            .px_2()
                            .py_2()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(if status.ffmpeg_available {
                                        rgba(0x22c55eeb)
                                    } else {
                                        rgba(0xf87171eb)
                                    })
                                    .child(ffmpeg_line),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(if status.ffprobe_available {
                                        rgba(0x22c55eeb)
                                    } else {
                                        rgba(0xf87171eb)
                                    })
                                    .child(ffprobe_line),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(if gstreamer_available {
                                        rgba(0x22c55eeb)
                                    } else {
                                        rgba(0xf87171eb)
                                    })
                                    .child(gstreamer_line),
                            ),
                    )
                    .child(install_rows)
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.62))
                            .child("Please click Re-check after first-time setup."),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .h(px(28.0))
                                    .px_3()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(white().opacity(0.16))
                                    .bg(white().opacity(0.05))
                                    .text_xs()
                                    .text_color(white().opacity(0.85))
                                    .hover(|s| s.bg(white().opacity(0.1)))
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child("Re-check")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |_this, _, _, cx| {
                                            let preferred = {
                                                let gs = global_for_recheck.read(cx);
                                                gs.ffmpeg_path.clone()
                                            };
                                            let next_status =
                                                detect_or_bootstrap_media_dependencies(Some(
                                                    &preferred,
                                                ));
                                            let next_gstreamer = detect_gstreamer_cli(None);
                                            global_for_recheck.update(cx, |gs, cx| {
                                                gs.apply_gstreamer_dependency_status(
                                                    next_gstreamer.clone(),
                                                );
                                                gs.apply_media_dependency_status(
                                                    next_status.clone(),
                                                    true,
                                                );
                                                if next_status.all_available()
                                                    && next_gstreamer.is_some()
                                                {
                                                    gs.hide_media_dependency_modal();
                                                    gs.ui_notice = Some(
                                                        "FFmpeg/FFprobe/GStreamer detected. Media features unlocked."
                                                            .to_string(),
                                                    );
                                                } else {
                                                    let mut missing = next_status
                                                        .missing_tools()
                                                        .into_iter()
                                                        .map(ToString::to_string)
                                                        .collect::<Vec<_>>();
                                                    if next_gstreamer.is_none() {
                                                        missing.push("gstreamer".to_string());
                                                    }
                                                    gs.show_media_dependency_modal();
                                                    gs.ui_notice = Some(format!(
                                                        "Missing tools: {}",
                                                        missing.join(", ")
                                                    ));
                                                }
                                                cx.notify();
                                            });
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .h(px(28.0))
                                    .px_3()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(white().opacity(0.16))
                                    .bg(white().opacity(0.05))
                                    .text_xs()
                                    .text_color(white().opacity(0.85))
                                    .hover(|s| s.bg(white().opacity(0.1)))
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child("Close")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |_this, _, _, cx| {
                                            global_for_close.update(cx, |gs, cx| {
                                                gs.hide_media_dependency_modal();
                                                cx.notify();
                                            });
                                        }),
                                    ),
                            ),
                    ),
            )
    }

    fn render_preview_memory_budget_modal(&self, cx: &mut Context<Self>) -> gpui::Div {
        let (show_modal, current_budget_mb) = {
            let gs = self.global.read(cx);
            (
                gs.show_preview_memory_budget_modal,
                gs.preview_memory_budget_mb.map(|v| v.clamp(256, 32768)),
            )
        };
        if !show_modal {
            return div();
        }

        let options: [(&str, Option<usize>); 6] = [
            ("No Limit (Recommended)", None),
            ("512 MB", Some(512)),
            ("1024 MB", Some(1024)),
            ("2048 MB", Some(2048)),
            ("4096 MB", Some(4096)),
            ("8192 MB", Some(8192)),
        ];

        let option_buttons = options.into_iter().fold(
            div().flex().flex_wrap().gap_2(),
            |rows, (label, budget_mb)| {
                let active = current_budget_mb == budget_mb;
                let global_for_apply = self.global.clone();
                let preview_for_apply = self.preview.clone();
                rows.child(
                    div()
                        .h(px(30.0))
                        .px_3()
                        .rounded_md()
                        .border_1()
                        .border_color(white().opacity(if active { 0.45 } else { 0.18 }))
                        .bg(if active {
                            rgba(0x1d4ed8d9)
                        } else {
                            rgba(0xffffff0f)
                        })
                        .text_xs()
                        .text_color(white().opacity(if active { 0.98 } else { 0.88 }))
                        .hover(|s| s.bg(white().opacity(0.1)))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(label)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |_this, _, _, cx| {
                                preview_for_apply.update(cx, |preview, _cx| {
                                    preview.set_memory_budget_mb(budget_mb);
                                });
                                global_for_apply.update(cx, |gs, cx| {
                                    gs.set_preview_memory_budget_mb(budget_mb);
                                    gs.hide_preview_memory_budget_modal();
                                    gs.ui_notice = Some(match budget_mb {
                                        Some(v) => {
                                            format!("Preview memory budget set to {v} MB.")
                                        }
                                        None => {
                                            "Preview memory budget set to no limit.".to_string()
                                        }
                                    });
                                    cx.notify();
                                });
                            }),
                        ),
                )
            },
        );

        let global_for_close = self.global.clone();
        let current_label = match current_budget_mb {
            Some(v) => format!("{v} MB"),
            None => "No limit".to_string(),
        };

        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right_0()
            .bg(rgba(0x05080fc7))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(560.0))
                    .rounded_lg()
                    .border_1()
                    .border_color(white().opacity(0.14))
                    .bg(rgb(0x111827))
                    .shadow_2xl()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.96))
                            .child("Preview Memory Budget"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.72))
                            .child(format!("Current: {current_label}")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.6))
                            .child(
                                "This controls preview-side cache pressure. No limit keeps best responsiveness but can consume more RAM.",
                            ),
                    )
                    .child(option_buttons)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .child(
                                div()
                                    .h(px(28.0))
                                    .px_3()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(white().opacity(0.16))
                                    .bg(white().opacity(0.05))
                                    .text_xs()
                                    .text_color(white().opacity(0.85))
                                    .hover(|s| s.bg(white().opacity(0.1)))
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child("Close")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |_this, _, _, cx| {
                                            global_for_close.update(cx, |gs, cx| {
                                                gs.hide_preview_memory_budget_modal();
                                                cx.notify();
                                            });
                                        }),
                                    ),
                            ),
                    ),
            )
    }

    // ── Silence preview modal ──
    // Renders a modal overlay with checkbox rows for each silence candidate.
    // User picks which silence ranges to cut, then confirms to inject back into ACP chat.
    fn render_silence_preview_modal(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let modal_state = self.global.read(cx).silence_preview_modal.clone();
        let Some(modal) = modal_state else {
            return div();
        };

        // Build checkbox rows for each silence candidate
        let mut rows = div().flex().flex_col().gap_1();
        for (i, candidate) in modal.candidates.iter().enumerate() {
            let duration_ms = candidate.end_ms.saturating_sub(candidate.start_ms);
            let label = format!(
                "{:.1}s – {:.1}s  ({:.1}s)  {:.0}%  {}",
                candidate.start_ms as f64 / 1000.0,
                candidate.end_ms as f64 / 1000.0,
                duration_ms as f64 / 1000.0,
                candidate.confidence * 100.0,
                candidate.reason,
            );
            let selected = candidate.selected;
            let global_for_toggle = self.global.clone();

            rows = rows.child(
                div()
                    .h(px(28.0))
                    .px_2()
                    .rounded_sm()
                    .bg(if selected {
                        gpui::Hsla::from(rgb(0x1d4ed8)).opacity(0.13)
                    } else {
                        white().opacity(0.02)
                    })
                    .hover(|s| s.bg(white().opacity(0.07)))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap_2()
                    // Checkbox indicator
                    .child(
                        div()
                            .w(px(16.0))
                            .h(px(16.0))
                            .rounded_sm()
                            .border_1()
                            .border_color(if selected {
                                rgb(0x3b82f6).into()
                            } else {
                                white().opacity(0.3)
                            })
                            .bg(if selected {
                                rgb(0x2563eb).into()
                            } else {
                                white().opacity(0.04)
                            })
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(if selected {
                                div().text_xs().text_color(white().opacity(0.95)).child("✓")
                            } else {
                                div()
                            }),
                    )
                    // Candidate label text
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.88))
                            .truncate()
                            .child(label),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_this, _, _, cx| {
                            global_for_toggle.update(cx, |gs, cx| {
                                gs.toggle_silence_candidate(i);
                                cx.notify();
                            });
                        }),
                    ),
            );
        }

        let has_selection = modal.candidates.iter().any(|c| c.selected);

        let global_for_cancel = self.global.clone();
        let global_for_confirm = self.global.clone();
        let ai_agents_page_for_confirm = self.ai_agents_page.clone();
        // Select All / Deselect All toggle
        let all_selected = modal.candidates.iter().all(|c| c.selected);
        let global_for_select_all = self.global.clone();
        let candidate_count = modal.candidates.len();

        // Modal overlay: dark backdrop + centered card
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right_0()
            .bg(rgba(0x05080fc7))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(560.0))
                    .max_h(px(480.0))
                    .rounded_lg()
                    .border_1()
                    .border_color(white().opacity(0.14))
                    .bg(rgb(0x111827))
                    .shadow_2xl()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    // Title
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.96))
                            .child("Silence Preview — Select ranges to cut"),
                    )
                    // Select All / Deselect All button
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .h(px(24.0))
                                    .px_2()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(white().opacity(0.16))
                                    .bg(white().opacity(0.05))
                                    .text_xs()
                                    .text_color(white().opacity(0.82))
                                    .hover(|s| s.bg(white().opacity(0.1)))
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(if all_selected {
                                        "Deselect All"
                                    } else {
                                        "Select All"
                                    })
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |_this, _, _, cx| {
                                            global_for_select_all.update(cx, |gs, cx| {
                                                if let Some(modal) =
                                                    gs.silence_preview_modal.as_mut()
                                                {
                                                    let new_val = !modal
                                                        .candidates
                                                        .iter()
                                                        .all(|c| c.selected);
                                                    for c in modal.candidates.iter_mut() {
                                                        c.selected = new_val;
                                                    }
                                                }
                                                cx.notify();
                                            });
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.5))
                                    .child(format!("{} candidates", candidate_count)),
                            ),
                    )
                    // Scrollable candidate list
                    .child(div().flex_1().min_h_0().overflow_y_scrollbar().child(rows))
                    // Action buttons: Cancel + Confirm
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .h(px(28.0))
                                    .px_3()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(white().opacity(0.16))
                                    .bg(white().opacity(0.05))
                                    .text_xs()
                                    .text_color(white().opacity(0.85))
                                    .hover(|s| s.bg(white().opacity(0.1)))
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child("Cancel")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |_this, _, _, cx| {
                                            global_for_cancel.update(cx, |gs, cx| {
                                                gs.hide_silence_preview_modal();
                                                cx.notify();
                                            });
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .h(px(28.0))
                                    .px_3()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(if has_selection {
                                        gpui::Hsla::from(rgb(0x2563eb)).opacity(0.7)
                                    } else {
                                        white().opacity(0.12)
                                    })
                                    .bg(if has_selection {
                                        gpui::Hsla::from(rgb(0x1d4ed8)).opacity(0.85)
                                    } else {
                                        white().opacity(0.04)
                                    })
                                    .text_xs()
                                    .text_color(white().opacity(if has_selection {
                                        0.95
                                    } else {
                                        0.4
                                    }))
                                    .hover(|s| {
                                        s.bg(if has_selection {
                                            gpui::Hsla::from(rgb(0x2563eb)).opacity(0.95)
                                        } else {
                                            white().opacity(0.04)
                                        })
                                    })
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child("Confirm & Send to Agent")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |_this, _, window, cx| {
                                            // Collect selected candidates as JSON prompt for ACP agent
                                            let json_prompt = {
                                                let gs = global_for_confirm.read(cx);
                                                gs.selected_silence_candidates_json()
                                            };
                                            // Close modal
                                            global_for_confirm.update(cx, |gs, cx| {
                                                gs.hide_silence_preview_modal();
                                                cx.notify();
                                            });
                                            // Inject selected ranges into ACP chat as user prompt
                                            if let Some(json) = json_prompt {
                                                ai_agents_page_for_confirm.update(
                                                    cx,
                                                    |page, cx| {
                                                        page.send_prompt_from_external(
                                                            json, window, cx,
                                                        );
                                                    },
                                                );
                                            }
                                        }),
                                    ),
                            ),
                    ),
            )
    }

    fn render_media_pool_context_menu(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let Some(menu) = self.global.read(cx).media_pool_context_menu.clone() else {
            return div();
        };

        let viewport_w = window.viewport_size().width / px(1.0);
        let viewport_h = window.viewport_size().height / px(1.0);
        let menu_w = 220.0;
        let menu_h = 68.0;
        let menu_x = menu.x.clamp(8.0, (viewport_w - menu_w - 8.0).max(8.0));
        let menu_y = menu.y.clamp(8.0, (viewport_h - menu_h - 8.0).max(8.0));

        let global_for_close_left = self.global.clone();
        let global_for_close_right = self.global.clone();
        let global_for_remove = self.global.clone();
        let global_for_locate = self.global.clone();
        let remove_path = menu.path.clone();
        let locate_path = menu.path.clone();

        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right_0()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_this, _, _, cx| {
                    cx.stop_propagation();
                    global_for_close_left.update(cx, |gs, cx| {
                        gs.close_media_pool_context_menu();
                        cx.emit(MediaPoolUiEvent::StateChanged);
                    });
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |_this, _, _, cx| {
                    cx.stop_propagation();
                    global_for_close_right.update(cx, |gs, cx| {
                        gs.close_media_pool_context_menu();
                        cx.emit(MediaPoolUiEvent::StateChanged);
                    });
                }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(menu_x))
                    .top(px(menu_y))
                    .w(px(menu_w))
                    .rounded_md()
                    .bg(rgb(0x1f1f23))
                    .border_1()
                    .border_color(white().opacity(0.14))
                    .p_1()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_this, _, _, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |_this, _, _, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    // Locate Media – open file picker to relink a media file.
                    .child(
                        div()
                            .h(px(28.0))
                            .rounded_sm()
                            .px_2()
                            .flex()
                            .items_center()
                            .text_sm()
                            .text_color(rgb(0xe0e0e0))
                            .bg(white().opacity(0.03))
                            .hover(|style| style.bg(white().opacity(0.10)))
                            .cursor_pointer()
                            .child("Locate Media")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |_this, _, win, cx| {
                                    let old_path = locate_path.clone();
                                    let global = global_for_locate.clone();
                                    global.update(cx, |gs, cx| {
                                        gs.close_media_pool_context_menu();
                                        cx.emit(MediaPoolUiEvent::StateChanged);
                                    });
                                    let file_name = std::path::Path::new(&old_path)
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_else(|| old_path.clone());
                                    let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
                                        files: true,
                                        directories: false,
                                        multiple: false,
                                        prompt: Some(format!("Locate: {file_name}").into()),
                                    });
                                    cx.spawn_in(win, async move |view, window| {
                                        let Ok(result) = rx.await else { return };
                                        let Some(paths) = result.ok().flatten() else {
                                            return;
                                        };
                                        let Some(new_path) = paths.into_iter().next() else {
                                            return;
                                        };
                                        let new_path_str = new_path.to_string_lossy().to_string();
                                        let _ = view.update_in(window, |_this, _win, cx| {
                                            global.update(cx, |gs, cx| {
                                                gs.relocate_media_path(&old_path, &new_path_str);
                                                gs.ui_notice =
                                                    Some(format!("Media relocated: {}", file_name));
                                                cx.emit(MediaPoolUiEvent::StateChanged);
                                                cx.notify();
                                            });
                                        });
                                    })
                                    .detach();
                                }),
                            ),
                    )
                    .child(
                        div()
                            .h(px(28.0))
                            .rounded_sm()
                            .px_2()
                            .flex()
                            .items_center()
                            .text_sm()
                            .text_color(rgb(0xfca5a5))
                            .bg(white().opacity(0.03))
                            .hover(|style| style.bg(white().opacity(0.10)))
                            .cursor_pointer()
                            .child("Delete from Media Pool")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |_this, _, _, cx| {
                                    global_for_remove.update(cx, |gs, cx| {
                                        let removed_name = gs
                                            .media_pool
                                            .iter()
                                            .find(|item| item.path == remove_path)
                                            .map(|item| item.name.clone())
                                            .unwrap_or_else(|| remove_path.clone());
                                        let removed = gs.remove_media_pool_item(&remove_path);
                                        gs.close_media_pool_context_menu();
                                        gs.ui_notice = Some(if removed {
                                            format!("Removed from Media Pool: {removed_name}")
                                        } else {
                                            "Media item no longer exists in Media Pool.".to_string()
                                        });
                                        cx.emit(MediaPoolUiEvent::StateChanged);
                                        cx.notify();
                                    });
                                }),
                            ),
                    ),
            )
    }
}

impl Render for AppRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (active_page, media_drag, inspector_panel_w) = {
            let gs = self.global.read(cx);
            (
                gs.active_page,
                gs.media_pool_drag.clone(),
                gs.inspector_panel_width(),
            )
        };
        if active_page != AppPage::Editor && self.inspector_expand_modal_open {
            self.close_inspector_expand_modal(cx);
        }
        if !self.ai_chat_widget_open && self.ai_chat_expand_modal_open {
            self.close_ai_chat_expand_modal(cx);
        }
        let timeline_low_load_effective = if active_page == AppPage::Editor {
            self.timeline
                .update(cx, |timeline, cx| timeline.is_low_load_mode_effective(cx))
        } else {
            false
        };
        self.ensure_chat_widget_input(window, cx);
        if self.ai_chat_send_on_next_render {
            self.ai_chat_send_on_next_render = false;
            self.send_chat_widget_prompt(window, cx);
        }
        let display_modal_overlay = self.timeline.update(cx, |timeline, cx| {
            render_display_settings_modal_overlay(timeline, window, cx)
        });
        let export_modal_overlay = self.timeline.update(cx, |timeline, cx| {
            render_export_modal_overlay(timeline, window, cx)
        });
        let inspector_layer_fx_modal_overlay = if timeline_low_load_effective {
            div()
        } else {
            self.inspector.update(cx, |inspector, cx| {
                inspector.render_layer_fx_script_modal_overlay(window, cx)
            })
        };
        let media_pool_expand_modal_overlay = if timeline_low_load_effective {
            div()
        } else {
            self.editor.update(cx, |editor, cx| {
                editor.render_media_pool_expand_modal_overlay(window, cx)
            })
        };

        let editor_left_panel = if timeline_low_load_effective {
            div()
                .w(px(300.0))
                .min_w(px(220.0))
                .h_full()
                .min_h_0()
                .bg(rgb(0x09090b))
                .border_r_1()
                .border_color(white().opacity(0.10))
                .flex()
                .items_start()
                .px_4()
                .py_3()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(white().opacity(0.78))
                        .child("Media Pool"),
                )
                .into_any_element()
        } else {
            self.editor.clone().into_any_element()
        };

        let editor_right_panel = if timeline_low_load_effective {
            div()
                .w(px(inspector_panel_w))
                .h_full()
                .min_h_0()
                .bg(rgb(0x18181b))
                .border_l_1()
                .border_color(white().opacity(0.1))
                .flex()
                .items_start()
                .px_4()
                .py_3()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(white().opacity(0.78))
                        .child("Inspector"),
                )
                .into_any_element()
        } else if self.inspector_expand_modal_open {
            div()
                .w(px(0.0))
                .h_full()
                .min_h_0()
                .flex_shrink_0()
                .into_any_element()
        } else {
            self.inspector.clone().into_any_element()
        };

        let editor_layout = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .child(editor_left_panel)
                    .child(
                        div()
                            .flex_1()
                            .flex_shrink()
                            .min_w_0()
                            .min_h_0()
                            .h_full()
                            // Clip preview rendering to its own column so it cannot push/crop the inspector.
                            .overflow_hidden()
                            .child(self.preview.clone()),
                    )
                    .child(editor_right_panel),
            )
            .child(self.timeline.clone());

        let ai_srt_layout = div().flex_1().min_h_0().child(self.ai_srt_page.clone());
        let ai_agents_layout = div().flex_1().min_h_0().child(self.ai_agents_page.clone());
        let motionloom_layout = div().flex_1().min_h_0().child(self.motionloom_page.clone());
        let vector_lab_layout = div().flex_1().min_h_0().child(self.vector_lab_page.clone());
        let inspector_expand_toggle =
            if active_page == AppPage::Editor && !self.inspector_expand_modal_open {
                div()
                    .absolute()
                    .right(px(12.0))
                    .top(px(10.0))
                    .h(px(26.0))
                    .px_2()
                    .rounded_sm()
                    .border_1()
                    .border_color(white().opacity(0.2))
                    .bg(white().opacity(0.06))
                    .text_xs()
                    .text_color(white().opacity(0.86))
                    .cursor_pointer()
                    .child("Expand")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.open_inspector_expand_modal(cx);
                            cx.notify();
                        }),
                    )
            } else {
                div()
            };

        let inspector_expand_modal_overlay =
            if active_page == AppPage::Editor && self.inspector_expand_modal_open {
                let viewport_w = window.viewport_size().width / px(1.0);
                let viewport_h = window.viewport_size().height / px(1.0);
                let card_w = (viewport_w - 72.0).clamp(820.0, 1500.0);
                let card_h = (viewport_h - 72.0).clamp(560.0, 980.0);
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .bg(gpui_component::black().opacity(0.62))
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.close_inspector_expand_modal(cx);
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .w(px(card_w))
                            .h(px(card_h))
                            .rounded_md()
                            .bg(rgb(0x141419))
                            .border_1()
                            .border_color(white().opacity(0.16))
                            .p_3()
                            .overflow_hidden()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_, _, _, cx| {
                                    cx.stop_propagation();
                                }),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .h_full()
                                    .min_h_0()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(white().opacity(0.95))
                                                    .child("Inspector"),
                                            )
                                            .child(
                                                div()
                                                    .h(px(28.0))
                                                    .px_3()
                                                    .rounded_md()
                                                    .border_1()
                                                    .border_color(white().opacity(0.18))
                                                    .bg(white().opacity(0.05))
                                                    .text_xs()
                                                    .text_color(white().opacity(0.9))
                                                    .cursor_pointer()
                                                    .hover(|s| s.bg(white().opacity(0.10)))
                                                    .child("Close")
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(|this, _, _, cx| {
                                                            this.close_inspector_expand_modal(cx);
                                                            cx.notify();
                                                        }),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_h_0()
                                            .overflow_hidden()
                                            .child(self.inspector.clone()),
                                    ),
                            ),
                    )
            } else {
                div()
            };

        let (chat_panel_w, chat_panel_h) = if active_page == AppPage::AiAgents {
            (420.0, 350.0)
        } else {
            (420.0, 520.0)
        };
        let chat_widget_panel =
            if self.ai_chat_widget_open && !self.ai_chat_expand_modal_open {
                div().absolute().right(px(16.0)).bottom(px(16.0)).child(
                    self.render_chat_widget_card(chat_panel_w, chat_panel_h, true, window, cx),
                )
            } else {
                div()
            };
        let chat_widget_expand_modal = if self.ai_chat_widget_open && self.ai_chat_expand_modal_open
        {
            let viewport_w = window.viewport_size().width / px(1.0);
            let viewport_h = window.viewport_size().height / px(1.0);
            let card_w = (viewport_w - 72.0).clamp(760.0, 1280.0);
            let card_h = (viewport_h - 72.0).clamp(520.0, 940.0);
            // Expanded chat keeps the same widget content but gives long
            // conversations a dedicated overlay-sized reading area.
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .right_0()
                .bg(gpui_component::black().opacity(0.62))
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.close_ai_chat_expand_modal(cx);
                    }),
                )
                .child(
                    div()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|_, _, _, cx| {
                                cx.stop_propagation();
                            }),
                        )
                        .child(self.render_chat_widget_card(card_w, card_h, false, window, cx)),
                )
        } else {
            div()
        };
        let media_pool_context_menu = self.render_media_pool_context_menu(window, cx);
        let media_dependency_modal = self.render_media_dependency_modal(cx);
        let preview_memory_budget_modal = self.render_preview_memory_budget_modal(cx);
        // Render silence preview modal overlay (ACP inspect-intent flow)
        let silence_preview_modal = self.render_silence_preview_modal(window, cx);

        let global_for_drag_move = self.global.clone();
        let global_for_drag_up = self.global.clone();

        let media_drag_ghost = if let Some(drag) = media_drag {
            // Draw a simple ghost card during media-pool drag interactions.
            div()
                .absolute()
                .left(px(drag.cursor_x + 14.0))
                .top(px(drag.cursor_y + 14.0))
                .rounded_md()
                .border_1()
                .border_color(white().opacity(0.25))
                .bg(rgb(0x111827))
                .px_3()
                .py_2()
                .text_xs()
                .text_color(white().opacity(0.92))
                .child(format!("Drop: {}", drag.name))
        } else {
            div()
        };

        let nav_button = |icon_path: &'static str, page: AppPage, active: bool| {
            let global_for_nav = self.global.clone();
            let mut base = div()
                .w(px(44.0))
                .h(px(44.0))
                .rounded_md()
                .border_1()
                .border_color(white().opacity(if active { 0.45 } else { 0.12 }))
                .bg(if active { rgb(0x1f2937) } else { rgb(0x0b0b0e) })
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .path(icon_path)
                        .w(px(20.0))
                        .h(px(20.0))
                        .text_color(white().opacity(if active { 0.95 } else { 0.6 })),
                );
            if !active {
                base = base.hover(|s| s.bg(white().opacity(0.06))).cursor_pointer();
            }
            base.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.close_inspector_expand_modal(cx);
                    global_for_nav.update(cx, |gs, cx| {
                        gs.set_active_page(page);
                        cx.notify();
                    });
                    cx.notify();
                }),
            )
        };
        let chat_widget_button = |active: bool| {
            let mut base = div()
                .w(px(44.0))
                .h(px(44.0))
                .rounded_lg()
                .border_1()
                .border_color(white().opacity(if active { 0.55 } else { 0.18 }))
                .bg(if active { rgb(0x0f3a2a) } else { rgb(0x0b0b0e) })
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .path("icons/robot_chat.svg")
                        .w(px(20.0))
                        .h(px(20.0))
                        .text_color(white().opacity(if active { 0.98 } else { 0.78 })),
                );
            if !active {
                base = base.hover(|s| s.bg(white().opacity(0.06))).cursor_pointer();
            }
            base.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.close_ai_chat_expand_modal(cx);
                    this.ai_chat_widget_open = !this.ai_chat_widget_open;
                    cx.notify();
                }),
            )
        };

        div()
            .size_full()
            .relative()
            .on_mouse_move(
                cx.listener(move |_this, evt: &MouseMoveEvent, _window, cx| {
                    // Update drag cursor globally so ghost follows pointer across the app.
                    let x = evt.position.x / px(1.0);
                    let y = evt.position.y / px(1.0);
                    global_for_drag_move.update(cx, |gs, cx| {
                        if gs.media_pool_drag.is_some() {
                            gs.update_media_pool_drag_cursor(x, y);
                            cx.emit(MediaPoolUiEvent::DragCursorChanged);
                        }
                    });
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |_this, evt: &MouseUpEvent, window, cx| {
                    // Only clear here when release happens outside timeline region.
                    // Timeline owns drop handling inside its panel and will clear drag itself.
                    global_for_drag_up.update(cx, |gs, cx| {
                        let release_y = evt.position.y / px(1.0);
                        let timeline_top = (window.viewport_size().height / px(1.0)
                            - TIMELINE_PANEL_HEIGHT_PX)
                            .max(0.0);
                        let released_outside_timeline =
                            gs.active_page != AppPage::Editor || release_y < timeline_top;
                        if gs.media_pool_drag.is_some() && released_outside_timeline {
                            // Cancel pending drag state and clear transient drag hint when drop is aborted.
                            gs.clear_media_pool_drag();
                            gs.ui_notice = None;
                            cx.emit(MediaPoolUiEvent::StateChanged);
                        }
                    });
                }),
            )
            .flex()
            .child(
                div()
                    .w(px(64.0))
                    .flex_shrink_0()
                    .h_full()
                    .bg(rgb(0x0b0b0e))
                    .border_r_1()
                    .border_color(white().opacity(0.12))
                    .py_2()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .child(nav_button(
                        "icons/timeline.svg",
                        AppPage::Editor,
                        active_page == AppPage::Editor,
                    ))
                    .child(nav_button(
                        "icons/script.svg",
                        AppPage::AiSrt,
                        active_page == AppPage::AiSrt,
                    ))
                    .child(nav_button(
                        "icons/ai.svg",
                        AppPage::AiAgents,
                        active_page == AppPage::AiAgents,
                    ))
                    .child(nav_button(
                        "icons/motionloom.svg",
                        AppPage::MotionLoom,
                        active_page == AppPage::MotionLoom,
                    ))
                    .child(nav_button(
                        "icons/script.svg",
                        AppPage::VectorLab,
                        active_page == AppPage::VectorLab,
                    ))
                    .child(div().h(px(18.0)))
                    .child(chat_widget_button(self.ai_chat_widget_open)),
            )
            .child(match active_page {
                AppPage::Editor => editor_layout,
                AppPage::AiSrt => ai_srt_layout,
                AppPage::AiAgents => ai_agents_layout,
                AppPage::MotionLoom => motionloom_layout,
                AppPage::VectorLab => vector_lab_layout,
            })
            .child(inspector_expand_toggle)
            .child(media_drag_ghost)
            .child(display_modal_overlay)
            .child(export_modal_overlay)
            .child(media_pool_expand_modal_overlay)
            .child(inspector_expand_modal_overlay)
            .child(inspector_layer_fx_modal_overlay)
            .child(chat_widget_panel)
            .child(chat_widget_expand_modal)
            .child(media_dependency_modal)
            .child(preview_memory_budget_modal)
            .child(silence_preview_modal)
            .child(media_pool_context_menu)
    }
}
