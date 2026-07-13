use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use motionloom::api::{SceneRenderProfile, SceneRenderer, parse_graph_script};
use serde::Serialize;

const DEFAULT_COUNTS: &[usize] = &[100, 1_000, 5_000, 10_000, 30_000];

#[derive(Clone, Copy, Debug)]
enum WorkloadMode {
    Static,
    Transform,
    Morph,
}

impl WorkloadMode {
    const ALL: [Self; 3] = [Self::Static, Self::Transform, Self::Morph];

    fn name(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Transform => "transform",
            Self::Morph => "morph",
        }
    }
}

#[derive(Debug)]
struct Options {
    counts: Vec<usize>,
    warmup: usize,
    samples: usize,
    width: u32,
    height: u32,
    json: Option<PathBuf>,
    emit_dsl: Option<PathBuf>,
}

#[derive(Serialize)]
struct Summary {
    benchmark: &'static str,
    resolution: [u32; 2],
    warmup: usize,
    samples: usize,
    present_note: &'static str,
    results: Vec<WorkloadResult>,
}

#[derive(Serialize)]
struct WorkloadResult {
    mode: &'static str,
    path_count: usize,
    segment_count: usize,
    parse_ms: Distribution,
    flatten_ms: Distribution,
    encode_ms: Distribution,
    gpu_ms: Distribution,
    present_ms: Option<Distribution>,
    primitive_count: u32,
    upload_bytes: usize,
    path_cache_hits: usize,
    path_cache_misses: usize,
}

#[derive(Serialize)]
struct Distribution {
    median: f64,
    p95: f64,
    min: f64,
    max: f64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = parse_options()?;
    let mut renderer = pollster::block_on(SceneRenderer::new(SceneRenderProfile::Gpu))?;
    let mut results = Vec::new();

    for mode in WorkloadMode::ALL {
        for &path_count in &options.counts {
            eprintln!("benchmarking {:>9} {:>6} paths...", mode.name(), path_count);
            let script = generate_workload(mode, path_count, options.width, options.height);
            if let Some(directory) = &options.emit_dsl {
                std::fs::create_dir_all(directory)?;
                std::fs::write(
                    directory.join(format!("{}-{}.motionloom", mode.name(), path_count)),
                    &script,
                )?;
            }

            let mut parse_samples = Vec::with_capacity(options.samples);
            let mut graph = None;
            for _ in 0..options.samples {
                let started = Instant::now();
                let parsed = parse_graph_script(&script)?;
                parse_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
                graph = Some(parsed);
            }
            let graph = graph.expect("samples is at least one");
            for warmup_index in 0..options.warmup {
                let frame = workload_frame(mode, warmup_index);
                pollster::block_on(renderer.benchmark_vector_frame(&graph, frame))?;
            }

            let mut flatten = Vec::with_capacity(options.samples);
            let mut encode = Vec::with_capacity(options.samples);
            let mut gpu = Vec::with_capacity(options.samples);
            let mut primitive_count = 0;
            let mut upload_bytes = 0;
            let mut path_cache_hits = 0;
            let mut path_cache_misses = 0;
            for sample_index in 0..options.samples {
                let frame = workload_frame(mode, options.warmup + sample_index);
                let sample = pollster::block_on(renderer.benchmark_vector_frame(&graph, frame))?;
                flatten.push(sample.flatten_ms);
                encode.push(sample.encode_ms);
                gpu.push(sample.gpu_ms);
                primitive_count = sample.primitive_count;
                upload_bytes = sample.upload_bytes;
                path_cache_hits = sample.path_cache_hits;
                path_cache_misses = sample.path_cache_misses;
            }

            results.push(WorkloadResult {
                mode: mode.name(),
                path_count,
                segment_count: path_count.saturating_mul(4),
                parse_ms: distribution(parse_samples),
                flatten_ms: distribution(flatten),
                encode_ms: distribution(encode),
                gpu_ms: distribution(gpu),
                present_ms: None,
                primitive_count,
                upload_bytes,
                path_cache_hits,
                path_cache_misses,
            });
        }
    }

    let summary = Summary {
        benchmark: "motionloom-path-dsl-paris-style",
        resolution: [options.width, options.height],
        warmup: options.warmup,
        samples: options.samples,
        present_note: "headless offscreen benchmark: no surface present is performed",
        results,
    };
    let json = serde_json::to_string_pretty(&summary)?;
    println!("{json}");
    if let Some(path) = options.json {
        std::fs::write(path, format!("{json}\n"))?;
    }
    Ok(())
}

fn workload_frame(mode: WorkloadMode, index: usize) -> u32 {
    match mode {
        WorkloadMode::Static => 0,
        WorkloadMode::Transform | WorkloadMode::Morph => (index as u32 % 59).saturating_add(1),
    }
}

