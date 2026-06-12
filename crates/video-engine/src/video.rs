// =========================================
// =========================================
// crates/video-engine/src/video.rs
// Note: use simple Vec<u8> for RGBA data

use crate::Error;
#[cfg(target_os = "macos")]
use core_foundation::base::{CFType, TCFType};
#[cfg(target_os = "macos")]
use core_foundation::boolean::CFBoolean;
#[cfg(target_os = "macos")]
use core_foundation::dictionary::CFDictionary;
#[cfg(target_os = "macos")]
use core_foundation::string::CFString;
#[cfg(target_os = "macos")]
use core_video::pixel_buffer::{
    CVPixelBuffer, CVPixelBufferKeys, kCVPixelFormatType_32BGRA,
    kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
};
#[cfg(target_os = "macos")]
use core_video::r#return::kCVReturnSuccess;
use gst::message::MessageView;
use gstreamer as gst;
use gstreamer_app as gst_app;
use gstreamer_app::prelude::*;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::*;
use parking_lot::{Mutex, RwLock};
use std::collections::VecDeque;
use std::io::Read;
#[cfg(target_os = "macos")]
use std::ops::ControlFlow;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
#[repr(C)]
struct GstCoreVideoMetaRaw {
    meta: gst::ffi::GstMeta,
    pixel_buffer: core_video::pixel_buffer::CVPixelBufferRef,
}

/// Position in the media.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Position {
    /// Position based on time.
    Time(Duration),
    /// Position based on nth frame.
    Frame(u64),
}

impl From<Position> for gst::GenericFormattedValue {
    fn from(pos: Position) -> Self {
        match pos {
            Position::Time(t) => gst::ClockTime::from_nseconds(t.as_nanos() as _).into(),
            Position::Frame(f) => gst::format::Default::from_u64(f).into(),
        }
    }
}

impl From<Duration> for Position {
    fn from(t: Duration) -> Self {
        Position::Time(t)
    }
}

impl From<u64> for Position {
    fn from(f: u64) -> Self {
        Position::Frame(f)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Frame {
    Gst(gst::Sample),
    RawBgra {
        data: Arc<Vec<u8>>,
        width: u32,
        height: u32,
    },
}

impl Frame {
    pub fn empty() -> Self {
        Self::Gst(gst::Sample::builder().build())
    }

    pub fn empty_raw_bgra() -> Self {
        // FFmpeg preview must not construct GStreamer objects before gst::init.
        Self::RawBgra {
            data: Arc::new(Vec::new()),
            width: 0,
            height: 0,
        }
    }

    pub fn readable(&'_ self) -> Option<gst::BufferMap<'_, gst::buffer::Readable>> {
        match self {
            Self::Gst(sample) => sample.buffer().and_then(|x| x.map_readable().ok()),
            Self::RawBgra { .. } => None,
        }
    }

    pub fn raw_bgra(&self) -> Option<(&[u8], u32, u32)> {
        match self {
            Self::RawBgra {
                data,
                width,
                height,
            } => Some((data.as_ref().as_slice(), *width, *height)),
            Self::Gst(_) => None,
        }
    }
}

const PLAYBIN_FLAGS_VIDEO_ONLY: u32 = 0x1;
const PLAYBIN_FLAGS_AUDIO_ONLY: u32 = 0x2;
static VIDEO_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub(crate) struct FfmpegControl {
    paused: Arc<AtomicBool>,
    muted: Arc<AtomicBool>,
    volume: Arc<AtomicU64>,
    position_ns: Arc<AtomicU64>,
    seek_request_ns: Arc<AtomicU64>,
}

/// Options for initializing a `Video`.
#[derive(Debug, Clone)]
pub struct VideoOptions {
    /// Optional initial frame buffer capacity (0 disables buffering). Defaults to 3.
    pub frame_buffer_capacity: Option<usize>,
    /// Optional initial looping flag. Defaults to false.
    pub looping: Option<bool>,
    /// Optional initial playback speed. Defaults to 1.0.
    pub speed: Option<f64>,
    /// Optional preview decode scale (e.g. 0.5 halves width/height). Defaults to None (no scale).
    pub preview_scale: Option<f32>,
    /// Optional preview max dimension (applies to the larger side). Defaults to None.
    pub preview_max_dim: Option<u32>,
    /// Optional exact preview output size. When set, the GStreamer pipeline inserts videoscale.
    pub preview_width: Option<u32>,
    pub preview_height: Option<u32>,
    /// Optional preview framerate cap (e.g. 15/20/30). Defaults to None (keep source fps).
    pub preview_fps: Option<u32>,
    /// Optional appsink queue length for preview frames. Defaults to 1.
    pub appsink_max_buffers: Option<u32>,
    /// If true on macOS, prefer NV12 surface decode for `paint_surface`.
    /// Set false to force BGRA decode path (safer across varied proxy sources).
    pub prefer_surface: bool,
    /// macOS-only strict mode used by proxy+NV12 preview:
    /// keep surface paint path and avoid image fallback in renderer.
    pub strict_surface_proxy_nv12: bool,
    /// Benchmark-only path: feed appsink directly without a fixed output caps string.
    /// This lets upstream negotiation choose the decode format and may break rendering.
    pub benchmark_raw_appsink: bool,
    /// If true, the pipeline will discard video data (fakesink) and only play audio.
    /// This drastically reduces CPU usage for background clips.
    pub is_audio_only: bool,
}

impl Default for VideoOptions {
    fn default() -> Self {
        Self {
            frame_buffer_capacity: Some(3),
            looping: Some(false),
            speed: Some(1.0),
            preview_scale: None,
            preview_max_dim: None,
            preview_width: None,
            preview_height: None,
            preview_fps: None,
            appsink_max_buffers: Some(1),
            prefer_surface: true,
            strict_surface_proxy_nv12: false,
            benchmark_raw_appsink: false,
            is_audio_only: false,
        }
    }
}

impl VideoOptions {
    /// Hidden runtime toggle for raw appsink benchmarking.
    /// Set `ANICA_BENCHMARK_RAW_APPSINK=1` to bypass fixed preview caps.
    pub fn benchmark_raw_appsink_from_env() -> bool {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| {
            std::env::var("ANICA_BENCHMARK_RAW_APPSINK")
                .map(|value| {
                    !matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "" | "0" | "false" | "off" | "no"
                    )
                })
                .unwrap_or(false)
        })
    }
}

#[derive(Debug)]
#[allow(unused)]
pub(crate) struct Internal {
    pub(crate) id: u64,
    pub(crate) bus: Option<gst::Bus>,
    pub(crate) source: Option<gst::Pipeline>,
    pub(crate) ffmpeg: Option<FfmpegControl>,
    pub(crate) alive: Arc<AtomicBool>,
    pub(crate) worker: Option<std::thread::JoinHandle<()>>,

    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) framerate: f64,
    pub(crate) duration: Duration,
    pub(crate) speed: Arc<AtomicU64>,

    pub(crate) frame: Arc<Mutex<Frame>>,
    pub(crate) upload_frame: Arc<AtomicBool>,
    pub(crate) frame_buffer: Arc<Mutex<VecDeque<Frame>>>,
    pub(crate) frame_buffer_capacity: Arc<AtomicUsize>,
    pub(crate) last_frame_time: Arc<Mutex<Instant>>,
    pub(crate) looping: Arc<AtomicBool>,
    pub(crate) is_eos: Arc<AtomicBool>,
    pub(crate) restart_stream: bool,

    pub(crate) subtitle_text: Arc<Mutex<Option<String>>>,
    pub(crate) upload_text: Arc<AtomicBool>,
    pub(crate) last_frame_pts_ns: Arc<AtomicU64>,
    pub(crate) decoded_frame_counter: Arc<AtomicU64>,

    // Optional display size overrides. If only one is set, the other is
    // inferred using the natural aspect ratio (width / height).
    pub(crate) display_width_override: Option<u32>,
    pub(crate) display_height_override: Option<u32>,
    pub(crate) strict_surface_proxy_nv12: bool,
}

impl Internal {
    pub(crate) fn seek(&self, position: impl Into<Position>, accurate: bool) -> Result<(), Error> {
        let position = position.into();
        if let Some(ffmpeg) = self.ffmpeg.as_ref() {
            let ns = match position {
                Position::Time(t) => t.as_nanos().min(u128::from(u64::MAX)) as u64,
                Position::Frame(f) => {
                    let fps = self.framerate.max(1.0);
                    ((f as f64 / fps) * 1_000_000_000.0).max(0.0) as u64
                }
            };
            ffmpeg.position_ns.store(ns, Ordering::SeqCst);
            ffmpeg
                .seek_request_ns
                .store(ns.saturating_add(1), Ordering::SeqCst);
            self.is_eos.store(false, Ordering::SeqCst);
            self.frame_buffer.lock().clear();
            self.upload_frame.store(false, Ordering::SeqCst);
            self.last_frame_pts_ns.store(0, Ordering::SeqCst);
            return Ok(());
        }
        let Some(source) = self.source.as_ref() else {
            return Err(Error::Ffmpeg("missing playback backend".to_string()));
        };
        let current_speed = f64::from_bits(self.speed.load(Ordering::SeqCst));

        // Clear EOS so the worker resumes pulling after a seek.
        self.is_eos.store(false, Ordering::SeqCst);

        // Build seek flags. When not accurate, snap in the playback direction to
        // avoid jumping backward to a previous keyframe.
        let mut flags = gst::SeekFlags::FLUSH;
        if accurate {
            flags |= gst::SeekFlags::ACCURATE;
        } else {
            flags |= gst::SeekFlags::KEY_UNIT;
            if current_speed >= 0.0 {
                flags |= gst::SeekFlags::SNAP_AFTER;
            } else {
                flags |= gst::SeekFlags::SNAP_BEFORE;
            }
        }

        match &position {
            Position::Time(_) => source.seek(
                current_speed,
                flags,
                gst::SeekType::Set,
                gst::GenericFormattedValue::from(position),
                gst::SeekType::None,
                gst::ClockTime::NONE,
            )?,
            Position::Frame(_) => source.seek(
                current_speed,
                flags,
                gst::SeekType::Set,
                gst::GenericFormattedValue::from(position),
                gst::SeekType::None,
                gst::format::Default::NONE,
            )?,
        };

        *self.subtitle_text.lock() = None;
        self.upload_text.store(true, Ordering::SeqCst);

        // Clear any buffered frames so old frames do not display after a seek,
        // which can visually appear as a larger-than-intended jump.
        self.frame_buffer.lock().clear();
        self.upload_frame.store(false, Ordering::SeqCst);
        self.last_frame_pts_ns.store(0, Ordering::SeqCst);

        Ok(())
    }

    pub(crate) fn set_speed(&mut self, speed: f64) -> Result<(), Error> {
        if self.ffmpeg.is_some() {
            self.speed.store(speed.to_bits(), Ordering::SeqCst);
            return Ok(());
        }
        let Some(source) = self.source.as_ref() else {
            return Err(Error::Ffmpeg("missing playback backend".to_string()));
        };
        let Some(position) = source.query_position::<gst::ClockTime>() else {
            return Err(Error::Caps);
        };
        if speed > 0.0 {
            source.seek(
                speed,
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::SeekType::Set,
                position,
                gst::SeekType::End,
                gst::ClockTime::from_seconds(0),
            )?;
        } else {
            source.seek(
                speed,
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::SeekType::Set,
                gst::ClockTime::from_seconds(0),
                gst::SeekType::Set,
                position,
            )?;
        }
        self.speed.store(speed.to_bits(), Ordering::SeqCst);
        Ok(())
    }

    pub(crate) fn restart_stream(&mut self) -> Result<(), Error> {
        self.is_eos.store(false, Ordering::SeqCst);
        self.set_paused(false);
        self.seek(0, false)?;
        Ok(())
    }

    pub(crate) fn set_paused(&mut self, paused: bool) {
        if let Some(ffmpeg) = self.ffmpeg.as_ref() {
            ffmpeg.paused.store(paused, Ordering::SeqCst);
            return;
        }
        let Some(source) = self.source.as_ref() else {
            return;
        };
        // Avoid forcing repeated identical transitions.
        // Keep this strict: if backend is not yet at target state, re-issue transition.
        // This favors transport reliability over minimal state-call count.
        let target_state = if paused {
            gst::State::Paused
        } else {
            gst::State::Playing
        };
        let (_, current_state, pending_state) = source.state(gst::ClockTime::from_mseconds(5));
        let already_at_target = current_state == target_state
            && (pending_state == gst::State::VoidPending || pending_state == target_state);
        if already_at_target {
            return;
        }

        // Never panic on state transitions; log and continue so a transient backend error does not crash UI.
        if let Err(err) = source.set_state(target_state) {
            log::error!(
                "[Video {}] failed set_state {:?} from {:?}: {}",
                self.id,
                target_state,
                current_state,
                err
            );
            return;
        }

        if self.is_eos.load(Ordering::Acquire) && !paused {
            self.restart_stream = true;
        }
    }

