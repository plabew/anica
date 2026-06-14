#![cfg(not(target_arch = "wasm32"))]

use motionloom::{SceneRenderProfile, parse_graph_script, render_scene_frame};

#[test]
fn public_scene_render_api_draws_cpu_frame() {
    let graph = parse_graph_script(
        r##"
<Graph fps={60} duration="1s" size={[32,24]}>
  <Background color="#000000" />

  <Scene id="api_scene">
    <Timeline>
      <Track id="main" z="0">
        <Sequence duration="1s">
          <Layer>
            <Rect x="4" y="6" width="10" height="8" color="#ff0000" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="api_scene" />
</Graph>
"##,
    )
    .expect("parse scene graph");

    let frame = pollster::block_on(render_scene_frame(&graph, 0, SceneRenderProfile::Cpu))
        .expect("render frame");
    assert_eq!(frame.width(), 32);
    assert_eq!(frame.height(), 24);

    let red = frame.get_pixel(8, 10);
    assert!(red[0] > 200 && red[1] < 40 && red[2] < 40, "got {red:?}");
}
