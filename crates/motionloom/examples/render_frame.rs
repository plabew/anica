use std::path::PathBuf;

use motionloom::{SceneRenderProfile, parse_graph_script, render_scene_graph_frame};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("motionloom_frame.png"));

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
            <Text x="320" y="306" value="MotionLoom" fontSize="34" color="#f7f7f7" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="example_scene" />
</Graph>
"##;

    let graph = parse_graph_script(script)?;
    let frame = pollster::block_on(render_scene_graph_frame(&graph, 0, SceneRenderProfile::Gpu))?;
    frame.save(&output)?;
    println!("saved {}", output.display());
    Ok(())
}
