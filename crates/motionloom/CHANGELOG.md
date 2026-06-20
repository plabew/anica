# Changelog

## 0.1.0

Initial public MotionLoom crate release.

- Parses MotionLoom graph DSL for scene, process, and mixed scene/process graphs.
- Renders scene/composition frames through CPU and wgpu-backed paths.
- Exports single frames and PNG sequences without FFmpeg.
- Exports video through a caller-supplied FFmpeg binary.
- Provides process/effect runtime evaluation and a process catalog for host UI integration.
- Provides preview APIs for MotionLoom-owned wgpu textures, caller-owned wgpu targets, and platform preview surfaces.
- Exposes `motionloom::api` as the recommended stable integration surface.
