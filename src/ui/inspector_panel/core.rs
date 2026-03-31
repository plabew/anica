use super::*;

impl InspectorPanel {
    pub(super) fn hash_f32(value: f32, hasher: &mut std::collections::hash_map::DefaultHasher) {
        value.to_bits().hash(hasher);
    }

    pub(super) fn state_signature(gs: &GlobalState) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        gs.selected_clip_id.hash(&mut hasher);
        gs.selected_subtitle_id.hash(&mut hasher);
        gs.selected_layer_effect_clip_id.hash(&mut hasher);
        gs.selected_clip_ids.hash(&mut hasher);
        gs.selected_subtitle_ids.hash(&mut hasher);
        gs.timeline_edit_token().hash(&mut hasher);
        gs.active_local_mask_layer().hash(&mut hasher);

        Self::hash_f32(
            gs.get_selected_clip_brightness().unwrap_or(0.0),
            &mut hasher,
        );
        Self::hash_f32(gs.get_selected_clip_contrast().unwrap_or(1.0), &mut hasher);
        Self::hash_f32(
            gs.get_selected_clip_saturation().unwrap_or(1.0),
            &mut hasher,
        );
        Self::hash_f32(gs.get_selected_clip_opacity().unwrap_or(1.0), &mut hasher);
        Self::hash_f32(
            gs.get_selected_clip_blur_sigma().unwrap_or(0.0),
            &mut hasher,
        );
        Self::hash_f32(gs.get_selected_clip_rotation().unwrap_or(0.0), &mut hasher);
        if let Some(audio_gain_db) = gs.get_selected_audio_clip_gain_db() {
            Self::hash_f32(audio_gain_db, &mut hasher);
        }

        let (scale, pos_x, pos_y) = gs.get_selected_clip_transform().unwrap_or((0.8, 0.0, 0.0));
        Self::hash_f32(scale, &mut hasher);
        Self::hash_f32(pos_x, &mut hasher);
        Self::hash_f32(pos_y, &mut hasher);

        Self::hash_f32(
            gs.get_selected_layer_effect_brightness().unwrap_or(0.0),
            &mut hasher,
        );
        Self::hash_f32(
            gs.get_selected_layer_effect_contrast().unwrap_or(1.0),
            &mut hasher,
        );
        Self::hash_f32(
            gs.get_selected_layer_effect_saturation().unwrap_or(1.0),
            &mut hasher,
        );
        Self::hash_f32(
            gs.get_selected_layer_effect_blur().unwrap_or(0.0),
            &mut hasher,
        );

        if let Some(layer_count) = gs.get_selected_clip_local_mask_layer_count() {
            layer_count.hash(&mut hasher);
        }
        if let Some((enabled, cx, cy, radius, feather, strength)) =
            gs.get_selected_clip_local_mask_layer(gs.active_local_mask_layer())
        {
            enabled.hash(&mut hasher);
            Self::hash_f32(cx, &mut hasher);
            Self::hash_f32(cy, &mut hasher);
            Self::hash_f32(radius, &mut hasher);
            Self::hash_f32(feather, &mut hasher);
            Self::hash_f32(strength, &mut hasher);
        }
        if let Some((brightness, contrast, saturation, opacity, blur_sigma)) =
            gs.get_selected_clip_local_mask_adjust_layer(gs.active_local_mask_layer())
        {
            Self::hash_f32(brightness, &mut hasher);
            Self::hash_f32(contrast, &mut hasher);
            Self::hash_f32(saturation, &mut hasher);
            Self::hash_f32(opacity, &mut hasher);
            Self::hash_f32(blur_sigma, &mut hasher);
        }

