use super::video::VideoClipPanelState;
use super::*;
impl Focusable for InspectorPanel {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for InspectorPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_layer_fx_script_input(window, cx);
        self.sync_layer_fx_script_from_selected_layer(window, cx);
        self.ensure_semantic_schema_input(window, cx);
        self.sync_semantic_schema_from_selected_clip(window, cx);
        let layer_fx_curve_lanes_elem = self.layer_fx_curve_lanes_wrap(cx).into_any_element();

        // ✅ 1. Isolate read operations (scoped read)
        let (
            t_h,
            t_s,
            t_l,
            t_a, // HSLA Overlay Values
            brightness_val,
            contrast_val,
            vid_saturation_val,
            opacity_val,    // Effects
            blur_sigma_val, // Effects
            layer_brightness_val,
            layer_contrast_val,
            layer_saturation_val,
            layer_blur_sigma_val,
            layer_brightness_enabled,
            layer_contrast_enabled,
            layer_saturation_enabled,
            layer_blur_enabled,
            local_mask_enabled,
            local_mask_center_x_val,
            local_mask_center_y_val,
            local_mask_radius_val,
            local_mask_feather_val,
            local_mask_strength_val,
            local_mask_brightness_val,
            local_mask_contrast_val,
            local_mask_saturation_val,
            local_mask_opacity_val,
            local_mask_blur_sigma_val,
            local_mask_layer_count,
            active_local_mask_layer_idx,
            fade_in_val,
            fade_out_val,
            dissolve_in_val,
            dissolve_out_val,
            slide_in_dir,
            slide_out_dir,
            slide_in_val,
            slide_out_val,
            zoom_in_val,
            zoom_out_val,
            zoom_amount_val,
            shock_in_val,
            shock_out_val,
            shock_amount_val,
            transform_val, // (Scale, X, Y)
            rotation_val,
            has_timeline_clip_selection,
            has_visual_clip_selection,
            has_audio_clip_selection,
            has_subtitle_selection,
            has_layer_effect_selection,
            layer_brightness_keyframe_active,
            layer_contrast_keyframe_active,
            layer_saturation_keyframe_active,
            layer_blur_keyframe_active,
            pos_x_keyframe_active,
            pos_y_keyframe_active,
            scale_keyframe_active,
            rotation_keyframe_active,
            brightness_keyframe_active,
            contrast_keyframe_active,
            saturation_keyframe_active,
            opacity_keyframe_active,
            blur_keyframe_active,
            selected_clip_duration,
            selected_clip_local_playhead,
            scale_keyframe_times,
            rotation_keyframe_times,
            pos_x_keyframe_times,
            pos_y_keyframe_times,
            brightness_keyframe_times,
            contrast_keyframe_times,
            saturation_keyframe_times,
            opacity_keyframe_times,
            blur_keyframe_times,
            selected_subtitle_id,
            selected_subtitle_ids,
            selected_subtitle_text,
            subtitle_transform_val,
            subtitle_group_id,
            subtitle_group_transform_val,
            selected_subtitle_font,
            selected_subtitle_color_rgba,
            selected_subtitle_group_color_rgba,
        ) = {
            let gs = self.global.read(cx);
            let hsla_overlay = gs
                .get_selected_clip_hsla_overlay()
                .unwrap_or((0.0, 0.0, 0.0, 0.0));
            // let transform = gs.get_selected_clip_transform().unwrap_or((1.0, 0.0, 0.0));
            let transform = gs.get_selected_clip_transform().unwrap_or((0.8, 0.0, 0.0)); // Default to 0.8 to preserve the original size
            let subtitle_transform = gs
                .get_selected_subtitle_transform()
                .unwrap_or((-0.30, 0.35, 48.0));
            let subtitle_group_transform = gs
                .get_selected_subtitle_group_transform()
                .unwrap_or((0.0, 0.0, 1.0));
            let (slide_in_dir, slide_out_dir, slide_in_val, slide_out_val) = gs
                .get_selected_clip_slide()
                .unwrap_or((SlideDirection::Right, SlideDirection::Left, 0.0, 0.0));
            let (zoom_in_val, zoom_out_val, zoom_amount_val) =
                gs.get_selected_clip_zoom().unwrap_or((0.0, 0.0, 1.1));
            let (shock_in_val, shock_out_val, shock_amount_val) =
                gs.get_selected_clip_shock_zoom().unwrap_or((0.0, 0.0, 1.2));
            let selected_clip_id = gs.selected_clip_id;
            let has_audio_clip_selection = Self::selected_clip_is_audio(gs);
            let has_visual_clip_selection = selected_clip_id.is_some() && !has_audio_clip_selection;
            let local_mask_layer_count = gs
                .get_selected_clip_local_mask_layer_count()
                .unwrap_or(1)
                .clamp(1, MAX_LOCAL_MASK_LAYERS);
            let active_local_mask_layer_idx = gs.active_local_mask_layer();
            let (
                local_mask_enabled,
                local_mask_center_x,
                local_mask_center_y,
                local_mask_radius,
                local_mask_feather,
                local_mask_strength,
            ) = gs
                .get_selected_clip_local_mask_layer(active_local_mask_layer_idx)
                .unwrap_or((false, 0.5, 0.5, 0.25, 0.15, 1.0));
            let (
                local_mask_brightness,
                local_mask_contrast,
                local_mask_saturation,
                local_mask_opacity,
                local_mask_blur_sigma,
            ) = gs
                .get_selected_clip_local_mask_adjust_layer(active_local_mask_layer_idx)
                .unwrap_or((0.0, 1.0, 1.0, 1.0, 0.0));

            (
                hsla_overlay.0,
                hsla_overlay.1,
                hsla_overlay.2,
                hsla_overlay.3,
                gs.get_selected_clip_brightness().unwrap_or(0.0),
                gs.get_selected_clip_contrast().unwrap_or(1.0),
                gs.get_selected_clip_saturation().unwrap_or(1.0),
                gs.get_selected_clip_opacity().unwrap_or(1.0),
                gs.get_selected_clip_blur_sigma().unwrap_or(0.0),
                gs.get_selected_layer_effect_brightness().unwrap_or(0.0),
                gs.get_selected_layer_effect_contrast().unwrap_or(1.0),
                gs.get_selected_layer_effect_saturation().unwrap_or(1.0),
                gs.get_selected_layer_effect_blur().unwrap_or(0.0),
                gs.selected_layer_effect_brightness_enabled(),
                gs.selected_layer_effect_contrast_enabled(),
                gs.selected_layer_effect_saturation_enabled(),
                gs.selected_layer_effect_blur_enabled(),
                local_mask_enabled,
                local_mask_center_x,
                local_mask_center_y,
                local_mask_radius,
                local_mask_feather,
                local_mask_strength,
                local_mask_brightness,
                local_mask_contrast,
                local_mask_saturation,
                local_mask_opacity,
                local_mask_blur_sigma,
                local_mask_layer_count,
                active_local_mask_layer_idx,
                gs.get_selected_clip_fade_in().unwrap_or(0.0),
                gs.get_selected_clip_fade_out().unwrap_or(0.0),
                gs.get_selected_clip_dissolve_in().unwrap_or(0.0),
                gs.get_selected_clip_dissolve_out().unwrap_or(0.0),
                slide_in_dir,
                slide_out_dir,
                slide_in_val,
                slide_out_val,
                zoom_in_val,
                zoom_out_val,
                zoom_amount_val,
                shock_in_val,
                shock_out_val,
                shock_amount_val,
                transform,
                gs.get_selected_clip_rotation().unwrap_or(0.0),
                selected_clip_id.is_some(),
                has_visual_clip_selection,
                has_audio_clip_selection,
                gs.selected_subtitle_id.is_some(),
                gs.layer_effect_clip_selected(),
                gs.selected_layer_effect_has_brightness_keyframe(),
                gs.selected_layer_effect_has_contrast_keyframe(),
                gs.selected_layer_effect_has_saturation_keyframe(),
                gs.selected_layer_effect_has_blur_keyframe(),
                gs.selected_clip_has_pos_x_keyframe(),
                gs.selected_clip_has_pos_y_keyframe(),
                gs.selected_clip_has_scale_keyframe(),
                gs.selected_clip_has_rotation_keyframe(),
                gs.selected_clip_has_brightness_keyframe(),
                gs.selected_clip_has_contrast_keyframe(),
                gs.selected_clip_has_saturation_keyframe(),
                gs.selected_clip_has_opacity_keyframe(),
                gs.selected_clip_has_blur_keyframe(),
                gs.selected_clip_duration(),
                gs.selected_clip_local_playhead_time(),
                gs.selected_clip_keyframe_times(ClipKeyframeChannel::Scale),
                gs.selected_clip_keyframe_times(ClipKeyframeChannel::Rotation),
                gs.selected_clip_keyframe_times(ClipKeyframeChannel::PosX),
                gs.selected_clip_keyframe_times(ClipKeyframeChannel::PosY),
                gs.selected_clip_keyframe_times(ClipKeyframeChannel::Brightness),
                gs.selected_clip_keyframe_times(ClipKeyframeChannel::Contrast),
                gs.selected_clip_keyframe_times(ClipKeyframeChannel::Saturation),
                gs.selected_clip_keyframe_times(ClipKeyframeChannel::Opacity),
                gs.selected_clip_keyframe_times(ClipKeyframeChannel::Blur),
                gs.selected_subtitle_id,
                gs.selected_subtitle_ids.clone(),
                gs.get_selected_subtitle_text(),
                subtitle_transform,
                gs.get_selected_subtitle_group_id(),
                subtitle_group_transform,
                gs.get_selected_subtitle_font(),
                gs.get_selected_subtitle_color_rgba()
                    .unwrap_or((255, 255, 255, 255)),
                gs.get_selected_subtitle_group_color_rgba()
                    .unwrap_or((255, 255, 255, 255)),
            )
        };
        self.active_local_mask_layer = active_local_mask_layer_idx;
        if !has_visual_clip_selection {
            self.selected_clip_keyframe_channel = None;
        }
        if !has_layer_effect_selection {
            self.layer_fx_script_modal_open = false;
            self.layer_fx_template_modal_open = false;
        }
        if self.global.read(cx).selected_semantic_clip_id().is_none() {
            self.semantic_schema_modal_open = false;
        }
        let can_add_mask_layer = local_mask_layer_count < MAX_LOCAL_MASK_LAYERS;

