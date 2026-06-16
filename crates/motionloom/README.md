# MotionLoom

MotionLoom is the DSL parser and renderer crate used by Anica for video effects,
scene graphs, motion graphics, and world graphs.

It is designed to be used as a Rust library. Anica can expose MotionLoom through
application tools such as `anica.motionloom/render_scene`, while this crate
provides the lower-level API for parsing, rendering, runtime evaluation, process
catalog lookup, and GLB inspection.

## Core Functions

### DSL detection and parsing

`is_graph_script(input: &str) -> bool`

Checks whether a string looks like a MotionLoom graph script.

`parse_graph_script(input: &str) -> Result<GraphScript, GraphParseError>`

Parses a scene/effect/composition graph. Use this for `<Scene>`, `<Tex>`,
`<Pass>`, `<Layer>`, `<Precompose>`, `<Mask>`, and most motion graphics DSL.

`is_world_graph_script(input: &str) -> bool`

Checks whether a string should be handled by the world graph parser.

`parse_world_graph_script(input: &str) -> Result<WorldGraph, GraphParseError>`

Parses a `<World>` graph. Use this for GLB actors, camera/world
world, directional sprites, retarget maps, and skeletal actions.

## Present Rule

Every MotionLoom graph must contain exactly one `<Present ... />` node.

`<Present ... />` must be a direct child of `<Graph>` and must be the final node
before `</Graph>`. It cannot be nested inside `<Scene>`, `<World>`, or
`<Process>`.

For process graphs, use root-level present from the process id:

```xml
<Process id="FinalProcess">
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
</Process>

<Present from="FinalProcess" />
```

## Scene Rendering

`render_scene_frame(graph: &GraphScript, frame: u32, profile: SceneRenderProfile)`

Renders one scene/composition frame to an `image::RgbaImage`.

`render_scene_graph_frame(graph: &GraphScript, frame: u32, profile: SceneRenderProfile)`

Same frame-rendering entrypoint with the full graph-oriented name.

`SceneRenderer::new(profile: SceneRenderProfile)`

Creates a reusable scene renderer. Prefer this when rendering many frames,
because internal caches can be reused across frames.

`render_scene_graph_to_video_with_progress(ffmpeg_bin, graph, output_path, profile, progress_every_frames, callback)`

Renders a full scene/composition graph to a video file through ffmpeg and reports
progress.

`next_scene_output_path(output_dir)` and `next_scene_output_path_for_profile(output_dir, profile)`

Build timestamped output paths for scene renders.

`SceneRenderProfile`

Selects the renderer/output path:

- `SceneRenderProfile::Cpu`
- `SceneRenderProfile::Gpu`
- `SceneRenderProfile::GpuProRes`

Scene `zDepth` uses camera-space depth: negative is closer, positive is farther.

## Preview Surfaces

`SceneRenderer::render_frame_to_preview_surface(graph, frame, options)`

Renders a scene frame to the fastest preview surface available on the current
platform. This is the intended integration point for host applications such as
Anica that want to display live previews without choosing between CPU, GPU, and
platform interop code themselves.

`ScenePreviewSurfaceOptions::default()` uses `ScenePreviewBackend::Auto`, which
picks a platform surface when available, otherwise falls back to a wgpu texture
or CPU BGRA bytes.

Supported backends:

- `ScenePreviewBackend::Auto` — prefer platform surface, then wgpu texture, then
  CPU BGRA.
- `ScenePreviewBackend::WgpuTexture` — return a `SceneGpuTexture` wrapping an
  `Arc<wgpu::Texture>` in `Rgba8Unorm`.
- `ScenePreviewBackend::PlatformSurface` — return a platform display surface.
- `ScenePreviewBackend::CpuBgra` — return CPU BGRA bytes for compatibility.

Platform surfaces are host-consumable descriptors. MotionLoom produces the
surface; it is the downstream application's responsibility to import and paint
it (for example through GPUI, DirectComposition, Wayland, or Metal).

- **macOS** — `ScenePlatformPreviewSurface::MacOs { surface: CVPixelBuffer, ... }`
  in `Bgra8Unorm`. The pixel buffer is Metal-compatible.
- **Windows** — `ScenePlatformPreviewSurface::WindowsD3D(WindowsD3DSharedSurface)`
  in `Bgra8Unorm`. The contained `WindowsD3DSharedSurface` keeps the
  `ID3D11Texture2D` alive so the legacy DXGI shared handle remains valid. The
  host can open `shared_handle` on another D3D10/11 device on the same adapter;
  the handle is owned by the OS and does not need to be closed by the caller.
