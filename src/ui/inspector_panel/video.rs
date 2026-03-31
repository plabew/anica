use super::*;

pub(super) struct VideoClipPanelState {
    pub t_h: f32,
    pub t_s: f32,
    pub t_l: f32,
    pub t_a: f32,
    pub brightness_val: f32,
    pub contrast_val: f32,
    pub vid_saturation_val: f32,
    pub opacity_val: f32,
    pub blur_sigma_val: f32,
    pub local_mask_enabled: bool,
    pub local_mask_center_x_val: f32,
    pub local_mask_center_y_val: f32,
    pub local_mask_radius_val: f32,
    pub local_mask_feather_val: f32,
    pub local_mask_strength_val: f32,
    pub local_mask_brightness_val: f32,
    pub local_mask_contrast_val: f32,
    pub local_mask_saturation_val: f32,
    pub local_mask_opacity_val: f32,
    pub local_mask_blur_sigma_val: f32,
    pub local_mask_layer_count: usize,
    pub active_local_mask_layer_idx: usize,
    pub can_add_mask_layer: bool,
    pub fade_in_val: f32,
    pub fade_out_val: f32,
    pub dissolve_in_val: f32,
    pub dissolve_out_val: f32,
    pub slide_in_dir: SlideDirection,
    pub slide_out_dir: SlideDirection,
    pub slide_in_val: f32,
    pub slide_out_val: f32,
    pub zoom_in_val: f32,
    pub zoom_out_val: f32,
    pub zoom_amount_val: f32,
    pub shock_in_val: f32,
    pub shock_out_val: f32,
    pub shock_amount_val: f32,
    pub scale_val: f32,
    pub pos_x_val: f32,
    pub pos_y_val: f32,
    pub rotation_val: f32,
    pub show_transition: bool,
    pub fade_active: bool,
    pub dissolve_active: bool,
    pub slide_active: bool,
    pub zoom_active: bool,
    pub shock_active: bool,
    pub scale_keyframe_active: bool,
    pub rotation_keyframe_active: bool,
    pub pos_x_keyframe_active: bool,
    pub pos_y_keyframe_active: bool,
    pub brightness_keyframe_active: bool,
    pub contrast_keyframe_active: bool,
    pub saturation_keyframe_active: bool,
    pub opacity_keyframe_active: bool,
    pub blur_keyframe_active: bool,
    pub selected_clip_duration: Option<Duration>,
    pub selected_clip_local_playhead: Option<Duration>,
}

