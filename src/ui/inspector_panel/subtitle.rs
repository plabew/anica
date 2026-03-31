use super::*;

pub(super) struct SubtitleRenderState {
    pub has_subtitle_group: bool,
    pub has_multi_subtitle_selection: bool,
    pub use_group_transform: bool,
    pub sub_pos_x_display: f32,
    pub sub_pos_y_display: f32,
    pub subtitle_color_display: (u8, u8, u8, u8),
    pub sub_color_h: f32,
    pub sub_color_s: f32,
    pub sub_color_l: f32,
    pub sub_color_a: f32,
}

impl InspectorPanel {
    pub(super) fn apply_subtitle_slider_value(
        gs: &mut GlobalState,
        key: &str,
        clamped: f32,
    ) -> bool {
        match key {
            "sub_pos_x" => gs.set_selected_subtitle_pos_x(clamped),
            "sub_pos_y" => gs.set_selected_subtitle_pos_y(clamped),
            "sub_size" => gs.set_selected_subtitle_font_size(clamped),
            "sub_group_scale" => gs.set_selected_subtitle_group_scale(clamped),
            // Subtitle color channels are handled by HSLA slider callbacks.
            "sub_color_hue" | "sub_color_sat" | "sub_color_lum" | "sub_color_alpha" => {}
            _ => return false,
        }
        true
    }

    pub(super) fn prepare_subtitle_render_state(
        &mut self,
        subtitle_group_id: Option<u64>,
        selected_subtitle_ids: &[u64],
        subtitle_transform_val: (f32, f32, f32),
        subtitle_group_transform_val: (f32, f32, f32),
        selected_subtitle_color_rgba: (u8, u8, u8, u8),
        selected_subtitle_group_color_rgba: (u8, u8, u8, u8),
    ) -> SubtitleRenderState {
        let (sub_pos_x_val, sub_pos_y_val, _) = subtitle_transform_val;
        let (group_pos_x_val, group_pos_y_val, _) = subtitle_group_transform_val;

        let has_subtitle_group = subtitle_group_id.is_some();
        let has_multi_subtitle_selection = selected_subtitle_ids.len() >= 2;
        if self.subtitle_edit_mode == SubtitleEditMode::Group && !has_subtitle_group {
            self.subtitle_edit_mode = SubtitleEditMode::Individual;
        }
        let use_group_transform =
            self.subtitle_edit_mode == SubtitleEditMode::Group && has_subtitle_group;
        let sub_pos_x_display = if use_group_transform {
            group_pos_x_val
        } else {
            sub_pos_x_val
        };
        let sub_pos_y_display = if use_group_transform {
            group_pos_y_val
        } else {
            sub_pos_y_val
        };
        let subtitle_color_display = if use_group_transform {
            selected_subtitle_group_color_rgba
        } else {
            selected_subtitle_color_rgba
        };
        let (sub_color_h, sub_color_s, sub_color_l, sub_color_a) =
            Self::rgba_to_hsla(subtitle_color_display);

        SubtitleRenderState {
            has_subtitle_group,
            has_multi_subtitle_selection,
            use_group_transform,
            sub_pos_x_display,
            sub_pos_y_display,
            subtitle_color_display,
            sub_color_h,
            sub_color_s,
            sub_color_l,
            sub_color_a,
        }
    }

