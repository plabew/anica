# macOS App Bundle Packaging

This document describes the current macOS packaging path for Anica when you want
to ship a self-contained `.app` bundle with bundled FFmpeg and GStreamer.

## Scope

The goal is:

- build `anica` and `anica-acp`
- create `dist/Anica.app`
- bundle:
  - app binaries
  - `assets/`
  - `docs/`
  - local FFmpeg runtime
  - Homebrew GStreamer runtime

This is a practical local packaging path for macOS first. It is not a complete
public release pipeline yet.

## Current output layout

```text
dist/Anica.app/
  Contents/
    Info.plist
    MacOS/
      anica
    Resources/
      anica-acp
      assets/
      docs/
      LICENSE
      NOTICE
      SECURITY.md
      runtime/
        current/
          macos/
            ffmpeg/
              bin/
              lib/
            gstreamer/
              bin/
              lib/
              libexec/
```

At runtime, the app now looks inside bundle resources first for:

- `assets`
- `docs`
- `assets/fonts`
- `assets/twemoji`
- `runtime/current/macos`

## Prerequisites

1. Build dependencies installed for source builds
2. Homebrew GStreamer installed
3. Repo-local FFmpeg runtime available

Typical setup:

```bash
brew install gstreamer
./scripts/setup_media_tools.sh --mode local-lgpl --yes --tools-home ./tools/runtime/current/macos
```

## Build the app bundle

```bash
./scripts/package_macos_app.sh
```

This will:

1. `cargo build --release --bins`
2. create `dist/Anica.app`
3. copy FFmpeg from `tools/runtime/current/macos/ffmpeg`
4. copy GStreamer from `brew --prefix gstreamer`
5. patch Mach-O install names / rpaths
6. ad-hoc sign the bundle by default

## Useful options

```bash
./scripts/package_macos_app.sh --profile debug
./scripts/package_macos_app.sh --skip-build
./scripts/package_macos_app.sh --ffmpeg-runtime /path/to/ffmpeg-runtime-root
./scripts/package_macos_app.sh --gstreamer-prefix /opt/homebrew/opt/gstreamer
./scripts/package_macos_app.sh --codesign-identity "Developer ID Application: Your Name (TEAMID)"
./scripts/package_macos_app.sh --skip-codesign
```

## What this solves

- The app binary no longer depends on repo-relative assets at runtime.
- FFmpeg can be bundled under app resources instead of requiring user install.
- GStreamer can be copied from Homebrew into the app bundle and patched to use
  bundle-local dylibs/plugins.

## What still remains for a real public release

1. Developer ID signing for every shipped binary and dylib
2. notarization
3. DMG or ZIP artifact generation
4. upstream runtime license text inventory for the exact GStreamer/FFmpeg build shipped
5. tighter plugin selection if you do not want to ship the full Homebrew GStreamer set

## Licensing

If you redistribute FFmpeg or GStreamer inside an official app release, you are
redistributing those runtimes yourself. That means you must satisfy the license
obligations for the exact binaries/plugins you ship.

Read:

- `docs/legal/THIRD_PARTY_NOTICES.md`
- https://ffmpeg.org/legal.html
- https://gstreamer.freedesktop.org/
