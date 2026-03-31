# gpui anica-edition Changes

Base: `gpui-v0.2.2` tag
Fork: https://github.com/LOVELYZOMBIEYHO/zed/tree/anica-edition/crates/gpui

## Summary

**+1359 lines added, -71 lines removed** across 14 files.
Core purpose: NV12 (420v) zero-copy external texture/surface rendering for video playback.

## New File

- `src/platform/mac/anica_render.rs` — Metal-based external texture renderer for NV12 video frames; contains `SurfaceExParams_anica`, `draw_surfaces_anica`, `classify_nv12_surface`, `pixel_format_fourcc`

## Changes Per File

| File | -/+ | What changed |
|------|-----|--------------|
| `metal_renderer.rs` | -46/+298 | Added `polychrome_sprites_anica_pipeline_state` and `surfaces_anica_pipeline_state` Metal pipelines; added opt-in profiling via env vars |
| `window.rs` | -12/+82 | Added anica frame callback and surface API |
| `scene.rs` | -1/+167 | Added new scene primitives for external texture rendering |
| `shaders.metal` | +only | Added `polychrome_sprite_anica_vertex/fragment`, `surface_vertex_anica`, `surface_fragment_anica` shaders |
| `build.rs` | +only | Registered new bindgen types: `PolychromeSpriteAnica`, `SurfaceBounds_anica`, `SurfaceInputIndex_anica`; added `anica_render.rs` to rerun-if-changed |
| `gpui.rs` | +only | Re-exported `SurfaceExParams_anica` for external crate access (macOS only) |
| `platform.rs` | -1/+1 | Minor tweak |
| `blade_renderer.rs` | +only | +43 lines blade/wgsl surface support |
| `directx_renderer.rs` | +only | +42 lines Windows DirectX surface support |
| `shaders.wgsl` / `shaders.hlsl` | +only | Shader additions for blade/windows |

## Profiling Env Vars

- `ANICA_GPUI_METAL_BATCH_PROFILER=1` — logs per-batch render timing
- `ANICA_GPUI_RENDERER_TIMING=1` — logs total frame timing (warns if >40ms)
