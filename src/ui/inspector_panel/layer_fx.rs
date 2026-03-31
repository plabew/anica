use super::*;

impl InspectorPanel {
    pub(super) fn layer_fx_curve_lanes_wrap(&self, cx: &mut Context<Self>) -> gpui::Div {
        let mut lanes = div().flex().flex_col().gap_3();
        if self.layer_fx_curve_editors.is_empty() {
            lanes = lanes.child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.55))
                    .child("Apply Script to generate curve lanes from Pass list."),
            );
        } else {
            for row_idx in 0..self.layer_fx_curve_editors.len() {
                let row = &self.layer_fx_curve_editors[row_idx];
                let effect = if row.effect_name.trim().is_empty() {
                    "unknown_effect".to_string()
                } else {
                    row.effect_name.clone()
                };
                let param_label = row.param_label.clone();
                let value_min = row.value_min;
                let value_max = row.value_max;
                let value_span = (value_max - value_min).max(0.000_001);
                let duration_sec = row.duration_sec.max(0.01);
                let points = row.points.clone();
                let selected_idx = row.selected_point.min(points.len().saturating_sub(1));

                let mut graph = div()
                    .relative()
                    .w(px(CURVE_GRAPH_W))
                    .h(px(CURVE_GRAPH_H))
                    .rounded_sm()
                    .border_1()
                    .border_color(white().opacity(0.14))
                    .bg(rgb(0x0b1020))
                    .overflow_hidden()
                    .on_mouse_move(cx.listener(
                        move |this, evt: &gpui::MouseMoveEvent, window, cx| {
                            this.update_curve_drag(row_idx, evt, window, cx);
                            cx.notify();
                        },
                    ))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _: &gpui::MouseUpEvent, _, cx| {
                            this.end_curve_drag_for_row(row_idx);
                            cx.notify();
                        }),
                    );

                for i in 0..=4 {
                    let y = (i as f32 / 4.0) * CURVE_GRAPH_H;
                    graph = graph.child(
                        div()
                            .absolute()
                            .left(px(0.0))
                            .right(px(0.0))
                            .top(px(y))
                            .h(px(1.0))
                            .bg(white().opacity(0.07)),
                    );
                }

                let sample_count = 96usize;
                for i in 0..sample_count {
                    let u = if sample_count <= 1 {
                        0.0
                    } else {
                        i as f32 / (sample_count - 1) as f32
                    };
                    let t = u * duration_sec;
                    let v_raw = Self::sample_curve_value(&points, t);
                    let v = ((v_raw - value_min) / value_span).clamp(0.0, 1.0);
                    let x = u * CURVE_GRAPH_W;
                    let y = (1.0 - v) * CURVE_GRAPH_H;
                    graph = graph.child(
                        div()
                            .absolute()
                            .left(px((x - 1.0).max(0.0)))
                            .top(px((y - 1.0).max(0.0)))
                            .w(px(2.0))
                            .h(px(2.0))
                            .rounded_sm()
                            .bg(rgb(0x60a5fa)),
                    );
                }

                for (point_idx, point) in points.iter().enumerate() {
                    let (x, y) =
                        Self::curve_point_to_canvas(*point, duration_sec, value_min, value_max);
                    let active = point_idx == selected_idx;
                    graph = graph.child(
                        div()
                            .absolute()
                            .left(px((x - 4.0).max(0.0)))
                            .top(px((y - 4.0).max(0.0)))
                            .w(px(8.0))
                            .h(px(8.0))
                            .rounded_full()
                            .border_1()
                            .border_color(if active {
                                rgba(0x93c5fdff)
                            } else {
                                rgba(0xffffff73)
                            })
                            .bg(if active {
                                rgba(0x2563ebff)
                            } else {
                                rgba(0xffffffa6)
                            })
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, evt: &gpui::MouseDownEvent, _, cx| {
                                    this.start_curve_drag(row_idx, point_idx, evt);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            ),
                    );
                }

                let mut segment_controls = div().flex().flex_col().gap_1();
                for seg_idx in 0..points.len().saturating_sub(1) {
                    let active_ease = points[seg_idx].ease;
                    let menu_open = self.layer_fx_curve_open_menu == Some((row_idx, seg_idx));
                    segment_controls = segment_controls.child(
                        div()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(white().opacity(0.6))
                                            .child(format!("Seg {}", seg_idx + 1)),
                                    )
                                    .child(
                                        div()
                                            .h(px(22.0))
                                            .px_2()
                                            .rounded_sm()
                                            .border_1()
                                            .border_color(white().opacity(0.22))
                                            .bg(white().opacity(0.08))
                                            .text_xs()
                                            .text_color(white().opacity(0.9))
                                            .cursor_pointer()
                                            .child(format!(
                                                "{} ▼",
                                                Self::curve_ease_label(active_ease)
                                            ))
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(move |this, _, _, cx| {
                                                    this.layer_fx_curve_open_menu = if this
                                                        .layer_fx_curve_open_menu
                                                        == Some((row_idx, seg_idx))
                                                    {
                                                        None
                                                    } else {
                                                        Some((row_idx, seg_idx))
                                                    };
                                                    cx.notify();
                                                }),
                                            ),
                                    ),
                            )
                            .when(menu_open, |panel| {
                                let option_btn = |label: &str, ease: LayerFxCurveEase| {
                                    div()
                                        .h(px(22.0))
                                        .px_2()
                                        .rounded_sm()
                                        .border_1()
                                        .border_color(white().opacity(0.16))
                                        .bg(white().opacity(0.05))
                                        .text_xs()
                                        .text_color(white().opacity(0.88))
                                        .cursor_pointer()
                                        .child(label.to_string())
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |this, _, window, cx| {
                                                this.set_curve_segment_ease(
                                                    row_idx, seg_idx, ease, window, cx,
                                                );
                                                cx.notify();
                                            }),
                                        )
                                };
                                panel.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .items_start()
                                        .gap_1()
                                        .child(option_btn("Linear", LayerFxCurveEase::Linear))
                                        .child(option_btn("Ease In", LayerFxCurveEase::EaseIn))
                                        .child(option_btn("Ease Out", LayerFxCurveEase::EaseOut))
                                        .child(option_btn(
                                            "Ease In Out",
                                            LayerFxCurveEase::EaseInOut,
                                        )),
                                )
                            }),
                    );
                }

                let selected_point_text = points
                    .get(selected_idx)
                    .map(|p| format!("t={:.2}s, {}={:.3}", p.t_sec, row.param_key, p.value))
                    .unwrap_or_else(|| "n/a".to_string());

                lanes = lanes.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(white().opacity(0.75))
                                .child(format!(
                                    "{effect}  (pass: {}, param: {})",
                                    row.pass_id, param_label
                                )),
                        )
                        .child(
                            div()
                                .w_full()
                                .max_w(px(CURVE_GRAPH_W))
                                .overflow_hidden()
                                .child(graph),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .h(px(22.0))
                                        .px_2()
                                        .rounded_sm()
                                        .border_1()
                                        .border_color(white().opacity(0.2))
                                        .bg(white().opacity(0.08))
                                        .text_xs()
                                        .text_color(white().opacity(0.9))
                                        .cursor_pointer()
                                        .child("Add Point")
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |this, _, window, cx| {
                                                this.add_curve_point(row_idx, window, cx);
                                                cx.notify();
                                            }),
                                        ),
                                )
                                .child(
                                    div()
                                        .h(px(22.0))
                                        .px_2()
                                        .rounded_sm()
                                        .border_1()
                                        .border_color(white().opacity(0.2))
                                        .bg(white().opacity(0.08))
                                        .text_xs()
                                        .text_color(white().opacity(0.9))
                                        .cursor_pointer()
                                        .child("Delete Point")
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |this, _, window, cx| {
                                                this.remove_selected_curve_point(
                                                    row_idx, window, cx,
                                                );
                                                cx.notify();
                                            }),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(white().opacity(0.62))
                                        .child(format!(
                                            "Selected: {selected_point_text} | range=[{value_min:.3},{value_max:.3}] | duration={duration_sec:.2}s"
                                        )),
                                ),
                        )
                        .child(segment_controls),
                );
            }
        }

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
                    .child("CURVE LANES (curve-enabled params by pass)"),
            )
            .child(lanes)
    }

    pub(super) fn ensure_layer_fx_script_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.layer_fx_script_input.is_some() {
            return;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("tsx")
                .rows(8)
                .line_number(true)
                .soft_wrap(true)
                .placeholder("<Graph ...> layer FX script")
        });
        let initial = self.layer_fx_script_text.clone();
        input.update(cx, |this, cx| {
            this.set_value(initial.clone(), window, cx);
        });
        let sub = cx.subscribe(&input, |this, input, ev, _cx| {
            if matches!(ev, InputEvent::Change | InputEvent::PressEnter { .. }) {
                this.layer_fx_script_text = input.read(_cx).value().to_string();
            }
        });
        self.layer_fx_script_input = Some(input);
        self.layer_fx_script_input_sub = Some(sub);
    }

    pub(super) fn sync_layer_fx_script_from_selected_layer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = {
            let gs = self.global.read(cx);
            gs.selected_layer_effect_clip()
        };
        let Some(layer) = selected else {
            self.layer_fx_script_layer_id = None;
            return;
        };
        if self.layer_fx_script_layer_id == Some(layer.id) {
            return;
        }
        self.layer_fx_script_layer_id = Some(layer.id);
        let next_script = layer.motionloom_script;
        self.layer_fx_script_text = next_script.clone();
        if let Some(input) = self.layer_fx_script_input.as_ref() {
            input.update(cx, |this, cx| {
                this.set_value(next_script.clone(), window, cx);
            });
        }
        self.layer_fx_script_status = format!("Editing Layer FX #{} script.", layer.id);
        self.rebuild_layer_fx_curve_editors(window, cx);
    }

    pub(super) fn pass_param_value(pass: &motionloom::PassNode, key: &str) -> Option<String> {
        pass.params
            .iter()
            .find(|p| p.key.trim().eq_ignore_ascii_case(key))
            .map(|p| p.value.trim().trim_matches('"').trim().to_string())
    }

    pub(super) fn pass_duration_sec(pass: &motionloom::PassNode, default_value: f32) -> f32 {
        Self::pass_param_value(pass, "durationSec")
            .or_else(|| Self::pass_param_value(pass, "duration_sec"))
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(default_value)
    }

    pub(super) fn normalized_effect_name(raw: &str) -> String {
        raw.trim()
            .trim_matches('"')
            .trim()
            .to_ascii_lowercase()
            .replace('-', "_")
    }

    pub(super) fn curve_param_spec_for_pass(pass: &motionloom::PassNode) -> Option<CurveParamSpec> {
        let effect = Self::normalized_effect_name(&pass.effect);
        match effect.as_str() {
            "opacity" | "fade_in" | "fade_out" | "dip" | "dissolve" => Some(CurveParamSpec {
                key: "opacity",
                label: "Opacity",
                value_min: 0.0,
                value_max: 1.0,
                default_value: 1.0,
            }),
            "hsla_overlay" | "hsla" | "tint_overlay" => Some(CurveParamSpec {
                key: "alpha",
                label: "Alpha",
                value_min: 0.0,
                value_max: 1.0,
                default_value: 0.45,
            }),
            "gaussian_blur" | "gaussian_5tap_h" | "gaussian_5tap_v" | "sharpen" | "unsharp"
            | "box" => Some(CurveParamSpec {
                key: "sigma",
                label: "Sigma",
                value_min: 0.0,
                value_max: 64.0,
                default_value: 2.0,
            }),
            _ => None,
        }
    }

    pub(super) fn curve_ease_token(ease: LayerFxCurveEase) -> &'static str {
        match ease {
            LayerFxCurveEase::Linear => "linear",
            LayerFxCurveEase::EaseIn => "ease_in",
            LayerFxCurveEase::EaseOut => "ease_out",
            LayerFxCurveEase::EaseInOut => "ease_in_out",
        }
    }

    pub(super) fn curve_ease_label(ease: LayerFxCurveEase) -> &'static str {
        match ease {
            LayerFxCurveEase::Linear => "Linear",
            LayerFxCurveEase::EaseIn => "Ease In",
            LayerFxCurveEase::EaseOut => "Ease Out",
            LayerFxCurveEase::EaseInOut => "Ease In Out",
        }
    }

    pub(super) fn parse_curve_ease_token(raw: &str) -> LayerFxCurveEase {
        match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "ease_in" => LayerFxCurveEase::EaseIn,
            "ease_out" => LayerFxCurveEase::EaseOut,
            "ease_in_out" => LayerFxCurveEase::EaseInOut,
            _ => LayerFxCurveEase::Linear,
        }
    }

    pub(super) fn apply_curve_ease(ease: LayerFxCurveEase, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match ease {
            LayerFxCurveEase::Linear => t,
            LayerFxCurveEase::EaseIn => t * t,
            LayerFxCurveEase::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            LayerFxCurveEase::EaseInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - ((-2.0 * t + 2.0).powi(2) / 2.0)
                }
            }
        }
    }

    pub(super) fn normalize_curve_points(
        points: &mut Vec<LayerFxCurvePoint>,
        duration_sec: f32,
        value_min: f32,
        value_max: f32,
    ) {
        let duration_sec = duration_sec.max(0.01);
        let value_min = value_min.min(value_max);
        let value_max = value_max.max(value_min + 0.000_001);
        points.retain(|p| p.t_sec.is_finite() && p.value.is_finite());
        for point in points.iter_mut() {
            point.t_sec = point.t_sec.clamp(0.0, duration_sec);
            point.value = point.value.clamp(value_min, value_max);
        }
        points.sort_by(|a, b| a.t_sec.total_cmp(&b.t_sec));
        if points.is_empty() {
            points.push(LayerFxCurvePoint {
                t_sec: 0.0,
                value: value_max,
                ease: LayerFxCurveEase::Linear,
            });
            points.push(LayerFxCurvePoint {
                t_sec: duration_sec,
                value: value_max,
                ease: LayerFxCurveEase::Linear,
            });
        } else if points.len() == 1 {
            let p = points[0];
            points[0].t_sec = 0.0;
            points.push(LayerFxCurvePoint {
                t_sec: duration_sec,
                value: p.value,
                ease: p.ease,
            });
        }
        points[0].t_sec = 0.0;
        if points.len() >= 2 {
            for idx in 1..points.len() {
                let min_t = (points[idx - 1].t_sec + CURVE_TIME_EPS).min(duration_sec);
                points[idx].t_sec = points[idx].t_sec.clamp(min_t, duration_sec);
            }
            for idx in (0..points.len() - 1).rev() {
                let max_t = (points[idx + 1].t_sec - CURVE_TIME_EPS).max(0.0);
                points[idx].t_sec = points[idx].t_sec.min(max_t);
            }
            points[0].t_sec = 0.0;
        }
    }

    pub(super) fn parse_curve_points_expr(
        raw: &str,
        duration_sec: f32,
        value_min: f32,
        value_max: f32,
    ) -> Option<Vec<LayerFxCurvePoint>> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Ok(value) = trimmed.trim_matches('"').trim().parse::<f32>() {
            let mut points = vec![
                LayerFxCurvePoint {
                    t_sec: 0.0,
                    value,
                    ease: LayerFxCurveEase::Linear,
                },
                LayerFxCurvePoint {
                    t_sec: duration_sec.max(0.01),
                    value,
                    ease: LayerFxCurveEase::Linear,
                },
            ];
            Self::normalize_curve_points(&mut points, duration_sec, value_min, value_max);
            return Some(points);
        }

        let mut expr = trimmed.trim_matches('"').trim().to_string();
        expr = expr.replace("\\\"", "\"").replace("\\'", "'");
        if !expr.to_ascii_lowercase().starts_with("curve(") || !expr.ends_with(')') {
            return None;
        }
        let inner = expr
            .trim_start_matches("curve(")
            .trim_end_matches(')')
            .trim()
            .trim_matches('"')
            .trim_matches('\'');
        if inner.is_empty() {
            return None;
        }
        let mut points = Vec::new();
        for token in inner.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            let parts: Vec<&str> = token.split(':').map(str::trim).collect();
            if parts.len() < 2 {
                continue;
            }
            let Ok(t_sec) = parts[0].parse::<f32>() else {
                continue;
            };
            let Ok(value) = parts[1].parse::<f32>() else {
                continue;
            };
            let ease = if parts.len() >= 3 {
                Self::parse_curve_ease_token(parts[2].trim_matches('"'))
            } else {
                LayerFxCurveEase::Linear
            };
            points.push(LayerFxCurvePoint { t_sec, value, ease });
        }
        if points.is_empty() {
            return None;
        }
        Self::normalize_curve_points(&mut points, duration_sec, value_min, value_max);
        Some(points)
    }

    pub(super) fn default_curve_points_for_pass(
        pass: &motionloom::PassNode,
        spec: CurveParamSpec,
        default_duration_sec: f32,
    ) -> Vec<LayerFxCurvePoint> {
        let duration_sec =
            Self::pass_duration_sec(pass, default_duration_sec).clamp(0.01, 99_999.0);
        if let Some(existing) = Self::pass_param_value(pass, spec.key)
            && let Some(points) = Self::parse_curve_points_expr(
                &existing,
                duration_sec,
                spec.value_min,
                spec.value_max,
            )
        {
            return points;
        }
        let effect = Self::normalized_effect_name(&pass.effect);
        let mut points = if spec.key == "opacity" && effect == "fade_in" {
            vec![
                LayerFxCurvePoint {
                    t_sec: 0.0,
                    value: 0.0,
                    ease: LayerFxCurveEase::Linear,
                },
                LayerFxCurvePoint {
                    t_sec: duration_sec,
                    value: 1.0,
                    ease: LayerFxCurveEase::EaseInOut,
                },
            ]
        } else if spec.key == "opacity" && effect == "fade_out" {
            vec![
                LayerFxCurvePoint {
                    t_sec: 0.0,
                    value: 1.0,
                    ease: LayerFxCurveEase::Linear,
                },
                LayerFxCurvePoint {
                    t_sec: duration_sec,
                    value: 0.0,
                    ease: LayerFxCurveEase::EaseInOut,
                },
            ]
        } else {
            vec![
                LayerFxCurvePoint {
                    t_sec: 0.0,
                    value: spec.default_value,
                    ease: LayerFxCurveEase::Linear,
                },
                LayerFxCurvePoint {
                    t_sec: duration_sec,
                    value: spec.default_value,
                    ease: LayerFxCurveEase::Linear,
                },
            ]
        };
        Self::normalize_curve_points(&mut points, duration_sec, spec.value_min, spec.value_max);
        points
    }

    pub(super) fn sample_curve_value(points: &[LayerFxCurvePoint], time_sec: f32) -> f32 {
        if points.is_empty() {
            return 0.0;
        }
        if points.len() == 1 {
            return points[0].value;
        }
        let x = time_sec.max(0.0);
        if x <= points[0].t_sec {
            return points[0].value;
        }
        let last = points.len() - 1;
        if x >= points[last].t_sec {
            return points[last].value;
        }
        for idx in 0..last {
            let a = points[idx];
            let b = points[idx + 1];
            if x >= a.t_sec && x <= b.t_sec {
                let span = (b.t_sec - a.t_sec).max(0.000_001);
                let u = ((x - a.t_sec) / span).clamp(0.0, 1.0);
                let eased = Self::apply_curve_ease(a.ease, u);
                return a.value + (b.value - a.value) * eased;
            }
        }
        points[last].value
    }

    pub(super) fn curve_point_to_canvas(
        point: LayerFxCurvePoint,
        duration_sec: f32,
        value_min: f32,
        value_max: f32,
    ) -> (f32, f32) {
        let duration_sec = duration_sec.max(0.01);
        let span = (value_max - value_min).max(0.000_001);
        let yn = ((point.value - value_min) / span).clamp(0.0, 1.0);
        let x = ((point.t_sec / duration_sec).clamp(0.0, 1.0)) * CURVE_GRAPH_W;
        let y = (1.0 - yn) * CURVE_GRAPH_H;
        (x, y)
    }

    pub(super) fn curve_points_to_expr(points: &[LayerFxCurvePoint]) -> String {
        let body = points
            .iter()
            .map(|p| {
                format!(
                    "{:.2}:{:.3}:{}",
                    p.t_sec.max(0.0),
                    p.value,
                    Self::curve_ease_token(p.ease)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("curve(\"{body}\")")
    }

    pub(super) fn rewrite_pass_curve_param_in_block(
        block: &str,
        param_key: &str,
        curve_expr: &str,
    ) -> String {
        let Some(close_tag_ix) = block.rfind("/>") else {
            return block.to_string();
        };

        if let Some(params_ix) = block.find("params={{") {
            let body_start = params_ix + "params={{".len();
            if let Some(params_end_rel) = block[body_start..].find("}}") {
                let body_end = body_start + params_end_rel;
                let body = &block[body_start..body_end];
                let mut rows: Vec<String> = body.lines().map(|line| line.to_string()).collect();
                let mut replaced = false;
                for row in &mut rows {
                    let leading_ws = row.chars().take_while(|c| c.is_whitespace()).count();
                    let trimmed = row.trim_start();
                    if trimmed.starts_with(&format!("{param_key}:")) {
                        let indent = &row[..leading_ws];
                        *row = format!("{indent}{param_key}: {curve_expr},");
                        replaced = true;
                        break;
                    }
                }
                if !replaced {
                    let indent = rows
                        .iter()
                        .find_map(|row| {
                            let trimmed = row.trim();
                            if trimmed.is_empty() {
                                None
                            } else {
                                Some(
                                    row.chars()
                                        .take_while(|c| c.is_whitespace())
                                        .collect::<String>(),
                                )
                            }
                        })
                        .unwrap_or_else(|| "          ".to_string());
                    rows.push(format!("{indent}{param_key}: {curve_expr},"));
                }
                let mut next_body = rows.join("\n");
                if !next_body.starts_with('\n') {
                    next_body.insert(0, '\n');
                }
                if !next_body.ends_with('\n') {
                    next_body.push('\n');
                }
                return format!(
                    "{}{}{}{}",
                    &block[..body_start],
                    next_body,
                    &block[body_end..close_tag_ix],
                    &block[close_tag_ix..]
                );
            }
        }

        let insertion =
            format!("\n        params={{\n          {param_key}: {curve_expr},\n        }}\n  ");
        format!(
            "{}{}{}",
            &block[..close_tag_ix],
            insertion,
            &block[close_tag_ix..]
        )
    }

    pub(super) fn upsert_pass_curve_param(
        script: &str,
        pass_id: &str,
        param_key: &str,
        curve_expr: &str,
    ) -> Option<String> {
        let id_pattern = format!("id=\"{pass_id}\"");
        let mut cursor = 0usize;
        let mut out = String::new();

        while let Some(start_rel) = script[cursor..].find("<Pass") {
            let start = cursor + start_rel;
            let Some(end_rel) = script[start..].find("/>") else {
                break;
            };
            let end = start + end_rel + 2;
            out.push_str(&script[cursor..start]);
            let block = &script[start..end];
            if block.contains(&id_pattern) {
                out.push_str(&Self::rewrite_pass_curve_param_in_block(
                    block, param_key, curve_expr,
                ));
                out.push_str(&script[end..]);
                return Some(out);
            }
            out.push_str(block);
            cursor = end;
        }
        None
    }

    pub(super) fn rebuild_layer_fx_curve_editors(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.layer_fx_curve_editors.clear();
        self.layer_fx_curve_drag = None;
        self.layer_fx_curve_open_menu = None;
        let script = self.layer_fx_script_text.trim();
        if script.is_empty() || !is_graph_script(script) {
            return;
        }
        let Ok(graph) = parse_graph_script(script) else {
            return;
        };
        let layer_duration_sec = self
            .global
            .read(_cx)
            .selected_layer_effect_clip()
            .map(|clip| clip.duration.as_secs_f32())
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(2.0);
        let graph_default_duration_sec = match graph.apply {
            GraphApplyScope::Clip => layer_duration_sec,
            GraphApplyScope::Graph => {
                if graph.duration_explicit {
                    (graph.duration_ms as f32 / 1000.0).max(0.01)
                } else {
                    layer_duration_sec
                }
            }
        };
        for pass in graph.passes {
            let Some(spec) = Self::curve_param_spec_for_pass(&pass) else {
                continue;
            };
            let duration_sec =
                Self::pass_duration_sec(&pass, graph_default_duration_sec).clamp(0.01, 99_999.0);
            let mut points = Self::default_curve_points_for_pass(&pass, spec, duration_sec);
            Self::normalize_curve_points(&mut points, duration_sec, spec.value_min, spec.value_max);
            self.layer_fx_curve_editors.push(LayerFxCurveEditor {
                pass_id: pass.id,
                effect_name: pass.effect,
                param_key: spec.key.to_string(),
                param_label: spec.label.to_string(),
                value_min: spec.value_min,
                value_max: spec.value_max,
                duration_sec,
                points,
                selected_point: 0,
            });
        }
    }

    pub(super) fn start_curve_drag(
        &mut self,
        row_idx: usize,
        point_idx: usize,
        evt: &gpui::MouseDownEvent,
    ) {
        let Some(row) = self.layer_fx_curve_editors.get_mut(row_idx) else {
            return;
        };
        let Some(point) = row.points.get(point_idx).copied() else {
            return;
        };
        row.selected_point = point_idx;
        self.layer_fx_curve_drag = Some(LayerFxCurveDragState {
            row_idx,
            point_idx,
            start_mouse_x: f32::from(evt.position.x),
            start_mouse_y: f32::from(evt.position.y),
            start_t_sec: point.t_sec,
            start_value: point.value,
        });
    }

    pub(super) fn update_curve_drag(
        &mut self,
        row_idx: usize,
        evt: &gpui::MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.layer_fx_curve_drag else {
            return;
        };
        if drag.row_idx != row_idx || !evt.dragging() {
            return;
        }
        let Some(row) = self.layer_fx_curve_editors.get_mut(drag.row_idx) else {
            return;
        };
        if drag.point_idx >= row.points.len() {
            return;
        }
        let duration_sec = row.duration_sec.max(0.01);
        let value_span = (row.value_max - row.value_min).max(0.000_001);
        let dx = f32::from(evt.position.x) - drag.start_mouse_x;
        let dy = f32::from(evt.position.y) - drag.start_mouse_y;
        let mut t_sec = drag.start_t_sec;
        let value = (drag.start_value - (dy / CURVE_GRAPH_H) * value_span)
            .clamp(row.value_min, row.value_max);
        let is_first = drag.point_idx == 0;
        if !is_first {
            t_sec = drag.start_t_sec + (dx / CURVE_GRAPH_W) * duration_sec;
            let prev_t = row.points[drag.point_idx - 1].t_sec + CURVE_TIME_EPS;
            let next_t = if drag.point_idx + 1 < row.points.len() {
                row.points[drag.point_idx + 1].t_sec - CURVE_TIME_EPS
            } else {
                duration_sec
            };
            t_sec = if next_t > prev_t {
                t_sec.clamp(prev_t, next_t)
            } else {
                prev_t
            };
        } else if is_first {
            t_sec = 0.0;
        }
        row.points[drag.point_idx].t_sec = t_sec.max(0.0);
        row.points[drag.point_idx].value = value;
        Self::normalize_curve_points(&mut row.points, duration_sec, row.value_min, row.value_max);
        self.apply_layer_fx_curves(window, cx, true);
    }

    pub(super) fn end_curve_drag_for_row(&mut self, row_idx: usize) {
        if self
            .layer_fx_curve_drag
            .map(|d| d.row_idx == row_idx)
            .unwrap_or(false)
        {
            self.layer_fx_curve_drag = None;
        }
    }

    pub(super) fn set_curve_segment_ease(
        &mut self,
        row_idx: usize,
        seg_idx: usize,
        ease: LayerFxCurveEase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.layer_fx_curve_editors.get_mut(row_idx) else {
            return;
        };
        if seg_idx >= row.points.len().saturating_sub(1) {
            return;
        }
        row.points[seg_idx].ease = ease;
        self.layer_fx_curve_open_menu = None;
        self.apply_layer_fx_curves(window, cx, true);
    }

    pub(super) fn add_curve_point(
        &mut self,
        row_idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.layer_fx_curve_editors.get_mut(row_idx) else {
            return;
        };
        if row.points.len() < 2 {
            return;
        }
        let selected = row.selected_point.min(row.points.len() - 1);
        let (left_idx, right_idx) = if selected + 1 < row.points.len() {
            (selected, selected + 1)
        } else {
            (selected.saturating_sub(1), selected)
        };
        let left = row.points[left_idx];
        let right = row.points[right_idx];
        let insert = LayerFxCurvePoint {
            t_sec: (left.t_sec + right.t_sec) * 0.5,
            value: ((left.value + right.value) * 0.5).clamp(0.0, 1.0),
            ease: LayerFxCurveEase::Linear,
        };
        row.points.insert(right_idx, insert);
        row.selected_point = right_idx;
        Self::normalize_curve_points(
            &mut row.points,
            row.duration_sec,
            row.value_min,
            row.value_max,
        );
        self.apply_layer_fx_curves(window, cx, true);
    }

    pub(super) fn remove_selected_curve_point(
        &mut self,
        row_idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.layer_fx_curve_editors.get_mut(row_idx) else {
            return;
        };
        if row.points.len() <= 2 {
            return;
        }
        let selected = row.selected_point.min(row.points.len() - 1);
        if selected == 0 || selected + 1 == row.points.len() {
            return;
        }
        row.points.remove(selected);
        row.selected_point = selected.saturating_sub(1);
        Self::normalize_curve_points(
            &mut row.points,
            row.duration_sec,
            row.value_min,
            row.value_max,
        );
        self.apply_layer_fx_curves(window, cx, true);
    }

    pub(super) fn persist_layer_fx_script_validated(
        &mut self,
        layer_id: u64,
        script: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !is_graph_script(script) {
            return Err(
                "Layer FX script must be <Graph ...> DSL (legacy commands are not supported here)."
                    .to_string(),
            );
        }
        let graph = parse_graph_script(script)
            .map_err(|err| format!("Parse error at line {}: {}", err.line, err.message))?;
        let runtime = compile_runtime_program(graph)
            .map_err(|err| format!("Runtime compile error: {}", err.message))?;
        if !runtime.unsupported_kernels().is_empty() {
            return Err(format!(
                "Unsupported kernel(s): {}",
                runtime.unsupported_kernels().join(", ")
            ));
        }
        let script_owned = script.to_string();
        self.global.update(cx, |gs, cx| {
            let _ = gs.set_layer_effect_clip_motionloom_script(layer_id, script_owned.clone());
            cx.notify();
        });
        Ok(())
    }

    pub(super) fn apply_layer_fx_curves(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        live_preview: bool,
    ) {
        if self.layer_fx_curve_editors.is_empty() {
            if !live_preview {
                self.layer_fx_script_status =
                    "No Pass curves available. Apply Script once to generate curve lanes."
                        .to_string();
            }
            return;
        }
        let Some(layer_id) = self.layer_fx_script_layer_id else {
            self.layer_fx_script_status = "No Layer FX selected.".to_string();
            return;
        };
        let mut next_script = self.layer_fx_script_text.clone();
        for row in &mut self.layer_fx_curve_editors {
            Self::normalize_curve_points(
                &mut row.points,
                row.duration_sec,
                row.value_min,
                row.value_max,
            );
            let curve_expr = Self::curve_points_to_expr(&row.points);
            let Some(updated) = Self::upsert_pass_curve_param(
                &next_script,
                &row.pass_id,
                &row.param_key,
                &curve_expr,
            ) else {
                self.layer_fx_script_status = format!(
                    "Cannot locate pass '{}' or param '{}' while applying curves.",
                    row.pass_id, row.param_key
                );
                return;
            };
            next_script = updated;
        }
        self.set_layer_fx_script_text(next_script, window, cx);
        let script_to_persist = self.layer_fx_script_text.clone();
        match self.persist_layer_fx_script_validated(layer_id, &script_to_persist, cx) {
            Ok(()) => {
                if !live_preview {
                    self.layer_fx_script_status =
                        "Curve lanes applied to script (curve-enabled params).".to_string();
                }
            }
            Err(err) => {
                self.layer_fx_script_status = err;
            }
        }
    }

    pub(super) fn apply_layer_fx_script(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(layer_id) = self.layer_fx_script_layer_id else {
            self.layer_fx_script_status = "No Layer FX selected.".to_string();
            return;
        };
        let script = self.layer_fx_script_text.trim().to_string();
        if script.is_empty() {
            self.global.update(cx, |gs, cx| {
                let _ = gs.set_layer_effect_clip_motionloom_script(layer_id, String::new());
                cx.notify();
            });
            self.layer_fx_script_status = format!("Layer FX #{} script cleared.", layer_id);
            self.layer_fx_curve_editors.clear();
            return;
        }
        match self.persist_layer_fx_script_validated(layer_id, &script, cx) {
            Ok(()) => {
                self.layer_fx_script_status = format!("Layer FX #{} script applied.", layer_id);
                self.rebuild_layer_fx_curve_editors(window, cx);
            }
            Err(err) => {
                self.layer_fx_script_status = err;
            }
        }
    }

    pub(super) fn set_layer_fx_script_text(
        &mut self,
        script: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layer_fx_script_text = script;
        if let Some(input) = self.layer_fx_script_input.as_ref() {
            let value = self.layer_fx_script_text.clone();
            input.update(cx, |this, cx| {
                this.set_value(value.clone(), window, cx);
            });
        }
    }

    pub(super) fn layer_fx_template_label(
        kind: motionloom_templates::LayerEffectTemplateKind,
    ) -> &'static str {
        match kind {
            motionloom_templates::LayerEffectTemplateKind::BlurGaussian => "Blur Gaussian",
            motionloom_templates::LayerEffectTemplateKind::Sharpen => "Sharpen",
            motionloom_templates::LayerEffectTemplateKind::Opacity => "Opacity",
            motionloom_templates::LayerEffectTemplateKind::Lut => "LUT",
            motionloom_templates::LayerEffectTemplateKind::HslaOverlay => "HSLA Overlay",
            motionloom_templates::LayerEffectTemplateKind::TransitionFadeInOut => {
                "Transition Fade In/Out"
            }
        }
    }

    pub(super) fn toggle_layer_fx_template_selection(
        &mut self,
        kind: motionloom_templates::LayerEffectTemplateKind,
    ) {
        if let Some(idx) = self
            .layer_fx_template_selected
            .iter()
            .position(|selected| *selected == kind)
        {
            self.layer_fx_template_selected.remove(idx);
            return;
        }
        self.layer_fx_template_selected.push(kind);
    }

    pub(super) fn selected_layer_fx_template_summary(&self) -> String {
        if self.layer_fx_template_selected.is_empty() {
            return "No templates selected.".to_string();
        }
        self.layer_fx_template_selected
            .iter()
            .map(|kind| Self::layer_fx_template_label(*kind))
            .collect::<Vec<_>>()
            .join(" -> ")
    }

    pub(super) fn apply_selected_layer_fx_templates(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.layer_fx_template_selected.is_empty() {
            self.layer_fx_script_status =
                "Choose at least one template before pressing OK.".to_string();
            return;
        }

        let add_time = self.layer_fx_template_add_time_parameter;
        let add_curve = self.layer_fx_template_add_curve_parameter;
        let selected = self.layer_fx_template_selected.clone();
        let existing_script = self.layer_fx_script_text.trim().to_string();
        let selection_label = self.selected_layer_fx_template_summary();
        let result = if existing_script.is_empty() {
            motionloom_templates::build_layer_effect_chain_script(&selected, add_time, add_curve)
        } else {
            motionloom_templates::append_layer_effect_template_chain_script(
                &existing_script,
                &selected,
                add_curve,
            )
        };

        let Some(script) = result else {
            self.layer_fx_script_status =
                "Current script is not a standard chainable layer graph. Clear it before applying a multi-template selection."
                    .to_string();
            return;
        };

        self.set_layer_fx_script_text(script, window, cx);
        self.layer_fx_template_modal_open = false;
        self.layer_fx_template_selected.clear();
        self.layer_fx_script_status = if existing_script.is_empty() {
            if add_time && add_curve {
                format!("Inserted template chain: {selection_label} (+apply graph + curve params).")
            } else if add_time {
                format!("Inserted template chain: {selection_label} (+apply graph, duration 5s).")
            } else if add_curve {
                format!("Inserted template chain: {selection_label} (+curve params).")
            } else {
                format!("Inserted template chain: {selection_label}.")
            }
        } else if add_curve {
            format!("Appended template chain: {selection_label} (+curve params).")
        } else {
            format!("Appended template chain: {selection_label}.")
        };
    }

    pub(super) fn open_layer_fx_template_modal(&mut self) {
        self.layer_fx_template_modal_open = true;
        self.layer_fx_template_selected.clear();
        self.layer_fx_script_status = "Template picker opened.".to_string();
    }

    pub(super) fn render_layer_fx_template_tile(
        &self,
        kind: motionloom_templates::LayerEffectTemplateKind,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let selected = self.layer_fx_template_selected.contains(&kind);
        let border = if selected {
            rgba(0x4f8fffeb)
        } else {
            rgba(0xffffff3d)
        };
        let bg = if selected {
            rgba(0x253c62c7)
        } else {
            rgba(0xffffff1f)
        };
        let label = Self::layer_fx_template_label(kind);
        div()
            .h(px(34.0))
            .w(px(220.0))
            .px_3()
            .rounded_sm()
            .border_1()
            .border_color(border)
            .bg(bg)
            .text_sm()
            .text_color(white().opacity(0.94))
            .cursor_pointer()
            .overflow_hidden()
            .child(div().w_full().truncate().child(label))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.toggle_layer_fx_template_selection(kind);
                    cx.notify();
                }),
            )
    }

    pub(super) fn render_layer_fx_template_modal_overlay(
        &mut self,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let add_time_label = if self.layer_fx_template_add_time_parameter {
            "ADD TIME PARAMETER: ON"
        } else {
            "ADD TIME PARAMETER: OFF"
        };
        let add_curve_label = if self.layer_fx_template_add_curve_parameter {
            "ADD CURVE PARAMETER: ON"
        } else {
            "ADD CURVE PARAMETER: OFF"
        };
        let selection_summary = self.selected_layer_fx_template_summary();

        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right_0()
            .bg(gpui_component::black().opacity(0.55))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.layer_fx_template_modal_open = false;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(820.0))
                    .h(px(500.0))
                    .rounded_md()
                    .bg(rgb(0x1f1f23))
                    .border_1()
                    .border_color(white().opacity(0.16))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.9))
                            .child("MOTIONLOOM TEMPLATE PICKER"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.65))
                            .child("Select one or more templates, then press OK to generate one graph."),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                div()
                                    .h(px(30.0))
                                    .px_3()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(white().opacity(0.2))
                                    .bg(white().opacity(0.08))
                                    .text_xs()
                                    .text_color(white().opacity(0.9))
                                    .cursor_pointer()
                                    .child(add_time_label)
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.layer_fx_template_add_time_parameter =
                                                !this.layer_fx_template_add_time_parameter;
                                            cx.notify();
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .h(px(30.0))
                                    .px_3()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(white().opacity(0.2))
                                    .bg(rgba(0x253c62c7))
                                    .text_xs()
                                    .text_color(white().opacity(0.94))
                                    .cursor_pointer()
                                    .child("OK")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, window, cx| {
                                            this.apply_selected_layer_fx_templates(window, cx);
                                            cx.notify();
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .h(px(30.0))
                                    .px_3()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(white().opacity(0.2))
                                    .bg(white().opacity(0.08))
                                    .text_xs()
                                    .text_color(white().opacity(0.9))
                                    .cursor_pointer()
                                    .child(add_curve_label)
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.layer_fx_template_add_curve_parameter =
                                                !this.layer_fx_template_add_curve_parameter;
                                            cx.notify();
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .h(px(30.0))
                                    .px_3()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(white().opacity(0.2))
                                    .bg(white().opacity(0.06))
                                    .text_xs()
                                    .text_color(white().opacity(0.82))
                                    .cursor_pointer()
                                    .child("Close")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.layer_fx_template_modal_open = false;
                                            cx.notify();
                                        }),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.0))
                            .rounded_sm()
                            .border_1()
                            .border_color(white().opacity(0.12))
                            .bg(rgb(0x17181d))
                            .p_2()
                            .overflow_y_scrollbar()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.68))
                                    .child(format!("Selection: {selection_summary}")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.72))
                                    .child("Color Tuning"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(self.render_layer_fx_template_tile(
                                        motionloom_templates::LayerEffectTemplateKind::HslaOverlay,
                                        cx,
                                    ))
                                    .child(self.render_layer_fx_template_tile(
                                        motionloom_templates::LayerEffectTemplateKind::Lut,
                                        cx,
                                    )),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.72))
                                    .child("Blend & Opacity"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(self.render_layer_fx_template_tile(
                                        motionloom_templates::LayerEffectTemplateKind::Opacity,
                                        cx,
                                    )),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.72))
                                    .child("Detail & Blur"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(self.render_layer_fx_template_tile(
                                        motionloom_templates::LayerEffectTemplateKind::Sharpen,
                                        cx,
                                    ))
                                    .child(self.render_layer_fx_template_tile(
                                        motionloom_templates::LayerEffectTemplateKind::BlurGaussian,
                                        cx,
                                    )),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.72))
                                    .child("Transitions"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(self.render_layer_fx_template_tile(
                                        motionloom_templates::LayerEffectTemplateKind::TransitionFadeInOut,
                                        cx,
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.58))
                            .child(
                                "ADD TIME PARAMETER adds apply=graph + duration(5s). ADD CURVE PARAMETER injects curve(...) into template params. Selected templates are chained in the order shown above.",
                            ),
                    ),
            )
    }

    pub fn render_layer_fx_script_modal_overlay(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        if self.semantic_mask_painter.is_open() {
            return self.render_semantic_mask_painter_modal_overlay(window, cx);
        }
        if self.semantic_schema_modal_open {
            return self.render_semantic_schema_modal_overlay(window, cx);
        }
        if self.layer_fx_template_modal_open {
            return self.render_layer_fx_template_modal_overlay(cx);
        }
        if !self.layer_fx_script_modal_open {
            return div();
        }

        let modal_editor_elem = if let Some(input) = self.layer_fx_script_input.as_ref() {
            div()
                .w_full()
                .h(px(470.0))
                .rounded_sm()
                .border_1()
                .border_color(white().opacity(0.16))
                .bg(rgb(0x0b1020))
                .overflow_hidden()
                .child(Input::new(input).h_full().w_full())
                .into_any_element()
        } else {
            div()
                .w_full()
                .h(px(470.0))
                .rounded_sm()
                .bg(white().opacity(0.05))
                .into_any_element()
        };
        let modal_controls = div()
            .flex()
            .items_center()
            .flex_wrap()
            .justify_start()
            .gap_2()
            .child(
                div()
                    .h(px(28.0))
                    .px_2()
                    .rounded_sm()
                    .border_1()
                    .border_color(white().opacity(0.2))
                    .bg(white().opacity(0.06))
                    .text_xs()
                    .text_color(white().opacity(0.85))
                    .cursor_pointer()
                    .child("Insert Template")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.open_layer_fx_template_modal();
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .h(px(28.0))
                    .px_2()
                    .rounded_sm()
                    .border_1()
                    .border_color(white().opacity(0.2))
                    .bg(white().opacity(0.1))
                    .text_xs()
                    .text_color(white().opacity(0.9))
                    .cursor_pointer()
                    .child("Apply Script")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.apply_layer_fx_script(window, cx);
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .h(px(28.0))
                    .px_2()
                    .rounded_sm()
                    .border_1()
                    .border_color(white().opacity(0.2))
                    .bg(white().opacity(0.06))
                    .text_xs()
                    .text_color(white().opacity(0.82))
                    .cursor_pointer()
                    .child("Close")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.layer_fx_script_modal_open = false;
                            cx.notify();
                        }),
                    ),
            )
            .into_any_element();

        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right_0()
            .bg(gpui_component::black().opacity(0.55))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.layer_fx_script_modal_open = false;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(920.0))
                    .h(px(640.0))
                    .rounded_md()
                    .bg(rgb(0x1f1f23))
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
                            .flex()
                            .flex_col()
                            .gap_2()
                            .overflow_y_scrollbar()
                            .child(Self::layer_fx_script_editor_wrap(
                                "MOTIONLOOM SCRIPT (Expanded)",
                                modal_editor_elem,
                                modal_controls,
                                self.layer_fx_script_status.clone(),
                            ))
                            .child(self.layer_fx_curve_lanes_wrap(cx)),
                    ),
            )
    }
}
