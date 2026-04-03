// =========================================
// =========================================
// src/core/subtitle_renderer.rs
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache};
use image::{DynamicImage, Rgba, RgbaImage, imageops::FilterType};
use thiserror::Error;

use crate::core::global_state::{SubtitleClip, SubtitleGroupTransform, SubtitleTrack};
use crate::runtime_paths;

#[derive(Debug, Error)]
pub enum SubtitleRenderError {
    #[error("failed to read time: {source}")]
    ReadTime { source: std::time::SystemTimeError },
    #[error("failed to create subtitle temp dir ({path}): {source}")]
    CreateTempDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to save subtitle png ({path}): {source}")]
    SaveSubtitlePng {
        path: PathBuf,
        source: image::ImageError,
    },
}

pub struct RenderedSubtitle {
    pub path: PathBuf,
    pub start: f64,
    pub end: f64,
}

pub struct SubtitleRenderOutput {
    pub overlays: Vec<RenderedSubtitle>,
    temp_dir: PathBuf,
}

impl Drop for SubtitleRenderOutput {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.temp_dir);
    }
}

pub fn render_subtitle_pngs(
    subtitle_tracks: &[SubtitleTrack],
    subtitle_groups: &std::collections::HashMap<u64, SubtitleGroupTransform>,
    output_canvas_w: u32,
    output_canvas_h: u32,
    layout_canvas_w: u32,
    layout_canvas_h: u32,
) -> Result<SubtitleRenderOutput, SubtitleRenderError> {
    if subtitle_tracks.is_empty() {
        return Ok(SubtitleRenderOutput {
            overlays: Vec::new(),
            temp_dir: std::env::temp_dir().join("anica_subtitles_export_empty"),
        });
    }

    let temp_dir = create_temp_dir()?;
    let mut font_system = FontSystem::new();
    load_extra_fonts(&mut font_system);
    let mut swash_cache = SwashCache::new();
    let mut emoji_cache: HashMap<String, DynamicImage> = HashMap::new();

    let mut overlays = Vec::new();
    for track in subtitle_tracks {
        for clip in &track.clips {
            let path = temp_dir.join(format!("subtitle_{}.png", clip.id));
            render_clip_to_png(
                &mut font_system,
                &mut swash_cache,
                &mut emoji_cache,
                clip,
                subtitle_groups,
                output_canvas_w,
                output_canvas_h,
                layout_canvas_w,
                layout_canvas_h,
                &path,
            )?;
            overlays.push(RenderedSubtitle {
                path,
                start: clip.start.as_secs_f64(),
                end: (clip.start + clip.duration).as_secs_f64(),
            });
        }
    }

    Ok(SubtitleRenderOutput { overlays, temp_dir })
}

fn create_temp_dir() -> Result<PathBuf, SubtitleRenderError> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| SubtitleRenderError::ReadTime { source })?
        .as_millis();
    let dir = std::env::temp_dir().join(format!("anica_subtitles_export_{stamp}"));
    fs::create_dir_all(&dir).map_err(|source| SubtitleRenderError::CreateTempDir {
        path: dir.clone(),
        source,
    })?;
    Ok(dir)
}

fn load_extra_fonts(font_system: &mut FontSystem) {
    for dir in runtime_paths::candidate_font_dirs() {
        if !dir.exists() {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            let ext = ext.to_ascii_lowercase();
            if ext == "ttf" || ext == "otf" || ext == "ttc" {
                let _ = font_system.db_mut().load_font_file(&path);
            }
        }
    }
}

