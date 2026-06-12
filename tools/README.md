# Runtime Drop Zone

Place vendored FFmpeg/FFprobe runtime files here.

Runtime bootstrap in `scripts/setup_media_tools.sh` and `scripts/setup_media_tools.ps1`
syncs physical copies into `tools/runtime/current/...` for reproducible local runs.

Use one of:

- `tools/runtime/<os>-<arch>/...`
- `tools/runtime/current/...`

See `docs/MEDIA_RUNTIME_DROPIN.md` for the current structure and environment wiring.

## License And Compliance Notes

If you distribute runtime binaries from this folder:

- FFmpeg should be built as `LGPL-only`, for example with `--disable-gpl --disable-nonfree`.
- LGPL builds may still enable permissive codec libraries such as `libvpx`, `libaom`, `libsvtav1`, and `libopus`.
- Include third-party notices and source-obtain information for the exact FFmpeg build you ship.

This folder is only a drop zone. Compliance obligations apply when binaries are redistributed.

## Bootstrap Coverage

- macOS/Linux: `scripts/setup_media_tools.sh` syncs or downloads the FFmpeg runtime.
- Windows: `scripts/setup_media_tools.ps1` syncs or downloads the FFmpeg runtime.

## Runtime Form

FFmpeg local runtime is installed under versioned folders and mirrored via `current`.
