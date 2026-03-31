// =========================================
// =========================================
// src/ui/display_settings_modal.rs
use gpui::{Context, Hsla, MouseButton, Window, div, prelude::*, px, rgb};
use gpui_component::{black, scroll::ScrollableElement, white};

use crate::core::export::{export_fps_choices_for_ui, export_resolution_choices_for_ui};

use super::timeline_panel::TimelinePanel;

#[derive(Clone, Copy, PartialEq, Eq)]
enum CanvasOrientation {
    Landscape,
    Portrait,
    Square,
}

#[derive(Clone)]
struct CanvasResolutionPreset {
    id: &'static str,
    display_label: String,
    orientation: CanvasOrientation,
}

fn format_canvas_choice(w: u32, h: u32) -> String {
    format!("{w}x{h}")
}

fn parse_canvas_choice_str(choice: &str) -> Option<(u32, u32)> {
    let (w, h) = choice.split_once('x')?;
    let w = w.parse::<u32>().ok()?.max(2);
    let h = h.parse::<u32>().ok()?.max(2);
    Some((w, h))
}

fn gcd_u32(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a.max(1)
}

fn ratio_label_from_dims(w: u32, h: u32) -> String {
    let d = gcd_u32(w, h);
    format!("{}:{}", w / d, h / d)
}

pub fn display_ratio_label(canvas_w: f32, canvas_h: f32) -> String {
    let w = canvas_w.round().max(2.0) as u32;
    let h = canvas_h.round().max(2.0) as u32;
    ratio_label_from_dims(w, h)
}

fn orientation_for_dims(w: u32, h: u32) -> CanvasOrientation {
    if w > h {
        CanvasOrientation::Landscape
    } else if h > w {
        CanvasOrientation::Portrait
    } else {
        CanvasOrientation::Square
    }
}

fn format_resolution_preset_label(base_label: &str, id: &str, ratio: &str) -> String {
    if base_label.contains(id) {
        format!(
            "{} {}",
            base_label.replacen(id, &format!("({id})"), 1),
            ratio
        )
    } else {
        format!("{base_label} ({id}) {ratio}")
    }
}

fn canvas_resolution_presets() -> Vec<CanvasResolutionPreset> {
    export_resolution_choices_for_ui()
        .iter()
        .filter_map(|(id, label)| {
            if *id == "canvas" {
                return None;
            }
            let (w, h) = parse_canvas_choice_str(id)?;
            let ratio = ratio_label_from_dims(w, h);
            Some(CanvasResolutionPreset {
                id,
                display_label: format_resolution_preset_label(label, id, &ratio),
                orientation: orientation_for_dims(w, h),
            })
        })
        .collect()
}

fn preset_id_from_canvas_choice(choice: &str) -> Option<String> {
    export_resolution_choices_for_ui()
        .iter()
        .find_map(|(id, _)| {
            if *id == "canvas" {
                return None;
            }
            if *id == choice {
                Some((*id).to_string())
            } else {
                None
            }
        })
}

pub struct DisplaySettingsModalState {
    pub open: bool,
    pub canvas_choice: String,
    pub preview_fps: u32,
    pub selected_preset_id: Option<String>,
}

impl DisplaySettingsModalState {
    pub fn new() -> Self {
        let default_canvas_choice = "1920x1080".to_string();
        Self {
            open: false,
            canvas_choice: default_canvas_choice.clone(),
            preview_fps: 60,
            selected_preset_id: preset_id_from_canvas_choice(&default_canvas_choice),
        }
    }

    pub fn open_with_current(&mut self, canvas_w: f32, canvas_h: f32, preview_fps: u32) {
        let w = canvas_w.round().max(1.0) as u32;
        let h = canvas_h.round().max(1.0) as u32;
        self.canvas_choice = format_canvas_choice(w, h);
        self.selected_preset_id = preset_id_from_canvas_choice(&self.canvas_choice);
        self.preview_fps = if export_fps_choices_for_ui().contains(&preview_fps) {
            preview_fps
        } else {
            30
        };
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    fn select_canvas_preset(&mut self, preset_id: &str) {
        let Some((w, h)) = parse_canvas_choice_str(preset_id) else {
            return;
        };
        self.canvas_choice = format_canvas_choice(w, h);
        self.selected_preset_id = Some(preset_id.to_string());
    }

    fn selected_ratio_label(&self) -> String {
        let Some((w, h)) = parse_canvas_choice_str(&self.canvas_choice) else {
            return "16:9".to_string();
        };
        ratio_label_from_dims(w, h)
    }

    fn selected_canvas_summary(&self) -> String {
        if let Some(id) = self.selected_preset_id.as_deref()
            && let Some(preset) = canvas_resolution_presets().into_iter().find(|p| p.id == id)
        {
            return preset.display_label;
        }
        format!(
            "Custom ({}) {}",
            self.canvas_choice,
            self.selected_ratio_label()
        )
    }
}

fn modal_btn(label: impl Into<String>) -> gpui::Div {
    div()
        .h(px(28.0))
        .px_3()
        .rounded_lg()
        .border_1()
        .border_color(white().opacity(0.12))
        .bg(white().opacity(0.05))
        .text_color(white().opacity(0.85))
        .hover(|s| s.bg(white().opacity(0.10)))
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .child(label.into())
}

fn render_canvas_preset_row(
    panel: &TimelinePanel,
    cx: &mut Context<TimelinePanel>,
    orientation: CanvasOrientation,
) -> gpui::Div {
    let selected_id = panel.display_settings_modal.selected_preset_id.clone();
    let mut row = div().flex().flex_wrap().gap_2();
    for preset in canvas_resolution_presets()
        .into_iter()
        .filter(|preset| preset.orientation == orientation)
    {
        let is_selected = selected_id.as_deref() == Some(preset.id);
        let id_owned = preset.id.to_string();
        let label_owned = preset.display_label.clone();
        row = row.child(
            div()
                .h(px(28.0))
                .px_2()
                .rounded_md()
                .border_1()
                .border_color(if is_selected {
                    Hsla::from(rgb(0x60a5fa)).opacity(0.95)
                } else {
                    white().opacity(0.14)
                })
                .bg(if is_selected {
                    Hsla::from(rgb(0x1e3a8a)).opacity(0.35)
                } else {
                    white().opacity(0.04)
                })
                .text_xs()
                .text_color(white().opacity(0.9))
                .hover(|s| s.bg(white().opacity(0.1)))
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .child(label_owned)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.display_settings_modal.select_canvas_preset(&id_owned);
                        cx.notify();
                    }),
                ),
        );
    }
    row
}

