# MotionLoom

MotionLoom is the DSL parser and renderer crate used by Anica for video effects,
scene graphs, motion graphics, and world graphs.

It is designed to be used as a Rust library. Anica can expose MotionLoom through
application tools such as `anica.motionloom/render_scene`, while this crate
provides the lower-level API for parsing, rendering, runtime evaluation, process
catalog lookup, and GLB inspection.

## Install

```toml
[dependencies]
motionloom = "0.1"
```

MotionLoom requires Rust 1.85 or newer. WebGPU/wgpu is part of the core
renderer path, so the crate intentionally depends on `wgpu` by default.

Video export requires an FFmpeg binary path supplied by the caller. MotionLoom
does not bundle FFmpeg. Single-frame rendering and PNG sequence export do not
require FFmpeg.

## Public API

New Rust integrations should start with `motionloom::api` or
`motionloom::prelude`.

- `motionloom::api` is the recommended stable integration surface.
- `motionloom::prelude` contains a small convenience import set for common use.
- `motionloom::experimental` exposes advanced editor, world, GLB, text layout,
  and timeline helpers. These APIs are public, but their stability is lower than
  `motionloom::api`.
- The crate root keeps broader re-exports for short-term compatibility with
  existing applications. Prefer `motionloom::api` in new code.

See `PUBLIC_API.md` for the curated main API map.

## Release Notes

See `CHANGELOG.md` for crate release history.

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

`render_scene_graph_frame(graph: &GraphScript, frame: u32, profile: SceneRenderProfile)`

Renders one scene/composition graph frame to an `image::RgbaImage`.

`SceneRenderer::new(profile: SceneRenderProfile)`

Creates a reusable scene renderer. Prefer this when rendering many frames,
because internal caches can be reused across frames.

`render_scene_graph_to_video_with_progress(ffmpeg_bin, graph, output_path, profile, progress_every_frames, callback)`

Renders a full scene/composition graph to a video file through ffmpeg and reports
progress.

`render_scene_graph_to_png_sequence_with_progress(graph, output_dir, progress_every_frames, callback)`

Renders a scene/composition graph to a PNG image sequence. In this mode
`output_dir` is treated as an output directory and frames are written as
`frame_000000.png`, `frame_000001.png`, and so on. FFmpeg is not used for PNG
sequence export.

`next_scene_output_path(output_dir)` and `next_scene_output_path_for_profile(output_dir, profile)`

Build timestamped output paths for scene renders.

`SceneRenderProfile`

Selects the renderer/output path:

- `SceneRenderProfile::Cpu` — CPU render, ProRes MOV export through FFmpeg.
- `SceneRenderProfile::Gpu` — GPU scene compositor, H.264/MP4 export through
  FFmpeg.
- `SceneRenderProfile::GpuProRes` — GPU scene compositor, ProRes MOV export
  through FFmpeg.
- `SceneRenderProfile::GpuProRes4444` — GPU scene compositor, ProRes 4444 MOV
  export through FFmpeg.
- `SceneRenderProfile::GpuPngSequence` — GPU scene compositor, PNG frame
  sequence export without FFmpeg.

Export support:

| Output | API | FFmpeg required |
| --- | --- | --- |
| Single frame PNG/image | `render_scene_graph_frame` then save the returned `RgbaImage` | No |
| PNG sequence | `render_scene_graph_to_png_sequence_with_progress` | No |
| MP4/MOV video | `render_scene_graph_to_video_with_progress` with `Cpu`, `Gpu`, `GpuProRes`, or `GpuProRes4444` | Yes |
| Root document PNG sequence | `render_motionloom_document_to_png_sequence_with_progress` | No |
| Root document video | `render_motionloom_document_to_video_with_progress` | Yes |

Process graphs that use external timeline inputs such as
`<Input id="clip0" from="input:clip0" />` are intended for host applications
such as Layer FX. Standalone MotionLoom export cannot render those process-only
graphs unless the process wraps a self-contained `<Scene>` or `<World>` source.

Scene `zDepth` uses camera-space depth: negative is closer, positive is farther.

## Scene DSL Feature Map For Editors And LLMs

The scene DSL now has a set of editor-oriented controls intended for Anica VFX
Studio and LLM-generated examples:

For UI property panels, use the ACP schema at
`docs/acp/motionloom/scene-ui-schema.json`. It is the machine-readable source
for Scene Camera and Layer3D labels, groups, property types, and animatability.

