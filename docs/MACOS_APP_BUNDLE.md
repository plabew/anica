# macOS App Bundle

`scripts/package_macos_app.sh` builds a self-contained `.app` bundle with the Anica binary, assets, and FFmpeg runtime.

## Requirements

- Rust toolchain
- FFmpeg runtime under `tools/runtime/current/macos/ffmpeg`

## Build

```bash
./scripts/setup_media_tools.sh --sync-only
./scripts/package_macos_app.sh
```

Use a custom FFmpeg prefix:

```bash
./scripts/package_macos_app.sh --ffmpeg-prefix /path/to/ffmpeg
```

If you redistribute the bundle, include notices and source-obtain information for the exact FFmpeg build shipped.
