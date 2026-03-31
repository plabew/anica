// =========================================
// =========================================
// src/ui/motionloom_page.rs — MotionLoom VFX Studio page with graph preview and template picker

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    Context, Element, Entity, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    MouseButton, PathPromptOptions, Render, RenderImage, Style, Subscription, Window, div,
    prelude::*, px, rgb, rgba,
};
use gpui_component::{
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    white,
};
use gpui_video_renderer::VideoElement;
use image::{ImageBuffer, Rgba};
use motionloom::{RuntimeProgram, compile_runtime_program, is_graph_script, parse_graph_script};
use smallvec::SmallVec;
use thiserror::Error;
use url::Url;
use video_engine::{Position, Video, VideoOptions};

use crate::core::export::get_media_duration;
use crate::core::global_state::GlobalState;
use crate::core::thumbnail;
use crate::ui::motionloom_templates;
use crate::ui::motionloom_templates::LayerEffectTemplateKind;

const THUMB_MAX_DIM: u32 = 640;
const PREVIEW_BOX_W: f32 = 880.0;
const PREVIEW_BOX_H: f32 = 520.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportedClipKind {
    Image,
    Video,
}

impl ImportedClipKind {
    const fn label(self) -> &'static str {
        match self {
            ImportedClipKind::Image => "Image",
            ImportedClipKind::Video => "Video",
        }
    }
}

#[derive(Clone)]
struct LoadedPreview {
    image: Arc<RenderImage>,
    width: u32,
    height: u32,
    bgra: Arc<Vec<u8>>,
}

#[derive(Clone)]
struct ImportedClip {
    name: String,
    path: String,
    kind: ImportedClipKind,
    duration: Duration,
    preview: Option<LoadedPreview>,
    error: Option<String>,
}

#[derive(Debug, Error)]
enum MotionLoomPageError {
    #[error("Failed to open preview image: {source}")]
    OpenPreviewImage { source: image::ImageError },
    #[error("Failed to construct preview image buffer")]
    BuildPreviewImageBuffer,
    #[error("Failed to construct runtime preview buffer")]
    BuildRuntimePreviewBuffer,
    #[error("Failed to convert path to URL: {path}")]
    PathToUrl { path: PathBuf },
    #[error("Failed to open video preview player: {message}")]
    OpenVideoPreviewPlayer { message: String },
    #[error(transparent)]
    Thumbnail(#[from] crate::core::thumbnail::ThumbnailError),
}

// Fit-to-container preview image element that renders a source image
// centered inside the available bounds with aspect-ratio preservation.
struct FitPreviewImageElement {
    image: Arc<RenderImage>,
    width: u32,
    height: u32,
}

impl FitPreviewImageElement {
    fn new(image: Arc<RenderImage>, width: u32, height: u32) -> Self {
        Self {
            image,
            width,
            height,
        }
    }

    // Calculate destination bounds that fit the image into the container
    // while preserving the original aspect ratio.
    fn fitted_bounds(&self, bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
        let container_w: f32 = bounds.size.width.into();
        let container_h: f32 = bounds.size.height.into();
        let frame_w = self.width as f32;
        let frame_h = self.height as f32;
        if frame_w == 0.0 || frame_h == 0.0 {
            return bounds;
        }

        let fit_scale = (container_w / frame_w).min(container_h / frame_h);
        let dest_w = frame_w * fit_scale;
        let dest_h = frame_h * fit_scale;
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

impl Element for FitPreviewImageElement {
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

impl IntoElement for FitPreviewImageElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct MotionLoomPage {
    pub global: Entity<GlobalState>,
    clips: Vec<ImportedClip>,
    selected_idx: Option<usize>,
    preview_frame: u32,
    status_line: String,
    script_text: String,
    script_input: Option<Entity<InputState>>,
    script_input_sub: Option<Subscription>,
    graph_runtime: Option<RuntimeProgram>,
    runtime_preview_cache_key: Option<(usize, u32, i32, i32, i32, i32, i32, i32, u32, u32)>,
    runtime_preview_cache_image: Option<Arc<RenderImage>>,
    preview_playing: bool,
    preview_play_token: u64,
    preview_last_tick: Option<Instant>,
    preview_frame_accum: f32,
    video_preview_player: Option<Video>,
    video_preview_player_path: Option<String>,
    video_preview_last_seek_frame: Option<u32>,
    // Template picker state
    template_modal_open: bool,
    template_selected: Vec<LayerEffectTemplateKind>,
    template_add_time_parameter: bool,
    template_add_curve_parameter: bool,
}

impl MotionLoomPage {
    pub fn new(global: Entity<GlobalState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&global, |_this, _global, cx| {
            cx.notify();
        })
        .detach();

        Self {
            global,
            clips: Vec::new(),
            selected_idx: None,
            preview_frame: 0,
            status_line: "Import a video or still to start building a MotionLoom graph."
                .to_string(),
            script_text: motionloom_templates::DEFAULT_GRAPH_SCRIPT.to_string(),
            script_input: None,
            script_input_sub: None,
            graph_runtime: None,
            runtime_preview_cache_key: None,
            runtime_preview_cache_image: None,
            preview_playing: false,
            preview_play_token: 0,
            preview_last_tick: None,
            preview_frame_accum: 0.0,
            video_preview_player: None,
            video_preview_player_path: None,
            video_preview_last_seek_frame: None,
            template_modal_open: false,
            template_selected: Vec::new(),
            template_add_time_parameter: false,
            template_add_curve_parameter: false,
        }
    }

    fn is_image_path(path: &str) -> bool {
        let p = path.to_ascii_lowercase();
        p.ends_with(".jpg")
            || p.ends_with(".jpeg")
            || p.ends_with(".png")
            || p.ends_with(".webp")
            || p.ends_with(".bmp")
            || p.ends_with(".gif")
    }

    fn is_video_path(path: &str) -> bool {
        let p = path.to_ascii_lowercase();
        p.ends_with(".mp4")
            || p.ends_with(".mov")
            || p.ends_with(".mkv")
            || p.ends_with(".webm")
            || p.ends_with(".avi")
            || p.ends_with(".flv")
            || p.ends_with(".m4v")
    }

    fn is_supported_clip_path(path: &str) -> bool {
        Self::is_image_path(path) || Self::is_video_path(path)
    }

    fn load_render_image(path: &Path) -> Result<LoadedPreview, MotionLoomPageError> {
        let decoded =
            image::open(path).map_err(|source| MotionLoomPageError::OpenPreviewImage { source })?;
        Self::load_render_image_from_dynamic(decoded)
    }

    fn load_render_image_from_dynamic(
        decoded: image::DynamicImage,
    ) -> Result<LoadedPreview, MotionLoomPageError> {
        let rgba = decoded.to_rgba8();
        let (w, h) = rgba.dimensions();
        let mut bgra = rgba.into_raw();
        for px in bgra.chunks_mut(4) {
            let r = px[0];
            let b = px[2];
            px[0] = b;
            px[2] = r;
        }
        let source_bgra = Arc::new(bgra.clone());
        let image_buffer = ImageBuffer::<Rgba<u8>, _>::from_raw(w, h, bgra)
            .ok_or(MotionLoomPageError::BuildPreviewImageBuffer)?;
        let frames = SmallVec::from_elem(image::Frame::new(image_buffer), 1);
        Ok(LoadedPreview {
            image: Arc::new(RenderImage::new(frames)),
            width: w,
            height: h,
            bgra: source_bgra,
        })
    }

    fn render_image_from_bgra(
        width: u32,
        height: u32,
        bgra: Vec<u8>,
    ) -> Result<Arc<RenderImage>, MotionLoomPageError> {
        let image_buffer = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, bgra)
            .ok_or(MotionLoomPageError::BuildRuntimePreviewBuffer)?;
        let frames = SmallVec::from_elem(image::Frame::new(image_buffer), 1);
        Ok(Arc::new(RenderImage::new(frames)))
    }

    // Nearest-neighbor resize for CPU preview rendering.
    fn resize_bgra_nearest(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
        if src_w == dst_w && src_h == dst_h {
            return src.to_vec();
        }
        if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
            return src.to_vec();
        }

        let mut dst = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
        for y in 0..dst_h {
            let sy = ((y as u64 * src_h as u64) / dst_h as u64) as u32;
            for x in 0..dst_w {
                let sx = ((x as u64 * src_w as u64) / dst_w as u64) as u32;
                let src_ix = ((sy as usize) * (src_w as usize) + (sx as usize)) * 4;
                let dst_ix = ((y as usize) * (dst_w as usize) + (x as usize)) * 4;
                dst[dst_ix..dst_ix + 4].copy_from_slice(&src[src_ix..src_ix + 4]);
            }
        }
        dst
    }