| Need | DSL | Notes |
| --- | --- | --- |
| Compact numeric animation | `curve("time:value:ease, ...")` | Use for hand-authored numeric properties. |
| UI/time keyframes | `<AnimationTarget>` + `<Key>` | Direct graph children targeting a node id and property. |
| Editable path shape keys | `AnimationTarget property="d"` | Use path-string keys; do not use `curve(...)` for `Path.d`. |
| Puppet deformation | `<PuppetWarp>` + `<PuppetPin>` | Target-based editor syntax; `<Puppet>` + `<Pin>` remains supported. |
| Manual topology | `<MeshTopology>` + `<Vertex>` + `<Triangle>` | Optional expert path when auto mesh is not enough. |
| Character rigs | `<Skeleton>`, `<Action>`, `<Character>`, `<Part>`, `<ApplyAction>` | Bone-attached scene artwork. |
| IK controls | `<IK root mid end ...>` or `<IK chain="..." ...>` | Two-bone and CCD-chain solvers. |
| Scene camera motion | `<Track role="camera"><Sequence><Camera ... /></Sequence></Track>` | 2D pan/zoom/rotation/follow camera; do not use `mode`. |
| 2.5D scene cards | `<Layer3D>` | Flat card/panel depth and tilt, not true 3D mesh rendering. |
| Moving masks | `<Mask follow="node:id">` | Follow/reveal style masks. |
| Procedural face outline | `<FaceJaw>` | Parameterized jaw/face curve generation. |

### AnimationTarget

`AnimationTarget` is a keyframe layer for editor and UI workflows.
It is evaluated before rendering and reuses the existing transform, opacity,
path morph, puppet, and skeleton pipelines.

Editor integrations can use `extract_editable_animation_timeline`,
`replace_editable_animation_targets`, and `upsert_editable_animation_target`
to read UI-editable keys from `.motionloom` text and write them back as
parser-validated DSL.

Supported properties are `x`, `y`, `rotation`, `scale`, `scaleX`, `scaleY`, `skewX`, `skewY`, `transformOriginX`, `transformOriginY`, `opacity`, and `d`.
All properties except `d` require numeric key values. `property="d"` accepts SVG path
strings for editable path morph keys. Each `<Key>` must use exactly one timing
attribute: `time="1.5s"`/`time="500ms"` or `frame="45"`. Internally both forms
are sampled in seconds; `frame` is converted through the graph `fps`. Writers
preserve whichever form the input used, while new UI keys should prefer `time`
so edits survive FPS changes. `Key.ease` uses the same easing parser as
`curve(...)`: `linear`, `ease_in`, `ease_out`, `ease_in_out`, or
`ease(x1,y1,x2,y2)`.

```xml
<AnimationTarget node="card" property="rotation">
  <Key time="0s" value="0" ease="linear" />
  <Key time="0.5s" value="18" ease="ease_in_out" />
  <Key time="1s" value="0" ease="ease_out" />
</AnimationTarget>
```

For a UI edit, build an `EditableAnimationTarget` for one `node/property`
channel and call `upsert_editable_animation_target(script, target)`. For a full
timeline save, call `replace_editable_animation_targets(script, targets)`.

For Puppet pins, `AnimationTarget node="pin_id" property="x/y"` animates the
pin target position. The pin's own `x/y` are the rest/source anchor.

### Puppet, Pin, and MeshTopology

`Puppet` defaults to auto mesh. Keep visual children and pins in the same local
coordinate system as `width` and `height` (`0..width`, `0..height`), then move
the whole puppet with `Puppet x/y`. Use `density="high"` or a larger pin
`radius` for smoother deformation.

Use `MeshTopology` only for advanced cases that need explicit vertices,
triangles, edges, or regions. Pins can attach to topology vertices with
`vertex="vertex_id"`.

For DSL-first editor workflows, place `PuppetWarp` beside the target Group and
bind pins to semantic descendant ids. Moving a pin writes `targetX/targetY`;
the target artwork remains the single source of geometry:

```xml
<Group id="character">
  <Group id="left_hand" x="220" y="410">
    <Circle x="0" y="0" radius="30" color="#f2c9b8" />
  </Group>
</Group>
<PuppetWarp id="character_warp" target="character" width="1200" height="900">
  <PuppetPin id="left_hand_pin" bindTo="left_hand"
             targetX="250" targetY="390" radius="160" />
</PuppetWarp>
```

`PuppetWarp`/`PuppetPin` are aliases over the native Puppet renderer. A bound
pin uses its target node's local Scene anchor as the rest position. Free pins
can continue to use explicit `x/y`.

### Character Sources

`Character` can draw vector children and/or a raster source with `src`,
`image`, or `path`. Character raster loading uses the same loader as `<Image>`,
including PNG/JPG asset paths and `data:image/png;base64,...` or
`data:image/jpeg;base64,...` self-contained sources. SVG data URIs should use
`<Svg>`.

## Scene Character IK

Scene `<Action>` supports optional IK targets for 2D `<Skeleton>` rigs. Use it
when a chain endpoint must reach a target while keeping FK poses as the base
motion:

