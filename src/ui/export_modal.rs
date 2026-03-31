use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gpui::{
    Context, Entity, Focusable, MouseButton, PathPromptOptions, SharedString, Subscription, Window,
    div, prelude::*, px, rgb, rgba,
};
use gpui_component::{
    black,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
    white,
};

use crate::core::export::{
    ExportMode, ExportPreset, ExportProgress, ExportRange, ExportSettings, FfmpegExporter,
    export_fps_choices_for_ui, export_resolution_choices_for_ui, is_cancelled_export_error,
};

use super::timeline_panel::TimelinePanel;

#[derive(Clone, Debug)]
struct ExportChoiceOption {
    id: String,
    label: String,
}

impl ExportChoiceOption {
    fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

impl SelectItem for ExportChoiceOption {
    type Value = String;

    fn title(&self) -> SharedString {
        SharedString::from(self.label.clone())
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

type ChoiceSelectState = SelectState<SearchableVec<ExportChoiceOption>>;

pub struct ExportModalState {
    pub open: bool,
    pub dir: Option<PathBuf>,
    pub name: String,
    pub overwrite_path: Option<String>,
    pub preset: ExportPreset,
    pub mode: ExportMode,
    pub settings: ExportSettings,
    pub target_resolution: String,
    pub use_custom_range: bool,
    pub range_start_seconds: String,
    pub range_end_seconds: String,
    pub range_error: Option<String>,
    pub cancel_signal: Option<Arc<AtomicBool>>,

    input: Option<Entity<InputState>>,
    input_sub: Option<Subscription>,
    range_start_input: Option<Entity<InputState>>,
    range_start_sub: Option<Subscription>,
    range_end_input: Option<Entity<InputState>>,
    range_end_sub: Option<Subscription>,

    fps_select: Option<Entity<ChoiceSelectState>>,
    fps_select_sub: Option<Subscription>,

    resolution_select: Option<Entity<ChoiceSelectState>>,
    resolution_select_sub: Option<Subscription>,

    preset_select: Option<Entity<ChoiceSelectState>>,
    preset_select_sub: Option<Subscription>,
    mode_select: Option<Entity<ChoiceSelectState>>,
    mode_select_sub: Option<Subscription>,

    encoder_preset_select: Option<Entity<ChoiceSelectState>>,
    encoder_preset_select_sub: Option<Subscription>,

    audio_bitrate_select: Option<Entity<ChoiceSelectState>>,
    audio_bitrate_select_sub: Option<Subscription>,
}

impl ExportModalState {
    const fn default_preset_for_platform() -> ExportPreset {
        #[cfg(target_os = "macos")]
        {
            ExportPreset::H264VideotoolboxMp4
        }
        #[cfg(not(target_os = "macos"))]
        {
            ExportPreset::H264Mp4
        }
    }

    pub fn new() -> Self {
        Self {
            open: false,
            dir: None,
            name: String::new(),
            overwrite_path: None,
            preset: Self::default_preset_for_platform(),
            mode: ExportMode::SmartUniversal,
            settings: ExportSettings::default(),
            target_resolution: "canvas".to_string(),
            use_custom_range: false,
            range_start_seconds: "0".to_string(),
            range_end_seconds: String::new(),
            range_error: None,
            cancel_signal: None,

            input: None,
            input_sub: None,
            range_start_input: None,
            range_start_sub: None,
            range_end_input: None,
            range_end_sub: None,

            fps_select: None,
            fps_select_sub: None,
            resolution_select: None,
            resolution_select_sub: None,
            preset_select: None,
            preset_select_sub: None,
            mode_select: None,
            mode_select_sub: None,
            encoder_preset_select: None,
            encoder_preset_select_sub: None,
            audio_bitrate_select: None,
            audio_bitrate_select_sub: None,
        }
    }

    pub fn open_with_default_name(&mut self, default_dir: PathBuf) {
        if self.dir.is_none() {
            self.dir = Some(default_dir);
        }
        let current = self.name.trim();
        let is_default_name = current
            .strip_prefix("sequence_export_")
            .map(|suffix| !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or(false);
        if current.is_empty() || is_default_name {
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::from_secs(0))
                .as_secs();
            self.name = format!("sequence_export_{ts}");
        }
        self.overwrite_path = None;
        self.range_error = None;
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.overwrite_path = None;
        self.range_error = None;
    }

    pub fn output_resolution(&self, canvas_w: f32, canvas_h: f32) -> (f32, f32) {
        if self.target_resolution == "canvas" {
            return (canvas_w, canvas_h);
        }

        let Some((w_str, h_str)) = self.target_resolution.split_once('x') else {
            return (canvas_w, canvas_h);
        };
        let Ok(w) = w_str.parse::<u32>() else {
            return (canvas_w, canvas_h);
        };
        let Ok(h) = h_str.parse::<u32>() else {
            return (canvas_w, canvas_h);
        };

        (w.max(2) as f32, h.max(2) as f32)
    }

    pub fn output_range(&self, timeline_total: Duration) -> Result<Option<ExportRange>, String> {
        if !self.use_custom_range {
            return Ok(None);
        }

        let total_secs = timeline_total.as_secs_f64();
        if total_secs <= 0.0 {
            return Err("Timeline is empty, nothing to export.".to_string());
        }

        let start_txt = self.range_start_seconds.trim();
        let start_secs = if start_txt.is_empty() {
            0.0
        } else {
            start_txt
                .parse::<f64>()
                .map_err(|_| "Range start must be a number of seconds.".to_string())?
        };

        let end_txt = self.range_end_seconds.trim();
        let end_secs = if end_txt.is_empty() {
            total_secs
        } else {
            end_txt
                .parse::<f64>()
                .map_err(|_| "Range end must be a number of seconds.".to_string())?
        };

        let start_secs = start_secs.max(0.0).min(total_secs);
        let end_secs = end_secs.max(0.0).min(total_secs);
        if end_secs <= start_secs + 0.001 {
            return Err("Range end must be greater than range start.".to_string());
        }

        let start = Duration::from_secs_f64(start_secs);
        let end = Duration::from_secs_f64(end_secs);
        Ok(Some(ExportRange { start, end }))
    }

    fn output_filename(&self) -> String {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs();
        let raw = if self.name.trim().is_empty() {
            format!("sequence_export_{ts}")
        } else {
            self.name.trim().to_string()
        };

        let ext = self.preset.file_extension();
        let path = Path::new(&raw);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("sequence_export");
        format!("{stem}.{ext}")
    }

    fn choice_items(items: &[(&str, &str)]) -> SearchableVec<ExportChoiceOption> {
        SearchableVec::new(
            items
                .iter()
                .map(|(id, label)| ExportChoiceOption::new(*id, *label))
                .collect::<Vec<_>>(),
        )
    }

    fn preset_choice_items() -> SearchableVec<ExportChoiceOption> {
        SearchableVec::new(
            ExportPreset::all_for_ui()
                .iter()
                .copied()
                .map(|preset| {
                    let category = if preset.is_audio_only() {
                        "Audio"
                    } else {
                        "Video"
                    };
                    ExportChoiceOption::new(preset.id(), format!("{category}: {}", preset.label()))
                })
                .collect::<Vec<_>>(),
        )
    }

    fn fps_choice_items() -> SearchableVec<ExportChoiceOption> {
        SearchableVec::new(
            export_fps_choices_for_ui()
                .iter()
                .copied()
                .map(|fps| ExportChoiceOption::new(fps.to_string(), format!("{fps} fps")))
                .collect::<Vec<_>>(),
        )
    }

    fn resolution_choice_items() -> SearchableVec<ExportChoiceOption> {
        SearchableVec::new(
            export_resolution_choices_for_ui()
                .iter()
                .map(|(id, label)| ExportChoiceOption::new(*id, *label))
                .collect::<Vec<_>>(),
        )
    }

    fn mode_choice_items() -> SearchableVec<ExportChoiceOption> {
        SearchableVec::new(
            ExportMode::all_for_ui()
                .iter()
                .copied()
                .map(|mode| ExportChoiceOption::new(mode.id(), mode.label()))
                .collect::<Vec<_>>(),
        )
    }

    fn ensure_input(&mut self, window: &mut Window, cx: &mut Context<TimelinePanel>) {
        if !self.open {
            return;
        }

        if self.input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Export name"));
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                this.export_modal.name = input.read(cx).value().to_string();
                this.export_modal.overwrite_path = None;
                cx.notify();
            });
            self.input = Some(input);
            self.input_sub = Some(sub);
        }