    // Resolve the target canvas size from the graph runtime or fall back to None.
    fn runtime_target_size(&self) -> Option<(u32, u32)> {
        let runtime = self.graph_runtime.as_ref()?;
        let graph = runtime.graph();
        if let Some(size) = graph.resource_size(&graph.present.from) {
            return Some(size);
        }
        Some(graph.size)
    }

    fn playback_fps(&self) -> f32 {
        self.graph_runtime
            .as_ref()
            .map(|runtime| runtime.graph().fps)
            .filter(|fps| fps.is_finite() && *fps > 0.0)
            .unwrap_or(30.0)
    }

    fn ensure_video_preview_player(&mut self, path: &str) -> Result<(), MotionLoomPageError> {
        if self.video_preview_player.is_some()
            && self.video_preview_player_path.as_deref() == Some(path)
        {
            return Ok(());
        }

        self.video_preview_player = None;
        self.video_preview_player_path = None;
        self.video_preview_last_seek_frame = None;

        let pb = PathBuf::from(path);
        let url = Url::from_file_path(&pb)
            .map_err(|_| MotionLoomPageError::PathToUrl { path: pb.clone() })?;
        let fps = self.playback_fps().round().clamp(1.0, 240.0) as u32;
        let opts = VideoOptions {
            frame_buffer_capacity: Some(2),
            preview_scale: None,
            preview_max_dim: None,
            preview_fps: Some(fps),
            appsink_max_buffers: Some(2),
            #[cfg(target_os = "macos")]
            prefer_surface: true,
            #[cfg(target_os = "macos")]
            strict_surface_proxy_nv12: false,
            benchmark_raw_appsink: VideoOptions::benchmark_raw_appsink_from_env(),
            ..Default::default()
        };
        let player = Video::new_with_options(&url, opts).map_err(|err| {
            MotionLoomPageError::OpenVideoPreviewPlayer {
                message: err.to_string(),
            }
        })?;
        player.set_muted(true);
        player.set_paused(true);
        let _ = player.seek(Position::Time(Duration::ZERO), false);
        self.video_preview_player = Some(player);
        self.video_preview_player_path = Some(path.to_string());
        Ok(())
    }

    fn seek_video_preview_frame(&mut self, path: &str, frame: u32) -> bool {
        if self.ensure_video_preview_player(path).is_err() {
            return false;
        }
        let fps = self.playback_fps().max(1.0);
        if self.video_preview_last_seek_frame != Some(frame) {
            let seek_t = Duration::from_secs_f64(frame as f64 / fps as f64);
            if let Some(player) = self.video_preview_player.as_ref() {
                let _ = player.seek(Position::Time(seek_t), false);
            }
            self.video_preview_last_seek_frame = Some(frame);
        }
        true
    }

    // Provide the video player handle for the stage preview, managing
    // play/pause transport depending on whether live playback is active.
    fn video_preview_player_for_stage(
        &mut self,
        path: &str,
        frame: u32,
        use_live_playback: bool,
    ) -> Option<Video> {
        if self.ensure_video_preview_player(path).is_err() {
            return None;
        }
        let player = self.video_preview_player.as_ref()?;
        if use_live_playback {
            player.set_paused(false);
            self.video_preview_last_seek_frame = None;
        } else {
            player.set_paused(true);
            if self.video_preview_last_seek_frame != Some(frame) {
                let fps = self.playback_fps().max(1.0);
                let seek_t = Duration::from_secs_f64(frame as f64 / fps as f64);
                let _ = player.seek(Position::Time(seek_t), false);
                self.video_preview_last_seek_frame = Some(frame);
            }
        }
        self.video_preview_player.clone()
    }

