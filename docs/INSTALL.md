# Anica — Build from Source

This guide walks you through building Anica from source on macOS, Linux, and Windows.

---

## 1. Install Rust

All platforms use [rustup](https://rustup.rs/).

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

After install, restart your terminal and verify:

```bash
rustc --version
cargo --version
```

The repository includes `rust-toolchain.toml` which pins the required Rust version. `rustup` will install it automatically on first build.

---

## 2. Install system dependencies

### macOS

```bash
# Xcode command-line tools (required for Metal, linker, system headers)
xcode-select --install

# GStreamer (video pipeline)
brew install gstreamer

# FFmpeg (export encoding)
brew install ffmpeg
```

On current Homebrew/macOS setups, the common `gst-*` plugin sets are bundled into the `gstreamer` formula.

### Ubuntu / Debian

```bash
# Build essentials
sudo apt-get update
sudo apt-get install -y build-essential pkg-config cmake

# GStreamer
sudo apt-get install -y \
  libgstreamer1.0-dev \
  libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-plugins-base \
  gstreamer1.0-plugins-good \
  gstreamer1.0-editing-services \
  libges-1.0-dev

# FFmpeg
sudo apt-get install -y ffmpeg

# Additional libraries that GPUI may need
sudo apt-get install -y \
  libxkbcommon-dev \
  libwayland-dev \
  libvulkan-dev
```

### Windows

1. Install [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the "Desktop development with C++" workload.

2. Install GStreamer from [gstreamer.freedesktop.org](https://gstreamer.freedesktop.org/download/):
   - Download the **MSVC 64-bit** runtime and development installers.
   - Set `GSTREAMER_1_0_ROOT_MSVC_X86_64` environment variable to the install path (the installer usually does this).

3. Install FFmpeg:
   - Download a release build from [gyan.dev](https://www.gyan.dev/ffmpeg/builds/) or [BtbN](https://github.com/BtbN/FFmpeg-Builds/releases).
   - Add the `bin/` folder to your `PATH`.

4. Verify:

```powershell
gst-launch-1.0 --version
ffmpeg -version
```

Optional runtime bootstrap (sync into `tools/runtime/current/windows`):

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\setup_media_tools.ps1 -Mode local-lgpl -InstallGStreamer -Yes -ToolsHome .\tools\runtime\current\windows
```

`local-lgpl` mode rejects FFmpeg builds that expose `--enable-gpl` or `--enable-nonfree` in `ffmpeg -version`.

---

## 3. Clone and build

```bash
git clone https://github.com/LOVELYZOMBIEYHO/anica.git
cd anica
cargo build
```

This compiles the main `anica` binary and the `anica-acp` agent binary.

### Run

```bash
cargo run
```

### Build release (optimized)

```bash
cargo build --release
./target/release/anica
```

---

## 4. Verify dependencies

After building, confirm the runtime dependencies are accessible:

```bash
ffmpeg -version
gst-launch-1.0 --version
```

If either command fails, Anica's export and preview features will not work.

---

## 5. Install AI CLI tools (optional, for ACP chat)

Anica's AI chat feature (ACP) uses external CLI tools to communicate with LLM providers. You need **at least one** of the following.

### Prerequisites: Node.js and npm

All three CLIs are installed via npm. If you don't have npm:

**macOS:**
```bash
brew install node
```

**Ubuntu / Debian:**
```bash
curl -fsSL https://deb.nodesource.com/setup_lts.x | sudo -E bash -
sudo apt-get install -y nodejs
```

**Windows:**

Download and install from [nodejs.org](https://nodejs.org/).

Verify:
```bash
node -v
npm -v
```

---

### Option A: OpenAI Codex CLI

```bash
npm install -g @openai/codex
```

Login:
```bash
codex login
```

If browser login is not available (e.g. SSH):
```bash
codex login --device-auth
```

Verify:
```bash
codex --version
```

---

### Option B: Google Gemini CLI

```bash
npm install -g @google/gemini-cli
```

Login (OAuth, no API key needed):
```bash
gemini
# Then type: /auth
# Select: oauth-personal
# Follow the browser flow
```

Or use an API key instead:
```bash
export GEMINI_API_KEY="your-key-here"
```

Verify:
```bash
gemini --version
```

---

### Option C: Anthropic Claude CLI

```bash
npm install -g @anthropic-ai/claude-code
```

Login:
```bash
claude auth login
```

Verify:
```bash
claude auth status
```

---

## 6. Connect ACP in Anica

1. Open Anica and navigate to the **AI Agents** page (sidebar icon).
2. Select a provider (Codex / Gemini / Claude).
3. The ACP Agent Command field should auto-fill. If not, point it to `target/debug/anica-acp`.
4. Click **Connect**.
5. Once the status shows green "Connected", use the AI Chat widget to interact.

---

## Troubleshooting

| Problem | Solution |
|---------|----------|
| `cargo build` fails with missing `gstreamer` | Ensure GStreamer dev packages are installed (see step 2) |
| `cargo build` fails with linker errors on macOS | Run `xcode-select --install` |
| FFmpeg not found at runtime | Add `ffmpeg` to your `PATH` |
| Codex/Gemini/Claude CLI not found | Run the `npm install -g` command again and check `PATH` |
| ACP shows "Disconnected" immediately | Check the ACP Agent Command path points to a valid `anica-acp` binary |
| `rust-toolchain.toml` triggers download | This is normal — `rustup` is installing the pinned Rust version |
