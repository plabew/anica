# Install Anica From Source

## Requirements

- Rust toolchain from `rust-toolchain.toml`
- FFmpeg and FFprobe

## macOS

```bash
brew install git ffmpeg
cargo run
```

## Linux

```bash
sudo apt install ffmpeg
cargo run
```

## Windows

Install FFmpeg, or use the repo runtime bootstrap:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\setup_media_tools.ps1 -Yes
cargo run
```

## Verify Runtime

```bash
ffmpeg -version
ffprobe -version
```

## Troubleshooting

| Problem | Fix |
| --- | --- |
| `ffmpeg` missing | Install FFmpeg or run the runtime bootstrap script |
| `ffprobe` missing | Ensure FFprobe is installed beside FFmpeg |
| Preview is slow | Use preview proxy, lower preview resolution, or check hardware acceleration logs |