    fn video_preview_frame_bgra(&mut self, path: &str, frame: u32) -> Option<(Vec<u8>, u32, u32)> {
        if !self.seek_video_preview_frame(path, frame) {
            return None;
        }
        self.video_preview_player
            .as_ref()
            .and_then(|player| player.current_frame_data())
    }

    // Calculate total frame count from graph runtime duration or clip duration.
    fn playback_frame_count(&self) -> u32 {
        if let Some(runtime) = self.graph_runtime.as_ref() {
            let graph = runtime.graph();
            let total = ((graph.duration_ms as f64 / 1000.0) * graph.fps as f64).round() as u32;
            if total > 1 {
                return total;
            }
        }

        if let Some(clip) = self.current_clip() {
            let secs = clip.duration.as_secs_f64();
            if secs > 0.0 {
                let fps = self.playback_fps().max(1.0) as f64;
                let total = (secs * fps).round() as u32;
                if total > 1 {
                    return total;
                }
            }
        }

        // Default minimum frame count for still images
        60
    }

    // Schedule the next frame tick for continuous playback preview.
    fn schedule_preview_playback(
        &mut self,
        token: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.on_next_frame(window, move |this, window, cx| {
            if !this.preview_playing || this.preview_play_token != token {
                return;
            }

            let now = Instant::now();
            let dt = now.saturating_duration_since(this.preview_last_tick.unwrap_or(now));
            this.preview_last_tick = Some(now);

            let fps = this.playback_fps();
            this.preview_frame_accum += dt.as_secs_f32() * fps;
            let step = this.preview_frame_accum.floor() as u32;
            if step > 0 {
                this.preview_frame_accum -= step as f32;
                let frame_count = this.playback_frame_count();
                this.preview_frame = (this.preview_frame + step) % frame_count;
                cx.notify();
            }

            if this.preview_playing && this.preview_play_token == token {
                this.schedule_preview_playback(token, window, cx);
            }
        });
    }

    fn step_preview_frame(&mut self, delta: i32) {
        self.preview_playing = false;
        self.preview_last_tick = None;
        self.preview_frame_accum = 0.0;
        if delta >= 0 {
            self.preview_frame = self.preview_frame.saturating_add(delta as u32);
        } else {
            self.preview_frame = self.preview_frame.saturating_sub(delta.unsigned_abs());
        }
    }

    fn toggle_preview_playback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.preview_playing {
            self.preview_playing = false;
            self.preview_last_tick = None;
            self.preview_frame_accum = 0.0;
            self.status_line = format!("Paused at frame {}.", self.preview_frame);
            cx.notify();
            return;
        }
        if self.current_clip().is_none() {
            self.status_line = "Import/select a clip before playback.".to_string();
            cx.notify();
            return;
        }

        self.preview_playing = true;
        self.preview_play_token = self.preview_play_token.wrapping_add(1);
        self.preview_last_tick = Some(Instant::now());
        self.preview_frame_accum = 0.0;
        let token = self.preview_play_token;
        self.status_line = format!("Playback started at {} fps.", self.playback_fps());
        self.schedule_preview_playback(token, window, cx);
        cx.notify();
    }

