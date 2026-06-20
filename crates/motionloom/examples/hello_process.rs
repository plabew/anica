use std::path::PathBuf;

use motionloom::api::{SceneRenderProfile, parse_graph_script, render_scene_graph_frame};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("hello_process.png"));

    let script = r##"
<Graph fps={30} duration="1s" size={[640,360]} renderSize={[640,360]}>
  <Background color="#111827" />

  <Scene id="source_scene">
    <Rect x="0" y="0" width="640" height="360" color="#111827" />
    <Circle x="240" y="180" radius="88" color="#38BDF8" opacity="0.72" />
    <Circle x="400" y="180" radius="88" color="#F97316" opacity="0.72" />
    <Text x="320" y="298" value="brightness process" fontSize="30" color="#E5E7EB" />
  </Scene>

  <Process id="brightness_process">
    <Tex id="src" fmt="rgba16f" from="scene:source_scene" />
    <Tex id="out" fmt="rgba16f" size={[640,360]} />
    <Pass id="fx_brightness" kind="compute" effect="brightness"
          in={["src"]} out={["out"]}
          params={{ brightness: "1.3" }} />
  </Process>

  <Present from="brightness_process" />
</Graph>
"##;

    let graph = parse_graph_script(script)?;
    let frame = pollster::block_on(render_scene_graph_frame(&graph, 0, SceneRenderProfile::Gpu))?;
    frame.save(&output)?;
    println!("saved {}", output.display());
    Ok(())
}
