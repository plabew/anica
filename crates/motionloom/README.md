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
<Graph fps={60} duration="1s" size={[640,360]}>
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

## Notes

Scene graph APIs are for 2D scene/motion graphics/effect graphs. World graph
APIs are for world/camera/actor/directional-character rendering.

GPU rendering requires a working `wgpu` backend on the host machine. For tools
that need robust fallback behavior, try `SceneRenderProfile::Gpu` first and fall
back to `SceneRenderProfile::Cpu` when GPU initialization fails.

Video rendering functions require an ffmpeg binary path supplied by the caller.
