use std::{env, fs, path::PathBuf};

use motionloom::{SceneRenderProfile, parse_graph_script, render_scene_graph_frame};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args().nth(1).expect("expected MotionLoom file path");
    let output = env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/private/tmp/motionloom_frame_check.png"));
    let frame_index = env::args()
        .nth(3)
        .map(|value| value.parse::<u32>())
        .transpose()?
        .unwrap_or(0);
    let profile = env::args()
        .nth(4)
        .as_deref()
        .map(parse_profile)
        .transpose()?
        .unwrap_or(SceneRenderProfile::Gpu);

    let script = fs::read_to_string(&path)?;
    let graph = parse_graph_script(&script)?;
    let frame = pollster::block_on(render_scene_graph_frame(&graph, frame_index, profile))?;

    frame.save(&output)?;
    println!("saved frame {} to {}", frame_index, output.display());
    Ok(())
}

fn parse_profile(value: &str) -> Result<SceneRenderProfile, String> {
    match value {
        "cpu" => Ok(SceneRenderProfile::Cpu),
        "gpu" => Ok(SceneRenderProfile::Gpu),
        "prores" => Ok(SceneRenderProfile::GpuProRes),
        "prores4444" | "prores-4444" => Ok(SceneRenderProfile::GpuProRes4444),
        "png" | "png-sequence" => Ok(SceneRenderProfile::GpuPngSequence),
        other => Err(format!("unknown profile: {other}")),
    }
}