        if self.range_start_input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("0.0"));
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                this.export_modal.range_start_seconds = input.read(cx).value().to_string();
                this.export_modal.range_error = None;
                cx.notify();
            });
            self.range_start_input = Some(input);
            self.range_start_sub = Some(sub);
        }

        if self.range_end_input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("end"));
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                this.export_modal.range_end_seconds = input.read(cx).value().to_string();
                this.export_modal.range_error = None;
                cx.notify();
            });
            self.range_end_input = Some(input);
            self.range_end_sub = Some(sub);
        }
    }

    fn ensure_selects(&mut self, window: &mut Window, cx: &mut Context<TimelinePanel>) {
        if !self.open {
            return;
        }

        if self.preset_select.is_none() {
            let state = cx.new(|cx| {
                SelectState::new(Self::preset_choice_items(), None, window, cx).searchable(true)
            });
            let selected_id = self.preset.id().to_string();
            state.update(cx, |this, cx| {
                this.set_selected_value(&selected_id, window, cx);
            });
            let sub = cx.subscribe(
                &state,
                |this, _, ev: &SelectEvent<SearchableVec<ExportChoiceOption>>, cx| {
                    let SelectEvent::Confirm(value) = ev;
                    let Some(value) = value else {
                        return;
                    };
                    let Some(preset) = ExportPreset::from_id(value.as_str()) else {
                        return;
                    };
                    this.export_modal.preset = preset;
                    this.export_modal.overwrite_path = None;
                    cx.notify();
                },
            );
            self.preset_select = Some(state);
            self.preset_select_sub = Some(sub);
        }

        if self.mode_select.is_none() {
            let state = cx.new(|cx| {
                SelectState::new(Self::mode_choice_items(), None, window, cx).searchable(false)
            });
            let selected_id = self.mode.id().to_string();
            state.update(cx, |this, cx| {
                this.set_selected_value(&selected_id, window, cx);
            });
            let sub = cx.subscribe(
                &state,
                |this, _, ev: &SelectEvent<SearchableVec<ExportChoiceOption>>, cx| {
                    let SelectEvent::Confirm(value) = ev;
                    let Some(value) = value else {
                        return;
                    };
                    let Some(mode) = ExportMode::from_id(value.as_str()) else {
                        return;
                    };
                    this.export_modal.mode = mode;
                    cx.notify();
                },
            );
            self.mode_select = Some(state);
            self.mode_select_sub = Some(sub);
        }

        if self.fps_select.is_none() {
            let state = cx.new(|cx| {
                SelectState::new(Self::fps_choice_items(), None, window, cx).searchable(false)
            });
            let selected_id = self.settings.fps.to_string();
            state.update(cx, |this, cx| {
                this.set_selected_value(&selected_id, window, cx);
            });
            let sub = cx.subscribe(
                &state,
                |this, _, ev: &SelectEvent<SearchableVec<ExportChoiceOption>>, cx| {
                    let SelectEvent::Confirm(value) = ev;
                    let Some(value) = value else {
                        return;
                    };
                    if let Ok(fps) = value.parse::<u32>() {
                        this.export_modal.settings.fps = fps;
                        cx.notify();
                    }
                },
            );
            self.fps_select = Some(state);
            self.fps_select_sub = Some(sub);
        }

        if self.resolution_select.is_none() {
            let state = cx.new(|cx| {
                SelectState::new(Self::resolution_choice_items(), None, window, cx)
                    .searchable(false)
            });
            let selected_id = self.target_resolution.clone();
            state.update(cx, |this, cx| {
                this.set_selected_value(&selected_id, window, cx);
            });
            let sub = cx.subscribe(
                &state,
                |this, _, ev: &SelectEvent<SearchableVec<ExportChoiceOption>>, cx| {
                    let SelectEvent::Confirm(value) = ev;
                    let Some(value) = value else {
                        return;
                    };
                    this.export_modal.target_resolution = value.clone();
                    this.export_modal.overwrite_path = None;
                    cx.notify();
                },
            );
            self.resolution_select = Some(state);
            self.resolution_select_sub = Some(sub);
        }

        if self.encoder_preset_select.is_none() {
            let state = cx.new(|cx| {
                SelectState::new(
                    Self::choice_items(&[
                        ("ultrafast", "ultrafast"),
                        ("superfast", "superfast"),
                        ("veryfast", "veryfast"),
                        ("faster", "faster"),
                        ("fast", "fast"),
                        ("medium", "medium"),
                        ("slow", "slow"),
                        ("slower", "slower"),
                        ("veryslow", "veryslow"),
                    ]),
                    None,
                    window,
                    cx,
                )
                .searchable(false)
            });
            let selected_id = self.settings.encoder_preset.clone();
            state.update(cx, |this, cx| {
                this.set_selected_value(&selected_id, window, cx);
            });
            let sub = cx.subscribe(
                &state,
                |this, _, ev: &SelectEvent<SearchableVec<ExportChoiceOption>>, cx| {
                    let SelectEvent::Confirm(value) = ev;
                    let Some(value) = value else {
                        return;
                    };
                    this.export_modal.settings.encoder_preset = value.clone();
                    cx.notify();
                },
            );
            self.encoder_preset_select = Some(state);
            self.encoder_preset_select_sub = Some(sub);
        }

        if self.audio_bitrate_select.is_none() {
            let state = cx.new(|cx| {
                SelectState::new(
                    Self::choice_items(&[
                        ("96", "96 kbps"),
                        ("128", "128 kbps"),
                        ("160", "160 kbps"),
                        ("192", "192 kbps"),
                        ("256", "256 kbps"),
                        ("320", "320 kbps"),
                    ]),
                    None,
                    window,
                    cx,
                )
                .searchable(false)
            });
            let selected_id = self.settings.audio_bitrate_kbps.to_string();
            state.update(cx, |this, cx| {
                this.set_selected_value(&selected_id, window, cx);
            });
            let sub = cx.subscribe(
                &state,
                |this, _, ev: &SelectEvent<SearchableVec<ExportChoiceOption>>, cx| {
                    let SelectEvent::Confirm(value) = ev;
                    let Some(value) = value else {
                        return;
                    };
                    if let Ok(kbps) = value.parse::<u32>() {
                        this.export_modal.settings.audio_bitrate_kbps = kbps;
                        cx.notify();
                    }
                },
            );
            self.audio_bitrate_select = Some(state);
            self.audio_bitrate_select_sub = Some(sub);
        }
    }

    fn sync_input(&mut self, window: &mut Window, cx: &mut Context<TimelinePanel>) {
        if !self.open {
            return;
        }

        if let Some(input) = self.input.as_ref() {
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            if focused {
            } else {
                let current = input.read(cx).value().to_string();
                if current != self.name {
                    let value = self.name.clone();
                    input.update(cx, |input, cx| {
                        input.set_value(value, window, cx);
                    });
                }
            }
        }

        if let Some(input) = self.range_start_input.as_ref() {
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            if !focused {
                let current = input.read(cx).value().to_string();
                if current != self.range_start_seconds {
                    let value = self.range_start_seconds.clone();
                    input.update(cx, |input, cx| {
                        input.set_value(value, window, cx);
                    });
                }
            }
        }

        if let Some(input) = self.range_end_input.as_ref() {
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            if !focused {
                let current = input.read(cx).value().to_string();
                if current != self.range_end_seconds {
                    let value = self.range_end_seconds.clone();
                    input.update(cx, |input, cx| {
                        input.set_value(value, window, cx);
                    });
                }
            }
        }
    }
}