fn render_clip_to_png(
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    emoji_cache: &mut HashMap<String, DynamicImage>,
    clip: &SubtitleClip,
    subtitle_groups: &HashMap<u64, SubtitleGroupTransform>,
    output_canvas_w: u32,
    output_canvas_h: u32,
    layout_canvas_w: u32,
    layout_canvas_h: u32,
    output_path: &Path,
) -> Result<(), SubtitleRenderError> {
    let mut canvas = RgbaImage::from_pixel(output_canvas_w, output_canvas_h, Rgba([0, 0, 0, 0]));

    if clip.text.trim().is_empty() {
        canvas
            .save(output_path)
            .map_err(|source| SubtitleRenderError::SaveSubtitlePng {
                path: output_path.to_path_buf(),
                source,
            })?;
        return Ok(());
    }

    let (pos_x, pos_y, base_font_size) = if let Some(group_id) = clip.group_id
        && let Some(group) = subtitle_groups.get(&group_id)
    {
        (
            clip.pos_x + group.offset_x,
            clip.pos_y + group.offset_y,
            (clip.font_size * group.scale).max(1.0),
        )
    } else {
        (clip.pos_x, clip.pos_y, clip.font_size.max(1.0))
    };

    let layout_w = layout_canvas_w.max(1) as f32;
    let layout_h = layout_canvas_h.max(1) as f32;
    let output_w = output_canvas_w.max(1) as f32;
    let output_h = output_canvas_h.max(1) as f32;
    // Subtitle font size is authored against the editor canvas. Scale it to
    // output resolution to preserve preview parity for non-canvas exports.
    let font_scale = (output_w / layout_w).min(output_h / layout_h).max(0.01);
    let font_size = (base_font_size * font_scale).max(1.0);

    let line_height = (font_size * 1.2).max(1.0);
    let metrics = Metrics::new(font_size, line_height);
    if let Some(path) = clip.font_path.as_ref()
        && Path::new(path).exists()
    {
        let _ = font_system.db_mut().load_font_file(path);
    }

    let mut buffer = Buffer::new(font_system, metrics);
    let mut attrs = Attrs::new().family(Family::SansSerif);
    if let Some(family) = clip.font_family.as_deref() {
        attrs = attrs.family(Family::Name(family));
    }
    buffer.set_text(font_system, &clip.text, &attrs, Shaping::Advanced);
    buffer.set_size(font_system, None, None);
    buffer.shape_until_scroll(font_system, true);

    let x_base = (output_canvas_w as f32 / 2.0) + (pos_x * output_canvas_w as f32);
    let y_base = (output_canvas_h as f32 / 2.0) + (pos_y * output_canvas_h as f32);

    let (r, g, b, a) = clip.color_rgba;
    let text_color = Color::rgba(r, g, b, a);

    buffer.draw(
        font_system,
        swash_cache,
        text_color,
        |x, y, _w, _h, color| {
            let px = x_base as i32 + x;
            let py = y_base as i32 + y;
            if px < 0 || py < 0 {
                return;
            }
            let (px, py) = (px as u32, py as u32);
            if px >= output_canvas_w || py >= output_canvas_h {
                return;
            }
            let (sr, sg, sb, sa) = color.as_rgba_tuple();
            blend_pixel(&mut canvas, px, py, [sr, sg, sb, sa]);
        },
    );

    let emoji_spans = collect_emoji_spans(&buffer, x_base, y_base);
    for span in emoji_spans {
        if let Some(image) = load_twemoji_image(&span.code, emoji_cache) {
            let target_size = span.h.max(1.0).round() as u32;
            let emoji = image.resize_exact(target_size, target_size, FilterType::Lanczos3);
            clear_rect(&mut canvas, span.x, span.y, span.w, span.h);
            let offset_x = span.x + (span.w - target_size as f32) / 2.0;
            let offset_y = span.y + (span.h - target_size as f32) / 2.0;
            overlay_image(&mut canvas, &emoji, offset_x, offset_y);
        }
    }

    canvas
        .save(output_path)
        .map_err(|source| SubtitleRenderError::SaveSubtitlePng {
            path: output_path.to_path_buf(),
            source,
        })?;
    Ok(())
}

struct EmojiSpan {
    code: String,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

fn collect_emoji_spans(buffer: &Buffer, x_base: f32, y_base: f32) -> Vec<EmojiSpan> {
    let mut spans = Vec::new();
    for run in buffer.layout_runs() {
        for glyph in run.glyphs.iter() {
            let Some(cluster) = run.text.get(glyph.start..glyph.end) else {
                continue;
            };
            if !is_emoji_cluster(cluster) {
                continue;
            }
            let Some(code) = twemoji_code(cluster) else {
                continue;
            };
            spans.push(EmojiSpan {
                code,
                x: x_base + glyph.x,
                y: y_base + run.line_top,
                w: glyph.w.max(run.line_height),
                h: run.line_height,
            });
        }
    }
    spans
}

fn is_emoji_cluster(cluster: &str) -> bool {
    cluster.chars().any(is_emoji_char)
}

fn is_emoji_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x1F000..=0x1FAFF
            | 0x2600..=0x26FF
            | 0x2700..=0x27BF
            | 0x200D
            | 0xFE0F
    )
}

fn twemoji_code(cluster: &str) -> Option<String> {
    let mut codepoints: Vec<u32> = cluster.chars().map(|c| c as u32).collect();
    if codepoints.is_empty() {
        return None;
    }
    let primary = join_codepoints(&codepoints);
    if twemoji_asset_exists(&primary) {
        return Some(primary);
    }
    codepoints.retain(|c| *c != 0xFE0F);
    if codepoints.is_empty() {
        return None;
    }
    let fallback = join_codepoints(&codepoints);
    if twemoji_asset_exists(&fallback) {
        return Some(fallback);
    }
    Some(primary)
}