        let (scale_val, pos_x_val, pos_y_val) = transform_val;
        let fade_active = fade_in_val > 0.001 || fade_out_val > 0.001;
        let dissolve_active = dissolve_in_val > 0.001 || dissolve_out_val > 0.001;
        let show_transition = fade_active
            || dissolve_active
            || slide_in_val > 0.001
            || slide_out_val > 0.001
            || zoom_in_val > 0.001
            || zoom_out_val > 0.001
            || shock_in_val > 0.001
            || shock_out_val > 0.001;
        let slide_active = slide_in_val > 0.001 || slide_out_val > 0.001;
        let zoom_active = zoom_in_val > 0.001 || zoom_out_val > 0.001;
        let shock_active = shock_in_val > 0.001 || shock_out_val > 0.001;
        let layer_brightness_active = layer_brightness_enabled;
        let layer_contrast_active = layer_contrast_enabled;
        let layer_saturation_active = layer_saturation_enabled;
        let layer_blur_active = layer_blur_enabled;
        let any_layer_effect_active = layer_brightness_active
            || layer_contrast_active
            || layer_saturation_active
            || layer_blur_active;
        let (sub_pos_x_val, sub_pos_y_val, sub_size_val) = subtitle_transform_val;
        let (_, _, group_scale_val) = subtitle_group_transform_val;
        let subtitle_state = self.prepare_subtitle_render_state(
            subtitle_group_id,
            &selected_subtitle_ids,
            subtitle_transform_val,
            subtitle_group_transform_val,
            selected_subtitle_color_rgba,
            selected_subtitle_group_color_rgba,
        );
        let video_panel_state = VideoClipPanelState {
            t_h,
            t_s,
            t_l,
            t_a,
            brightness_val,
            contrast_val,
            vid_saturation_val,
            opacity_val,
            blur_sigma_val,
            local_mask_enabled,
            local_mask_center_x_val,
            local_mask_center_y_val,
            local_mask_radius_val,
            local_mask_feather_val,
            local_mask_strength_val,
            local_mask_brightness_val,
            local_mask_contrast_val,
            local_mask_saturation_val,
            local_mask_opacity_val,
            local_mask_blur_sigma_val,
            local_mask_layer_count,
            active_local_mask_layer_idx,
            can_add_mask_layer,
            fade_in_val,
            fade_out_val,
            dissolve_in_val,
            dissolve_out_val,
            slide_in_dir,
            slide_out_dir,
            slide_in_val,
            slide_out_val,
            zoom_in_val,
            zoom_out_val,
            zoom_amount_val,
            shock_in_val,
            shock_out_val,
            shock_amount_val,
            scale_val,
            pos_x_val,
            pos_y_val,
            rotation_val,
            show_transition,
            fade_active,
            dissolve_active,
            slide_active,
            zoom_active,
            shock_active,
            scale_keyframe_active,
            rotation_keyframe_active,
            pos_x_keyframe_active,
            pos_y_keyframe_active,
            brightness_keyframe_active,
            contrast_keyframe_active,
            saturation_keyframe_active,
            opacity_keyframe_active,
            blur_keyframe_active,
            selected_clip_duration,
            selected_clip_local_playhead,
        };