fn modal_btn(label: &'static str) -> gpui::Div {
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
        .child(label)
}

fn fmt_time_short(d: Duration) -> String {
    let total = d.as_secs();
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{:01}:{:02}:{:02}", h, m, s)
    } else {
        format!("{:02}:{:02}", m, s)
    }
}

enum ExportWorkerEvent {
    Progress(ExportProgress),
    Finished(Result<String, String>),
}

pub fn render_export_modal_overlay(
    panel: &mut TimelinePanel,
    window: &mut Window,
    cx: &mut Context<TimelinePanel>,
) -> gpui::Div {
    panel.export_modal.ensure_input(window, cx);
    panel.export_modal.ensure_selects(window, cx);
    panel.export_modal.sync_input(window, cx);

    let (
        export_in_progress,
        export_progress_ratio,
        export_progress_rendered,
        export_progress_total,
        export_eta,
        export_out_path,
        media_tools_ready_for_export,
    ) = {
        let gs = panel.global.read(cx);
        (
            gs.export_in_progress,
            gs.export_progress_ratio,
            gs.export_progress_rendered,
            gs.export_progress_total,
            gs.export_eta,
            gs.export_last_out_path.clone(),
            gs.media_tools_ready_for_export(),
        )
    };

    if !panel.export_modal.open {
        if !export_in_progress {
            return div();
        }

        let pct = (export_progress_ratio * 100.0).round().clamp(0.0, 100.0);
        let bar_total_w = 420.0_f32;
        let bar_fill_w = (bar_total_w * export_progress_ratio.clamp(0.0, 1.0)).max(2.0);
        let eta_label = export_eta
            .map(fmt_time_short)
            .unwrap_or_else(|| "--:--".to_string());
        let progress_label = format!(
            "{} / {}",
            fmt_time_short(export_progress_rendered),
            fmt_time_short(export_progress_total)
        );
        let out_label = export_out_path
            .as_deref()
            .and_then(|p| {
                Path::new(p)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| "Output".to_string());
        let cancel_signal = panel.export_modal.cancel_signal.clone();
        let is_stopping = cancel_signal
            .as_ref()
            .map(|s| s.load(Ordering::Relaxed))
            .unwrap_or(false);

        return div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right_0()
            .bg(black().opacity(0.52))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(520.0))
                    .rounded_md()
                    .bg(rgb(0x1f1f23))
                    .border_1()
                    .border_color(white().opacity(0.14))
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.95))
                            .child("Exporting"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.62))
                            .child(out_label),
                    )
                    .child(
                        div()
                            .w(px(bar_total_w))
                            .h(px(10.0))
                            .rounded_lg()
                            .bg(white().opacity(0.12))
                            .child(
                                div()
                                    .w(px(bar_fill_w))
                                    .h(px(10.0))
                                    .rounded_lg()
                                    .bg(rgb(0x60a5fa)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .w(px(bar_total_w))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.85))
                                    .child(format!("{pct:.0}%")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.75))
                                    .child(progress_label),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0xfbbf24))
                                    .child(format!("ETA {eta_label}")),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .w(px(bar_total_w))
                            .child(if let Some(signal) = cancel_signal {
                                if is_stopping {
                                    modal_btn("Stopping…")
                                        .text_color(white().opacity(0.55))
                                        .bg(white().opacity(0.04))
                                        .into_any_element()
                                } else {
                                    modal_btn("Stop")
                                        .text_color(white().opacity(0.85))
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |this, _, _, cx| {
                                                signal.store(true, Ordering::Relaxed);
                                                this.global.update(cx, |gs, cx| {
                                                    gs.ui_notice =
                                                        Some("Stopping export…".to_string());
                                                    cx.notify();
                                                });
                                            }),
                                        )
                                        .into_any_element()
                                }
                            } else {
                                div().into_any_element()
                            }),
                    ),
            );
    }

    if export_in_progress {
        return div();
    }

    let dir_label = panel
        .export_modal
        .dir
        .as_ref()
        .map(|dir| dir.display().to_string())
        .unwrap_or_else(|| TimelinePanel::default_export_dir().display().to_string());

    let overwrite_warning = if let Some(path) = panel.export_modal.overwrite_path.as_ref() {
        let name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        div().text_xs().text_color(rgb(0xf87171)).child(format!(
            "File already exists: {name}. Click Export again to overwrite."
        ))
    } else {
        div()
    };
    let dependency_warning = if media_tools_ready_for_export {
        div().into_any_element()
    } else {
        div()
            .rounded_md()
            .border_1()
            .border_color(rgba(0xf8717180))
            .bg(rgba(0x3f1d1d59))
            .px_2()
            .py_2()
            .text_xs()
            .text_color(rgb(0xfca5a5))
            .child("Export requires FFmpeg and FFprobe. Install tools first.")
            .into_any_element()
    };

    let export_btn_label = if panel.export_modal.overwrite_path.is_some() {
        "Overwrite"
    } else {
        "Export"
    };

    let input_elem = if let Some(input) = panel.export_modal.input.as_ref() {
        Input::new(input).h(px(32.0)).w_full().into_any_element()
    } else {
        div()
            .h(px(32.0))
            .rounded_sm()
            .bg(white().opacity(0.06))
            .into_any_element()
    };
    let range_start_elem = if let Some(input) = panel.export_modal.range_start_input.as_ref() {
        Input::new(input).h(px(32.0)).w_full().into_any_element()
    } else {
        div()
            .h(px(32.0))
            .rounded_sm()
            .bg(white().opacity(0.06))
            .into_any_element()
    };
    let range_end_elem = if let Some(input) = panel.export_modal.range_end_input.as_ref() {
        Input::new(input).h(px(32.0)).w_full().into_any_element()
    } else {
        div()
            .h(px(32.0))
            .rounded_sm()
            .bg(white().opacity(0.06))
            .into_any_element()
    };
    let range_error = if let Some(err) = panel.export_modal.range_error.as_ref() {
        div()
            .text_xs()
            .text_color(rgb(0xf87171))
            .child(err.clone())
            .into_any_element()
    } else {
        div().into_any_element()
    };

    let export_color_mode = panel.global.read(cx).export_color_mode;
    let selected_preset = panel.export_modal.preset;
    let selected_mode = panel.export_modal.mode;
    let crf_supported = selected_preset.supports_crf();
    let is_audio_only = selected_preset.is_audio_only();

    let preset_select_elem = if let Some(select) = panel.export_modal.preset_select.as_ref() {
        Select::new(select)
            .placeholder("Output preset")
            .menu_width(px(320.0))
            .into_any_element()
    } else {
        div()
            .text_xs()
            .text_color(white().opacity(0.5))
            .child("Output preset")
            .into_any_element()
    };
    let mode_select_elem = if let Some(select) = panel.export_modal.mode_select.as_ref() {
        Select::new(select)
            .placeholder("Export mode")
            .menu_width(px(320.0))
            .into_any_element()
    } else {
        div()
            .text_xs()
            .text_color(white().opacity(0.5))
            .child("Export mode")
            .into_any_element()
    };

    let fps_select_elem = if let Some(select) = panel.export_modal.fps_select.as_ref() {
        Select::new(select)
            .placeholder("FPS")
            .menu_width(px(200.0))
            .into_any_element()
    } else {
        div()
            .text_xs()
            .text_color(white().opacity(0.5))
            .child("FPS")
            .into_any_element()
    };

    let resolution_select_elem = if let Some(select) = panel.export_modal.resolution_select.as_ref()
    {
        Select::new(select)
            .placeholder("Resolution")
            .menu_width(px(240.0))
            .into_any_element()
    } else {
        div()
            .text_xs()
            .text_color(white().opacity(0.5))
            .child("Resolution")
            .into_any_element()
    };

    let encoder_preset_select_elem =
        if let Some(select) = panel.export_modal.encoder_preset_select.as_ref() {
            Select::new(select)
                .placeholder("Encoder preset")
                .menu_width(px(220.0))
                .into_any_element()
        } else {
            div()
                .text_xs()
                .text_color(white().opacity(0.5))
                .child("Encoder preset")
                .into_any_element()
        };

    let audio_bitrate_select_elem =
        if let Some(select) = panel.export_modal.audio_bitrate_select.as_ref() {
            Select::new(select)
                .placeholder("Audio bitrate")
                .menu_width(px(200.0))
                .into_any_element()
        } else {
            div()
                .text_xs()
                .text_color(white().opacity(0.5))
                .child("Audio bitrate")
                .into_any_element()
        };

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
                this.export_modal.close();
                cx.notify();
            }),
        )
        .child(
            div()
                .w(px(640.0))
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
                        .child("Export"),
                )
                .child(dependency_warning)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(white().opacity(0.5))
                                .child(dir_label),
                        )
                        .child(modal_btn("Choose Folder").on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |_this, _, win, cx| {
                                let rx = cx.prompt_for_paths(PathPromptOptions {
                                    files: false,
                                    directories: true,
                                    multiple: false,
                                    prompt: Some("Choose Folder".into()),
                                });

                                cx.spawn_in(win, async move |view, window| {
                                    let Ok(result) = rx.await else {
                                        return;
                                    };
                                    let paths = match result {
                                        Ok(Some(paths)) => paths,
                                        Ok(None) => return,
                                        Err(err) => {
                                            eprintln!("[Export] Folder picker error: {err}");
                                            return;
                                        }
                                    };

                                    let Some(path) = paths.into_iter().next() else {
                                        return;
                                    };
                                    let _ = view.update_in(window, |this, _window, cx| {
                                        this.export_modal.dir = Some(path);
                                        this.export_modal.overwrite_path = None;
                                        cx.notify();
                                    });
                                })
                                .detach();
                            }),
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(white().opacity(0.5))
                                .child("Export mode"),
                        )
                        .child(mode_select_elem)
                        .child(
                            div()
                                .text_xs()
                                .text_color(white().opacity(0.45))
                                .child(selected_mode.description()),
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
                                .text_color(white().opacity(0.5))
                                .child("File name"),
                        )
                        .child(input_elem),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(white().opacity(0.5))
                                .child("Output preset"),
                        )
                        .child(preset_select_elem)
                        .child(
                            div()
                                .text_xs()
                                .text_color(white().opacity(0.45))
                                .child(selected_preset.description()),
                        ),
                )
                .child(if is_audio_only {
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.45))
                        .child("Audio-only preset: video settings are ignored.")
                        .into_any_element()
                } else {
                    div().into_any_element()
                })
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .items_end()
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(white().opacity(0.5))
                                        .child("Target FPS"),
                                )
                                .child(fps_select_elem),
                        )
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(white().opacity(0.5))
                                        .child("Resolution"),
                                )
                                .child(resolution_select_elem),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .items_end()
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(white().opacity(0.5))
                                        .child("Encoder preset"),
                                )
                                .child(encoder_preset_select_elem),
                        )
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(white().opacity(0.5))
                                        .child("Audio bitrate"),
                                )
                                .child(audio_bitrate_select_elem),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(white().opacity(0.5))
                                .child("Export range"),
                        )
                        .child(
                            modal_btn(if panel.export_modal.use_custom_range {
                                "Custom"
                            } else {
                                "Full Timeline"
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.export_modal.use_custom_range =
                                        !this.export_modal.use_custom_range;
                                    this.export_modal.range_error = None;
                                    cx.notify();
                                }),
                            ),
                        ),
                )
                .child(if panel.export_modal.use_custom_range {
                    div()
                        .flex()
                        .gap_3()
                        .items_end()
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(white().opacity(0.5))
                                        .child("Start (sec)"),
                                )
                                .child(range_start_elem),
                        )
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(white().opacity(0.5))
                                        .child("End (sec, blank = end)"),
                                )
                                .child(range_end_elem),
                        )
                        .into_any_element()
                } else {
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.45))
                        .child("Full timeline will be exported.")
                        .into_any_element()
                })
                .child(range_error)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(white().opacity(0.5))
                                .child("CRF"),
                        )
                        .child(if crf_supported {
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(modal_btn("-").on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.export_modal.settings.crf =
                                            this.export_modal.settings.crf.saturating_sub(1);
                                        cx.notify();
                                    }),
                                ))
                                .child(
                                    div()
                                        .min_w(px(52.0))
                                        .h(px(28.0))
                                        .rounded_lg()
                                        .border_1()
                                        .border_color(white().opacity(0.12))
                                        .bg(white().opacity(0.04))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_xs()
                                        .text_color(white().opacity(0.85))
                                        .child(panel.export_modal.settings.crf.to_string()),
                                )
                                .child(modal_btn("+").on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.export_modal.settings.crf = this
                                            .export_modal
                                            .settings
                                            .crf
                                            .saturating_add(1)
                                            .min(51);
                                        cx.notify();
                                    }),
                                ))
                        } else {
                            div()
                                .text_xs()
                                .text_color(white().opacity(0.45))
                                .child("N/A for this preset")
                        }),
                )
                .child(overwrite_warning)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(white().opacity(0.5))
                                .child("Color mode"),
                        )
                        .child({
                            let global_for_export_mode = panel.global.clone();
                            modal_btn(export_color_mode.label()).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |_this, _, _, cx| {
                                    global_for_export_mode.update(cx, |gs, cx| {
                                        gs.cycle_export_color_mode();
                                        cx.notify();
                                    });
                                }),
                            )
                        }),
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
                                this.export_modal.close();
                                cx.notify();
                            }),
                        ))
                        .child({
                            let global_for_export_run = panel.global.clone();
                            modal_btn(export_btn_label).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, win, cx| {
                                    let filename = this.export_modal.output_filename();
                                    let dir = this
                                        .export_modal
                                        .dir
                                        .clone()
                                        .unwrap_or_else(TimelinePanel::default_export_dir);
                                    let out_path = dir.join(filename).to_string_lossy().to_string();

                                    if Path::new(&out_path).exists()
                                        && this.export_modal.overwrite_path.as_deref()
                                            != Some(&out_path)
                                    {
                                        this.export_modal.overwrite_path = Some(out_path);
                                        cx.notify();
                                        return;
                                    }

                                    let media_tools_ready = {
                                        let gs = global_for_export_run.read(cx);
                                        gs.media_tools_ready_for_export()
                                    };
                                    if !media_tools_ready {
                                        global_for_export_run.update(cx, |gs, cx| {
                                            gs.ui_notice = Some(
                                                "Export requires FFmpeg and FFprobe. Install tools first."
                                                    .to_string(),
                                            );
                                            gs.show_media_dependency_modal();
                                            cx.notify();
                                        });
                                        return;
                                    }

                                    let preset = this.export_modal.preset;
                                    let export_mode = this.export_modal.mode;
                                    let export_settings = this.export_modal.settings.clone();

                                    let (
                                        ffmpeg_path,
                                        v1,
                                        audio_tracks,
                                        video_tracks,
                                        subtitle_tracks,
                                        subtitle_groups,
                                        canvas_w,
                                        canvas_h,
                                        layer_effects,
                                        layer_effect_clips,
                                        export_color_mode,
                                        timeline_total,
                                    ) = {
                                        let gs = global_for_export_run.read(cx);
                                        if gs.export_in_progress {
                                            return;
                                        }
                                        (
                                            gs.ffmpeg_path.clone(),
                                            gs.v1_clips.clone(),
                                            gs.audio_tracks.clone(),
                                            gs.video_tracks.clone(),
                                            gs.subtitle_tracks.clone(),
                                            gs.subtitle_groups.clone(),
                                            gs.canvas_w,
                                            gs.canvas_h,
                                            gs.layer_color_blur_effects(),
                                            gs.layer_effect_clips().to_vec(),
                                            gs.export_color_mode,
                                            gs.timeline_total(),
                                        )
                                    };

                                    let (export_w, export_h) =
                                        this.export_modal.output_resolution(canvas_w, canvas_h);
                                    let export_range =
                                        match this.export_modal.output_range(timeline_total) {
                                            Ok(v) => v,
                                            Err(err) => {
                                                this.export_modal.range_error = Some(err);
                                                cx.notify();
                                                return;
                                            }
                                        };
                                    let export_total = export_range
                                        .map(|r| r.duration())
                                        .unwrap_or(timeline_total)
                                        .max(Duration::from_millis(1));
                                    let cancel_signal = Arc::new(AtomicBool::new(false));

                                    this.export_modal.close();
                                    this.export_modal.cancel_signal = Some(cancel_signal.clone());
                                    cx.notify();

                                    global_for_export_run.update(cx, |gs, cx| {
                                        gs.export_begin(out_path.clone(), export_total);
                                        cx.notify();
                                    });

                                    let (tx, rx) = mpsc::channel::<ExportWorkerEvent>();
                                    let out_path_for_thread = out_path.clone();
                                    std::thread::spawn(move || {
                                        let tx_progress = tx.clone();
                                        let result = FfmpegExporter::export(
                                            &ffmpeg_path,
                                            &v1,
                                            &audio_tracks,
                                            &video_tracks,
                                            &subtitle_tracks,
                                            &subtitle_groups,
                                            &out_path_for_thread,
                                            canvas_w,
                                            canvas_h,
                                            export_w,
                                            export_h,
                                            layer_effects,
                                            &layer_effect_clips,
                                            export_color_mode,
                                            export_mode,
                                            preset,
                                            export_settings,
                                            export_range,
                                            cancel_signal,
                                            move |progress| {
                                                let _ = tx_progress
                                                    .send(ExportWorkerEvent::Progress(progress));
                                            },
                                        );
                                        let result = match result {
                                            Ok(_) => Ok(out_path_for_thread),
                                            Err(err) => {
                                                eprintln!("[Export] Failed: {err}");
                                                Err(err.to_string())
                                            }
                                        };
                                        let _ = tx.send(ExportWorkerEvent::Finished(result));
                                    });

                                    let global_finish = global_for_export_run.clone();
                                    cx.spawn_in(win, async move |view, window| {
                                        loop {
                                            gpui::Timer::after(Duration::from_millis(120)).await;

                                            let mut latest_progress = None;
                                            let mut finished = None;
                                            loop {
                                                match rx.try_recv() {
                                                    Ok(ExportWorkerEvent::Progress(p)) => {
                                                        latest_progress = Some(p);
                                                    }
                                                    Ok(ExportWorkerEvent::Finished(result)) => {
                                                        finished = Some(result);
                                                        break;
                                                    }
                                                    Err(mpsc::TryRecvError::Empty) => break,
                                                    Err(mpsc::TryRecvError::Disconnected) => {
                                                        finished = Some(Err(
                                                            "Export worker disconnected."
                                                                .to_string(),
                                                        ));
                                                        break;
                                                    }
                                                }
                                            }

                                            let has_finished = finished.is_some();
                                            let updated =
                                                view.update_in(window, |this, _win, cx| {
                                                    global_finish.update(cx, |gs, cx| {
                                                        if let Some(p) = latest_progress {
                                                            gs.export_update_progress(
                                                                p.rendered, p.total, p.speed,
                                                            );
                                                        }

                                                        if let Some(result) = finished {
                                                            match result {
                                                                Ok(path) => {
                                                                    gs.export_done();
                                                                    gs.export_last_out_path =
                                                                        Some(path.clone());
                                                                    gs.ui_notice = Some(format!(
                                                                        "Export saved: {}",
                                                                        path
                                                                    ));
                                                                }
                                                                Err(err) => {
                                                                    if is_cancelled_export_error(
                                                                        &err,
                                                                    ) {
                                                                        gs.export_cancelled();
                                                                        gs.ui_notice = Some(
                                                                            "Export stopped."
                                                                                .to_string(),
                                                                        );
                                                                    } else {
                                                                        gs.export_fail(err);
                                                                    }
                                                                }
                                                            }
                                                            this.export_modal.cancel_signal = None;
                                                        }

                                                        cx.notify();
                                                    });
                                                });

                                            if has_finished || updated.is_err() {
                                                break;
                                            }
                                        }
                                    })
                                    .detach();
                                }),
                            )
                        }),
                ),
        )
}
