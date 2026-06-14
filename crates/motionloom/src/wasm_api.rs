// =========================================
// =========================================
// crates/motionloom/src/wasm_api.rs

use wasm_bindgen::prelude::*;

use std::sync::Arc;

use crate::asset::MemoryAssetResolver;
use crate::dsl::{GraphScript, is_graph_script, parse_graph_script};
use crate::process::render_process_frame_cpu;
use crate::scene::render::{SceneRenderProfile, SceneRenderer, render_scene_graph_frame};
use crate::world::{WorldFrameRenderer, is_world_graph_script, parse_world_graph_script};

fn js_error(message: String) -> JsValue {
    js_sys::Error::new(&message).into()
}

/// Parse a MotionLoom script and return a short diagnostic summary.
///
/// Returns an error string if parsing fails.
#[wasm_bindgen]
pub fn motionloom_parse_summary(script: &str) -> Result<String, JsValue> {
    if is_graph_script(script) {
        let graph = parse_graph_script(script).map_err(|err| js_error(err.to_string()))?;
        return Ok(format!(
            "scene graph: {} scene node(s), {} frame(s)",
            graph.scene_nodes.len(),
            graph.duration_ms
        ));
    }
    if is_world_graph_script(script) {
        let graph = parse_world_graph_script(script).map_err(|err| js_error(err.to_string()))?;
        return Ok(format!(
            "world graph: {} world node(s), {} frame(s)",
            graph.worlds.len(),
            graph.duration_ms
        ));
    }
    Err(js_error(
        "script does not look like a scene or world graph".to_string(),
    ))
}

