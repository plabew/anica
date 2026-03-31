use super::*;

const AUDIO_CLIP_GAIN_MIN_DB: f32 = -60.0;
const AUDIO_CLIP_GAIN_MAX_DB: f32 = 12.0;

impl InspectorPanel {
    pub(super) fn selected_clip_is_audio(gs: &GlobalState) -> bool {
        let Some(id) = gs.selected_clip_id else {
            return false;
        };
        gs.audio_tracks
            .iter()
            .any(|track| track.clips.iter().any(|clip| clip.id == id))
    }

    pub(super) fn apply_audio_slider_value(gs: &mut GlobalState, key: &str, clamped: f32) -> bool {
        match key {
            "clip_audio_gain" => gs.set_selected_audio_clip_gain_db(clamped),
            _ => false,
        }
    }

    fn ensure_audio_clip_gain_slider(
        &mut self,
        gain_db: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.audio_clip_gain_slider.is_none() {
            let slider = cx.new(|_| {
                SliderState::new()
                    .min(AUDIO_CLIP_GAIN_MIN_DB)
                    .max(AUDIO_CLIP_GAIN_MAX_DB)
                    .default_value(gain_db)
                    .step(0.1)
            });
            let sub = cx.subscribe(&slider, |this, _, ev, cx| {
                let SliderEvent::Change(value) = ev;
                this.global.update(cx, |gs, cx| {
                    if gs.set_selected_audio_clip_gain_db(value.start()) {
                        cx.notify();
                    }
                });
                cx.notify();
            });
            self.audio_clip_gain_slider = Some(slider);
            self.audio_clip_gain_sub = Some(sub);
            return;
        }

        if let Some(slider) = self.audio_clip_gain_slider.as_ref()
            && (slider.read(cx).value().start() - gain_db).abs() > 0.01
        {
            slider.update(cx, |state, cx| state.set_value(gain_db, window, cx));
        }
    }

    pub(super) fn render_audio_clip_editor_panel(
        &mut self,
        has_audio_clip_selection: bool,
        has_subtitle_selection: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !has_audio_clip_selection || has_subtitle_selection {
            return None;
        }

        let gain_db = self
            .global
            .read(cx)
            .get_selected_audio_clip_gain_db()
            .unwrap_or(0.0)
            .clamp(AUDIO_CLIP_GAIN_MIN_DB, AUDIO_CLIP_GAIN_MAX_DB);
        self.ensure_audio_clip_gain_slider(gain_db, window, cx);
        let gain_slider = self.audio_clip_gain_slider.as_ref()?;
        let gain_value = self.editable_value_display(
            "clip_audio_gain",
            format!("{:+.1} dB", gain_db),
            AUDIO_CLIP_GAIN_MIN_DB,
            AUDIO_CLIP_GAIN_MAX_DB,
            cx,
        );

        Some(
            div()
                .border_1()
                .border_color(white().opacity(0.1))
                .rounded_md()
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(0.5))
                        .child("AUDIO CLIP"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .text_color(white().opacity(0.82))
                                .child("Clip Gain"),
                        )
                        .child(
                            div().flex().items_center().gap_2().child(gain_value).child(
                                div()
                                    .h(px(24.0))
                                    .px_2()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(white().opacity(0.2))
                                    .bg(white().opacity(0.06))
                                    .text_xs()
                                    .text_color(white().opacity(0.86))
                                    .cursor_pointer()
                                    .child("Reset 0 dB")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.global.update(cx, |gs, cx| {
                                                if gs.set_selected_audio_clip_gain_db(0.0) {
                                                    cx.notify();
                                                }
                                            });
                                            cx.notify();
                                        }),
                                    ),
                            ),
                        ),
                )
                .child(Slider::new(gain_slider).w_full())
                .into_any_element(),
        )
    }
}
