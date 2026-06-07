use crate::scene::model::{GradientDef, GradientStop};
use crate::scene::render::MotionLoomSceneRenderError;

use super::{GpuSceneMatteMode, Point2};

#[derive(Debug, Clone)]
pub(crate) enum ResolvedPaint {
    None,
    Solid([u8; 4]),
    Gradient(ResolvedGradient),
}

#[derive(Debug, Clone)]
pub(crate) enum ResolvedGradient {
    Linear {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        stops: Vec<ResolvedGradientStop>,
        units: GradientUnits,
    },
    Radial {
        cx: f32,
        cy: f32,
        r: f32,
        stops: Vec<ResolvedGradientStop>,
        units: GradientUnits,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ResolvedGradientStop {
    pub(crate) offset: f32,
    pub(crate) color: [u8; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GradientUnits {
    ObjectBoundingBox,
    UserSpace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SceneBlendMode {
    Normal,
    Multiply,
    Screen,
    Add,
}

impl SceneBlendMode {
    pub(crate) fn gpu_code(self) -> f32 {
        match self {
            Self::Normal => 0.0,
            Self::Multiply => 1.0,
            Self::Screen => 2.0,
            Self::Add => 3.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PaintBounds {
    pub(crate) min_x: f32,
    pub(crate) min_y: f32,
    pub(crate) max_x: f32,
    pub(crate) max_y: f32,
}

impl PaintBounds {
    pub(crate) const fn new(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> Self {
        Self {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }
}

pub(crate) fn resolve_gradient_paint(
    source_value: &str,
    gradient: &GradientDef,
) -> Result<ResolvedGradient, MotionLoomSceneRenderError> {
    match gradient {
        GradientDef::Linear(linear) => Ok(ResolvedGradient::Linear {
            x1: parse_gradient_number(&linear.x1, 0.0),
            y1: parse_gradient_number(&linear.y1, 0.0),
            x2: parse_gradient_number(&linear.x2, 1.0),
            y2: parse_gradient_number(&linear.y2, 0.0),
            stops: resolve_gradient_stops(source_value, &linear.stops)?,
            units: parse_gradient_units(&linear.units),
        }),
        GradientDef::Radial(radial) => Ok(ResolvedGradient::Radial {
            cx: parse_gradient_number(&radial.cx, 0.5),
            cy: parse_gradient_number(&radial.cy, 0.5),
            r: parse_gradient_number(&radial.r, 0.5).max(0.0001),
            stops: resolve_gradient_stops(source_value, &radial.stops)?,
            units: parse_gradient_units(&radial.units),
        }),
    }
}

fn resolve_gradient_stops(
    source_value: &str,
    stops: &[GradientStop],
) -> Result<Vec<ResolvedGradientStop>, MotionLoomSceneRenderError> {
    stops
        .iter()
        .map(|stop| {
            Ok(ResolvedGradientStop {
                offset: stop.offset.clamp(0.0, 1.0),
                color: parse_color(&stop.color).map_err(|err| {
                    MotionLoomSceneRenderError::InvalidPaint {
                        value: source_value.to_string(),
                        message: err.to_string(),
                    }
                })?,
            })
        })
        .collect()
}

fn parse_gradient_units(value: &str) -> GradientUnits {
    match value.trim().to_ascii_lowercase().as_str() {
        "userspace" | "user-space" | "userspaceonuse" | "user-space-on-use" => {
            GradientUnits::UserSpace
        }
        _ => GradientUnits::ObjectBoundingBox,
    }
}

fn parse_gradient_number(value: &str, default: f32) -> f32 {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        return percent
            .trim()
            .parse::<f32>()
            .map(|v| v / 100.0)
            .unwrap_or(default);
    }
    value.parse::<f32>().unwrap_or(default)
}

pub(crate) fn gradient_ref_id(value: &str) -> Option<&str> {
    let value = value.trim();
    let rest = value.strip_prefix("url(#")?;
    rest.strip_suffix(')')
}

pub(crate) fn is_gpu_native_blend(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().replace('_', "-").as_str(),
        "" | "normal"
            | "over"
            | "source-over"
            | "multiply"
            | "screen"
            | "add"
            | "plus"
            | "linear-dodge"
    )
}

pub(crate) fn parse_scene_blend(value: &str) -> Result<SceneBlendMode, MotionLoomSceneRenderError> {
    match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "" | "normal" | "over" | "source-over" => Ok(SceneBlendMode::Normal),
        "multiply" => Ok(SceneBlendMode::Multiply),
        "screen" => Ok(SceneBlendMode::Screen),
        "add" | "plus" | "linear-dodge" => Ok(SceneBlendMode::Add),
        other => Err(MotionLoomSceneRenderError::InvalidPaint {
            value: value.to_string(),
            message: format!("unsupported blend mode: {other}"),
        }),
    }
}

pub(crate) fn gpu_matte_mode(value: &str) -> GpuSceneMatteMode {
    match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "luma" | "luminance" => GpuSceneMatteMode::Luma,
        _ => GpuSceneMatteMode::Alpha,
    }
}

pub(crate) fn scene_mask_mode_inverts(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().replace('_', "-").as_str(),
        "inverse" | "invert" | "inverted"
    )
}

pub(crate) fn parse_paint(value: &str) -> Result<Option<[u8; 4]>, MotionLoomSceneRenderError> {
    if is_none_paint(value) {
        return Ok(None);
    }
    let color = parse_color(value)?;
    if color[3] == 0 {
        return Ok(None);
    }
    Ok(Some(color))
}

pub(crate) fn is_none_paint(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.is_empty() || value == "none" || value == "transparent"
}

pub(crate) fn parse_color(value: &str) -> Result<[u8; 4], MotionLoomSceneRenderError> {
    let trimmed = value.trim();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        return parse_bgra_array_color(trimmed, value);
    }

    let lower = trimmed.to_ascii_lowercase();
    match lower.as_str() {
        "black" => return Ok([0, 0, 0, 255]),
        "white" => return Ok([255, 255, 255, 255]),
        "red" => return Ok([255, 0, 0, 255]),
        "green" => return Ok([0, 255, 0, 255]),
        "blue" => return Ok([0, 0, 255, 255]),
        "transparent" => return Ok([0, 0, 0, 0]),
        _ => {}
    }

    let hex = lower
        .strip_prefix('#')
        .or_else(|| lower.strip_prefix("0x"))
        .unwrap_or(lower.as_str());
    let expanded;
    let hex = if hex.len() == 3 {
        expanded = hex.chars().flat_map(|ch| [ch, ch]).collect::<String>();
        expanded.as_str()
    } else {
        hex
    };
    if hex.len() != 6 && hex.len() != 8 {
        return Err(MotionLoomSceneRenderError::InvalidColor {
            value: value.to_string(),
        });
    }
    let r = parse_hex_byte(hex, 0, value)?;
    let g = parse_hex_byte(hex, 2, value)?;
    let b = parse_hex_byte(hex, 4, value)?;
    let a = if hex.len() == 8 {
        parse_hex_byte(hex, 6, value)?
    } else {
        255
    };
    Ok([r, g, b, a])
}

fn parse_bgra_array_color(
    value: &str,
    original: &str,
) -> Result<[u8; 4], MotionLoomSceneRenderError> {
    let inner = value
        .strip_prefix('[')
        .and_then(|text| text.strip_suffix(']'))
        .ok_or_else(|| MotionLoomSceneRenderError::InvalidColor {
            value: original.to_string(),
        })?;
    let parts = inner
        .split(',')
        .map(str::trim)
        .map(|part| part.parse::<f32>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| MotionLoomSceneRenderError::InvalidColor {
            value: original.to_string(),
        })?;
    if parts.len() != 4 || parts.iter().any(|component| !component.is_finite()) {
        return Err(MotionLoomSceneRenderError::InvalidColor {
            value: original.to_string(),
        });
    }

    let uses_byte_range = parts.iter().any(|component| *component > 1.0);
    let to_byte = |component: f32| {
        let scaled = if uses_byte_range {
            component
        } else {
            component * 255.0
        };
        scaled.round().clamp(0.0, 255.0) as u8
    };

    let b = to_byte(parts[0]);
    let g = to_byte(parts[1]);
    let r = to_byte(parts[2]);
    let a = to_byte(parts[3]);
    Ok([r, g, b, a])
}

fn parse_hex_byte(
    hex: &str,
    start: usize,
    original: &str,
) -> Result<u8, MotionLoomSceneRenderError> {
    u8::from_str_radix(&hex[start..start + 2], 16).map_err(|_| {
        MotionLoomSceneRenderError::InvalidColor {
            value: original.to_string(),
        }
    })
}

pub(crate) fn sample_paint(
    paint: &ResolvedPaint,
    point: Point2,
    bounds: PaintBounds,
    opacity: f32,
) -> Option<[u8; 4]> {
    let mut color = match paint {
        ResolvedPaint::None => return None,
        ResolvedPaint::Solid(color) => *color,
        ResolvedPaint::Gradient(gradient) => sample_gradient(gradient, point, bounds),
    };
    color[3] = ((color[3] as f32) * opacity).round().clamp(0.0, 255.0) as u8;
    (color[3] > 0).then_some(color)
}

fn sample_gradient(gradient: &ResolvedGradient, point: Point2, bounds: PaintBounds) -> [u8; 4] {
    match gradient {
        ResolvedGradient::Linear {
            x1,
            y1,
            x2,
            y2,
            stops,
            units,
        } => {
            let (px, py, sx, sy, ex, ey) = match units {
                GradientUnits::ObjectBoundingBox => {
                    let w = (bounds.max_x - bounds.min_x).max(0.0001);
                    let h = (bounds.max_y - bounds.min_y).max(0.0001);
                    (
                        (point.x - bounds.min_x) / w,
                        (point.y - bounds.min_y) / h,
                        *x1,
                        *y1,
                        *x2,
                        *y2,
                    )
                }
                GradientUnits::UserSpace => (point.x, point.y, *x1, *y1, *x2, *y2),
            };
            let dx = ex - sx;
            let dy = ey - sy;
            let len2 = (dx * dx + dy * dy).max(0.000001);
            let t = (((px - sx) * dx + (py - sy) * dy) / len2).clamp(0.0, 1.0);
            sample_gradient_stops(stops, t)
        }
        ResolvedGradient::Radial {
            cx,
            cy,
            r,
            stops,
            units,
        } => {
            let t = match units {
                GradientUnits::ObjectBoundingBox => {
                    let w = (bounds.max_x - bounds.min_x).max(0.0001);
                    let h = (bounds.max_y - bounds.min_y).max(0.0001);
                    let px = (point.x - bounds.min_x) / w;
                    let py = (point.y - bounds.min_y) / h;
                    let aspect = if w > h { h / w } else { 1.0 };
                    let dx = (px - *cx) / aspect.max(0.0001);
                    let dy = py - *cy;
                    ((dx * dx + dy * dy).sqrt() / *r).clamp(0.0, 1.0)
                }
                GradientUnits::UserSpace => {
                    let dx = point.x - *cx;
                    let dy = point.y - *cy;
                    ((dx * dx + dy * dy).sqrt() / *r).clamp(0.0, 1.0)
                }
            };
            sample_gradient_stops(stops, t)
        }
    }
}

fn sample_gradient_stops(stops: &[ResolvedGradientStop], t: f32) -> [u8; 4] {
    let Some(first) = stops.first() else {
        return [0, 0, 0, 0];
    };
    if t <= first.offset {
        return first.color;
    }
    for pair in stops.windows(2) {
        let a = pair[0];
        let b = pair[1];
        if t <= b.offset {
            let span = (b.offset - a.offset).max(0.000001);
            let local_t = ((t - a.offset) / span).clamp(0.0, 1.0);
            return lerp_color(a.color, b.color, local_t);
        }
    }
    stops.last().map(|stop| stop.color).unwrap_or(first.color)
}

fn lerp_color(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    [
        (a[0] as f32 + (b[0] as f32 - a[0] as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8,
        (a[1] as f32 + (b[1] as f32 - a[1] as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8,
        (a[2] as f32 + (b[2] as f32 - a[2] as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8,
        (a[3] as f32 + (b[3] as f32 - a[3] as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8,
    ]
}
