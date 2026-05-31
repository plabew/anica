// =========================================
// =========================================
// src/ui/transition_select.rs
use gpui::{Context, Entity, IntoElement, MouseButton, Render, Window, div, prelude::*, px, rgb};
use gpui_component::{scroll::ScrollableElement, white};

use crate::core::global_state::{GlobalState, TransitionType};

pub struct TransitionSelect {
    pub global: Entity<GlobalState>,
}

impl TransitionSelect {
    pub fn new(global: Entity<GlobalState>, cx: &mut Context<Self>) -> Self {
        // Re-render this panel whenever transition-related global state changes.
        cx.observe(&global, |_, _, cx| {
            cx.notify();
        })
        .detach();

        Self { global }
    }

    fn list_card(title: &'static str, desc: &'static str, color: u32, active: bool) -> gpui::Div {
        div()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(if active { 0.35 } else { 0.12 }))
            .bg(white().opacity(if active { 0.08 } else { 0.03 }))
            .p_2()
            .flex()
            .items_center()
            .gap_2()
            .hover(|s| s.bg(white().opacity(0.06)))
            .cursor_pointer()
            .child(div().w(px(36.0)).h(px(24.0)).rounded_sm().bg(rgb(color)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.9))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.55))
                            .child(desc),
                    ),
            )
    }

    // Toggle selection state for drag-apply transition mode.
    fn toggle_transition(
        global: &Entity<GlobalState>,
        transition: TransitionType,
        cx: &mut Context<Self>,
    ) {
        global.update(cx, |gs, cx| {
            if gs.pending_transition == Some(transition) {
                gs.clear_transition_drag();
            } else {
                gs.begin_transition_drag(transition);
            }
            cx.notify();
        });
    }
}

impl Render for TransitionSelect {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pending_transition = {
            let gs = self.global.read(cx);
            gs.pending_transition
        };

        let fade_active = matches!(pending_transition, Some(TransitionType::Fade));
        let slide_active = matches!(pending_transition, Some(TransitionType::Slide));
        let zoom_active = matches!(pending_transition, Some(TransitionType::Zoom));
        let shock_zoom_active = matches!(pending_transition, Some(TransitionType::ShockZoom));

        let global_for_fade = self.global.clone();
        let global_for_slide = self.global.clone();
        let global_for_zoom = self.global.clone();
        let global_for_shock_zoom = self.global.clone();

        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .px_3()
            .py_2()
            .overflow_y_scrollbar()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(white().opacity(0.9))
                    .child("Transitions"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        Self::list_card("Fade", "Basic crossfade", 0x2dd4bf, fade_active)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |_this, _, _, cx| {
                                    Self::toggle_transition(
                                        &global_for_fade,
                                        TransitionType::Fade,
                                        cx,
                                    );
                                }),
                            ),
                    )
                    .child(
                        Self::list_card("Slide", "Directional", 0xf472b6, slide_active)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |_this, _, _, cx| {
                                    Self::toggle_transition(
                                        &global_for_slide,
                                        TransitionType::Slide,
                                        cx,
                                    );
                                }),
                            ),
                    )
                    .child(
                        Self::list_card("Zoom", "Push zoom", 0xf59e0b, zoom_active).on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |_this, _, _, cx| {
                                Self::toggle_transition(&global_for_zoom, TransitionType::Zoom, cx);
                            }),
                        ),
                    )
                    .child(
                        Self::list_card("Shock Zoom", "Punch zoom", 0x22c55e, shock_zoom_active)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |_this, _, _, cx| {
                                    Self::toggle_transition(
                                        &global_for_shock_zoom,
                                        TransitionType::ShockZoom,
                                        cx,
                                    );
                                }),
                            ),
                    ),
            )
    }
}
