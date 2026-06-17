// =========================================
// crates/motionloom/src/process/cpu_renderer.rs
// =========================================

use image::RgbaImage;

use crate::dsl::{GraphScript, PassNode, parse_graph_script};
use crate::error::{GraphParseError, RuntimeCompileError};
use crate::process::cpu_effects::{
    apply_gaussian_blur, apply_hsla_overlay, apply_separable_gaussian_blur,
};
use crate::process::runtime::{RuntimeProgram, compile_runtime_program, eval_time_expr};

#[derive(Debug, thiserror::Error)]
pub enum ProcessCpuRenderError {
    #[error(transparent)]
    Parse(#[from] GraphParseError),
    #[error(transparent)]
    Compile(#[from] RuntimeCompileError),
    #[error("invalid RGBA buffer: expected {expected} bytes for {width}x{height}, got {actual}")]
    InvalidRgbaBuffer {
        width: u32,
        height: u32,
        expected: usize,
        actual: usize,
    },
    #[error("invalid RGBA image buffer")]
    InvalidRgbaImageBuffer,
}

pub struct ProcessCpuRenderer {
    graph: GraphScript,
    _runtime: RuntimeProgram,
}

impl ProcessCpuRenderer {
    pub fn new(graph: GraphScript) -> Result<Self, ProcessCpuRenderError> {
        let runtime = compile_runtime_program(graph.clone())?;
        Ok(Self {
            graph,
            _runtime: runtime,
        })
    }

    pub fn render_frame(
        &self,
        frame: u32,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<RgbaImage, ProcessCpuRenderError> {
        let expected = width as usize * height as usize * 4;
        if width == 0 || height == 0 || rgba.len() != expected {
            return Err(ProcessCpuRenderError::InvalidRgbaBuffer {
                width,
                height,
                expected,
                actual: rgba.len(),
            });
        }
        let image = RgbaImage::from_raw(width, height, rgba.to_vec())
            .ok_or(ProcessCpuRenderError::InvalidRgbaImageBuffer)?;
        Ok(self.render_image(frame, image))
    }

    pub fn render_image(&self, frame: u32, mut image: RgbaImage) -> RgbaImage {
        let time_sec = frame as f32 / self.graph.fps.max(1.0);
        let duration_sec =
            (self.graph.duration_ms as f32 / 1000.0).max(1.0 / self.graph.fps.max(1.0));
        let time_norm = (time_sec / duration_sec).clamp(0.0, 1.0);

        for pass in &self.graph.passes {
            image = apply_process_pass(image, pass, time_norm, time_sec);
        }

        image
    }
}

pub fn render_process_frame_cpu(
    script: &str,
    frame: u32,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<RgbaImage, ProcessCpuRenderError> {
    let graph = parse_graph_script(script)?;
    ProcessCpuRenderer::new(graph)?.render_frame(frame, width, height, rgba)
}

fn apply_process_pass(
    image: RgbaImage,
    pass: &PassNode,
    time_norm: f32,
    time_sec: f32,
) -> RgbaImage {
    use crate::process::effect_kind::{ProcessEffect, resolve_process_effect};
    match resolve_process_effect(&pass.effect) {
        Some(ProcessEffect::HslaOverlay) => {
            let hue = process_param_f32(pass, &["hue", "h"], time_norm, time_sec, 0.0);
            let saturation =
                process_param_f32(pass, &["saturation", "sat", "s"], time_norm, time_sec, 0.0);
            let lightness =
                process_param_f32(pass, &["lightness", "lum", "l"], time_norm, time_sec, 0.0);
            let alpha = process_param_f32(pass, &["alpha", "a"], time_norm, time_sec, 0.0);
            apply_hsla_overlay(&image, hue, saturation, lightness, alpha)
        }
        Some(ProcessEffect::GaussianBlur) => {
            let sigma = process_param_f32(pass, &["sigma"], time_norm, time_sec, 1.0);
            apply_gaussian_blur(&image, sigma.clamp(0.0, 64.0))
        }
        Some(ProcessEffect::GaussianBlurHorizontal) => {
            let sigma = process_param_f32(pass, &["sigma"], time_norm, time_sec, 1.0);
            apply_separable_gaussian_blur(&image, sigma.clamp(0.0, 64.0), true)
        }
        Some(ProcessEffect::GaussianBlurVertical) => {
            let sigma = process_param_f32(pass, &["sigma"], time_norm, time_sec, 1.0);
            apply_separable_gaussian_blur(&image, sigma.clamp(0.0, 64.0), false)
        }
        Some(ProcessEffect::GlowBloom) => {
            // CPU renderer does not implement bloom yet; pass through unchanged.
            image
        }
        Some(ProcessEffect::GlowStack)
        | Some(ProcessEffect::ToneMap)
        | Some(ProcessEffect::LightSweep) => {
            // CPU renderer does not implement these effects yet; pass through unchanged.
            image
        }
        None => image,
    }
}

fn process_param_f32(
    pass: &PassNode,
    keys: &[&str],
    time_norm: f32,
    time_sec: f32,
    fallback: f32,
) -> f32 {
    keys.iter()
        .find_map(|key| {
            pass.params
                .iter()
                .find(|param| param.key == *key)
                .and_then(|param| eval_time_expr(&param.value, time_norm, time_sec).ok())
        })
        .unwrap_or(fallback)
}
