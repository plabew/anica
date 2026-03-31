# Runtime Drop Zone

Place vendored dynamic media runtime files here.

Runtime bootstrap in `scripts/setup_media_tools.sh` and `scripts/setup_media_tools.ps1`
now syncs **physical copies** into `tools/runtime/current/...` (not host-path symlinks/junctions).

Use one of:

- `tools/runtime/<os>-<arch>/...`
- `tools/runtime/current/...`

See `docs/MEDIA_RUNTIME_DROPIN.md` for full structure and environment wiring.

## License And Compliance Notes

If you distribute runtime binaries from this folder:

- FFmpeg should be built as `LGPL-only` (for example: `--disable-gpl --disable-nonfree`).
- GStreamer runtime distribution must keep its original license notices.
- Include third-party notices and source-obtain information for both FFmpeg and GStreamer.

This folder is only a drop zone. Compliance obligations apply when binaries are redistributed.

## Bootstrap Coverage

- macOS/Linux: `scripts/setup_media_tools.sh` supports `local-lgpl` bootstrap into runtime folders.
- Windows: `scripts/setup_media_tools.ps1` supports `local-lgpl` bootstrap and runtime sync, with a strict FFmpeg GPL/nonfree rejection check.

## Runtime Form

- GStreamer runtime sync is physical-copy based for reproducible local runtime folders.
- FFmpeg local-lgpl runtime is installed under versioned folders and linked via `current`.