    pub(crate) fn paused(&self) -> bool {
        if let Some(ffmpeg) = self.ffmpeg.as_ref() {
            return ffmpeg.paused.load(Ordering::SeqCst);
        }
        let Some(source) = self.source.as_ref() else {
            return true;
        };
        // Treat pending transitions explicitly to keep transport logic stable while
        // GStreamer is asynchronously switching states.
        let (_, current_state, pending_state) = source.state(gst::ClockTime::ZERO);
        if pending_state == gst::State::Playing {
            return false;
        }
        if pending_state == gst::State::Paused {
            return true;
        }
        current_state != gst::State::Playing
    }

    pub(crate) fn set_blur_sigma(&self, _sigma: f64) {
        // Blur is renderer-driven (Metal/CPU path), no longer controlled by
        // GStreamer caps downscale/upscale pipeline stages.
    }
}

/// A multimedia video loaded from a URI (e.g., a local file path or HTTP stream).
#[derive(Debug, Clone)]
pub struct Video(pub(crate) Arc<RwLock<Internal>>);

impl Drop for Video {
    fn drop(&mut self) {
        // Only cleanup if this is the last reference
        if Arc::strong_count(&self.0) == 1
            && let Some(mut inner) = self.0.try_write()
        {
            if let Some(source) = inner.source.as_ref() {
                // Drop should be best-effort; do not panic even if backend rejects state transition.
                if let Err(err) = source.set_state(gst::State::Null) {
                    log::error!(
                        "[Video {}] failed set_state Null in drop: {}",
                        inner.id,
                        err
                    );
                }
            }

            inner.alive.store(false, Ordering::SeqCst);
            if let Some(worker) = inner.worker.take()
                && let Err(err) = worker.join()
            {
                match err.downcast_ref::<String>() {
                    Some(e) => log::error!("Video thread panicked: {e}"),
                    None => log::error!("Video thread panicked with unknown reason"),
                }
            }
        }
    }
}

