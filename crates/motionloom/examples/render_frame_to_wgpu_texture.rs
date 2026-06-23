// =========================================
// =========================================
// crates/motionloom/examples/render_frame_to_wgpu_texture.rs

use motionloom::{SceneRenderProfile, SceneRenderer, parse_graph_script};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let script = r##"
<Graph fps={30} duration="1s" size={[640,360]}>
  <Background color="#101827" />

  <Scene id="example_scene">
    <Timeline>
      <Track id="main" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Circle x="320" y="180" radius="96" color="#4cc9f0" />
            <Path id="spark"
                  d="M 320 84 L 342 158 L 416 180 L 342 202 L 320 276 L 298 202 L 224 180 L 298 158 Z"
                  fill="#ffffff"
                  opacity="0.86" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="example_scene" />
</Graph>
"##;

    let graph = parse_graph_script(script)?;
    let mut renderer = pollster::block_on(SceneRenderer::new(SceneRenderProfile::Gpu))?;
    let gpu_texture = pollster::block_on(renderer.render_frame_to_wgpu_texture(&graph, 0))?;

    println!(
        "rendered {}x{} {:?} texture",
        gpu_texture.width, gpu_texture.height, gpu_texture.format
    );

    // The texture can now be passed to another wgpu pipeline. Native handle
    // extraction and display require app-specific platform integration.
    let _view = gpu_texture
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    Ok(())
}