- **Linux** — `ScenePlatformPreviewSurface::LinuxDmabuf { ... }` is the planned
  DMA-BUF BGRA descriptor. As long as real fd/export is not implemented,
  `PlatformSurface` returns a clear error and `Auto` falls back to `CpuBgra`.

```rust
use motionloom::{
    ScenePreviewBackend, ScenePreviewSurface, ScenePreviewSurfaceOptions, SceneRenderer,
    SceneRenderProfile, parse_graph_script,
};

let graph = parse_graph_script(script)?;
let mut renderer = pollster::block_on(SceneRenderer::new(SceneRenderProfile::Gpu))?;
let surface = pollster::block_on(renderer.render_frame_to_preview_surface(
    &graph,
    0,
    ScenePreviewSurfaceOptions::default(),
))?;

match surface {
    ScenePreviewSurface::PlatformSurface(platform) => {
        // Hand the platform surface off to the host compositor/GPUI.
    }
    ScenePreviewSurface::WgpuTexture(tex) => {
        // Consume tex.texture as a wgpu texture.
    }
    ScenePreviewSurface::CpuBgra { width, height, data, .. } => {
        // Upload data (width x height BGRA bytes) to the UI.
    }
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

## World Rendering

`render_world_frame(graph: &WorldGraph, frame: u32, asset_root)`

Renders one world frame using the compatibility world renderer.

`WorldFrameRenderer::new()`

Creates a reusable world renderer with image, GLB mesh, and GPU caches.

`WorldFrameRenderer::render_frame(graph, frame, asset_root)`

Renders an world frame on the CPU/debug path.

`WorldFrameRenderer::render_frame_gpu(graph, frame, asset_root)`

Renders an world frame using the GPU actor path where available.

`WorldFrameRenderer::render_frame_gpu_with_ground_grid(...)`

Renders GPU world with a ground grid overlay for viewport/debug use.

`render_world_graph_to_video_with_progress(ffmpeg_bin, graph, asset_root, output_path, profile, progress_every_frames, callback)`

Renders an world graph to video through ffmpeg and reports progress.

## Runtime Evaluation

`compile_runtime_program(graph: GraphScript) -> Result<RuntimeProgram, RuntimeCompileError>`

Compiles a graph into a timeline/effect runtime program.

`RuntimeProgram::evaluate_frame(frame: u32) -> RuntimeFrameOutput`

Evaluates effect parameters for a frame.

`RuntimeProgram::evaluate_at_time_sec(time_norm: f32, time_sec: f32) -> RuntimeFrameOutput`

Evaluates effect parameters at an explicit normalized time and second value.

`eval_time_expr(value, time_norm, time_sec)`

Evaluates MotionLoom time expressions such as `$time.sec`, `curve(...)`, and
math expressions used in effect parameters.

## GPU Compatibility Inspector

`inspect_gpu_compatibility(script: &str) -> Result<GpuCompatibilityReport, GraphParseError>`

Inspects a MotionLoom script and reports whether it is likely to run on the
strict GPU preview paths or fall back to CPU. This is a static diagnostic tool:
it parses and analyzes the DSL, but it does not render frames and does not
allocate GPU resources.

Use this before choosing a preview/render path in host applications:

```rust
use motionloom::{GpuCompatibilitySeverity, inspect_gpu_compatibility};

let report = inspect_gpu_compatibility(script)?;

if report.likely_cpu_fallback {
    for issue in report.blocking_issues() {
        eprintln!("[{:?}] {}: {}", issue.target, issue.code, issue.message);
    }
}

// `likely_preview_path` predicts what `ScenePreviewBackend::Auto` will pick
// on the current platform: `MacOsCVPixelBuffer`, `WindowsD3D`, `WgpuTexture`,
// or `CpuBgra`. (Linux DMA-BUF is planned but reports `CpuBgra` for now.)
match report.likely_preview_path {
    _ => {}
}

if report.can_use_wasm_scene_canvas {
    // Browser/WASM can try the direct WebGPU scene canvas path.
} else if report.can_use_wasm_process_webgpu {
    // Browser/WASM can try the process WebGPU path for supported process effects.
} else {
    // Use CPU/WASM fallback or a compatibility renderer.
}

