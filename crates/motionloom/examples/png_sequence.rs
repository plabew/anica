use std::path::PathBuf;

use motionloom::api::{parse_graph_script, render_scene_graph_to_png_sequence_with_progress};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output_dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("motionloom_frames"));

    let script = r##"
<Graph fps={12} duration="1s" size={[320,180]}>
  <Background color="#020617" />
  <Scene id="moving_dot">
    <Timeline>
      <Track id="main" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Circle x={curve("0:64:ease_out, 1:256:ease_in_out")}
                    y="90"
                    radius="34"
                    color="#A3E635" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="moving_dot" />
</Graph>
"##;

    let graph = parse_graph_script(script)?;
    pollster::block_on(render_scene_graph_to_png_sequence_with_progress(
        &graph,
        &output_dir,
        1,
        |progress| {
            println!(
                "rendered {}/{} PNG frames",
                progress.rendered_frames, progress.total_frames
            );
        },
    ))?;

    println!("saved PNG sequence to {}", output_dir.display());
    Ok(())
}
