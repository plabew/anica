// =========================================
// =========================================
// crates/motionloom/src/runtime.rs

use crate::dsl::{
    GraphApplyScope, GraphScript, PassNode, PassTransitionEasing, PassTransitionMode,
};
use crate::effect_kernel_map::resolve_pass_kernel;
pub use crate::error::RuntimeCompileError;
use crate::process_catalog::is_known_process_kernel;
use exmex::Express;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum BlurSharpenMode {
    #[default]
    Gaussian5tapH,
    Gaussian5tapV,
    Box,
    Unsharp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransitionCoreEffect {
    FadeIn,
    FadeOut,
    Dip,
    Dissolve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CurveEase {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CurvePoint {
    t_sec: f32,
    value: f32,
    ease: CurveEase,
}

#[derive(Debug, Clone)]
enum RuntimePass {
    InvertMix {
        mix_expr: String,
    },
    BlurSharpenDetailGaussian {
        effect: BlurSharpenMode,
        sigma_expr: String,
    },
    TransitionCore {
        mode: PassTransitionMode,
        effect: TransitionCoreEffect,
        easing: PassTransitionEasing,
        progress_expr: Option<String>,
        opacity_expr: Option<String>,
        start_sec: Option<f32>,
        duration_sec: f32,
    },
    CompositeOpacity {
        opacity_expr: String,
    },
    ColorCoreLut {
        mix_expr: String,
    },
    ColorCoreHslaOverlay {
        hue_expr: String,
        saturation_expr: String,
        lightness_expr: String,
        alpha_expr: String,
    },
    GpuOnlyKernel {
        kernel: String,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RuntimeFrameOutput {
    pub frame: u32,
    pub time_sec: f32,
    pub time_norm: f32,
    pub invert_mix: f32,
    pub blur_sharpen_mode: Option<BlurSharpenMode>,
    pub layer_blur_sigma: Option<f32>,
    pub layer_sharpen_sigma: Option<f32>,
    pub transition_dissolve_mix: Option<f32>,
    pub layer_transition_opacity: Option<f32>,
    pub layer_lut_mix: Option<f32>,
    pub layer_hsla_hue: Option<f32>,
    pub layer_hsla_saturation: Option<f32>,
    pub layer_hsla_lightness: Option<f32>,
    pub layer_hsla_alpha: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct RuntimeProgram {
    graph: GraphScript,
    passes: Vec<RuntimePass>,
    unsupported_kernels: Vec<String>,
}

pub fn compile_runtime_program(graph: GraphScript) -> Result<RuntimeProgram, RuntimeCompileError> {
    RuntimeProgram::new(graph)
}

impl RuntimeProgram {
    pub fn new(graph: GraphScript) -> Result<Self, RuntimeCompileError> {
        let mut passes = Vec::<RuntimePass>::new();
        let mut unsupported = Vec::<String>::new();

        for pass in &graph.passes {
            let resolved_kernel = resolve_kernel_for_pass(pass)?;
            match resolved_kernel.as_str() {
                // v1 supported runtime kernel: animatable invert mix.
                "invert_mix.wgsl" => {
                    let mix_expr = pass_param(pass, "mix")
                        .map(normalize_param_expr)
                        .unwrap_or_else(|| "1.0".to_string());
                    validate_expr(&mix_expr).map_err(|e| RuntimeCompileError {
                        message: format!("pass {} invalid mix expression: {}", pass.id, e),
                    })?;
                    passes.push(RuntimePass::InvertMix { mix_expr });
                }
                "blur_sharpen_detail_gaussian.wgsl"
                | "blur_sharpen_detail_gaussian_5tap.wgsl"
                | "effect_for_testing_run.wgsl" => {
                    let effect = parse_blur_sharpen_effect(&pass.effect).map_err(|e| {
                        RuntimeCompileError {
                            message: format!("pass {} invalid effect: {}", pass.id, e),
                        }
                    })?;
                    let sigma_expr = pass_param(pass, "sigma")
                        .map(normalize_param_expr)
                        .unwrap_or_else(|| "2.0".to_string());
                    validate_expr(&sigma_expr).map_err(|e| RuntimeCompileError {
                        message: format!("pass {} invalid sigma expression: {}", pass.id, e),
                    })?;
                    passes.push(RuntimePass::BlurSharpenDetailGaussian { effect, sigma_expr });
                }
                "transition_core.wgsl" => {
                    let mode = pass.transition.clone().unwrap_or(PassTransitionMode::Auto);
                    let effect = parse_transition_core_effect(&pass.effect).map_err(|e| {
                        RuntimeCompileError {
                            message: format!("pass {} invalid effect: {}", pass.id, e),
                        }
                    })?;
                    let easing = pass_param(pass, "easing")
                        .map(normalize_param_expr)
                        .as_deref()
                        .map(parse_transition_easing_param)
                        .transpose()
                        .map_err(|e| RuntimeCompileError {
                            message: format!("pass {} invalid easing: {}", pass.id, e),
                        })?
                        .or_else(|| pass.transition_easing.clone())
                        .unwrap_or(PassTransitionEasing::Linear);

                    let progress_expr = if effect == TransitionCoreEffect::Dissolve {
                        pass_param(pass, "progress")
                            .or_else(|| pass_param(pass, "mix"))
                            .map(normalize_param_expr)
                    } else {
                        None
                    };
                    if let Some(expr) = progress_expr.as_deref() {
                        validate_expr(expr).map_err(|e| RuntimeCompileError {
                            message: format!("pass {} invalid progress expression: {}", pass.id, e),
                        })?;
                    }

                    let start_sec = pass_param_f32(pass, &["startSec", "start_sec"]);
                    let duration_sec =
                        pass_param_f32(pass, &["durationSec", "duration_sec"]).unwrap_or(0.6);
                    let opacity_expr = pass_param(pass, "opacity").map(normalize_param_expr);
                    if let Some(expr) = opacity_expr.as_deref() {
                        validate_expr(expr).map_err(|e| RuntimeCompileError {
                            message: format!("pass {} invalid opacity expression: {}", pass.id, e),
                        })?;
                    }
                    if !duration_sec.is_finite() || duration_sec <= 0.0 {
                        return Err(RuntimeCompileError {
                            message: format!(
                                "pass {} invalid durationSec: expected > 0, got {}",
                                pass.id, duration_sec
                            ),
                        });
                    }

                    passes.push(RuntimePass::TransitionCore {
                        mode,
                        effect,
                        easing,
                        progress_expr,
                        opacity_expr,
                        start_sec,
                        duration_sec,
                    });
                }
                "transition_fade_in_out.wgsl" | "transition_dissolve.wgsl" => {
                    return Err(RuntimeCompileError {
                        message: format!(
                            "pass {} uses removed kernel {}; use transition_core.wgsl + effect instead",
                            pass.id, resolved_kernel
                        ),
                    });
                }
                "composite_core.wgsl" => {
                    let normalized_effect = pass
                        .effect
                        .trim()
                        .trim_matches('"')
                        .trim()
                        .to_ascii_lowercase();
                    if normalized_effect != "opacity" && normalized_effect != "composite.opacity" {
                        passes.push(RuntimePass::GpuOnlyKernel {
                            kernel: resolved_kernel.clone(),
                        });
                        continue;
                    }
                    let opacity_expr = pass_param(pass, "opacity")
                        .map(normalize_param_expr)
                        .unwrap_or_else(|| "1.0".to_string());
                    validate_expr(&opacity_expr).map_err(|e| RuntimeCompileError {
                        message: format!("pass {} invalid opacity expression: {}", pass.id, e),
                    })?;
                    passes.push(RuntimePass::CompositeOpacity { opacity_expr });
                }
                "color_core.wgsl" => {
                    let normalized_effect = pass
                        .effect
                        .trim()
                        .trim_matches('"')
                        .trim()
                        .to_ascii_lowercase();
                    if normalized_effect == "lut" || normalized_effect == "color_tone.lut" {
                        let mix_expr = pass_param(pass, "mix")
                            .or_else(|| pass_param(pass, "lutMix"))
                            .or_else(|| pass_param(pass, "lut_mix"))
                            .map(normalize_param_expr)
                            .unwrap_or_else(|| "1.0".to_string());
                        validate_expr(&mix_expr).map_err(|e| RuntimeCompileError {
                            message: format!("pass {} invalid LUT mix expression: {}", pass.id, e),
                        })?;
                        passes.push(RuntimePass::ColorCoreLut { mix_expr });
                        continue;
                    }
                    if normalized_effect == "hsla_overlay"
                        || normalized_effect == "hsla"
                        || normalized_effect == "tint_overlay"
                        || normalized_effect == "color_tone.hsla_overlay"
                    {
                        let hue_expr = pass_param(pass, "hue")
                            .or_else(|| pass_param(pass, "h"))
                            .map(normalize_param_expr)
                            .unwrap_or_else(|| "0.0".to_string());
                        let saturation_expr = pass_param(pass, "saturation")
                            .or_else(|| pass_param(pass, "sat"))
                            .or_else(|| pass_param(pass, "s"))
                            .map(normalize_param_expr)
                            .unwrap_or_else(|| "0.0".to_string());
                        let lightness_expr = pass_param(pass, "lightness")
                            .or_else(|| pass_param(pass, "lum"))
                            .or_else(|| pass_param(pass, "l"))
                            .map(normalize_param_expr)
                            .unwrap_or_else(|| "0.0".to_string());
                        let alpha_expr = pass_param(pass, "alpha")
                            .or_else(|| pass_param(pass, "a"))
                            .map(normalize_param_expr)
                            .unwrap_or_else(|| "0.0".to_string());

                        validate_expr(&hue_expr).map_err(|e| RuntimeCompileError {
                            message: format!("pass {} invalid HSLA hue expression: {}", pass.id, e),
                        })?;
                        validate_expr(&saturation_expr).map_err(|e| RuntimeCompileError {
                            message: format!(
                                "pass {} invalid HSLA saturation expression: {}",
                                pass.id, e
                            ),
                        })?;
                        validate_expr(&lightness_expr).map_err(|e| RuntimeCompileError {
                            message: format!(
                                "pass {} invalid HSLA lightness expression: {}",
                                pass.id, e
                            ),
                        })?;
                        validate_expr(&alpha_expr).map_err(|e| RuntimeCompileError {
                            message: format!(
                                "pass {} invalid HSLA alpha expression: {}",
                                pass.id, e
                            ),
                        })?;

                        passes.push(RuntimePass::ColorCoreHslaOverlay {
                            hue_expr,
                            saturation_expr,
                            lightness_expr,
                            alpha_expr,
                        });
                        continue;
                    }
                    passes.push(RuntimePass::GpuOnlyKernel {
                        kernel: resolved_kernel.clone(),
                    });
                    continue;
                }
                other if is_known_process_kernel(other) => {
                    passes.push(RuntimePass::GpuOnlyKernel {
                        kernel: other.to_string(),
                    });
                }
                other if other.ends_with(".wgsl") => {
                    // Explicit kernel path is treated as custom GPU kernel.
                    passes.push(RuntimePass::GpuOnlyKernel {
                        kernel: other.to_string(),
                    });
                }
                other => {
                    unsupported.push(format!("{} ({})", pass.id, other));
                }
            }
        }

        Ok(Self {
            graph,
            passes,
            unsupported_kernels: unsupported,
        })
    }

    pub fn graph(&self) -> &GraphScript {
        &self.graph
    }

    pub fn total_pass_count(&self) -> usize {
        self.graph.passes.len()
    }

    pub fn supported_pass_count(&self) -> usize {
        self.passes.len()
    }

    pub fn unsupported_kernels(&self) -> &[String] {
        &self.unsupported_kernels
    }

    pub fn evaluate_frame(&self, frame: u32) -> RuntimeFrameOutput {
        let fps = self.graph.fps.max(1.0);
        let time_sec = frame as f32 / fps;
        self.evaluate_at_time_sec(time_sec, None)
    }

    pub fn evaluate_at_time_sec(
        &self,
        time_sec: f32,
        clip_duration_sec: Option<f32>,
    ) -> RuntimeFrameOutput {
        let fps = self.graph.fps.max(1.0);
        let graph_duration_sec = (self.graph.duration_ms as f32 / 1000.0).max(0.0001);
        let time_sec = time_sec.max(0.0);
        let time_norm = (time_sec / graph_duration_sec).clamp(0.0, 1.0);
        let mut out = RuntimeFrameOutput {
            frame: (time_sec * fps).floor().max(0.0) as u32,
            time_sec,
            time_norm,
            invert_mix: 0.0,
            blur_sharpen_mode: None,
            layer_blur_sigma: None,
            layer_sharpen_sigma: None,
            transition_dissolve_mix: None,
            layer_transition_opacity: None,
            layer_lut_mix: None,
            layer_hsla_hue: None,
            layer_hsla_saturation: None,
            layer_hsla_lightness: None,
            layer_hsla_alpha: None,
        };
        // apply="graph" only gates when duration is explicitly provided on <Graph>.
        if self.graph.apply == GraphApplyScope::Graph
            && self.graph.duration_explicit
            && time_sec >= graph_duration_sec
        {
            return out;
        }

        for pass in &self.passes {
            match pass {
                RuntimePass::InvertMix { mix_expr } => {
                    if let Ok(value) = eval_expr(mix_expr, time_norm, time_sec) {
                        out.invert_mix = value.clamp(0.0, 1.0);
                    }
                }
                RuntimePass::BlurSharpenDetailGaussian { effect, sigma_expr } => {
                    if let Ok(value) = eval_expr(sigma_expr, time_norm, time_sec) {
                        out.blur_sharpen_mode = Some(*effect);
                        let amount = value.clamp(0.0, 64.0);
                        match effect {
                            BlurSharpenMode::Unsharp => {
                                out.layer_sharpen_sigma = Some(amount);
                            }
                            _ => {
                                out.layer_blur_sigma = Some(amount);
                            }
                        }
                    }
                }
                RuntimePass::TransitionCore {
                    mode,
                    effect,
                    easing,
                    progress_expr,
                    opacity_expr,
                    start_sec,
                    duration_sec: pass_duration_sec,
                } => {
                    if *mode == PassTransitionMode::Off {
                        continue;
                    }

                    let progress = if let Some(expr) = progress_expr.as_deref() {
                        match eval_expr(expr, time_norm, time_sec) {
                            Ok(v) => v.clamp(0.0, 1.0),
                            Err(_) => continue,
                        }
                    } else {
                        let clip_span_sec = clip_duration_sec
                            .filter(|v| *v > 0.0)
                            .unwrap_or(graph_duration_sec);
                        let start_sec = match (effect, start_sec) {
                            (_, Some(v)) => *v,
                            (TransitionCoreEffect::FadeOut, None) => {
                                (clip_span_sec - *pass_duration_sec).max(0.0)
                            }
                            _ => 0.0,
                        };
                        ((time_sec - start_sec) / *pass_duration_sec).clamp(0.0, 1.0)
                    };
                    let progress = apply_transition_easing(progress, easing.clone());
                    let curve_opacity = opacity_expr
                        .as_deref()
                        .and_then(|expr| eval_expr(expr, time_norm, time_sec).ok())
                        .map(|v| v.clamp(0.0, 1.0));

                    match effect {
                        TransitionCoreEffect::FadeIn => {
                            let opacity = curve_opacity.unwrap_or(progress.clamp(0.0, 1.0));
                            out.layer_transition_opacity = Some(
                                out.layer_transition_opacity
                                    .map_or(opacity, |prev| prev.min(opacity)),
                            );
                        }
                        TransitionCoreEffect::FadeOut => {
                            let opacity = curve_opacity.unwrap_or((1.0 - progress).clamp(0.0, 1.0));
                            out.layer_transition_opacity = Some(
                                out.layer_transition_opacity
                                    .map_or(opacity, |prev| prev.min(opacity)),
                            );
                        }
                        TransitionCoreEffect::Dip => {
                            let opacity =
                                curve_opacity.unwrap_or(v1_dissolve_opacity_from_mix(progress));
                            out.layer_transition_opacity = Some(
                                out.layer_transition_opacity
                                    .map_or(opacity, |prev| prev.min(opacity)),
                            );
                        }
                        TransitionCoreEffect::Dissolve => {
                            let mix = progress.clamp(0.0, 1.0);
                            out.transition_dissolve_mix = Some(
                                out.transition_dissolve_mix
                                    .map_or(mix, |prev| prev.max(mix)),
                            );
                        }
                    }
                }
                RuntimePass::GpuOnlyKernel { kernel } => {
                    let _ = kernel;
                }
                RuntimePass::CompositeOpacity { opacity_expr } => {
                    if let Ok(value) = eval_expr(opacity_expr, time_norm, time_sec) {
                        let opacity = value.clamp(0.0, 1.0);
                        out.layer_transition_opacity = Some(
                            out.layer_transition_opacity
                                .map_or(opacity, |prev| prev.min(opacity)),
                        );
                    }
                }
                RuntimePass::ColorCoreLut { mix_expr } => {
                    if let Ok(value) = eval_expr(mix_expr, time_norm, time_sec) {
                        out.layer_lut_mix = Some(value.clamp(0.0, 1.0));
                    }
                }
                RuntimePass::ColorCoreHslaOverlay {
                    hue_expr,
                    saturation_expr,
                    lightness_expr,
                    alpha_expr,
                } => {
                    let Ok(hue) = eval_expr(hue_expr, time_norm, time_sec) else {
                        continue;
                    };
                    let Ok(saturation) = eval_expr(saturation_expr, time_norm, time_sec) else {
                        continue;
                    };
                    let Ok(lightness) = eval_expr(lightness_expr, time_norm, time_sec) else {
                        continue;
                    };
                    let Ok(alpha) = eval_expr(alpha_expr, time_norm, time_sec) else {
                        continue;
                    };
                    out.layer_hsla_hue = Some(hue.rem_euclid(360.0));
                    out.layer_hsla_saturation = Some(saturation.clamp(0.0, 1.0));
                    out.layer_hsla_lightness = Some(lightness.clamp(0.0, 1.0));
                    out.layer_hsla_alpha = Some(alpha.clamp(0.0, 1.0));
                }
            }
        }

        out
    }

    pub fn summary(&self) -> String {
        format!(
            "Runtime ready: total_passes={}, supported_passes={}, unsupported_passes={}",
            self.total_pass_count(),
            self.supported_pass_count(),
            self.unsupported_kernels.len()
        )
    }
}

fn pass_param<'a>(pass: &'a PassNode, key: &str) -> Option<&'a str> {
    pass.params
        .iter()
        .find(|p| p.key == key)
        .map(|p| p.value.as_str())
}

fn resolve_kernel_for_pass(pass: &PassNode) -> Result<String, RuntimeCompileError> {
    resolve_pass_kernel(pass).ok_or_else(|| RuntimeCompileError {
        message: format!(
            "pass {} missing kernel and no default mapping for effect '{}'",
            pass.id, pass.effect
        ),
    })
}

fn pass_param_f32(pass: &PassNode, keys: &[&str]) -> Option<f32> {
    for key in keys {
        if let Some(v) = pass_param(pass, key)
            && let Ok(parsed) = normalize_param_expr(v).parse::<f32>()
        {
            return Some(parsed);
        }
    }
    None
}

fn normalize_param_expr(value: &str) -> String {
    value.trim().trim_matches('"').trim().to_string()
}

fn validate_expr(expr: &str) -> Result<(), String> {
    if is_curve_expr(expr) {
        parse_curve_points(expr).map(|_| ())?;
        return Ok(());
    }
    let replaced = replace_time_vars(expr, 0.5, 1.0);
    exmex::FlatEx::<f64>::parse(&replaced)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn eval_expr(expr: &str, time_norm: f32, time_sec: f32) -> Result<f32, String> {
    if is_curve_expr(expr) {
        return eval_curve_points(expr, time_sec);
    }
    let replaced = replace_time_vars(expr, time_norm, time_sec);
    let parsed = exmex::FlatEx::<f64>::parse(&replaced).map_err(|e| e.to_string())?;
    let val = parsed.eval(&[]).map_err(|e| e.to_string())?;
    Ok(val as f32)
}

fn replace_time_vars(expr: &str, time_norm: f32, time_sec: f32) -> String {
    expr.replace("$time.norm", &format!("{time_norm:.9}"))
        .replace("$time.sec", &format!("{time_sec:.9}"))
}

fn parse_blur_sharpen_effect(raw: &str) -> Result<BlurSharpenMode, String> {
    let normalized = raw.trim().trim_matches('"').trim().to_ascii_lowercase();
    let normalized = normalized.replace('-', "_");
    match normalized.as_str() {
        "gaussian_blur" => Ok(BlurSharpenMode::Gaussian5tapH),
        "gaussian_5tap_h" => Ok(BlurSharpenMode::Gaussian5tapH),
        "gaussian_5tap_v" => Ok(BlurSharpenMode::Gaussian5tapV),
        "box" => Ok(BlurSharpenMode::Box),
        "sharpen" => Ok(BlurSharpenMode::Unsharp),
        "unsharp" => Ok(BlurSharpenMode::Unsharp),
        other => Err(format!(
            "expected gaussian_blur | gaussian_5tap_h | gaussian_5tap_v | sharpen | unsharp | box, got '{}'",
            other
        )),
    }
}

fn parse_transition_core_effect(raw: &str) -> Result<TransitionCoreEffect, String> {
    let normalized = raw.trim().trim_matches('"').trim().to_ascii_lowercase();
    let normalized = normalized.replace('-', "_");
    match normalized.as_str() {
        "fade_in" => Ok(TransitionCoreEffect::FadeIn),
        "fade_out" => Ok(TransitionCoreEffect::FadeOut),
        "dip" | "dip_to_black" => Ok(TransitionCoreEffect::Dip),
        "dissolve" => Ok(TransitionCoreEffect::Dissolve),
        other => Err(format!(
            "expected fade_in | fade_out | dip | dissolve, got '{}'",
            other
        )),
    }
}

fn parse_transition_easing_param(raw: &str) -> Result<PassTransitionEasing, String> {
    let normalized = raw.trim().trim_matches('"').trim().to_ascii_lowercase();
    match normalized.as_str() {
        "linear" => Ok(PassTransitionEasing::Linear),
        "ease-in" | "ease_in" => Ok(PassTransitionEasing::EaseIn),
        "ease-out" | "ease_out" => Ok(PassTransitionEasing::EaseOut),
        "ease-in-out" | "ease_in_out" => Ok(PassTransitionEasing::EaseInOut),
        other => Err(format!(
            "expected linear | ease-in | ease-out | ease-in-out, got '{}'",
            other
        )),
    }
}

fn v1_dissolve_opacity_from_mix(mix: f32) -> f32 {
    ((mix * 2.0) - 1.0).abs().clamp(0.0, 1.0)
}

fn apply_transition_easing(t: f32, easing: PassTransitionEasing) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match easing {
        PassTransitionEasing::Linear => t,
        PassTransitionEasing::EaseIn => t * t,
        PassTransitionEasing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
        PassTransitionEasing::EaseInOut => {
            if t < 0.5 {
                2.0 * t * t
            } else {
                1.0 - ((-2.0 * t + 2.0).powi(2) / 2.0)
            }
        }
    }
}

fn is_curve_expr(expr: &str) -> bool {
    let trimmed = expr.trim();
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("curve(") && trimmed.ends_with(')')
}

fn eval_curve_points(expr: &str, time_sec: f32) -> Result<f32, String> {
    let points = parse_curve_points(expr)?;
    if points.len() == 1 {
        return Ok(points[0].value);
    }
    let x = time_sec.max(0.0);
    if x <= points[0].t_sec {
        return Ok(points[0].value);
    }
    if x >= points[points.len() - 1].t_sec {
        return Ok(points[points.len() - 1].value);
    }
    for idx in 0..(points.len() - 1) {
        let a = points[idx];
        let b = points[idx + 1];
        if x >= a.t_sec && x <= b.t_sec {
            let span = (b.t_sec - a.t_sec).max(0.000_001);
            let u = ((x - a.t_sec) / span).clamp(0.0, 1.0);
            let eased = apply_curve_ease(u, a.ease);
            return Ok(a.value + (b.value - a.value) * eased);
        }
    }
    Ok(points[points.len() - 1].value)
}

fn parse_curve_points(expr: &str) -> Result<Vec<CurvePoint>, String> {
    let trimmed = expr.trim();
    let Some(open_ix) = trimmed.find('(') else {
        return Err("curve expression missing '('.".to_string());
    };
    if !trimmed.ends_with(')') {
        return Err("curve expression missing ')'.".to_string());
    }
    let inner_owned = trimmed[open_ix + 1..trimmed.len() - 1]
        .trim()
        .replace("\\\"", "\"")
        .replace("\\'", "'");
    let mut inner = inner_owned.trim();
    if (inner.starts_with('"') && inner.ends_with('"'))
        || (inner.starts_with('\'') && inner.ends_with('\''))
    {
        inner = inner[1..inner.len() - 1].trim();
    }
    if inner.is_empty() {
        return Err("curve() requires at least one point.".to_string());
    }

    let mut points = Vec::<CurvePoint>::new();
    for raw_point in inner.split(',') {
        let token = raw_point.trim();
        if token.is_empty() {
            continue;
        }
        let parts: Vec<&str> = token.split(':').map(str::trim).collect();
        if parts.len() < 2 || parts.len() > 3 {
            return Err(format!(
                "invalid curve point '{}'; expected t:value[:ease]",
                token
            ));
        }
        let t_sec = parts[0]
            .parse::<f32>()
            .map_err(|_| format!("invalid curve time '{}'", parts[0]))?;
        let value = parts[1]
            .parse::<f32>()
            .map_err(|_| format!("invalid curve value '{}'", parts[1]))?;
        if !t_sec.is_finite() || !value.is_finite() {
            return Err(format!("non-finite curve point '{}'", token));
        }
        let ease = if parts.len() == 3 {
            parse_curve_ease(parts[2])?
        } else {
            CurveEase::Linear
        };
        points.push(CurvePoint {
            t_sec: t_sec.max(0.0),
            value,
            ease,
        });
    }

    if points.is_empty() {
        return Err("curve() requires at least one valid point.".to_string());
    }
    points.sort_by(|a, b| a.t_sec.total_cmp(&b.t_sec));
    Ok(points)
}

fn parse_curve_ease(raw: &str) -> Result<CurveEase, String> {
    let normalized = raw
        .trim()
        .trim_matches('"')
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_");
    match normalized.as_str() {
        "linear" => Ok(CurveEase::Linear),
        "ease_in" => Ok(CurveEase::EaseIn),
        "ease_out" => Ok(CurveEase::EaseOut),
        "ease_in_out" => Ok(CurveEase::EaseInOut),
        other => Err(format!(
            "invalid curve easing '{}'; expected linear | ease_in | ease_out | ease_in_out",
            other
        )),
    }
}

fn apply_curve_ease(t: f32, easing: CurveEase) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match easing {
        CurveEase::Linear => t,
        CurveEase::EaseIn => t * t,
        CurveEase::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
        CurveEase::EaseInOut => {
            if t < 0.5 {
                2.0 * t * t
            } else {
                1.0 - ((-2.0 * t + 2.0).powi(2) / 2.0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::dsl::parse_graph_script;

    use super::{BlurSharpenMode, compile_runtime_program};

    #[test]
    fn runtime_eval_invert_mix_changes_with_time() {
        let script = r#"
<Graph scope="clip" fps={60} duration="2s" size={[256,256]}>
  <Tex id="src" fmt="rgba8" from="input:clip0" />
  <Tex id="out" fmt="rgba8" size={[256,256]} />
  <Pass id="invert_pulse" kernel="invert_mix.wgsl" effect="invert_mix"
        in={["src"]}
        out={["out"]}
        params={{
          mix: "0.5 + 0.5*sin($time.sec*6.28318)"
        }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");
        let at_0 = runtime.evaluate_frame(0);
        let at_15 = runtime.evaluate_frame(15);
        assert!(at_0.invert_mix >= 0.0 && at_0.invert_mix <= 1.0);
        assert!(at_15.invert_mix >= 0.0 && at_15.invert_mix <= 1.0);
    }

    #[test]
    fn runtime_eval_blur_kernel_maps_sigma_to_layer_blur() {
        let script = r#"
<Graph scope="clip" fps={60} duration="1s" size={[1920,1080]}>
  <Tex id="src" fmt="rgba8" from="input:clip0" />
  <Tex id="out" fmt="rgba8" size={[1920,1080]} />
  <Pass id="fx_blur" kind="compute" kernel="blur_sharpen_detail_gaussian.wgsl" effect="gaussian_5tap_h"
        in={["src"]}
        out={["out"]}
        params={{
          sigma: "1.5 + 0.5*sin($time.sec*6.28318)"
        }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");
        let at_0 = runtime.evaluate_frame(0);
        let at_15 = runtime.evaluate_frame(15);
        let sigma0 = at_0.layer_blur_sigma.unwrap_or(-1.0);
        let sigma15 = at_15.layer_blur_sigma.unwrap_or(-1.0);
        assert!(sigma0 >= 0.0);
        assert!(sigma15 >= 0.0);
        assert!((sigma0 - sigma15).abs() > 0.0001);
    }

    #[test]
    fn runtime_duration_limits_effect_window() {
        let script = r#"
<Graph scope="clip" fps={60} apply="graph" duration="2s" size={[1920,1080]}>
  <Tex id="src" fmt="rgba8" from="input:clip0" />
  <Tex id="out" fmt="rgba8" size={[1920,1080]} />
  <Pass id="fx_blur" kind="compute" kernel="blur_sharpen_detail_gaussian.wgsl" effect="gaussian_5tap_h"
        in={["src"]}
        out={["out"]}
        params={{
          sigma: "10.0"
        }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");
        let in_window = runtime.evaluate_frame(119);
        let out_window = runtime.evaluate_frame(120);
        assert_eq!(in_window.layer_blur_sigma, Some(10.0));
        assert_eq!(out_window.layer_blur_sigma, None);
    }

    #[test]
    fn runtime_default_apply_clip_does_not_gate_by_duration() {
        let script = r#"
<Graph scope="clip" fps={60} duration="2s" size={[1920,1080]}>
  <Tex id="src" fmt="rgba8" from="input:clip0" />
  <Tex id="out" fmt="rgba8" size={[1920,1080]} />
  <Pass id="fx_blur" kind="compute" kernel="blur_sharpen_detail_gaussian.wgsl" effect="gaussian_5tap_h"
        in={["src"]}
        out={["out"]}
        params={{
          sigma: "10.0"
        }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");
        let out_window = runtime.evaluate_frame(120);
        assert_eq!(out_window.layer_blur_sigma, Some(10.0));
    }

    #[test]
    fn runtime_uses_pass_effect_field_for_blur_sharpen() {
        let script = r#"
<Graph scope="clip" fps={60} size={[1920,1080]}>
  <Tex id="src" fmt="rgba8" from="input:clip0" />
  <Tex id="out" fmt="rgba8" size={[1920,1080]} />
  <Pass id="fx_unsharp" kind="compute" kernel="blur_sharpen_detail_gaussian.wgsl" effect="unsharp"
        in={["src"]}
        out={["out"]}
        params={{ sigma: "2.0" }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");
        let out = runtime.evaluate_frame(0);
        assert_eq!(out.blur_sharpen_mode, Some(BlurSharpenMode::Unsharp));
        assert_eq!(out.layer_blur_sigma, None);
        assert_eq!(out.layer_sharpen_sigma, Some(2.0));
    }

    #[test]
    fn runtime_rejects_invalid_blur_effect() {
        let script = r#"
<Graph scope="clip" fps={60} size={[1920,1080]}>
  <Tex id="src" fmt="rgba8" from="input:clip0" />
  <Tex id="out" fmt="rgba8" size={[1920,1080]} />
  <Pass id="fx_bad" kind="compute" kernel="blur_sharpen_detail_gaussian.wgsl" effect="gaussian_9tap"
        in={["src"]}
        out={["out"]}
        params={{ sigma: "2.0" }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let err =
            compile_runtime_program(graph).expect_err("compile should fail on invalid effect");
        assert!(
            err.message
                .contains(
                    "expected gaussian_blur | gaussian_5tap_h | gaussian_5tap_v | sharpen | unsharp | box",
                ),
            "unexpected error: {}",
            err.message
        );
    }

    #[test]
    fn runtime_transition_core_fade_in_uses_param_window() {
        let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="fade_in" kind="render" role="transition" kernel="transition_core.wgsl"
        effect="fade_in"
        in={["under"]} out={["out"]}
        params={{
          startSec: "0.5",
          durationSec: "1.0"
        }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");
        assert_eq!(
            runtime.evaluate_frame(0).layer_transition_opacity,
            Some(0.0)
        );
        assert_eq!(
            runtime.evaluate_frame(30).layer_transition_opacity,
            Some(0.0)
        );
        assert_eq!(
            runtime.evaluate_frame(60).layer_transition_opacity,
            Some(0.5)
        );
        assert_eq!(
            runtime.evaluate_frame(90).layer_transition_opacity,
            Some(1.0)
        );
    }

    #[test]
    fn runtime_transition_core_fade_out_uses_param_window() {
        let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="fade_out" kind="render" role="transition" kernel="transition_core.wgsl"
        effect="fade_out"
        in={["under"]} out={["out"]}
        params={{
          startSec: "0.0",
          durationSec: "2.0"
        }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");
        assert_eq!(
            runtime.evaluate_frame(0).layer_transition_opacity,
            Some(1.0)
        );
        assert_eq!(
            runtime.evaluate_frame(60).layer_transition_opacity,
            Some(0.5)
        );
        assert_eq!(
            runtime.evaluate_frame(120).layer_transition_opacity,
            Some(0.0)
        );
    }

    #[test]
    fn runtime_transition_core_fade_out_defaults_to_tail_when_start_missing() {
        let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="fade_out" kind="render" role="transition" kernel="transition_core.wgsl"
        effect="fade_out"
        in={["under"]} out={["out"]}
        params={{
          durationSec: "0.6"
        }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");
        // Simulate a 3s layer clip: start fade-out at 2.4s when startSec is omitted.
        assert_eq!(
            runtime
                .evaluate_at_time_sec(1.0, Some(3.0))
                .layer_transition_opacity,
            Some(1.0)
        );
        assert_eq!(
            runtime
                .evaluate_at_time_sec(2.4, Some(3.0))
                .layer_transition_opacity,
            Some(1.0)
        );
        let mid = runtime
            .evaluate_at_time_sec(2.7, Some(3.0))
            .layer_transition_opacity
            .unwrap_or(-1.0);
        assert!((mid - 0.5).abs() < 0.0005, "unexpected mid opacity: {mid}");
        let end = runtime
            .evaluate_at_time_sec(3.0, Some(3.0))
            .layer_transition_opacity
            .unwrap_or(-1.0);
        assert!(end.abs() < 0.0005, "unexpected end opacity: {end}");
    }

    #[test]
    fn runtime_transition_core_fade_out_ignores_progress_expr_and_uses_tail_window() {
        let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="fade_out" kind="render" role="transition" kernel="transition_core.wgsl"
        effect="fade_out"
        in={["under"]} out={["out"]}
        params={{
          durationSec: "0.6",
          mix: "$time.norm"
        }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");
        // With a 3s layer clip, fade-out should still happen at tail regardless of mix/progress.
        assert_eq!(
            runtime
                .evaluate_at_time_sec(1.0, Some(3.0))
                .layer_transition_opacity,
            Some(1.0)
        );
        let mid = runtime
            .evaluate_at_time_sec(2.7, Some(3.0))
            .layer_transition_opacity
            .unwrap_or(-1.0);
        assert!((mid - 0.5).abs() < 0.0005, "unexpected mid opacity: {mid}");
    }

    #[test]
    fn runtime_transition_core_fade_in_and_fade_out_compose_for_clip() {
        let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="fade_in" kind="render" role="transition" kernel="transition_core.wgsl"
        effect="fade_in"
        in={["under"]} out={["out"]}
        params={{
          durationSec: "2.0"
        }} />
  <Pass id="fade_out" kind="render" role="transition" kernel="transition_core.wgsl"
        effect="fade_out"
        in={["under"]} out={["out"]}
        params={{
          durationSec: "2.0"
        }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");
        // Simulate a 10s layer clip.
        assert_eq!(
            runtime
                .evaluate_at_time_sec(0.0, Some(10.0))
                .layer_transition_opacity,
            Some(0.0)
        );
        let fade_in_mid = runtime
            .evaluate_at_time_sec(1.0, Some(10.0))
            .layer_transition_opacity
            .unwrap_or(-1.0);
        assert!(
            (fade_in_mid - 0.5).abs() < 0.0005,
            "unexpected fade-in midpoint opacity: {fade_in_mid}"
        );
        assert_eq!(
            runtime
                .evaluate_at_time_sec(5.0, Some(10.0))
                .layer_transition_opacity,
            Some(1.0)
        );
        let fade_out_mid = runtime
            .evaluate_at_time_sec(9.0, Some(10.0))
            .layer_transition_opacity
            .unwrap_or(-1.0);
        assert!(
            (fade_out_mid - 0.5).abs() < 0.0005,
            "unexpected fade-out midpoint opacity: {fade_out_mid}"
        );
    }

    #[test]
    fn runtime_transition_core_dissolve_respects_easing() {
        let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Input id="prev" type="video" from="input:prev" />
  <Input id="next" type="video" from="input:next" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="dissolve" kind="render" role="transition" kernel="transition_core.wgsl"
        effect="dissolve"
        in={["prev","next"]} out={["out"]}
        params={{
          progress: "$time.sec*0.5",
          easing: "ease-in"
        }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");
        let at_mid = runtime.evaluate_frame(60);
        assert!(
            (at_mid.transition_dissolve_mix.unwrap_or(-1.0) - 0.25).abs() < 0.0001,
            "unexpected mix: {:?}",
            at_mid.transition_dissolve_mix
        );
    }

    #[test]
    fn runtime_transition_core_missing_effect_is_parser_error() {
        let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="bad_transition" kind="render" role="transition" kernel="transition_core.wgsl"
        in={["under"]} out={["out"]} />
  <Present from="out" />
</Graph>
"#;
        let err = parse_graph_script(script).expect_err("missing effect should fail in parser");
        assert!(
            err.message.contains("Missing required attribute: effect"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn runtime_rejects_removed_transition_kernel_names() {
        let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="legacy_transition" kind="render" role="transition" kernel="transition_dissolve.wgsl" effect="dissolve"
        in={["under"]} out={["out"]} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let err = compile_runtime_program(graph).expect_err("legacy transition kernel should fail");
        assert!(
            err.message
                .contains("use transition_core.wgsl + effect instead"),
            "unexpected error: {}",
            err.message
        );
    }

    #[test]
    fn runtime_uses_explicit_kernel_with_effect() {
        let script = r#"
<Graph scope="clip" fps={60} size={[1920,1080]}>
  <Tex id="src" fmt="rgba8" from="input:clip0" />
  <Tex id="out" fmt="rgba8" size={[1920,1080]} />
  <Pass id="fx_blur" kind="compute" kernel="blur_sharpen_detail_gaussian.wgsl" effect="gaussian_5tap_h"
        in={["src"]}
        out={["out"]}
        params={{ sigma: "2.0" }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");
        let out = runtime.evaluate_frame(0);
        assert_eq!(out.blur_sharpen_mode, Some(BlurSharpenMode::Gaussian5tapH));
        assert_eq!(out.layer_blur_sigma, Some(2.0));
    }

    #[test]
    fn runtime_opacity_effect_sets_layer_transition_opacity() {
        let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="fx_opacity" kind="compute" kernel="composite_core.wgsl" effect="opacity"
        in={["under"]}
        out={["out"]}
        params={{ opacity: "0.7" }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");
        let out = runtime.evaluate_frame(0);
        assert_eq!(out.layer_transition_opacity, Some(0.7));
    }

    #[test]
    fn runtime_transition_opacity_curve_uses_seconds_domain() {
        let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="fade_in" kind="render" role="transition" kernel="transition_core.wgsl"
        effect="fade_in"
        in={["under"]} out={["out"]}
        params={{
          opacity: "curve(\"0.00:0.0:linear, 1.00:1.0:ease_in_out\")",
          durationSec: "2.0"
        }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");
        assert_eq!(
            runtime.evaluate_frame(0).layer_transition_opacity,
            Some(0.0)
        );
        assert!(
            runtime
                .evaluate_frame(30)
                .layer_transition_opacity
                .unwrap_or(0.0)
                > 0.0
        );
        assert_eq!(
            runtime.evaluate_frame(60).layer_transition_opacity,
            Some(1.0)
        );
    }

    #[test]
    fn runtime_opacity_curve_matches_points_and_holds_tail_value() {
        let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="fx_opacity" kind="compute" effect="opacity"
        in={["under"]} out={["out"]}
        params={{
          opacity: "curve(\"0.00:0.137:ease_in, 1.56:0.929:linear, 2.00:0.700:ease_in_out\")"
        }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");

        let v0 = runtime
            .evaluate_at_time_sec(0.00, Some(30.0))
            .layer_transition_opacity
            .unwrap_or(-1.0);
        let v156 = runtime
            .evaluate_at_time_sec(1.56, Some(30.0))
            .layer_transition_opacity
            .unwrap_or(-1.0);
        let v178 = runtime
            .evaluate_at_time_sec(1.78, Some(30.0))
            .layer_transition_opacity
            .unwrap_or(-1.0);
        let v200 = runtime
            .evaluate_at_time_sec(2.00, Some(30.0))
            .layer_transition_opacity
            .unwrap_or(-1.0);
        let v500 = runtime
            .evaluate_at_time_sec(5.00, Some(30.0))
            .layer_transition_opacity
            .unwrap_or(-1.0);

        assert!((v0 - 0.137).abs() < 1e-4, "unexpected t=0.00 value: {v0}");
        assert!(
            (v156 - 0.929).abs() < 1e-4,
            "unexpected t=1.56 value: {v156}"
        );
        // Segment [1.56, 2.00] is linear (easing comes from the starting point at 1.56).
        assert!(
            (v178 - 0.8145).abs() < 1e-4,
            "unexpected t=1.78 value: {v178}"
        );
        assert!(
            (v200 - 0.700).abs() < 1e-4,
            "unexpected t=2.00 value: {v200}"
        );
        assert!(
            (v500 - 0.700).abs() < 1e-4,
            "unexpected t=5.00 value: {v500}"
        );
    }

    #[test]
    fn runtime_lut_effect_sets_layer_lut_mix() {
        let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="fx_lut" kind="compute" kernel="color_core.wgsl" effect="lut"
        in={["under"]}
        out={["out"]}
        params={{ mix: "0.6 + 0.2*sin($time.sec)" }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");
        let out = runtime.evaluate_frame(0);
        let mix = out.layer_lut_mix.expect("lut mix");
        assert!(mix >= 0.0 && mix <= 1.0);
    }

    #[test]
    fn runtime_hsla_overlay_effect_sets_hsla_fields() {
        let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="fx_hsla_overlay" kind="compute" kernel="color_core.wgsl" effect="hsla_overlay"
        in={["under"]}
        out={["out"]}
        params={{
          hue: "210.0",
          saturation: "0.70",
          lightness: "0.41",
          alpha: "0.45"
        }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");
        let out = runtime.evaluate_frame(0);
        assert_eq!(out.layer_hsla_hue, Some(210.0));
        assert_eq!(out.layer_hsla_saturation, Some(0.70));
        assert_eq!(out.layer_hsla_lightness, Some(0.41));
        assert_eq!(out.layer_hsla_alpha, Some(0.45));
    }

    #[test]
    fn runtime_blur_sigma_curve_uses_seconds_domain_and_holds_tail() {
        let script = r#"
<Graph scope="layer" fps={60} size={[1920,1080]}>
  <Input id="under" type="video" from="input:under" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="fx_blur" kind="compute" effect="gaussian_5tap_h"
        in={["under"]} out={["out"]}
        params={{ sigma: "curve(\"0.00:2.0:linear, 1.00:10.0:ease_in_out\")" }} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");

        let v0 = runtime
            .evaluate_at_time_sec(0.0, Some(8.0))
            .layer_blur_sigma
            .unwrap_or(-1.0);
        let v050 = runtime
            .evaluate_at_time_sec(0.5, Some(8.0))
            .layer_blur_sigma
            .unwrap_or(-1.0);
        let v100 = runtime
            .evaluate_at_time_sec(1.0, Some(8.0))
            .layer_blur_sigma
            .unwrap_or(-1.0);
        let v500 = runtime
            .evaluate_at_time_sec(5.0, Some(8.0))
            .layer_blur_sigma
            .unwrap_or(-1.0);

        assert!((v0 - 2.0).abs() < 1e-4, "unexpected t=0.0 sigma: {v0}");
        assert!(v050 > 2.0 && v050 < 10.0, "unexpected t=0.5 sigma: {v050}");
        assert!((v100 - 10.0).abs() < 1e-4, "unexpected t=1.0 sigma: {v100}");
        assert!((v500 - 10.0).abs() < 1e-4, "unexpected t=5.0 sigma: {v500}");
    }

    #[test]
    fn runtime_rejects_missing_kernel_when_effect_not_mapped() {
        let script = r#"
<Graph scope="clip" fps={60} size={[1920,1080]}>
  <Tex id="src" fmt="rgba8" from="input:clip0" />
  <Tex id="out" fmt="rgba8" size={[1920,1080]} />
  <Pass id="fx_unknown" kind="compute" effect="unknown_effect"
        in={["src"]}
        out={["out"]} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let err = compile_runtime_program(graph).expect_err("compile should fail");
        assert!(
            err.message
                .contains("missing kernel and no default mapping for effect"),
            "unexpected error: {}",
            err.message
        );
    }

    #[test]
    fn runtime_accepts_explicit_custom_wgsl_kernel() {
        let script = r#"
<Graph scope="clip" fps={60} size={[1920,1080]}>
  <Tex id="src" fmt="rgba8" from="input:clip0" />
  <Tex id="out" fmt="rgba8" size={[1920,1080]} />
  <Pass id="fx_custom" kind="compute" kernel="my_custom_shader.wgsl" effect="my_effect"
        in={["src"]}
        out={["out"]} />
  <Present from="out" />
</Graph>
"#;
        let graph = parse_graph_script(script).expect("graph parse");
        let runtime = compile_runtime_program(graph).expect("runtime compile");
        assert!(runtime.unsupported_kernels().is_empty());
        assert_eq!(runtime.supported_pass_count(), 1);
    }
}
