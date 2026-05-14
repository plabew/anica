use std::path::PathBuf;

use motionloom::{SceneRenderProfile, parse_graph_script, render_scene_frame};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("motionloom_frame.png"));

    let script = r##"
<Graph scope="scene" fps={60} duration="1s" size={[640,360]}>
  <Scene id="example_scene">
    <Solid color="#101827" />
    <Circle x="320" y="180" radius="96" color="#4cc9f0" />
    <Path id="spark"
          d="M 320 84 L 342 158 L 416 180 L 342 202 L 320 276 L 298 202 L 224 180 L 298 158 Z"
          fill="#ffffff"
          opacity="0.86" />
    <Text x="320" y="306" value="MotionLoom" fontSize="34" color="#f7f7f7" />
  </Scene>
  <Present from="example_scene" />
</Graph>
"##;

    let graph = parse_graph_script(script)?;
    let frame = render_scene_frame(&graph, 0, SceneRenderProfile::Gpu)
        .or_else(|_| render_scene_frame(&graph, 0, SceneRenderProfile::Cpu))?;
    frame.save(&output)?;
    println!("saved {}", output.display());
    Ok(())
}
