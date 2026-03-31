// =========================================
// =========================================
// src/ui/editor_shell.rs
use gpui::{Context, Entity, IntoElement, Render, Window, div, prelude::*, px};
use gpui_component::{scroll::ScrollableElement, white};

use crate::core::global_state::GlobalState;
use crate::ui::media_pool_select::MediaPoolSelect;
use crate::ui::transition_select::TransitionSelect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorShellTab {
    MediaPool,
    Transitions,
}

pub struct EditorShell {
    pub media_pool_select: Entity<MediaPoolSelect>,
    pub transition_select: Entity<TransitionSelect>,
    active_tab: EditorShellTab,
}

impl EditorShell {
    pub fn new(global: Entity<GlobalState>, cx: &mut Context<Self>) -> Self {
        let global_for_transition_select = global.clone();
        let global_for_media_pool_select = global.clone();
        let transition_select =
            cx.new(move |cx| TransitionSelect::new(global_for_transition_select.clone(), cx));
        let media_pool_select =
            cx.new(move |cx| MediaPoolSelect::new(global_for_media_pool_select.clone(), cx));

        Self {
            media_pool_select,
            transition_select,
            active_tab: EditorShellTab::MediaPool,
        }
    }

    pub fn render_media_pool_expand_modal_overlay(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        self.media_pool_select.update(cx, |panel, cx| {
            panel.render_expand_modal_overlay(window, cx)
        })
    }
}

impl Render for EditorShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab_button = |label: &'static str, icon: &'static str, active: bool| {
            let base = div()
                .rounded_lg()
                .border_1()
                .border_color(white().opacity(if active { 0.35 } else { 0.08 }))
                .bg(if active {
                    white().opacity(0.12)
                } else {
                    white().opacity(0.02)
                })
                .p_2()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .text_color(white().opacity(if active { 0.95 } else { 0.65 }))
                        .child(icon),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(white().opacity(if active { 0.95 } else { 0.55 }))
                        .child(label),
                );
            if active {
                base
            } else {
                base.hover(|s| s.bg(white().opacity(0.05))).cursor_pointer()
            }
        };

        // Keep layout static while switching panel content by active tab.
        let active_panel = match self.active_tab {
            EditorShellTab::MediaPool => self.media_pool_select.clone().into_any_element(),
            EditorShellTab::Transitions => self.transition_select.clone().into_any_element(),
        };

        let tab_item = |label: &'static str, icon: &'static str, tab: EditorShellTab| {
            let is_active = self.active_tab == tab;
            tab_button(label, icon, is_active).on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.active_tab = tab;
                    cx.notify();
                }),
            )
        };

        div()
            .w(px(300.0))
            .min_w(px(220.0))
            .h_full()
            .min_h_0()
            .overflow_hidden()
            .bg(gpui::rgb(0x09090b))
            .border_r_1()
            .border_color(white().opacity(0.10))
            .flex()
            .flex_col()
            .p_2()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .overflow_hidden()
                    .child(
                        div()
                            .w(px(76.0))
                            .flex_shrink_0()
                            .h_full()
                            .min_h_0()
                            .border_r_1()
                            .border_color(white().opacity(0.08))
                            .px_2()
                            .py_2()
                            .overflow_hidden()
                            .child(
                                div()
                                    .w_full()
                                    .h_full()
                                    .min_h_0()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .overflow_y_scrollbar()
                                    // Order: Media Pool first, then transition/effect tabs.
                                    .child(tab_item("Media", "M", EditorShellTab::MediaPool))
                                    .child(tab_item(
                                        "Transitions",
                                        "T",
                                        EditorShellTab::Transitions,
                                    )),
                            ),
                    )
                    // Render active right-side panel based on selected tab.
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .overflow_hidden()
                            .child(active_panel),
                    ),
            )
    }
}
