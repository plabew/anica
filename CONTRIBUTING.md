# Contributing to Anica

Thanks for contributing.

This project is an open source Rust video editor. Please read this file before opening a PR.

## Development Setup

### Requirements

- Rust stable toolchain
- `cargo`
- GStreamer runtime installed
- On macOS, FFmpeg runtime is bootstrapped on first `cargo run` when needed
- macOS FFmpeg bootstrap still requires Homebrew for build dependencies

### Run locally

```bash
cargo run
```

### Quick validation before PR

```bash
cargo fmt --check
cargo check
cargo clippy --workspace --all-targets -- -D clippy::correctness -D clippy::suspicious -W clippy::perf
cargo test
```

If your change is UI-only and there are no tests for that area yet, still run `cargo check`.

## Coding Standard

Follow the repository engineering standard in:

- `docs/rust_contributor_standard.md`

That document defines:

- where logic should live
- how to use domain types
- error handling expectations
- GPUI state and rendering boundaries
- validation expectations for contributor PRs

## Project Structure (high-level)

- `src/core/` - timeline state, export, proxy, subtitle logic
- `src/ui/` - GPUI panels and interaction
- `crates/ai-subtitle-engine/` - local Whisper runtime, model pack loading, subtitle inference
- `crates/video-engine/` - GStreamer playback and frame access
- `crates/gpui-video-renderer/` - rendering bridge for preview

When possible:

- put business logic in `src/core`
- keep `src/ui` focused on presentation and event wiring

## AI Subtitle Model Packs

AI subtitle models are discovered from:

- `crates/ai-subtitle-engine/src/model/onnx/<model_pack_folder>/`

Each model pack should include:

- encoder ONNX file
- decoder ONNX file
- `tokenizer.json`
- `config.json` (required)
- `manifest.json` (required)

Recommended:

- `preprocessor_config.json` (recommended for stable frontend/audio settings)

Required `manifest.json` fields:

- `id`
- `display_name`
- `runtime_kind`
- `model_config`
- `encoder`
- `decoder`
- `tokenizer`
- `overlap_frames`

`runtime_kind` values (current):

- `whisper_seq2seq_v1` (required for current runtime)

Meaning:

- `seq2seq` = sequence-to-sequence (encoder-decoder) runtime pipeline.
- `v1` = current Anica Whisper runtime contract version.
- This is runtime compatibility metadata, not a model quality label.

Optional but supported `manifest.json` fields:

- `preprocessor_config` (default: `preprocessor_config.json`)
- `max_decode_steps` (if omitted, engine reads `config.json.max_target_positions`)
- `architecture`, `frontend`, `sample_rate`, `n_fft`, `hop_length`, `n_mels`, `chunk_length_sec`
- `model_author`, `model_repo`, `precision`, `variant`

Model config expectations (`config.json`):

- should include architecture info (`architectures` or `model_type`)
- should include `num_mel_bins` unless provided elsewhere
- should include `max_target_positions` unless `manifest.max_decode_steps` is set
- `max_source_positions` is used as a fallback to infer chunk length when needed

Preprocessor expectations (`preprocessor_config.json`, if used):

- typical fields: `sampling_rate`, `n_fft`, `hop_length`, `feature_size`, `chunk_length`, `nb_max_frames`
- if this file is missing, equivalent values must be supplied by `manifest.json`/`config.json`

Minimal `manifest.json` example:

```json
{
  "id": "whisper_large_v3_turbo_fp16",
  "display_name": "Whisper Large V3 Turbo FP16",
  "runtime_kind": "whisper_seq2seq_v1",
  "model_config": "config.json",
  "preprocessor_config": "preprocessor_config.json",
  "overlap_frames": 150,
  "encoder": "encoder_model_fp16.onnx",
  "decoder": "decoder_model_merged_fp16.onnx",
  "tokenizer": "tokenizer.json"
}
```

Notes:

- The AI SRT model dropdown only shows valid model packs.
- A pack is considered valid only when all required files in `manifest.json` exist.
- `preprocessor_config.json` can be omitted, but quality/stability may be lower on some packs.

## Contribution Rules

### 1) Keep PRs focused

- One feature/fix per PR
- Avoid mixing refactor + behavior changes unless necessary

### 2) Do not commit heavy/generated files

Do not commit:

- `target/`
- local proxies (`.proxy/`)
- exported media
- temporary test media

### 3) Preserve behavior unless requested

If you change UX/behavior, explain:

- previous behavior
- new behavior
- reason

### 4) Respect performance-sensitive paths

Preview, decode, render, and export are sensitive paths. For these changes, include:

- what changed
- expected performance impact
- how you verified impact

## Code Style

- Prefer clear, small functions
- Avoid large monolithic handlers
- Comments must be concise English comments that explain purpose or constraints
- Keep the required 3-line Rust file header comment at the top of source files
- Use descriptive names over abbreviations

## Pull Request Checklist

Before opening a PR:

- [ ] `cargo check` passes
- [ ] tests updated or reason provided
- [ ] no unrelated file churn
- [ ] screenshots/GIF for UI changes
- [ ] migration notes if state format changed

## Reporting Bugs

Include:

- OS and hardware
- input media format (codec, resolution, fps, bitrate)
- expected vs actual behavior
- logs (if available)
- minimal reproduction steps

For crashes, include backtrace if possible:

```bash
RUST_BACKTRACE=1 cargo run
```

## Feature Requests

Please provide:

- user problem to solve
- expected workflow
- constraints (performance, quality, export compatibility)

This helps keep implementation practical and reviewable.
