# MotionLoom WASM Smoke Test

This directory contains a browser smoke-test harness for the MotionLoom WASM API.

## Prerequisites

Install `wasm-pack`:

```bash
curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
```

## Build

From the repository root:

```bash
wasm-pack build crates/motionloom --target web --out-dir wasm-smoke-test/pkg
```

## Run in a browser

Serve the `wasm-smoke-test` directory with any static file server, for example:

```bash
cd crates/motionloom/wasm-smoke-test
python3 -m http.server 8080
```

Open <http://localhost:8080> and click the buttons to verify:

- `motionloom_document_type` detects a scene graph.
- `motionloom_parse_summary` returns a scene graph summary.
- `WasmSceneRenderer` with CPU profile renders a red background to RGBA bytes.
- `WasmSceneRenderer` with GPU profile renders a GPU-native scene directly to a canvas.
- `motionloom_add_asset` feeds an in-memory PNG into the renderer.

## Run headless tests

```bash
wasm-pack test crates/motionloom --headless --chrome --test wasm_browser_smoke
```

Or with Node:

```bash
wasm-pack test crates/motionloom --node --test wasm_browser_smoke
```

The tests live in `crates/motionloom/tests/wasm_browser_smoke.rs`.
