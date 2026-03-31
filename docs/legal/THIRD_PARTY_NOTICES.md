# Third-Party License Notes (Anica)

This document summarizes important third-party licensing topics for Anica.
It is intended for maintainers preparing public releases.

## 0. Runtime distribution model (important)

- Anica source code is Apache-2.0.
- FFmpeg/FFprobe and GStreamer keep their own upstream licenses.
- Current default setup is user-installed runtime:
  users install/download FFmpeg/FFprobe and GStreamer on their own machine
  (for example via setup scripts), rather than receiving those binaries
  bundled inside an official Anica release artifact.
- In this source-only model, keep NOTICE/legal docs in the project and make
  it explicit that FFmpeg/GStreamer licenses apply separately.
- If you later ship app binaries that include FFmpeg/GStreamer, you become a
  runtime redistributor and must satisfy the redistribution obligations for
  the exact binaries/plugins you ship.

## 1. Anica license

- Anica source code: Apache-2.0 (project-level intent).
- Your own source can remain Apache-2.0 even when using third-party dependencies.

## 2. Rust dependency layer vs runtime library layer

Anica uses Rust crates and also depends on native runtime libraries.
These are separate licensing layers:

- Rust crate license: the license of the Rust package itself.
- Native runtime license: the license of the linked/loaded native library.

You must satisfy both where applicable.

## 3. GStreamer

- Anica uses GStreamer via Rust crates (`gstreamer`, `gstreamer-app`, etc.).
- Rust bindings do not replace GStreamer runtime license obligations.
- GStreamer core is generally LGPL-2.1-or-later.
- Plugin licenses vary; some plugins may be GPL.
- Default project workflow is user-side install/sync of GStreamer runtime.
- If you redistribute GStreamer binaries/plugins yourself, include required
  license texts/notices and keep a plugin-level license inventory.

### Release guidance

- Prefer dynamic library usage/distribution.
- Include third-party license texts for redistributed runtime binaries.
- Keep an inventory of plugins you ship and their licenses.

## 4. FFmpeg / FFprobe

- Current Anica setup expects user-installed FFmpeg/FFprobe by default.
- This keeps app-side compliance simpler for Apache-2.0 releases.
- FFmpeg/FFprobe licenses apply separately from Anica's Apache-2.0 license.
- If you later bundle FFmpeg binaries, add full FFmpeg-related notices and
  satisfy LGPL/GPL requirements for the exact build you distribute.

## 5. Practical checklist for release

1. Keep `NOTICE` in repository root (`anica/NOTICE`).
2. Keep this doc in `anica/docs/legal/THIRD_PARTY_NOTICES.md`.
3. Verify dependency licenses from `Cargo.lock` and direct crates.
4. If source-only release: state clearly that FFmpeg/GStreamer are user-installed
   and separately licensed.
5. If shipping runtime binaries: verify GStreamer runtime/plugin licenses for
   shipped binaries.
6. If shipping FFmpeg binaries: add FFmpeg-specific compliance artifacts.

## 6. Useful links

- GStreamer: https://gstreamer.freedesktop.org/
- FFmpeg legal: https://ffmpeg.org/legal.html
- SPDX license list: https://spdx.org/licenses/
