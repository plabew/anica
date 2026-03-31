# ACP User Guide Catalog

Use this catalog when users ask setup, install, login, or quick-start steps for external CLIs used with ACP workflows.

## Routing Rules
1. Open this file first.
2. Match by `When to open` + `Keywords`.
3. Open only the best matching `UG-xxxx.md`.
4. If command behavior differs by CLI version, tell user to run `--help` locally.

## Document List

| ID | Title | When to open | Keywords | File |
|---|---|---|---|---|
| UG-0001 | NPM install methods + Codex/Gemini/Claude CLI install and login | User asks how to install npm, install Codex CLI, run `codex login`, use `codex login --device-auth`, install Gemini CLI, login Gemini CLI without API key, or setup Claude CLI login. | `npm`, `node`, `install`, `@openai/codex`, `codex login`, `--device-auth`, `@google/gemini-cli`, `gemini`, `/auth`, `oauth-personal`, `claude`, `claude auth login`, `claude auth status`, `@anthropic-ai/claude-code`, `global install` | `anica/docs/acp/userguide/UG-0001.md` |
| UG-0002 | Full setup guide — build Anica and connect ACP | User asks how to build Anica from source, install Rust, install GStreamer, install FFmpeg, set up ACP, or get the full install guide. | `build`, `install`, `setup`, `cargo run`, `rust`, `gstreamer`, `ffmpeg`, `xcode`, `from source`, `connect ACP`, `full guide` | `anica/docs/acp/userguide/UG-0002.md` |

## Authoring Rules
- Keep one setup topic per `UG-xxxx.md`.
- Commands should be copy-pasteable.
- Mention whether a flag needs a value or not.
- Keep platform-specific notes short and practical.
- Do not create `index.md` in this folder; use `catalog.md`.