        hasher.finish()
    }

    pub fn new(global: Entity<GlobalState>, cx: &mut Context<Self>) -> Self {
        let state_sig = {
            let gs = global.read(cx);
            Self::state_signature(gs)
        };
        cx.observe(&global, |this, global, cx| {
            let sig = {
                let gs = global.read(cx);
                Self::state_signature(gs)
            };
            if sig != this.state_sig {
                this.state_sig = sig;
                cx.notify();
            }
        })
        .detach();

        Self {
            global,
            focus_handle: cx.focus_handle(),
            subtitle_editing_id: None,
            subtitle_input: None,
            subtitle_input_sub: None,
            sub_pos_x_slider: None,
            sub_pos_x_sub: None,
            sub_pos_y_slider: None,
            sub_pos_y_sub: None,
            sub_size_slider: None,
            sub_size_sub: None,
            sub_group_size_slider: None,
            sub_group_size_sub: None,
            subtitle_fonts: Vec::new(),
            subtitle_font_select: None,
            subtitle_font_select_sub: None,
            subtitle_color_hex_input: None,
            subtitle_color_hex_sub: None,
            subtitle_edit_mode: SubtitleEditMode::Individual,
            active_local_mask_layer: 0,
            sub_color_hue_slider: None,
            sub_color_hue_sub: None,
            sub_color_sat_slider: None,
            sub_color_sat_sub: None,
            sub_color_lum_slider: None,
            sub_color_lum_sub: None,
            sub_color_alpha_slider: None,
            sub_color_alpha_sub: None,
            audio_clip_gain_slider: None,
            audio_clip_gain_sub: None,
            // Semantic clip label editing
            semantic_editing_id: None,
            semantic_type_input: None,
            semantic_type_input_sub: None,
            semantic_label_input: None,
            semantic_label_input_sub: None,
            semantic_prompt_input: None,
            semantic_prompt_input_sub: None,
            semantic_image_api_key: String::new(),
            semantic_image_api_key_placeholder: String::new(),
            semantic_image_api_key_input: None,
            semantic_image_api_key_input_sub: None,
            semantic_input_image_path: String::new(),
            semantic_input_image_path_input: None,
            semantic_input_image_path_input_sub: None,
            semantic_input_mask_path: String::new(),
            semantic_input_mask_path_input: None,
            semantic_input_mask_path_input_sub: None,
            semantic_output_width: String::new(),
            semantic_output_width_input: None,
            semantic_output_width_input_sub: None,
            semantic_output_height: String::new(),
            semantic_output_height_input: None,
            semantic_output_height_input_sub: None,
            semantic_resolution_select: None,
            semantic_resolution_select_sub: None,
            semantic_resolution_select_sig: String::new(),
            semantic_selected_resolution: String::new(),
            semantic_resolution_apply_pending: false,
            semantic_mask_painter: MaskPainterState::new(),
            semantic_generate_status: "Image generation idle.".to_string(),
            semantic_schema_input: None,
            semantic_schema_input_sub: None,
            semantic_schema_text: String::new(),
            semantic_schema_clip_id: None,
            semantic_schema_mode: "video".to_string(),
            semantic_schema_status: "Semantic schema idle.".to_string(),
            semantic_schema_modal_open: false,
            // Initialize everything to None
            hue_slider: None,
            hue_sub: None,
            sat_slider: None,
            sat_sub: None,
            lum_slider: None,
            lum_sub: None,
            alpha_slider: None,
            alpha_sub: None,

            scale_slider: None,
            scale_sub: None,
            rotation_slider: None,
            rotation_sub: None,
            pos_x_slider: None,
            pos_x_sub: None,
            pos_y_slider: None,
            pos_y_sub: None,

            bright_slider: None,
            bright_sub: None,
            contrast_slider: None,
            contrast_sub: None,
            vid_sat_slider: None,
            vid_sat_sub: None,
            opacity_slider: None,
            opacity_sub: None,
            blur_slider: None,
            blur_sub: None,
            layer_brightness_slider: None,
            layer_brightness_sub: None,
            layer_contrast_slider: None,
            layer_contrast_sub: None,
            layer_saturation_slider: None,
            layer_saturation_sub: None,
            layer_blur_slider: None,
            layer_blur_sub: None,
            local_mask_center_x_slider: None,
            local_mask_center_x_sub: None,
            local_mask_center_y_slider: None,
            local_mask_center_y_sub: None,
            local_mask_radius_slider: None,
            local_mask_radius_sub: None,
            local_mask_feather_slider: None,
            local_mask_feather_sub: None,
            local_mask_strength_slider: None,
            local_mask_strength_sub: None,
            local_mask_bright_slider: None,
            local_mask_bright_sub: None,
            local_mask_contrast_slider: None,
            local_mask_contrast_sub: None,
            local_mask_sat_slider: None,
            local_mask_sat_sub: None,
            local_mask_opacity_slider: None,
            local_mask_opacity_sub: None,
            local_mask_blur_slider: None,
            local_mask_blur_sub: None,
            fade_in_slider: None,
            fade_in_sub: None,
            fade_out_slider: None,
            fade_out_sub: None,
            dissolve_in_slider: None,
            dissolve_in_sub: None,
            dissolve_out_slider: None,
            dissolve_out_sub: None,
            slide_in_slider: None,
            slide_in_sub: None,
            slide_out_slider: None,
            slide_out_sub: None,
            zoom_in_slider: None,
            zoom_in_sub: None,
            zoom_out_slider: None,
            zoom_out_sub: None,
            zoom_amount_slider: None,
            zoom_amount_sub: None,
            shock_in_slider: None,
            shock_in_sub: None,
            shock_out_slider: None,
            shock_out_sub: None,
            shock_amount_slider: None,
            shock_amount_sub: None,
            layer_fx_script_input: None,
            layer_fx_script_input_sub: None,
            layer_fx_script_text: String::new(),
            layer_fx_script_layer_id: None,
            layer_fx_script_status: "Layer FX script idle.".to_string(),
            layer_fx_script_modal_open: false,
            layer_fx_template_modal_open: false,
            layer_fx_template_add_time_parameter: false,
            layer_fx_template_add_curve_parameter: false,
            layer_fx_template_selected: Vec::new(),
            layer_fx_curve_editors: Vec::new(),
            layer_fx_curve_drag: None,
            layer_fx_curve_open_menu: None,
            editing_slider: None,
            selected_clip_keyframe_channel: None,
            state_sig,
        }
    }

    pub(super) fn start_editing_slider(
        &mut self,
        key: &str,
        current_val: &str,
        min: f32,
        max: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_editing_slider(cx);

        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_value(current_val.to_string(), window, cx);
            s
        });
        input.read(cx).focus_handle(cx).focus(window);

        let sub = cx.subscribe(&input, |this: &mut Self, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.commit_editing_slider(cx);
            }
        });

        let key_owned = key.to_string();
        self.editing_slider = Some(EditingSliderInfo {
            key: key_owned.clone(),
            input,
            _input_sub: sub,
            min,
            max,
        });
        cx.on_next_frame(window, move |this, window, cx| {
            if let Some(edit) = this.editing_slider.as_ref()
                && edit.key == key_owned
            {
                edit.input.read(cx).focus_handle(cx).focus(window);
            }
        });
        cx.notify();
    }

    pub(super) fn commit_editing_slider(&mut self, cx: &mut Context<Self>) {
        let Some(info) = self.editing_slider.take() else {
            return;
        };
        let text = info.input.read(cx).value().to_string();
        let stripped = text
            .trim_end_matches('\u{00B0}')
            .trim_end_matches('s')
            .trim_end_matches('x')
            .trim()
            .to_string();
        let Ok(val) = stripped.parse::<f32>() else {
            cx.notify();
            return;
        };

        let clamped = val.clamp(info.min, info.max);
        let key = info.key;
        let mask_layer = self.active_local_mask_layer;

        self.global.update(cx, |gs, cx| {
            let handled_video =
                Self::apply_video_slider_value(gs, key.as_str(), clamped, mask_layer);
            let handled_subtitle = Self::apply_subtitle_slider_value(gs, key.as_str(), clamped);
            let handled_audio = Self::apply_audio_slider_value(gs, key.as_str(), clamped);
            if handled_video || handled_subtitle || handled_audio {
                cx.notify();
            }
        });
        cx.notify();
    }

    pub(super) fn editable_value_display(
        &self,
        key: &str,
        val_str: String,
        min: f32,
        max: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let is_editing = self.editing_slider.as_ref().is_some_and(|e| e.key == key);

        if is_editing {
            let input = self.editing_slider.as_ref().unwrap().input.clone();
            let input_for_focus = input.clone();
            div()
                .w(px(72.0))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |_, _, window, cx| {
                        cx.stop_propagation();
                        input_for_focus.read(cx).focus_handle(cx).focus(window);
                    }),
                )
                .child(Input::new(&input).h(px(20.0)))
                .into_any_element()
        } else {
            let key_owned = key.to_string();
            let val_for_edit = val_str
                .trim_end_matches('\u{00B0}')
                .trim_end_matches('s')
                .trim_end_matches('x')
                .to_string();
            div()
                .w(px(72.0))
                .flex()
                .justify_end()
                .text_sm()
                .text_color(white().opacity(0.8))
                .cursor_pointer()
                .child(val_str)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.start_editing_slider(&key_owned, &val_for_edit, min, max, window, cx);
                    }),
                )
                .into_any_element()
        }
    }
}
