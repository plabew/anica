// =========================================
// =========================================
// src/ui/media_pool_select.rs
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use gpui::{
    Context, Element, Entity, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    MouseButton, MouseDownEvent, PathPromptOptions, Render, RenderImage, ScrollWheelEvent, Style,
    Window, div, prelude::*, px, rgb, rgba,
};
use gpui_component::white;
use image::{ImageBuffer, Rgba};
use smallvec::SmallVec;

use crate::core::{
    export::{get_media_duration, is_supported_media_path},
    global_state::{GlobalState, MediaPoolItem, MediaPoolUiEvent},
    thumbnail,
};

const THUMB_CARD_W: f32 = 104.0;
const THUMB_CARD_H: f32 = 58.0;
const THUMB_MAX_DIM: u32 = 320;
const MEDIA_POOL_ROW_EST_H: f32 = 84.0;
const MEDIA_POOL_VIRTUAL_PAD_ROWS: usize = 5;
const MEDIA_POOL_THUMB_PRELOAD_PAD_ROWS: usize = 10;
const TIMELINE_PANEL_HEIGHT_PX: f32 = 364.0;
const MEDIA_LIST_MIN_VIEW_H: f32 = 140.0;
const MEDIA_EXPANDED_LIST_MIN_VIEW_H: f32 = 280.0;

#[derive(Clone)]
enum ThumbnailState {
    Loading,
    Audio,
    RequiresFfmpeg,
    Ready {
        image: Arc<RenderImage>,
        width: u32,
        height: u32,
    },
    Failed,
}

struct ThumbnailImageElement {
    image: Arc<RenderImage>,
    width: u32,
    height: u32,
}

impl ThumbnailImageElement {
    fn new(image: Arc<RenderImage>, width: u32, height: u32) -> Self {
        Self {
            image,
            width,
            height,
        }
    }

    fn fitted_bounds(&self, bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
        let container_w: f32 = bounds.size.width.into();
        let container_h: f32 = bounds.size.height.into();
        let frame_w = self.width as f32;
        let frame_h = self.height as f32;
        if frame_w == 0.0 || frame_h == 0.0 {
            return bounds;
        }

        // Fit the media thumbnail without stretching.
        let scale = (container_w / frame_w).min(container_h / frame_h);
        let dest_w = frame_w * scale;
        let dest_h = frame_h * scale;
        let offset_x = (container_w - dest_w) * 0.5;
        let offset_y = (container_h - dest_h) * 0.5;

        gpui::Bounds::new(
            gpui::point(
                bounds.origin.x + gpui::px(offset_x),
                bounds.origin.y + gpui::px(offset_y),
            ),
            gpui::size(gpui::px(dest_w), gpui::px(dest_h)),
        )
    }
}

impl Element for ThumbnailImageElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = Style {
            size: gpui::Size {
                width: gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0)),
                height: gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0)),
            },
            ..Default::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: gpui::Bounds<gpui::Pixels>,
        _state: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut gpui::App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        _layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut gpui::App,
    ) {
        let dest_bounds = self.fitted_bounds(bounds);
        let _ = window.paint_image(
            dest_bounds,
            gpui::Corners::default(),
            self.image.clone(),
            0,
            false,
        );
    }
}

impl IntoElement for ThumbnailImageElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct MediaPoolSelect {
    pub global: Entity<GlobalState>,
    thumbnail_state: HashMap<String, ThumbnailState>,
    list_scroll_y: f32,
    expand_modal_open: bool,
    expand_list_scroll_y: f32,
}

impl MediaPoolSelect {
    pub fn new(global: Entity<GlobalState>, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&global, |_, _global, evt: &MediaPoolUiEvent, cx| {
            if matches!(evt, MediaPoolUiEvent::StateChanged) {
                cx.notify();
            }
        })
        .detach();