    pub(super) fn sync_subtitle_inputs_with_selection(
        &mut self,
        selected_subtitle_id: Option<u64>,
        selected_subtitle_text: &Option<String>,
        selected_subtitle_font: &Option<(String, String)>,
        subtitle_color_display: (u8, u8, u8, u8),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = selected_subtitle_id {
            if self.subtitle_editing_id != Some(id) {
                self.subtitle_editing_id = Some(id);
                if let Some(input) = self.subtitle_input.as_ref() {
                    let text = selected_subtitle_text.clone().unwrap_or_default();
                    input.update(cx, |input, cx| {
                        input.set_value(text, window, cx);
                    });
                }
            } else if let (Some(input), Some(text)) = (
                self.subtitle_input.as_ref(),
                selected_subtitle_text.as_ref(),
            ) {
                let focused = input.read(cx).focus_handle(cx).is_focused(window);
                if !focused {
                    let current = input.read(cx).value();
                    if current.as_ref() != text {
                        let text = text.clone();
                        input.update(cx, |input, cx| {
                            input.set_value(text, window, cx);
                        });
                    }
                }
            }
        } else if self.subtitle_editing_id.is_some() {
            self.subtitle_editing_id = None;
            if let Some(input) = self.subtitle_input.as_ref() {
                input.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                });
            }
        }

        if let Some(select) = self.subtitle_font_select.as_ref() {
            let desired = selected_subtitle_font
                .as_ref()
                .map(|v| v.0.clone())
                .unwrap_or_default();
            let current = select
                .read(cx)
                .selected_value()
                .cloned()
                .unwrap_or_default();
            if current != desired {
                select.update(cx, |state, cx| {
                    state.set_selected_value(&desired, window, cx);
                });
            }
        }

        if let Some(input) = self.subtitle_color_hex_input.as_ref() {
            let desired = Self::rgba_to_hex(subtitle_color_display);
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            if !focused {
                let current = input.read(cx).value().to_string();
                if current != desired {
                    input.update(cx, |input, cx| {
                        input.set_value(desired.clone(), window, cx);
                    });
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_subtitle_editor_panel(
        &self,
        has_subtitle_selection: bool,
        subtitle_state: &SubtitleRenderState,
        sub_px_ent: &Entity<SliderState>,
        sub_py_ent: &Entity<SliderState>,
        sub_group_sz_ent: &Entity<SliderState>,
        sub_sz_ent: &Entity<SliderState>,
        sub_color_hue_ent: &Entity<SliderState>,
        sub_color_sat_ent: &Entity<SliderState>,
        sub_color_lum_ent: &Entity<SliderState>,
        sub_color_alpha_ent: &Entity<SliderState>,
        ev_sub_pos_x: gpui::AnyElement,
        ev_sub_pos_y: gpui::AnyElement,
        ev_sub_group_scale: gpui::AnyElement,
        ev_sub_size: gpui::AnyElement,
        ev_sub_color_hue: gpui::AnyElement,
        ev_sub_color_sat: gpui::AnyElement,
        ev_sub_color_lum: gpui::AnyElement,
        ev_sub_color_alpha: gpui::AnyElement,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !has_subtitle_selection {
            return None;
        }
        let is_group_mode = self.subtitle_edit_mode == SubtitleEditMode::Group;
        let subtitle_swatch_color = gpui::hsla(
            subtitle_state.sub_color_h / 360.0,
            subtitle_state.sub_color_s,
            subtitle_state.sub_color_l,
            subtitle_state.sub_color_a,
        );
        let group_button = {
            let mut base = div()
                .h(px(24.0))
                .px_2()
                .rounded_sm()
                .bg(white().opacity(0.06))
                .text_color(
                    white().opacity(if subtitle_state.has_multi_subtitle_selection {
                        0.85
                    } else {
                        0.35
                    }),
                )
                .text_xs()
                .child("Group");
            if subtitle_state.has_multi_subtitle_selection {
                base = base
                    .hover(|s| s.bg(white().opacity(0.12)))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            let mut new_group = None;
                            this.global.update(cx, |gs, cx| {
                                new_group = gs.group_selected_subtitles();
                                if let Some(group_id) = new_group {
                                    gs.select_subtitle_group(group_id);
                                }
                                cx.notify();
                            });
                            if new_group.is_some() {
                                this.subtitle_edit_mode = SubtitleEditMode::Group;
                            }
                            cx.notify();
                        }),
                    );
            }
            base
        };
        let individual_chip = div()
            .h(px(24.0))
            .px_2()
            .rounded_sm()
            .bg(white().opacity(if !is_group_mode { 0.18 } else { 0.06 }))
            .text_color(white().opacity(if !is_group_mode { 0.95 } else { 0.6 }))
            .text_xs()
            .child("Individual")
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.subtitle_edit_mode = SubtitleEditMode::Individual;
                    this.global.update(cx, |gs, cx| {
                        let selected = gs.selected_subtitle_id;
                        gs.selected_subtitle_ids.clear();
                        if let Some(id) = selected {
                            gs.selected_subtitle_ids.push(id);
                        }
                        cx.notify();
                    });
                    cx.notify();
                }),
            );
        let mut group_chip = div()
            .h(px(24.0))
            .px_2()
            .rounded_sm()
            .bg(white().opacity(if is_group_mode { 0.18 } else { 0.06 }))
            .text_color(white().opacity(if is_group_mode { 0.95 } else { 0.6 }))
            .text_xs()
            .child("Global");
        if subtitle_state.has_subtitle_group {
            group_chip = group_chip.cursor_pointer().on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.subtitle_edit_mode = SubtitleEditMode::Group;
                    this.global.update(cx, |gs, cx| {
                        if let Some(group_id) = gs.get_selected_subtitle_group_id() {
                            gs.select_subtitle_group(group_id);
                        }
                        cx.notify();
                    });
                    cx.notify();
                }),
            );
        } else {
            group_chip = group_chip.text_color(white().opacity(0.35));
        }

        Some(
            div()
                .border_1()
                .border_color(white().opacity(0.1))
                .rounded_md()
                .p_2()
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
                                .text_xs()
                                .text_color(white().opacity(0.5))
                                .child("SUBTITLE"),
                        )
                        .child(group_button),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(individual_chip)
                        .child(group_chip),
                )
                .child(if let Some(input) = self.subtitle_input.as_ref() {
                    div()
                        .min_h(px(72.0))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|_, _, _, cx| {
                                cx.stop_propagation();
                            }),
                        )
                        .child(Input::new(input).h(px(72.0)))
                } else {
                    div()
                        .min_h(px(72.0))
                        .p_2()
                        .rounded_sm()
                        .bg(white().opacity(0.06))
                        .text_color(white().opacity(0.9))
                        .child("Type subtitle…")
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(Self::slider_row("Position X", sub_px_ent, ev_sub_pos_x))
                        .child(Self::slider_row("Position Y", sub_py_ent, ev_sub_pos_y))
                        .child(if subtitle_state.use_group_transform {
                            Self::slider_row("Scale", sub_group_sz_ent, ev_sub_group_scale)
                                .into_any_element()
                        } else {
                            Self::slider_row("Size", sub_sz_ent, ev_sub_size).into_any_element()
                        }),
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
                                .child("FONT"),
                        )
                        .child(if let Some(select) = self.subtitle_font_select.as_ref() {
                            Select::new(select)
                                .placeholder("Default")
                                .menu_width(px(220.0))
                                .into_any_element()
                        } else {
                            div()
                                .h(px(28.0))
                                .rounded_sm()
                                .bg(white().opacity(0.06))
                                .text_color(white().opacity(0.6))
                                .px_2()
                                .child("Default")
                                .into_any_element()
                        })
                        .child(
                            div()
                                .pt_1()
                                .text_xs()
                                .text_color(white().opacity(0.5))
                                .child("FONT COLOR"),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .w_5()
                                        .h_5()
                                        .rounded_sm()
                                        .bg(subtitle_swatch_color)
                                        .border_1()
                                        .border_color(white().opacity(0.8)),
                                )
                                .child(
                                    if let Some(input) = self.subtitle_color_hex_input.as_ref() {
                                        div()
                                            .flex_1()
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(|_, _, _, cx| {
                                                    cx.stop_propagation();
                                                }),
                                            )
                                            .child(Input::new(input).h(px(28.0)).w_full())
                                            .into_any_element()
                                    } else {
                                        div()
                                            .h(px(28.0))
                                            .rounded_sm()
                                            .bg(white().opacity(0.06))
                                            .text_color(white().opacity(0.6))
                                            .px_2()
                                            .child("#FFFFFF")
                                            .into_any_element()
                                    },
                                ),
                        )
                        .child(Self::slider_row("Hue", sub_color_hue_ent, ev_sub_color_hue))
                        .child(Self::slider_row("Sat", sub_color_sat_ent, ev_sub_color_sat))
                        .child(Self::slider_row("Lum", sub_color_lum_ent, ev_sub_color_lum))
                        .child(Self::slider_row(
                            "Alpha",
                            sub_color_alpha_ent,
                            ev_sub_color_alpha,
                        )),
                )
                .into_any_element(),
        )
    }

    pub(super) fn load_subtitle_fonts() -> Vec<SubtitleFont> {
        let mut fonts = Vec::new();
        let mut seen = HashSet::new();
        let font_dir = PathBuf::from("assets/fonts");
        if !font_dir.exists() {
            return fonts;
        }

        let mut db = fontdb::Database::new();
        let entries = match fs::read_dir(&font_dir) {
            Ok(entries) => entries,
            Err(_) => return fonts,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            let ext = ext.to_ascii_lowercase();
            if ext != "otf" && ext != "ttf" && ext != "ttc" {
                continue;
            }
            let before = db.faces().count();
            let _ = db.load_font_file(&path);
            let faces: Vec<_> = db.faces().skip(before).collect();
            for face in faces {
                let Some((family, _)) = face.families.first() else {
                    continue;
                };
                let key = format!("{}:{}", path.display(), family);
                if !seen.insert(key) {
                    continue;
                }
                let label = family.clone();
                fonts.push(SubtitleFont {
                    label,
                    family: family.clone(),
                    path: path.to_string_lossy().to_string(),
                });
            }
        }

        fonts.sort_by(|a, b| a.label.cmp(&b.label));
        fonts
    }

    pub(super) fn build_font_items(fonts: &[SubtitleFont]) -> SearchableVec<SubtitleFont> {
        let mut items = Vec::with_capacity(fonts.len() + 1);
        items.push(SubtitleFont {
            label: "Default".to_string(),
            family: String::new(),
            path: String::new(),
        });
        items.extend(fonts.iter().cloned());
        SearchableVec::new(items)
    }

    pub(super) fn rgba_to_hsla((r, g, b, a): (u8, u8, u8, u8)) -> (f32, f32, f32, f32) {
        let rf = r as f32 / 255.0;
        let gf = g as f32 / 255.0;
        let bf = b as f32 / 255.0;
        let max = rf.max(gf).max(bf);
        let min = rf.min(gf).min(bf);
        let delta = max - min;
        let light = (max + min) * 0.5;

        let mut hue = 0.0;
        let sat = if delta <= f32::EPSILON {
            0.0
        } else {
            let denom = 1.0 - (2.0 * light - 1.0).abs();
            if denom <= f32::EPSILON {
                0.0
            } else {
                if (max - rf).abs() <= f32::EPSILON {
                    hue = 60.0 * (((gf - bf) / delta) % 6.0);
                } else if (max - gf).abs() <= f32::EPSILON {
                    hue = 60.0 * (((bf - rf) / delta) + 2.0);
                } else {
                    hue = 60.0 * (((rf - gf) / delta) + 4.0);
                }
                delta / denom
            }
        };
        if hue < 0.0 {
            hue += 360.0;
        }
        (
            hue,
            sat.clamp(0.0, 1.0),
            light.clamp(0.0, 1.0),
            a as f32 / 255.0,
        )
    }

    pub(super) fn hsla_to_rgba(h: f32, s: f32, l: f32, a: f32) -> (u8, u8, u8, u8) {
        let hue = h.rem_euclid(360.0);
        let sat = s.clamp(0.0, 1.0);
        let lum = l.clamp(0.0, 1.0);
        let alpha = a.clamp(0.0, 1.0);

        let c = (1.0 - (2.0 * lum - 1.0).abs()) * sat;
        let x = c * (1.0 - (((hue / 60.0) % 2.0) - 1.0).abs());
        let m = lum - c * 0.5;

        let (r1, g1, b1) = if hue < 60.0 {
            (c, x, 0.0)
        } else if hue < 120.0 {
            (x, c, 0.0)
        } else if hue < 180.0 {
            (0.0, c, x)
        } else if hue < 240.0 {
            (0.0, x, c)
        } else if hue < 300.0 {
            (x, 0.0, c)
        } else {
            (c, 0.0, x)
        };

        let r = ((r1 + m).clamp(0.0, 1.0) * 255.0).round() as u8;
        let g = ((g1 + m).clamp(0.0, 1.0) * 255.0).round() as u8;
        let b = ((b1 + m).clamp(0.0, 1.0) * 255.0).round() as u8;
        let a = (alpha * 255.0).round() as u8;
        (r, g, b, a)
    }

    pub(super) fn rgba_to_hex((r, g, b, a): (u8, u8, u8, u8)) -> String {
        if a == 255 {
            format!("#{:02X}{:02X}{:02X}", r, g, b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", r, g, b, a)
        }
    }

    pub(super) fn parse_hex_rgba(raw: &str) -> Option<(u8, u8, u8, u8)> {
        let s = raw.trim().trim_start_matches('#');
        if s.len() != 6 && s.len() != 8 {
            return None;
        }
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        let a = if s.len() == 8 {
            u8::from_str_radix(&s[6..8], 16).ok()?
        } else {
            255
        };
        Some((r, g, b, a))
    }

    // ✅ [New Helper] Generic horizontal slider row (for transform / effects)
    // Parameters: label, slider entity, value string
    // Generic horizontal slider row — accepts any element for the value display
    pub(super) fn slider_row(
        label: &str,
        slider: &Entity<SliderState>,
        val_el: impl IntoElement,
    ) -> gpui::Div {
        div()
            .flex()
            .items_center()
            .justify_between()
            .h_8()
            .child(
                div()
                    .w_20()
                    .text_sm()
                    .text_color(white().opacity(0.8))
                    .child(label.to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .mx_2()
                    .child(Slider::new(slider).horizontal().h(px(20.)).w_full()),
            )
            .child(val_el)
    }

    pub(super) fn layer_fx_script_editor_wrap(
        title: &str,
        editor: gpui::AnyElement,
        controls: gpui::AnyElement,
        status: String,
    ) -> gpui::Div {
        div()
            .border_1()
            .border_color(white().opacity(0.12))
            .rounded_sm()
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.72))
                    .child(title.to_string()),
            )
            .child(editor)
            .child(controls)
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.6))
                    .child(status),
            )
    }
}
