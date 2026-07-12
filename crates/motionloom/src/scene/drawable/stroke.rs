use crate::scene::drawable::Point2;
use crate::scene::model::{LineNode, PathNode, PolylineNode};
use crate::scene::render::{MotionLoomSceneRenderError, eval_scene_number};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StrokeCap {
    Round,
    Butt,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StrokeJoin {
    Round,
    Miter,
    Bevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StrokeTexture {
    Solid,
    Sketch,
    Rough,
    Pencil,
    Ink,
    Charcoal,
    Marker,
    Hairline,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct StrokeStyle {
    pub(crate) cap: StrokeCap,
    pub(crate) join: StrokeJoin,
    pub(crate) taper_start: f32,
    pub(crate) taper_end: f32,
    pub(crate) width_start: f32,
    pub(crate) width_end: f32,
    pub(crate) width_profile: [(f32, f32); 8],
    pub(crate) width_profile_len: usize,
    pub(crate) texture: StrokeTexture,
    pub(crate) roughness: f32,
    pub(crate) copies: u32,
    pub(crate) texture_strength: f32,
    pub(crate) bristles: u32,
    pub(crate) pressure_auto: bool,
    pub(crate) pressure_min: f32,
    pub(crate) pressure_curve: f32,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        Self {
            cap: StrokeCap::Round,
            join: StrokeJoin::Round,
            taper_start: 0.0,
            taper_end: 0.0,
            width_start: 1.0,
            width_end: 1.0,
            width_profile: [(0.0, 1.0); 8],
            width_profile_len: 0,
            texture: StrokeTexture::Solid,
            roughness: 0.0,
            copies: 1,
            texture_strength: 0.0,
            bristles: 0,
            pressure_auto: false,
            pressure_min: 1.0,
            pressure_curve: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct StrokeParams {
    roughness: f32,
    copies: u32,
    texture_strength: f32,
    bristles: u32,
    pressure_auto: bool,
    pressure_min: f32,
    pressure_curve: f32,
}

pub(crate) fn evaluate_trim(
    trim_start: &str,
    trim_end: &str,
    time_norm: f32,
    time_sec: f32,
) -> Result<(f32, f32), MotionLoomSceneRenderError> {
    let start = eval_scene_number(trim_start, time_norm, time_sec)?.clamp(0.0, 1.0);
    let end = eval_scene_number(trim_end, time_norm, time_sec)?.clamp(0.0, 1.0);
    Ok((start, end))
}

pub(crate) fn eval_line_stroke_style(
    line: &LineNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<StrokeStyle, MotionLoomSceneRenderError> {
    let texture = parse_stroke_texture(&line.stroke_style);
    let params = eval_stroke_params(
        texture,
        &line.stroke_roughness,
        &line.stroke_copies,
        &line.stroke_texture,
        &line.stroke_bristles,
        &line.stroke_pressure,
        &line.stroke_pressure_min,
        &line.stroke_pressure_curve,
        time_norm,
        time_sec,
    )?;
    Ok(StrokeStyle {
        cap: parse_stroke_cap(&line.line_cap),
        join: StrokeJoin::Round,
        taper_start: eval_scene_number(&line.taper_start, time_norm, time_sec)?.clamp(0.0, 0.5),
        taper_end: eval_scene_number(&line.taper_end, time_norm, time_sec)?.clamp(0.0, 0.5),
        width_start: 1.0,
        width_end: 1.0,
        width_profile: [(0.0, 1.0); 8],
        width_profile_len: 0,
        texture,
        roughness: params.roughness,
        copies: params.copies,
        texture_strength: params.texture_strength,
        bristles: params.bristles,
        pressure_auto: params.pressure_auto,
        pressure_min: params.pressure_min,
        pressure_curve: params.pressure_curve,
    })
}

pub(crate) fn eval_polyline_stroke_style(
    polyline: &PolylineNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<StrokeStyle, MotionLoomSceneRenderError> {
    let texture = parse_stroke_texture(&polyline.stroke_style);
    let params = eval_stroke_params(
        texture,
        &polyline.stroke_roughness,
        &polyline.stroke_copies,
        &polyline.stroke_texture,
        &polyline.stroke_bristles,
        &polyline.stroke_pressure,
        &polyline.stroke_pressure_min,
        &polyline.stroke_pressure_curve,
        time_norm,
        time_sec,
    )?;
    Ok(StrokeStyle {
        cap: parse_stroke_cap(&polyline.line_cap),
        join: parse_stroke_join(&polyline.line_join),
        taper_start: eval_scene_number(&polyline.taper_start, time_norm, time_sec)?.clamp(0.0, 0.5),
        taper_end: eval_scene_number(&polyline.taper_end, time_norm, time_sec)?.clamp(0.0, 0.5),
        width_start: 1.0,
        width_end: 1.0,
        width_profile: [(0.0, 1.0); 8],
        width_profile_len: 0,
        texture,
        roughness: params.roughness,
        copies: params.copies,
        texture_strength: params.texture_strength,
        bristles: params.bristles,
        pressure_auto: params.pressure_auto,
        pressure_min: params.pressure_min,
        pressure_curve: params.pressure_curve,
    })
}

pub(crate) fn eval_path_stroke_style(
    path: &PathNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<StrokeStyle, MotionLoomSceneRenderError> {
    let texture = parse_stroke_texture(&path.stroke_style);
    let params = eval_stroke_params(
        texture,
        &path.stroke_roughness,
        &path.stroke_copies,
        &path.stroke_texture,
        &path.stroke_bristles,
        &path.stroke_pressure,
        &path.stroke_pressure_min,
        &path.stroke_pressure_curve,
        time_norm,
        time_sec,
    )?;
    let (width_profile, width_profile_len) = parse_width_profile(&path.stroke_width_profile);
    Ok(StrokeStyle {
        cap: parse_stroke_cap(&path.line_cap),
        join: parse_stroke_join(&path.line_join),
        taper_start: eval_scene_number(&path.taper_start, time_norm, time_sec)?.clamp(0.0, 0.5),
        taper_end: eval_scene_number(&path.taper_end, time_norm, time_sec)?.clamp(0.0, 0.5),
        width_start: eval_scene_number(&path.stroke_width_start, time_norm, time_sec)?
            .clamp(0.0, 16.0),
        width_end: eval_scene_number(&path.stroke_width_end, time_norm, time_sec)?.clamp(0.0, 16.0),
        width_profile,
        width_profile_len,
        texture,
        roughness: params.roughness,
        copies: params.copies,
        texture_strength: params.texture_strength,
        bristles: params.bristles,
        pressure_auto: params.pressure_auto,
        pressure_min: params.pressure_min,
        pressure_curve: params.pressure_curve,
    })
}

fn parse_width_profile(value: &str) -> ([(f32, f32); 8], usize) {
    let mut profile = [(0.0, 1.0); 8];
    let mut len = 0usize;
    for item in value.split(',') {
        if len == profile.len() {
            break;
        }
        let mut parts = item.trim().split(':');
        let Some(offset) = parts
            .next()
            .and_then(|part| part.trim().parse::<f32>().ok())
        else {
            continue;
        };
        let Some(width) = parts
            .next()
            .and_then(|part| part.trim().parse::<f32>().ok())
        else {
            continue;
        };
        profile[len] = (offset.clamp(0.0, 1.0), width.clamp(0.0, 16.0));
        len += 1;
    }
    profile[..len].sort_by(|a, b| a.0.total_cmp(&b.0));
    (profile, len)
}

fn parse_stroke_cap(value: &str) -> StrokeCap {
    match value.trim().to_ascii_lowercase().as_str() {
        "butt" => StrokeCap::Butt,
        "square" => StrokeCap::Square,
        _ => StrokeCap::Round,
    }
}

fn parse_stroke_join(value: &str) -> StrokeJoin {
    match value.trim().to_ascii_lowercase().as_str() {
        "miter" => StrokeJoin::Miter,
        "bevel" => StrokeJoin::Bevel,
        _ => StrokeJoin::Round,
    }
}

fn parse_stroke_texture(value: &str) -> StrokeTexture {
    match value.trim().to_ascii_lowercase().as_str() {
        "sketch" | "hand" | "handdrawn" | "hand_drawn" => StrokeTexture::Sketch,
        "rough" | "dry" => StrokeTexture::Rough,
        "pencil" => StrokeTexture::Pencil,
        "ink" | "cleanink" | "clean_ink" | "boldink" | "bold_ink" => StrokeTexture::Ink,
        "charcoal" => StrokeTexture::Charcoal,
        "marker" => StrokeTexture::Marker,
        "hairline" => StrokeTexture::Hairline,
        _ => StrokeTexture::Solid,
    }
}

fn stroke_texture_defaults(texture: StrokeTexture) -> StrokeParams {
    match texture {
        StrokeTexture::Solid => StrokeParams {
            roughness: 0.0,
            copies: 1,
            texture_strength: 0.0,
            bristles: 0,
            pressure_auto: false,
            pressure_min: 1.0,
            pressure_curve: 1.0,
        },
        StrokeTexture::Sketch => StrokeParams {
            roughness: 1.45,
            copies: 3,
            texture_strength: 0.34,
            bristles: 4,
            pressure_auto: false,
            pressure_min: 0.55,
            pressure_curve: 1.25,
        },
        StrokeTexture::Rough => StrokeParams {
            roughness: 2.4,
            copies: 5,
            texture_strength: 0.62,
            bristles: 7,
            pressure_auto: false,
            pressure_min: 0.45,
            pressure_curve: 1.15,
        },
        StrokeTexture::Pencil => StrokeParams {
            roughness: 1.2,
            copies: 4,
            texture_strength: 0.68,
            bristles: 5,
            pressure_auto: false,
            pressure_min: 0.22,
            pressure_curve: 1.55,
        },
        StrokeTexture::Ink => StrokeParams {
            roughness: 0.65,
            copies: 2,
            texture_strength: 0.05,
            bristles: 0,
            pressure_auto: false,
            pressure_min: 0.74,
            pressure_curve: 0.85,
        },
        StrokeTexture::Charcoal => StrokeParams {
            roughness: 3.2,
            copies: 8,
            texture_strength: 0.90,
            bristles: 14,
            pressure_auto: false,
            pressure_min: 0.18,
            pressure_curve: 1.0,
        },
        StrokeTexture::Marker => StrokeParams {
            roughness: 0.0,
            copies: 1,
            texture_strength: 0.10,
            bristles: 0,
            pressure_auto: false,
            pressure_min: 0.80,
            pressure_curve: 0.7,
        },
        StrokeTexture::Hairline => StrokeParams {
            roughness: 0.05,
            copies: 1,
            texture_strength: 0.0,
            bristles: 0,
            pressure_auto: false,
            pressure_min: 0.95,
            pressure_curve: 1.0,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn eval_stroke_params(
    texture: StrokeTexture,
    roughness_expr: &str,
    copies_expr: &str,
    texture_expr: &str,
    bristles_expr: &str,
    pressure_expr: &str,
    pressure_min_expr: &str,
    pressure_curve_expr: &str,
    time_norm: f32,
    time_sec: f32,
) -> Result<StrokeParams, MotionLoomSceneRenderError> {
    let defaults = stroke_texture_defaults(texture);
    let roughness = eval_scene_number(roughness_expr, time_norm, time_sec)?.clamp(0.0, 32.0);
    let copies = eval_scene_number(copies_expr, time_norm, time_sec)?
        .round()
        .clamp(1.0, 12.0) as u32;
    let texture_strength = eval_scene_number(texture_expr, time_norm, time_sec)?.clamp(0.0, 1.0);
    let bristles = eval_scene_number(bristles_expr, time_norm, time_sec)?
        .round()
        .clamp(0.0, 24.0) as u32;
    let pressure_auto = matches!(
        pressure_expr.trim().to_ascii_lowercase().as_str(),
        "auto" | "true" | "1" | "yes" | "on"
    );
    let pressure_min = eval_scene_number(pressure_min_expr, time_norm, time_sec)?.clamp(0.0, 1.0);
    let pressure_curve =
        eval_scene_number(pressure_curve_expr, time_norm, time_sec)?.clamp(0.05, 8.0);
    if texture == StrokeTexture::Solid {
        return Ok(StrokeParams {
            roughness,
            copies: copies.max(1),
            texture_strength,
            bristles,
            pressure_auto,
            pressure_min,
            pressure_curve,
        });
    }
    Ok(StrokeParams {
        roughness: if roughness <= 0.0001 {
            defaults.roughness
        } else {
            roughness
        },
        copies: if copies <= 1 { defaults.copies } else { copies },
        texture_strength: if texture_strength <= 0.0001 {
            defaults.texture_strength
        } else {
            texture_strength
        },
        bristles: if bristles == 0 {
            defaults.bristles
        } else {
            bristles
        },
        pressure_auto,
        pressure_min: if pressure_min >= 0.999 && pressure_auto {
            defaults.pressure_min
        } else {
            pressure_min
        },
        pressure_curve: if (pressure_curve - 1.0).abs() <= 0.0001 && pressure_auto {
            defaults.pressure_curve
        } else {
            pressure_curve
        },
    })
}

pub(crate) fn stroke_texture_copy_count(style: StrokeStyle) -> u32 {
    if style.texture == StrokeTexture::Solid || style.roughness <= 0.0001 {
        1
    } else {
        style.copies.clamp(1, 12)
    }
}

pub(crate) fn stroke_texture_variant(
    p0: Point2,
    p1: Point2,
    style: StrokeStyle,
    copy_ix: u32,
) -> (Point2, Point2, f32, f32) {
    if copy_ix == 0 || style.texture == StrokeTexture::Solid || style.roughness <= 0.0001 {
        return (p0, p1, 1.0, 1.0);
    }

    let dx = p1.x - p0.x;
    let dy = p1.y - p0.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len <= 0.0001 {
        return (p0, p1, 1.0, 1.0);
    }
    let tx = dx / len;
    let ty = dy / len;
    let nx = -ty;
    let ny = tx;
    let copy = copy_ix as f32;
    let rough = style.roughness * (0.75 + copy * 0.18);
    let seed = stroke_texture_seed(p0, p1, copy_ix);
    let n0 = stroke_hash_signed(seed + 11.7) * rough;
    let n1 = stroke_hash_signed(seed + 37.3) * rough;
    let t0 = stroke_hash_signed(seed + 71.9) * rough * 0.35;
    let t1 = stroke_hash_signed(seed + 103.1) * rough * 0.35;
    let start = Point2::new(p0.x + nx * n0 + tx * t0, p0.y + ny * n0 + ty * t0);
    let end = Point2::new(p1.x + nx * n1 + tx * t1, p1.y + ny * n1 + ty * t1);
    let (width_scale, opacity_scale) = match style.texture {
        StrokeTexture::Solid => (1.0, 1.0),
        StrokeTexture::Ink => (0.58, 0.34),
        StrokeTexture::Sketch => ((0.86 - copy * 0.07).max(0.42), 0.38),
        StrokeTexture::Rough => ((0.72 - copy * 0.05).max(0.35), 0.30),
        StrokeTexture::Pencil => ((0.46 - copy * 0.025).max(0.24), 0.24),
        StrokeTexture::Charcoal => ((0.38 - copy * 0.015).max(0.14), 0.18),
        StrokeTexture::Marker => (0.94, 0.72),
        StrokeTexture::Hairline => (0.75, 0.50),
    };
    (start, end, width_scale, opacity_scale)
}

pub(crate) fn stroke_texture_seed(p0: Point2, p1: Point2, copy_ix: u32) -> f32 {
    p0.x * 12.9898 + p0.y * 78.233 + p1.x * 37.719 + p1.y * 11.131 + copy_ix as f32 * 19.19
}

pub(crate) fn stroke_hash_signed(seed: f32) -> f32 {
    let raw = (seed.sin() * 43_758.547).fract();
    let unit = if raw < 0.0 { raw + 1.0 } else { raw };
    unit * 2.0 - 1.0
}

pub(crate) fn stroke_taper_pressure(t: f32, style: StrokeStyle) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let mut pressure = if style.width_profile_len >= 2 {
        let profile = &style.width_profile[..style.width_profile_len];
        let mut value = profile[0].1;
        for pair in profile.windows(2) {
            if t <= pair[1].0 {
                let span = (pair[1].0 - pair[0].0).max(0.00001);
                let local = ((t - pair[0].0) / span).clamp(0.0, 1.0);
                value = pair[0].1 + (pair[1].1 - pair[0].1) * local;
                break;
            }
            value = pair[1].1;
        }
        value
    } else {
        style.width_start + (style.width_end - style.width_start) * t
    };
    if style.taper_start > 0.0001 {
        pressure = pressure.min((t / style.taper_start).clamp(0.0, 1.0));
    }
    if style.taper_end > 0.0001 {
        pressure = pressure.min(((1.0 - t) / style.taper_end).clamp(0.0, 1.0));
    }
    if style.pressure_auto {
        let bell = (std::f32::consts::PI * t.clamp(0.0, 1.0)).sin().max(0.0);
        let shaped = bell.powf(style.pressure_curve.max(0.05));
        let auto_pressure = style.pressure_min + (1.0 - style.pressure_min) * shaped;
        pressure *= auto_pressure.clamp(0.0, 1.0);
    }
    pressure
}
