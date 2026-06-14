// =========================================
// =========================================
// crates/motionloom/tests/wasm_browser_smoke.rs

//! Browser smoke tests for the MotionLoom WASM API.
//!
//! These tests are compiled for `wasm32-unknown-unknown` and executed with
//! `wasm-bindgen-test` (via `wasm-pack test --headless --chrome` or Node).

#[cfg(target_arch = "wasm32")]
mod wasm_tests {
    use wasm_bindgen_test::*;

    use motionloom::wasm_api::{
        WasmSceneRenderer, WasmWorldRenderer, motionloom_document_type, motionloom_parse_summary,
        motionloom_render_scene_frame, motionloom_render_scene_frame_with_profile,
    };

    // These tests do not require browser-only APIs, so they run in Node.js by
    // default. They can also be executed in a real browser with:
    //   wasm-pack test crates/motionloom --headless --chrome --test wasm_browser_smoke
    // (Chrome must be installed; this macOS environment only has Safari/Firefox
    // and both drivers fail due to system restrictions.)

    const COLOR_SCENE: &str = r##"<Graph fps={30} duration="1s" size={[64,64]}>
  <Background color="#000000" />
  <Scene id="stage">
    <Timeline>
      <Track id="main" z="0">
        <Sequence duration="1s">
          <Layer>
            <Rect x="0" y="0" width="64" height="64" color="#ff0000" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="stage" />
</Graph>"##;

    #[wasm_bindgen_test]
    fn document_type_detects_scene() {
        assert_eq!(motionloom_document_type(COLOR_SCENE), "scene");
    }

    #[wasm_bindgen_test]
    fn parse_summary_returns_scene() {
        let summary = motionloom_parse_summary(COLOR_SCENE).expect("parse summary");
        assert!(summary.starts_with("scene graph"));
    }

    #[wasm_bindgen_test]
    async fn render_scene_frame_produces_rgba_buffer() {
        let mut renderer = WasmSceneRenderer::new(COLOR_SCENE, "cpu").expect("scene renderer");
        let buffer = renderer.render_frame(0).await.expect("render frame");
        // 64x64 RGBA = 16384 bytes.
        assert_eq!(buffer.len(), 64 * 64 * 4);
        // The background is pure red; verify the first pixel.
        assert_eq!(buffer[0], 255);
        assert_eq!(buffer[1], 0);
        assert_eq!(buffer[2], 0);
        assert_eq!(buffer[3], 255);
    }

    #[wasm_bindgen_test]
    async fn standalone_render_scene_frame_produces_rgba_buffer() {
        let buffer = motionloom_render_scene_frame(COLOR_SCENE, 0, 32, 32)
            .await
            .expect("standalone render frame");
        assert_eq!(buffer.len(), 32 * 32 * 4);
    }

    #[wasm_bindgen_test]
    async fn render_scene_frame_with_gpu_cpu_fallback() {
        let buffer = motionloom_render_scene_frame_with_profile(COLOR_SCENE, 0, 32, 32, "gpu-cpu")
            .await
            .expect("gpu-cpu render frame");
        assert_eq!(buffer.len(), 32 * 32 * 4);
    }

    #[wasm_bindgen_test]
    async fn scene_renderer_owns_assets_independently() {
        let image_scene = r##"<Graph fps={30} duration="1s" size={[64,64]}>
  <Scene id="stage">
    <Timeline>
      <Track id="main" z="0">
        <Sequence duration="1s">
          <Layer>
            <Background color="#000000" />
            <Image src="pixel.png" x={0} y={0} width={64} height={64} />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="stage" />
</Graph>"##;

        // Valid 1x1 white RGB PNG generated with zlib-compressed scanline data.
        let png = [
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9c, 0x63, 0xf8, 0xff, 0xff, 0x3f, 0x00, 0x05, 0xfe, 0x02, 0xfe, 0x0d, 0xef, 0x46,
            0xb8, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ];

        let mut renderer = WasmSceneRenderer::new(image_scene, "cpu").expect("scene renderer");
        renderer.add_asset("pixel.png", &png);
        let buffer = renderer.render_frame(0).await.expect("render with asset");
        assert_eq!(buffer.len(), 64 * 64 * 4);
        // Top-left pixel should be white (image covers the whole canvas).
        assert_eq!(buffer[0], 255);
        assert_eq!(buffer[1], 255);
        assert_eq!(buffer[2], 255);
        assert_eq!(buffer[3], 255);
    }

    #[wasm_bindgen_test]
    fn world_renderer_can_be_constructed() {
        let world_script = r##"<Graph fps={30} duration="1s" size={[64,64]}>
  <World id="stage">
    <Background color="#00ff00" />
    <Camera yaw="0" pitch="0" zoom="1" />
  </World>
  <Present from="stage" />
</Graph>"##;
        let _renderer = WasmWorldRenderer::new(world_script).expect("world renderer");
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native_stub {
    // This file is intentionally empty on native targets. The WASM API tests
    // are only meaningful when compiled for wasm32-unknown-unknown.
}