    // CPU-side preview rendering with color/blur/opacity effects from the graph runtime.
    fn runtime_preview_image(
        &mut self,
        clip_idx: usize,
        source_bgra: &[u8],
        source_w: u32,
        source_h: u32,
        fallback_image: Arc<RenderImage>,
        frame: u32,
        invert_mix: f32,
        brightness: f32,
        contrast: f32,
        saturation: f32,
        blur_sigma: f32,
        opacity: f32,
        target_size: (u32, u32),
    ) -> (Arc<RenderImage>, u32, u32) {
        let mix = invert_mix.clamp(0.0, 1.0);
        let brightness = brightness.clamp(-1.0, 1.0);
        let contrast = contrast.clamp(0.0, 2.0);
        let saturation = saturation.clamp(0.0, 2.0);
        let blur_sigma = blur_sigma.clamp(-64.0, 64.0);
        let opacity = opacity.clamp(0.0, 1.0);
        let target_w = target_size.0.max(1);
        let target_h = target_size.1.max(1);
        let quantized = (mix * 1000.0).round() as i32;
        let bq = (brightness * 1000.0).round() as i32;
        let cq = (contrast * 1000.0).round() as i32;
        let sq = (saturation * 1000.0).round() as i32;
        let blur_q = (blur_sigma * 1000.0).round() as i32;
        let oq = (opacity * 1000.0).round() as i32;
        let key = (
            clip_idx, frame, quantized, bq, cq, sq, blur_q, oq, target_w, target_h,
        );
        if self.runtime_preview_cache_key == Some(key)
            && let Some(image) = self.runtime_preview_cache_image.as_ref()
        {
            return (image.clone(), target_w, target_h);
        }

        let mut bgra =
            Self::resize_bgra_nearest(source_bgra, source_w, source_h, target_w, target_h);

        // Apply color grading effects (invert, brightness, contrast, saturation, opacity)
        if mix > 0.0001
            || brightness.abs() > 0.0001
            || (contrast - 1.0).abs() > 0.0001
            || (saturation - 1.0).abs() > 0.0001
            || (opacity - 1.0).abs() > 0.0001
        {
            for px in bgra.chunks_mut(4) {
                let sb = px[0] as f32;
                let sg = px[1] as f32;
                let sr = px[2] as f32;

                let mut r = sr / 255.0;
                let mut g = sg / 255.0;
                let mut b = sb / 255.0;

                if mix > 0.0001 {
                    r = r * (1.0 - mix) + (1.0 - r) * mix;
                    g = g * (1.0 - mix) + (1.0 - g) * mix;
                    b = b * (1.0 - mix) + (1.0 - b) * mix;
                }

                r = ((r + brightness) - 0.5) * contrast + 0.5;
                g = ((g + brightness) - 0.5) * contrast + 0.5;
                b = ((b + brightness) - 0.5) * contrast + 0.5;

                let luma = 0.299 * r + 0.587 * g + 0.114 * b;
                r = luma + (r - luma) * saturation;
                g = luma + (g - luma) * saturation;
                b = luma + (b - luma) * saturation;

                let mut nr = r.clamp(0.0, 1.0);
                let mut ng = g.clamp(0.0, 1.0);
                let mut nb = b.clamp(0.0, 1.0);
                if opacity < 0.9999 {
                    let srn = sr / 255.0;
                    let sgn = sg / 255.0;
                    let sbn = sb / 255.0;
                    nr = srn * (1.0 - opacity) + nr * opacity;
                    ng = sgn * (1.0 - opacity) + ng * opacity;
                    nb = sbn * (1.0 - opacity) + nb * opacity;
                }

                px[2] = (nr * 255.0).round().clamp(0.0, 255.0) as u8;
                px[1] = (ng * 255.0).round().clamp(0.0, 255.0) as u8;
                px[0] = (nb * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }

        // Apply gaussian blur or unsharp-mask sharpening
        if blur_sigma > 0.05 {
            let mut rgba = Vec::with_capacity(bgra.len());
            for px in bgra.chunks_exact(4) {
                rgba.push(px[2]);
                rgba.push(px[1]);
                rgba.push(px[0]);
                rgba.push(px[3]);
            }
            if let Some(rgba_img) =
                ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(target_w, target_h, rgba)
            {
                let blurred = image::imageops::blur(&rgba_img, blur_sigma);
                let raw = blurred.into_raw();
                for (dst, src) in bgra.chunks_exact_mut(4).zip(raw.chunks_exact(4)) {
                    dst[0] = src[2];
                    dst[1] = src[1];
                    dst[2] = src[0];
                    dst[3] = src[3];
                }
            }
        } else if blur_sigma < -0.05 {
            let sharpen_sigma = blur_sigma.abs();
            let amount = 1.0_f32;
            let base = bgra.clone();
            let mut rgba = Vec::with_capacity(base.len());
            for px in base.chunks_exact(4) {
                rgba.push(px[2]);
                rgba.push(px[1]);
                rgba.push(px[0]);
                rgba.push(px[3]);
            }
            if let Some(rgba_img) =
                ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(target_w, target_h, rgba)
            {
                let blurred = image::imageops::blur(&rgba_img, sharpen_sigma);
                let raw_blur = blurred.into_raw();
                for ((dst, src_base), src_blur) in bgra
                    .chunks_exact_mut(4)
                    .zip(base.chunks_exact(4))
                    .zip(raw_blur.chunks_exact(4))
                {
                    for ch in 0..3 {
                        let b = src_base[ch] as f32 / 255.0;
                        let bl = src_blur[ch] as f32 / 255.0;
                        let v = (b + (b - bl) * amount).clamp(0.0, 1.0);
                        dst[ch] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
                    }
                    dst[3] = src_base[3];
                }
            }
        }

        let image = Self::render_image_from_bgra(target_w, target_h, bgra)
            .unwrap_or_else(|_| fallback_image.clone());
        self.runtime_preview_cache_key = Some(key);
        self.runtime_preview_cache_image = Some(image.clone());
        (image, target_w, target_h)
    }

    // Build an ImportedClip from a file path, generating a thumbnail if possible.
    fn build_imported_clip(
        path: &str,
        ffmpeg_path: &str,
        cache_root: &Path,
        can_generate_video_thumbnail: bool,
    ) -> ImportedClip {
        let pb = PathBuf::from(path);
        let name = pb
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(path)
            .to_string();
        let duration = if Self::is_video_path(path) {
            get_media_duration(path)
        } else {
            Duration::ZERO
        };

        if Self::is_image_path(path) {
            return match Self::load_render_image(&pb) {
                Ok(preview) => ImportedClip {
                    name,
                    path: path.to_string(),
                    kind: ImportedClipKind::Image,
                    duration,
                    preview: Some(preview),
                    error: None,
                },
                Err(err) => ImportedClip {
                    name,
                    path: path.to_string(),
                    kind: ImportedClipKind::Image,
                    duration,
                    preview: None,
                    error: Some(err.to_string()),
                },
            };
        }

        if !can_generate_video_thumbnail {
            return ImportedClip {
                name,
                path: path.to_string(),
                kind: ImportedClipKind::Video,
                duration,
                preview: None,
                error: Some("FFmpeg is required to generate video preview thumbnails.".to_string()),
            };
        }

        let thumb_path = thumbnail::thumbnail_path_for_in(cache_root, &pb, THUMB_MAX_DIM);
        let preview = thumbnail::run_thumbnail_job(ffmpeg_path, &pb, &thumb_path, THUMB_MAX_DIM)
            .map_err(MotionLoomPageError::from)
            .and_then(|_| Self::load_render_image(&thumb_path));

        match preview {
            Ok(preview) => ImportedClip {
                name,
                path: path.to_string(),
                kind: ImportedClipKind::Video,
                duration,
                preview: Some(preview),
                error: None,
            },
            Err(err) => ImportedClip {
                name,
                path: path.to_string(),
                kind: ImportedClipKind::Video,
                duration,
                preview: None,
                error: Some(err.to_string()),
            },
        }
    }

    fn current_clip(&self) -> Option<&ImportedClip> {
        let idx = self.selected_idx?;
        self.clips.get(idx)
    }

    fn ensure_script_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.script_input.is_some() {
            return;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("tsx")
                .rows(8)
                .line_number(true)
                .soft_wrap(true)
                .placeholder("<Graph ...> MotionLoom DSL script")
        });
        let initial = self.script_text.clone();
        input.update(cx, |this, cx| {
            this.set_value(initial.clone(), window, cx);
        });
        let sub = cx.subscribe(&input, |this, input, ev, cx| match ev {
            InputEvent::Change => {
                this.script_text = input.read(cx).value().to_string();
            }
            InputEvent::PressEnter { secondary } => {
                this.script_text = input.read(cx).value().to_string();
                if *secondary {
                    this.apply_script_command(cx);
                    cx.notify();
                }
            }
            _ => {}
        });
        self.script_input = Some(input);
        self.script_input_sub = Some(sub);
    }

