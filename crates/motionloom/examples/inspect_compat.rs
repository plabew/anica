use motionloom::api::{GpuCompatibilitySeverity, inspect_gpu_compatibility};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let script = r##"
<Graph fps={30} duration="1s" size={[640,360]}>
  <Background color="#101827" />
  <Scene id="compat_scene">
    <Timeline>
      <Track id="main" space="world" z="0">
        <Sequence from="0s" duration="1s" out="hold">
          <Layer>
            <Circle x="320" y="180" radius="96" color="#4CC9F0" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="compat_scene" />
</Graph>
"##;

    let report = inspect_gpu_compatibility(script)?;
    println!("likely CPU fallback: {}", report.likely_cpu_fallback);
    println!("preview path: {:?}", report.likely_preview_path);

    for issue in &report.issues {
        let level = match issue.severity {
            GpuCompatibilitySeverity::Blocking => "blocking",
            GpuCompatibilitySeverity::Warning => "warning",
            GpuCompatibilitySeverity::Info => "info",
        };
        println!(
            "[{level}] {:?} {}: {}",
            issue.target, issue.code, issue.message
        );
    }

    Ok(())
}
