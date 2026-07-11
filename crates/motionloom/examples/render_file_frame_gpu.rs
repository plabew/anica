use std::{
    env, fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use motionloom::{SceneRenderProfile, SceneRenderer, parse_graph_script, set_scene_asset_roots};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args().nth(1).expect("expected MotionLoom file path");
    let output = env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/private/tmp/motionloom_gpu_frame_check.png"));
    let frame_index = env::args()
        .nth(3)
        .map(|value| value.parse::<u32>())
        .transpose()?
        .unwrap_or(0);

    let total_started_at = Instant::now();

    let read_started_at = Instant::now();
    let script = fs::read_to_string(&path)?;
    if let Some(parent) = PathBuf::from(&path).parent() {
        set_scene_asset_roots(vec![parent.to_path_buf()]);
    }
    let read_elapsed = read_started_at.elapsed();

    let parse_started_at = Instant::now();
    let graph = parse_graph_script(&script)?;
    let parse_elapsed = parse_started_at.elapsed();

    let strict_gpu_started_at = Instant::now();
    let mut renderer = pollster::block_on(SceneRenderer::new(SceneRenderProfile::Gpu))?;
    let gpu_texture =
        pollster::block_on(renderer.render_frame_to_wgpu_texture(&graph, frame_index))?;
    let strict_gpu_elapsed = strict_gpu_started_at.elapsed();

    let render_started_at = Instant::now();
    let frame = pollster::block_on(renderer.render_frame_gpu_readback(&graph, frame_index))?;
    let render_elapsed = render_started_at.elapsed();

    let save_started_at = Instant::now();
    frame.save(&output)?;
    let save_elapsed = save_started_at.elapsed();

    println!(
        "saved strict GPU frame {} to {}",
        frame_index,
        output.display()
    );
    println!(
        "strict GPU texture: {}x{} {:?}",
        gpu_texture.width, gpu_texture.height, gpu_texture.format
    );
    println!(
        "timing: read={} parse={} strict_gpu={} gpu_readback={} save={} total={}",
        format_duration(read_elapsed),
        format_duration(parse_elapsed),
        format_duration(strict_gpu_elapsed),
        format_duration(render_elapsed),
        format_duration(save_elapsed),
        format_duration(total_started_at.elapsed())
    );
    Ok(())
}

fn format_duration(duration: Duration) -> String {
    format!("{:.1}ms", duration.as_secs_f64() * 1000.0)
}