    // Parse and compile the graph script, activating the runtime for preview.
    fn apply_script_command(&mut self, _cx: &mut Context<Self>) {
        let raw = self.script_text.clone();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            self.status_line =
                "Script is empty. Use the Template Picker or write a <Graph> script.".to_string();
            return;
        }

        // Only accept Graph DSL scripts (XML-based MotionLoom format)
        if !is_graph_script(&raw) {
            self.status_line = "Not a valid Graph script. Use the Template Picker to generate one, or write a <Graph ...> block.".to_string();
            return;
        }

        match parse_graph_script(&raw) {
            Ok(graph) => match compile_runtime_program(graph.clone()) {
                Ok(runtime) => {
                    if !runtime.unsupported_kernels().is_empty() {
                        self.graph_runtime = None;
                        self.status_line = format!(
                            "Unsupported kernel(s): {}",
                            runtime.unsupported_kernels().join(", ")
                        );
                        return;
                    }
                    let graph_summary = graph.summary();
                    let runtime_summary = runtime.summary();
                    self.graph_runtime = Some(runtime);
                    self.preview_frame = 0;
                    self.preview_playing = false;
                    self.preview_last_tick = None;
                    self.preview_frame_accum = 0.0;
                    self.runtime_preview_cache_key = None;
                    self.runtime_preview_cache_image = None;
                    self.video_preview_last_seek_frame = None;
                    self.status_line =
                        format!("Runtime ACTIVE | {} | {}", graph_summary, runtime_summary);
                }
                Err(err) => {
                    self.graph_runtime = None;
                    self.status_line = format!("Runtime compile error: {}", err.message);
                }
            },
            Err(err) => {
                self.graph_runtime = None;
                self.status_line =
                    format!("Graph parse error at line {}: {}", err.line, err.message);
            }
        }
    }

    // Set the script text and sync into the input widget.
    fn set_script_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.script_text = text.clone();
        if let Some(input) = self.script_input.as_ref() {
            input.update(cx, |this, cx| {
                this.set_value(text, window, cx);
            });
        }
    }

    fn control_button(label: &'static str) -> gpui::Div {
        div()
            .h(px(28.0))
            .px_2()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.15))
            .bg(white().opacity(0.06))
            .hover(|s| s.bg(white().opacity(0.1)))
            .cursor_pointer()
            .text_xs()
            .text_color(white().opacity(0.9))
            .flex()
            .items_center()
            .justify_center()
            .child(label)
    }

    // --- Template picker logic (ported from inspector_panel) ---

    fn template_label(kind: LayerEffectTemplateKind) -> &'static str {
        match kind {
            LayerEffectTemplateKind::BlurGaussian => "Blur Gaussian",
            LayerEffectTemplateKind::Sharpen => "Sharpen",
            LayerEffectTemplateKind::Opacity => "Opacity",
            LayerEffectTemplateKind::Lut => "LUT",
            LayerEffectTemplateKind::HslaOverlay => "HSLA Overlay",
            LayerEffectTemplateKind::TransitionFadeInOut => "Transition Fade In/Out",
        }
    }

    fn toggle_template_selection(&mut self, kind: LayerEffectTemplateKind) {
        if let Some(idx) = self
            .template_selected
            .iter()
            .position(|selected| *selected == kind)
        {
            self.template_selected.remove(idx);
            return;
        }
        self.template_selected.push(kind);
    }

    fn selected_template_summary(&self) -> String {
        if self.template_selected.is_empty() {
            return "No templates selected.".to_string();
        }
        self.template_selected
            .iter()
            .map(|kind| Self::template_label(*kind))
            .collect::<Vec<_>>()
            .join(" -> ")
    }

    // Apply selected templates: generate a new graph script or append to existing.
    fn apply_selected_templates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.template_selected.is_empty() {
            self.status_line = "Choose at least one template before pressing OK.".to_string();
            return;
        }

        let add_time = self.template_add_time_parameter;
        let add_curve = self.template_add_curve_parameter;
        let selected = self.template_selected.clone();
        let existing_script = self.script_text.trim().to_string();
        let selection_label = self.selected_template_summary();

        // Build new chain or append to existing graph
        let result = if existing_script.is_empty() || !is_graph_script(&existing_script) {
            motionloom_templates::build_layer_effect_chain_script(&selected, add_time, add_curve)
        } else {
            motionloom_templates::append_layer_effect_template_chain_script(
                &existing_script,
                &selected,
                add_curve,
            )
        };

        let Some(script) = result else {
            self.status_line =
                "Current script is not a standard chainable layer graph. Clear it before applying a multi-template selection."
                    .to_string();
            return;
        };

        self.set_script_text(script, window, cx);
        self.template_modal_open = false;
        self.template_selected.clear();
        self.status_line = if existing_script.is_empty() || !is_graph_script(&existing_script) {
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

    fn open_template_modal(&mut self) {
        self.template_modal_open = true;
        self.template_selected.clear();
        self.status_line = "Template picker opened.".to_string();
    }

    // Render a single selectable template tile in the picker modal.
    fn render_template_tile(
        &self,
        kind: LayerEffectTemplateKind,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let selected = self.template_selected.contains(&kind);
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
        let label = Self::template_label(kind);
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
                    this.toggle_template_selection(kind);
                    cx.notify();
                }),
            )
    }

    // Render the full-screen template picker modal overlay.
    fn render_template_modal_overlay(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let add_time_label = if self.template_add_time_parameter {
            "ADD TIME PARAMETER: ON"
        } else {
            "ADD TIME PARAMETER: OFF"
        };
        let add_curve_label = if self.template_add_curve_parameter {
            "ADD CURVE PARAMETER: ON"
        } else {
            "ADD CURVE PARAMETER: OFF"
        };
        let selection_summary = self.selected_template_summary();

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
                    this.template_modal_open = false;
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
                    // Stop click propagation so clicking inside the modal doesn't close it
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
                        // Control bar: toggle buttons + OK + Close
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
                                            this.template_add_time_parameter =
                                                !this.template_add_time_parameter;
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
                                            this.apply_selected_templates(window, cx);
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
                                            this.template_add_curve_parameter =
                                                !this.template_add_curve_parameter;
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
                                            this.template_modal_open = false;
                                            cx.notify();
                                        }),
                                    ),
                            ),
                    )
                    .child(
                        // Template grid organized by category
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
                                    .child(self.render_template_tile(
                                        LayerEffectTemplateKind::HslaOverlay,
                                        cx,
                                    ))
                                    .child(self.render_template_tile(
                                        LayerEffectTemplateKind::Lut,
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
                                    .child(self.render_template_tile(
                                        LayerEffectTemplateKind::Opacity,
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
                                    .child(self.render_template_tile(
                                        LayerEffectTemplateKind::Sharpen,
                                        cx,
                                    ))
                                    .child(self.render_template_tile(
                                        LayerEffectTemplateKind::BlurGaussian,
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
                                    .child(self.render_template_tile(
                                        LayerEffectTemplateKind::TransitionFadeInOut,
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
            .into_any_element()
    }
}

impl Render for MotionLoomPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_script_input(window, cx);
        let selected_idx = self.selected_idx;
        let selected = self.current_clip().cloned();
        let imported_count = self.clips.len();
        let runtime_active = self.graph_runtime.is_some();
        let selected_name = selected
            .as_ref()
            .map(|clip| clip.name.clone())
            .unwrap_or_else(|| "No source clip selected".to_string());
        let selected_kind_label = selected
            .as_ref()
            .map(|clip| clip.kind.label().to_string())
            .unwrap_or_else(|| "Source".to_string());
        let selected_duration_label = selected
            .as_ref()
            .map(|clip| {
                if clip.duration > Duration::ZERO {
                    format!("{:.2}s", clip.duration.as_secs_f32())
                } else {
                    "Still".to_string()
                }
            })
            .unwrap_or_else(|| "-".to_string());
        let selected_path_label = selected
            .as_ref()
            .map(|clip| clip.path.clone())
            .unwrap_or_else(|| "Import an image or video clip to begin previewing.".to_string());

        // Evaluate graph runtime output for current frame
        let runtime_output = self
            .graph_runtime
            .as_ref()
            .map(|runtime| runtime.evaluate_frame(self.preview_frame));
        let runtime_target_size = self.runtime_target_size();
        let runtime_mix = runtime_output.as_ref().map(|o| o.invert_mix).unwrap_or(0.0);
        let runtime_brightness = 0.0_f32;
        let runtime_contrast = 1.0_f32;
        let runtime_saturation = 1.0_f32;
        let runtime_blur = runtime_output
            .as_ref()
            .map(|o| {
                o.layer_blur_sigma
                    .or_else(|| o.layer_sharpen_sigma.map(|v| -v))
                    .unwrap_or(0.0)
            })
            .unwrap_or(0.0);
        let runtime_opacity = 1.0_f32;

        // When any runtime effect is active, fall back to CPU rendering path
        // because VideoElement hardware path does not apply CPU-side effects.
        let has_active_effects = runtime_mix.abs() > 0.0001
            || runtime_brightness.abs() > 0.0001
            || (runtime_contrast - 1.0).abs() > 0.0001
            || (runtime_saturation - 1.0).abs() > 0.0001
            || runtime_blur.abs() > 0.05
            || (runtime_opacity - 1.0).abs() > 0.0001;
        let selected_prefers_video_element = selected
            .as_ref()
            .map(|clip| clip.kind == ImportedClipKind::Video && !has_active_effects)
            .unwrap_or(false);
        let selected_video_player = selected.as_ref().and_then(|clip| {
            if clip.kind != ImportedClipKind::Video || !selected_prefers_video_element {
                return None;
            }
            self.video_preview_player_for_stage(
                &clip.path,
                self.preview_frame,
                self.preview_playing,
            )
        });
        let selected_video_waiting_for_first_frame = selected_video_player
            .as_ref()
            .map(|player| player.last_frame_pts_ns() == 0)
            .unwrap_or(false);
        if let Some(player) = selected_video_player.as_ref()
            && (player.last_frame_pts_ns() == 0 || player.peek_frame_ready())
        {
            window.request_animation_frame();
        }

        // Build the script editor element (compact height to avoid page scroll)
        let script_input_elem = if let Some(input) = self.script_input.as_ref() {
            div()
                .w_full()
                .h(px(160.0))
                .rounded_md()
                .border_1()
                .border_color(white().opacity(0.18))
                .bg(rgb(0x0b1020))
                .overflow_hidden()
                .child(Input::new(input).h_full().w_full())
                .into_any_element()
        } else {
            div()
                .h(px(160.0))
                .w_full()
                .rounded_sm()
                .bg(white().opacity(0.05))
                .into_any_element()
        };

        // Import clip button
        let import_button = Self::control_button("Import Clip").on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_this, _, win, cx| {
                let rx = cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: true,
                    prompt: Some("Import clips into MotionLoom".into()),
                });
                cx.spawn_in(win, async move |view, window| {
                    let Ok(result) = rx.await else {
                        return;
                    };
                    let Some(paths) = result.ok().flatten() else {
                        return;
                    };

                    let _ = view.update_in(window, |this, _window, cx| {
                        let (ffmpeg_path, cache_root, can_generate_video_thumbnail) = {
                            let gs = this.global.read(cx);
                            (
                                gs.ffmpeg_path.clone(),
                                gs.cache_root_dir(),
                                gs.media_tools_ready_for_preview_gen(),
                            )
                        };

                        let mut imported = 0usize;
                        for path in paths {
                            let path_str = path.to_string_lossy().to_string();
                            if !Self::is_supported_clip_path(&path_str) {
                                continue;
                            }
                            if this.clips.iter().any(|item| item.path == path_str) {
                                continue;
                            }
                            let clip = Self::build_imported_clip(
                                &path_str,
                                &ffmpeg_path,
                                &cache_root,
                                can_generate_video_thumbnail,
                            );
                            this.clips.push(clip);
                            imported += 1;
                        }

                        if imported > 0 {
                            this.selected_idx = Some(this.clips.len().saturating_sub(1));
                            this.status_line =
                                format!("Imported {} clip(s) into MotionLoom Studio.", imported);
                        } else {
                            this.status_line =
                                "No new supported image/video clip was imported.".to_string();
                        }
                        cx.notify();
                    });
                })
                .detach();
            }),
        );

        // --- Left panel: source info, clip list ---
        let left_panel = div()
            .w(px(360.0))
            .flex_shrink_0()
            .h_full()
            .border_r_1()
            .border_color(white().opacity(0.12))
            .bg(rgb(0x090b12))
            .p_3()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .rounded_lg()
                    .border_1()
                    .border_color(white().opacity(0.14))
                    .bg(rgb(0x0d1320))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_lg()
                            .text_color(white().opacity(0.96))
                            .child("MotionLoom · VFX Studio(Under Development)"),
                    )
                    .child(
                        div().text_xs().text_color(white().opacity(0.72)).child(
                            "Preview the source and edit the MotionLoom graph side by side.",
                        ),
                    ),
            )
            .child(
                div()
                    .rounded_lg()
                    .border_1()
                    .border_color(white().opacity(0.12))
                    .bg(rgb(0x0c111b))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.92))
                            .child("Source"),
                    )
                    .child(
                        div().text_xs().text_color(white().opacity(0.68)).child(
                            "Import a still or video clip, then pick the active source below.",
                        ),
                    )
                    .child(import_button)
                    .child(
                        div()
                            .rounded_md()
                            .border_1()
                            .border_color(white().opacity(0.1))
                            .bg(rgb(0x0f1726))
                            .p_2()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.58))
                                    .child("Current source"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(white().opacity(0.94))
                                    .truncate()
                                    .child(selected_name.clone()),
                            )
                            .child(div().text_xs().text_color(white().opacity(0.6)).child(
                                format!(
                                    "{} · {} · {} imported",
                                    selected_kind_label, selected_duration_label, imported_count
                                ),
                            )),
                    ),
            )
            .child(
                // Imported clips list with selection
                div()
                    .flex_1()
                    .min_h_0()
                    .rounded_lg()
                    .border_1()
                    .border_color(white().opacity(0.12))
                    .bg(rgb(0x0c111b))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.92))
                            .child("Imported Clips"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.66))
                            .child("Pick the clip to send into the stage and code preview."),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.54))
                            .truncate()
                            .child(selected_path_label.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scrollbar()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .children(self.clips.iter().enumerate().map(|(idx, clip)| {
                                let active = self.selected_idx == Some(idx);
                                let idx_for_select = idx;
                                let duration_label = if clip.duration > Duration::ZERO {
                                    format!("{:.2}s", clip.duration.as_secs_f32())
                                } else {
                                    "Still".to_string()
                                };
                                div()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(white().opacity(if active { 0.35 } else { 0.14 }))
                                    .bg(if active { rgb(0x1f2937) } else { rgb(0x111827) })
                                    .px_2()
                                    .py_2()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(white().opacity(0.09)))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(white().opacity(0.6))
                                            .child(clip.kind.label()),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(white().opacity(0.93))
                                            .truncate()
                                            .child(clip.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(white().opacity(0.6))
                                            .truncate()
                                            .child(duration_label),
                                    )
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            this.selected_idx = Some(idx_for_select);
                                            cx.notify();
                                        }),
                                    )
                            })),
                    ),
            );

        // --- Preview card: VideoElement for video clips without effects, CPU path for effects ---
        let video_element_preview = if let Some(clip) = selected.as_ref() {
            if clip.kind == ImportedClipKind::Video && !has_active_effects {
                if let Some(player) = selected_video_player.clone() {
                    let video_element = VideoElement::new(player)
                        .preview_transform(1.0, 0.0, 0.0, PREVIEW_BOX_W, PREVIEW_BOX_H)
                        .color_balance(runtime_brightness, runtime_contrast, runtime_saturation)
                        .blur_sigma(runtime_blur)
                        .opacity({
                            #[cfg(target_os = "macos")]
                            {
                                1.0
                            }
                            #[cfg(not(target_os = "macos"))]
                            {
                                runtime_opacity
                            }
                        });
                    Some(
                        div()
                            .w_full()
                            .flex_1()
                            .min_h_0()
                            .rounded_lg()
                            .border_1()
                            .border_color(white().opacity(0.14))
                            .bg(rgb(0x05070c))
                            .overflow_hidden()
                            .child(video_element)
                            .into_any_element(),
                    )
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let preview_card = if let Some(preview) = video_element_preview {
            preview
        } else if let Some(clip) = selected {
            if let Some(preview) = clip.preview {
                let mut source_w = preview.width;
                let mut source_h = preview.height;
                let mut source_bgra = preview.bgra.as_ref().to_vec();
                if clip.kind == ImportedClipKind::Video
                    && let Some((bgra, w, h)) =
                        self.video_preview_frame_bgra(&clip.path, self.preview_frame)
                {
                    source_bgra = bgra;
                    source_w = w;
                    source_h = h;
                }
                let (display_image, display_w, display_h) = if runtime_active {
                    let idx = selected_idx.unwrap_or(0);
                    let size = runtime_target_size.unwrap_or((preview.width, preview.height));
                    self.runtime_preview_image(
                        idx,
                        &source_bgra,
                        source_w,
                        source_h,
                        preview.image.clone(),
                        self.preview_frame,
                        runtime_mix,
                        runtime_brightness,
                        runtime_contrast,
                        runtime_saturation,
                        runtime_blur,
                        runtime_opacity,
                        size,
                    )
                } else {
                    (
                        Self::render_image_from_bgra(source_w, source_h, source_bgra)
                            .unwrap_or_else(|_| preview.image.clone()),
                        source_w,
                        source_h,
                    )
                };
                div()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .rounded_lg()
                    .border_1()
                    .border_color(white().opacity(0.14))
                    .bg(rgb(0x05070c))
                    .overflow_hidden()
                    .child(FitPreviewImageElement::new(
                        display_image,
                        display_w,
                        display_h,
                    ))
                    .into_any_element()
            } else if clip.kind == ImportedClipKind::Video
                && let Some((bgra, w, h)) =
                    self.video_preview_frame_bgra(&clip.path, self.preview_frame)
            {
                let (display_image, display_w, display_h) = if runtime_active {
                    let idx = selected_idx.unwrap_or(0);
                    let size = runtime_target_size.unwrap_or((w, h));
                    let fallback = Self::render_image_from_bgra(w, h, bgra.clone())
                        .unwrap_or_else(|_| Arc::new(RenderImage::new(SmallVec::new())));
                    self.runtime_preview_image(
                        idx,
                        &bgra,
                        w,
                        h,
                        fallback,
                        self.preview_frame,
                        runtime_mix,
                        runtime_brightness,
                        runtime_contrast,
                        runtime_saturation,
                        runtime_blur,
                        runtime_opacity,
                        size,
                    )
                } else {
                    (
                        Self::render_image_from_bgra(w, h, bgra)
                            .unwrap_or_else(|_| Arc::new(RenderImage::new(SmallVec::new()))),
                        w,
                        h,
                    )
                };
                div()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .rounded_lg()
                    .border_1()
                    .border_color(white().opacity(0.14))
                    .bg(rgb(0x05070c))
                    .overflow_hidden()
                    .child(FitPreviewImageElement::new(
                        display_image,
                        display_w,
                        display_h,
                    ))
                    .into_any_element()
            } else {
                let no_preview_message = if clip.kind == ImportedClipKind::Video
                    && selected_video_waiting_for_first_frame
                {
                    "Loading video preview frame...".to_string()
                } else {
                    clip.error
                        .unwrap_or_else(|| "No preview available for this clip.".to_string())
                };
                div()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .rounded_lg()
                    .border_1()
                    .border_color(white().opacity(0.14))
                    .bg(rgb(0x05070c))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.68))
                            .child(no_preview_message),
                    )
                    .into_any_element()
            }
        } else {
            div()
                .w_full()
                .flex_1()
                .min_h_0()
                .rounded_lg()
                .border_1()
                .border_color(white().opacity(0.14))
                .bg(rgb(0x05070c))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(white().opacity(0.55))
                        .child("Import a clip to start the VFX stage."),
                )
                .into_any_element()
        };

        // --- Graph Lab panel: script editor + Apply/Template buttons ---
        let graph_lab_panel = div()
            .w_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .rounded_lg()
                    .border_1()
                    .border_color(white().opacity(0.12))
                    .bg(rgb(0x0c111b))
                    .p_3()
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
                                    .text_color(white().opacity(0.94))
                                    .child("Graph Lab"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(Self::control_button("Template Picker").on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            this.open_template_modal();
                                            cx.notify();
                                        }),
                                    ))
                                    .child(Self::control_button("Apply Effect").on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            this.apply_script_command(cx);
                                            cx.notify();
                                        }),
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.68))
                            .child("Edit the MotionLoom graph here and run it directly on the current stage preview."),
                    )
                    .child(script_input_elem),
            );

        // --- Template picker modal overlay (rendered on top when open) ---
        let template_modal = if self.template_modal_open {
            Some(self.render_template_modal_overlay(cx))
        } else {
            None
        };

        // --- Main layout: left panel + right content (no scroll, fit to window) ---
        div()
            .size_full()
            .bg(rgb(0x080a10))
            .flex()
            .child(left_panel)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    // VFX Stage: preview fills remaining vertical space
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .rounded_lg()
                            .border_1()
                            .border_color(white().opacity(0.12))
                            .bg(rgb(0x0c111b))
                            .p_3()
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
                                            .flex()
                                            .items_center()
                                            .gap_3()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(white().opacity(0.95))
                                                    .child("VFX Stage"),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(white().opacity(0.55))
                                                    .child(format!(
                                                        "{} · {} · {}",
                                                        selected_name,
                                                        selected_kind_label,
                                                        selected_duration_label
                                                    )),
                                            ),
                                    )
                                    // Playback controls inline with VFX Stage header
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(white().opacity(0.75))
                                                    .child("Frame"),
                                            )
                                            .child(
                                                Self::control_button(if self.preview_playing {
                                                    "Pause"
                                                } else {
                                                    "Play"
                                                })
                                                .on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(move |this, _, window, cx| {
                                                        this.toggle_preview_playback(window, cx);
                                                    }),
                                                ),
                                            )
                                            .child(Self::control_button("-1").on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(move |this, _, _, cx| {
                                                    this.step_preview_frame(-1);
                                                    cx.notify();
                                                }),
                                            ))
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(white().opacity(0.9))
                                                    .child(format!("{}", self.preview_frame)),
                                            )
                                            .child(Self::control_button("+1").on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(move |this, _, _, cx| {
                                                    this.step_preview_frame(1);
                                                    cx.notify();
                                                }),
                                            )),
                                    ),
                            )
                            // Preview fills remaining space in the VFX Stage card
                            .child(preview_card)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.66))
                                    .truncate()
                                    .child(self.status_line.clone()),
                            ),
                    )
                    // Graph Lab: fixed-height section at bottom
                    .child(graph_lab_panel),
            )
            .when(self.template_modal_open, |el| {
                el.child(template_modal.unwrap())
            })
            .when(self.preview_playing, |el| {
                window.request_animation_frame();
                el
            })
    }
}