fn generate_workload(mode: WorkloadMode, count: usize, width: u32, height: u32) -> String {
    let mut script = String::with_capacity(count.saturating_mul(260));
    writeln!(
        script,
        "<Graph fps={{30}} duration=\"2s\" size={{[{width},{height}]}} renderSize={{[{width},{height}]}}>"
    )
    .unwrap();
    script.push_str("  <Background color=\"#10151D\" />\n  <Scene id=\"PathBenchmark\">\n    <Timeline>\n      <Track id=\"paths\" space=\"screen\" z=\"0\">\n        <Sequence from=\"0s\" duration=\"2s\" out=\"hold\">\n          <Layer>\n");
    let columns = ((count as f64).sqrt().ceil() as usize).max(1);
    let cell_w = width as f32 / columns as f32;
    let rows = count.div_ceil(columns).max(1);
    let cell_h = height as f32 / rows as f32;
    let radius = (cell_w.min(cell_h) * 0.38).clamp(1.5, 18.0);

    for index in 0..count {
        let column = index % columns;
        let row = index / columns;
        let x = (column as f32 + 0.5) * cell_w;
        let y = (row as f32 + 0.5) * cell_h;
        let hue = index.wrapping_mul(47) % 255;
        let color = format!(
            "#{:02X}{:02X}{:02X}",
            64 + hue / 3,
            96 + hue / 4,
            180 + hue / 5
        );
        let path_a = diamond_d(radius, 1.0);
        match mode {
            WorkloadMode::Static => {
                writeln!(
                    script,
                    "            <Path id=\"p{index}\" x=\"{x:.3}\" y=\"{y:.3}\" d=\"{path_a}\" fill=\"{color}\" />"
                )
                .unwrap();
            }
            WorkloadMode::Transform => {
                writeln!(
                    script,
                    "            <Group id=\"g{index}\" x={{curve(\"0:{x:.3}:linear, 2:{:.3}:ease_in_out\")}} y=\"{y:.3}\" rotation={{curve(\"0:0:linear, 2:24:ease_in_out\")}}>\n              <Path id=\"p{index}\" d=\"{path_a}\" fill=\"{color}\" />\n            </Group>",
                    x + radius * 0.5
                )
                .unwrap();
            }
            WorkloadMode::Morph => {
                let path_b = diamond_d(radius, 0.62);
                writeln!(
                    script,
                    "            <Path id=\"p{index}\" x=\"{x:.3}\" y=\"{y:.3}\" d={{morph(\"0:{path_a}\", \"2:{path_b}\")}} fill=\"{color}\" />"
                )
                .unwrap();
            }
        }
    }
    script.push_str("          </Layer>\n        </Sequence>\n      </Track>\n    </Timeline>\n  </Scene>\n  <Present from=\"PathBenchmark\" />\n</Graph>\n");
    script
}

// Four line segments keep the 30K baseline bounded and comparable. Curved
// flattening deserves a separate stress profile because its primitive output
// depends heavily on tolerance and scale.
fn diamond_d(radius: f32, squash: f32) -> String {
    let x = radius;
    let y = radius * squash;
    format!("M 0 -{y:.3} L {x:.3} 0 L 0 {y:.3} L -{x:.3} 0 Z")
}

fn distribution(mut values: Vec<f64>) -> Distribution {
    values.sort_by(f64::total_cmp);
    let percentile = |p: f64| {
        let index = ((values.len() - 1) as f64 * p).ceil() as usize;
        values[index.min(values.len() - 1)]
    };
    Distribution {
        median: percentile(0.50),
        p95: percentile(0.95),
        min: values[0],
        max: values[values.len() - 1],
    }
}

fn parse_options() -> Result<Options, Box<dyn std::error::Error>> {
    let mut options = Options {
        counts: DEFAULT_COUNTS.to_vec(),
        warmup: 2,
        samples: 10,
        width: 1_600,
        height: 1_600,
        json: None,
        emit_dsl: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--counts" => {
                let value = args
                    .next()
                    .ok_or("--counts requires a comma-separated value")?;
                options.counts = value
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<Vec<usize>, _>>()?;
            }
            "--warmup" => {
                options.warmup = args.next().ok_or("--warmup requires a value")?.parse()?
            }
            "--samples" => {
                options.samples = args.next().ok_or("--samples requires a value")?.parse()?
            }
            "--size" => {
                let value = args.next().ok_or("--size requires WIDTHxHEIGHT")?;
                let (width, height) = value
                    .split_once('x')
                    .ok_or("--size requires WIDTHxHEIGHT")?;
                options.width = width.parse()?;
                options.height = height.parse()?;
            }
            "--json" => options.json = Some(args.next().ok_or("--json requires a path")?.into()),
            "--emit-dsl" => {
                options.emit_dsl =
                    Some(args.next().ok_or("--emit-dsl requires a directory")?.into())
            }
            "--help" | "-h" => {
                println!(
                    "path_dsl_benchmark [--counts 100,1000,5000,10000,30000] [--warmup 2] [--samples 10] [--size 1600x1600] [--json result.json] [--emit-dsl directory]"
                );
                std::process::exit(0);
            }
            _ => return Err(format!("unknown option: {arg}").into()),
        }
    }
    if options.counts.is_empty()
        || options.samples == 0
        || options.width == 0
        || options.height == 0
    {
        return Err("counts/samples/size must be non-zero".into());
    }
    Ok(options)
}