        Self {
            global,
            thumbnail_state: HashMap::new(),
            list_scroll_y: 0.0,
            expand_modal_open: false,
            expand_list_scroll_y: 0.0,
        }
    }

    fn is_audio_ext(path: &str) -> bool {
        let p = path.to_lowercase();
        p.ends_with(".mp3")
            || p.ends_with(".wav")
            || p.ends_with(".m4a")
            || p.ends_with(".aac")
            || p.ends_with(".flac")
            || p.ends_with(".ogg")
            || p.ends_with(".opus")
    }

    fn is_image_ext(path: &str) -> bool {
        let p = path.to_lowercase();
        p.ends_with(".jpg")
            || p.ends_with(".jpeg")
            || p.ends_with(".png")
            || p.ends_with(".webp")
            || p.ends_with(".bmp")
            || p.ends_with(".gif")
            || p.ends_with(".tif")
            || p.ends_with(".tiff")
    }

    fn load_render_image(path: &Path) -> Result<(Arc<RenderImage>, u32, u32), String> {
        let decoded =
            image::open(path).map_err(|e| format!("Failed to open thumbnail image: {e}"))?;
        Self::load_render_image_from_dynamic(decoded)
    }

    fn load_render_image_from_jpeg_base64(
        preview_jpeg_base64: &str,
    ) -> Result<(Arc<RenderImage>, u32, u32), String> {
        let bytes = BASE64_STANDARD
            .decode(preview_jpeg_base64)
            .map_err(|e| format!("Failed to decode embedded preview base64: {e}"))?;
        let decoded = image::load_from_memory(&bytes)
            .map_err(|e| format!("Failed to decode embedded preview image: {e}"))?;
        Self::load_render_image_from_dynamic(decoded)
    }

    fn load_render_image_from_dynamic(
        decoded: image::DynamicImage,
    ) -> Result<(Arc<RenderImage>, u32, u32), String> {
        let rgba = decoded.to_rgba8();
        let (w, h) = rgba.dimensions();
        // GPUI image rendering expects BGRA-like channel order for this path.
        let mut bgra = rgba.into_raw();
        for px in bgra.chunks_mut(4) {
            let r = px[0];
            let b = px[2];
            px[0] = b;
            px[2] = r;
        }
        let image_buffer = ImageBuffer::<Rgba<u8>, _>::from_raw(w, h, bgra)
            .ok_or_else(|| "Failed to construct RGBA thumbnail buffer".to_string())?;
        let frames = SmallVec::from_elem(image::Frame::new(image_buffer), 1);
        Ok((Arc::new(RenderImage::new(frames)), w, h))
    }

    fn queue_thumbnail_if_needed(&mut self, item: &MediaPoolItem, cx: &mut Context<Self>) {
        if let Some(existing) = self.thumbnail_state.get(&item.path) {
            let can_generate_now = self.global.read(cx).media_tools_ready_for_preview_gen();
            if !matches!(existing, ThumbnailState::RequiresFfmpeg) || !can_generate_now {
                return;
            }
            self.thumbnail_state.remove(&item.path);
        }
        if !is_supported_media_path(&item.path) {
            self.thumbnail_state
                .insert(item.path.clone(), ThumbnailState::Failed);
            return;
        }
        if Self::is_audio_ext(&item.path) {
            // Audio-only items show a dedicated card label instead of frame extraction.
            self.thumbnail_state
                .insert(item.path.clone(), ThumbnailState::Audio);
            return;
        }

        if let Some(preview_jpeg_base64) = item.preview_jpeg_base64.as_deref()
            && let Ok((image, width, height)) =
                Self::load_render_image_from_jpeg_base64(preview_jpeg_base64)
        {
            self.thumbnail_state.insert(
                item.path.clone(),
                ThumbnailState::Ready {
                    image,
                    width,
                    height,
                },
            );
            return;
        }

        if Self::is_image_ext(&item.path) {
            match Self::load_render_image(Path::new(&item.path)) {
                Ok((image, width, height)) => {
                    self.thumbnail_state.insert(
                        item.path.clone(),
                        ThumbnailState::Ready {
                            image,
                            width,
                            height,
                        },
                    );
                }
                Err(_) => {
                    self.thumbnail_state
                        .insert(item.path.clone(), ThumbnailState::Failed);
                }
            }
            return;
        }

        let can_generate_preview = self.global.read(cx).media_tools_ready_for_preview_gen();
        if !can_generate_preview {
            self.thumbnail_state
                .insert(item.path.clone(), ThumbnailState::RequiresFfmpeg);
            return;
        }

        // Mark loading before spawning so duplicate renders do not enqueue duplicate jobs.
        self.thumbnail_state
            .insert(item.path.clone(), ThumbnailState::Loading);

        let src_path = PathBuf::from(item.path.clone());
        let item_path = item.path.clone();
        let (ffmpeg_path, cache_root) = {
            let gs = self.global.read(cx);
            (gs.ffmpeg_path.clone(), gs.cache_root_dir())
        };
        let thumb_path = thumbnail::thumbnail_path_for_in(&cache_root, &src_path, THUMB_MAX_DIM);

        cx.spawn(async move |view, cx| {
            let bg_result = cx
                .background_spawn(async move {
                    thumbnail::run_thumbnail_job(
                        &ffmpeg_path,
                        &src_path,
                        &thumb_path,
                        THUMB_MAX_DIM,
                    )
                    .map(|_| thumb_path)
                })
                .await;

            let _ = view.update(cx, |this, cx| {
                let next_state = match bg_result {
                    Ok(path) => match Self::load_render_image(&path) {
                        Ok((image, width, height)) => ThumbnailState::Ready {
                            image,
                            width,
                            height,
                        },
                        Err(_) => ThumbnailState::Failed,
                    },
                    Err(_) => ThumbnailState::Failed,
                };
                this.thumbnail_state.insert(item_path.clone(), next_state);
                cx.notify();
            });
        })
        .detach();
    }

    fn import_button() -> gpui::Div {
        div()
            .h(px(28.0))
            .px_3()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.12))
            .bg(white().opacity(0.05))
            .text_xs()
            .text_color(white().opacity(0.88))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|s| s.bg(white().opacity(0.10)))
            .child("Import Media")
    }

    fn expand_button() -> gpui::Div {
        div()
            .h(px(28.0))
            .px_3()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.12))
            .bg(white().opacity(0.05))
            .text_xs()
            .text_color(white().opacity(0.88))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|s| s.bg(white().opacity(0.10)))
            .child("Expand")
    }

    fn format_mmss(d: Duration) -> String {
        let secs = d.as_secs();
        let m = secs / 60;
        let s = secs % 60;
        format!("{m:02}:{s:02}")
    }

    fn estimate_list_view_h(
        window: &Window,
        media_tools_ready_for_preview: bool,
        media_tools_ready_for_export: bool,
    ) -> f32 {
        // Match editor-top area height (window minus fixed timeline strip), then remove
        // non-list blocks (titles/buttons/notices) to estimate list viewport.
        let window_h = window.viewport_size().height / px(1.0);
        let editor_h = (window_h - TIMELINE_PANEL_HEIGHT_PX).max(220.0);
        let mut non_list_h = 150.0;
        if !media_tools_ready_for_export {
            non_list_h += 94.0;
        }
        if !media_tools_ready_for_preview {
            non_list_h += 26.0;
        }
        (editor_h - non_list_h).max(MEDIA_LIST_MIN_VIEW_H)
    }

    fn estimate_expanded_list_view_h(window: &Window) -> f32 {
        let viewport_h = window.viewport_size().height / px(1.0);
        (viewport_h - 220.0).max(MEDIA_EXPANDED_LIST_MIN_VIEW_H)
    }

    fn compute_virtual_rows(
        total_items: usize,
        scroll_y: f32,
        view_h: f32,
        pad_rows: usize,
    ) -> (usize, usize, f32, f32, f32, f32) {
        if total_items == 0 {
            return (0, 0, 0.0, 0.0, 0.0, 0.0);
        }

        let total_h = (total_items as f32) * MEDIA_POOL_ROW_EST_H;
        let max_scroll_y = (total_h - view_h).max(0.0);
        let clamped_scroll = scroll_y.clamp(0.0, max_scroll_y);

        let first_visible = (clamped_scroll / MEDIA_POOL_ROW_EST_H).floor() as usize;
        let visible_rows = ((view_h / MEDIA_POOL_ROW_EST_H).ceil() as usize).max(1);
        let start = first_visible.saturating_sub(pad_rows);
        let end = (first_visible + visible_rows + pad_rows).min(total_items);
        let top_spacer_h = (start as f32) * MEDIA_POOL_ROW_EST_H;
        let bottom_spacer_h = ((total_items.saturating_sub(end)) as f32) * MEDIA_POOL_ROW_EST_H;
        (
            start,
            end,
            top_spacer_h,
            bottom_spacer_h,
            max_scroll_y,
            clamped_scroll,
        )
    }

    fn item_card(
        item: MediaPoolItem,
        is_active: bool,
        is_pending_drop: bool,
        thumbnail_state: Option<ThumbnailState>,
        global: Entity<GlobalState>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let item_path = item.path.clone();
        let item_path_for_left = item_path.clone();
        let item_path_for_right = item_path.clone();
        let item_name = item.name.clone();
        let row_bg = if is_active {
            white().opacity(0.10)
        } else if is_pending_drop {
            white().opacity(0.08)
        } else {
            white().opacity(0.03)
        };
        let row_border = if is_pending_drop {
            white().opacity(0.28)
        } else {
            white().opacity(0.12)
        };
        let duration_label = Self::format_mmss(item.duration);
        let thumbnail = match thumbnail_state {
            Some(ThumbnailState::Ready {
                image,
                width,
                height,
            }) => div()
                .w(px(THUMB_CARD_W))
                .h(px(THUMB_CARD_H))
                .rounded_sm()
                .overflow_hidden()
                .bg(white().opacity(0.03))
                .child(ThumbnailImageElement::new(image, width, height)),
            Some(ThumbnailState::Loading) => div()
                .w(px(THUMB_CARD_W))
                .h(px(THUMB_CARD_H))
                .rounded_sm()
                .bg(white().opacity(0.05))
                .text_xs()
                .text_color(white().opacity(0.6))
                .flex()
                .items_center()
                .justify_center()
                .child("Loading"),
            Some(ThumbnailState::Audio) => div()
                .w(px(THUMB_CARD_W))
                .h(px(THUMB_CARD_H))
                .rounded_sm()
                .bg(white().opacity(0.05))
                .text_xs()
                .text_color(white().opacity(0.72))
                .flex()
                .items_center()
                .justify_center()
                .child("Audio"),
            Some(ThumbnailState::RequiresFfmpeg) => div()
                .w(px(THUMB_CARD_W))
                .h(px(THUMB_CARD_H))
                .rounded_sm()
                .bg(white().opacity(0.05))
                .text_xs()
                .text_color(rgb(0xfca5a5))
                .flex()
                .items_center()
                .justify_center()
                .child("Needs FFmpeg"),
            _ => div()
                .w(px(THUMB_CARD_W))
                .h(px(THUMB_CARD_H))
                .rounded_sm()
                .bg(white().opacity(0.05))
                .text_xs()
                .text_color(white().opacity(0.55))
                .flex()
                .items_center()
                .justify_center()
                .child("No Preview"),
        };
        let global_for_left = global.clone();
        let global_for_right = global.clone();

        div()
            .rounded_md()
            .border_1()
            .border_color(row_border)
            .bg(row_bg)
            .px_2()
            .py_2()
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .hover(|s| s.bg(white().opacity(0.08)))
            .child(thumbnail)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.92))
                            .truncate()
                            .child(item_name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.45))
                            .truncate()
                            .child(item.path.clone()),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.7))
                    .child(duration_label),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_this, _, _, cx| {
                    // Select the item and start a drag payload for timeline drop.
                    global_for_left.update(cx, |gs, cx| {
                        if gs.activate_media_pool_item(&item_path_for_left)
                            && gs.begin_media_pool_drag(item_path_for_left.clone())
                        {
                            // Drag state is enough visual feedback; no global notice needed.
                        }
                        cx.emit(MediaPoolUiEvent::StateChanged);
                    });
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |_this, evt: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    global_for_right.update(cx, |gs, cx| {
                        let _ = gs.activate_media_pool_item(&item_path_for_right);
                        let menu_x = evt.position.x / px(1.0);
                        let menu_y = evt.position.y / px(1.0);
                        let _ = gs.open_media_pool_context_menu(
                            item_path_for_right.clone(),
                            menu_x,
                            menu_y,
                        );
                        cx.emit(MediaPoolUiEvent::StateChanged);
                    });
                }),
            )
    }

    pub fn render_expand_modal_overlay(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        if !self.expand_modal_open {
            return div();
        }

        let (
            items,
            active_path,
            pending_path,
            media_tools_ready_for_preview,
            media_tools_ready_for_export,
        ) = {
            let gs = self.global.read(cx);
            (
                gs.media_pool.clone(),
                gs.active_source_path.clone(),
                gs.pending_media_pool_path.clone(),
                gs.media_tools_ready_for_preview_gen(),
                gs.media_tools_ready_for_export(),
            )
        };

        self.thumbnail_state
            .retain(|path, _| items.iter().any(|item| item.path == *path));

        let list_view_h = Self::estimate_expanded_list_view_h(window);
        let (
            visible_start,
            visible_end,
            top_spacer_h,
            bottom_spacer_h,
            max_scroll_y,
            clamped_scroll,
        ) = Self::compute_virtual_rows(
            items.len(),
            self.expand_list_scroll_y,
            list_view_h,
            MEDIA_POOL_VIRTUAL_PAD_ROWS,
        );
        if (clamped_scroll - self.expand_list_scroll_y).abs() > f32::EPSILON {
            self.expand_list_scroll_y = clamped_scroll;
        }

        let preload_start = visible_start.saturating_sub(MEDIA_POOL_THUMB_PRELOAD_PAD_ROWS);
        let preload_end = (visible_end + MEDIA_POOL_THUMB_PRELOAD_PAD_ROWS).min(items.len());
        for item in items
            .iter()
            .skip(preload_start)
            .take(preload_end.saturating_sub(preload_start))
        {
            self.queue_thumbnail_if_needed(item, cx);
        }

        let viewport_w = window.viewport_size().width / px(1.0);
        let viewport_h = window.viewport_size().height / px(1.0);
        let card_w = (viewport_w - 120.0).clamp(760.0, 1320.0);
        let card_h = (viewport_h - 100.0).clamp(520.0, 920.0);
        let items_empty = items.is_empty();

        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right_0()
            .bg(gpui_component::black().opacity(0.62))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.expand_modal_open = false;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(card_w))
                    .h(px(card_h))
                    .rounded_md()
                    .bg(rgb(0x141419))
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
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(white().opacity(0.95))
                                            .child("Media Pool"),
                                    )
                                    .child(
                                        div()
                                            .h(px(28.0))
                                            .px_3()
                                            .rounded_md()
                                            .border_1()
                                            .border_color(white().opacity(0.18))
                                            .bg(white().opacity(0.05))
                                            .text_xs()
                                            .text_color(white().opacity(0.9))
                                            .cursor_pointer()
                                            .hover(|s| s.bg(white().opacity(0.10)))
                                            .child("Close")
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(|this, _, _, cx| {
                                                    this.expand_modal_open = false;
                                                    cx.notify();
                                                }),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.6))
                                    .child("Expanded view for browsing a larger media pool."),
                            )
                            .child(if media_tools_ready_for_export {
                                div().into_any_element()
                            } else {
                                div()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(rgba(0xf59e0b73))
                                    .bg(rgba(0x45280a47))
                                    .px_2()
                                    .py_2()
                                    .text_xs()
                                    .text_color(rgb(0xfcd34d))
                                    .child(
                                        "Basic import mode. Install FFmpeg/FFprobe to enable previews, export, and ACP media probe.",
                                    )
                                    .into_any_element()
                            })
                            .child(if media_tools_ready_for_preview {
                                div().into_any_element()
                            } else {
                                div()
                                    .text_xs()
                                    .text_color(rgb(0xfca5a5))
                                    .child("Thumbnail extraction disabled: requires FFmpeg.")
                                    .into_any_element()
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .child(if items_empty {
                                        div()
                                            .rounded_md()
                                            .border_1()
                                            .border_color(white().opacity(0.1))
                                            .bg(white().opacity(0.02))
                                            .p_3()
                                            .text_xs()
                                            .text_color(white().opacity(0.5))
                                            .child("No media imported.")
                                            .into_any_element()
                                    } else {
                                        let total_virtual_h =
                                            ((items.len() as f32) * MEDIA_POOL_ROW_EST_H)
                                                .max(list_view_h);
                                        div()
                                            .w_full()
                                            .h(px(list_view_h))
                                            .min_h(px(MEDIA_EXPANDED_LIST_MIN_VIEW_H))
                                            .relative()
                                            .overflow_hidden()
                                            .on_scroll_wheel(cx.listener(
                                                move |this, evt: &ScrollWheelEvent, _window, cx| {
                                                    let delta_y =
                                                        evt.delta.pixel_delta(px(20.0)).y / px(1.0);
                                                    if delta_y.abs() <= f32::EPSILON {
                                                        return;
                                                    }
                                                    this.expand_list_scroll_y =
                                                        (this.expand_list_scroll_y - delta_y)
                                                            .clamp(0.0, max_scroll_y);
                                                    cx.notify();
                                                },
                                            ))
                                            .child(
                                                div()
                                                    .w_full()
                                                    .h(px(total_virtual_h))
                                                    .relative()
                                                    .child(
                                                        div()
                                                            .w_full()
                                                            .absolute()
                                                            .top(px(-clamped_scroll))
                                                            .child(
                                                                div()
                                                                    .w_full()
                                                                    .pt(px(top_spacer_h))
                                                                    .pb(px(bottom_spacer_h))
                                                                    .flex()
                                                                    .flex_col()
                                                                    .gap_2()
                                                                    .children(items.iter().enumerate().skip(visible_start).take(visible_end.saturating_sub(visible_start)).map(|(_, item)| {
                                                                        let is_active = !active_path.is_empty() && item.path == active_path;
                                                                        let is_pending_drop =
                                                                            pending_path.as_ref().is_some_and(|p| *p == item.path);
                                                                        let thumb_state = self.thumbnail_state.get(&item.path).cloned();
                                                                        Self::item_card(
                                                                            item.clone(),
                                                                            is_active,
                                                                            is_pending_drop,
                                                                            thumb_state,
                                                                            self.global.clone(),
                                                                            cx,
                                                                        )
                                                                    })),
                                                            ),
                                                    ),
                                            )
                                            .into_any_element()
                                    }),
                            ),
                    ),
            )
    }
}

