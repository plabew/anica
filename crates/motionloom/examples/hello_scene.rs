use std::path::PathBuf;

use motionloom::api::{SceneRenderProfile, parse_graph_script, render_scene_graph_frame};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("hello_scene.png"));

    let script = r##"
<Graph fps={30} duration="1s" size={[640,360]}>
  <Background color="#101827" />
  <Scene id="hello_scene">
    <Circle x="320" y="170" radius="92" color="#38BDF8" />
    <Text x="320" y="300" value="Hello MotionLoom" fontSize="34" color="#FFFFFF" />
  </Scene>
  <Present from="hello_scene" />
</Graph>
"##;

    let graph = parse_graph_script(script)?;
    let frame = pollster::block_on(render_scene_graph_frame(&graph, 0, SceneRenderProfile::Gpu))?;
    frame.save(&output)?;
    println!("saved {}", output.display());
    Ok(())
}
