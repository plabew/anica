use motionloom::api::{SceneRenderProfile, SceneRenderer, parse_graph_script};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let script = r##"
<Graph fps={30} duration="1s" size={[320,180]}>
  <Background color="#111827" />
  <Scene id="preview_scene">
    <Timeline>
      <Track id="main" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Rect x="28" y="28" width="264" height="124" radius="24" color="#1E293B" />
            <Circle x="160" y="90" radius="44" color="#22D3EE" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="preview_scene" />
</Graph>
"##;

    let graph = parse_graph_script(script)?;
    let mut renderer = pollster::block_on(SceneRenderer::new(SceneRenderProfile::Gpu))?;
    let texture = pollster::block_on(renderer.render_frame_to_wgpu_texture(&graph, 0))?;

    println!(
        "rendered preview texture: {}x{} {:?}",
        texture.width, texture.height, texture.format
    );

    Ok(())
}