impl Render for MediaPoolSelect {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (
            items,
            active_path,
            pending_path,
            media_tools_ready_for_preview,
            media_tools_ready_for_export,
        ) = {
            let gs = self.global.read(cx);
            (
                gs.media_pool.clone(),
                gs.active_source_path.clone(),
                gs.pending_media_pool_path.clone(),
                gs.media_tools_ready_for_preview_gen(),
                gs.media_tools_ready_for_export(),
            )
        };

        // Keep thumbnail cache aligned with existing media pool entries.
        self.thumbnail_state
            .retain(|path, _| items.iter().any(|item| item.path == *path));
        let list_view_h = Self::estimate_list_view_h(
            window,
            media_tools_ready_for_preview,
            media_tools_ready_for_export,
        );
        let (visible_start, visible_end, top_spacer_h, bottom_spacer_h, max_scroll_y, scroll_y) =
            Self::compute_virtual_rows(
                items.len(),
                self.list_scroll_y,
                list_view_h,
                MEDIA_POOL_VIRTUAL_PAD_ROWS,
            );
        if (scroll_y - self.list_scroll_y).abs() > f32::EPSILON {
            self.list_scroll_y = scroll_y;
        }

        // Lazy-queue thumbnails only near visible rows to avoid large pool stalls.
        let preload_start = visible_start.saturating_sub(MEDIA_POOL_THUMB_PRELOAD_PAD_ROWS);
        let preload_end = (visible_end + MEDIA_POOL_THUMB_PRELOAD_PAD_ROWS).min(items.len());
        for item in items
            .iter()
            .skip(preload_start)
            .take(preload_end.saturating_sub(preload_start))
        {
            self.queue_thumbnail_if_needed(item, cx);
        }