pub fn render_display_settings_modal_overlay(
    panel: &mut TimelinePanel,
    _window: &mut Window,
    cx: &mut Context<TimelinePanel>,
) -> gpui::Div {
    if !panel.display_settings_modal.open {
        return div();
    }

    let landscape_row = render_canvas_preset_row(panel, cx, CanvasOrientation::Landscape);
    let portrait_row = render_canvas_preset_row(panel, cx, CanvasOrientation::Portrait);
    let square_row = render_canvas_preset_row(panel, cx, CanvasOrientation::Square);

    let selected_fps = panel.display_settings_modal.preview_fps;
    let mut fps_grid = div().flex().flex_wrap().gap_2();
    for fps in export_fps_choices_for_ui().iter().copied() {
        let selected = fps == selected_fps;
        fps_grid = fps_grid.child(
            div()
                .h(px(28.0))
                .px_2()
                .rounded_md()
                .border_1()
                .border_color(if selected {
                    Hsla::from(rgb(0x60a5fa)).opacity(0.95)
                } else {
                    white().opacity(0.14)
                })
                .bg(if selected {
                    Hsla::from(rgb(0x1e3a8a)).opacity(0.35)
                } else {
                    white().opacity(0.04)
                })
                .text_xs()
                .text_color(white().opacity(0.9))
                .hover(|s| s.bg(white().opacity(0.1)))
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .child(format!("{fps} fps"))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.display_settings_modal.preview_fps = fps;
                        cx.notify();
                    }),
                ),
        );
    }

    let selected_canvas_for_apply = panel.display_settings_modal.canvas_choice.clone();
    let selected_fps_for_apply = panel.display_settings_modal.preview_fps;
    let selected_ratio_for_apply = panel.display_settings_modal.selected_ratio_label();
    let global_for_apply = panel.global.clone();

    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left_0()
        .right_0()
        .bg(black().opacity(0.6))
        .flex()
        .flex_col()
        .items_center()
        .justify_start()
        .pt(px(44.0))
        .pb(px(20.0))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.display_settings_modal.close();
                cx.notify();
            }),
        )
        .child(
            div()
                .w(px(860.0))
                .max_h(px(820.0))
                .rounded_md()
                .bg(rgb(0x1f1f23))
                .border_1()
                .border_color(white().opacity(0.12))
                .p_3()
                .flex()
                .flex_col()
                .gap_3()
                .overflow_y_scrollbar()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |_this, _, _, cx| {
                        cx.stop_propagation();
                    }),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(white().opacity(0.9))
                        .child("Display Settings"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.55))
                        .child(
                            "Choose canvas from FFmpeg export resolutions. Export with 'Match Canvas' will follow this size.",
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.5))
                        .child("Canvas Resolution Presets"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.40))
                        .child("Landscape"),
                )
                .child(landscape_row)
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.40))
                        .child("Portrait"),
                )
                .child(portrait_row)
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.40))
                        .child("Square"),
                )
                .child(square_row)
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.45))
                        .child(format!(
                            "Selected canvas: {}",
                            panel.display_settings_modal.selected_canvas_summary()
                        )),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.5))
                        .child("Preview FPS"),
                )
                .child(fps_grid)
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.45))
                        .child(format!(
                            "Selected preview FPS: {} fps",
                            panel.display_settings_modal.preview_fps
                        )),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(modal_btn("Cancel").on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.display_settings_modal.close();
                                cx.notify();
                            }),
                        ))
                        .child(modal_btn("Apply").on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                let mut applied = false;
                                let (w, h) = match selected_canvas_for_apply
                                    .split_once('x')
                                    .and_then(|(w, h)| {
                                        Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))
                                    }) {
                                    Some((w, h)) => (w.max(2) as f32, h.max(2) as f32),
                                    None => {
                                        global_for_apply.update(cx, |gs, cx| {
                                            gs.ui_notice =
                                                Some("Invalid canvas resolution choice.".to_string());
                                            cx.notify();
                                        });
                                        this.display_settings_modal.close();
                                        cx.notify();
                                        return;
                                    }
                                };
                                global_for_apply.update(cx, |gs, cx| {
                                    gs.set_canvas_size(w, h);
                                    gs.set_preview_fps_value(selected_fps_for_apply);
                                    gs.ui_notice = Some(format!(
                                        "Display updated: {}x{} ({}) @ {}fps",
                                        w.round() as u32,
                                        h.round() as u32,
                                        selected_ratio_for_apply,
                                        selected_fps_for_apply
                                    ));
                                    cx.notify();
                                    applied = true;
                                });
                                if applied {
                                    this.display_settings_modal.close();
                                    cx.notify();
                                }
                            }),
                        )),
                ),
        )
}