```xml
<Skeleton id="arm">
  <Bone id="upper" x="0" y="0" />
  <Bone id="lower" parent="upper" x="40" y="0" />
  <Bone id="hand" parent="lower" x="40" y="0" />
</Skeleton>

<Action id="reach" skeleton="arm" duration="1s">
  <IK root="upper" mid="lower" end="hand"
      targetX="40" targetY="40" bend="1" weight="1" />
</Action>
```

`root`, `mid`, and `end` must be a direct parented chain. `targetX` and
`targetY` use the skeleton's local coordinate space and may be numeric
expressions or `curve(...)` values. `bend` chooses the elbow/finger bend side
(`1` or `-1`), and `weight` blends between FK and IK.

For longer chains, use CCD IK with `chain`:

```xml
<IK chain="finger_1,finger_2,finger_3,finger_tip"
    targetX="24" targetY="-120" iterations="10" weight="1" />
```

## Profile-Driven 2D Skeletons

`<Skeleton>` can carry proportion, anatomy, silhouette, validation, drawing-guide,
and editor-control metadata without adding a wrapper tag. All additions are
optional, so existing bone-only rigs remain compatible.

```xml
<Skeleton id="hero_rig" profile="anime_6_head" height="720"
          facing="front" symmetryAxis="body_center"
          validation="strict" autoCorrect="proportions">
  <Bone id="root" role="root" x="0" y="0" />
  <Bone id="head" role="head" parent="root" x="0" y="-600" />
  <Landmark id="body_center" bone="root" offset={[0,0]} />
  <Landmark id="left_eye" bone="head" offset={[-28,-12]} />
  <Landmark id="right_eye" bone="head" offset={[28,-12]} />
  <Measure id="head_height" from="head_top" to="chin" />
  <Ratio measure="head_height" relativeTo="body_height" value="0.1667" />
  <Region id="head_volume" role="head" type="ellipse"
          center="face_center" radiusX="72" radiusY="88" />
  <Constraint type="symmetry" left="left_eye" right="right_eye"
              axis="body_center" />
  <Guide id="eye_horizontal" type="line" through="eye_line" angle="0" />
  <Control id="look_control" type="aim" targets={["left_eye","right_eye"]} />
</Skeleton>
```

Built-in profiles cover `chibi_2_head`, `chibi_3_head`, `anime_5_head`,
`anime_6_head`, `heroic_7_head`, `realistic_7_5_head`, and
`realistic_8_head`. Explicit Skeleton values remain the source of truth.

Hosts can call `GraphScript::skeleton_validation_reports()` for diagnostics,
`auto_correct_skeleton()` for deterministic symmetry correction, and
`build_skeleton_overlay()` to obtain editor-ready bone, landmark, region,
guide, and control primitives. Angle-limit constraints are also enforced after
FK/IK sampling.

`chain` ids must be direct parent-child bones. The last id is the end effector;
all earlier ids can rotate during the solve.

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

## Standalone WGPU Live Preview Example

`crates/motionloom/examples/wgpu_live_preview.rs` is a native diagnostic viewer
for testing MotionLoom's direct wgpu preview path without Anica, ffmpeg, video
encoding, or CPU readback. Use it to measure scene/process GPU render cost,
surface format behavior, and preview quality tradeoffs.

The example keeps CLI parsing, window creation, keyboard controls, and surface
presentation local to the example. Renderer lifecycle, graph quality scaling,
preview target texture allocation, parsed graph caching, and frame rendering are
shared through `motionloom::preview::WgpuPreviewEngine` plus
`WgpuPreviewGraphCache`, which are also suitable for embedded hosts such as
Anica. This keeps standalone preview and app preview on the same render path
without coupling host window/event-loop code to MotionLoom.

Run the live preview:

```bash
cargo run --release -p motionloom --example wgpu_live_preview -- ../motionloom-example/showcase/s-000005/main.motionloom
```

Print copyable timing stats once per second with `--print-stats` or `--stats`:

```bash
cargo run --release -p motionloom --example wgpu_live_preview -- --print-stats ../motionloom-example/showcase/s-000005/main.motionloom
```

Run the same viewer as an external preview host for editor controllers:

```bash
cargo run --release -p motionloom --example wgpu_live_preview -- --listen 127.0.0.1:49377
```

The host speaks newline-delimited JSON over local TCP using
`PreviewCommand`/`PreviewEvent`. It does not require FFmpeg; it keeps the same
persistent wgpu window and render loop as standalone preview mode. A path can be
passed after `--listen` for manual diagnostics, but normal Anica controller mode
starts with an internal placeholder scene and waits for `LoadScript`. In host
mode the viewer uses an always-on-top utility window; on macOS it also runs as
an accessory companion that can join all Spaces, so Mission Control should keep
it with the editor instead of treating it as a separate desktop app. Controllers
can send `SetWindowBounds` to make the host window borderless and align it over
their own preview panel, preserving the fast external render path while making
the viewer behave like an attached panel.