impl Video {
    fn visual_ffmpeg_backend_enabled() -> bool {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| {
            !std::env::var("ANICA_VIDEO_BACKEND")
                .map(|raw| raw.trim().eq_ignore_ascii_case("gstreamer"))
                .unwrap_or(false)
        })
    }

    fn ffmpeg_preview_debug_enabled() -> bool {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| {
            std::env::var("ANICA_DEBUG_FFMPEG_PREVIEW")
                .ok()
                .map(|raw| {
                    let value = raw.trim();
                    value == "1"
                        || value.eq_ignore_ascii_case("true")
                        || value.eq_ignore_ascii_case("yes")
                        || value.eq_ignore_ascii_case("on")
                })
                .unwrap_or(false)
        })
    }

    fn gstreamer_backend_enabled() -> bool {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| {
            let env_truthy = |name: &str| {
                std::env::var(name)
                    .ok()
                    .map(|raw| {
                        let value = raw.trim();
                        value == "1"
                            || value.eq_ignore_ascii_case("true")
                            || value.eq_ignore_ascii_case("yes")
                            || value.eq_ignore_ascii_case("on")
                    })
                    .unwrap_or(false)
            };
            env_truthy("ANICA_ENABLE_GSTREAMER")
                || std::env::var("ANICA_VIDEO_BACKEND")
                    .ok()
                    .map(|raw| {
                        let value = raw.trim();
                        value.eq_ignore_ascii_case("gstreamer") || value.eq_ignore_ascii_case("gst")
                    })
                    .unwrap_or(false)
        })
    }

    fn ffmpeg_path_from_env() -> String {
        std::env::var("ANICA_FFMPEG_PATH")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "ffmpeg".to_string())
    }

    fn ffprobe_path_from_ffmpeg(ffmpeg_path: &str) -> String {
        let lower = ffmpeg_path.to_ascii_lowercase();
        if lower.ends_with("ffmpeg.exe") {
            format!(
                "{}ffprobe.exe",
                &ffmpeg_path[..ffmpeg_path.len() - "ffmpeg.exe".len()]
            )
        } else if lower.ends_with("ffmpeg") {
            format!(
                "{}ffprobe",
                &ffmpeg_path[..ffmpeg_path.len() - "ffmpeg".len()]
            )
        } else {
            "ffprobe".to_string()
        }
    }

    fn parse_ratio_fps(raw: &str) -> Option<f64> {
        let trimmed = raw.trim();
        if let Some((num, den)) = trimmed.split_once('/') {
            let num = num.parse::<f64>().ok()?;
            let den = den.parse::<f64>().ok()?;
            if den.abs() > f64::EPSILON {
                return Some(num / den);
            }
            return None;
        }
        trimmed.parse::<f64>().ok()
    }

    fn ffprobe_video_info(
        ffmpeg_path: &str,
        path: &str,
    ) -> Result<(u32, u32, f64, Duration), Error> {
        let ffprobe = Self::ffprobe_path_from_ffmpeg(ffmpeg_path);
        let output = Command::new(&ffprobe)
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=width,height,r_frame_rate,duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
                path,
            ])
            .output()
            .map_err(|err| Error::Ffmpeg(format!("failed to execute ffprobe: {err}")))?;
        if !output.status.success() {
            return Err(Error::Ffmpeg(format!(
                "ffprobe failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let mut lines = text.lines();
        let width = lines
            .next()
            .and_then(|v| v.trim().parse::<u32>().ok())
            .unwrap_or(0);
        let height = lines
            .next()
            .and_then(|v| v.trim().parse::<u32>().ok())
            .unwrap_or(0);
        let fps = lines.next().and_then(Self::parse_ratio_fps).unwrap_or(30.0);
        let duration = lines
            .next()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
            .map(Duration::from_secs_f64)
            .unwrap_or(Duration::ZERO);
        if width == 0 || height == 0 {
            return Err(Error::Ffmpeg(
                "ffprobe returned invalid video size".to_string(),
            ));
        }
        Ok((width, height, fps.clamp(1.0, 240.0), duration))
    }

    fn ffmpeg_decode_args(
        path: &str,
        start: Duration,
        width: u32,
        height: u32,
        fps: u32,
        is_image: bool,
        use_hwaccel: bool,
    ) -> Vec<String> {
        let mut args = vec![
            "-hide_banner".to_string(),
            "-nostats".to_string(),
            "-loglevel".to_string(),
            "error".to_string(),
        ];
        if use_hwaccel && !is_image {
            args.extend(["-hwaccel".to_string(), Self::ffmpeg_hwaccel_name()]);
        }
        if is_image {
            args.extend(["-loop".to_string(), "1".to_string()]);
        } else {
            args.extend(["-ss".to_string(), format!("{:.6}", start.as_secs_f64())]);
        }
        args.extend(["-i".to_string(), path.to_string(), "-an".to_string()]);
        let vf = format!("fps={fps},scale={width}:{height}:flags=fast_bilinear,format=bgra");
        args.extend([
            "-vf".to_string(),
            vf,
            "-pix_fmt".to_string(),
            "bgra".to_string(),
            "-f".to_string(),
            "rawvideo".to_string(),
            "pipe:1".to_string(),
        ]);
        args
    }

    fn ffmpeg_hwaccel_name() -> String {
        std::env::var("ANICA_FFMPEG_HWACCEL_NAME")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                if cfg!(target_os = "windows") {
                    "d3d11va".to_string()
                } else if cfg!(target_os = "macos") {
                    "videotoolbox".to_string()
                } else {
                    "auto".to_string()
                }
            })
    }

    #[cfg(target_os = "windows")]
    fn ffmpeg_dxva_direct_texture_requested() -> bool {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| {
            std::env::var("ANICA_FFMPEG_DXVA_DIRECT_TEXTURE")
                .ok()
                .map(|raw| {
                    let value = raw.trim();
                    value == "1"
                        || value.eq_ignore_ascii_case("true")
                        || value.eq_ignore_ascii_case("yes")
                        || value.eq_ignore_ascii_case("on")
                })
                .unwrap_or(false)
        })
    }

    fn ffmpeg_hwaccel_enabled() -> bool {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| {
            let default_enabled = cfg!(any(target_os = "macos", target_os = "windows"));
            std::env::var("ANICA_FFMPEG_HWACCEL")
                .ok()
                .map(|raw| {
                    let value = raw.trim();
                    !(value == "0"
                        || value.eq_ignore_ascii_case("false")
                        || value.eq_ignore_ascii_case("no")
                        || value.eq_ignore_ascii_case("off")
                        || value.eq_ignore_ascii_case("none"))
                })
                .unwrap_or(default_enabled)
        })
    }

    fn frame_deadline(started_at: Instant, frame_duration: Duration, frame_index: u64) -> Instant {
        started_at + Duration::from_secs_f64(frame_duration.as_secs_f64() * frame_index as f64)
    }

    fn new_ffmpeg_visual(uri: &url::Url, options: VideoOptions) -> Result<Self, Error> {
        let path = uri
            .to_file_path()
            .map_err(|_| Error::Ffmpeg("ffmpeg preview requires a local file".to_string()))?;
        let path_str = path.to_string_lossy().to_string();
        let ffmpeg_path = Self::ffmpeg_path_from_env();
        let (src_w, src_h, source_fps, source_duration) =
            Self::ffprobe_video_info(&ffmpeg_path, &path_str)?;
        let fps = options
            .preview_fps
            .filter(|fps| *fps > 0)
            .unwrap_or_else(|| source_fps.round().clamp(1.0, 60.0) as u32)
            .clamp(1, 120);
        let (mut width, mut height) = options
            .preview_width
            .zip(options.preview_height)
            .filter(|(w, h)| *w >= 2 && *h >= 2)
            .unwrap_or_else(|| {
                if let Some(max_dim) = options.preview_max_dim.filter(|v| *v >= 2) {
                    let max_src = src_w.max(src_h);
                    if max_src > max_dim {
                        let scale = max_dim as f32 / max_src as f32;
                        let w = ((src_w as f32 * scale).round() as u32).max(2);
                        let h = ((src_h as f32 * scale).round() as u32).max(2);
                        return (w, h);
                    }
                }
                (src_w, src_h)
            });
        #[cfg(target_os = "macos")]
        {
            // macOS preview renders video through NV12 IOSurface, which requires even plane sizes.
            width = (width.max(2) + 1) & !1;
            height = (height.max(2) + 1) & !1;
        }
        let duration = if source_duration.is_zero() && Self::is_image_uri(uri) {
            Duration::from_secs(24 * 60 * 60)
        } else {
            source_duration
        };
        let id = VIDEO_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        #[cfg(target_os = "windows")]
        if Self::ffmpeg_dxva_direct_texture_requested() {
            log::warn!(
                "[Video {id}][FFmpeg] ANICA_FFMPEG_DXVA_DIRECT_TEXTURE requested, but the current FFmpeg CLI rawvideo backend cannot export ID3D11Texture2D frames; using d3d11va decode plus BGRA pipe fallback"
            );
        }
        let frame = Arc::new(Mutex::new(Frame::empty_raw_bgra()));
        let upload_frame = Arc::new(AtomicBool::new(false));
        let frame_buffer = Arc::new(Mutex::new(VecDeque::new()));
        let frame_buffer_capacity = Arc::new(AtomicUsize::new(0));
        let alive = Arc::new(AtomicBool::new(true));
        let last_frame_time = Arc::new(Mutex::new(Instant::now()));
        let looping = Arc::new(AtomicBool::new(options.looping.unwrap_or(false)));
        let is_eos = Arc::new(AtomicBool::new(false));
        let speed = Arc::new(AtomicU64::new(options.speed.unwrap_or(1.0).to_bits()));
        let subtitle_text = Arc::new(Mutex::new(None));
        let upload_text = Arc::new(AtomicBool::new(false));
        let last_frame_pts_ns = Arc::new(AtomicU64::new(0));
        let decoded_frame_counter = Arc::new(AtomicU64::new(0));
        let paused = Arc::new(AtomicBool::new(false));
        let muted = Arc::new(AtomicBool::new(false));
        let volume = Arc::new(AtomicU64::new(1.0f64.to_bits()));
        let position_ns = Arc::new(AtomicU64::new(0));
        let seek_request_ns = Arc::new(AtomicU64::new(0));

        let frame_ref = Arc::clone(&frame);
        let upload_frame_ref = Arc::clone(&upload_frame);
        let frame_buffer_ref = Arc::clone(&frame_buffer);
        let frame_buffer_capacity_ref = Arc::clone(&frame_buffer_capacity);
        let alive_ref = Arc::clone(&alive);
        let last_frame_time_ref = Arc::clone(&last_frame_time);
        let looping_ref = Arc::clone(&looping);
        let is_eos_ref = Arc::clone(&is_eos);
        let last_frame_pts_ns_ref = Arc::clone(&last_frame_pts_ns);
        let decoded_frame_counter_ref = Arc::clone(&decoded_frame_counter);
        let paused_ref = Arc::clone(&paused);
        let position_ns_ref = Arc::clone(&position_ns);
        let seek_request_ns_ref = Arc::clone(&seek_request_ns);
        let is_image = Self::is_image_uri(uri);
        let frame_bytes = width as usize * height as usize * 4;
        let frame_duration = Duration::from_secs_f64(1.0 / f64::from(fps));
        let duration_ns = duration.as_nanos().min(u128::from(u64::MAX)) as u64;
        let debug_ffmpeg_preview = Self::ffmpeg_preview_debug_enabled();

        let worker = std::thread::spawn(move || {
            let mut stream_seek_marker = seek_request_ns_ref.load(Ordering::Acquire);
            while alive_ref.load(Ordering::Acquire) {
                if paused_ref.load(Ordering::Acquire) {
                    let marker = seek_request_ns_ref.load(Ordering::Acquire);
                    if marker != stream_seek_marker {
                        stream_seek_marker = marker;
                        let stream_start = Duration::from_nanos(marker.saturating_sub(1));
                        let _ = Self::decode_one_ffmpeg_frame(
                            &ffmpeg_path,
                            &path_str,
                            stream_start,
                            width,
                            height,
                            fps,
                            is_image,
                            &frame_ref,
                            &upload_frame_ref,
                            &last_frame_time_ref,
                            &last_frame_pts_ns_ref,
                            &decoded_frame_counter_ref,
                        );
                    }
                    std::thread::sleep(Duration::from_millis(12));
                    continue;
                }

                let start_ns = position_ns_ref.load(Ordering::Acquire);
                let stream_start = Duration::from_nanos(start_ns);
                stream_seek_marker = seek_request_ns_ref.load(Ordering::Acquire);
                let mut use_hwaccel = Self::ffmpeg_hwaccel_enabled() && !is_image;
                let hwaccel_name = Self::ffmpeg_hwaccel_name();
                loop {
                    let args = Self::ffmpeg_decode_args(
                        &path_str,
                        stream_start,
                        width,
                        height,
                        fps,
                        is_image,
                        use_hwaccel,
                    );
                    let spawn_started = Instant::now();
                    let mut child = match Command::new(&ffmpeg_path)
                        .args(&args)
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .spawn()
                    {
                        Ok(child) => child,
                        Err(err) => {
                            log::error!("[Video {id}][FFmpeg] spawn failed: {err}");
                            std::thread::sleep(Duration::from_millis(120));
                            break;
                        }
                    };
                    if debug_ffmpeg_preview {
                        log::info!(
                            "[Video {id}][FFmpegPreview] spawn_ms={:.2} start={:.3}s frame={}x{} fps={} hwaccel={} hwaccel_name={}",
                            spawn_started.elapsed().as_secs_f64() * 1000.0,
                            stream_start.as_secs_f64(),
                            width,
                            height,
                            fps,
                            use_hwaccel,
                            hwaccel_name
                        );
                    }
                    let Some(mut stdout) = child.stdout.take() else {
                        let _ = child.kill();
                        std::thread::sleep(Duration::from_millis(120));
                        break;
                    };
                    let mut frame_index = 0u64;
                    let mut dropped_since_publish = 0u32;
                    let mut read_buffer = vec![0u8; frame_bytes];
                    let stream_wall_start = Instant::now();
                    let mut metric_started = Instant::now();
                    let mut metric_frames = 0u64;
                    let mut metric_dropped = 0u64;
                    let mut metric_read_us = 0u128;
                    let mut metric_read_max_us = 0u128;
                    let mut metric_publish_us = 0u128;
                    let mut metric_publish_max_us = 0u128;
                    let mut retry_without_hwaccel = false;
                    loop {
                        if !alive_ref.load(Ordering::Acquire)
                            || paused_ref.load(Ordering::Acquire)
                            || seek_request_ns_ref.load(Ordering::Acquire) != stream_seek_marker
                        {
                            let _ = child.kill();
                            let _ = child.wait();
                            break;
                        }
                        let read_started = Instant::now();
                        if let Err(err) = stdout.read_exact(read_buffer.as_mut_slice()) {
                            let _ = child.wait();
                            if frame_index == 0 && use_hwaccel {
                                retry_without_hwaccel = true;
                                log::warn!(
                                    "[Video {id}][FFmpeg] hwaccel produced no frames; retrying software decode"
                                );
                                break;
                            }
                            if looping_ref.load(Ordering::Acquire) && duration_ns > 0 {
                                position_ns_ref.store(0, Ordering::SeqCst);
                                is_eos_ref.store(false, Ordering::SeqCst);
                            } else {
                                is_eos_ref.store(true, Ordering::SeqCst);
                                paused_ref.store(true, Ordering::SeqCst);
                            }
                            if err.kind() != std::io::ErrorKind::UnexpectedEof {
                                log::debug!("[Video {id}][FFmpeg] read ended: {err}");
                            }
                            break;
                        }
                        let read_us = read_started.elapsed().as_micros();
                        let deadline =
                            Self::frame_deadline(stream_wall_start, frame_duration, frame_index);
                        let now = Instant::now();
                        if let Some(wait) = deadline.checked_duration_since(now) {
                            std::thread::sleep(wait);
                        } else {
                            let late_by = now.saturating_duration_since(deadline);
                            if !is_image && late_by > frame_duration && dropped_since_publish < 12 {
                                metric_dropped = metric_dropped.saturating_add(1);
                                dropped_since_publish = dropped_since_publish.saturating_add(1);
                                frame_index = frame_index.saturating_add(1);
                                continue;
                            }
                        }

                        let pts = stream_start + frame_duration.saturating_mul(frame_index as u32);
                        let pts_ns = pts.as_nanos().min(u128::from(u64::MAX)) as u64;
                        position_ns_ref.store(pts_ns, Ordering::SeqCst);
                        last_frame_pts_ns_ref.store(pts_ns, Ordering::SeqCst);
                        decoded_frame_counter_ref.fetch_add(1, Ordering::SeqCst);
                        *last_frame_time_ref.lock() = Instant::now();
                        let publish_started = Instant::now();
                        let published = std::mem::replace(&mut read_buffer, vec![0u8; frame_bytes]);
                        let data = Arc::new(published);
                        let raw = Frame::RawBgra {
                            data: Arc::clone(&data),
                            width,
                            height,
                        };
                        *frame_ref.lock() = raw.clone();
                        let capacity = frame_buffer_capacity_ref.load(Ordering::SeqCst);
                        if capacity > 0 {
                            let mut buf = frame_buffer_ref.lock();
                            buf.push_back(raw);
                            while buf.len() > capacity {
                                buf.pop_front();
                            }
                        }
                        upload_frame_ref.store(true, Ordering::SeqCst);
                        dropped_since_publish = 0;
                        let publish_us = publish_started.elapsed().as_micros();
                        if debug_ffmpeg_preview {
                            metric_frames = metric_frames.saturating_add(1);
                            metric_read_us = metric_read_us.saturating_add(read_us);
                            metric_read_max_us = metric_read_max_us.max(read_us);
                            metric_publish_us = metric_publish_us.saturating_add(publish_us);
                            metric_publish_max_us = metric_publish_max_us.max(publish_us);
                            let elapsed = metric_started.elapsed();
                            if elapsed >= Duration::from_secs(1) {
                                let frames = metric_frames.max(1);
                                log::info!(
                                    "[Video {id}][FFmpegPreview] decoded_fps={:.1} dropped={} read_avg_ms={:.2} read_max_ms={:.2} publish_avg_ms={:.2} publish_max_ms={:.2} bytes_per_frame={} hwaccel={} hwaccel_name={}",
                                    metric_frames as f64 / elapsed.as_secs_f64(),
                                    metric_dropped,
                                    metric_read_us as f64 / frames as f64 / 1000.0,
                                    metric_read_max_us as f64 / 1000.0,
                                    metric_publish_us as f64 / frames as f64 / 1000.0,
                                    metric_publish_max_us as f64 / 1000.0,
                                    frame_bytes,
                                    use_hwaccel,
                                    hwaccel_name,
                                );
                                metric_started = Instant::now();
                                metric_frames = 0;
                                metric_dropped = 0;
                                metric_read_us = 0;
                                metric_read_max_us = 0;
                                metric_publish_us = 0;
                                metric_publish_max_us = 0;
                            }
                        }
                        frame_index = frame_index.saturating_add(1);
                    }
                    if retry_without_hwaccel {
                        use_hwaccel = false;
                        continue;
                    }
                    break;
                }
            }
            log::info!("[Video {id}][FFmpeg] worker exit");
        });

        Ok(Video(Arc::new(RwLock::new(Internal {
            id,
            bus: None,
            source: None,
            ffmpeg: Some(FfmpegControl {
                paused,
                muted,
                volume,
                position_ns,
                seek_request_ns,
            }),
            alive,
            worker: Some(worker),
            width: width as i32,
            height: height as i32,
            framerate: f64::from(fps),
            duration,
            speed,
            frame,
            upload_frame,
            frame_buffer,
            frame_buffer_capacity,
            last_frame_time,
            looping,
            is_eos,
            restart_stream: false,
            subtitle_text,
            upload_text,
            last_frame_pts_ns,
            decoded_frame_counter,
            display_width_override: None,
            display_height_override: None,
            strict_surface_proxy_nv12: false,
        }))))
    }

    #[allow(clippy::too_many_arguments)]
    fn decode_one_ffmpeg_frame(
        ffmpeg_path: &str,
        path: &str,
        start: Duration,
        width: u32,
        height: u32,
        fps: u32,
        is_image: bool,
        frame: &Arc<Mutex<Frame>>,
        upload_frame: &Arc<AtomicBool>,
        last_frame_time: &Arc<Mutex<Instant>>,
        last_frame_pts_ns: &Arc<AtomicU64>,
        decoded_frame_counter: &Arc<AtomicU64>,
    ) -> Result<(), Error> {
        let mut use_hwaccel = Self::ffmpeg_hwaccel_enabled() && !is_image;
        let mut last_err: Option<std::io::Error> = None;
        let output = loop {
            let args =
                Self::ffmpeg_decode_args(path, start, width, height, fps, is_image, use_hwaccel);
            let result = Command::new(ffmpeg_path)
                .args(args)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .and_then(|mut child| {
                    let mut data = vec![0u8; width as usize * height as usize * 4];
                    if let Some(stdout) = child.stdout.as_mut() {
                        stdout.read_exact(&mut data)?;
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                    Ok(data)
                });
            match result {
                Ok(data) => break data,
                Err(err) if use_hwaccel => {
                    last_err = Some(err);
                    use_hwaccel = false;
                }
                Err(err) => {
                    let detail = last_err
                        .map(|first| format!("{err}; hwaccel first error: {first}"))
                        .unwrap_or_else(|| err.to_string());
                    return Err(Error::Ffmpeg(format!(
                        "failed to decode seek frame: {detail}"
                    )));
                }
            }
        };
        *frame.lock() = Frame::RawBgra {
            data: Arc::new(output),
            width,
            height,
        };
        *last_frame_time.lock() = Instant::now();
        last_frame_pts_ns.store(
            start.as_nanos().min(u128::from(u64::MAX)) as u64,
            Ordering::SeqCst,
        );
        decoded_frame_counter.fetch_add(1, Ordering::SeqCst);
        upload_frame.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn is_image_uri(uri: &url::Url) -> bool {
        if let Some(path) = uri.path_segments().and_then(|mut segs| segs.next_back()) {
            let lower = path.to_ascii_lowercase();
            return lower.ends_with(".jpg")
                || lower.ends_with(".jpeg")
                || lower.ends_with(".png")
                || lower.ends_with(".webp")
                || lower.ends_with(".bmp")
                || lower.ends_with(".gif")
                || lower.ends_with(".tif")
                || lower.ends_with(".tiff");
        }
        false
    }

    fn extract_engine_appsink(video_sink_elem: gst::Element) -> Result<gst_app::AppSink, Error> {
        if let Ok(appsink) = video_sink_elem.clone().downcast::<gst_app::AppSink>() {
            return Ok(appsink);
        }

        if let Ok(bin) = video_sink_elem.clone().downcast::<gst::Bin>()
            && let Some(sink_element) = bin.by_name("engine_sink")
        {
            return sink_element
                .downcast::<gst_app::AppSink>()
                .map_err(|_| Error::Cast);
        }

        if let Some(pad) = video_sink_elem.pads().first().cloned()
            && let Ok(ghost_pad) = pad.dynamic_cast::<gst::GhostPad>()
            && let Some(parent) = ghost_pad.parent_element()
            && let Ok(bin) = parent.downcast::<gst::Bin>()
            && let Some(sink_element) = bin.by_name("engine_sink")
        {
            return sink_element
                .downcast::<gst_app::AppSink>()
                .map_err(|_| Error::Cast);
        }

        Err(Error::AppSink("engine_sink".to_string()))
    }

    fn build_preview_caps(
        preview_format: &str,
        preview_fps: Option<u32>,
        width: Option<i32>,
        height: Option<i32>,
    ) -> gst::Caps {
        let mut caps = gst::Caps::builder("video/x-raw")
            .field("format", &preview_format)
            .field("pixel-aspect-ratio", &gst::Fraction::new(1, 1));

        if let Some(fps) = preview_fps.filter(|fps| *fps > 0)
            && let Ok(fps_i32) = i32::try_from(fps)
        {
            caps = caps.field("framerate", &gst::Fraction::new(fps_i32, 1));
        }
        if let Some(width) = width {
            caps = caps.field("width", &width);
        }
        if let Some(height) = height {
            caps = caps.field("height", &height);
        }

        caps.build()
    }

    /// Create a new video player from a given video which loads from `uri`.
    pub fn new(uri: &url::Url) -> Result<Self, Error> {
        Self::new_with_options(uri, VideoOptions::default())
    }

    /// Create a new video player from a given video which loads from `uri`,
    /// applying initialization options.
    pub fn new_with_options(uri: &url::Url, options: VideoOptions) -> Result<Self, Error> {
        if !options.is_audio_only && Self::visual_ffmpeg_backend_enabled() {
            return Self::new_ffmpeg_visual(uri, options);
        }

        if !Self::gstreamer_backend_enabled() {
            // GStreamer is an explicit fallback only; default preview/audio paths must avoid gst::init.
            return Err(Error::Ffmpeg(
                "GStreamer backend disabled. Set ANICA_ENABLE_GSTREAMER=1 to opt in.".to_string(),
            ));
        }

        gst::init()?;

        let use_surface_path =
            cfg!(target_os = "macos") && options.prefer_surface && !Self::is_image_uri(uri);
        let preview_format = if use_surface_path { "NV12" } else { "BGRA" };
        let preview_fps = options.preview_fps.filter(|fps| *fps > 0);

        let pipeline_str = if options.is_audio_only {
            // Audio-Only: force playbin into audio-only mode to avoid wasting CPU
            // on video decode for dedicated audio players.
            // Use an explicit volume element as audio-filter so set_volume() works
            // reliably on macOS where playbin's built-in volume can be ignored.
            format!(
                r#"playbin uri="{uri}" flags={playbin_flags} video-sink="fakesink" text-sink="fakesink" audio-filter="volume name=anica_vol""#,
                uri = uri.as_str(),
                playbin_flags = PLAYBIN_FLAGS_AUDIO_ONLY,
            )
        } else {
            let appsink_max_buffers = options.appsink_max_buffers.unwrap_or(1).max(1);
            // Do not include framerate in appsink caps — videorate max-rate already
            // limits the output rate.  Forcing an exact framerate here causes negotiation
            // failure when the source fps (e.g. 29.97) cannot be converted by
            // `videorate drop-only=true` to the requested target (e.g. 60/1).
            let preview_size = options
                .preview_width
                .zip(options.preview_height)
                .filter(|(w, h)| *w >= 2 && *h >= 2);
            let preview_caps = if let Some((width, height)) = preview_size {
                format!(
                    "video/x-raw,format={preview_format},pixel-aspect-ratio=1/1,width={width},height={height}",
                    preview_format = preview_format,
                    width = width,
                    height = height,
                )
            } else {
                format!(
                    "video/x-raw,format={preview_format},pixel-aspect-ratio=1/1",
                    preview_format = preview_format,
                )
            };
            let appsink_chain = if preview_size.is_some() {
                format!(
                    r#"videoscale ! {preview_caps} !
                       appsink name=engine_sink sync=true drop=true max-buffers={appsink_max_buffers} enable-last-sample=false"#,
                    preview_caps = preview_caps,
                    appsink_max_buffers = appsink_max_buffers,
                )
            } else {
                format!(
                    r#"appsink name=engine_sink caps={preview_caps} sync=true drop=true max-buffers={appsink_max_buffers} enable-last-sample=false"#,
                    preview_caps = preview_caps,
                    appsink_max_buffers = appsink_max_buffers,
                )
            };
            let video_sink = if options.benchmark_raw_appsink {
                // Benchmark-only path: expose decoder output as-is.
                format!(
                    r#"appsink name=engine_sink sync=true drop=true max-buffers={appsink_max_buffers} enable-last-sample=false"#,
                    appsink_max_buffers = appsink_max_buffers,
                )
            } else {
                if let Some(fps) = preview_fps {
                    format!(
                        r#"videorate drop-only=true max-rate={fps} !
                            {appsink_chain}"#,
                        fps = fps,
                        appsink_chain = appsink_chain,
                    )
                } else {
                    appsink_chain
                }
            };
            // Visual preview path: decode video only, discard audio/text entirely.
            format!(
                r#"playbin uri="{uri}" flags={playbin_flags} audio-sink="fakesink" text-sink="fakesink"
                    video-sink="{video_sink}""#,
                uri = uri.as_str(),
                playbin_flags = PLAYBIN_FLAGS_VIDEO_ONLY,
                video_sink = video_sink,
            )
        };

        let pipeline = gst::parse::launch(pipeline_str.as_ref())?
            .downcast::<gst::Pipeline>()
            .map_err(|_| Error::Cast)?;

        // Only extract the appsink if we are NOT in audio-only mode
        let video_sink = if !options.is_audio_only {
            let video_sink_elem: gst::Element = pipeline.property("video-sink");
            Some(Self::extract_engine_appsink(video_sink_elem)?)
        } else {
            None
        };

        Self::from_gst_pipeline_with_options(pipeline, video_sink, None, options)
    }

    /// Creates a new video based on an existing GStreamer pipeline and appsink.
    pub fn from_gst_pipeline(
        pipeline: gst::Pipeline,
        video_sink: gst_app::AppSink,
        text_sink: Option<gst_app::AppSink>,
    ) -> Result<Self, Error> {
        Self::from_gst_pipeline_with_options(
            pipeline,
            Some(video_sink),
            text_sink,
            VideoOptions::default(),
        )
    }

    /// Creates a new video based on an existing GStreamer pipeline and appsink,
    /// applying initialization options.
    pub fn from_gst_pipeline_with_options(
        pipeline: gst::Pipeline,
        video_sink: Option<gst_app::AppSink>, // Changed to Option for audio-only support
        text_sink: Option<gst_app::AppSink>,
        options: VideoOptions,
    ) -> Result<Self, Error> {
        gst::init()?;
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        // Track each pipeline lifecycle in terminal logs to debug boundary stalls and leaks.
        log::info!(
            "[Video {}] init audio_only={} preview_max_dim={:?} preview_size={:?}x{:?} preview_fps={:?} raw_appsink={}",
            id,
            options.is_audio_only,
            options.preview_max_dim,
            options.preview_width,
            options.preview_height,
            options.preview_fps,
            options.benchmark_raw_appsink,
        );

        macro_rules! cleanup {
            ($expr:expr) => {
                $expr.map_err(|e| {
                    let _ = pipeline.set_state(gst::State::Null);
                    e
                })
            };
        }

        cleanup!(pipeline.set_state(gst::State::Playing))?;

        // Wait a brief moment for the pipeline to start playing
        let _ = pipeline.state(gst::ClockTime::from_mseconds(100));
        cleanup!(pipeline.state(gst::ClockTime::from_seconds(5)).0)?;

        let (mut width, mut height, framerate) = if let Some(sink) = &video_sink {
            let pad = sink.pads().first().cloned().unwrap();
            let caps = cleanup!(pad.current_caps().ok_or(Error::Caps))?;
            let s = cleanup!(caps.structure(0).ok_or(Error::Caps))?;
            let w = cleanup!(s.get::<i32>("width").map_err(|_| Error::Caps))?;
            let h = cleanup!(s.get::<i32>("height").map_err(|_| Error::Caps))?;
            let fr = cleanup!(s.get::<gst::Fraction>("framerate").map_err(|_| Error::Caps))?;
            let fr_val = fr.numer() as f64 / fr.denom() as f64;
            (w, h, fr_val)
        } else {
            (0, 0, 0.0)
        };

        let target_scale = if options.benchmark_raw_appsink {
            None
        } else if let Some(scale) = options.preview_scale {
            Some(scale)
        } else if let Some(max_dim) = options.preview_max_dim {
            if width > 0 && height > 0 {
                let larger = width.max(height) as f32;
                let ratio = (max_dim as f32) / larger;
                Some(ratio)
            } else {
                None
            }
        } else {
            None
        };

        if let (Some(scale), Some(sink)) = (target_scale, &video_sink) {
            if !options.is_audio_only
                && scale.is_finite()
                && scale > 0.0
                && scale < 1.0
                && width > 0
                && height > 0
            {
                let clamped = scale.clamp(0.01, 1.0);
                let target_w = ((width as f32) * clamped).round().max(2.0) as i32;
                let target_h = ((height as f32) * clamped).round().max(2.0) as i32;

                if target_w != width || target_h != height {
                    let negotiated_format = sink.pads().first().and_then(|pad| {
                        let caps = pad.current_caps()?;
                        let s = caps.structure(0)?;
                        let fmt = s.get::<String>("format").ok()?;
                        if fmt.is_empty() { None } else { Some(fmt) }
                    });
                    let preview_format = negotiated_format.unwrap_or_else(|| {
                        if cfg!(target_os = "macos") {
                            "NV12".to_string()
                        } else {
                            "BGRA".to_string()
                        }
                    });
                    let caps = Self::build_preview_caps(
                        preview_format.as_str(),
                        options.preview_fps,
                        Some(target_w),
                        Some(target_h),
                    );
                    sink.set_caps(Some(&caps));

                    let _ = pipeline.state(gst::ClockTime::from_mseconds(100));
                    width = target_w;
                    height = target_h;
                    if let Some(pad) = sink.pads().first() {
                        if let Some(caps) = pad.current_caps() {
                            if let Some(s) = caps.structure(0) {
                                if let Ok(w) = s.get::<i32>("width") {
                                    width = w;
                                }
                                if let Ok(h) = s.get::<i32>("height") {
                                    height = h;
                                }
                            }
                        }
                    }
                }
            }
        }

        if !options.is_audio_only
            && !options.benchmark_raw_appsink
            && (framerate.is_nan()
                || framerate.is_infinite()
                || framerate < 0.0
                || framerate.abs() < f64::EPSILON)
        {
            let _ = pipeline.set_state(gst::State::Null);
            return Err(Error::Framerate(framerate));
        }

        let duration = Duration::from_nanos(
            pipeline
                .query_duration::<gst::ClockTime>()
                .map(|duration| duration.nseconds())
                .unwrap_or(0),
        );

        let frame = Arc::new(Mutex::new(Frame::empty()));
        let upload_frame = Arc::new(AtomicBool::new(false));
        let frame_buffer = Arc::new(Mutex::new(VecDeque::new()));
        // Default to a small buffer so the element can consume buffered frames
        let frame_buffer_capacity = Arc::new(AtomicUsize::new(
            options.frame_buffer_capacity.unwrap_or_default(),
        ));
        let alive = Arc::new(AtomicBool::new(true));
        let last_frame_time = Arc::new(Mutex::new(Instant::now()));
        let initial_looping = options.looping.unwrap_or_default();
        let looping_flag = Arc::new(AtomicBool::new(initial_looping));
        let looping_ref = Arc::clone(&looping_flag);
        let initial_speed = options.speed.unwrap_or_default();
        let speed_state = Arc::new(AtomicU64::new(initial_speed.to_bits()));
        let speed_ref = Arc::clone(&speed_state);

        let frame_ref = Arc::clone(&frame);
        let upload_frame_ref = Arc::clone(&upload_frame);
        let frame_buffer_ref = Arc::clone(&frame_buffer);
        let frame_buffer_capacity_ref = Arc::clone(&frame_buffer_capacity);
        let alive_ref = Arc::clone(&alive);
        let last_frame_time_ref = Arc::clone(&last_frame_time);

        let subtitle_text = Arc::new(Mutex::new(None));
        let upload_text = Arc::new(AtomicBool::new(false));
        let last_frame_pts_ns = Arc::new(AtomicU64::new(0));
        let decoded_frame_counter = Arc::new(AtomicU64::new(0));
        let subtitle_text_ref = Arc::clone(&subtitle_text);
        let upload_text_ref = Arc::clone(&upload_text);
        let last_frame_pts_ns_ref = Arc::clone(&last_frame_pts_ns);
        let decoded_frame_counter_ref = Arc::clone(&decoded_frame_counter);

        let pipeline_ref = pipeline.clone();
        let bus_ref = pipeline_ref.bus().unwrap();
        let is_eos = Arc::new(AtomicBool::new(false));
        let is_eos_ref = Arc::clone(&is_eos);
        // Keep pull timeout roughly aligned with the expected preview cadence.
        let pull_timeout = options
            .preview_fps
            .filter(|fps| *fps > 0)
            .map(|fps| {
                let timeout_ns = (1_000_000_000u64 / fps as u64).max(1_000_000);
                gst::ClockTime::from_nseconds(timeout_ns)
            })
            .unwrap_or_else(|| gst::ClockTime::from_mseconds(16));

        let worker = std::thread::spawn(move || {
            let mut clear_subtitles_at = None;

            while alive_ref.load(Ordering::Acquire) {
                // Drain bus messages to detect EOS/errors
                while let Some(msg) = bus_ref.timed_pop(gst::ClockTime::from_seconds(0)) {
                    match msg.view() {
                        MessageView::Eos(_) => {
                            if looping_ref.load(Ordering::SeqCst) {
                                let mut flags = gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT;
                                let current_speed =
                                    f64::from_bits(speed_ref.load(Ordering::SeqCst));
                                if current_speed >= 0.0 {
                                    flags |= gst::SeekFlags::SNAP_AFTER;
                                } else {
                                    flags |= gst::SeekFlags::SNAP_BEFORE;
                                }
                                match pipeline_ref.seek(
                                    current_speed,
                                    flags,
                                    gst::SeekType::Set,
                                    gst::GenericFormattedValue::from(gst::ClockTime::from_seconds(
                                        0,
                                    )),
                                    gst::SeekType::None,
                                    gst::ClockTime::NONE,
                                ) {
                                    Ok(_) => {
                                        is_eos_ref.store(false, Ordering::SeqCst);
                                        let _ = pipeline_ref.set_state(gst::State::Playing);
                                        frame_buffer_ref.lock().clear();
                                        upload_frame_ref.store(false, Ordering::SeqCst);
                                        *subtitle_text_ref.lock() = None;
                                        upload_text_ref.store(true, Ordering::SeqCst);
                                        *last_frame_time_ref.lock() = Instant::now();
                                        continue;
                                    }
                                    Err(err) => {
                                        log::error!("failed to restart video for looping: {}", err);
                                        is_eos_ref.store(true, Ordering::SeqCst);
                                    }
                                }
                            } else {
                                is_eos_ref.store(true, Ordering::SeqCst);
                            }
                        }
                        MessageView::Error(err) => {
                            let debug = err.debug().unwrap_or_default();
                            log::error!(
                                "gstreamer error from {:?}: {} ({debug})",
                                err.src(),
                                err.error()
                            );
                        }
                        _ => {}
                    }
                }

                if is_eos_ref.load(Ordering::Acquire) {
                    // Stop busy-polling once EOS reached
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }

                // If no sink (Audio Only), just wait and continue loop
                let Some(sink) = &video_sink else {
                    std::thread::sleep(Duration::from_millis(16));
                    continue;
                };

                if let Err(err) = (|| -> Result<(), gst::FlowError> {
                    // Try to pull a new sample; on timeout just continue (no frame this tick)
                    let maybe_sample =
                        if pipeline_ref.state(gst::ClockTime::ZERO).1 != gst::State::Playing {
                            sink.try_pull_preroll(pull_timeout)
                        } else {
                            sink.try_pull_sample(pull_timeout)
                        };

                    let Some(sample) = maybe_sample else {
                        // No sample available yet (timeout). Don't treat as error.
                        return Ok(());
                    };

                    *last_frame_time_ref.lock() = Instant::now();

                    let frame_segment = sample.segment().cloned().ok_or(gst::FlowError::Error)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let frame_pts = buffer.pts().ok_or(gst::FlowError::Error)?;
                    let frame_duration = buffer.duration().ok_or(gst::FlowError::Error)?;
                    last_frame_pts_ns_ref.store(frame_pts.nseconds(), Ordering::SeqCst);
                    decoded_frame_counter_ref.fetch_add(1, Ordering::SeqCst);

                    // Store the sample
                    {
                        let mut frame_guard = frame_ref.lock();
                        *frame_guard = Frame::Gst(sample);
                    }

                    // Push into frame buffer if enabled, trimming to capacity
                    let capacity = frame_buffer_capacity_ref.load(Ordering::SeqCst);
                    if capacity > 0 {
                        let sample_for_buffer = match &*frame_ref.lock() {
                            Frame::Gst(sample) => Some(sample.clone()),
                            Frame::RawBgra { .. } => None,
                        };
                        if let Some(sample_for_buffer) = sample_for_buffer {
                            let mut buf = frame_buffer_ref.lock();
                            buf.push_back(Frame::Gst(sample_for_buffer));
                            while buf.len() > capacity {
                                buf.pop_front();
                            }
                        }
                    }

                    // Always mark frame as ready for upload
                    upload_frame_ref.store(true, Ordering::SeqCst);

                    // Handle subtitles
                    if let Some(at) = clear_subtitles_at
                        && frame_pts >= at
                    {
                        *subtitle_text_ref.lock() = None;
                        upload_text_ref.store(true, Ordering::SeqCst);
                        clear_subtitles_at = None;
                    }

                    let text = text_sink
                        .as_ref()
                        .and_then(|sink| sink.try_pull_sample(gst::ClockTime::from_seconds(0)));
                    if let Some(text) = text {
                        let text_segment = text.segment().ok_or(gst::FlowError::Error)?;
                        let text = text.buffer().ok_or(gst::FlowError::Error)?;
                        let text_pts = text.pts().ok_or(gst::FlowError::Error)?;
                        let text_duration = text.duration().ok_or(gst::FlowError::Error)?;

                        let frame_running_time = frame_segment.to_running_time(frame_pts).value();
                        let frame_running_time_end = frame_segment
                            .to_running_time(frame_pts + frame_duration)
                            .value();

                        let text_running_time = text_segment.to_running_time(text_pts).value();
                        let text_running_time_end = text_segment
                            .to_running_time(text_pts + text_duration)
                            .value();

                        if text_running_time_end > frame_running_time
                            && frame_running_time_end > text_running_time
                        {
                            let duration = text.duration().unwrap_or(gst::ClockTime::ZERO);
                            let map = text.map_readable().map_err(|_| gst::FlowError::Error)?;

                            let text = std::str::from_utf8(map.as_slice())
                                .map_err(|_| gst::FlowError::Error)?
                                .to_string();
                            *subtitle_text_ref.lock() = Some(text);
                            upload_text_ref.store(true, Ordering::SeqCst);

                            clear_subtitles_at = Some(text_pts + duration);
                        }
                    }

                    Ok(())
                })() {
                    // Only log non-EOS errors
                    if err != gst::FlowError::Eos {
                        log::error!("error processing frame: {:?}", err);
                    }
                }
            }
            log::info!("[Video {}] worker exit", id);
        });

        // Apply initial playback speed if specified (must be after pipeline started)
        if (initial_speed - 1.0).abs() > f64::EPSILON {
            let position = cleanup!(
                pipeline
                    .query_position::<gst::ClockTime>()
                    .ok_or(Error::Caps)
            )?;
            if initial_speed > 0.0 {
                cleanup!(pipeline.seek(
                    initial_speed,
                    gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                    gst::SeekType::Set,
                    position,
                    gst::SeekType::End,
                    gst::ClockTime::from_seconds(0),
                ))?;
            } else {
                cleanup!(pipeline.seek(
                    initial_speed,
                    gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                    gst::SeekType::Set,
                    gst::ClockTime::from_seconds(0),
                    gst::SeekType::Set,
                    position,
                ))?;
            }
        }

        Ok(Video(Arc::new(RwLock::new(Internal {
            id,
            bus: pipeline.bus(),
            source: Some(pipeline),
            ffmpeg: None,
            alive,
            worker: Some(worker),

            width,
            height,
            framerate,
            duration,
            speed: speed_state,

            frame,
            upload_frame,
            frame_buffer,
            frame_buffer_capacity,
            last_frame_time,
            looping: looping_flag,
            is_eos,
            restart_stream: false,

            subtitle_text,
            upload_text,
            last_frame_pts_ns,
            decoded_frame_counter,

            display_width_override: None,
            display_height_override: None,
            strict_surface_proxy_nv12: options.strict_surface_proxy_nv12,
        }))))
    }

    pub(crate) fn read(&'_ self) -> parking_lot::RwLockReadGuard<'_, Internal> {
        self.0.read()
    }

    pub(crate) fn write(&'_ self) -> parking_lot::RwLockWriteGuard<'_, Internal> {
        self.0.write()
    }

    /// Get the size/resolution of the video as `(width, height)`.
    pub fn size(&self) -> (i32, i32) {
        (self.read().width, self.read().height)
    }

    /// Stable runtime id for this `Video` instance (useful for external cache keys).
    pub fn id(&self) -> u64 {
        self.read().id
    }

    /// Get the natural aspect ratio (width / height) of the video as f32.
    pub fn aspect_ratio(&self) -> f32 {
        let (w, h) = self.size();
        if w <= 0 || h <= 0 {
            return 1.0;
        }
        w as f32 / h as f32
    }

    /// Set an override display width in pixels. Pass `None` to clear.
    pub fn set_display_width(&self, width: Option<u32>) {
        self.write().display_width_override = width;
    }

    /// Set an override display height in pixels. Pass `None` to clear.
    pub fn set_display_height(&self, height: Option<u32>) {
        self.write().display_height_override = height;
    }

    /// Set override display size in pixels. Any value set to `None` is cleared.
    pub fn set_display_size(&self, width: Option<u32>, height: Option<u32>) {
        let mut inner = self.write();
        inner.display_width_override = width;
        inner.display_height_override = height;
    }

    /// Get the effective display size honoring overrides. If only one of
    /// width/height is overridden, the other is inferred from the natural
    /// aspect ratio, rounded to nearest pixel.
    pub fn display_size(&self) -> (u32, u32) {
        let inner = self.read();
        let natural_w = inner.width.max(0) as u32;
        let natural_h = inner.height.max(0) as u32;
        let ar = if natural_h == 0 {
            1.0
        } else {
            natural_w as f32 / natural_h as f32
        };

        match (inner.display_width_override, inner.display_height_override) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => {
                let h = if ar == 0.0 {
                    natural_h
                } else {
                    (w as f32 / ar).round() as u32
                };
                (w, h)
            }
            (None, Some(h)) => {
                let w = ((h as f32) * ar).round() as u32;
                (w, h)
            }
            (None, None) => (natural_w, natural_h),
        }
    }

    /// Get the framerate of the video as frames per second.
    pub fn framerate(&self) -> f64 {
        self.read().framerate
    }

    /// Set the volume multiplier of the audio.
    pub fn set_volume(&self, volume: f64) {
        let inner = self.write();
        if let Some(ffmpeg) = inner.ffmpeg.as_ref() {
            ffmpeg.volume.store(volume.to_bits(), Ordering::SeqCst);
            return;
        }
        let Some(source) = inner.source.as_ref() else {
            return;
        };
        // Prefer the explicit audio-filter volume element when present.
        let bin: &gst::Bin = source.upcast_ref();
        if let Some(vol_elem) = bin.by_name("anica_vol") {
            vol_elem.set_property("volume", volume);
        } else {
            source.set_property("volume", volume);
        }
    }

    /// Get the volume multiplier of the audio.
    pub fn volume(&self) -> f64 {
        let inner = self.read();
        if let Some(ffmpeg) = inner.ffmpeg.as_ref() {
            return f64::from_bits(ffmpeg.volume.load(Ordering::SeqCst));
        }
        let Some(source) = inner.source.as_ref() else {
            return 1.0;
        };
        // Read from the explicit audio-filter volume element when present.
        let bin: &gst::Bin = source.upcast_ref();
        if let Some(vol_elem) = bin.by_name("anica_vol") {
            vol_elem.property("volume")
        } else {
            source.property("volume")
        }
    }

    /// Set if the audio is muted or not.
    pub fn set_muted(&self, muted: bool) {
        let inner = self.write();
        if let Some(ffmpeg) = inner.ffmpeg.as_ref() {
            ffmpeg.muted.store(muted, Ordering::SeqCst);
        } else if let Some(source) = inner.source.as_ref() {
            source.set_property("mute", muted);
        }
    }

    /// Get if the audio is muted or not.
    pub fn muted(&self) -> bool {
        let inner = self.read();
        if let Some(ffmpeg) = inner.ffmpeg.as_ref() {
            return ffmpeg.muted.load(Ordering::SeqCst);
        }
        inner
            .source
            .as_ref()
            .map(|source| source.property("mute"))
            .unwrap_or(false)
    }

    /// Get if the stream ended or not.
    pub fn eos(&self) -> bool {
        self.read().is_eos.load(Ordering::Acquire)
    }

    /// Get if the media will loop or not.
    pub fn looping(&self) -> bool {
        self.read().looping.load(Ordering::SeqCst)
    }

    /// Set if the media will loop or not.
    pub fn set_looping(&self, looping: bool) {
        self.write().looping.store(looping, Ordering::SeqCst);
    }

    /// Set if the media is paused or not.
    pub fn set_paused(&self, paused: bool) {
        self.write().set_paused(paused)
    }

    /// Get if the media is paused or not.
    pub fn paused(&self) -> bool {
        self.read().paused()
    }

    /// Lightweight state snapshot for diagnostics (`current`, `pending`).
    pub fn state_debug(&self) -> (String, String) {
        let inner = self.read();
        let Some(source) = inner.source.as_ref() else {
            let current = if inner.paused() { "Paused" } else { "Playing" };
            return (current.to_string(), "VoidPending".to_string());
        };
        let (_, current_state, pending_state) = source.state(gst::ClockTime::ZERO);
        (format!("{current_state:?}"), format!("{pending_state:?}"))
    }

    /// Jumps to a specific position in the media.
    pub fn seek(&self, position: impl Into<Position>, accurate: bool) -> Result<(), Error> {
        self.write().seek(position, accurate)
    }

    /// Set the playback speed of the media.
    pub fn set_speed(&self, speed: f64) -> Result<(), Error> {
        self.write().set_speed(speed)
    }

    /// Get the current playback speed.
    pub fn speed(&self) -> f64 {
        f64::from_bits(self.read().speed.load(Ordering::SeqCst))
    }

    /// Get the current playback position in time.
    pub fn position(&self) -> Duration {
        let inner = self.read();
        if let Some(ffmpeg) = inner.ffmpeg.as_ref() {
            return Duration::from_nanos(ffmpeg.position_ns.load(Ordering::Acquire));
        }
        Duration::from_nanos(
            inner
                .source
                .as_ref()
                .and_then(|source| source.query_position::<gst::ClockTime>())
                .map_or(0, |pos| pos.nseconds()),
        )
    }

    /// Get the underlying GStreamer pipeline.
    pub fn pipeline(&self) -> gst::Pipeline {
        self.read()
            .source
            .clone()
            .expect("GStreamer pipeline is unavailable for the FFmpeg video backend")
    }

    /// Get the media duration.
    pub fn duration(&self) -> Duration {
        self.read().duration
    }

    /// Restarts a stream.
    pub fn restart_stream(&self) -> Result<(), Error> {
        self.write().restart_stream()
    }

    pub fn set_blur_sigma(&self, sigma: f64) {
        self.read().set_blur_sigma(sigma);
    }

    fn copy_bgra_rows_to_packed(
        width: usize,
        height: usize,
        src_bgra: &[u8],
        src_stride: usize,
    ) -> Option<Vec<u8>> {
        let row_bytes = width.checked_mul(4)?;
        if row_bytes == 0 || height == 0 || src_stride < row_bytes {
            return None;
        }
        if src_bgra.len() < src_stride.checked_mul(height)? {
            return None;
        }

        let mut out = vec![0u8; row_bytes.checked_mul(height)?];
        for row in 0..height {
            let src_off = row.checked_mul(src_stride)?;
            let dst_off = row.checked_mul(row_bytes)?;
            out[dst_off..(dst_off + row_bytes)]
                .copy_from_slice(&src_bgra[src_off..(src_off + row_bytes)]);
        }
        Some(out)
    }

    #[cfg(target_os = "macos")]
    fn nv12_to_bgra_rows(
        width: usize,
        height: usize,
        y_plane: &[u8],
        y_stride: usize,
        uv_plane: &[u8],
        uv_stride: usize,
    ) -> Option<Vec<u8>> {
        if width == 0
            || height == 0
            || (width & 1) != 0
            || (height & 1) != 0
            || y_stride < width
            || uv_stride < width
        {
            return None;
        }
        if y_plane.len() < y_stride.checked_mul(height)?
            || uv_plane.len() < uv_stride.checked_mul(height / 2)?
        {
            return None;
        }

        let mut bgra = vec![0u8; width.checked_mul(height)?.checked_mul(4)?];
        for y in 0..height {
            let uv_row = y / 2;
            let y_row_off = y.checked_mul(y_stride)?;
            let uv_row_off = uv_row.checked_mul(uv_stride)?;
            for x in 0..width {
                let yv = y_plane[y_row_off + x] as i32;
                let uv_idx = uv_row_off + (x & !1);
                let u = uv_plane[uv_idx] as i32 - 128;
                let v = uv_plane[uv_idx + 1] as i32 - 128;

                // Full-range NV12 (BT.601-ish) -> BGRA.
                let r = (yv + ((359 * v) >> 8)).clamp(0, 255) as u8;
                let g = (yv - ((88 * u + 183 * v) >> 8)).clamp(0, 255) as u8;
                let b = (yv + ((454 * u) >> 8)).clamp(0, 255) as u8;

                let dst = (y * width + x) * 4;
                bgra[dst] = b;
                bgra[dst + 1] = g;
                bgra[dst + 2] = r;
                bgra[dst + 3] = 255;
            }
        }
        Some(bgra)
    }

    /// Get the current frame data (BGRA) if available.
    pub fn current_frame_data(&self) -> Option<(Vec<u8>, u32, u32)> {
        let inner = self.read();

        let frame_guard = inner.frame.lock();
        if let Some((data, width, height)) = frame_guard.raw_bgra() {
            if !data.is_empty() && width > 0 && height > 0 {
                return Some((data.to_vec(), width, height));
            }
            return None;
        }
        let mut width = inner.width.max(0) as u32;
        let mut height = inner.height.max(0) as u32;
        let mut format = String::from("BGRA");
        let sample = match &*frame_guard {
            Frame::Gst(sample) => sample,
            Frame::RawBgra { .. } => return None,
        };
        let caps = sample.caps();
        if let Some(caps_ref) = caps.as_ref()
            && let Some(s) = caps_ref.structure(0)
        {
            if let Ok(w) = s.get::<i32>("width")
                && w > 0
            {
                width = w as u32;
            }
            if let Ok(h) = s.get::<i32>("height")
                && h > 0
            {
                height = h as u32;
            }
            if let Ok(fmt) = s.get::<String>("format")
                && !fmt.is_empty()
            {
                format = fmt;
            }
        }
        if width == 0 || height == 0 {
            return None;
        }

        let buffer = sample.buffer();

        #[cfg(target_os = "macos")]
        if format.eq_ignore_ascii_case("NV12") {
            if let (Some(caps_ref), Some(buffer_ref)) = (caps.as_ref(), buffer)
                && let Ok(info) = gst_video::VideoInfo::from_caps(caps_ref)
                && let Ok(video_frame) =
                    gst_video::VideoFrameRef::from_buffer_ref_readable(buffer_ref, &info)
            {
                let y_stride_i32 = *video_frame.plane_stride().first().unwrap_or(&0);
                let uv_stride_i32 = *video_frame.plane_stride().get(1).unwrap_or(&0);
                if y_stride_i32 > 0
                    && uv_stride_i32 > 0
                    && let (Ok(y_plane), Ok(uv_plane)) =
                        (video_frame.plane_data(0), video_frame.plane_data(1))
                    && let Some(bgra) = Self::nv12_to_bgra_rows(
                        width as usize,
                        height as usize,
                        y_plane,
                        y_stride_i32 as usize,
                        uv_plane,
                        uv_stride_i32 as usize,
                    )
                {
                    return Some((bgra, width, height));
                }
            }

            // Legacy fallback for tightly packed NV12 memory.
            if let Some(readable) = frame_guard.readable() {
                let data = readable.as_slice();
                let w = width as usize;
                let h = height as usize;
                let y_len = w * h;
                let uv_len = y_len / 2;
                if data.len() >= (y_len + uv_len) {
                    let y_plane = &data[..y_len];
                    let uv_plane = &data[y_len..(y_len + uv_len)];
                    if let Some(bgra) = Self::nv12_to_bgra_rows(w, h, y_plane, w, uv_plane, w) {
                        return Some((bgra, width, height));
                    }
                }
            }

            return None;
        }

        if format.eq_ignore_ascii_case("BGRA") {
            if let (Some(caps_ref), Some(buffer_ref)) = (caps.as_ref(), buffer)
                && let Ok(info) = gst_video::VideoInfo::from_caps(caps_ref)
                && let Ok(video_frame) =
                    gst_video::VideoFrameRef::from_buffer_ref_readable(buffer_ref, &info)
            {
                let stride_i32 = *video_frame.plane_stride().first().unwrap_or(&0);
                if stride_i32 > 0
                    && let Ok(src_plane) = video_frame.plane_data(0)
                    && let Some(bgra) = Self::copy_bgra_rows_to_packed(
                        width as usize,
                        height as usize,
                        src_plane,
                        stride_i32 as usize,
                    )
                {
                    return Some((bgra, width, height));
                }
            }
        }

        // Generic fallback for tightly packed buffers.
        if let Some(readable) = frame_guard.readable() {
            let data = readable.as_slice();
            if data.is_empty() {
                return None;
            }
            return Some((data.to_vec(), width, height));
        }

        None
    }

    #[cfg(target_os = "macos")]
    fn build_surface_nv12_copy(
        width: u32,
        height: u32,
        src: &[u8],
        cv_options: &CFDictionary<CFString, CFType>,
    ) -> Option<CVPixelBuffer> {
        let w = width as usize;
        let h = height as usize;
        let y_src_len = w * h;
        let uv_src_len = y_src_len / 2;
        let expected = y_src_len + uv_src_len;
        if src.len() < expected {
            return None;
        }
        let y_src = &src[..y_src_len];
        let uv_src = &src[y_src_len..(y_src_len + uv_src_len)];
        Self::build_surface_nv12_copy_from_planes(width, height, y_src, w, uv_src, w, cv_options)
    }

    #[cfg(target_os = "macos")]
    fn build_surface_nv12_copy_from_bgra(
        width: u32,
        height: u32,
        src: &[u8],
        cv_options: &CFDictionary<CFString, CFType>,
    ) -> Option<CVPixelBuffer> {
        let w = width as usize;
        let h = height as usize;
        if w == 0 || h == 0 || (w & 1) != 0 || (h & 1) != 0 {
            return None;
        }
        if src.len() < w.checked_mul(h)?.checked_mul(4)? {
            return None;
        }

        // Convert FFmpeg's packed BGRA frame into full-range NV12 for GPUI's macOS surface renderer.
        let mut y_plane = vec![0u8; w.checked_mul(h)?];
        let mut uv_plane = vec![0u8; w.checked_mul(h / 2)?];
        for y in 0..h {
            for x in 0..w {
                let off = (y * w + x) * 4;
                let b = src[off] as i32;
                let g = src[off + 1] as i32;
                let r = src[off + 2] as i32;
                y_plane[y * w + x] = ((77 * r + 150 * g + 29 * b) >> 8).clamp(0, 255) as u8;
            }
        }
        for y in (0..h).step_by(2) {
            for x in (0..w).step_by(2) {
                let mut u_sum = 0i32;
                let mut v_sum = 0i32;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let off = ((y + dy) * w + (x + dx)) * 4;
                        let b = src[off] as i32;
                        let g = src[off + 1] as i32;
                        let r = src[off + 2] as i32;
                        u_sum += (((-43 * r - 85 * g + 128 * b) >> 8) + 128).clamp(0, 255);
                        v_sum += (((128 * r - 107 * g - 21 * b) >> 8) + 128).clamp(0, 255);
                    }
                }
                let uv_off = (y / 2) * w + x;
                uv_plane[uv_off] = (u_sum / 4) as u8;
                uv_plane[uv_off + 1] = (v_sum / 4) as u8;
            }
        }
        Self::build_surface_nv12_copy_from_planes(
            width, height, &y_plane, w, &uv_plane, w, cv_options,
        )
    }

    #[cfg(target_os = "macos")]
    fn create_bgra_surface_with_options(
        width: u32,
        height: u32,
        cv_options: &CFDictionary<CFString, CFType>,
    ) -> Option<CVPixelBuffer> {
        let w = width as usize;
        let h = height as usize;
        if w == 0 || h == 0 {
            return None;
        }

        // Allocate an IOSurface-backed BGRA CVPixelBuffer so GPUI can import it as a Metal texture.
        CVPixelBuffer::new(
            kCVPixelFormatType_32BGRA,
            width as usize,
            height as usize,
            Some(cv_options),
        )
        .ok()
    }

    #[cfg(target_os = "macos")]
    pub fn create_bgra_surface(width: u32, height: u32) -> Option<CVPixelBuffer> {
        let iosurface_props: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[]);
        let cv_options: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[
            (
                CFString::from(CVPixelBufferKeys::MetalCompatibility),
                CFBoolean::true_value().as_CFType(),
            ),
            (
                CFString::from(CVPixelBufferKeys::IOSurfaceProperties),
                iosurface_props.as_CFType(),
            ),
        ]);
        Self::create_bgra_surface_with_options(width, height, &cv_options)
    }

    #[cfg(target_os = "macos")]
    pub fn copy_bgra_into_surface(
        pixel_buffer: &CVPixelBuffer,
        width: u32,
        height: u32,
        src: &[u8],
    ) -> bool {
        let w = width as usize;
        let h = height as usize;
        if w == 0
            || h == 0
            || pixel_buffer.get_pixel_format() != kCVPixelFormatType_32BGRA
            || pixel_buffer.get_width() < w
            || pixel_buffer.get_height() < h
        {
            return false;
        }
        if src.len()
            < w.checked_mul(h)
                .and_then(|px| px.checked_mul(4))
                .unwrap_or(0)
        {
            return false;
        }

        if pixel_buffer.lock_base_address(0) != kCVReturnSuccess {
            return false;
        }

        let copied = (|| {
            let dst_stride = pixel_buffer.get_bytes_per_row();
            let dst_height = pixel_buffer.get_height();
            if dst_height < h || dst_stride < w.checked_mul(4)? {
                return None;
            }

            let dst_ptr = unsafe { pixel_buffer.get_base_address() as *mut u8 };
            if dst_ptr.is_null() {
                return None;
            }

            let dst_len = dst_stride.checked_mul(dst_height)?;
            let dst = unsafe { std::slice::from_raw_parts_mut(dst_ptr, dst_len) };
            let src_stride = w.checked_mul(4)?;
            for row in 0..h {
                let src_off = row * src_stride;
                let dst_off = row * dst_stride;
                dst[dst_off..(dst_off + src_stride)]
                    .copy_from_slice(&src[src_off..(src_off + src_stride)]);
            }
            Some(())
        })()
        .is_some();

        let _ = pixel_buffer.unlock_base_address(0);
        copied
    }

    #[cfg(target_os = "macos")]
    pub fn build_surface_bgra_copy_from_data(
        width: u32,
        height: u32,
        src: &[u8],
    ) -> Option<CVPixelBuffer> {
        let pixel_buffer = Self::create_bgra_surface(width, height)?;
        if Self::copy_bgra_into_surface(&pixel_buffer, width, height, src) {
            Some(pixel_buffer)
        } else {
            None
        }
    }

    #[cfg(target_os = "macos")]
    fn build_surface_nv12_copy_from_planes(
        width: u32,
        height: u32,
        y_src: &[u8],
        y_src_stride: usize,
        uv_src: &[u8],
        uv_src_stride: usize,
        cv_options: &CFDictionary<CFString, CFType>,
    ) -> Option<CVPixelBuffer> {
        let w = width as usize;
        let h = height as usize;
        if w == 0 || h == 0 || (w & 1) != 0 || (h & 1) != 0 || y_src_stride < w || uv_src_stride < w
        {
            return None;
        }
        if y_src.len() < y_src_stride.checked_mul(h)?
            || uv_src.len() < uv_src_stride.checked_mul(h / 2)?
        {
            return None;
        }

        // Fallback path: allocate an IOSurface-backed NV12 buffer and copy planes into it.
        let pixel_buffer = CVPixelBuffer::new(
            kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
            width as usize,
            height as usize,
            Some(cv_options),
        )
        .ok()?;
        if pixel_buffer.lock_base_address(0) != kCVReturnSuccess {
            return None;
        }

        let copied = (|| {
            if pixel_buffer.get_plane_count() < 2 {
                return None;
            }

            let y_stride = pixel_buffer.get_bytes_per_row_of_plane(0);
            let uv_stride = pixel_buffer.get_bytes_per_row_of_plane(1);
            let y_plane_h = pixel_buffer.get_height_of_plane(0);
            let uv_plane_h = pixel_buffer.get_height_of_plane(1);
            if y_plane_h < h || uv_plane_h < (h / 2) || y_stride < w || uv_stride < w {
                return None;
            }

            let y_ptr = unsafe { pixel_buffer.get_base_address_of_plane(0) as *mut u8 };
            let uv_ptr = unsafe { pixel_buffer.get_base_address_of_plane(1) as *mut u8 };
            if y_ptr.is_null() || uv_ptr.is_null() {
                return None;
            }

            let y_len = y_stride * y_plane_h;
            let uv_len = uv_stride * uv_plane_h;
            let y_plane = unsafe { std::slice::from_raw_parts_mut(y_ptr, y_len) };
            let uv_plane = unsafe { std::slice::from_raw_parts_mut(uv_ptr, uv_len) };

            for row in 0..h {
                let src_off = row * y_src_stride;
                let dst_off = row * y_stride;
                y_plane[dst_off..(dst_off + w)].copy_from_slice(&y_src[src_off..(src_off + w)]);
            }
            for row in 0..(h / 2) {
                let src_off = row * uv_src_stride;
                let dst_off = row * uv_stride;
                uv_plane[dst_off..(dst_off + w)].copy_from_slice(&uv_src[src_off..(src_off + w)]);
            }
            Some(())
        })()
        .is_some();

        let _ = pixel_buffer.unlock_base_address(0);
        if copied { Some(pixel_buffer) } else { None }
    }

    #[cfg(target_os = "macos")]
    fn log_strict_surface_failure(
        video_id: u64,
        reason: &str,
        width: u32,
        height: u32,
        format: &str,
    ) {
        static FAIL_COUNT: AtomicU64 = AtomicU64::new(0);
        let hit = FAIL_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        if hit <= 8 || hit % 120 == 0 {
            log::warn!(
                "[Video {}] strict-nv12-surface miss hit={} reason={} frame={}x{} format={}",
                video_id,
                hit,
                reason,
                width,
                height,
                format
            );
        }
    }

    #[cfg(target_os = "macos")]
    fn nv12_debug_enabled() -> bool {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| {
            std::env::var("ANICA_DEBUG_NV12_PATH")
                .ok()
                .map(|raw| {
                    let s = raw.trim();
                    s == "1"
                        || s.eq_ignore_ascii_case("true")
                        || s.eq_ignore_ascii_case("yes")
                        || s.eq_ignore_ascii_case("on")
                })
                .unwrap_or(false)
        })
    }

    #[cfg(target_os = "macos")]
    fn nv12_pixel_format_tag(pixel_format: u32) -> &'static str {
        if pixel_format == kCVPixelFormatType_32BGRA {
            "BGRA"
        } else if pixel_format == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange {
            "420f"
        } else if pixel_format == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange {
            "420v"
        } else {
            "other"
        }
    }

    #[cfg(target_os = "macos")]
    fn log_nv12_surface_path(
        video_id: u64,
        path: &str,
        pixel_format: u32,
        width: u32,
        height: u32,
        note: &str,
    ) {
        if !Self::nv12_debug_enabled() {
            return;
        }
        static HIT_COUNT: AtomicU64 = AtomicU64::new(0);
        let hit = HIT_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        if hit <= 20 || hit % 120 == 0 {
            log::info!(
                "[Video {}][SurfacePath] hit={} path={} fmt={}({:#x}) frame={}x{} note={}",
                video_id,
                hit,
                path,
                Self::nv12_pixel_format_tag(pixel_format),
                pixel_format,
                width,
                height,
                note
            );
        }
    }

    #[cfg(target_os = "macos")]
    fn map_video_range_luma_to_full(v: u8) -> u8 {
        // studio/video range luma [16,235] -> full range [0,255]
        let x = v as i32;
        (((x - 16) * 255 + 109) / 219).clamp(0, 255) as u8
    }

    #[cfg(target_os = "macos")]
    fn map_video_range_chroma_to_full(v: u8) -> u8 {
        // studio/video range chroma [16,240] -> full range [0,255]
        let x = v as i32;
        (((x - 16) * 255 + 112) / 224).clamp(0, 255) as u8
    }

    #[cfg(target_os = "macos")]
    fn build_surface_nv12_copy_from_planes_video_to_full(
        width: u32,
        height: u32,
        y_src: &[u8],
        y_src_stride: usize,
        uv_src: &[u8],
        uv_src_stride: usize,
        cv_options: &CFDictionary<CFString, CFType>,
    ) -> Option<CVPixelBuffer> {
        let w = width as usize;
        let h = height as usize;
        if w == 0 || h == 0 || (w & 1) != 0 || (h & 1) != 0 || y_src_stride < w || uv_src_stride < w
        {
            return None;
        }
        if y_src.len() < y_src_stride.checked_mul(h)?
            || uv_src.len() < uv_src_stride.checked_mul(h / 2)?
        {
            return None;
        }

        let mut y_full = vec![0u8; w.checked_mul(h)?];
        let mut uv_full = vec![0u8; w.checked_mul(h / 2)?];

        for row in 0..h {
            let src_off = row.checked_mul(y_src_stride)?;
            let dst_off = row.checked_mul(w)?;
            for col in 0..w {
                y_full[dst_off + col] = Self::map_video_range_luma_to_full(y_src[src_off + col]);
            }
        }
        for row in 0..(h / 2) {
            let src_off = row.checked_mul(uv_src_stride)?;
            let dst_off = row.checked_mul(w)?;
            for col in 0..w {
                uv_full[dst_off + col] =
                    Self::map_video_range_chroma_to_full(uv_src[src_off + col]);
            }
        }

        Self::build_surface_nv12_copy_from_planes(
            width, height, &y_full, w, &uv_full, w, cv_options,
        )
    }

    #[cfg(target_os = "macos")]
    fn current_frame_surface_from_core_video_meta_420v_to_420f(
        frame: &Frame,
        width: u32,
        height: u32,
        cv_options: &CFDictionary<CFString, CFType>,
    ) -> Option<CVPixelBuffer> {
        let sample = match frame {
            Frame::Gst(sample) => sample,
            Frame::RawBgra { .. } => return None,
        };
        let sample_buffer = sample.buffer()?;
        let mut result: Option<CVPixelBuffer> = None;

        sample_buffer.foreach_meta(|meta| {
            if meta.api().name() != "GstCoreVideoMetaAPI" {
                return ControlFlow::Continue(());
            }

            let raw_meta = meta.as_ptr() as *const GstCoreVideoMetaRaw;
            if raw_meta.is_null() {
                return ControlFlow::Continue(());
            }
            let raw_pixel_buffer = unsafe { (*raw_meta).pixel_buffer };
            if raw_pixel_buffer.is_null() {
                return ControlFlow::Continue(());
            }

            let surface = unsafe { CVPixelBuffer::wrap_under_get_rule(raw_pixel_buffer) };
            let pixel_format = surface.get_pixel_format();
            if pixel_format != kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
                || surface.get_plane_count() < 2
            {
                return ControlFlow::Continue(());
            }

            if surface.lock_base_address(0) != kCVReturnSuccess {
                return ControlFlow::Continue(());
            }

            let converted = (|| {
                let w = width as usize;
                let h = height as usize;
                if w == 0 || h == 0 || (w & 1) != 0 || (h & 1) != 0 {
                    return None;
                }

                let y_stride = surface.get_bytes_per_row_of_plane(0);
                let uv_stride = surface.get_bytes_per_row_of_plane(1);
                let y_plane_h = surface.get_height_of_plane(0);
                let uv_plane_h = surface.get_height_of_plane(1);
                if y_stride < w || uv_stride < w || y_plane_h < h || uv_plane_h < (h / 2) {
                    return None;
                }

                let y_ptr = unsafe { surface.get_base_address_of_plane(0) as *const u8 };
                let uv_ptr = unsafe { surface.get_base_address_of_plane(1) as *const u8 };
                if y_ptr.is_null() || uv_ptr.is_null() {
                    return None;
                }

                let y_src = unsafe { std::slice::from_raw_parts(y_ptr, y_stride * y_plane_h) };
                let uv_src = unsafe { std::slice::from_raw_parts(uv_ptr, uv_stride * uv_plane_h) };

                Self::build_surface_nv12_copy_from_planes_video_to_full(
                    width, height, y_src, y_stride, uv_src, uv_stride, cv_options,
                )
            })();

            let _ = surface.unlock_base_address(0);

            if let Some(surface_420f) = converted {
                result = Some(surface_420f);
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });

        result
    }

    #[cfg(target_os = "macos")]
    fn current_frame_surface_from_core_video_meta(
        frame: &Frame,
    ) -> Result<CVPixelBuffer, &'static str> {
        let sample = match frame {
            Frame::Gst(sample) => sample,
            Frame::RawBgra { .. } => return Err("raw-bgra-frame"),
        };
        let sample_buffer = sample.buffer().ok_or("missing-sample-buffer")?;
        let mut result: Option<CVPixelBuffer> = None;
        let mut saw_core_video_meta = false;
        let mut saw_null_pixel_buffer = false;
        let mut saw_unsupported_pixel_format = false;

        // Prefer native CoreVideo-backed buffers to avoid per-frame NV12 plane copy on macOS.
        sample_buffer.foreach_meta(|meta| {
            if meta.api().name() != "GstCoreVideoMetaAPI" {
                return ControlFlow::Continue(());
            }
            saw_core_video_meta = true;

            let raw_meta = meta.as_ptr() as *const GstCoreVideoMetaRaw;
            if raw_meta.is_null() {
                saw_null_pixel_buffer = true;
                return ControlFlow::Continue(());
            }
            let raw_pixel_buffer = unsafe { (*raw_meta).pixel_buffer };
            if raw_pixel_buffer.is_null() {
                saw_null_pixel_buffer = true;
                return ControlFlow::Continue(());
            }

            let surface = unsafe { CVPixelBuffer::wrap_under_get_rule(raw_pixel_buffer) };
            let pixel_format = surface.get_pixel_format();
            let plane_count = surface.get_plane_count();
            // Accept both 420f (full-range) and 420v (video-range) NV12 for zero-copy.
            // GPUI Metal shader handles both via the color_range uniform flag.
            let is_supported_nv12 = (pixel_format
                == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
                || pixel_format == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange)
                && plane_count >= 2;

            if is_supported_nv12 {
                result = Some(surface);
                ControlFlow::Break(())
            } else {
                saw_unsupported_pixel_format = true;
                ControlFlow::Continue(())
            }
        });

        if let Some(surface) = result {
            return Ok(surface);
        }
        if !saw_core_video_meta {
            return Err("no-corevideo-meta");
        }
        if saw_null_pixel_buffer {
            return Err("corevideo-meta-null-pixel-buffer");
        }
        if saw_unsupported_pixel_format {
            return Err("corevideo-meta-unsupported-pixel-format");
        }
        Err("corevideo-meta-unavailable")
    }

    /// Return true when the active preview frame is an FFmpeg BGRA frame.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub fn current_frame_is_raw_bgra(&self) -> bool {
        let inner = self.read();
        let frame_guard = inner.frame.lock();
        matches!(
            &*frame_guard,
            Frame::RawBgra {
                data,
                width,
                height
            } if !data.is_empty() && *width > 0 && *height > 0
        )
    }

    /// Return current FFmpeg BGRA dimensions without cloning the frame bytes.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub fn current_raw_bgra_dimensions(&self) -> Option<(u32, u32)> {
        let inner = self.read();
        let frame_guard = inner.frame.lock();
        match &*frame_guard {
            Frame::RawBgra {
                data,
                width,
                height,
            } if !data.is_empty() && *width > 0 && *height > 0 => Some((*width, *height)),
            _ => None,
        }
    }

    /// Run a closure with the current FFmpeg BGRA bytes without cloning them.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub fn with_current_raw_bgra_frame<R>(
        &self,
        f: impl FnOnce(&[u8], u32, u32) -> R,
    ) -> Option<R> {
        let inner = self.read();
        let frame_guard = inner.frame.lock();
        match &*frame_guard {
            Frame::RawBgra {
                data,
                width,
                height,
            } if !data.is_empty() && *width > 0 && *height > 0 => {
                Some(f(data.as_slice(), *width, *height))
            }
            _ => None,
        }
    }

    /// Copy the current FFmpeg BGRA frame into a caller-owned CVPixelBuffer.
    #[cfg(target_os = "macos")]
    pub fn copy_current_frame_to_bgra_surface(
        &self,
        surface: &CVPixelBuffer,
    ) -> Option<(u32, u32)> {
        let inner = self.read();
        let frame_guard = inner.frame.lock();
        if let Frame::RawBgra {
            data,
            width,
            height,
        } = &*frame_guard
        {
            if !data.is_empty()
                && *width > 0
                && *height > 0
                && Self::copy_bgra_into_surface(surface, *width, *height, data.as_slice())
            {
                return Some((*width, *height));
            }
        }
        None
    }

    /// Build a macOS BGRA `CVPixelBuffer` for GPUI surface rendering.
    #[cfg(target_os = "macos")]
    pub fn current_frame_surface_bgra(&self) -> Option<CVPixelBuffer> {
        let inner = self.read();
        let video_id = inner.id;
        let frame_guard = inner.frame.lock();
        if let Frame::RawBgra {
            data,
            width,
            height,
        } = &*frame_guard
        {
            let surface = Self::build_surface_bgra_copy_from_data(*width, *height, data.as_slice());
            if let Some(surface) = surface.as_ref() {
                Self::log_nv12_surface_path(
                    video_id,
                    "ffmpeg-bgra-surface",
                    surface.get_pixel_format(),
                    surface.get_width() as u32,
                    surface.get_height() as u32,
                    "from-raw-bgra",
                );
            }
            return surface;
        }
        None
    }

    /// Build a macOS `CVPixelBuffer` in NV12 format for GPUI `paint_surface`.
    /// Uses 420f directly; other NV12 variants fall back to a safe 420f copy path.
    #[cfg(target_os = "macos")]
    pub fn current_frame_surface_nv12(&self) -> Option<CVPixelBuffer> {
        let inner = self.read();
        let video_id = inner.id;
        let strict_surface = inner.strict_surface_proxy_nv12;
        let frame_guard = inner.frame.lock();
        let iosurface_props: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[]);
        let cv_options: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[
            (
                CFString::from(CVPixelBufferKeys::MetalCompatibility),
                CFBoolean::true_value().as_CFType(),
            ),
            (
                CFString::from(CVPixelBufferKeys::IOSurfaceProperties),
                iosurface_props.as_CFType(),
            ),
        ]);
        if let Frame::RawBgra {
            data,
            width,
            height,
        } = &*frame_guard
        {
            let surface = Self::build_surface_nv12_copy_from_bgra(
                *width,
                *height,
                data.as_slice(),
                &cv_options,
            );
            if let Some(surface) = surface.as_ref() {
                Self::log_nv12_surface_path(
                    video_id,
                    "ffmpeg-bgra-copy",
                    surface.get_pixel_format(),
                    surface.get_width() as u32,
                    surface.get_height() as u32,
                    "from-raw-bgra",
                );
            }
            return surface;
        }
        let sample = match &*frame_guard {
            Frame::Gst(sample) => sample,
            Frame::RawBgra { .. } => return None,
        };

        match Self::current_frame_surface_from_core_video_meta(&frame_guard) {
            Ok(surface) => {
                Self::log_nv12_surface_path(
                    video_id,
                    "corevideo-meta",
                    surface.get_pixel_format(),
                    surface.get_width() as u32,
                    surface.get_height() as u32,
                    "zero-copy",
                );
                return Some(surface);
            }
            Err(reason) => {
                if Self::nv12_debug_enabled() {
                    static MISS_COUNT: AtomicU64 = AtomicU64::new(0);
                    let hit = MISS_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                    if hit <= 20 || hit % 120 == 0 {
                        log::info!(
                            "[Video {}][NV12Path] corevideo-meta miss hit={} reason={}",
                            video_id,
                            hit,
                            reason
                        );
                    }
                }
            }
        }

        // Resolve frame size from current sample caps when available.
        let mut width = inner.width.max(0) as u32;
        let mut height = inner.height.max(0) as u32;
        let mut format = String::from("NV12");
        let caps = sample.caps();
        if let Some(caps_ref) = caps.as_ref()
            && let Some(s) = caps_ref.structure(0)
        {
            if let Ok(w) = s.get::<i32>("width")
                && w > 0
            {
                width = w as u32;
            }
            if let Ok(h) = s.get::<i32>("height")
                && h > 0
            {
                height = h as u32;
            }
            if let Ok(fmt) = s.get::<String>("format")
                && !fmt.is_empty()
            {
                format = fmt;
            }
        }

        // Surface path expects the sample to already be NV12.
        if !format.eq_ignore_ascii_case("NV12") || width == 0 || height == 0 {
            if strict_surface {
                Self::log_strict_surface_failure(
                    video_id,
                    "invalid-sample-format-or-size",
                    width,
                    height,
                    &format,
                );
            }
            return None;
        }

        // Some macOS/Videotoolbox proxy streams report odd display width in caps
        // (e.g. 359x202) while underlying NV12 planes are still even-aligned.
        // Strict proxy surface mode normalizes to even before building CVPixelBuffer.
        let surface_width = if strict_surface {
            (width + 1) & !1
        } else {
            width
        };
        let surface_height = if strict_surface {
            (height + 1) & !1
        } else {
            height
        };
        if strict_surface && (surface_width != width || surface_height != height) {
            static NORMALIZE_COUNT: AtomicU64 = AtomicU64::new(0);
            let hit = NORMALIZE_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            if hit <= 8 || hit % 120 == 0 {
                log::info!(
                    "[Video {}] strict-nv12-surface normalize hit={} frame={}x{} -> {}x{}",
                    video_id,
                    hit,
                    width,
                    height,
                    surface_width,
                    surface_height
                );
            }
        }
        if (surface_width & 1) != 0 || (surface_height & 1) != 0 {
            if strict_surface {
                Self::log_strict_surface_failure(
                    video_id,
                    "surface-size-not-even-after-normalize",
                    width,
                    height,
                    &format,
                );
            }
            return None;
        }

        if inner.strict_surface_proxy_nv12
            && let Some(surface) = Self::current_frame_surface_from_core_video_meta_420v_to_420f(
                &frame_guard,
                surface_width,
                surface_height,
                &cv_options,
            )
        {
            Self::log_nv12_surface_path(
                video_id,
                "corevideo-meta-420v-to-420f",
                surface.get_pixel_format(),
                surface.get_width() as u32,
                surface.get_height() as u32,
                "strict-convert",
            );
            return Some(surface);
        }

        let buffer = sample.buffer();
        if let (Some(caps_ref), Some(buffer_ref)) = (caps.as_ref(), buffer)
            && let Ok(info) = gst_video::VideoInfo::from_caps(caps_ref)
            && let Ok(video_frame) =
                gst_video::VideoFrameRef::from_buffer_ref_readable(buffer_ref, &info)
        {
            let y_stride_i32 = *video_frame.plane_stride().first().unwrap_or(&0);
            let uv_stride_i32 = *video_frame.plane_stride().get(1).unwrap_or(&0);
            if y_stride_i32 > 0
                && uv_stride_i32 > 0
                && let (Ok(y_plane), Ok(uv_plane)) =
                    (video_frame.plane_data(0), video_frame.plane_data(1))
                && let Some(surface) = Self::build_surface_nv12_copy_from_planes(
                    surface_width,
                    surface_height,
                    y_plane,
                    y_stride_i32 as usize,
                    uv_plane,
                    uv_stride_i32 as usize,
                    &cv_options,
                )
            {
                Self::log_nv12_surface_path(
                    video_id,
                    "plane-copy",
                    surface.get_pixel_format(),
                    surface.get_width() as u32,
                    surface.get_height() as u32,
                    "from-video-frame-planes",
                );
                return Some(surface);
            }
        }

        let readable = frame_guard.readable()?;
        // Keep a stable, Metal-compatible surface path to avoid renderer panics.
        let fallback_surface = Self::build_surface_nv12_copy(
            surface_width,
            surface_height,
            readable.as_slice(),
            &cv_options,
        );
        if strict_surface && fallback_surface.is_none() {
            Self::log_strict_surface_failure(
                video_id,
                "surface-copy-failed",
                width,
                height,
                &format,
            );
        }
        if let Some(surface) = fallback_surface.as_ref() {
            Self::log_nv12_surface_path(
                video_id,
                "readable-copy",
                surface.get_pixel_format(),
                surface.get_width() as u32,
                surface.get_height() as u32,
                "from-readable-buffer",
            );
        } else if Self::nv12_debug_enabled() {
            static FAIL_COUNT: AtomicU64 = AtomicU64::new(0);
            let hit = FAIL_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            if hit <= 20 || hit % 120 == 0 {
                log::warn!(
                    "[Video {}][NV12Path] fallback readable-copy failed hit={} frame={}x{} format={} strict={}",
                    video_id,
                    hit,
                    width,
                    height,
                    format,
                    strict_surface
                );
            }
        }
        fallback_surface
    }

    /// Backward-compatible alias kept while callers migrate to the range-agnostic name.
    #[cfg(target_os = "macos")]
    pub fn current_frame_surface_nv12_full(&self) -> Option<CVPixelBuffer> {
        self.current_frame_surface_nv12()
    }

    pub fn strict_surface_only(&self) -> bool {
        self.read().strict_surface_proxy_nv12
    }

    /// Returns true if a new frame arrived since last check and resets the flag.
    pub fn take_frame_ready(&self) -> bool {
        self.read().upload_frame.swap(false, Ordering::SeqCst)
    }

    /// Configure the frame buffer capacity (0 disables buffering).
    pub fn set_frame_buffer_capacity(&self, capacity: usize) {
        let inner = self.read();
        if inner.ffmpeg.is_some() {
            // FFmpeg preview uses latest-frame-only transport; historical buffers create avoidable lag.
            inner.frame_buffer_capacity.store(0, Ordering::SeqCst);
            inner.frame_buffer.lock().clear();
            return;
        }
        inner
            .frame_buffer_capacity
            .store(capacity, Ordering::SeqCst);
        if capacity == 0 {
            inner.frame_buffer.lock().clear();
        } else {
            let mut buf = inner.frame_buffer.lock();
            while buf.len() > capacity {
                buf.pop_front();
            }
        }
    }

    /// Retrieve the current frame buffer capacity.
    pub fn frame_buffer_capacity(&self) -> usize {
        self.read().frame_buffer_capacity.load(Ordering::SeqCst)
    }

    /// Pop the oldest buffered frame, returning raw NV12 bytes with width/height.
    /// Returns None if the buffer is empty or mapping fails.
    pub fn pop_buffered_frame(&self) -> Option<(Vec<u8>, u32, u32)> {
        let (width, height) = self.size();
        let inner = self.read();
        let maybe_frame = inner.frame_buffer.lock().pop_front();
        if let Some(frame) = maybe_frame
            && let Some(readable) = frame.readable()
        {
            let data = readable.as_slice().to_vec();
            if !data.is_empty() {
                return Some((data, width as u32, height as u32));
            }
        }
        None
    }

    /// Number of frames currently buffered.
    pub fn buffered_len(&self) -> usize {
        self.read().frame_buffer.lock().len()
    }

    pub fn peek_frame_ready(&self) -> bool {
        self.read().upload_frame.load(Ordering::Acquire)
    }

    /// Last decoded frame PTS in nanoseconds. `0` means unknown/not-yet-sampled.
    pub fn last_frame_pts_ns(&self) -> u64 {
        self.read().last_frame_pts_ns.load(Ordering::Acquire)
    }

    /// Total number of decoded samples pulled by the worker for this video instance.
    pub fn decoded_frame_counter(&self) -> u64 {
        self.read().decoded_frame_counter.load(Ordering::Acquire)
    }
}