        // =========================================================
        // 🔥 Initialize sliders (lazy init)
        // =========================================================
        if self.subtitle_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .multi_line(true)
                    .rows(3)
                    .placeholder("Type subtitle…")
            });
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                if this.subtitle_editing_id.is_none() {
                    return;
                }
                let text = input.read(cx).value().to_string();
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_subtitle_text(text);
                    cx.notify();
                });
                cx.notify();
            });
            self.subtitle_input = Some(input);
            self.subtitle_input_sub = Some(sub);
        }

        self.ensure_semantic_render_inputs(window, cx);

        if self.subtitle_fonts.is_empty() {
            self.subtitle_fonts = Self::load_subtitle_fonts();
        }

        if self.subtitle_font_select.is_none() {
            let items = Self::build_font_items(&self.subtitle_fonts);
            let state = cx.new(|cx| SelectState::new(items, None, window, cx).searchable(true));
            let sub = cx.subscribe(
                &state,
                |this, _, ev: &SelectEvent<SearchableVec<SubtitleFont>>, cx| {
                    let SelectEvent::Confirm(value) = ev;
                    let Some(value) = value else {
                        return;
                    };
                    if value.is_empty() {
                        this.global.update(cx, |gs, cx| {
                            if this.subtitle_edit_mode == SubtitleEditMode::Group
                                && gs.get_selected_subtitle_group_id().is_some()
                            {
                                gs.set_selected_subtitle_group_font(None, None);
                            } else {
                                gs.set_selected_subtitle_font(None, None);
                            }
                            cx.notify();
                        });
                        cx.notify();
                        return;
                    }

                    if let Some(font) = this.subtitle_fonts.iter().find(|f| f.path == *value) {
                        let path = font.path.clone();
                        let family = font.family.clone();
                        this.global.update(cx, |gs, cx| {
                            if this.subtitle_edit_mode == SubtitleEditMode::Group
                                && gs.get_selected_subtitle_group_id().is_some()
                            {
                                gs.set_selected_subtitle_group_font(Some(path), Some(family));
                            } else {
                                gs.set_selected_subtitle_font(Some(path), Some(family));
                            }
                            cx.notify();
                        });
                        cx.notify();
                    }
                },
            );
            self.subtitle_font_select = Some(state);
            self.subtitle_font_select_sub = Some(sub);
        }

        if self.subtitle_color_hex_input.is_none() {
            let input =
                cx.new(|cx| InputState::new(window, cx).placeholder("#RRGGBB or #RRGGBBAA"));
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                let value = input.read(cx).value().to_string();
                let Some(rgba) = Self::parse_hex_rgba(&value) else {
                    return;
                };
                this.global.update(cx, |gs, cx| {
                    let use_group = this.subtitle_edit_mode == SubtitleEditMode::Group
                        && gs.get_selected_subtitle_group_id().is_some();
                    if use_group {
                        gs.set_selected_subtitle_group_color_rgba(rgba);
                    } else {
                        gs.set_selected_subtitle_color_rgba(rgba);
                    }
                    cx.notify();
                });
                cx.notify();
            });
            self.subtitle_color_hex_input = Some(input);
            self.subtitle_color_hex_sub = Some(sub);
        }

        if self.sub_color_hue_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(360.0)
                    .default_value(subtitle_state.sub_color_h)
                    .step(1.0)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(_) = ev;
                let h = this
                    .sub_color_hue_slider
                    .as_ref()
                    .map(|x| x.read(cx).value().start())
                    .unwrap_or(0.0);
                let s = this
                    .sub_color_sat_slider
                    .as_ref()
                    .map(|x| x.read(cx).value().start())
                    .unwrap_or(0.0);
                let l = this
                    .sub_color_lum_slider
                    .as_ref()
                    .map(|x| x.read(cx).value().start())
                    .unwrap_or(1.0);
                let a = this
                    .sub_color_alpha_slider
                    .as_ref()
                    .map(|x| x.read(cx).value().start())
                    .unwrap_or(1.0);
                let rgba = Self::hsla_to_rgba(h, s, l, a);
                this.global.update(cx, |gs, cx| {
                    let use_group = this.subtitle_edit_mode == SubtitleEditMode::Group
                        && gs.get_selected_subtitle_group_id().is_some();
                    if use_group {
                        gs.set_selected_subtitle_group_color_rgba(rgba);
                    } else {
                        gs.set_selected_subtitle_color_rgba(rgba);
                    }
                    cx.notify();
                });
                cx.notify();
            });
            self.sub_color_hue_slider = Some(s);
            self.sub_color_hue_sub = Some(sub);
        }
        if self.sub_color_sat_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(1.0)
                    .default_value(subtitle_state.sub_color_s)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(_) = ev;
                let h = this
                    .sub_color_hue_slider
                    .as_ref()
                    .map(|x| x.read(cx).value().start())
                    .unwrap_or(0.0);
                let s = this
                    .sub_color_sat_slider
                    .as_ref()
                    .map(|x| x.read(cx).value().start())
                    .unwrap_or(0.0);
                let l = this
                    .sub_color_lum_slider
                    .as_ref()
                    .map(|x| x.read(cx).value().start())
                    .unwrap_or(1.0);
                let a = this
                    .sub_color_alpha_slider
                    .as_ref()
                    .map(|x| x.read(cx).value().start())
                    .unwrap_or(1.0);
                let rgba = Self::hsla_to_rgba(h, s, l, a);
                this.global.update(cx, |gs, cx| {
                    let use_group = this.subtitle_edit_mode == SubtitleEditMode::Group
                        && gs.get_selected_subtitle_group_id().is_some();
                    if use_group {
                        gs.set_selected_subtitle_group_color_rgba(rgba);
                    } else {
                        gs.set_selected_subtitle_color_rgba(rgba);
                    }
                    cx.notify();
                });
                cx.notify();
            });
            self.sub_color_sat_slider = Some(s);
            self.sub_color_sat_sub = Some(sub);
        }
        if self.sub_color_lum_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(1.0)
                    .default_value(subtitle_state.sub_color_l)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(_) = ev;
                let h = this
                    .sub_color_hue_slider
                    .as_ref()
                    .map(|x| x.read(cx).value().start())
                    .unwrap_or(0.0);
                let s = this
                    .sub_color_sat_slider
                    .as_ref()
                    .map(|x| x.read(cx).value().start())
                    .unwrap_or(0.0);
                let l = this
                    .sub_color_lum_slider
                    .as_ref()
                    .map(|x| x.read(cx).value().start())
                    .unwrap_or(1.0);
                let a = this
                    .sub_color_alpha_slider
                    .as_ref()
                    .map(|x| x.read(cx).value().start())
                    .unwrap_or(1.0);
                let rgba = Self::hsla_to_rgba(h, s, l, a);
                this.global.update(cx, |gs, cx| {
                    let use_group = this.subtitle_edit_mode == SubtitleEditMode::Group
                        && gs.get_selected_subtitle_group_id().is_some();
                    if use_group {
                        gs.set_selected_subtitle_group_color_rgba(rgba);
                    } else {
                        gs.set_selected_subtitle_color_rgba(rgba);
                    }
                    cx.notify();
                });
                cx.notify();
            });
            self.sub_color_lum_slider = Some(s);
            self.sub_color_lum_sub = Some(sub);
        }
        if self.sub_color_alpha_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(1.0)
                    .default_value(subtitle_state.sub_color_a)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(_) = ev;
                let h = this
                    .sub_color_hue_slider
                    .as_ref()
                    .map(|x| x.read(cx).value().start())
                    .unwrap_or(0.0);
                let s = this
                    .sub_color_sat_slider
                    .as_ref()
                    .map(|x| x.read(cx).value().start())
                    .unwrap_or(0.0);
                let l = this
                    .sub_color_lum_slider
                    .as_ref()
                    .map(|x| x.read(cx).value().start())
                    .unwrap_or(1.0);
                let a = this
                    .sub_color_alpha_slider
                    .as_ref()
                    .map(|x| x.read(cx).value().start())
                    .unwrap_or(1.0);
                let rgba = Self::hsla_to_rgba(h, s, l, a);
                this.global.update(cx, |gs, cx| {
                    let use_group = this.subtitle_edit_mode == SubtitleEditMode::Group
                        && gs.get_selected_subtitle_group_id().is_some();
                    if use_group {
                        gs.set_selected_subtitle_group_color_rgba(rgba);
                    } else {
                        gs.set_selected_subtitle_color_rgba(rgba);
                    }
                    cx.notify();
                });
                cx.notify();
            });
            self.sub_color_alpha_slider = Some(s);
            self.sub_color_alpha_sub = Some(sub);
        }

        // --- 1. HSLA Overlay Sliders ---
        if self.hue_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(360.0)
                    .default_value(t_h)
                    .step(1.0)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_hsla_overlay_hue(val.start());
                    cx.notify();
                });
            });
            self.hue_slider = Some(s);
            self.hue_sub = Some(sub);
        }

        // --- Subtitle Transform Sliders ---
        if self.sub_pos_x_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(-1.0)
                    .max(1.0)
                    .default_value(sub_pos_x_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                let value = val.start();
                this.global.update(cx, |gs, cx| {
                    let use_group = this.subtitle_edit_mode == SubtitleEditMode::Group
                        && gs.get_selected_subtitle_group_id().is_some();
                    if use_group {
                        gs.set_selected_subtitle_group_pos_x(value);
                    } else {
                        gs.set_selected_subtitle_pos_x(value);
                    }
                    cx.notify();
                });
            });
            self.sub_pos_x_slider = Some(s);
            self.sub_pos_x_sub = Some(sub);
        }
        if self.sub_pos_y_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(-1.0)
                    .max(1.0)
                    .default_value(sub_pos_y_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                let value = val.start();
                this.global.update(cx, |gs, cx| {
                    let use_group = this.subtitle_edit_mode == SubtitleEditMode::Group
                        && gs.get_selected_subtitle_group_id().is_some();
                    if use_group {
                        gs.set_selected_subtitle_group_pos_y(value);
                    } else {
                        gs.set_selected_subtitle_pos_y(value);
                    }
                    cx.notify();
                });
            });
            self.sub_pos_y_slider = Some(s);
            self.sub_pos_y_sub = Some(sub);
        }
        if self.sub_size_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(8.0)
                    .max(256.0)
                    .default_value(sub_size_val)
                    .step(1.0)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                if this.subtitle_edit_mode == SubtitleEditMode::Group {
                    return;
                }
                let value = val.start();
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_subtitle_font_size(value);
                    cx.notify();
                });
            });
            self.sub_size_slider = Some(s);
            self.sub_size_sub = Some(sub);
        }
        if self.sub_group_size_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.25)
                    .max(4.0)
                    .default_value(group_scale_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                if this.subtitle_edit_mode != SubtitleEditMode::Group {
                    return;
                }
                let value = val.start();
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_subtitle_group_scale(value);
                    cx.notify();
                });
            });
            self.sub_group_size_slider = Some(s);
            self.sub_group_size_sub = Some(sub);
        }
        if self.sat_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(1.0)
                    .default_value(t_s)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_hsla_overlay_saturation(val.start());
                    cx.notify();
                });
            });
            self.sat_slider = Some(s);
            self.sat_sub = Some(sub);
        }
        if self.lum_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(1.0)
                    .default_value(t_l)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_hsla_overlay_lightness(val.start());
                    cx.notify();
                });
            });
            self.lum_slider = Some(s);
            self.lum_sub = Some(sub);
        }
        if self.alpha_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(1.0)
                    .default_value(t_a)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_hsla_overlay_alpha(val.start());
                    cx.notify();
                });
            });
            self.alpha_slider = Some(s);
            self.alpha_sub = Some(sub);
        }

        // --- 2. Transform Sliders ---
        if self.scale_slider.is_none() {
            // Scale: 0.0 ~ 5.0 (Default 1.0)
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(5.0)
                    .default_value(scale_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_scale(val.start());
                    cx.notify();
                });
            });
            self.scale_slider = Some(s);
            self.scale_sub = Some(sub);
        }
        if self.rotation_slider.is_none() {
            // Rotation: -180° ~ 180°
            let s = cx.new(|_| {
                SliderState::new()
                    .min(-180.0)
                    .max(180.0)
                    .default_value(rotation_val)
                    .step(0.1)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_rotation(val.start());
                    cx.notify();
                });
            });
            self.rotation_slider = Some(s);
            self.rotation_sub = Some(sub);
        }
        if self.pos_x_slider.is_none() {
            // Pos X: -1.0 ~ 1.0
            let s = cx.new(|_| {
                SliderState::new()
                    .min(-1.0)
                    .max(1.0)
                    .default_value(pos_x_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_pos_x(val.start());
                    cx.notify();
                });
            });
            self.pos_x_slider = Some(s);
            self.pos_x_sub = Some(sub);
        }
        if self.pos_y_slider.is_none() {
            // Pos Y: -1.0 ~ 1.0
            let s = cx.new(|_| {
                SliderState::new()
                    .min(-1.0)
                    .max(1.0)
                    .default_value(pos_y_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_pos_y(val.start());
                    cx.notify();
                });
            });
            self.pos_y_slider = Some(s);
            self.pos_y_sub = Some(sub);
        }

        // --- 3. Video Effect Sliders ---
        if self.bright_slider.is_none() {
            // Brightness: -1.0 ~ 1.0
            let s = cx.new(|_| {
                SliderState::new()
                    .min(-1.0)
                    .max(1.0)
                    .default_value(brightness_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_brightness(val.start());
                    cx.notify();
                });
            });
            self.bright_slider = Some(s);
            self.bright_sub = Some(sub);
        }
        if self.contrast_slider.is_none() {
            // Contrast: 0.0 ~ 2.0 (Default 1.0)
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(2.0)
                    .default_value(contrast_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_contrast(val.start());
                    cx.notify();
                });
            });
            self.contrast_slider = Some(s);
            self.contrast_sub = Some(sub);
        }
        if self.vid_sat_slider.is_none() {
            // Saturation: 0.0 ~ 2.0 (Default 1.0)
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(2.0)
                    .default_value(vid_saturation_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_saturation(val.start());
                    cx.notify();
                });
            });
            self.vid_sat_slider = Some(s);
            self.vid_sat_sub = Some(sub);
        }
        if self.opacity_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(1.0)
                    .default_value(opacity_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_opacity(val.start());
                    cx.notify();
                });
            });
            self.opacity_slider = Some(s);
            self.opacity_sub = Some(sub);
        }
        if self.blur_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(32.0)
                    .default_value(blur_sigma_val)
                    .step(0.1)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_blur_sigma(val.start());
                    cx.notify();
                });
            });
            self.blur_slider = Some(s);
            self.blur_sub = Some(sub);
        }
        if self.layer_brightness_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(-1.0)
                    .max(1.0)
                    .default_value(layer_brightness_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    let _ = gs.set_selected_layer_effect_brightness(val.start());
                    cx.notify();
                });
            });
            self.layer_brightness_slider = Some(s);
            self.layer_brightness_sub = Some(sub);
        }
        if self.layer_contrast_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(2.0)
                    .default_value(layer_contrast_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    let _ = gs.set_selected_layer_effect_contrast(val.start());
                    cx.notify();
                });
            });
            self.layer_contrast_slider = Some(s);
            self.layer_contrast_sub = Some(sub);
        }
        if self.layer_saturation_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(2.0)
                    .default_value(layer_saturation_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    let _ = gs.set_selected_layer_effect_saturation(val.start());
                    cx.notify();
                });
            });
            self.layer_saturation_slider = Some(s);
            self.layer_saturation_sub = Some(sub);
        }
        if self.layer_blur_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(64.0)
                    .default_value(layer_blur_sigma_val)
                    .step(0.1)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    let _ = gs.set_selected_layer_effect_blur(val.start());
                    cx.notify();
                });
            });
            self.layer_blur_slider = Some(s);
            self.layer_blur_sub = Some(sub);
        }
        if self.local_mask_center_x_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(1.0)
                    .default_value(local_mask_center_x_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_local_mask_center_x_at(
                        this.active_local_mask_layer,
                        val.start(),
                    );
                    cx.notify();
                });
            });
            self.local_mask_center_x_slider = Some(s);
            self.local_mask_center_x_sub = Some(sub);
        }
        if self.local_mask_center_y_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(1.0)
                    .default_value(local_mask_center_y_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_local_mask_center_y_at(
                        this.active_local_mask_layer,
                        val.start(),
                    );
                    cx.notify();
                });
            });
            self.local_mask_center_y_slider = Some(s);
            self.local_mask_center_y_sub = Some(sub);
        }
        if self.local_mask_radius_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(1.0)
                    .default_value(local_mask_radius_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_local_mask_radius_at(
                        this.active_local_mask_layer,
                        val.start(),
                    );
                    cx.notify();
                });
            });
            self.local_mask_radius_slider = Some(s);
            self.local_mask_radius_sub = Some(sub);
        }
        if self.local_mask_feather_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(1.0)
                    .default_value(local_mask_feather_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_local_mask_feather_at(
                        this.active_local_mask_layer,
                        val.start(),
                    );
                    cx.notify();
                });
            });
            self.local_mask_feather_slider = Some(s);
            self.local_mask_feather_sub = Some(sub);
        }
        if self.local_mask_strength_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(1.0)
                    .default_value(local_mask_strength_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_local_mask_strength_at(
                        this.active_local_mask_layer,
                        val.start(),
                    );
                    cx.notify();
                });
            });
            self.local_mask_strength_slider = Some(s);
            self.local_mask_strength_sub = Some(sub);
        }
        if self.local_mask_bright_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(-1.0)
                    .max(1.0)
                    .default_value(local_mask_brightness_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_local_mask_adjust_brightness_at(
                        this.active_local_mask_layer,
                        val.start(),
                    );
                    cx.notify();
                });
            });
            self.local_mask_bright_slider = Some(s);
            self.local_mask_bright_sub = Some(sub);
        }
        if self.local_mask_contrast_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(2.0)
                    .default_value(local_mask_contrast_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_local_mask_adjust_contrast_at(
                        this.active_local_mask_layer,
                        val.start(),
                    );
                    cx.notify();
                });
            });
            self.local_mask_contrast_slider = Some(s);
            self.local_mask_contrast_sub = Some(sub);
        }
        if self.local_mask_sat_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(2.0)
                    .default_value(local_mask_saturation_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_local_mask_adjust_saturation_at(
                        this.active_local_mask_layer,
                        val.start(),
                    );
                    cx.notify();
                });
            });
            self.local_mask_sat_slider = Some(s);
            self.local_mask_sat_sub = Some(sub);
        }
        if self.local_mask_opacity_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(1.0)
                    .default_value(local_mask_opacity_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_local_mask_adjust_opacity_at(
                        this.active_local_mask_layer,
                        val.start(),
                    );
                    cx.notify();
                });
            });
            self.local_mask_opacity_slider = Some(s);
            self.local_mask_opacity_sub = Some(sub);
        }
        if self.local_mask_blur_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(32.0)
                    .default_value(local_mask_blur_sigma_val)
                    .step(0.1)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_local_mask_adjust_blur_sigma_at(
                        this.active_local_mask_layer,
                        val.start(),
                    );
                    cx.notify();
                });
            });
            self.local_mask_blur_slider = Some(s);
            self.local_mask_blur_sub = Some(sub);
        }
        if self.fade_in_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(10.0)
                    .default_value(fade_in_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_fade_in(val.start());
                    cx.notify();
                });
            });
            self.fade_in_slider = Some(s);
            self.fade_in_sub = Some(sub);
        }
        if self.fade_out_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(10.0)
                    .default_value(fade_out_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_fade_out(val.start());
                    cx.notify();
                });
            });
            self.fade_out_slider = Some(s);
            self.fade_out_sub = Some(sub);
        }
        if self.dissolve_in_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(10.0)
                    .default_value(dissolve_in_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_dissolve_in(val.start());
                    cx.notify();
                });
            });
            self.dissolve_in_slider = Some(s);
            self.dissolve_in_sub = Some(sub);
        }
        if self.dissolve_out_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(10.0)
                    .default_value(dissolve_out_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_dissolve_out(val.start());
                    cx.notify();
                });
            });
            self.dissolve_out_slider = Some(s);
            self.dissolve_out_sub = Some(sub);
        }
        if self.slide_in_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(10.0)
                    .default_value(slide_in_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_slide_in(val.start());
                    cx.notify();
                });
            });
            self.slide_in_slider = Some(s);
            self.slide_in_sub = Some(sub);
        }
        if self.slide_out_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(10.0)
                    .default_value(slide_out_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_slide_out(val.start());
                    cx.notify();
                });
            });
            self.slide_out_slider = Some(s);
            self.slide_out_sub = Some(sub);
        }
        if self.zoom_in_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(10.0)
                    .default_value(zoom_in_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_zoom_in(val.start());
                    cx.notify();
                });
            });
            self.zoom_in_slider = Some(s);
            self.zoom_in_sub = Some(sub);
        }
        if self.zoom_out_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(10.0)
                    .default_value(zoom_out_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_zoom_out(val.start());
                    cx.notify();
                });
            });
            self.zoom_out_slider = Some(s);
            self.zoom_out_sub = Some(sub);
        }
        if self.zoom_amount_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.5)
                    .max(2.0)
                    .default_value(zoom_amount_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_zoom_amount(val.start());
                    cx.notify();
                });
            });
            self.zoom_amount_slider = Some(s);
            self.zoom_amount_sub = Some(sub);
        }
        if self.shock_in_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(10.0)
                    .default_value(shock_in_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_shock_zoom_in(val.start());
                    cx.notify();
                });
            });
            self.shock_in_slider = Some(s);
            self.shock_in_sub = Some(sub);
        }
        if self.shock_out_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(10.0)
                    .default_value(shock_out_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_shock_zoom_out(val.start());
                    cx.notify();
                });
            });
            self.shock_out_slider = Some(s);
            self.shock_out_sub = Some(sub);
        }
        if self.shock_amount_slider.is_none() {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.5)
                    .max(3.0)
                    .default_value(shock_amount_val)
                    .step(0.01)
            });
            let sub = cx.subscribe(&s, |this, _, ev, cx| {
                let SliderEvent::Change(val) = ev;
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_clip_shock_zoom_amount(val.start());
                    cx.notify();
                });
            });
            self.shock_amount_slider = Some(s);
            self.shock_amount_sub = Some(sub);
        }

        // Manual value input is committed on Enter or when another value field is opened.
        // Avoid blur-based auto-commit here; focus detection can race and close input instantly.
        self.sync_subtitle_inputs_with_selection(
            selected_subtitle_id,
            &selected_subtitle_text,
            &selected_subtitle_font,
            subtitle_state.subtitle_color_display,
            window,
            cx,
        );
        self.sync_semantic_inputs_with_selection(window, cx);

        // Retrieve all entities
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
        let layer_brightness_ent = self.layer_brightness_slider.as_ref().unwrap();
        let layer_contrast_ent = self.layer_contrast_slider.as_ref().unwrap();
        let layer_saturation_ent = self.layer_saturation_slider.as_ref().unwrap();
        let layer_blur_ent = self.layer_blur_slider.as_ref().unwrap();
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
        let sub_px_ent = self.sub_pos_x_slider.as_ref().unwrap();
        let sub_py_ent = self.sub_pos_y_slider.as_ref().unwrap();
        let sub_sz_ent = self.sub_size_slider.as_ref().unwrap();
        let sub_group_sz_ent = self.sub_group_size_slider.as_ref().unwrap();
        let sub_color_hue_ent = self.sub_color_hue_slider.as_ref().unwrap();
        let sub_color_sat_ent = self.sub_color_sat_slider.as_ref().unwrap();
        let sub_color_lum_ent = self.sub_color_lum_slider.as_ref().unwrap();
        let sub_color_alpha_ent = self.sub_color_alpha_slider.as_ref().unwrap();

        // =========================================================
        // 🔥 Sync sliders (global -> UI)
        // =========================================================

        // Helper to sync
        let mut update_if_needed =
            |ent: &Entity<SliderState>, target: f32, cx: &mut Context<Self>| {
                if (ent.read(cx).value().start() - target).abs() > 0.001 {
                    ent.update(cx, |s, cx| s.set_value(target, window, cx));
                }
            };

        // HSLA Overlay
        update_if_needed(hue_ent, t_h, cx);
        update_if_needed(sat_ent, t_s, cx);
        update_if_needed(lum_ent, t_l, cx);
        update_if_needed(alpha_ent, t_a, cx);
        // Transform
        update_if_needed(scale_ent, scale_val, cx);
        update_if_needed(rot_ent, rotation_val, cx);
        update_if_needed(px_ent, pos_x_val, cx);
        update_if_needed(py_ent, pos_y_val, cx);
        // Effects
        update_if_needed(br_ent, brightness_val, cx);
        update_if_needed(ct_ent, contrast_val, cx);
        update_if_needed(vs_ent, vid_saturation_val, cx);
        update_if_needed(op_ent, opacity_val, cx);
        update_if_needed(blur_ent, blur_sigma_val, cx);
        update_if_needed(layer_brightness_ent, layer_brightness_val, cx);
        update_if_needed(layer_contrast_ent, layer_contrast_val, cx);
        update_if_needed(layer_saturation_ent, layer_saturation_val, cx);
        update_if_needed(layer_blur_ent, layer_blur_sigma_val, cx);
        update_if_needed(mask_cx_ent, local_mask_center_x_val, cx);
        update_if_needed(mask_cy_ent, local_mask_center_y_val, cx);
        update_if_needed(mask_radius_ent, local_mask_radius_val, cx);
        update_if_needed(mask_feather_ent, local_mask_feather_val, cx);
        update_if_needed(mask_strength_ent, local_mask_strength_val, cx);
        update_if_needed(mask_bright_ent, local_mask_brightness_val, cx);
        update_if_needed(mask_contrast_ent, local_mask_contrast_val, cx);
        update_if_needed(mask_sat_ent, local_mask_saturation_val, cx);
        update_if_needed(mask_opacity_ent, local_mask_opacity_val, cx);
        update_if_needed(mask_blur_ent, local_mask_blur_sigma_val, cx);
        update_if_needed(fade_in_ent, fade_in_val, cx);
        update_if_needed(fade_out_ent, fade_out_val, cx);
        update_if_needed(dissolve_in_ent, dissolve_in_val, cx);
        update_if_needed(dissolve_out_ent, dissolve_out_val, cx);
        update_if_needed(slide_in_ent, slide_in_val, cx);
        update_if_needed(slide_out_ent, slide_out_val, cx);
        update_if_needed(zoom_in_ent, zoom_in_val, cx);
        update_if_needed(zoom_out_ent, zoom_out_val, cx);
        update_if_needed(zoom_amount_ent, zoom_amount_val, cx);
        update_if_needed(shock_in_ent, shock_in_val, cx);
        update_if_needed(shock_out_ent, shock_out_val, cx);
        update_if_needed(shock_amount_ent, shock_amount_val, cx);
        // Subtitle
        update_if_needed(sub_px_ent, subtitle_state.sub_pos_x_display, cx);
        update_if_needed(sub_py_ent, subtitle_state.sub_pos_y_display, cx);
        update_if_needed(sub_sz_ent, sub_size_val, cx);
        if subtitle_state.use_group_transform {
            update_if_needed(sub_group_sz_ent, group_scale_val, cx);
        }
        update_if_needed(sub_color_hue_ent, subtitle_state.sub_color_h, cx);
        update_if_needed(sub_color_sat_ent, subtitle_state.sub_color_s, cx);
        update_if_needed(sub_color_lum_ent, subtitle_state.sub_color_l, cx);
        update_if_needed(sub_color_alpha_ent, subtitle_state.sub_color_a, cx);

        // =========================================================
        // Build the UI
        // =========================================================
        let (inspector_panel_base_w, inspector_expanded) = {
            let gs = self.global.read(cx);
            (gs.inspector_panel_width(), gs.inspector_panel_expanded())
        };
        let inspector_panel_width = if inspector_expanded {
            let viewport_w = window.viewport_size().width / px(1.0);
            (viewport_w - 180.0).clamp(760.0, 1320.0)
        } else {
            inspector_panel_base_w
        };

        let container =
            div()
                .track_focus(&self.focus_handle)
                .relative()
                .w(px(inspector_panel_width))
                .h_full()
                .min_h_0()
                .flex_shrink_0()
                .bg(rgb(0x18181b))
                .border_l_1()
                .border_color(white().opacity(0.1))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, _cx| {
                        this.focus_handle.focus(window);
                    }),
                )
                .on_key_down(cx.listener(|this, evt: &KeyDownEvent, window, cx| {
                    let key = evt.keystroke.key.as_str();
                    if key != "backspace" && key != "delete" {
                        return;
                    }

                    let input_is_focused =
                        this.editing_slider.as_ref().is_some_and(|edit| {
                            edit.input.read(cx).focus_handle(cx).is_focused(window)
                        }) || this.subtitle_input.as_ref().is_some_and(|input| {
                            input.read(cx).focus_handle(cx).is_focused(window)
                        }) || this.subtitle_color_hex_input.as_ref().is_some_and(|input| {
                            input.read(cx).focus_handle(cx).is_focused(window)
                        }) || this.layer_fx_script_input.as_ref().is_some_and(|input| {
                            input.read(cx).focus_handle(cx).is_focused(window)
                        }) || this.semantic_type_input.as_ref().is_some_and(|input| {
                            input.read(cx).focus_handle(cx).is_focused(window)
                        }) || this.semantic_label_input.as_ref().is_some_and(|input| {
                            input.read(cx).focus_handle(cx).is_focused(window)
                        }) || this.semantic_prompt_input.as_ref().is_some_and(|input| {
                            input.read(cx).focus_handle(cx).is_focused(window)
                        }) || this
                            .semantic_image_api_key_input
                            .as_ref()
                            .is_some_and(|input| {
                                input.read(cx).focus_handle(cx).is_focused(window)
                            })
                            || this
                                .semantic_input_image_path_input
                                .as_ref()
                                .is_some_and(|input| {
                                    input.read(cx).focus_handle(cx).is_focused(window)
                                })
                            || this
                                .semantic_input_mask_path_input
                                .as_ref()
                                .is_some_and(|input| {
                                    input.read(cx).focus_handle(cx).is_focused(window)
                                })
                            || this
                                .semantic_output_width_input
                                .as_ref()
                                .is_some_and(|input| {
                                    input.read(cx).focus_handle(cx).is_focused(window)
                                })
                            || this
                                .semantic_output_height_input
                                .as_ref()
                                .is_some_and(|input| {
                                    input.read(cx).focus_handle(cx).is_focused(window)
                                })
                            || this.semantic_schema_input.as_ref().is_some_and(|input| {
                                input.read(cx).focus_handle(cx).is_focused(window)
                            });
                    if input_is_focused {
                        return;
                    }

                    let Some(channel) = this.selected_clip_keyframe_channel else {
                        return;
                    };
                    this.global.update(cx, |gs, cx| {
                        if gs.remove_selected_clip_keyframe_at_playhead(channel) {
                            cx.notify();
                        }
                    });
                    cx.notify();
                }))
                .flex()
                .flex_col()
                .px_4()
                .py_3()
                .gap_4()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(white().opacity(0.8))
                        .child("Inspector"),
                );

        // Slider row with keyframe button — takes a pre-built value element
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

        // Pre-build all editable value display elements so cx borrows don't
        // overlap with the keyframe_button closure which also borrows cx.
        let ev_layer_brightness = self.editable_value_display(
            "layer_brightness",
            format!("{:.2}", layer_brightness_val),
            -1.0,
            1.0,
            cx,
        );
        let ev_layer_contrast = self.editable_value_display(
            "layer_contrast",
            format!("{:.2}", layer_contrast_val),
            0.0,
            2.0,
            cx,
        );
        let ev_layer_saturation = self.editable_value_display(
            "layer_saturation",
            format!("{:.2}", layer_saturation_val),
            0.0,
            2.0,
            cx,
        );
        let ev_layer_blur = self.editable_value_display(
            "layer_blur",
            format!("{:.1}", layer_blur_sigma_val),
            0.0,
            64.0,
            cx,
        );
        let ev_sub_pos_x = self.editable_value_display(
            "sub_pos_x",
            format!("{:.2}", subtitle_state.sub_pos_x_display),
            -1.0,
            1.0,
            cx,
        );
        let ev_sub_pos_y = self.editable_value_display(
            "sub_pos_y",
            format!("{:.2}", subtitle_state.sub_pos_y_display),
            -1.0,
            1.0,
            cx,
        );
        let ev_sub_group_scale = self.editable_value_display(
            "sub_group_scale",
            format!("{:.2}", group_scale_val),
            0.25,
            4.0,
            cx,
        );
        let ev_sub_size =
            self.editable_value_display("sub_size", format!("{:.0}", sub_size_val), 8.0, 200.0, cx);
        let ev_sub_color_hue = self.editable_value_display(
            "sub_color_hue",
            format!("{:.0}", subtitle_state.sub_color_h),
            0.0,
            360.0,
            cx,
        );
        let ev_sub_color_sat = self.editable_value_display(
            "sub_color_sat",
            format!("{:.2}", subtitle_state.sub_color_s),
            0.0,
            1.0,
            cx,
        );
        let ev_sub_color_lum = self.editable_value_display(
            "sub_color_lum",
            format!("{:.2}", subtitle_state.sub_color_l),
            0.0,
            1.0,
            cx,
        );
        let ev_sub_color_alpha = self.editable_value_display(
            "sub_color_alpha",
            format!("{:.2}", subtitle_state.sub_color_a),
            0.0,
            1.0,
            cx,
        );

        let keyframe_button = |active: bool,
                               handler: fn(
            &mut Self,
            &gpui::MouseDownEvent,
            &mut Window,
            &mut Context<Self>,
        )| {
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
                .on_mouse_down(MouseButton::Left, cx.listener(handler))
        };

        let remove_button =
            |handler: fn(&mut Self, &gpui::MouseDownEvent, &mut Window, &mut Context<Self>)| {
                div()
                    .w(px(18.0))
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
                    .child("X")
                    .on_mouse_down(MouseButton::Left, cx.listener(handler))
            };

        {
            // Keep inspector controls scrollable when transition sections make the panel taller.
            let mut panel_body = div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .gap_6()
                .overflow_y_scrollbar();

            if has_layer_effect_selection {
                let mut layer_rows = div().flex().flex_col().gap_2();
                if any_layer_effect_active {
                    if layer_brightness_active {
                        layer_rows = layer_rows.child(slider_row_keyframe(
                            "Brightness",
                            layer_brightness_ent,
                            ev_layer_brightness,
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(keyframe_button(
                                    layer_brightness_keyframe_active,
                                    |this, _, _, cx| {
                                        this.global.update(cx, |gs, cx| {
                                            let _ =
                                                gs.add_selected_layer_effect_brightness_keyframe();
                                            cx.notify();
                                        });
                                        cx.notify();
                                    },
                                ))
                                .child(remove_button(|this, _, _, cx| {
                                    this.global.update(cx, |gs, cx| {
                                        let _ = gs.remove_selected_layer_effect_brightness();
                                        cx.notify();
                                    });
                                    cx.notify();
                                })),
                        ));
                    }
                    if layer_contrast_active {
                        layer_rows = layer_rows.child(slider_row_keyframe(
                            "Contrast",
                            layer_contrast_ent,
                            ev_layer_contrast,
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(keyframe_button(
                                    layer_contrast_keyframe_active,
                                    |this, _, _, cx| {
                                        this.global.update(cx, |gs, cx| {
                                            let _ =
                                                gs.add_selected_layer_effect_contrast_keyframe();
                                            cx.notify();
                                        });
                                        cx.notify();
                                    },
                                ))
                                .child(remove_button(|this, _, _, cx| {
                                    this.global.update(cx, |gs, cx| {
                                        let _ = gs.remove_selected_layer_effect_contrast();
                                        cx.notify();
                                    });
                                    cx.notify();
                                })),
                        ));
                    }
                    if layer_saturation_active {
                        layer_rows = layer_rows.child(slider_row_keyframe(
                            "Saturation",
                            layer_saturation_ent,
                            ev_layer_saturation,
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(keyframe_button(
                                    layer_saturation_keyframe_active,
                                    |this, _, _, cx| {
                                        this.global.update(cx, |gs, cx| {
                                            let _ =
                                                gs.add_selected_layer_effect_saturation_keyframe();
                                            cx.notify();
                                        });
                                        cx.notify();
                                    },
                                ))
                                .child(remove_button(|this, _, _, cx| {
                                    this.global.update(cx, |gs, cx| {
                                        let _ = gs.remove_selected_layer_effect_saturation();
                                        cx.notify();
                                    });
                                    cx.notify();
                                })),
                        ));
                    }
                    if layer_blur_active {
                        layer_rows = layer_rows.child(slider_row_keyframe(
                            "Blur",
                            layer_blur_ent,
                            ev_layer_blur,
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(keyframe_button(
                                    layer_blur_keyframe_active,
                                    |this, _, _, cx| {
                                        this.global.update(cx, |gs, cx| {
                                            let _ = gs.add_selected_layer_effect_blur_keyframe();
                                            cx.notify();
                                        });
                                        cx.notify();
                                    },
                                ))
                                .child(remove_button(|this, _, _, cx| {
                                    this.global.update(cx, |gs, cx| {
                                        let _ = gs.remove_selected_layer_effect_blur();
                                        cx.notify();
                                    });
                                    cx.notify();
                                })),
                        ));
                    }
                }

                let layer_effect_rows = layer_rows.into_any_element();
                let layer_fx_script_input_elem = if self.layer_fx_template_modal_open {
                    div()
                        .w_full()
                        .h(px(190.0))
                        .rounded_sm()
                        .border_1()
                        .border_color(white().opacity(0.16))
                        .bg(rgb(0x0b1020))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .text_xs()
                                .text_color(white().opacity(0.7))
                                .child("Template picker opened in modal."),
                        )
                        .into_any_element()
                } else if self.layer_fx_script_modal_open {
                    div()
                        .w_full()
                        .h(px(190.0))
                        .rounded_sm()
                        .border_1()
                        .border_color(white().opacity(0.16))
                        .bg(rgb(0x0b1020))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .text_xs()
                                .text_color(white().opacity(0.7))
                                .child("Editor opened in modal."),
                        )
                        .into_any_element()
                } else if let Some(input) = self.layer_fx_script_input.as_ref() {
                    div()
                        .w_full()
                        .h(px(190.0))
                        .rounded_sm()
                        .border_1()
                        .border_color(white().opacity(0.16))
                        .bg(rgb(0x0b1020))
                        .overflow_hidden()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|_, _, _, cx| {
                                cx.stop_propagation();
                            }),
                        )
                        .child(Input::new(input).h_full().w_full())
                        .into_any_element()
                } else {
                    div()
                        .w_full()
                        .h(px(190.0))
                        .rounded_sm()
                        .bg(white().opacity(0.05))
                        .into_any_element()
                };
                let layer_fx_script_controls = div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .justify_start()
                    .gap_2()
                    .child(
                        div()
                            .h(px(26.0))
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
                            .h(px(26.0))
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
                            .h(px(26.0))
                            .px_2()
                            .rounded_sm()
                            .border_1()
                            .border_color(white().opacity(0.2))
                            .bg(white().opacity(0.06))
                            .text_xs()
                            .text_color(white().opacity(0.82))
                            .cursor_pointer()
                            .child("Expand")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.layer_fx_script_modal_open = true;
                                    cx.notify();
                                }),
                            ),
                    )
                    .into_any_element();

                panel_body = panel_body.child(
                    div()
                        .border_1()
                        .border_color(white().opacity(0.1))
                        .rounded_md()
                        .p_2()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div().flex().items_center().justify_start().child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.5))
                                    .child("LAYER EFFECTS"),
                            ),
                        )
                        .child(layer_effect_rows)
                        .child(div().h(px(1.0)).bg(white().opacity(0.08)))
                        .child(Self::layer_fx_script_editor_wrap(
                            "MOTIONLOOM SCRIPT (for selected Layer FX)",
                            layer_fx_script_input_elem,
                            layer_fx_script_controls,
                            self.layer_fx_script_status.clone(),
                        ))
                        .child(layer_fx_curve_lanes_elem),
                );
            }

            let selected_semantic_id = self.global.read(cx).selected_semantic_clip_id;
            let has_semantic_selection = selected_semantic_id.is_some();

            if !has_timeline_clip_selection
                && !has_subtitle_selection
                && !has_layer_effect_selection
                && !has_semantic_selection
            {
                panel_body = panel_body.child(
                    div()
                        .text_sm()
                        .text_color(white().opacity(0.45))
                        .child("No Clip Selected"),
                );
            }

            if let Some(semantic_panel) = self.render_semantic_editor_panel(
                has_timeline_clip_selection,
                has_subtitle_selection,
                selected_semantic_id,
                cx,
            ) {
                panel_body = panel_body.child(semantic_panel);
            }

            if let Some(subtitle_panel) = self.render_subtitle_editor_panel(
                has_subtitle_selection,
                &subtitle_state,
                sub_px_ent,
                sub_py_ent,
                sub_group_sz_ent,
                sub_sz_ent,
                sub_color_hue_ent,
                sub_color_sat_ent,
                sub_color_lum_ent,
                sub_color_alpha_ent,
                ev_sub_pos_x,
                ev_sub_pos_y,
                ev_sub_group_scale,
                ev_sub_size,
                ev_sub_color_hue,
                ev_sub_color_sat,
                ev_sub_color_lum,
                ev_sub_color_alpha,
                cx,
            ) {
                panel_body = panel_body.child(subtitle_panel);
            }

            if let Some(audio_panel) = self.render_audio_clip_editor_panel(
                has_audio_clip_selection,
                has_subtitle_selection,
                window,
                cx,
            ) {
                panel_body = panel_body.child(audio_panel);
            }

            if let Some(video_panel) = self.render_video_clip_editor_panel(
                has_visual_clip_selection,
                has_subtitle_selection,
                &video_panel_state,
                &scale_keyframe_times,
                &rotation_keyframe_times,
                &pos_x_keyframe_times,
                &pos_y_keyframe_times,
                &brightness_keyframe_times,
                &contrast_keyframe_times,
                &saturation_keyframe_times,
                &opacity_keyframe_times,
                &blur_keyframe_times,
                cx,
            ) {
                panel_body = panel_body.child(video_panel);
            }

            panel_body = panel_body.child(div().h(px(64.0)));
            container.child(panel_body)
        }
    }
}