impl InspectorPanel {
    pub(super) fn apply_video_slider_value(
        gs: &mut GlobalState,
        key: &str,
        clamped: f32,
        mask_layer: usize,
    ) -> bool {
        match key {
            "clip_scale" => gs.set_selected_clip_scale(clamped),
            "clip_rotation" => gs.set_selected_clip_rotation(clamped),
            "clip_pos_x" => gs.set_selected_clip_pos_x(clamped),
            "clip_pos_y" => gs.set_selected_clip_pos_y(clamped),
            "clip_brightness" => gs.set_selected_clip_brightness(clamped),
            "clip_contrast" => gs.set_selected_clip_contrast(clamped),
            "clip_saturation" => gs.set_selected_clip_saturation(clamped),
            "clip_opacity" => gs.set_selected_clip_opacity(clamped),
            "clip_blur" => gs.set_selected_clip_blur_sigma(clamped),
            "layer_brightness" => {
                gs.set_selected_layer_effect_brightness(clamped);
            }
            "layer_contrast" => {
                gs.set_selected_layer_effect_contrast(clamped);
            }
            "layer_saturation" => {
                gs.set_selected_layer_effect_saturation(clamped);
            }
            "layer_blur" => {
                gs.set_selected_layer_effect_blur(clamped);
            }
            "mask_center_x" => gs.set_selected_clip_local_mask_center_x_at(mask_layer, clamped),
            "mask_center_y" => gs.set_selected_clip_local_mask_center_y_at(mask_layer, clamped),
            "mask_radius" => gs.set_selected_clip_local_mask_radius_at(mask_layer, clamped),
            "mask_feather" => gs.set_selected_clip_local_mask_feather_at(mask_layer, clamped),
            "mask_strength" => gs.set_selected_clip_local_mask_strength_at(mask_layer, clamped),
            "mask_brightness" => {
                gs.set_selected_clip_local_mask_adjust_brightness_at(mask_layer, clamped)
            }
            "mask_contrast" => {
                gs.set_selected_clip_local_mask_adjust_contrast_at(mask_layer, clamped)
            }
            "mask_saturation" => {
                gs.set_selected_clip_local_mask_adjust_saturation_at(mask_layer, clamped)
            }
            "mask_opacity" => {
                gs.set_selected_clip_local_mask_adjust_opacity_at(mask_layer, clamped)
            }
            "mask_blur" => {
                gs.set_selected_clip_local_mask_adjust_blur_sigma_at(mask_layer, clamped)
            }
            "fade_in" => gs.set_selected_clip_fade_in(clamped),
            "fade_out" => gs.set_selected_clip_fade_out(clamped),
            "dissolve_in" => gs.set_selected_clip_dissolve_in(clamped),
            "dissolve_out" => gs.set_selected_clip_dissolve_out(clamped),
            "slide_in" => gs.set_selected_clip_slide_in(clamped),
            "slide_out" => gs.set_selected_clip_slide_out(clamped),
            "zoom_in" => gs.set_selected_clip_zoom_in(clamped),
            "zoom_out" => gs.set_selected_clip_zoom_out(clamped),
            "zoom_amount" => gs.set_selected_clip_zoom_amount(clamped),
            "shock_in" => gs.set_selected_clip_shock_zoom_in(clamped),
            "shock_out" => gs.set_selected_clip_shock_zoom_out(clamped),
            "shock_amount" => gs.set_selected_clip_shock_zoom_amount(clamped),
            _ => return false,
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_video_clip_editor_panel(
        &mut self,
        has_clip_selection: bool,
        has_subtitle_selection: bool,
        state: &VideoClipPanelState,
        scale_keyframe_times: &[Duration],
        rotation_keyframe_times: &[Duration],
        pos_x_keyframe_times: &[Duration],
        pos_y_keyframe_times: &[Duration],
        brightness_keyframe_times: &[Duration],
        contrast_keyframe_times: &[Duration],
        saturation_keyframe_times: &[Duration],
        opacity_keyframe_times: &[Duration],
        blur_keyframe_times: &[Duration],
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !has_clip_selection || has_subtitle_selection {
            return None;
        }

        let hue_ent = self.hue_slider.as_ref().unwrap();
        let sat_ent = self.sat_slider.as_ref().unwrap();
        let lum_ent = self.lum_slider.as_ref().unwrap();
        let alpha_ent = self.alpha_slider.as_ref().unwrap();
        let scale_ent = self.scale_slider.as_ref().unwrap();
        let rot_ent = self.rotation_slider.as_ref().unwrap();
        let px_ent = self.pos_x_slider.as_ref().unwrap();
        let py_ent = self.pos_y_slider.as_ref().unwrap();
        let br_ent = self.bright_slider.as_ref().unwrap();
        let ct_ent = self.contrast_slider.as_ref().unwrap();
        let vs_ent = self.vid_sat_slider.as_ref().unwrap();
        let op_ent = self.opacity_slider.as_ref().unwrap();
        let blur_ent = self.blur_slider.as_ref().unwrap();
        let mask_cx_ent = self.local_mask_center_x_slider.as_ref().unwrap();
        let mask_cy_ent = self.local_mask_center_y_slider.as_ref().unwrap();
        let mask_radius_ent = self.local_mask_radius_slider.as_ref().unwrap();
        let mask_feather_ent = self.local_mask_feather_slider.as_ref().unwrap();
        let mask_strength_ent = self.local_mask_strength_slider.as_ref().unwrap();
        let mask_bright_ent = self.local_mask_bright_slider.as_ref().unwrap();
        let mask_contrast_ent = self.local_mask_contrast_slider.as_ref().unwrap();
        let mask_sat_ent = self.local_mask_sat_slider.as_ref().unwrap();
        let mask_opacity_ent = self.local_mask_opacity_slider.as_ref().unwrap();
        let mask_blur_ent = self.local_mask_blur_slider.as_ref().unwrap();
        let fade_in_ent = self.fade_in_slider.as_ref().unwrap();
        let fade_out_ent = self.fade_out_slider.as_ref().unwrap();
        let dissolve_in_ent = self.dissolve_in_slider.as_ref().unwrap();
        let dissolve_out_ent = self.dissolve_out_slider.as_ref().unwrap();
        let slide_in_ent = self.slide_in_slider.as_ref().unwrap();
        let slide_out_ent = self.slide_out_slider.as_ref().unwrap();
        let zoom_in_ent = self.zoom_in_slider.as_ref().unwrap();
        let zoom_out_ent = self.zoom_out_slider.as_ref().unwrap();
        let zoom_amount_ent = self.zoom_amount_slider.as_ref().unwrap();
        let shock_in_ent = self.shock_in_slider.as_ref().unwrap();
        let shock_out_ent = self.shock_out_slider.as_ref().unwrap();
        let shock_amount_ent = self.shock_amount_slider.as_ref().unwrap();

        let swatch_color = gpui::hsla(state.t_h / 360.0, state.t_s, state.t_l, state.t_a);
        let hsla_col = |label: &str, entity: &Entity<SliderState>, val_str: String| {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.6))
                        .child(label.to_string()),
                )
                .child(Slider::new(entity).vertical().h(px(120.)).w(px(30.)))
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.8))
                        .child(val_str),
                )
        };
        let slider_row_keyframe = |label: &str,
                                   slider: &Entity<SliderState>,
                                   val_el: gpui::AnyElement,
                                   key_button: gpui::Div| {
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
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(val_el)
                        .child(key_button),
                )
        };

        let ev_clip_scale = self.editable_value_display(
            "clip_scale",
            format!("{:.2}", state.scale_val),
            0.0,
            5.0,
            cx,
        );
        let ev_clip_rotation = self.editable_value_display(
            "clip_rotation",
            format!("{:.1}\u{00B0}", state.rotation_val),
            -180.0,
            180.0,
            cx,
        );
        let ev_clip_pos_x = self.editable_value_display(
            "clip_pos_x",
            format!("{:.2}", state.pos_x_val),
            -1.0,
            1.0,
            cx,
        );
        let ev_clip_pos_y = self.editable_value_display(
            "clip_pos_y",
            format!("{:.2}", state.pos_y_val),
            -1.0,
            1.0,
            cx,
        );
        let ev_clip_brightness = self.editable_value_display(
            "clip_brightness",
            format!("{:.2}", state.brightness_val),
            -1.0,
            1.0,
            cx,
        );
        let ev_clip_contrast = self.editable_value_display(
            "clip_contrast",
            format!("{:.2}", state.contrast_val),
            0.0,
            2.0,
            cx,
        );
        let ev_clip_saturation = self.editable_value_display(
            "clip_saturation",
            format!("{:.2}", state.vid_saturation_val),
            0.0,
            2.0,
            cx,
        );
        let ev_clip_opacity = self.editable_value_display(
            "clip_opacity",
            format!("{:.2}", state.opacity_val),
            0.0,
            1.0,
            cx,
        );
        let ev_clip_blur = self.editable_value_display(
            "clip_blur",
            format!("{:.1}", state.blur_sigma_val),
            0.0,
            32.0,
            cx,
        );
        let ev_mask_center_x = self.editable_value_display(
            "mask_center_x",
            format!("{:.2}", state.local_mask_center_x_val),
            0.0,
            1.0,
            cx,
        );
        let ev_mask_center_y = self.editable_value_display(
            "mask_center_y",
            format!("{:.2}", state.local_mask_center_y_val),
            0.0,
            1.0,
            cx,
        );
        let ev_mask_radius = self.editable_value_display(
            "mask_radius",
            format!("{:.2}", state.local_mask_radius_val),
            0.0,
            1.0,
            cx,
        );
        let ev_mask_feather = self.editable_value_display(
            "mask_feather",
            format!("{:.2}", state.local_mask_feather_val),
            0.0,
            1.0,
            cx,
        );
        let ev_mask_strength = self.editable_value_display(
            "mask_strength",
            format!("{:.2}", state.local_mask_strength_val),
            0.0,
            1.0,
            cx,
        );
        let ev_mask_brightness = self.editable_value_display(
            "mask_brightness",
            format!("{:.2}", state.local_mask_brightness_val),
            -1.0,
            1.0,
            cx,
        );
        let ev_mask_contrast = self.editable_value_display(
            "mask_contrast",
            format!("{:.2}", state.local_mask_contrast_val),
            0.0,
            2.0,
            cx,
        );
        let ev_mask_saturation = self.editable_value_display(
            "mask_saturation",
            format!("{:.2}", state.local_mask_saturation_val),
            0.0,
            2.0,
            cx,
        );
        let ev_mask_opacity = self.editable_value_display(
            "mask_opacity",
            format!("{:.2}", state.local_mask_opacity_val),
            0.0,
            1.0,
            cx,
        );
        let ev_mask_blur = self.editable_value_display(
            "mask_blur",
            format!("{:.1}", state.local_mask_blur_sigma_val),
            0.0,
            32.0,
            cx,
        );
        let ev_fade_in = self.editable_value_display(
            "fade_in",
            format!("{:.2}s", state.fade_in_val),
            0.0,
            10.0,
            cx,
        );
        let ev_fade_out = self.editable_value_display(
            "fade_out",
            format!("{:.2}s", state.fade_out_val),
            0.0,
            10.0,
            cx,
        );
        let ev_dissolve_in = self.editable_value_display(
            "dissolve_in",
            format!("{:.2}s", state.dissolve_in_val),
            0.0,
            10.0,
            cx,
        );
        let ev_dissolve_out = self.editable_value_display(
            "dissolve_out",
            format!("{:.2}s", state.dissolve_out_val),
            0.0,
            10.0,
            cx,
        );
        let ev_slide_in = self.editable_value_display(
            "slide_in",
            format!("{:.2}s", state.slide_in_val),
            0.0,
            10.0,
            cx,
        );
        let ev_slide_out = self.editable_value_display(
            "slide_out",
            format!("{:.2}s", state.slide_out_val),
            0.0,
            10.0,
            cx,
        );
        let ev_zoom_in = self.editable_value_display(
            "zoom_in",
            format!("{:.2}s", state.zoom_in_val),
            0.0,
            10.0,
            cx,
        );
        let ev_zoom_out = self.editable_value_display(
            "zoom_out",
            format!("{:.2}s", state.zoom_out_val),
            0.0,
            10.0,
            cx,
        );
        let ev_zoom_amount = self.editable_value_display(
            "zoom_amount",
            format!("{:.2}x", state.zoom_amount_val),
            0.5,
            2.0,
            cx,
        );
        let ev_shock_in = self.editable_value_display(
            "shock_in",
            format!("{:.2}s", state.shock_in_val),
            0.0,
            10.0,
            cx,
        );
        let ev_shock_out = self.editable_value_display(
            "shock_out",
            format!("{:.2}s", state.shock_out_val),
            0.0,
            10.0,
            cx,
        );
        let ev_shock_amount = self.editable_value_display(
            "shock_amount",
            format!("{:.2}x", state.shock_amount_val),
            0.5,
            3.0,
            cx,
        );

        let clip_keyframe_controls = |channel: ClipKeyframeChannel, active: bool| {
            div().flex().items_center().gap_1().child(
                div()
                    .w(px(18.0))
                    .h(px(18.0))
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(0x22c55e))
                    .bg(if active {
                        rgb(0x22c55e)
                    } else {
                        rgba(0x22c55e22)
                    })
                    .text_color(if active { rgb(0x0b0b0d) } else { rgb(0x22c55e) })
                    .text_xs()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .child("◆")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.selected_clip_keyframe_channel = Some(channel);
                            this.global.update(cx, |gs, cx| {
                                if gs.toggle_selected_clip_keyframe_at_playhead(channel) {
                                    cx.notify();
                                }
                            });
                            cx.notify();
                        }),
                    ),
            )
        };

        let clip_keyframe_timeline_row =
            |channel: ClipKeyframeChannel, key_times: &[Duration]| -> gpui::AnyElement {
                if key_times.is_empty() {
                    return div().into_any_element();
                }

                let duration_secs = state
                    .selected_clip_duration
                    .map(|d| d.as_secs_f32().max(0.001))
                    .unwrap_or(0.001);
                let duration_label = state
                    .selected_clip_duration
                    .map(|d| format!("{:.2}s", d.as_secs_f32()))
                    .unwrap_or_else(|| "--".to_string());
                let playhead_secs = state
                    .selected_clip_local_playhead
                    .map(|d| d.as_secs_f32().clamp(0.0, duration_secs));
                let highlight_threshold = Duration::from_millis(33).as_secs_f32();

                let mut lane = div()
                    .relative()
                    .h(px(24.0))
                    .w_full()
                    .rounded_sm()
                    .border_1()
                    .border_color(rgba(0xffffff29))
                    .bg(rgba(0xffffff0a));

                let mut rail = div()
                    .absolute()
                    .left(px(8.0))
                    .right(px(8.0))
                    .top(px(0.0))
                    .h_full()
                    .relative()
                    .child(
                        div()
                            .absolute()
                            .left(px(0.0))
                            .right(px(0.0))
                            .top(px(11.0))
                            .h(px(2.0))
                            .rounded_full()
                            .bg(rgba(0xffffff2e)),
                    );

                if let Some(playhead) = playhead_secs {
                    let ratio = (playhead / duration_secs).clamp(0.0, 1.0);
                    rail = rail.child(
                        div()
                            .absolute()
                            .left(relative(ratio))
                            .top(px(4.0))
                            .w(px(2.0))
                            .h(px(16.0))
                            .rounded_full()
                            .bg(rgba(0x22c55e99)),
                    );
                }

                for time in key_times {
                    let key_time = *time;
                    let secs = key_time.as_secs_f32().clamp(0.0, duration_secs);
                    let ratio = (secs / duration_secs).clamp(0.0, 1.0);
                    let is_at_playhead = playhead_secs
                        .map(|playhead| (playhead - secs).abs() <= highlight_threshold)
                        .unwrap_or(false);
                    let is_active_channel = self.selected_clip_keyframe_channel == Some(channel);
                    let is_active = is_active_channel && is_at_playhead;

                    let mut node = div()
                        .absolute()
                        .left(relative(ratio))
                        .top(px(7.0))
                        .w(px(10.0))
                        .h(px(10.0))
                        .rounded_full()
                        .border_1()
                        .border_color(if is_active {
                            rgb(0x22c55e)
                        } else {
                            rgba(0xffffffb8)
                        })
                        .bg(if is_active {
                            rgb(0x22c55e)
                        } else {
                            rgb(0x0f1118)
                        })
                        .cursor_pointer();

                    node = node.on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.selected_clip_keyframe_channel = Some(channel);
                            this.global.update(cx, |gs, cx| {
                                if gs.selected_clip_set_playhead_to_local_time(key_time) {
                                    cx.notify();
                                }
                            });
                            cx.notify();
                        }),
                    );

                    node = node.on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, _, cx| {
                            this.selected_clip_keyframe_channel = Some(channel);
                            this.global.update(cx, |gs, cx| {
                                let jumped = gs.selected_clip_set_playhead_to_local_time(key_time);
                                let removed =
                                    jumped && gs.remove_selected_clip_keyframe_at_playhead(channel);
                                if removed {
                                    cx.notify();
                                }
                            });
                            cx.notify();
                        }),
                    );

                    rail = rail.child(node);
                }

                lane = lane.child(rail);

                let del_button = div()
                    .w(px(30.0))
                    .h(px(18.0))
                    .rounded_sm()
                    .border_1()
                    .border_color(rgb(0xef4444))
                    .bg(rgba(0xef444422))
                    .text_color(rgb(0xef4444))
                    .text_xs()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .child("Del")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.selected_clip_keyframe_channel = Some(channel);
                            this.global.update(cx, |gs, cx| {
                                if gs.remove_selected_clip_keyframe_at_playhead(channel) {
                                    cx.notify();
                                }
                            });
                            cx.notify();
                        }),
                    );

                let mut timeline = div()
                    .w(px(296.0))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().child(lane))
                            .child(del_button),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_xs()
                            .text_color(rgba(0xffffff73))
                            .child("0s")
                            .child(
                                playhead_secs
                                    .map(|secs| format!("Head {secs:.2}s"))
                                    .unwrap_or_else(|| "--".to_string()),
                            )
                            .child(duration_label),
                    );

                if self.selected_clip_keyframe_channel == Some(channel) {
                    timeline = timeline.child(
                        div()
                            .text_xs()
                            .text_color(rgba(0xffffff57))
                            .child("L-click node = jump, R-click node / Del = delete"),
                    );
                }

                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(timeline)
                    .into_any_element()
            };

        let clip_slider_row_keyframe =
            |label: &str,
             slider: &Entity<SliderState>,
             val_el: gpui::AnyElement,
             channel: ClipKeyframeChannel,
             active: bool,
             key_times: &[Duration]| {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(slider_row_keyframe(
                        label,
                        slider,
                        val_el,
                        clip_keyframe_controls(channel, active),
                    ))
                    .when(!key_times.is_empty(), |this| {
                        this.child(clip_keyframe_timeline_row(channel, key_times))
                    })
            };

        let dir_chip = |label: &str,
                        active: bool,
                        on_click: fn(
            &mut Self,
            &gpui::MouseDownEvent,
            &mut Window,
            &mut Context<Self>,
        )| {
            let mut chip = div()
                .h(px(24.0))
                .px_2()
                .rounded_sm()
                .bg(if active {
                    white().opacity(0.18)
                } else {
                    white().opacity(0.06)
                })
                .text_color(white().opacity(if active { 0.95 } else { 0.6 }))
                .text_xs()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .child(label.to_string());
            chip = chip.on_mouse_down(MouseButton::Left, cx.listener(on_click));
            chip
        };

        let mut mask_layer_tabs = div().flex().items_center().gap_1();
        for layer_idx in 0..state.local_mask_layer_count {
            let is_active = layer_idx == state.active_local_mask_layer_idx;
            let mut chip = div()
                .h(px(22.0))
                .min_w(px(24.0))
                .px_2()
                .rounded_sm()
                .border_1()
                .border_color(if is_active {
                    white().opacity(0.45)
                } else {
                    white().opacity(0.14)
                })
                .bg(if is_active {
                    white().opacity(0.16)
                } else {
                    white().opacity(0.06)
                })
                .text_color(white().opacity(if is_active { 0.95 } else { 0.72 }))
                .text_xs()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .child((layer_idx + 1).to_string());
            chip = chip.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.global.update(cx, |gs, cx| {
                        gs.set_active_local_mask_layer(layer_idx);
                        cx.notify();
                    });
                    this.active_local_mask_layer = layer_idx;
                    cx.notify();
                }),
            );
            mask_layer_tabs = mask_layer_tabs.child(chip);
        }

        Some(
            div()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(white().opacity(0.5))
                                .child("TRANSFORM"),
                        )
                        .child(clip_slider_row_keyframe(
                            "Scale",
                            scale_ent,
                            ev_clip_scale,
                            ClipKeyframeChannel::Scale,
                            state.scale_keyframe_active,
                            scale_keyframe_times,
                        ))
                        .child(clip_slider_row_keyframe(
                            "Rotation",
                            rot_ent,
                            ev_clip_rotation,
                            ClipKeyframeChannel::Rotation,
                            state.rotation_keyframe_active,
                            rotation_keyframe_times,
                        ))
                        .child(clip_slider_row_keyframe(
                            "Position X",
                            px_ent,
                            ev_clip_pos_x,
                            ClipKeyframeChannel::PosX,
                            state.pos_x_keyframe_active,
                            pos_x_keyframe_times,
                        ))
                        .child(clip_slider_row_keyframe(
                            "Position Y",
                            py_ent,
                            ev_clip_pos_y,
                            ClipKeyframeChannel::PosY,
                            state.pos_y_keyframe_active,
                            pos_y_keyframe_times,
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
                                .child("VIDEO EFFECTS"),
                        )
                        .child(clip_slider_row_keyframe(
                            "Brightness",
                            br_ent,
                            ev_clip_brightness,
                            ClipKeyframeChannel::Brightness,
                            state.brightness_keyframe_active,
                            brightness_keyframe_times,
                        ))
                        .child(clip_slider_row_keyframe(
                            "Contrast",
                            ct_ent,
                            ev_clip_contrast,
                            ClipKeyframeChannel::Contrast,
                            state.contrast_keyframe_active,
                            contrast_keyframe_times,
                        ))
                        .child(clip_slider_row_keyframe(
                            "Saturation",
                            vs_ent,
                            ev_clip_saturation,
                            ClipKeyframeChannel::Saturation,
                            state.saturation_keyframe_active,
                            saturation_keyframe_times,
                        ))
                        .child(clip_slider_row_keyframe(
                            "Opacity",
                            op_ent,
                            ev_clip_opacity,
                            ClipKeyframeChannel::Opacity,
                            state.opacity_keyframe_active,
                            opacity_keyframe_times,
                        ))
                        .child(clip_slider_row_keyframe(
                            "Blur",
                            blur_ent,
                            ev_clip_blur,
                            ClipKeyframeChannel::Blur,
                            state.blur_keyframe_active,
                            blur_keyframe_times,
                        ))
                        .child(
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
                                        .child(format!(
                                            "Mask Layer {}",
                                            state.active_local_mask_layer_idx + 1
                                        )),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child({
                                            let mut add_btn = div()
                                                .h(px(24.0))
                                                .w(px(24.0))
                                                .rounded_sm()
                                                .border_1()
                                                .border_color(if state.can_add_mask_layer {
                                                    white().opacity(0.36)
                                                } else {
                                                    white().opacity(0.1)
                                                })
                                                .bg(if state.can_add_mask_layer {
                                                    white().opacity(0.1)
                                                } else {
                                                    white().opacity(0.03)
                                                })
                                                .text_color(if state.can_add_mask_layer {
                                                    white().opacity(0.95)
                                                } else {
                                                    white().opacity(0.35)
                                                })
                                                .text_sm()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .child("+");
                                            if state.can_add_mask_layer {
                                                add_btn = add_btn.cursor_pointer().on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(|this, _, _, cx| {
                                                        let mut new_layer = None;
                                                        this.global.update(cx, |gs, cx| {
                                                            new_layer =
                                                                gs.add_selected_clip_local_mask_layer();
                                                            if let Some(i) = new_layer {
                                                                gs.set_active_local_mask_layer(i);
                                                            }
                                                            cx.notify();
                                                        });
                                                        if let Some(i) = new_layer {
                                                            this.active_local_mask_layer = i;
                                                        }
                                                        cx.notify();
                                                    }),
                                                );
                                            }
                                            add_btn
                                        })
                                        .child(
                                            div()
                                                .h(px(24.0))
                                                .px_3()
                                                .rounded_sm()
                                                .bg(if state.local_mask_enabled {
                                                    white().opacity(0.14)
                                                } else {
                                                    white().opacity(0.06)
                                                })
                                                .border_1()
                                                .border_color(if state.local_mask_enabled {
                                                    white().opacity(0.5)
                                                } else {
                                                    white().opacity(0.12)
                                                })
                                                .text_color(if state.local_mask_enabled {
                                                    white().opacity(0.95)
                                                } else {
                                                    white().opacity(0.7)
                                                })
                                                .text_xs()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .cursor_pointer()
                                                .child(if state.local_mask_enabled { "On" } else { "Off" })
                                                .on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(|this, _, _, cx| {
                                                        let layer_idx = this.active_local_mask_layer;
                                                        this.global.update(cx, |gs, cx| {
                                                            let enabled = gs
                                                                .get_selected_clip_local_mask_layer(layer_idx)
                                                                .map(|v| v.0)
                                                                .unwrap_or(false);
                                                            gs.set_selected_clip_local_mask_enabled_at(
                                                                layer_idx,
                                                                !enabled,
                                                            );
                                                            cx.notify();
                                                        });
                                                        cx.notify();
                                                    }),
                                                ),
                                        ),
                                ),
                        )
                        .child(mask_layer_tabs)
                        .when(state.local_mask_enabled, |panel| {
                            panel
                                .child(Self::slider_row("Mask X", mask_cx_ent, ev_mask_center_x))
                                .child(Self::slider_row("Mask Y", mask_cy_ent, ev_mask_center_y))
                                .child(Self::slider_row(
                                    "Mask Radius",
                                    mask_radius_ent,
                                    ev_mask_radius,
                                ))
                                .child(Self::slider_row(
                                    "Mask Feather",
                                    mask_feather_ent,
                                    ev_mask_feather,
                                ))
                                .child(Self::slider_row(
                                    "Mask Amount",
                                    mask_strength_ent,
                                    ev_mask_strength,
                                ))
                                .child(Self::slider_row(
                                    &format!("L{} Brightness", state.active_local_mask_layer_idx + 1),
                                    mask_bright_ent,
                                    ev_mask_brightness,
                                ))
                                .child(Self::slider_row(
                                    &format!("L{} Contrast", state.active_local_mask_layer_idx + 1),
                                    mask_contrast_ent,
                                    ev_mask_contrast,
                                ))
                                .child(Self::slider_row(
                                    &format!("L{} Saturation", state.active_local_mask_layer_idx + 1),
                                    mask_sat_ent,
                                    ev_mask_saturation,
                                ))
                                .child(Self::slider_row(
                                    &format!("L{} Opacity", state.active_local_mask_layer_idx + 1),
                                    mask_opacity_ent,
                                    ev_mask_opacity,
                                ))
                                .child(Self::slider_row(
                                    &format!("L{} Blur", state.active_local_mask_layer_idx + 1),
                                    mask_blur_ent,
                                    ev_mask_blur,
                                ))
                        }),
                )
                .when(state.show_transition, |panel| {
                    panel.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.5))
                                    .child("TRANSITION"),
                            )
                            .when(state.fade_active, |panel| {
                                panel
                                    .child(Self::slider_row("Fade In", fade_in_ent, ev_fade_in))
                                    .child(Self::slider_row("Fade Out", fade_out_ent, ev_fade_out))
                            })
                            .when(state.dissolve_active, |panel| {
                                panel
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(white().opacity(0.5))
                                            .child("DISSOLVE"),
                                    )
                                    .child(Self::slider_row(
                                        "Dissolve In",
                                        dissolve_in_ent,
                                        ev_dissolve_in,
                                    ))
                                    .child(Self::slider_row(
                                        "Dissolve Out",
                                        dissolve_out_ent,
                                        ev_dissolve_out,
                                    ))
                            })
                            .when(state.slide_active, |panel| {
                                panel
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(white().opacity(0.5))
                                            .child("SLIDE IN DIRECTION"),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .gap_2()
                                            .child(dir_chip(
                                                "Left",
                                                state.slide_in_dir == SlideDirection::Left,
                                                |this, _, _, cx| {
                                                    this.global.update(cx, |gs, cx| {
                                                        gs.set_selected_clip_slide_in_direction(
                                                            SlideDirection::Left,
                                                        );
                                                        cx.notify();
                                                    });
                                                },
                                            ))
                                            .child(dir_chip(
                                                "Right",
                                                state.slide_in_dir == SlideDirection::Right,
                                                |this, _, _, cx| {
                                                    this.global.update(cx, |gs, cx| {
                                                        gs.set_selected_clip_slide_in_direction(
                                                            SlideDirection::Right,
                                                        );
                                                        cx.notify();
                                                    });
                                                },
                                            ))
                                            .child(dir_chip(
                                                "Up",
                                                state.slide_in_dir == SlideDirection::Up,
                                                |this, _, _, cx| {
                                                    this.global.update(cx, |gs, cx| {
                                                        gs.set_selected_clip_slide_in_direction(
                                                            SlideDirection::Up,
                                                        );
                                                        cx.notify();
                                                    });
                                                },
                                            ))
                                            .child(dir_chip(
                                                "Down",
                                                state.slide_in_dir == SlideDirection::Down,
                                                |this, _, _, cx| {
                                                    this.global.update(cx, |gs, cx| {
                                                        gs.set_selected_clip_slide_in_direction(
                                                            SlideDirection::Down,
                                                        );
                                                        cx.notify();
                                                    });
                                                },
                                            )),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(white().opacity(0.5))
                                            .child("SLIDE OUT DIRECTION"),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .gap_2()
                                            .child(dir_chip(
                                                "Left",
                                                state.slide_out_dir == SlideDirection::Left,
                                                |this, _, _, cx| {
                                                    this.global.update(cx, |gs, cx| {
                                                        gs.set_selected_clip_slide_out_direction(
                                                            SlideDirection::Left,
                                                        );
                                                        cx.notify();
                                                    });
                                                },
                                            ))
                                            .child(dir_chip(
                                                "Right",
                                                state.slide_out_dir == SlideDirection::Right,
                                                |this, _, _, cx| {
                                                    this.global.update(cx, |gs, cx| {
                                                        gs.set_selected_clip_slide_out_direction(
                                                            SlideDirection::Right,
                                                        );
                                                        cx.notify();
                                                    });
                                                },
                                            ))
                                            .child(dir_chip(
                                                "Up",
                                                state.slide_out_dir == SlideDirection::Up,
                                                |this, _, _, cx| {
                                                    this.global.update(cx, |gs, cx| {
                                                        gs.set_selected_clip_slide_out_direction(
                                                            SlideDirection::Up,
                                                        );
                                                        cx.notify();
                                                    });
                                                },
                                            ))
                                            .child(dir_chip(
                                                "Down",
                                                state.slide_out_dir == SlideDirection::Down,
                                                |this, _, _, cx| {
                                                    this.global.update(cx, |gs, cx| {
                                                        gs.set_selected_clip_slide_out_direction(
                                                            SlideDirection::Down,
                                                        );
                                                        cx.notify();
                                                    });
                                                },
                                            )),
                                    )
                                    .child(Self::slider_row("Slide In", slide_in_ent, ev_slide_in))
                                    .child(Self::slider_row("Slide Out", slide_out_ent, ev_slide_out))
                            })
                            .when(state.zoom_active, |panel| {
                                panel
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(white().opacity(0.5))
                                            .child("ZOOM"),
                                    )
                                    .child(Self::slider_row("Zoom In", zoom_in_ent, ev_zoom_in))
                                    .child(Self::slider_row("Zoom Out", zoom_out_ent, ev_zoom_out))
                                    .child(Self::slider_row(
                                        "Zoom Amount",
                                        zoom_amount_ent,
                                        ev_zoom_amount,
                                    ))
                            })
                            .when(state.shock_active, |panel| {
                                panel
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(white().opacity(0.5))
                                            .child("SHOCK ZOOM"),
                                    )
                                    .child(Self::slider_row("Shock In", shock_in_ent, ev_shock_in))
                                    .child(Self::slider_row("Shock Out", shock_out_ent, ev_shock_out))
                                    .child(Self::slider_row(
                                        "Shock Amount",
                                        shock_amount_ent,
                                        ev_shock_amount,
                                    ))
                            }),
                    )
                })
                .child(
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
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(white().opacity(0.5))
                                        .child("HSLA OVERLAY"),
                                )
                                .child(
                                    div()
                                        .w_4()
                                        .h_4()
                                        .rounded_full()
                                        .bg(swatch_color)
                                        .border_1()
                                        .border_color(white()),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .gap_1()
                                .child(hsla_col("Hue", hue_ent, format!("{:.0}", state.t_h)))
                                .child(hsla_col("Sat", sat_ent, format!("{:.2}", state.t_s)))
                                .child(hsla_col("Lum", lum_ent, format!("{:.2}", state.t_l)))
                                .child(hsla_col("Alpha", alpha_ent, format!("{:.2}", state.t_a))),
                        ),
                )
                .into_any_element(),
        )
    }
}