/// Render one frame of a scene graph script to an RGBA byte buffer.
///
/// The returned `Vec<u8>` is row-major RGBA with dimensions `(width, height)`.
/// Hosts can wrap it in `Uint8Array` / `ImageData`.
///
/// This convenience function uses the default path-based asset resolver and
/// tries the GPU profile, falling back to CPU if GPU initialization fails.
/// To supply in-memory assets use `WasmSceneRenderer`.
#[wasm_bindgen]
pub async fn motionloom_render_scene_frame(
    script: &str,
    frame: u32,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, JsValue> {
    motionloom_render_scene_frame_with_profile(script, frame, width, height, "gpu-cpu").await
}

/// Render one frame with an explicit render profile.
///
/// `profile` accepts: `"cpu"`, `"gpu"`, `"gpu-cpu"` (try GPU, fallback to CPU).
#[wasm_bindgen]
pub async fn motionloom_render_scene_frame_with_profile(
    script: &str,
    frame: u32,
    width: u32,
    height: u32,
    profile: &str,
) -> Result<Vec<u8>, JsValue> {
    let mut graph = parse_graph_script(script).map_err(|err| js_error(err.to_string()))?;
    graph.size.0 = width.max(1);
    graph.size.1 = height.max(1);

    let (preferred, fallback) = parse_scene_profile_with_fallback(profile);
    let mut last_err = None;

    for profile in [Some(preferred), fallback].into_iter().flatten() {
        match render_scene_graph_frame(&graph, frame, profile).await {
            Ok(image) => return Ok(image.into_raw()),
            Err(err) => last_err = Some(err),
        }
    }

    Err(js_error(
        last_err
            .map(|err| err.to_string())
            .unwrap_or_else(|| "scene render failed".to_string()),
    ))
}

/// Render one frame of a process graph over an RGBA source buffer.
#[wasm_bindgen]
pub fn motionloom_render_process_frame(
    script: &str,
    frame: u32,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<Vec<u8>, JsValue> {
    render_process_frame_cpu(script, frame, width, height, rgba)
        .map(|image| image.into_raw())
        .map_err(|err| js_error(err.to_string()))
}

fn parse_scene_profile_with_fallback(
    profile: &str,
) -> (SceneRenderProfile, Option<SceneRenderProfile>) {
    match profile.to_ascii_lowercase().as_str() {
        "cpu" => (SceneRenderProfile::Cpu, None),
        "gpu" => (SceneRenderProfile::Gpu, None),
        "gpu-cpu" => (SceneRenderProfile::Gpu, Some(SceneRenderProfile::Cpu)),
        _ => (SceneRenderProfile::Gpu, Some(SceneRenderProfile::Cpu)),
    }
}

/// Render one frame of a world graph script to an RGBA byte buffer.
///
/// This convenience function uses the default path-based asset resolver.
/// To supply in-memory assets use `WasmWorldRenderer`.
#[wasm_bindgen]
pub fn motionloom_render_world_frame(
    script: &str,
    frame: u32,
    asset_root: &str,
) -> Result<Vec<u8>, JsValue> {
    let graph = parse_world_graph_script(script).map_err(|err| js_error(err.to_string()))?;
    let mut renderer = WorldFrameRenderer::new();
    let image = renderer
        .render_frame(&graph, frame, asset_root)
        .map_err(|err| js_error(err.to_string()))?;
    Ok(image.into_raw())
}

/// Inspect a script and return the document type as a string.
#[wasm_bindgen]
pub fn motionloom_document_type(script: &str) -> String {
    if is_graph_script(script) {
        "scene".to_string()
    } else if is_world_graph_script(script) {
        "world".to_string()
    } else {
        "unknown".to_string()
    }
}

/// WASM-facing wrapper around a parsed scene graph. Keeps the parsed script
/// alive across JS calls so that repeated frame renders avoid re-parsing.
///
/// Each renderer owns its own `MemoryAssetResolver`; assets added to one
/// renderer do not affect any other renderer or the global state.
#[wasm_bindgen]
pub struct WasmSceneRenderer {
    graph: GraphScript,
    profile: SceneRenderProfile,
    resolver: Arc<MemoryAssetResolver>,
}

#[wasm_bindgen]
impl WasmSceneRenderer {
    /// Parse `script` and prepare a renderer.
    #[wasm_bindgen(constructor)]
    pub fn new(script: &str, profile: &str) -> Result<WasmSceneRenderer, JsValue> {
        let graph = parse_graph_script(script).map_err(|err| js_error(err.to_string()))?;
        let profile = parse_profile(profile)?;
        Ok(Self {
            graph,
            profile,
            resolver: Arc::new(MemoryAssetResolver::new()),
        })
    }

    /// Register an in-memory asset for this renderer only.
    ///
    /// The `name` should match the `src` attribute used in `<Image>` or `<Svg>`
    /// nodes (e.g. `"logo.png"`). The `bytes` argument is the raw file content.
    pub fn add_asset(&mut self, name: &str, bytes: &[u8]) {
        self.resolver.insert(name.to_string(), bytes.to_vec());
    }

    /// Clear all assets previously registered on this renderer.
    pub fn clear_assets(&mut self) {
        self.resolver.clear();
    }

    /// Render `frame` to an RGBA byte buffer.
    pub async fn render_frame(&mut self, frame: u32) -> Result<Vec<u8>, JsValue> {
        let mut renderer = SceneRenderer::with_resolver(self.profile, self.resolver.clone())
            .await
            .map_err(|err| js_error(err.to_string()))?;
        let image = renderer
            .render_frame(&self.graph, frame)
            .await
            .map_err(|err| js_error(err.to_string()))?;
        Ok(image.into_raw())
    }

    /// Total number of frames for the graph's duration and fps.
    #[wasm_bindgen(getter)]
    pub fn total_frames(&self) -> u32 {
        let fps = self.graph.fps.max(1.0);
        let duration_sec = (self.graph.duration_ms as f32 / 1000.0).max(1.0 / fps);
        (duration_sec * fps).round() as u32
    }
}

/// WASM-facing wrapper around a parsed world graph with renderer-owned assets.
#[wasm_bindgen]
pub struct WasmWorldRenderer {
    graph: crate::world::WorldGraph,
    resolver: Arc<MemoryAssetResolver>,
}

#[wasm_bindgen]
impl WasmWorldRenderer {
    /// Parse `script` and prepare a renderer.
    #[wasm_bindgen(constructor)]
    pub fn new(script: &str) -> Result<WasmWorldRenderer, JsValue> {
        let graph = parse_world_graph_script(script).map_err(|err| js_error(err.to_string()))?;
        Ok(Self {
            graph,
            resolver: Arc::new(MemoryAssetResolver::new()),
        })
    }

    /// Register an in-memory asset for this renderer only.
    pub fn add_asset(&mut self, name: &str, bytes: &[u8]) {
        self.resolver.insert(name.to_string(), bytes.to_vec());
    }

    /// Clear all assets previously registered on this renderer.
    pub fn clear_assets(&mut self) {
        self.resolver.clear();
    }

    /// Render `frame` to an RGBA byte buffer using the provided asset root for
    /// relative-path fallback.
    pub fn render_frame(&mut self, frame: u32, asset_root: &str) -> Result<Vec<u8>, JsValue> {
        let mut renderer = WorldFrameRenderer::with_resolver(self.resolver.clone());
        let image = renderer
            .render_frame(&self.graph, frame, asset_root)
            .map_err(|err| js_error(err.to_string()))?;
        Ok(image.into_raw())
    }
}

fn parse_profile(profile: &str) -> Result<SceneRenderProfile, JsValue> {
    match profile.to_ascii_lowercase().as_str() {
        "cpu" => Ok(SceneRenderProfile::Cpu),
        "gpu" => Ok(SceneRenderProfile::Gpu),
        _ => Err(js_error(format!("unknown render profile: {profile}"))),
    }
}