Anica can opt into this external host without changing its embedded preview by
setting:

```bash
ANICA_MOTIONLOOM_PREVIEW_HOST=127.0.0.1:49377 cargo run -p anica
```

When running Anica from the repository root package, `cargo run` auto-starts a
local MotionLoom wgpu preview host unless disabled:

```bash
cargo run
```

Use `ANICA_MOTIONLOOM_PREVIEW_HOST=off cargo run` to keep the helper closed.
Use `ANICA_MOTIONLOOM_PREVIEW_HOST_BIN=/path/to/wgpu_live_preview cargo run` to
force a packaged or prebuilt helper binary. If no helper binary is found during
development, Anica falls back to spawning the same example through Cargo.

When enabled, the VFX Studio controller sends `LoadScript`, `SetFrame`,
`SetQuality`, `SetAssetRoots`, `SetWindowBounds`, `SetWindowVisible`,
`SetInteractionTarget`, and `SetInteractionTargets`
commands to the external viewer while keeping the existing in-app preview path
available. Anica uses `SetWindowVisible` to hide the companion viewer when the
editor window is no longer active. The host can emit `WindowBounds` events for
attach diagnostics so controllers can compare requested and actual native
window sizes during calibration. For direct preview editing, the controller can
send editable node bounds plus graph bounds with `SetInteractionTargets`; the
host hit-tests the native wgpu surface and returns `PickResult`,
`TransformDrag`, and `TransformDragEnd` events while the user drags. The older
single-node `SetInteractionTarget` command remains available for minimal
controllers.

Keyboard controls:

- `1` — Full quality, 100% render target.
- `2` — Balanced quality, 50% render target.
- `3` — Speed quality, 25% render target.
- `4` — High Speed quality, 10% render target.
- `5` — Ultra Speed quality, 5% render target.
- `Esc` — close the preview window.

The window title reports frame index, render time, blit/present time, tick rate,
target size, surface format, quality mode, and script path. `--print-stats`
prints rows such as `quality=... target=... render_ms=...` for benchmark notes.

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

World graphs also support PNG sequence export through
`render_world_graph_to_png_sequence_with_progress`. In that mode `output_dir`
is an output directory and FFmpeg is not used.

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

## Cinematic Light Process Effects

MotionLoom exposes cinematic lighting as process effects. They use the existing
`<Pass effect="...">` surface, so host applications can use them by updating the
MotionLoom crate and rendering the script; no new scene node schema is required.

Supported effect ids:

- `glow_stack` — multi-radius glow stack with threshold, intensity, radii, and
  tint controls.
- `tone_map` — exposure, contrast, filmic shoulder, gamma, and saturation.
- `light_sweep` — animated directional sweep highlight for text, logos, and
  energy reveals.

Copyable examples live in `motionloom-example/core/process/`:

- `cp-000008` — `glow_stack`
- `cp-000009` — `tone_map`
- `cp-000010` — `light_sweep`

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
use motionloom::{SceneRenderProfile, parse_graph_script, render_scene_graph_frame};

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
let frame = pollster::block_on(render_scene_graph_frame(&graph, 0, SceneRenderProfile::Gpu))
    .or_else(|_| pollster::block_on(render_scene_graph_frame(&graph, 0, SceneRenderProfile::Cpu)))?;
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

## Path DSL Benchmark

MotionLoom includes a Paris-30K-style Path DSL benchmark. It measures DSL
parsing, Path flattening/tessellation, WGPU command encoding, and GPU execution
for static, transformed, and morphed scenes at 100, 1K, 5K, 10K, and 30K Paths.
The command also writes a JSON report and the generated `.motionloom` workloads:

```bash
cargo run --release -p motionloom --example path_dsl_benchmark -- \
  --counts 100,1000,5000,10000,30000 \
  --warmup 2 \
  --samples 10 \
  --size 1600x1600 \
  --json target/path-dsl-benchmark.json \
  --emit-dsl target/path-dsl-workloads
```

See [`benchmarks/path-dsl/README.md`](benchmarks/path-dsl/README.md) for metric
definitions and benchmark methodology.

## Notes

Scene graph APIs are for 2D scene/motion graphics/effect graphs. World graph
APIs are for world/camera/actor/directional-character rendering.

GPU rendering requires a working `wgpu` backend on the host machine. For tools
that need robust fallback behavior, try `SceneRenderProfile::Gpu` first and fall
back to `SceneRenderProfile::Cpu` when GPU initialization fails.

Video rendering functions require an ffmpeg binary path supplied by the caller.
PNG sequence and single-frame image rendering do not require FFmpeg.