# let _ = GpuCompatibilitySeverity::Blocking;
# Ok::<(), Box<dyn std::error::Error>>(())
```

The report is intentionally conservative. It only flags known limitations, so a
script that passes inspection can still fail at runtime because of the user's
GPU, driver, browser, missing assets, or platform-specific surface integration.

Current report targets:

- `NativeScenePreview` — Anica/native live scene preview.
- `WasmSceneCanvas` — browser direct WebGPU scene-to-canvas render.
- `WasmProcessWebGpu` — browser WebGPU process/effect render.
- `WgpuTextureOutput` — `SceneRenderer::render_frame_to_wgpu_texture`.

Common CPU-fallback reasons:

- Mixed `<Scene>` + `<Process>` graphs need scene-to-process composition.
- `Tex` / `Pass` / `Output` composition is not supported by the strict direct
  scene canvas path yet.
- `Tex from="scene:..."` requires scene output to become a process input.
- Some process effects are not implemented in the WASM process WebGPU path yet.

Important distinction: Anica/native and WASM/browser do not have identical GPU
paths. A script can be GPU-compatible in one target and CPU fallback in another.
Use the per-target booleans and issue list instead of assuming one target's
result applies to every platform.

## Process Catalog

`process_effects() -> &'static [ProcessEffectDefinition]`

Returns the built-in effect/process catalog.

`process_effect_for_id(id: &str)`

Looks up one process effect by ID.

`process_effects_for_category(category)`

Lists process effects by category.

`kernel_source_by_name(kernel: &str)`

Returns embedded WGSL source for a known process kernel.

`is_known_process_kernel(kernel: &str) -> bool`

Checks whether a kernel name is built into the crate.

## GLB Helpers

`load_glb_metadata(path)` and `parse_glb_metadata(path, bytes)`

Load or parse lightweight GLB metadata such as nodes, meshes, joints, and
materials.

`load_glb_mesh_data(path)` and `parse_glb_mesh_data(path, bytes)`

Load or parse GLB mesh data for world rendering and diagnostics.

`diagnose_world_glb_gpu_plan(mesh)`

Builds a diagnostic report for whether a GLB mesh is suitable for the GPU
world path.

`diagnose_world_graph_actor_gpu_frame(graph, actor_id, frame, asset_root)`

Diagnoses one actor in an world graph at a specific frame.

## Minimal Scene Example

```rust
use motionloom::{SceneRenderProfile, parse_graph_script, render_scene_frame};

let script = r##"
<Graph fps={30} duration="1s" size={[640,360]}>
  <Background color="#101827" />
  <Scene id="example_scene">
    <Circle x="320" y="180" radius="96" color="#4cc9f0" />
    <Text x="320" y="306" value="MotionLoom" fontSize="34" color="#f7f7f7" />
  </Scene>
  <Present from="example_scene" />
</Graph>
"##;

let graph = parse_graph_script(script)?;
let frame = pollster::block_on(render_scene_frame(&graph, 0, SceneRenderProfile::Gpu))
    .or_else(|_| pollster::block_on(render_scene_frame(&graph, 0, SceneRenderProfile::Cpu)))?;
frame.save("motionloom_frame.png")?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Minimal World Example

```rust
use motionloom::{WorldFrameRenderer, parse_world_graph_script};

let script = r##"
<Graph fps={30} duration="1s" size={[320,180]}>
  <World id="world">
    <Background color="#ffffff" />
    <Camera yaw="0" pitch="0" zoom="1" />
  </World>
  <Present from="world" />
</Graph>
"##;

let graph = parse_world_graph_script(script)?;
let mut renderer = WorldFrameRenderer::new();
let frame = renderer.render_frame(&graph, 0, ".")?;
frame.save("motionloom_world_frame.png")?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Live Preview Example

Run the built-in `wgpu_live_preview` example to render a MotionLoom script
through the GPU preview pipeline and print timing statistics:

```bash
cargo run --release -p motionloom --example wgpu_live_preview -- --print-stats ../motionloom-example/showcase/s-000005/main.motionloom
```

This command:

1. Builds the `wgpu_live_preview` example in release mode for fast GPU rendering.
2. Parses the specified `.motionloom` script.
3. Renders the scene through `SceneRenderer::new(SceneRenderProfile::Gpu)`.
4. Prints frame-timing statistics (`--print-stats`) so you can see how long
each frame takes on the GPU compositor.

Use this example to quickly verify that a script renders correctly on the GPU
path without writing a full host application. Replace the path with any
`.motionloom` file you want to test.

## Notes

Scene graph APIs are for 2D scene/motion graphics/effect graphs. World graph
APIs are for world/camera/actor/directional-character rendering.

GPU rendering requires a working `wgpu` backend on the host machine. For tools
that need robust fallback behavior, try `SceneRenderProfile::Gpu` first and fall
back to `SceneRenderProfile::Cpu` when GPU initialization fails.

Video rendering functions require an ffmpeg binary path supplied by the caller.
