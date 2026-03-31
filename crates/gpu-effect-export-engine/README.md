# gpu-effect-export-engine

Experimental GPU effect export helpers for Anica.

Phase 1 scope:

- single-clip timeline (V1 only)
- opacity-only effect (including clip opacity keyframes)
- `h264_videotoolbox` encoder (macOS)

Implemented components:

- FFmpeg arg builder for the existing opacity route.
- `WgpuOpacityProcessor`: per-frame RGBA opacity shader (`rgb *= opacity`) using `wgpu`.

Timeline capability checks, decode/encode orchestration, and fallback behavior remain in `anica/src/core/export.rs`.
