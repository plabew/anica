# Media Runtime Drop-In

Anica uses FFmpeg/FFprobe for media preview, export, proxies, thumbnails, waveform preparation, and analysis.

## Layout

Preferred active runtime path:

```text
tools/runtime/current/<os>/ffmpeg/bin/ffmpeg
tools/runtime/current/<os>/ffmpeg/bin/ffprobe
```

Versioned runtime path:

```text
tools/runtime/<os>/ffmpeg/<version>/bin/ffmpeg
tools/runtime/<os>/ffmpeg/<version>/bin/ffprobe
```

## Bootstrap

- macOS/Linux: `./scripts/setup_media_tools.sh`
- Windows: `powershell -ExecutionPolicy Bypass -File .\scripts\setup_media_tools.ps1 -Yes`

## Environment

The app and runner prefer repo-local runtime tools and set:

- `ANICA_FFMPEG_PATH`
- `ANICA_FFPROBE_PATH`
- `ANICA_TOOLS_HOME`