fn join_codepoints(codepoints: &[u32]) -> String {
    let mut out = String::new();
    for (i, cp) in codepoints.iter().enumerate() {
        if i > 0 {
            out.push('-');
        }
        out.push_str(&format!("{:x}", cp));
    }
    out
}

fn twemoji_asset_exists(code: &str) -> bool {
    for dir in twemoji_search_dirs() {
        let path = dir.join(format!("{code}.png"));
        if path.exists() {
            return true;
        }
    }
    false
}

fn load_twemoji_image(
    code: &str,
    cache: &mut HashMap<String, DynamicImage>,
) -> Option<DynamicImage> {
    if let Some(image) = cache.get(code) {
        return Some(image.clone());
    }

    for dir in twemoji_search_dirs() {
        let path = dir.join(format!("{code}.png"));
        if let Ok(bytes) = fs::read(&path)
            && let Ok(image) = image::load_from_memory(&bytes)
        {
            cache.insert(code.to_string(), image.clone());
            return Some(image);
        }
    }

    if std::env::var("ANICA_TWEMOJI_DOWNLOAD").ok().as_deref() == Some("1")
        && let Some(image) = download_twemoji(code)
    {
        cache.insert(code.to_string(), image.clone());
        return Some(image);
    }

    None
}

fn twemoji_search_dirs() -> Vec<PathBuf> {
    runtime_paths::candidate_twemoji_dirs()
}

fn download_twemoji(code: &str) -> Option<DynamicImage> {
    let base = std::env::var("ANICA_TWEMOJI_BASE_URL").unwrap_or_else(|_| {
        "https://cdnjs.cloudflare.com/ajax/libs/twemoji/14.0.2/72x72".to_string()
    });
    let url = format!("{}/{}.png", base.trim_end_matches('/'), code);

    let response = ureq::get(&url).call().ok()?;
    let mut bytes = Vec::new();
    response.into_reader().read_to_end(&mut bytes).ok()?;

    let image = image::load_from_memory(&bytes).ok()?;

    let cache_dir = std::env::temp_dir().join("anica_twemoji/72x72");
    let _ = fs::create_dir_all(&cache_dir);
    let path = cache_dir.join(format!("{code}.png"));
    let _ = fs::write(path, bytes);

    Some(image)
}

fn blend_pixel(canvas: &mut RgbaImage, x: u32, y: u32, src: [u8; 4]) {
    let dst = canvas.get_pixel_mut(x, y);
    let (sr, sg, sb, sa) = (src[0] as f32, src[1] as f32, src[2] as f32, src[3] as f32);
    let (dr, dg, db, da) = (dst[0] as f32, dst[1] as f32, dst[2] as f32, dst[3] as f32);

    let sa = sa / 255.0;
    let da = da / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        *dst = Rgba([0, 0, 0, 0]);
        return;
    }

    let out_r = (sr * sa + dr * da * (1.0 - sa)) / out_a;
    let out_g = (sg * sa + dg * da * (1.0 - sa)) / out_a;
    let out_b = (sb * sa + db * da * (1.0 - sa)) / out_a;

    *dst = Rgba([
        out_r.round().clamp(0.0, 255.0) as u8,
        out_g.round().clamp(0.0, 255.0) as u8,
        out_b.round().clamp(0.0, 255.0) as u8,
        (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
    ]);
}

fn clear_rect(canvas: &mut RgbaImage, x: f32, y: f32, w: f32, h: f32) {
    let start_x = x.floor() as i32;
    let start_y = y.floor() as i32;
    let end_x = (x + w).ceil() as i32;
    let end_y = (y + h).ceil() as i32;
    let width = canvas.width() as i32;
    let height = canvas.height() as i32;

    for py in start_y.max(0)..end_y.min(height) {
        for px in start_x.max(0)..end_x.min(width) {
            let pixel = canvas.get_pixel_mut(px as u32, py as u32);
            *pixel = Rgba([0, 0, 0, 0]);
        }
    }
}

fn overlay_image(canvas: &mut RgbaImage, image: &DynamicImage, x: f32, y: f32) {
    let x = x.floor() as i64;
    let y = y.floor() as i64;
    let overlay = image.to_rgba8();
    image::imageops::overlay(canvas, &overlay, x, y);
}
