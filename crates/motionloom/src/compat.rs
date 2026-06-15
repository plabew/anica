// =========================================
// crates/motionloom/src/compat.rs
// =========================================

use crate::dsl::{GraphScript, parse_graph_script};
use crate::error::GraphParseError;
use crate::process::model::PassNode;
use crate::root::{RootGraphShell, inspect_root_graph};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GpuCompatibilityTarget {
    NativeScenePreview,
    WasmSceneCanvas,
    WasmProcessWebGpu,
    WgpuTextureOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GpuCompatibilitySeverity {
    Info,
    Warning,
    Blocking,
}

/// Preview surface path that `ScenePreviewBackend::Auto` is expected to take
/// on the current platform for a compatible scene graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScenePreviewPath {
    MacOsCVPixelBuffer,
    WindowsD3D,
    LinuxDmabuf,
    WgpuTexture,
    CpuBgra,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GpuCompatibilityIssue {
    pub target: GpuCompatibilityTarget,
    pub severity: GpuCompatibilitySeverity,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GpuCompatibilityReport {
    pub root: RootGraphShell,
    pub can_use_native_scene_preview: bool,
    pub can_use_wasm_scene_canvas: bool,
    pub can_use_wasm_process_webgpu: bool,
    pub can_use_wgpu_texture_output: bool,
    pub likely_cpu_fallback: bool,
    pub likely_preview_path: ScenePreviewPath,
    pub issues: Vec<GpuCompatibilityIssue>,
}

impl GpuCompatibilityReport {
    pub fn blocking_issues(&self) -> impl Iterator<Item = &GpuCompatibilityIssue> {
        self.issues
            .iter()
            .filter(|issue| issue.severity == GpuCompatibilitySeverity::Blocking)
    }
}

pub fn inspect_gpu_compatibility(script: &str) -> Result<GpuCompatibilityReport, GraphParseError> {
    let root = inspect_root_graph(script)?;
    let graph = parse_graph_script(script)?;
    let mut issues = Vec::<GpuCompatibilityIssue>::new();

    inspect_scene_preview_compatibility(&root, &graph, &mut issues);
    inspect_wasm_process_compatibility(&root, &graph, &mut issues);
    inspect_wgpu_texture_output_compatibility(&graph, &mut issues);

    let can_use_native_scene_preview =
        !has_blocking(&issues, GpuCompatibilityTarget::NativeScenePreview);
    let can_use_wasm_scene_canvas = !has_blocking(&issues, GpuCompatibilityTarget::WasmSceneCanvas);
    let can_use_wasm_process_webgpu =
        !has_blocking(&issues, GpuCompatibilityTarget::WasmProcessWebGpu);
    let can_use_wgpu_texture_output =
        !has_blocking(&issues, GpuCompatibilityTarget::WgpuTextureOutput);
    let likely_cpu_fallback = !issues.is_empty()
        && issues
            .iter()
            .any(|issue| issue.severity == GpuCompatibilitySeverity::Blocking);

    let likely_preview_path =
        choose_likely_preview_path(can_use_native_scene_preview, can_use_wgpu_texture_output);

    Ok(GpuCompatibilityReport {
        root,
        can_use_native_scene_preview,
        can_use_wasm_scene_canvas,
        can_use_wasm_process_webgpu,
        can_use_wgpu_texture_output,
        likely_cpu_fallback,
        likely_preview_path,
        issues,
    })
}

fn choose_likely_preview_path(
    can_use_native_scene_preview: bool,
    can_use_wgpu_texture_output: bool,
) -> ScenePreviewPath {
    if can_use_native_scene_preview {
        #[cfg(target_os = "macos")]
        return ScenePreviewPath::MacOsCVPixelBuffer;
        #[cfg(target_os = "windows")]
        return ScenePreviewPath::WindowsD3D;
        // Linux DMA-BUF is not implemented yet; Auto falls back to CpuBgra.
        #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
        return ScenePreviewPath::CpuBgra;
    }
    if can_use_wgpu_texture_output {
        return ScenePreviewPath::WgpuTexture;
    }
    ScenePreviewPath::CpuBgra
}

fn inspect_scene_preview_compatibility(
    root: &RootGraphShell,
    graph: &GraphScript,
    issues: &mut Vec<GpuCompatibilityIssue>,
) {
    if root.has_scene && root.has_process {
        push_blocking(
            issues,
            GpuCompatibilityTarget::NativeScenePreview,
            "mixed_scene_process",
            "Mixed <Scene> + <Process> graphs need scene-to-process composition; current native live GPU preview may use CPU fallback.",
        );
        push_blocking(
            issues,
            GpuCompatibilityTarget::WasmSceneCanvas,
            "mixed_scene_process",
            "Mixed <Scene> + <Process> graphs are not direct WebGPU scene-canvas renders; current WASM path may use CPU fallback.",
        );
    }

    if has_graph_composition(graph) {
        push_blocking(
            issues,
            GpuCompatibilityTarget::NativeScenePreview,
            "tex_pass_output_composition",
            "Tex/Pass/Output composition is not GPU-native in the scene live preview path yet.",
        );
        push_blocking(
            issues,
            GpuCompatibilityTarget::WasmSceneCanvas,
            "tex_pass_output_composition",
            "Tex/Pass/Output composition is not supported by direct WASM scene canvas rendering yet.",
        );
    }

    if graph.textures.iter().any(|texture| {
        texture
            .from
            .as_deref()
            .is_some_and(|from| from.starts_with("scene:"))
    }) {
        push_info(
            issues,
            GpuCompatibilityTarget::NativeScenePreview,
            "scene_texture_input",
            "A process texture reads from a scene source; this requires scene-to-process composition support.",
        );
        push_info(
            issues,
            GpuCompatibilityTarget::WasmSceneCanvas,
            "scene_texture_input",
            "A process texture reads from a scene source; this is outside direct scene canvas rendering.",
        );
    }
}

fn inspect_wasm_process_compatibility(
    root: &RootGraphShell,
    graph: &GraphScript,
    issues: &mut Vec<GpuCompatibilityIssue>,
) {
    if !root.has_process && graph.passes.is_empty() {
        return;
    }

    for pass in &graph.passes {
        if !is_wasm_process_webgpu_effect(pass) {
            push_blocking(
                issues,
                GpuCompatibilityTarget::WasmProcessWebGpu,
                "unsupported_wasm_process_effect",
                format!(
                    "Pass '{}' uses effect '{}', which is not supported by the WASM process WebGPU path yet.",
                    pass.id, pass.effect
                ),
            );
        }
    }
}

fn inspect_wgpu_texture_output_compatibility(
    graph: &GraphScript,
    issues: &mut Vec<GpuCompatibilityIssue>,
) {
    if has_graph_composition(graph) {
        push_blocking(
            issues,
            GpuCompatibilityTarget::WgpuTextureOutput,
            "tex_pass_output_composition",
            "Direct wgpu texture output does not support Tex/Pass/Output composition yet.",
        );
    }

    if !graph.texts.is_empty() && graph.scenes.is_empty() && graph.scene_nodes.is_empty() {
        push_blocking(
            issues,
            GpuCompatibilityTarget::WgpuTextureOutput,
            "top_level_text",
            "Direct wgpu texture output does not support top-level <Text> nodes in the simple scene path yet.",
        );
    }
}

fn has_graph_composition(graph: &GraphScript) -> bool {
    !graph.textures.is_empty()
        || !graph.passes.is_empty()
        || !graph.outputs.is_empty()
        || !graph.layers.is_empty()
        || !graph.world_sources.is_empty()
}

fn is_wasm_process_webgpu_effect(pass: &PassNode) -> bool {
    matches!(
        normalize_effect_key(&pass.effect).as_str(),
        "hsla_overlay"
            | "hsla"
            | "tint_overlay"
            | "color_tone_hsla_overlay"
            | "gaussian_5tap_blur"
            | "gaussian_blur"
            | "blur"
            | "gaussian_5tap_h"
            | "gaussian_5tap_v"
    )
}

fn normalize_effect_key(effect: &str) -> String {
    effect.trim().to_ascii_lowercase().replace(['.', '-'], "_")
}

fn has_blocking(issues: &[GpuCompatibilityIssue], target: GpuCompatibilityTarget) -> bool {
    issues
        .iter()
        .any(|issue| issue.target == target && issue.severity == GpuCompatibilitySeverity::Blocking)
}

fn push_info(
    issues: &mut Vec<GpuCompatibilityIssue>,
    target: GpuCompatibilityTarget,
    code: impl Into<String>,
    message: impl Into<String>,
) {
    push_issue(
        issues,
        target,
        GpuCompatibilitySeverity::Info,
        code,
        message,
    );
}

fn push_blocking(
    issues: &mut Vec<GpuCompatibilityIssue>,
    target: GpuCompatibilityTarget,
    code: impl Into<String>,
    message: impl Into<String>,
) {
    push_issue(
        issues,
        target,
        GpuCompatibilitySeverity::Blocking,
        code,
        message,
    );
}

fn push_issue(
    issues: &mut Vec<GpuCompatibilityIssue>,
    target: GpuCompatibilityTarget,
    severity: GpuCompatibilitySeverity,
    code: impl Into<String>,
    message: impl Into<String>,
) {
    issues.push(GpuCompatibilityIssue {
        target,
        severity,
        code: code.into(),
        message: message.into(),
    });
}

#[cfg(test)]
mod tests {
    use super::{
        GpuCompatibilitySeverity, GpuCompatibilityTarget, ScenePreviewPath,
        inspect_gpu_compatibility,
    };

    #[test]
    fn pure_scene_is_compatible_with_direct_scene_canvas() {
        let report = inspect_gpu_compatibility(
            r##"
<Graph fps={30} duration="1s" size={[320,180]}>
  <Background color="#000000" />
  <Scene id="demo_scene">
    <Timeline>
      <Track>
        <Sequence duration="1s">
          <Circle x="160" y="90" radius="40" color="#FFFFFF" />
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="demo_scene" />
</Graph>
"##,
        )
        .expect("compatibility report");

        assert!(report.can_use_wasm_scene_canvas);
        assert!(report.can_use_native_scene_preview);
        assert!(!report.likely_cpu_fallback);

        #[cfg(target_os = "macos")]
        assert_eq!(
            report.likely_preview_path,
            ScenePreviewPath::MacOsCVPixelBuffer
        );
        #[cfg(target_os = "windows")]
        assert_eq!(report.likely_preview_path, ScenePreviewPath::WindowsD3D);
        #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
        assert_eq!(report.likely_preview_path, ScenePreviewPath::CpuBgra);
    }

    #[test]
    fn mixed_scene_process_reports_cpu_fallback_reasons() {
        let report = inspect_gpu_compatibility(
            r##"
<Graph fps={30} duration="1s" size={[320,180]}>
  <Scene id="demo_scene">
    <Timeline>
      <Track>
        <Sequence duration="1s">
          <Circle x="160" y="90" radius="40" color="#FFFFFF" />
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Process id="post">
    <Tex id="scene_src" fmt="rgba16f" from="scene:demo_scene" />
    <Tex id="out" fmt="rgba16f" size={[320,180]} />
    <Pass id="bloom" kind="compute" effect="glow_bloom"
          in={["scene_src"]} out={["out"]}
          params={{ intensity: "1.0" }} />
  </Process>
  <Present from="post" />
</Graph>
"##,
        )
        .expect("compatibility report");

        assert!(report.likely_cpu_fallback);
        assert!(!report.can_use_wasm_scene_canvas);
        assert!(!report.can_use_native_scene_preview);
        assert!(!report.can_use_wasm_process_webgpu);
        assert_eq!(report.likely_preview_path, ScenePreviewPath::CpuBgra);
        assert!(report.issues.iter().any(|issue| {
            issue.target == GpuCompatibilityTarget::WasmProcessWebGpu
                && issue.severity == GpuCompatibilitySeverity::Blocking
                && issue.code == "unsupported_wasm_process_effect"
        }));
    }

    #[test]
    fn process_hsla_and_blur_are_wasm_process_webgpu_compatible() {
        let report = inspect_gpu_compatibility(
            r##"
<Graph fps={30} duration="1s" size={[320,180]}>
  <Process id="fx">
    <Input id="clip0" type="video" from="input:clip0" />
    <Tex id="src" fmt="rgba16f" from="clip0" />
    <Tex id="mid" fmt="rgba16f" size={[320,180]} />
    <Tex id="out" fmt="rgba16f" size={[320,180]} />
    <Pass id="tone" kind="compute" effect="hsla_overlay"
          in={["src"]} out={["mid"]}
          params={{ hue: "150", saturation: "0.2", lightness: "0.4", alpha: "0.25" }} />
    <Pass id="blur" kind="compute" effect="gaussian_5tap_blur"
          in={["mid"]} out={["out"]}
          params={{ sigma: "5" }} />
  </Process>
  <Present from="fx" />
</Graph>
"##,
        )
        .expect("compatibility report");

        assert!(report.can_use_wasm_process_webgpu);
    }
}
