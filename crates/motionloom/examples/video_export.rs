use std::path::PathBuf;

use motionloom::api::{SceneRenderProfile, render_motionloom_document_to_video_with_progress};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ffmpeg = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("FFMPEG").ok())
        .expect(
            "usage: cargo run -p motionloom --example video_export -- /path/to/ffmpeg [output.mp4]",
        );
    let output = std::env::args_os()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("motionloom_video.mp4"));

    let script = r##"
<Graph fps={24} duration="2s" size={[640,360]}>
  <Background color="#0F172A" />
  <Scene id="video_scene">
    <Circle x={curve("0:120:ease_out, 2:520:ease_in_out")}
            y="180"
            radius="72"
            color="#F43F5E" />
    <Text x="320" y="310" value="MotionLoom video export" fontSize="28" color="#FFFFFF" />
  </Scene>
  <Present from="video_scene" />
</Graph>
"##;

    pollster::block_on(render_motionloom_document_to_video_with_progress(
        &ffmpeg,
        script,
        ".",
        &output,
        SceneRenderProfile::Gpu,
        12,
        |progress| {
            println!(
                "rendered {}/{} frames",
                progress.rendered_frames(),
                progress.total_frames()
            );
        },
    ))?;

    println!("saved {}", output.display());
    Ok(())
}
