# GPU Effect Export Engine (Experimental)

This document describes the current Phase 1 setup for `gpu-effect-export-engine`.

## Goal

Reduce CPU-heavy export cost by moving selected effect paths to a true GPU shader route while keeping existing FFmpeg export as fallback.

## Current phase (Phase 1)

- Crate: `crates/gpu-effect-export-engine`
- Enabled route: true GPU shader path for `single-clip opacity` (V1)
- Integration point: `anica/src/core/export.rs`
- Decode/encode: FFmpeg decode -> Rust `wgpu` opacity shader -> FFmpeg `h264_videotoolbox`
- Fallback: existing `FfmpegExporter::build_ffmpeg_cmd` pipeline

## Route selection rules

The true GPU Phase 1 route is attempted only when all of the following are true:

- output preset is `h264_videotoolbox_mp4`
- exactly one V1 clip
- no additional video tracks
- no subtitle tracks
- no local mask layers
- no transform/color/blur keyframes
- clip effects are identity except optional active `Opacity`
- clip opacity is non-identity (supports clip opacity keyframes)
- default linked-audio layout (single source-aligned audio clip or no audio)

If any check fails, export continues with the existing FFmpeg path.

## Logging

When selected:

- `[Export][GPU Effect] Phase1 true-GPU path selected: single-clip opacity shader + h264_videotoolbox.`

When route fails and falls back:

- `[Export][GPU Effect] Phase1 true-GPU path fallback to standard FFmpeg graph: <reason>`

## Why this shape

This keeps risk low:

- no behavior changes for unsupported timelines
- no regression to current export stability
- explicit fallback to the existing FFmpeg compositor

## Next planned increments

1. Extend true-GPU route to multi-track opacity compositing.
2. Extend to transform (`position/scale`) with parity checks.
3. Add parity tests against existing exporter on sample projects.
