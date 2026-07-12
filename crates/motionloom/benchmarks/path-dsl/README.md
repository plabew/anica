# MotionLoom Path DSL benchmark

This is a Paris-30K-style throughput benchmark for MotionLoom's production
Scene WGPU path. It generates valid MotionLoom DSL at 100, 1K, 5K, 10K, and
30K Path nodes for three independent workloads:

- `static`: fixed Path geometry and transforms.
- `transform`: fixed Path geometry with animated Group translation/rotation.
- `morph`: animated Path `d={morph(...)}` geometry.

Every generated Path has four segments. The baseline uses line segments so
30K remains a path-throughput test rather than an unbounded curved-flattening
memory test.

## Run

```bash
cargo run --release -p motionloom --example path_dsl_benchmark -- \
  --counts 100,1000,5000,10000,30000 \
  --warmup 2 \
  --samples 10 \
  --size 1600x1600 \
  --json target/path-dsl-benchmark.json
```

Use `--emit-dsl target/path-dsl-workloads` to retain all generated `.motionloom`
files for inspection or execution in other MotionLoom hosts.

## Metrics

- `parse_ms`: complete DSL parse into `GraphScript`.
- `flatten_ms`: expression/morph evaluation, Path parsing, flattening, and
  fill/stroke tessellation into GPU primitives.
- `encode_ms`: tile binning, upload-byte packing, GPU buffer creation, and WGPU
  command encoding.
- `gpu_ms`: queue submission through completion of the submitted GPU work.
- `present_ms`: `null` in this headless benchmark because no surface present is
  performed. It is intentionally not approximated with an offscreen copy.
- `path_count`, `segment_count`, `primitive_count`, and `upload_bytes`: workload
  and actual renderer-output counters.

Each timing reports median, p95, minimum, and maximum. Renderer/device creation
is excluded; warmups occur before recorded frame samples.
