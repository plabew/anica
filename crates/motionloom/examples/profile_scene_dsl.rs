// =========================================
// =========================================
// crates/motionloom/examples/profile_scene_dsl.rs

use std::path::PathBuf;
use std::time::Instant;

use motionloom::api::{SceneRenderProfile, SceneRenderer, parse_graph_script};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (path, samples) = parse_options()?;
    let script = std::fs::read_to_string(&path)?;

    let parse_started = Instant::now();
    let graph = parse_graph_script(&script)?;
    let parse_ms = parse_started.elapsed().as_secs_f64() * 1_000.0;
    let mut renderer = pollster::block_on(SceneRenderer::new(SceneRenderProfile::Gpu))?;

    println!(
        "file={} bytes={} parse_ms={parse_ms:.3}",
        path.display(),
        script.len()
    );
    for sample in 0..samples {
        let started = Instant::now();
        let profile = pollster::block_on(renderer.benchmark_vector_frame(&graph, 0))?;
        let total_ms = started.elapsed().as_secs_f64() * 1_000.0;
        println!(
            "sample={} total_ms={total_ms:.3} flatten_ms={:.3} encode_ms={:.3} gpu_ms={:.3} primitives={} upload_bytes={} path_hits={} path_misses={}",
            sample + 1,
            profile.flatten_ms,
            profile.encode_ms,
            profile.gpu_ms,
            profile.primitive_count,
            profile.upload_bytes,
            profile.path_cache_hits,
            profile.path_cache_misses,
        );
    }
    Ok(())
}

fn parse_options() -> Result<(PathBuf, usize), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: profile_scene_dsl <file.motionloom> [--samples N]")?;
    let mut samples = 3usize;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--samples" => {
                samples = args.next().ok_or("--samples requires a value")?.parse()?;
            }
            "--help" | "-h" => {
                println!("profile_scene_dsl <file.motionloom> [--samples 3]");
                std::process::exit(0);
            }
            _ => return Err(format!("unknown option: {arg}").into()),
        }
    }
    if samples == 0 {
        return Err("samples must be non-zero".into());
    }
    Ok((path.into(), samples))
}
