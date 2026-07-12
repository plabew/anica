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
    inspect_scene_gpu_feature_compatibility(&graph, &mut issues);
    inspect_wasm_process_compatibility(&root, &graph, &mut issues);
    inspect_wgpu_texture_output_compatibility(&graph, &mut issues);

    let can_use_native_scene_preview =
        !has_blocking(&issues, GpuCompatibilityTarget::NativeScenePreview);
    let can_use_wasm_scene_canvas = !has_blocking(&issues, GpuCompatibilityTarget::WasmSceneCanvas);
    let can_use_wasm_process_webgpu =
        !has_blocking(&issues, GpuCompatibilityTarget::WasmProcessWebGpu);
    let can_use_wgpu_texture_output =
        !has_blocking(&issues, GpuCompatibilityTarget::WgpuTextureOutput);
    let likely_cpu_fallback = issues.iter().any(|issue| {
        issue.severity == GpuCompatibilitySeverity::Blocking
            && matches!(
                issue.target,
                GpuCompatibilityTarget::NativeScenePreview
                    | GpuCompatibilityTarget::WasmSceneCanvas
                    | GpuCompatibilityTarget::WasmProcessWebGpu
            )
    });

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
        let all_process_gpu_native = graph.passes.iter().all(is_wasm_process_webgpu_effect);
        if all_process_gpu_native {
            push_info(
                issues,
                GpuCompatibilityTarget::NativeScenePreview,
                "mixed_scene_process",
                "Mixed <Scene> + <Process> graph with GPU-native process passes; preview will work.",
            );
            push_info(
                issues,
                GpuCompatibilityTarget::WasmSceneCanvas,
                "mixed_scene_process",
                "Mixed <Scene> + <Process> graph with GPU-native process passes; canvas render will work.",
            );
        } else {
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
    }

    if has_non_gpu_native_composition(graph) {
        push_blocking(
            issues,
            GpuCompatibilityTarget::NativeScenePreview,
            "tex_pass_output_composition",
            "Tex/Pass/Output composition contains non-GPU-native elements in the scene live preview path.",
        );
        push_blocking(
            issues,
            GpuCompatibilityTarget::WasmSceneCanvas,
            "tex_pass_output_composition",
            "Tex/Pass/Output composition contains non-GPU-native elements for direct WASM scene canvas rendering.",
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

fn inspect_scene_gpu_feature_compatibility(
    graph: &GraphScript,
    issues: &mut Vec<GpuCompatibilityIssue>,
) {
    use crate::scene::drawable::is_gpu_native_blend;
    use crate::scene::model::SceneNode;

    let mut features = SceneGpuFeatures::default();

    fn scan_nodes(nodes: &[SceneNode], features: &mut SceneGpuFeatures) {
        for node in nodes {
            match node {
                SceneNode::Path(path) => {
                    features.has_path = true;
                    if path.trim_start.trim() != "0" && path.trim_start.trim() != "0.0" {
                        features.has_trim_path = true;
                    }
                    if path.trim_end.trim() != "1" && path.trim_end.trim() != "1.0" {
                        features.has_trim_path = true;
                    }
                    if !is_gpu_native_blend(&path.blend) {
                        features.has_non_gpu_blend = true;
                    }
                }
                SceneNode::Repeat(_) => {
                    features.has_repeat = true;
                }
                SceneNode::Text(text) => {
                    if text.font_family.is_some()
                        || (text.render_scale.trim() != "1" && text.render_scale.trim() != "1.0")
                    {
                        features.has_advanced_text = true;
                    }
                }
                SceneNode::Rect(rect) => {
                    if !is_gpu_native_blend(&rect.blend) {
                        features.has_non_gpu_blend = true;
                    }
                }
                SceneNode::Circle(circle) => {
                    if !is_gpu_native_blend(&circle.blend) {
                        features.has_non_gpu_blend = true;
                    }
                }
                SceneNode::Line(line) => {
                    if !is_gpu_native_blend(&line.blend) {
                        features.has_non_gpu_blend = true;
                    }
                }
                SceneNode::Polyline(polyline) => {
                    if !is_gpu_native_blend(&polyline.blend) {
                        features.has_non_gpu_blend = true;
                    }
                }
                SceneNode::Defs(defs) => {
                    for _gradient in &defs.gradients {
                        features.has_gradient = true;
                    }
                    for mask in &defs.masks {
                        features.has_mask_def = true;
                        if mask.follow.is_some() {
                            features.has_mask_follow = true;
                        }
                        scan_nodes(&mask.children, features);
                    }
                    for precompose in &defs.precomposes {
                        scan_nodes(&precompose.children, features);
                    }
                    for component in &defs.components {
                        scan_nodes(&component.children, features);
                    }
                }
                SceneNode::Timeline(timeline) => {
                    scan_nodes(&timeline.children, features);
                }
                SceneNode::Track(track) => {
                    scan_nodes(&track.children, features);
                }
                SceneNode::Sequence(sequence) => {
                    scan_nodes(&sequence.children, features);
                }
                SceneNode::Chain(chain) => {
                    scan_nodes(&chain.children, features);
                }
                SceneNode::Group(group) => {
                    if group.mask.is_some() || group.mask_from.is_some() {
                        features.has_scene_mask = true;
                    }
                    scan_nodes(&group.children, features);
                }
                SceneNode::Part(part) => {
                    scan_nodes(&part.children, features);
                }
                SceneNode::Camera(camera) => {
                    scan_nodes(&camera.children, features);
                }
                SceneNode::Character(character) => {
                    scan_nodes(&character.children, features);
                }
                SceneNode::Layer(layer) => {
                    if layer.mask.is_some()
                        || layer.mask_from.is_some()
                        || layer.matte_from.is_some()
                    {
                        features.has_scene_mask = true;
                    }
                    scan_nodes(&layer.children, features);
                }
                SceneNode::Precompose(precompose) => {
                    scan_nodes(&precompose.children, features);
                }
                SceneNode::Mask(mask) => {
                    features.has_mask_def = true;
                    if mask.follow.is_some() {
                        features.has_mask_follow = true;
                    }
                    scan_nodes(&mask.children, features);
                }
                _ => {}
            }
        }
    }

    for scene in &graph.scenes {
        scan_nodes(&scene.children, &mut features);
    }
    scan_nodes(&graph.scene_nodes, &mut features);

    // Check expressions for random() and complex math
    let script_str = graph.raw_script.as_deref().unwrap_or("");
    if script_str.contains("random(") {
        features.has_random_expr = true;
    }
    if script_str.contains("sin(") || script_str.contains("cos(") || script_str.contains("+ 0.0") {
        features.has_complex_expr = true;
    }

    if features.has_path {
        push_info(
            issues,
            GpuCompatibilityTarget::NativeScenePreview,
            "path",
            "Path nodes present; GPU path supports basic fill/stroke and trim.",
        );
    }
    if features.has_repeat {
        push_info(
            issues,
            GpuCompatibilityTarget::NativeScenePreview,
            "repeat",
            "Repeat nodes present; GPU path supports repeat with deterministic expressions.",
        );
    }
    if features.has_gradient {
        push_info(
            issues,
            GpuCompatibilityTarget::NativeScenePreview,
            "gradient",
            "Gradient fill present; GPU path supports LinearGradient and RadialGradient.",
        );
    }
    if features.has_non_gpu_blend {
        push_blocking(
            issues,
            GpuCompatibilityTarget::NativeScenePreview,
            "non_gpu_blend",
            "Non-GPU-native blend mode detected; GPU path only supports normal, multiply, screen, add.",
        );
        push_blocking(
            issues,
            GpuCompatibilityTarget::WasmSceneCanvas,
            "non_gpu_blend",
            "Non-GPU-native blend mode detected; GPU path only supports normal, multiply, screen, add.",
        );
    }
    if features.has_random_expr {
        push_info(
            issues,
            GpuCompatibilityTarget::NativeScenePreview,
            "random_expr",
            "Random expressions present; GPU path evaluates them deterministically at CPU time.",
        );
    }
    if features.has_complex_expr {
        push_info(
            issues,
            GpuCompatibilityTarget::NativeScenePreview,
            "complex_expr",
            "Complex math expressions present; GPU path evaluates them at CPU time.",
        );
    }
    if features.has_advanced_text {
        push_info(
            issues,
            GpuCompatibilityTarget::NativeScenePreview,
            "advanced_text",
            "Advanced text features (fontFamily, renderScale) present; GPU path rasterizes text to texture.",
        );
    }
    if features.has_trim_path {
        push_info(
            issues,
            GpuCompatibilityTarget::NativeScenePreview,
            "trim_path",
            "Path trimStart/trimEnd present; GPU path supports stroke trimming.",
        );
    }
    if features.has_scene_mask {
        push_info(
            issues,
            GpuCompatibilityTarget::NativeScenePreview,
            "scene_mask",
            "Scene mask/matte present; GPU path supports group and layer alpha/luma masks.",
        );
    }
    if features.has_mask_def {
        push_info(
            issues,
            GpuCompatibilityTarget::NativeScenePreview,
            "mask_def",
            "<Mask> definitions present; GPU path supports rect/circle/path mask textures.",
        );
    }
    if features.has_mask_follow {
        push_info(
            issues,
            GpuCompatibilityTarget::NativeScenePreview,
            "mask_follow",
            "<Mask follow=\"node:id\"> present; mask position follows the referenced scene node anchor.",
        );
    }
}

#[derive(Default)]
struct SceneGpuFeatures {
    has_path: bool,
    has_repeat: bool,
    has_gradient: bool,
    has_non_gpu_blend: bool,
    has_random_expr: bool,
    has_complex_expr: bool,
    has_advanced_text: bool,
    has_trim_path: bool,
    has_scene_mask: bool,
    has_mask_def: bool,
    has_mask_follow: bool,
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
    if has_non_gpu_native_composition(graph) {
        push_blocking(
            issues,
            GpuCompatibilityTarget::WgpuTextureOutput,
            "tex_pass_output_composition",
            "Direct wgpu texture output does not support non-GPU-native Tex/Pass/Output composition yet.",
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

fn has_non_gpu_native_composition(graph: &GraphScript) -> bool {
    let explicit_outputs = graph
        .outputs
        .iter()
        .filter(|o| !o.is_process_implicit)
        .count();
    !graph.layers.is_empty()
        || !graph.world_sources.is_empty()
        || explicit_outputs > 0
        || graph
            .passes
            .iter()
            .any(|pass| !is_wasm_process_webgpu_effect(pass))
}

fn is_wasm_process_webgpu_effect(pass: &PassNode) -> bool {
    crate::process::effect_kind::is_wasm_webgpu_compatible_effect(&pass.effect)
}

#[allow(dead_code)]
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
    fn mask_follow_reports_scene_gpu_info() {
        let report = inspect_gpu_compatibility(
            r##"
<Graph fps={30} duration="1s" size={[320,180]}>
  <Scene id="demo_scene">
    <Defs>
      <Mask id="spot" follow="node:target" shape="circle" x="0" y="0" radius="32" />
    </Defs>
    <Timeline>
      <Track>
        <Sequence duration="1s">
          <Layer>
            <Circle id="target" x="160" y="90" radius="4" color="#FFFFFF" opacity="0" />
            <Group mask="spot">
              <Rect x="0" y="0" width="320" height="180" color="#FF0000" />
            </Group>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="demo_scene" />
</Graph>
"##,
        )
        .expect("compatibility report");

        assert!(report.can_use_native_scene_preview);
        assert!(report.issues.iter().any(|issue| {
            issue.target == GpuCompatibilityTarget::NativeScenePreview
                && issue.severity == GpuCompatibilitySeverity::Info
                && issue.code == "mask_follow"
        }));
    }

    #[test]
    fn mixed_scene_process_with_gpu_native_passes_is_compatible() {
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

        // P4: GPU-native mixed graphs are no longer blocking.
        assert!(!report.likely_cpu_fallback);
        assert!(report.can_use_wasm_scene_canvas);
        assert!(report.can_use_native_scene_preview);
        // The glow_bloom effect is now recognized via alias (P1), so the WASM
        // process path itself is compatible.
        assert!(report.can_use_wasm_process_webgpu);
        assert!(report.issues.iter().any(|issue| {
            issue.target == GpuCompatibilityTarget::NativeScenePreview
                && issue.severity == GpuCompatibilitySeverity::Info
                && issue.code == "mixed_scene_process"
        }));
        assert!(report.issues.iter().any(|issue| {
            issue.target == GpuCompatibilityTarget::WasmSceneCanvas
                && issue.severity == GpuCompatibilitySeverity::Info
                && issue.code == "mixed_scene_process"
        }));
    }

    #[test]
    fn scene_plus_bloom_process_is_gpu_compatible() {
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
    <Pass id="bloom" kind="compute" effect="bloom"
          in={["scene_src"]} out={["out"]}
          params={{ intensity: "1.0" }} />
  </Process>
  <Present from="post" />
</Graph>
"##,
        )
        .expect("compatibility report");

        // The bloom alias is recognized, so WASM process itself is compatible.
        assert!(report.can_use_wasm_process_webgpu);
        // P4: Native scene preview is now compatible for GPU-native mixed graphs.
        assert!(report.can_use_native_scene_preview);
        assert!(!report.likely_cpu_fallback);
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

    #[test]
    fn cinematic_light_effects_are_wasm_process_webgpu_compatible() {
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
    <Tex id="glow_src" fmt="rgba16f" size={[320,180]} />
    <Tex id="sweep_src" fmt="rgba16f" size={[320,180]} />
    <Tex id="out" fmt="rgba16f" size={[320,180]} />
    <Pass id="glow" kind="compute" effect="glow_stack"
          in={["scene_src"]} out={["glow_src"]}
          params={{ threshold: "0.62", intensity: "1.4", radiusSmall: "6", radiusMedium: "18", radiusLarge: "48", tint: "#A5F3FC" }} />
    <Pass id="sweep" kind="compute" effect="light_sweep"
          in={["glow_src"]} out={["sweep_src"]}
          params={{ position: "0.35", angle: "-18", width: "0.16", softness: "0.08", intensity: "1.2", color: "#FFFFFF" }} />
    <Pass id="tone" kind="compute" effect="tone_map"
          in={["sweep_src"]} out={["out"]}
          params={{ exposure: "0.15", contrast: "1.1", shoulder: "0.85", gamma: "2.2", saturation: "1.04" }} />
  </Process>
  <Present from="post" />
</Graph>
"##,
        )
        .expect("compatibility report");

        assert!(report.can_use_wasm_process_webgpu);
        assert!(report.can_use_native_scene_preview);
        assert!(!report.likely_cpu_fallback);
    }

    #[test]
    fn simple_shapes_are_gpu_compatible() {
        let report = inspect_gpu_compatibility(
            r##"
<Graph fps={30} duration="1s" size={[320,180]}>
  <Scene id="demo">
    <Timeline>
      <Track>
        <Sequence duration="1s">
          <Rect x="10" y="10" width="100" height="80" color="#FF0000" />
          <Circle x="160" y="90" radius="40" color="#FFFFFF" />
          <Text value="Hello" x="50" y="50" fontSize="24" color="#000000" />
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene:demo" />
</Graph>
"##,
        )
        .expect("compatibility report");

        assert!(report.can_use_native_scene_preview);
        assert!(report.can_use_wasm_scene_canvas);
        assert!(!report.likely_cpu_fallback);
    }

    #[test]
    fn path_with_non_gpu_blend_blocks_gpu() {
        let report = inspect_gpu_compatibility(
            r##"
<Graph fps={30} duration="1s" size={[320,180]}>
  <Scene id="demo">
    <Timeline>
      <Track>
        <Sequence duration="1s">
          <Path d="M 0 0 L 100 100" stroke="#FF0000" strokeWidth="2" blend="hue" />
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene:demo" />
</Graph>
"##,
        )
        .expect("compatibility report");

        assert!(!report.can_use_native_scene_preview);
        assert!(!report.can_use_wasm_scene_canvas);
        assert!(report.likely_cpu_fallback);
        assert!(report.issues.iter().any(|issue| {
            issue.code == "non_gpu_blend" && issue.severity == GpuCompatibilitySeverity::Blocking
        }));
    }

    #[test]
    fn gradient_fill_reports_info() {
        let report = inspect_gpu_compatibility(
            r##"
<Graph fps={30} duration="1s" size={[320,180]}>
  <Scene id="demo">
    <Defs>
      <LinearGradient id="g" x1="0" y1="0" x2="1" y2="1"
                      stops="0:#FF0000, 1:#00FF00" />
    </Defs>
    <Timeline>
      <Track>
        <Sequence duration="1s">
          <Rect x="10" y="10" width="100" height="80" fill="url(#g)" />
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="scene:demo" />
</Graph>
"##,
        )
        .expect("compatibility report");

        assert!(report.can_use_native_scene_preview);
        assert!(report.issues.iter().any(|issue| {
            issue.code == "gradient" && issue.severity == GpuCompatibilitySeverity::Info
        }));
    }

    #[test]
    fn mixed_scene_process_with_scene_gpu_blocker_is_not_compatible() {
        let report = inspect_gpu_compatibility(
            r##"
<Graph fps={30} duration="1s" size={[320,180]}>
  <Scene id="demo">
    <Timeline>
      <Track>
        <Sequence duration="1s">
          <Path d="M 0 0 L 100 100" stroke="#FF0000" strokeWidth="2" blend="hue" />
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Process id="post">
    <Tex id="scene_src" fmt="rgba16f" from="scene:demo" />
    <Tex id="out" fmt="rgba16f" size={[320,180]} />
    <Pass id="bloom" kind="compute" effect="bloom"
          in={["scene_src"]} out={["out"]}
          params={{ intensity: "1.0" }} />
  </Process>
  <Present from="post" />
</Graph>
"##,
        )
        .expect("compatibility report");

        assert!(!report.can_use_native_scene_preview);
        assert!(!report.can_use_wasm_scene_canvas);
        assert!(report.likely_cpu_fallback);
        assert!(report.issues.iter().any(|issue| {
            issue.code == "non_gpu_blend" && issue.severity == GpuCompatibilitySeverity::Blocking
        }));
    }
}