        let items_empty = items.is_empty();

        let global_for_import = self.global.clone();
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(white().opacity(0.9))
                    .child("Media Pool"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.55))
                    .child("Import media, drag an item, then release on a timeline lane."),
            )
            .child(if media_tools_ready_for_export {
                div().into_any_element()
            } else {
                div()
                    .rounded_md()
                    .border_1()
                    .border_color(rgba(0xf59e0b73))
                    .bg(rgba(0x45280a47))
                    .px_2()
                    .py_2()
                    .text_xs()
                    .text_color(rgb(0xfcd34d))
                    .child(
                        "Basic import mode. Install FFmpeg/FFprobe to enable previews, export, and ACP media probe.",
                    )
                    .into_any_element()
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(Self::import_button().on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_this, _, win, cx| {
                            let global_for_import = global_for_import.clone();
                            let rx = cx.prompt_for_paths(PathPromptOptions {
                                files: true,
                                directories: false,
                                multiple: true,
                                prompt: Some("Import media files".into()),
                            });
                            cx.spawn_in(win, async move |view, window| {
                                let Ok(result) = rx.await else {
                                    return;
                                };
                                let Some(paths) = result.ok().flatten() else {
                                    return;
                                };
                                let _ = view.update_in(window, |_this, _window, cx| {
                                    // Add every valid media file into the pool and set a clear status message.
                                    global_for_import.update(cx, |gs, cx| {
                                        let mut imported = 0usize;
                                        for path in paths {
                                            let path_str = path.to_string_lossy().to_string();
                                            if !is_supported_media_path(&path_str) {
                                                continue;
                                            }
                                            let duration = get_media_duration(&path_str);
                                            if duration > Duration::ZERO {
                                                gs.load_source_video(path.to_path_buf(), duration);
                                                imported += 1;
                                            }
                                        }

                                        if imported == 0 {
                                            gs.ui_notice = Some(
                                                "No valid media file was imported.".to_string(),
                                            );
                                        } else if !gs.media_tools_ready_for_export() {
                                            gs.ui_notice = Some(
                                                "Imported in basic mode. Install FFmpeg/FFprobe for previews, export, and ACP media probe."
                                                    .to_string(),
                                            );
                                        } else {
                                            gs.ui_notice = None;
                                        }
                                        if imported > 0 {
                                            cx.emit(MediaPoolUiEvent::StateChanged);
                                        } else {
                                            cx.notify();
                                        }
                                    });
                                    cx.notify();
                                });
                            })
                            .detach();
                        }),
                    ))
                    .child(Self::expand_button().on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.expand_modal_open = true;
                            this.expand_list_scroll_y = this.list_scroll_y;
                            cx.notify();
                        }),
                    )),
            )
            .child(if media_tools_ready_for_preview {
                div().into_any_element()
            } else {
                div()
                    .text_xs()
                    .text_color(rgb(0xfca5a5))
                    .child("Thumbnail extraction disabled: requires FFmpeg.")
                    .into_any_element()
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(if items_empty {
                        div()
                            .rounded_md()
                            .border_1()
                            .border_color(white().opacity(0.1))
                            .bg(white().opacity(0.02))
                            .p_3()
                            .text_xs()
                            .text_color(white().opacity(0.5))
                            .child("No media imported.")
                            .into_any_element()
                    } else {
                        let total_virtual_h =
                            ((items.len() as f32) * MEDIA_POOL_ROW_EST_H).max(list_view_h);
                        div()
                            .w_full()
                            .h(px(list_view_h))
                            .min_h(px(MEDIA_LIST_MIN_VIEW_H))
                            .relative()
                            .overflow_hidden()
                            .on_scroll_wheel(cx.listener(
                                move |this, evt: &ScrollWheelEvent, _window, cx| {
                                    let delta_y = evt.delta.pixel_delta(px(20.0)).y / px(1.0);
                                    if delta_y.abs() <= f32::EPSILON {
                                        return;
                                    }
                                    this.list_scroll_y =
                                        (this.list_scroll_y - delta_y).clamp(0.0, max_scroll_y);
                                    cx.notify();
                                },
                            ))
                            .child(
                                div()
                                    .w_full()
                                    .h(px(total_virtual_h))
                                    .relative()
                                    .child(
                                        div()
                                            .w_full()
                                            .absolute()
                                            .top(px(-scroll_y))
                                            .child(
                                                div()
                                                    .w_full()
                                                    .pt(px(top_spacer_h))
                                                    .pb(px(bottom_spacer_h))
                                                    .flex()
                                                    .flex_col()
                                                    .gap_2()
                                                    .children(items.iter().enumerate().skip(visible_start).take(visible_end.saturating_sub(visible_start)).map(|(_, item)| {
                                                        let is_active = !active_path.is_empty() && item.path == active_path;
                                                        let is_pending_drop =
                                                            pending_path.as_ref().is_some_and(|p| *p == item.path);
                                                        let thumb_state = self.thumbnail_state.get(&item.path).cloned();
                                                        Self::item_card(
                                                            item.clone(),
                                                            is_active,
                                                            is_pending_drop,
                                                            thumb_state,
                                                            self.global.clone(),
                                                            cx,
                                                        )
                                                    })),
                                            ),
                                    ),
                            )
                            .into_any_element()
                    }),
            )
    }
}
