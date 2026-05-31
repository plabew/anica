use std::path::{Path, PathBuf};

use motionloom::{
    SceneRenderProfile, is_animation_graph_script, is_graph_script, parse_animation_graph_script,
    parse_graph_script, render_scene_graph_frame,
};

fn collect_motionloom_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == "temp_save") {
            continue;
        }

        if path.is_dir() {
            collect_motionloom_files(&path, out);
            continue;
        }

        if path.extension().is_some_and(|ext| ext == "motionloom") {
            out.push(path);
        }
    }
}

#[test]
fn bundled_motionloom_examples_parse() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/motionloom");
    let mut files = Vec::new();
    collect_motionloom_files(&root, &mut files);
    files.sort();

    assert!(!files.is_empty(), "expected bundled MotionLoom examples");

    for file in files {
        let script = std::fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));
        if !is_graph_script(&script) {
            continue;
        }
        if is_animation_graph_script(&script) {
            parse_animation_graph_script(&script).unwrap_or_else(|err| {
                panic!("failed to parse animation {}: {err}", file.display())
            });
        } else {
            parse_graph_script(&script)
                .unwrap_or_else(|err| panic!("failed to parse {}: {err}", file.display()));
        }
    }
}

#[test]
fn mask_matte_precompose_example_renders_gpu_profile() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../examples/motionloom/scene/motion_graphics/mask_matte_precompose_level1.motionloom",
    );
    let script = std::fs::read_to_string(&file)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));
    let graph = parse_graph_script(&script)
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", file.display()));

    render_scene_graph_frame(&graph, 0, SceneRenderProfile::Gpu)
        .unwrap_or_else(|err| panic!("GPU-profile render failed for {}: {err}", file.display()));
}

#[test]
fn font_family_hello_specimen_renders_cpu_and_gpu_profile() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../examples/motionloom/scene/motion_graphics/font_family_hello_specimen.motionloom",
    );
    let script = std::fs::read_to_string(&file)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));
    let graph = parse_graph_script(&script)
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", file.display()));

    render_scene_graph_frame(&graph, 0, SceneRenderProfile::Cpu)
        .unwrap_or_else(|err| panic!("CPU-profile render failed for {}: {err}", file.display()));
    render_scene_graph_frame(&graph, 0, SceneRenderProfile::Gpu)
        .unwrap_or_else(|err| panic!("GPU-profile render failed for {}: {err}", file.display()));
}

#[test]
fn pixel_cat_example_renders_cpu_and_gpu_profile() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/motionloom/scene/pixel/pixel_cat_level1.motionloom");
    let script = std::fs::read_to_string(&file)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));
    let graph = parse_graph_script(&script)
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", file.display()));

    render_scene_graph_frame(&graph, 0, SceneRenderProfile::Cpu)
        .unwrap_or_else(|err| panic!("CPU-profile render failed for {}: {err}", file.display()));
    render_scene_graph_frame(&graph, 0, SceneRenderProfile::Gpu)
        .unwrap_or_else(|err| panic!("GPU-profile render failed for {}: {err}", file.display()));
}

#[test]
fn bone_axis_map_parses_rest_offsets() {
    let graph = parse_animation_graph_script(
        r#"
<Graph fps={30} duration="1s" size={[64,64]}>
  <ModelProfile id="profile" model="actor.glb" preset="humanoid_v1">
    <BoneAxisMap>
      <Axis bone="upper_arm_l"
            side="rotationX:1"
            forward="rotationZ:1"
            restSide="-90"
            restForward="4" />
    </BoneAxisMap>
  </ModelProfile>
  <World id="stage">
    <Actor id="actor" model="actor.glb" profile="profile" />
  </World>
  <Present from="stage" />
</Graph>
"#,
    )
    .expect("parse animation graph");

    let axis = &graph.model_profiles[0]
        .bone_axis_map
        .as_ref()
        .expect("bone axis map")
        .axes[0];
    assert_eq!(axis.rest_side.as_deref(), Some("-90"));
    assert_eq!(axis.rest_forward.as_deref(), Some("4"));
}

#[test]
fn rest_pose_correction_tag_is_rejected() {
    let err = parse_animation_graph_script(
        r#"
<Graph fps={30} duration="1s" size={[64,64]}>
  <ModelProfile id="profile" model="actor.glb" preset="humanoid_v1">
    <BoneAxisMap>
      <Axis bone="upper_arm_l" side="rotationX:1" />
    </BoneAxisMap>
    <RestPoseCorrection>
      <Bone bone="upper_arm_l" side="-90" />
    </RestPoseCorrection>
  </ModelProfile>
  <World id="stage">
    <Actor id="actor" model="actor.glb" profile="profile" />
  </World>
  <Present from="stage" />
</Graph>
"#,
    )
    .expect_err("RestPoseCorrection must be rejected");

    assert!(
        err.to_string()
            .contains("RestPoseCorrection has been removed"),
        "{err}"
    );
}
