# Media Runtime Layout

This document describes Anica's current local media runtime model.

The current design is:

- FFmpeg may live in a repo-local runtime folder
- macOS still uses host/Homebrew GStreamer

This is not a fully vendored "no system install" media stack on macOS.

## Repo-local FFmpeg runtime

Anica can detect FFmpeg from a workspace-local runtime tree under:

```text
tools/runtime/<os>-<arch>/
tools/runtime/current/<os>/
tools/runtime/current/
```

Expected FFmpeg layout:

```text
tools/runtime/<os>-<arch>/
  ffmpeg/
    bin/
      ffmpeg
      ffprobe
    lib/   # optional
```

On Windows, the binaries are `ffmpeg.exe` and `ffprobe.exe`.

## How Anica finds FFmpeg

Rust-side media detection checks FFmpeg in this general order:

1. `ANICA_FFMPEG_PATH`
2. workspace runtime folders under `tools/runtime/...`
3. configured tools home from `ANICA_TOOLS_HOME`
4. user-local tools home such as `~/.anica/tools/...`
5. bundled app resources on macOS app bundles

On macOS and Linux, `cargo run` goes through:

```text
scripts/cargo_run_with_acp_check.sh
```

That runner exports the detected repo-local FFmpeg runtime when present.

## First-run FFmpeg bootstrap

If FFmpeg is missing, Anica may attempt a first-run bootstrap of a local LGPL FFmpeg runtime.

Current practical behavior:

- macOS: bootstrap is attempted through `scripts/setup_media_tools.sh`
- Linux: bootstrap is attempted through `scripts/setup_media_tools.sh`
- Windows: uses `scripts/setup_media_tools.ps1`

Important for macOS:

- FFmpeg bootstrap still requires Homebrew for build dependencies such as `pkg-config` and `nasm`
- first run is not a guarantee of a fully self-healing setup on a clean Mac without Homebrew

## GStreamer on macOS

Current macOS builds use host/Homebrew GStreamer.

Install it with:

```bash
brew install gstreamer
```

On current Homebrew/macOS setups, the common `gst-*` plugin sets are bundled into the `gstreamer` formula.

Anica checks common host locations such as:

- `/opt/homebrew/bin/gst-launch-1.0`
- `/usr/local/bin/gst-launch-1.0`

Repo-local FFmpeg does not imply repo-local GStreamer on macOS.

## Environment flags

Useful runtime-related environment variables:

- `ANICA_FFMPEG_PATH` - force a specific FFmpeg binary
- `ANICA_TOOLS_HOME` - override the tools home used for local runtime lookup/bootstrap
- `ANICA_RUNTIME_AUTO_DOWNLOAD=0` - disable automatic runtime bootstrap
- `ANICA_DISABLE_RUNTIME_AUTO_DOWNLOAD=1` - disable automatic runtime bootstrap
- `ANICA_MEDIA_RUNTIME_STRICT=0` - relax strict pinned-runtime behavior
- `ANICA_ALLOW_SYSTEM_MEDIA=1` - allow system FFmpeg fallback when strict mode is enabled

## Notes

- Keep FFmpeg license obligations in mind when redistributing runtime files.
- For commercial-safe routing, prefer LGPL-only FFmpeg builds when you want to avoid GPL-enabled builds.
