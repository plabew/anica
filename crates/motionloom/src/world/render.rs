use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use image::{Rgba, RgbaImage, imageops};
use thiserror::Error;

use crate::asset::{AssetResolver, AssetSource, PathAssetResolver};
use crate::common::gpu_async::{
    BufferMapAsyncFuture, DevicePoller, request_adapter_async, request_device_async,
};
use crate::process::runtime::eval_time_expr;
use crate::scene::render::SceneRenderProfile;
use crate::world::gltf_loader::{
    GlbLoadError, GlbMeshData, GlbTextureData, GlbTriangle, load_glb_mesh_data,
    load_glb_mesh_data_from_bytes,
};
use crate::world::model::{
    WorldAction, WorldActionBone, WorldActor, WorldBackgroundFit, WorldBoneAxis, WorldBoneAxisMap,
    WorldDirectionFrame, WorldDirectionalCharacter, WorldGraph, WorldMaterialStyle,
    WorldModelProfile, WorldNode, WorldPathStyle, WorldRetargetMap, WorldSpritePlayback, WorldTime,
};

#[derive(Debug, Error)]
pub enum WorldRenderError {
    #[error("world graph has no presented world '{0}'")]
    MissingWorld(String),
    #[error("failed to load background image {path}: {source}")]
    BackgroundImage {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
    #[error("failed to load directional character sheet {path}: {source}")]
    DirectionalCharacterImage {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
    #[error("directional character sheet does not exist: {0}")]
    MissingDirectionalCharacterImage(PathBuf),
    #[error("failed to load GLB model: {0}")]
    Glb(#[from] GlbLoadError),
    #[error("invalid world expression '{expr}': {message}")]
    Expression { expr: String, message: String },
    #[error("failed to create output directory ({path}): {source}")]
    CreateOutputDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to start ffmpeg: {source}")]
    StartFfmpeg {
        #[source]
        source: std::io::Error,
    },
    #[error("ffmpeg stdin was not available")]
    MissingFfmpegStdin,
    #[error("failed to write raw frame to ffmpeg: {source}. ffmpeg stderr: {stderr}")]
    WriteFrame {
        #[source]
        source: std::io::Error,
        stderr: String,
    },
    #[error("failed to wait for ffmpeg: {source}")]
    WaitFfmpeg {
        #[source]
        source: std::io::Error,
    },
    #[error("ffmpeg failed: {stderr}")]
    FfmpegFailed { stderr: String },
    #[error("failed to save PNG frame ({path}): {source}")]
    SavePngFrame {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
    #[error("world GPU render failed: {message}")]
    GpuRender { message: String },
    #[error("video export is not available on this platform: {message}")]
    VideoExportNotAvailable { message: String },
    #[error("world render cancelled")]
    Cancelled,
}

impl From<crate::export::EncodeError> for WorldRenderError {
    fn from(err: crate::export::EncodeError) -> Self {
        use crate::export::EncodeError;
        match err {
            EncodeError::CreateOutputDir { path, source } => Self::CreateOutputDir { path, source },
            EncodeError::StartEncoder(message) => Self::StartFfmpeg {
                source: std::io::Error::other(message),
            },
            EncodeError::MissingEncoderInput => Self::MissingFfmpegStdin,
            EncodeError::WriteFrame(source) => Self::WriteFrame {
                source,
                stderr: String::new(),
            },
            EncodeError::EncoderFailed(stderr) => Self::FfmpegFailed { stderr },
            EncodeError::NotImplemented(message) => Self::VideoExportNotAvailable { message },
            EncodeError::NotStarted => Self::GpuRender {
                message: "encoder was not started".to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldRenderProgress {
    pub rendered_frames: u32,
    pub total_frames: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldGpuDiagnostics {
    pub mesh_loaded: bool,
    pub vertex_count: usize,
    pub triangle_count: usize,
    pub material_count: usize,
    pub texture_count: usize,
    pub decoded_texture_count: usize,
    pub skin_joint_count: usize,
    pub gpu_draw_count: usize,
    pub gpu_vertex_count: usize,
    pub bone_override_count: usize,
    pub projected_bounds: Option<String>,
    pub projected_inside_count: usize,
    pub projected_nonfinite_count: usize,
    pub ndc_z_range: Option<String>,
    pub depth_pass_estimate_count: usize,
    pub depth_reject_estimate_count: usize,
    pub alpha_sample_count: usize,
    pub alpha_visible_sample_count: usize,
    pub alpha_zero_sample_count: usize,
    pub alpha_range: Option<String>,
    pub uv_outside_sample_count: usize,
    pub raw_draw_bounds: Option<String>,
    pub shader_local_bounds: Option<String>,
    pub shader_projected_bounds: Option<String>,
    pub shader_projected_inside_count: usize,
    pub shader_projected_nonfinite_count: usize,
    pub shader_joint_oob_count: usize,
    pub skipped_reasons: Vec<String>,
}

pub async fn render_world_frame(
    graph: &WorldGraph,
    frame: u32,
    asset_root: impl AsRef<Path>,
) -> Result<RgbaImage, WorldRenderError> {
    WorldFrameRenderer::new().render_frame(graph, frame, asset_root)
}

pub fn diagnose_world_glb_gpu_plan(mesh: &GlbMeshData) -> WorldGpuDiagnostics {
    let texture_count = mesh.textures.len();
    let decoded_texture_count = mesh
        .textures
        .iter()
        .filter(|texture| texture.is_some())
        .count();
    let skin_joint_count = mesh.skin.as_ref().map_or(0, |skin| skin.joints.len());
    let mut skipped_reasons = Vec::<String>::new();

    if mesh.positions.is_empty() {
        skipped_reasons.push("mesh has 0 positions; actor cannot create GPU vertices".to_string());
    }
    if mesh.triangles.is_empty() {
        skipped_reasons.push("mesh has 0 triangles; actor has nothing to draw".to_string());
    }

    let mut transparent_materials = 0usize;
    let mut missing_texture_materials = 0usize;
    for material in &mesh.materials {
        if material.base_color_factor[3] <= 0.001 {
            transparent_materials += 1;
        }
        if let Some(texture_index) = material.base_color_texture
            && mesh
                .textures
                .get(texture_index)
                .and_then(Option::as_ref)
                .is_none()
        {
            missing_texture_materials += 1;
        }
    }
    if transparent_materials > 0 {
        skipped_reasons.push(format!(
            "{transparent_materials} material(s) have near-zero base alpha; if draw count is non-zero, invisibility may be alpha/material related"
        ));
    }
    if missing_texture_materials > 0 {
        skipped_reasons.push(format!(
            "{missing_texture_materials} material(s) reference missing/undecoded textures; GPU will use flat fallback color"
        ));
    }

    let mut chunks = HashMap::<GpuWorldDrawKey, usize>::new();
    let mut invalid_index_triangles = 0usize;
    let mut missing_uv_textured_triangles = 0usize;
    let mut alpha_sample_count = 0usize;
    let mut alpha_visible_sample_count = 0usize;
    let mut alpha_zero_sample_count = 0usize;
    let mut uv_outside_sample_count = 0usize;
    let mut min_alpha = f32::INFINITY;
    let mut max_alpha = f32::NEG_INFINITY;
    for triangle in &mesh.triangles {
        let has_invalid_index = triangle
            .indices
            .iter()
            .any(|index| *index as usize >= mesh.positions.len());
        if has_invalid_index {
            invalid_index_triangles += 1;
            continue;
        }

        let material = triangle
            .material
            .and_then(|index| mesh.materials.get(index));
        let texture = material
            .and_then(|material| material.base_color_texture)
            .and_then(|texture_index| {
                mesh.textures
                    .get(texture_index)
                    .and_then(Option::as_ref)
                    .map(|_| texture_index)
            });
        if texture.is_some()
            && triangle.indices.iter().any(|index| {
                mesh.texcoords
                    .get(*index as usize)
                    .and_then(|uv| *uv)
                    .is_none()
            })
        {
            missing_uv_textured_triangles += 1;
        }

        let triangle_uvs = [
            mesh.texcoords
                .get(triangle.indices[0] as usize)
                .copied()
                .flatten(),
            mesh.texcoords
                .get(triangle.indices[1] as usize)
                .copied()
                .flatten(),
            mesh.texcoords
                .get(triangle.indices[2] as usize)
                .copied()
                .flatten(),
        ];
        if let Some((texture, material_factor, uvs)) =
            textured_triangle_source(mesh, triangle.material, triangle_uvs)
        {
            let centroid = [
                (uvs[0][0] + uvs[1][0] + uvs[2][0]) / 3.0,
                (uvs[0][1] + uvs[1][1] + uvs[2][1]) / 3.0,
            ];
            for uv in [uvs[0], uvs[1], uvs[2], centroid] {
                if uv[0] < 0.0 || uv[0] > 1.0 || uv[1] < 0.0 || uv[1] > 1.0 {
                    uv_outside_sample_count += 1;
                }
                let alpha = sampled_texture_alpha(texture, uv, material_factor);
                alpha_sample_count += 1;
                min_alpha = min_alpha.min(alpha);
                max_alpha = max_alpha.max(alpha);
                if alpha <= 0.001 {
                    alpha_zero_sample_count += 1;
                } else {
                    alpha_visible_sample_count += 1;
                }
            }
        }

        let key = GpuWorldDrawKey {
            material: triangle.material,
            texture,
            mesh_node: triangle.mesh_node,
        };
        *chunks.entry(key).or_insert(0) += 3;
    }

    let gpu_draw_count = chunks.values().filter(|vertices| **vertices > 0).count();
    let gpu_vertex_count = chunks.values().sum();
    if invalid_index_triangles > 0 {
        skipped_reasons.push(format!(
            "{invalid_index_triangles} triangle(s) skipped because they reference missing vertex positions"
        ));
    }
    if missing_uv_textured_triangles > 0 {
        skipped_reasons.push(format!(
            "{missing_uv_textured_triangles} textured triangle(s) have missing UVs; shader may sample texture corner"
        ));
    }
    if alpha_sample_count > 0 && alpha_visible_sample_count == 0 {
        skipped_reasons.push(
            "CPU texture alpha probe found 0 visible alpha samples; fragment shader will discard all sampled pixels"
                .to_string(),
        );
    } else if alpha_sample_count > 0 && alpha_visible_sample_count < alpha_sample_count / 20 {
        skipped_reasons.push(format!(
            "CPU texture alpha probe found very few visible samples ({alpha_visible_sample_count}/{alpha_sample_count}); alpha/UV path is suspicious"
        ));
    }
    if uv_outside_sample_count > 0 {
        skipped_reasons.push(format!(
            "{uv_outside_sample_count}/{alpha_sample_count} texture alpha probe sample(s) use UV outside 0..1; shader clamps these to texture edges"
        ));
    }
    if gpu_draw_count == 0 && !mesh.triangles.is_empty() {
        skipped_reasons.push(format!(
            "0 GPU draw chunks generated from {} triangle(s); inspect primitive indices/material grouping",
            mesh.triangles.len()
        ));
    } else if gpu_draw_count > 0 {
        skipped_reasons.push(
            "GPU draw chunks generated; if preview is still blank, inspect alpha/depth/texture/shader path"
                .to_string(),
        );
    }

    WorldGpuDiagnostics {
        mesh_loaded: true,
        vertex_count: mesh.positions.len(),
        triangle_count: mesh.triangles.len(),
        material_count: mesh.materials.len(),
        texture_count,
        decoded_texture_count,
        skin_joint_count,
        gpu_draw_count,
        gpu_vertex_count,
        bone_override_count: 0,
        projected_bounds: None,
        projected_inside_count: 0,
        projected_nonfinite_count: 0,
        ndc_z_range: None,
        depth_pass_estimate_count: 0,
        depth_reject_estimate_count: 0,
        alpha_sample_count,
        alpha_visible_sample_count,
        alpha_zero_sample_count,
        alpha_range: if min_alpha.is_finite() && max_alpha.is_finite() {
            Some(format!("{min_alpha:.3}..{max_alpha:.3}"))
        } else {
            None
        },
        uv_outside_sample_count,
        raw_draw_bounds: None,
        shader_local_bounds: None,
        shader_projected_bounds: None,
        shader_projected_inside_count: 0,
        shader_projected_nonfinite_count: 0,
        shader_joint_oob_count: 0,
        skipped_reasons,
    }
}

pub fn diagnose_world_graph_actor_gpu_frame(
    graph: &WorldGraph,
    actor_id: &str,
    frame: u32,
    asset_root: impl AsRef<Path>,
) -> Result<WorldGpuDiagnostics, WorldRenderError> {
    let asset_root = asset_root.as_ref();
    let world = graph
        .presented_world()
        .ok_or_else(|| WorldRenderError::MissingWorld(graph.present.from.clone()))?;
    let actor = world
        .actors
        .iter()
        .find(|actor| actor.id == actor_id)
        .or_else(|| world.actors.first())
        .ok_or_else(|| WorldRenderError::GpuRender {
            message: "GPU diagnostics found no Actor in presented world".to_string(),
        })?;
    let (model_key, mesh) = load_glb_mesh_resolved(
        asset_root,
        &actor.model,
        actor.path_style,
        &PathAssetResolver,
    )?;
    let mut diagnostics = diagnose_world_glb_gpu_plan(&mesh);
    let time = WorldTime {
        frame,
        fps: graph.fps,
        duration_ms: graph.duration_ms,
    };

    let overrides = actor_bone_overrides(graph, actor, time)?;
    diagnostics.bone_override_count = overrides.len();

    let positions = skinned_actor_positions(graph, actor, &mesh, time)?
        .unwrap_or_else(|| mesh.positions.clone());
    let (width, height) = graph.output_size();
    let width_f = width.max(1) as f32;
    let height_f = height.max(1) as f32;
    let camera_yaw = eval_number(&world.camera.yaw, 0.0, time)?;
    let camera_pitch = eval_number(&world.camera.pitch, 0.0, time)?;
    let camera_x = eval_number(&world.camera.x, 0.0, time)?;
    let camera_y = eval_number(&world.camera.y, 0.0, time)?;
    let camera_z = eval_number(&world.camera.z, 0.0, time)?;
    let camera_zoom = eval_number(&world.camera.zoom, 1.0, time)?.max(0.05);
    let fov = eval_number(&world.camera.fov, 35.0, time)?.clamp(10.0, 100.0);
    let distance = eval_number(&world.camera.distance, 3.2, time)?.max(0.2);
    let actor_x = eval_number(&actor.x, 0.0, time)?;
    let actor_y = eval_number(&actor.y, 0.0, time)?;
    let actor_z = eval_number(&actor.z, 0.0, time)?;
    let actor_yaw = eval_number(&actor.yaw, 0.0, time)?;
    let actor_scale = eval_number(&actor.scale, 1.0, time)?.max(0.01) * camera_zoom;
    let view = camera_actor_view(
        actor_x,
        actor_y,
        actor_z,
        actor_yaw,
        camera_x,
        camera_y,
        camera_z,
        camera_yaw,
        camera_pitch,
    );

    let model_height = (mesh.bounds_max[1] - mesh.bounds_min[1]).abs().max(0.001);
    let model_width = (mesh.bounds_max[0] - mesh.bounds_min[0]).abs().max(0.001);
    let model_depth = (mesh.bounds_max[2] - mesh.bounds_min[2]).abs().max(0.001);
    let model_center_x = (mesh.bounds_min[0] + mesh.bounds_max[0]) * 0.5;
    let model_center_z = (mesh.bounds_min[2] + mesh.bounds_max[2]) * 0.5;
    let px_per_world = (height_f / distance) * (35.0 / fov).clamp(0.35, 2.5);
    let model_px =
        (height_f * 0.58 * actor_scale * (3.2 / distance).clamp(0.25, 4.0)) / model_height;
    let cx = width_f * 0.5 + view.x * px_per_world;
    let ground_y = height_f * 0.82 - view.y * px_per_world;
    let yaw = view.yaw.to_radians();
    let cos_y = yaw.cos();
    let sin_y = yaw.sin();
    let pitch = camera_pitch.to_radians();
    let cos_p = pitch.cos();
    let sin_p = pitch.sin();
    let depth_scale = 0.45 / model_width.max(model_depth);

    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut inside = 0usize;
    let mut nonfinite = 0usize;
    let mut min_z = f32::INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    let mut depth_pass = 0usize;
    let mut depth_reject = 0usize;
    for position in positions {
        let x = position[0] - model_center_x;
        let y = position[1] - mesh.bounds_min[1];
        let z = position[2] - model_center_z;
        let rx = x * cos_y + z * sin_y;
        let rz = -x * sin_y + z * cos_y;
        let ry = y * cos_p - rz * sin_p;
        let rz = y * sin_p + rz * cos_p + view.depth * WORLD_DEPTH_SORT_SCALE;
        let screen_x = cx + rx * model_px;
        let screen_y = ground_y - ry * model_px;
        let ndc_z = (0.5 + rz * depth_scale).clamp(0.0, 1.0);
        if !screen_x.is_finite() || !screen_y.is_finite() || !ndc_z.is_finite() {
            nonfinite += 1;
            continue;
        }
        min_x = min_x.min(screen_x);
        min_y = min_y.min(screen_y);
        max_x = max_x.max(screen_x);
        max_y = max_y.max(screen_y);
        min_z = min_z.min(ndc_z);
        max_z = max_z.max(ndc_z);
        if ndc_z > 0.0 {
            depth_pass += 1;
        } else {
            depth_reject += 1;
        }
        if screen_x >= 0.0 && screen_x <= width_f && screen_y >= 0.0 && screen_y <= height_f {
            inside += 1;
        }
    }

    diagnostics.projected_inside_count = inside;
    diagnostics.projected_nonfinite_count = nonfinite;
    diagnostics.depth_pass_estimate_count = depth_pass;
    diagnostics.depth_reject_estimate_count = depth_reject;
    if min_z.is_finite() && max_z.is_finite() {
        diagnostics.ndc_z_range = Some(format!("{min_z:.3}..{max_z:.3}"));
        if depth_pass == 0 {
            diagnostics.skipped_reasons.push(
                "estimated GPU depth test rejects all vertices with current Greater/clear(0.0) setup"
                    .to_string(),
            );
        }
    }
    if min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite() {
        diagnostics.projected_bounds = Some(format!(
            "x {:.1}..{:.1}, y {:.1}..{:.1} on {}x{}",
            min_x, max_x, min_y, max_y, width, height
        ));
        if inside == 0 {
            diagnostics.skipped_reasons.push(
                "projected screen bbox has 0 vertices inside viewport; inspect actor/camera/skin transform"
                    .to_string(),
            );
        } else {
            diagnostics.skipped_reasons.push(format!(
                "{inside} projected vertex/vertices are inside viewport before GPU shader"
            ));
        }
    } else {
        diagnostics.skipped_reasons.push(
            "projected screen bbox is non-finite/empty; inspect bone matrices and node transforms"
                .to_string(),
        );
    }

    diagnose_gpu_shader_projection(
        graph,
        actor,
        &mesh,
        &model_key,
        width,
        height,
        world,
        time,
        &mut diagnostics,
    )?;

    Ok(diagnostics)
}

#[allow(clippy::too_many_arguments)]
fn diagnose_gpu_shader_projection(
    graph: &WorldGraph,
    actor: &WorldActor,
    mesh: &GlbMeshData,
    model_path: &Path,
    width: u32,
    height: u32,
    world: &WorldNode,
    time: WorldTime,
    diagnostics: &mut WorldGpuDiagnostics,
) -> Result<(), WorldRenderError> {
    let width_f = width.max(1) as f32;
    let height_f = height.max(1) as f32;
    let camera_zoom = eval_number(&world.camera.zoom, 1.0, time)?.max(0.05);
    let camera_view = perspective_camera_view(world, width, height, time)?;
    let actor_x = eval_number(&actor.x, 0.0, time)?;
    let actor_y = eval_number(&actor.y, 0.0, time)?;
    let actor_z = eval_number(&actor.z, 0.0, time)?;
    let actor_yaw = eval_number(&actor.yaw, 0.0, time)?;
    let actor_pitch = eval_number(&actor.pitch, 0.0, time)?;
    let actor_roll = eval_number(&actor.roll, 0.0, time)?;
    let actor_scale = eval_number(&actor.scale, 1.0, time)?.max(0.01) * camera_zoom;
    let actor_opacity = eval_number(&actor.opacity, 1.0, time)?.clamp(0.0, 1.0);
    let static_draws = build_actor_mesh_gpu_static_draws(actor, mesh, model_path);
    let mut skinning_strategy_cache = HashMap::new();
    let draw_calls = build_actor_mesh_gpu_draws(
        graph,
        actor,
        mesh,
        &static_draws,
        width,
        height,
        actor_x,
        actor_y,
        actor_z,
        actor_yaw,
        actor_pitch,
        actor_roll,
        camera_view,
        actor_scale,
        actor_opacity,
        time,
        model_path,
        &mut skinning_strategy_cache,
    )?;

    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut inside = 0usize;
    let mut nonfinite = 0usize;
    let mut joint_oob = 0usize;
    let mut raw_min = [f32::INFINITY; 3];
    let mut raw_max = [f32::NEG_INFINITY; 3];
    let mut local_min = [f32::INFINITY; 3];
    let mut local_max = [f32::NEG_INFINITY; 3];
    for draw in &draw_calls {
        for vertex in draw.vertices.iter() {
            accumulate_bounds3(&mut raw_min, &mut raw_max, vertex.position);
            let skinned = simulate_gpu_vertex_skinning(vertex, &draw.bone_matrices, &mut joint_oob);
            accumulate_bounds3(&mut local_min, &mut local_max, skinned);
            let local = [
                (skinned[0] - draw.params.model[0]) * draw.params.model[3],
                (skinned[1] - draw.params.model[1]) * draw.params.model[3],
                (skinned[2] - draw.params.model[2]) * draw.params.model[3],
            ];
            let actor_cos = draw.params.actor[3].cos();
            let actor_sin = draw.params.actor[3].sin();
            let world = [
                draw.params.actor[0] + local[0] * actor_cos + local[2] * actor_sin,
                draw.params.actor[1] + local[1],
                draw.params.actor[2] - local[0] * actor_sin + local[2] * actor_cos,
            ];
            let rel = [
                world[0] - draw.params.camera0[0],
                world[1] - draw.params.camera0[1],
                world[2] - draw.params.camera0[2],
            ];
            let view_x = dot3(
                rel,
                [
                    draw.params.camera1[0],
                    draw.params.camera1[1],
                    draw.params.camera1[2],
                ],
            );
            let view_y = dot3(
                rel,
                [
                    draw.params.camera2[0],
                    draw.params.camera2[1],
                    draw.params.camera2[2],
                ],
            );
            let view_z = dot3(
                rel,
                [
                    draw.params.camera3[0],
                    draw.params.camera3[1],
                    draw.params.camera3[2],
                ],
            )
            .max(draw.params.camera1[3]);
            let screen_x = draw.params.canvas[2] + view_x * draw.params.camera0[3] / view_z;
            let screen_y = draw.params.canvas[3] - view_y * draw.params.camera0[3] / view_z;
            if !screen_x.is_finite() || !screen_y.is_finite() {
                nonfinite += 1;
                continue;
            }
            min_x = min_x.min(screen_x);
            min_y = min_y.min(screen_y);
            max_x = max_x.max(screen_x);
            max_y = max_y.max(screen_y);
            if screen_x >= 0.0 && screen_x <= width_f && screen_y >= 0.0 && screen_y <= height_f {
                inside += 1;
            }
        }
    }
    if raw_min[0].is_finite() && raw_max[0].is_finite() {
        diagnostics.raw_draw_bounds = Some(format_bounds3(raw_min, raw_max));
    }
    if local_min[0].is_finite() && local_max[0].is_finite() {
        diagnostics.shader_local_bounds = Some(format_bounds3(local_min, local_max));
    }
    diagnostics.shader_projected_inside_count = inside;
    diagnostics.shader_projected_nonfinite_count = nonfinite;
    diagnostics.shader_joint_oob_count = joint_oob;
    if min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite() {
        diagnostics.shader_projected_bounds = Some(format!(
            "x {:.1}..{:.1}, y {:.1}..{:.1} on {}x{}",
            min_x, max_x, min_y, max_y, width, height
        ));
        if inside == 0 {
            diagnostics.skipped_reasons.push(
                "simulated GPU vertex shader projects 0 vertices inside viewport; inspect joint matrices / mesh inverse / actor transform"
                    .to_string(),
            );
        }
    } else {
        diagnostics.skipped_reasons.push(
            "simulated GPU vertex shader projection is empty/non-finite; inspect joint matrices and vertex attributes"
                .to_string(),
        );
    }
    if joint_oob > 0 {
        diagnostics.skipped_reasons.push(format!(
            "simulated GPU shader saw {joint_oob} joint reference(s) outside current bone matrix buffer"
        ));
    }
    Ok(())
}

fn simulate_gpu_vertex_skinning(
    vertex: &GpuWorldVertex,
    bone_matrices: &[[f32; 16]],
    joint_oob: &mut usize,
) -> [f32; 3] {
    let weight_sum = vertex.weights[0] + vertex.weights[1] + vertex.weights[2] + vertex.weights[3];
    if weight_sum <= 0.000001 {
        return vertex.position;
    }
    let mut out = [0.0f32; 3];
    for slot in 0..4 {
        let weight = vertex.weights[slot] / weight_sum;
        if weight <= 0.0 {
            continue;
        }
        let joint_index = (vertex.joints[slot] + 0.5).max(0.0) as usize;
        let Some(matrix) = bone_matrices.get(joint_index) else {
            *joint_oob += 1;
            continue;
        };
        let transformed = mat4_transform_point(*matrix, vertex.position);
        out[0] += transformed[0] * weight;
        out[1] += transformed[1] * weight;
        out[2] += transformed[2] * weight;
    }
    out
}

fn accumulate_bounds3(min: &mut [f32; 3], max: &mut [f32; 3], point: [f32; 3]) {
    for axis in 0..3 {
        min[axis] = min[axis].min(point[axis]);
        max[axis] = max[axis].max(point[axis]);
    }
}

fn format_bounds3(min: [f32; 3], max: [f32; 3]) -> String {
    format!(
        "x {:.3}..{:.3}, y {:.3}..{:.3}, z {:.3}..{:.3}",
        min[0], max[0], min[1], max[1], min[2], max[2]
    )
}

pub struct WorldFrameRenderer {
    asset_resolver: Arc<dyn AssetResolver>,
    image_cache: HashMap<PathBuf, RgbaImage>,
    mesh_cache: HashMap<PathBuf, GlbMeshData>,
    gpu_static_draw_cache: HashMap<GpuWorldStaticPlanKey, Vec<GpuWorldStaticDraw>>,
    skinning_strategy_cache: HashMap<SkinningStrategyKey, SkinningMatrixStrategy>,
    gpu_renderer: Option<GpuWorldRenderer>,
}

impl Default for WorldFrameRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldFrameRenderer {
    pub fn new() -> Self {
        Self::with_resolver(Arc::new(PathAssetResolver))
    }

    pub fn with_resolver(asset_resolver: Arc<dyn AssetResolver>) -> Self {
        Self {
            asset_resolver,
            image_cache: HashMap::new(),
            mesh_cache: HashMap::new(),
            gpu_static_draw_cache: HashMap::new(),
            skinning_strategy_cache: HashMap::new(),
            gpu_renderer: None,
        }
    }

    pub fn render_frame(
        &mut self,
        graph: &WorldGraph,
        frame: u32,
        asset_root: impl AsRef<Path>,
    ) -> Result<RgbaImage, WorldRenderError> {
        let asset_root = asset_root.as_ref();
        let (width, height) = graph.output_size();
        let mut canvas = RgbaImage::from_pixel(width.max(1), height.max(1), Rgba([0, 0, 0, 255]));
        let world = graph
            .presented_world()
            .ok_or_else(|| WorldRenderError::MissingWorld(graph.present.from.clone()))?;
        let time = WorldTime {
            frame,
            fps: graph.fps,
            duration_ms: graph.duration_ms,
        };

        let resolver = self.asset_resolver.as_ref();
        draw_world_background(
            &mut canvas,
            world,
            asset_root,
            resolver,
            time,
            &mut self.image_cache,
        )?;
        draw_directional_characters(
            &mut canvas,
            world,
            graph.size,
            asset_root,
            resolver,
            time,
            &mut self.image_cache,
        )?;
        draw_actor_debug_projections(
            &mut canvas,
            graph,
            world,
            asset_root,
            resolver,
            time,
            &mut self.mesh_cache,
        )?;
        Ok(canvas)
    }

    pub async fn render_frame_gpu(
        &mut self,
        graph: &WorldGraph,
        frame: u32,
        asset_root: impl AsRef<Path>,
    ) -> Result<RgbaImage, WorldRenderError> {
        self.render_frame_gpu_internal(graph, frame, asset_root, false, false)
            .await
    }

    pub async fn render_frame_gpu_with_ground_grid(
        &mut self,
        graph: &WorldGraph,
        frame: u32,
        asset_root: impl AsRef<Path>,
    ) -> Result<RgbaImage, WorldRenderError> {
        self.render_frame_gpu_internal(graph, frame, asset_root, true, false)
            .await
    }

    pub async fn render_frame_gpu_with_ground_grid_mode(
        &mut self,
        graph: &WorldGraph,
        frame: u32,
        asset_root: impl AsRef<Path>,
        debug_grid: bool,
    ) -> Result<RgbaImage, WorldRenderError> {
        self.render_frame_gpu_internal(graph, frame, asset_root, true, debug_grid)
            .await
    }

    async fn render_frame_gpu_internal(
        &mut self,
        graph: &WorldGraph,
        frame: u32,
        asset_root: impl AsRef<Path>,
        ground_grid: bool,
        ground_grid_debug: bool,
    ) -> Result<RgbaImage, WorldRenderError> {
        let asset_root = asset_root.as_ref();
        let (width, height) = graph.output_size();
        let width = width.max(1);
        let height = height.max(1);
        let mut canvas = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));
        let world = graph
            .presented_world()
            .ok_or_else(|| WorldRenderError::MissingWorld(graph.present.from.clone()))?;
        let time = WorldTime {
            frame,
            fps: graph.fps,
            duration_ms: graph.duration_ms,
        };

        let resolver = self.asset_resolver.as_ref();
        draw_world_background(
            &mut canvas,
            world,
            asset_root,
            resolver,
            time,
            &mut self.image_cache,
        )?;
        draw_directional_characters(
            &mut canvas,
            world,
            graph.size,
            asset_root,
            resolver,
            time,
            &mut self.image_cache,
        )?;
        let draw_calls = build_actor_gpu_draws(
            &mut canvas,
            graph,
            world,
            asset_root,
            resolver,
            time,
            &mut self.mesh_cache,
            &mut self.gpu_static_draw_cache,
            &mut self.skinning_strategy_cache,
        )?;
        if draw_calls.is_empty() && !ground_grid {
            return Ok(canvas);
        }

        let needs_renderer = self
            .gpu_renderer
            .as_ref()
            .is_none_or(|renderer| renderer.width != width || renderer.height != height);
        if needs_renderer {
            self.gpu_renderer = Some(GpuWorldRenderer::new(width, height).await?);
        }
        let grid_params = if ground_grid {
            let camera_view = perspective_camera_view(world, width, height, time)?;
            Some(if ground_grid_debug {
                GpuGroundGridParams::debug_from_camera(width, height, camera_view)
            } else {
                GpuGroundGridParams::from_camera(width, height, camera_view)
            })
        } else {
            None
        };
        self.gpu_renderer
            .as_mut()
            .expect("GPU renderer initialized above")
            .render(&canvas, &draw_calls, grid_params)
            .await
    }
}

#[derive(Default)]
pub struct CharacterDesignGpuViewport {
    renderer: WorldFrameRenderer,
    diagnostics_cache: HashMap<PathBuf, WorldGpuDiagnostics>,
}

pub struct CharacterDesignViewportFrame {
    pub image: RgbaImage,
    pub diagnostics: Option<WorldGpuDiagnostics>,
}

impl CharacterDesignGpuViewport {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn render_frame(
        &mut self,
        graph: &WorldGraph,
        frame: u32,
        asset_root: impl AsRef<Path>,
        actor_id: &str,
    ) -> Result<CharacterDesignViewportFrame, WorldRenderError> {
        let asset_root = asset_root.as_ref();
        let world = graph
            .presented_world()
            .ok_or_else(|| WorldRenderError::MissingWorld(graph.present.from.clone()))?;
        let actor = world
            .actors
            .iter()
            .find(|actor| actor.id == actor_id)
            .or_else(|| world.actors.first());
        let diagnostics = if let Some(actor) = actor {
            let (model_key, mesh) = load_glb_mesh_resolved(
                asset_root,
                &actor.model,
                actor.path_style,
                self.renderer.asset_resolver.as_ref(),
            )?;
            if !self.renderer.mesh_cache.contains_key(&model_key) {
                self.renderer.mesh_cache.insert(model_key.clone(), mesh);
            }
            if !self.diagnostics_cache.contains_key(&model_key) {
                let mesh = self
                    .renderer
                    .mesh_cache
                    .get(&model_key)
                    .expect("character viewport mesh cache entry inserted before diagnostics");
                let mut diagnostics = diagnose_world_glb_gpu_plan(mesh);
                diagnostics.skipped_reasons.push(
                    "Character Design viewport uses cached GLB/GPU resources; per-frame heavy diagnostics are intentionally skipped"
                        .to_string(),
                );
                self.diagnostics_cache
                    .insert(model_key.clone(), diagnostics);
            }
            let mut diagnostics = self.diagnostics_cache.get(&model_key).cloned();
            if let Some(diagnostics) = diagnostics.as_mut() {
                let time = WorldTime {
                    frame,
                    fps: graph.fps,
                    duration_ms: graph.duration_ms,
                };
                diagnostics.bone_override_count =
                    actor_bone_overrides(graph, actor, time).map_or(0, |overrides| overrides.len());
            }
            diagnostics
        } else {
            None
        };

        let image = self
            .renderer
            .render_frame_gpu(graph, frame, asset_root)
            .await?;
        Ok(CharacterDesignViewportFrame { image, diagnostics })
    }

    pub async fn render_frame_with_ground_grid(
        &mut self,
        graph: &WorldGraph,
        frame: u32,
        asset_root: impl AsRef<Path>,
        actor_id: &str,
    ) -> Result<CharacterDesignViewportFrame, WorldRenderError> {
        self.render_frame_with_ground_grid_mode(graph, frame, asset_root, actor_id, false)
            .await
    }

    pub async fn render_frame_with_ground_grid_mode(
        &mut self,
        graph: &WorldGraph,
        frame: u32,
        asset_root: impl AsRef<Path>,
        actor_id: &str,
        debug_grid: bool,
    ) -> Result<CharacterDesignViewportFrame, WorldRenderError> {
        let asset_root = asset_root.as_ref();
        let world = graph
            .presented_world()
            .ok_or_else(|| WorldRenderError::MissingWorld(graph.present.from.clone()))?;
        let actor = world
            .actors
            .iter()
            .find(|actor| actor.id == actor_id)
            .or_else(|| world.actors.first());
        let diagnostics = if let Some(actor) = actor {
            let (model_key, mesh) = load_glb_mesh_resolved(
                asset_root,
                &actor.model,
                actor.path_style,
                self.renderer.asset_resolver.as_ref(),
            )?;
            if !self.renderer.mesh_cache.contains_key(&model_key) {
                self.renderer.mesh_cache.insert(model_key.clone(), mesh);
            }
            if !self.diagnostics_cache.contains_key(&model_key) {
                let mesh = self
                    .renderer
                    .mesh_cache
                    .get(&model_key)
                    .expect("character viewport mesh cache entry inserted before diagnostics");
                let mut diagnostics = diagnose_world_glb_gpu_plan(mesh);
                diagnostics.skipped_reasons.push(
                    "Character Design viewport uses cached GLB/GPU resources; per-frame heavy diagnostics are intentionally skipped"
                        .to_string(),
                );
                self.diagnostics_cache
                    .insert(model_key.clone(), diagnostics);
            }
            let mut diagnostics = self.diagnostics_cache.get(&model_key).cloned();
            if let Some(diagnostics) = diagnostics.as_mut() {
                let time = WorldTime {
                    frame,
                    fps: graph.fps,
                    duration_ms: graph.duration_ms,
                };
                diagnostics.bone_override_count =
                    actor_bone_overrides(graph, actor, time).map_or(0, |overrides| overrides.len());
            }
            diagnostics
        } else {
            None
        };

        let image = self
            .renderer
            .render_frame_gpu_with_ground_grid_mode(graph, frame, asset_root, debug_grid)
            .await?;
        Ok(CharacterDesignViewportFrame { image, diagnostics })
    }
}

struct GpuWorldRenderer {
    device: Arc<wgpu::Device>,
    queue: wgpu::Queue,
    _poller: DevicePoller,
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    grid_pipeline: wgpu::RenderPipeline,
    grid_bind_group: wgpu::BindGroup,
    grid_params_buffer: wgpu::Buffer,
    grid_vertex_buffer: wgpu::Buffer,
    actor_resource_cache: HashMap<GpuWorldResourceKey, GpuWorldActorResource>,
    instance_resource_cache: HashMap<GpuWorldInstanceKey, GpuWorldInstanceResource>,
    actor_sampler: wgpu::Sampler,
    target: wgpu::Texture,
    depth_texture: wgpu::Texture,
    readback_buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    padded_bytes_per_row: u32,
}

impl GpuWorldRenderer {
    async fn new(width: u32, height: u32) -> Result<Self, WorldRenderError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = request_adapter_async(
            &instance,
            &wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            },
        )
        .await
        .map_err(|_| WorldRenderError::GpuRender {
            message: "no high-performance GPU adapter was available".to_string(),
        })?;
        let adapter_limits = adapter.limits();
        let max_texture_dimension_2d = adapter_limits.max_texture_dimension_2d;
        if width > max_texture_dimension_2d || height > max_texture_dimension_2d {
            return Err(WorldRenderError::GpuRender {
                message: format!(
                    "requested world render size {}x{} exceeds GPU max 2D texture dimension {}",
                    width, height, max_texture_dimension_2d
                ),
            });
        }

        let (device, queue) = request_device_async(
            &adapter,
            &wgpu::DeviceDescriptor {
                label: Some("anica-motionloom-world-gpu-device"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter_limits,
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            },
        )
        .await
        .map_err(|err| WorldRenderError::GpuRender {
            message: format!("device request failed: {err}"),
        })?;
        let device = Arc::new(device);
        let poller = DevicePoller::start(device.clone());

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("anica-motionloom-world-gpu-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(WGPU_WORLD_SHADER)),
        });
        let grid_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("anica-motionloom-ground-grid-gpu-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(WGPU_GROUND_GRID_SHADER)),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("anica-motionloom-world-gpu-bind-group-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let grid_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("anica-motionloom-ground-grid-bind-group-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("anica-motionloom-world-gpu-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let grid_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("anica-motionloom-ground-grid-pipeline-layout"),
            bind_group_layouts: &[&grid_bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("anica-motionloom-world-gpu-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 80,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 12,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 24,
                            shader_location: 2,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 40,
                            shader_location: 3,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 56,
                            shader_location: 4,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 64,
                            shader_location: 5,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Greater,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let grid_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("anica-motionloom-ground-grid-pipeline"),
            layout: Some(&grid_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &grid_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    }],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &grid_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Greater,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let target = Self::make_target_texture(&device, width, height);
        let depth_texture = Self::make_depth_texture(&device, width, height);
        let actor_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("anica-motionloom-world-gpu-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let padded_bytes_per_row = align_to_256(width.saturating_mul(4));
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("anica-motionloom-world-gpu-readback"),
            size: (padded_bytes_per_row as u64 * height as u64).max(4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let grid_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("anica-motionloom-ground-grid-params"),
            size: 128,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let grid_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("anica-motionloom-ground-grid-bind-group"),
            layout: &grid_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: grid_params_buffer.as_entire_binding(),
            }],
        });
        let grid_vertex_buffer = {
            let half = 200.0f32;
            let vertices = [
                [-half, 0.0, -half],
                [half, 0.0, -half],
                [half, 0.0, half],
                [-half, 0.0, -half],
                [half, 0.0, half],
                [-half, 0.0, half],
            ];
            let bytes = pack_f32x3_vertices(&vertices);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("anica-motionloom-ground-grid-vertices"),
                size: bytes.len() as u64,
                usage: wgpu::BufferUsages::VERTEX,
                mapped_at_creation: true,
            });
            buffer
                .slice(..bytes.len() as u64)
                .get_mapped_range_mut()
                .copy_from_slice(&bytes);
            buffer.unmap();
            buffer
        };

        Ok(Self {
            device,
            queue,
            _poller: poller,
            bind_group_layout,
            pipeline,
            grid_pipeline,
            grid_bind_group,
            grid_params_buffer,
            grid_vertex_buffer,
            actor_resource_cache: HashMap::new(),
            instance_resource_cache: HashMap::new(),
            actor_sampler,
            target,
            depth_texture,
            readback_buffer,
            width,
            height,
            padded_bytes_per_row,
        })
    }

    async fn render(
        &mut self,
        background: &RgbaImage,
        draw_calls: &[GpuWorldDraw],
        ground_grid: Option<GpuGroundGridParams>,
    ) -> Result<RgbaImage, WorldRenderError> {
        if background.width() != self.width || background.height() != self.height {
            return Err(WorldRenderError::GpuRender {
                message: format!(
                    "world GPU background size {}x{} does not match renderer {}x{}",
                    background.width(),
                    background.height(),
                    self.width,
                    self.height
                ),
            });
        }
        self.write_texture_rgba(&self.target, background.as_raw());
        let mut gpu_draws = Vec::<GpuWorldDrawResources>::with_capacity(draw_calls.len());
        for draw in draw_calls {
            if draw.vertices.is_empty() {
                continue;
            }
            if !self.actor_resource_cache.contains_key(&draw.resource_key) {
                let vertex_bytes = pack_gpu_world_vertices(draw.vertices.as_slice());
                let vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("anica-motionloom-world-gpu-vertices"),
                    size: vertex_bytes.len().max(4) as u64,
                    usage: wgpu::BufferUsages::VERTEX,
                    mapped_at_creation: true,
                });
                vertex_buffer
                    .slice(..vertex_bytes.len() as u64)
                    .get_mapped_range_mut()
                    .copy_from_slice(&vertex_bytes);
                vertex_buffer.unmap();

                let actor_texture = self.make_actor_texture(
                    draw.texture.as_ref(),
                    "anica-motionloom-world-gpu-texture",
                );
                let actor_texture_view =
                    actor_texture.create_view(&wgpu::TextureViewDescriptor::default());
                self.actor_resource_cache.insert(
                    draw.resource_key.clone(),
                    GpuWorldActorResource {
                        vertex_buffer,
                        _texture: actor_texture,
                        texture_view: actor_texture_view,
                        vertex_count: draw.vertices.len() as u32,
                    },
                );
            }

            let (vertex_buffer, texture_view, vertex_count) = {
                let resource = self
                    .actor_resource_cache
                    .get(&draw.resource_key)
                    .expect("GPU actor resource inserted before draw");
                (
                    resource.vertex_buffer.clone(),
                    resource.texture_view.clone(),
                    resource.vertex_count,
                )
            };
            let params_bytes = pack_gpu_world_params(draw.params);
            let bone_bytes = pack_gpu_world_bones(&draw.bone_matrices);
            let bone_buffer_size = bone_bytes.len().max(64) as u64;
            let needs_instance = self
                .instance_resource_cache
                .get(&draw.instance_key)
                .is_none_or(|resource| resource.bone_buffer_size < bone_buffer_size);
            if needs_instance {
                let params_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("anica-motionloom-world-gpu-params"),
                    size: params_bytes.len().max(4) as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let bone_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("anica-motionloom-world-gpu-bones"),
                    size: bone_buffer_size,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("anica-motionloom-world-gpu-bind-group"),
                    layout: &self.bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: params_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: bone_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&texture_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Sampler(&self.actor_sampler),
                        },
                    ],
                });
                self.instance_resource_cache.insert(
                    draw.instance_key.clone(),
                    GpuWorldInstanceResource {
                        params_buffer,
                        bone_buffer,
                        bone_buffer_size,
                        bind_group,
                    },
                );
            }
            let instance_resource = self
                .instance_resource_cache
                .get(&draw.instance_key)
                .expect("GPU world instance resource inserted before draw");
            self.queue
                .write_buffer(&instance_resource.params_buffer, 0, &params_bytes);
            self.queue
                .write_buffer(&instance_resource.bone_buffer, 0, &bone_bytes);
            gpu_draws.push(GpuWorldDrawResources {
                vertex_buffer,
                bind_group: instance_resource.bind_group.clone(),
                vertex_count,
            });
        }

        let view = self
            .target
            .create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = self
            .depth_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("anica-motionloom-world-gpu-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("anica-motionloom-world-gpu-render-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if let Some(grid) = ground_grid {
                self.queue.write_buffer(
                    &self.grid_params_buffer,
                    0,
                    &pack_ground_grid_params(grid),
                );
                pass.set_pipeline(&self.grid_pipeline);
                pass.set_bind_group(0, &self.grid_bind_group, &[]);
                pass.set_vertex_buffer(0, self.grid_vertex_buffer.slice(..));
                pass.draw(0..6, 0..1);
            }
            pass.set_pipeline(&self.pipeline);
            for draw in &gpu_draws {
                pass.set_bind_group(0, &draw.bind_group, &[]);
                pass.set_vertex_buffer(0, draw.vertex_buffer.slice(..));
                pass.draw(0..draw.vertex_count, 0..1);
            }
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        self.readback_rgba_async().await
    }

    fn make_target_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("anica-motionloom-world-gpu-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    fn make_depth_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("anica-motionloom-world-gpu-depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
    }

    fn write_texture_rgba(&self, texture: &wgpu::Texture, rgba: &[u8]) {
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.width.saturating_mul(4)),
                rows_per_image: Some(self.height),
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
    }

    fn make_actor_texture(&self, texture: &GpuWorldTexture, label: &'static str) -> wgpu::Texture {
        let width = texture.width.max(1);
        let height = texture.height.max(1);
        let expected_len = width as usize * height as usize * 4;
        let fallback;
        let rgba = if texture.rgba.len() == expected_len {
            texture.rgba.as_slice()
        } else {
            fallback = vec![255, 255, 255, 255];
            fallback.as_slice()
        };
        let gpu_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &gpu_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width.saturating_mul(4)),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        gpu_texture
    }

    async fn readback_rgba_async(&self) -> Result<RgbaImage, WorldRenderError> {
        let slice = self.readback_buffer.slice(..);
        BufferMapAsyncFuture::new(&self._poller, &self.readback_buffer)
            .await
            .map_err(|err| WorldRenderError::GpuRender {
                message: format!("readback map failed: {err}"),
            })?;

        let mapped = slice.get_mapped_range();
        let row_bytes = self.width as usize * 4;
        let padded_row = self.padded_bytes_per_row as usize;
        let mut out = vec![0u8; row_bytes * self.height as usize];
        for row in 0..self.height as usize {
            let src_off = row * padded_row;
            let dst_off = row * row_bytes;
            out[dst_off..dst_off + row_bytes]
                .copy_from_slice(&mapped[src_off..src_off + row_bytes]);
        }
        drop(mapped);
        self.readback_buffer.unmap();
        RgbaImage::from_raw(self.width, self.height, out).ok_or_else(|| {
            WorldRenderError::GpuRender {
                message: "failed to build RGBA image from world GPU readback".to_string(),
            }
        })
    }
}

struct GpuWorldDrawResources {
    vertex_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_count: u32,
}

struct GpuWorldActorResource {
    vertex_buffer: wgpu::Buffer,
    _texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    vertex_count: u32,
}

struct GpuWorldInstanceResource {
    params_buffer: wgpu::Buffer,
    bone_buffer: wgpu::Buffer,
    bone_buffer_size: u64,
    bind_group: wgpu::BindGroup,
}

fn pack_gpu_world_vertices(vertices: &[GpuWorldVertex]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vertices.len().saturating_mul(80));
    for vertex in vertices {
        for value in vertex.position {
            out.extend_from_slice(&value.to_ne_bytes());
        }
        for value in vertex.normal {
            out.extend_from_slice(&value.to_ne_bytes());
        }
        for value in vertex.joints {
            out.extend_from_slice(&value.to_ne_bytes());
        }
        for value in vertex.weights {
            out.extend_from_slice(&value.to_ne_bytes());
        }
        for value in vertex.uv {
            out.extend_from_slice(&value.to_ne_bytes());
        }
        for value in vertex.color {
            out.extend_from_slice(&value.to_ne_bytes());
        }
    }
    out
}

fn pack_gpu_world_params(params: GpuWorldParams) -> Vec<u8> {
    let mut out = Vec::with_capacity(144);
    for vector in [
        params.canvas,
        params.model,
        params.actor,
        params.actor_rotation,
        params.camera0,
        params.camera1,
        params.camera2,
        params.camera3,
        params.style,
    ] {
        for value in vector {
            out.extend_from_slice(&value.to_ne_bytes());
        }
    }
    out
}

fn pack_ground_grid_params(params: GpuGroundGridParams) -> Vec<u8> {
    let mut out = Vec::with_capacity(128);
    for vector in [
        params.canvas,
        params.camera0,
        params.camera1,
        params.camera2,
        params.camera3,
        params.options,
        [0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0],
    ] {
        for value in vector {
            out.extend_from_slice(&value.to_ne_bytes());
        }
    }
    out
}

fn pack_f32x3_vertices(vertices: &[[f32; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vertices.len().saturating_mul(12));
    for vertex in vertices {
        for value in *vertex {
            out.extend_from_slice(&value.to_ne_bytes());
        }
    }
    out
}

fn pack_gpu_world_bones(bones: &[[f32; 16]]) -> Vec<u8> {
    let matrices = if bones.is_empty() {
        vec![mat4_identity()]
    } else {
        bones.to_vec()
    };
    let mut out = Vec::with_capacity(matrices.len().saturating_mul(64));
    for matrix in matrices {
        for value in matrix {
            out.extend_from_slice(&value.to_ne_bytes());
        }
    }
    out
}

fn align_to_256(v: u32) -> u32 {
    const ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    v.div_ceil(ALIGN) * ALIGN
}

const WGPU_WORLD_SHADER: &str = r#"
struct Params {
    canvas: vec4<f32>,
    model: vec4<f32>,
    actor: vec4<f32>,
    actor_rotation: vec4<f32>,
    camera0: vec4<f32>,
    camera1: vec4<f32>,
    camera2: vec4<f32>,
    camera3: vec4<f32>,
    style: vec4<f32>,
};

struct BoneMatrices {
    matrices: array<mat4x4<f32>>,
};

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) joints: vec4<f32>,
    @location(3) weights: vec4<f32>,
    @location(4) uv: vec2<f32>,
    @location(5) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) light: f32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> bones: BoneMatrices;
@group(0) @binding(2) var actor_texture: texture_2d<f32>;
@group(0) @binding(3) var actor_sampler: sampler;

fn bone_transform(joint: f32, position: vec3<f32>) -> vec3<f32> {
    let joint_index = u32(max(joint + 0.5, 0.0));
    return (bones.matrices[joint_index] * vec4<f32>(position, 1.0)).xyz;
}

fn bone_transform_vector(joint: f32, vector: vec3<f32>) -> vec3<f32> {
    let joint_index = u32(max(joint + 0.5, 0.0));
    return (bones.matrices[joint_index] * vec4<f32>(vector, 0.0)).xyz;
}

fn actor_rotate(vector: vec3<f32>) -> vec3<f32> {
    let yaw_cos = cos(params.actor.w);
    let yaw_sin = sin(params.actor.w);
    let pitch_cos = cos(params.actor_rotation.x);
    let pitch_sin = sin(params.actor_rotation.x);
    let roll_cos = cos(params.actor_rotation.y);
    let roll_sin = sin(params.actor_rotation.y);

    let yawed = vec3<f32>(
        vector.x * yaw_cos + vector.z * yaw_sin,
        vector.y,
        -vector.x * yaw_sin + vector.z * yaw_cos,
    );
    let pitched = vec3<f32>(
        yawed.x,
        yawed.y * pitch_cos - yawed.z * pitch_sin,
        yawed.y * pitch_sin + yawed.z * pitch_cos,
    );
    return vec3<f32>(
        pitched.x * roll_cos - pitched.y * roll_sin,
        pitched.x * roll_sin + pitched.y * roll_cos,
        pitched.z,
    );
}

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
    let weight_sum = input.weights.x + input.weights.y + input.weights.z + input.weights.w;
    var skinned = input.position;
    var skinned_normal = input.normal;
    if (weight_sum > 0.000001) {
        skinned =
            bone_transform(input.joints.x, input.position) * (input.weights.x / weight_sum) +
            bone_transform(input.joints.y, input.position) * (input.weights.y / weight_sum) +
            bone_transform(input.joints.z, input.position) * (input.weights.z / weight_sum) +
            bone_transform(input.joints.w, input.position) * (input.weights.w / weight_sum);
        skinned_normal =
            bone_transform_vector(input.joints.x, input.normal) * (input.weights.x / weight_sum) +
            bone_transform_vector(input.joints.y, input.normal) * (input.weights.y / weight_sum) +
            bone_transform_vector(input.joints.z, input.normal) * (input.weights.z / weight_sum) +
            bone_transform_vector(input.joints.w, input.normal) * (input.weights.w / weight_sum);
    }

    let local = vec3<f32>(
        skinned.x - params.model.x,
        skinned.y - params.model.y,
        skinned.z - params.model.z,
    ) * params.model.w;
    let rotated = actor_rotate(local);
    let world = params.actor.xyz + rotated;

    let normal_world = normalize(actor_rotate(skinned_normal));
    let right = params.camera1.xyz;
    let up = params.camera2.xyz;
    let forward = params.camera3.xyz;
    let rel = world - params.camera0.xyz;
    let view_x = dot(rel, right);
    let view_y = dot(rel, up);
    let view_z = dot(rel, forward);
    let safe_z = max(view_z, params.camera1.w);
    let screen_x = params.canvas.z + (view_x * params.camera0.w) / safe_z;
    let screen_y = params.canvas.w - (view_y * params.camera0.w) / safe_z;
    let ndc_x = (screen_x / params.canvas.x) * 2.0 - 1.0;
    let ndc_y = 1.0 - (screen_y / params.canvas.y) * 2.0;
    let far = max(params.camera2.w, params.camera1.w + 0.001);
    let ndc_z = clamp(1.0 - ((safe_z - params.camera1.w) / (far - params.camera1.w)), 0.0, 1.0);

    let view_normal = normalize(vec3<f32>(
        dot(normal_world, right),
        dot(normal_world, up),
        dot(normal_world, forward),
    ));
    let light_dir = normalize(vec3<f32>(-0.35, 0.75, 0.55));
    let diffuse = max(dot(view_normal, light_dir), 0.0);
    let light = clamp(0.54 + diffuse * 0.48, 0.42, 1.08);

    var out: VertexOut;
    out.pos = vec4<f32>(ndc_x, ndc_y, ndc_z, 1.0);
    out.color = input.color;
    out.uv = input.uv;
    out.light = mix(1.0, light, params.style.y);
    return out;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    let uv = clamp(input.uv, vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0));
    let sampled = textureSample(actor_texture, actor_sampler, uv);
    let alpha = sampled.a * input.color.a * params.style.x;
    if (alpha <= 0.001) {
        discard;
    }
    return vec4<f32>(sampled.rgb * input.color.rgb * input.light, alpha);
}
"#;

const WGPU_GROUND_GRID_SHADER: &str = r#"
struct GridParams {
    canvas: vec4<f32>,
    camera0: vec4<f32>,
    camera1: vec4<f32>,
    camera2: vec4<f32>,
    camera3: vec4<f32>,
    options: vec4<f32>,
    _pad0: vec4<f32>,
    _pad1: vec4<f32>,
};

struct VertexIn {
    @location(0) offset: vec3<f32>,
};

struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
};

@group(0) @binding(0) var<uniform> params: GridParams;

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
    let center = vec3<f32>(params.camera0.x, 0.0, params.camera0.z);
    let world = center + input.offset;
    let right = params.camera1.xyz;
    let up = params.camera2.xyz;
    let forward = params.camera3.xyz;
    let rel = world - params.camera0.xyz;
    let view_x = dot(rel, right);
    let view_y = dot(rel, up);
    let view_z = dot(rel, forward);
    let near = params.camera1.w;
    let far = max(params.camera2.w, near + 0.001);

    var out: VertexOut;
    out.world_pos = world;
    if (view_z <= near) {
        out.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }

    let safe_z = max(view_z, near + 0.0001);
    let screen_x = params.canvas.z + (view_x * params.camera0.w) / safe_z;
    let screen_y = params.canvas.w - (view_y * params.camera0.w) / safe_z;
    let ndc_x = (screen_x / params.canvas.x) * 2.0 - 1.0;
    let ndc_y = 1.0 - (screen_y / params.canvas.y) * 2.0;
    let ndc_z = clamp(1.0 - ((safe_z - near) / (far - near)), 0.0, 1.0);
    out.pos = vec4<f32>(ndc_x, ndc_y, ndc_z, 1.0);
    return out;
}

fn grid_alpha(coord: vec2<f32>, scale: f32) -> f32 {
    let scaled = coord / scale;
    let derivative = max(fwidth(scaled), vec2<f32>(0.000001, 0.000001));
    let grid = abs(fract(scaled - 0.5) - 0.5) / derivative;
    let line_val = min(grid.x, grid.y);
    return 1.0 - min(line_val, 1.0);
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    let grid_size = max(params.options.w, 0.0001);
    let coord = input.world_pos.xz / grid_size;
    let debug_mode = params.options.y < 0.0;

    var fine_weight: f32 = 0.45;
    var coarse_weight: f32 = 1.00;
    var axis_width: f32 = grid_size * 0.04;
    var opacity: f32 = params.options.x;
    var fade: f32 = 1.0;
    if (!debug_mode) {
        // Must be per-fragment distance (not interpolated vertex distance),
        // otherwise the whole grid fades out when plane vertices are far away.
        let dist = distance(input.world_pos, params.camera0.xyz);
        fade = 1.0 - smoothstep(params.options.y, params.options.z, dist);
    } else {
        // Debug grid mode: strong, thick, high-contrast lines with no fade.
        fine_weight = 1.10;
        coarse_weight = 1.25;
        axis_width = grid_size * 0.10;
        opacity = 1.0;
        fade = 1.0;
    }

    let fine = grid_alpha(coord, 1.0) * fine_weight;
    let coarse = grid_alpha(coord, 10.0) * coarse_weight;
    let axis_x = 1.0 - smoothstep(0.0, axis_width, abs(input.world_pos.z));
    let axis_z = 1.0 - smoothstep(0.0, axis_width, abs(input.world_pos.x));
    let line_alpha = max(max(fine, coarse), max(axis_x, axis_z));

    let alpha = min(line_alpha, 1.0) * fade * opacity;
    if (alpha <= 0.001) {
        discard;
    }

    var base_color = mix(vec3<f32>(0.50, 0.54, 0.60), vec3<f32>(0.86, 0.89, 0.94), coarse);
    if (debug_mode) {
        base_color = mix(vec3<f32>(0.10, 0.12, 0.18), vec3<f32>(0.98, 0.98, 1.00), coarse);
    }
    let x_axis_color = vec3<f32>(0.95, 0.28, 0.28);
    let z_axis_color = vec3<f32>(0.30, 0.86, 0.42);
    var color = base_color;
    color = mix(color, x_axis_color, axis_x);
    color = mix(color, z_axis_color, axis_z);
    return vec4<f32>(color, alpha);
}
"#;

#[cfg_attr(target_arch = "wasm32", allow(unused_mut, unused_variables))]
pub async fn render_world_graph_to_video_with_progress<F>(
    ffmpeg_bin: &str,
    graph: &WorldGraph,
    asset_root: impl AsRef<Path>,
    output_path: &Path,
    profile: SceneRenderProfile,
    progress_every_frames: u32,
    progress_callback: F,
) -> Result<(), WorldRenderError>
where
    F: FnMut(WorldRenderProgress),
{
    render_world_graph_to_video_with_progress_and_cancel(
        ffmpeg_bin,
        graph,
        asset_root,
        output_path,
        profile,
        progress_every_frames,
        None,
        progress_callback,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
#[cfg_attr(target_arch = "wasm32", allow(unused_mut, unused_variables))]
pub async fn render_world_graph_to_video_with_progress_and_cancel<F>(
    ffmpeg_bin: &str,
    graph: &WorldGraph,
    asset_root: impl AsRef<Path>,
    output_path: &Path,
    profile: SceneRenderProfile,
    progress_every_frames: u32,
    cancel: Option<Arc<AtomicBool>>,
    mut progress_callback: F,
) -> Result<(), WorldRenderError>
where
    F: FnMut(WorldRenderProgress),
{
    if profile.is_png_sequence() {
        return render_world_graph_to_png_sequence_internal(
            graph,
            asset_root,
            output_path,
            profile,
            progress_every_frames,
            cancel,
            progress_callback,
        )
        .await;
    }

    #[cfg(target_arch = "wasm32")]
    {
        Err(WorldRenderError::VideoExportNotAvailable {
            message: "FFmpeg video export is not available in WASM".to_string(),
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        use crate::export::{FfmpegVideoEncoder, VideoEncoder};

        let asset_root = asset_root.as_ref().to_path_buf();
        let (w, h) = graph.output_size();
        let fps = graph.fps.max(1.0);
        let duration_sec = (graph.duration_ms as f32 / 1000.0).max(1.0 / fps);
        let total_frames = ((duration_sec * fps).round() as u32).max(1);
        let encoder_args = world_encoder_args(profile);
        let mut renderer = WorldFrameRenderer::default();
        progress_callback(WorldRenderProgress {
            rendered_frames: 0,
            total_frames,
        });
        if cancel
            .as_ref()
            .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
        {
            return Err(WorldRenderError::Cancelled);
        }

        let mut encoder =
            FfmpegVideoEncoder::new(ffmpeg_bin, output_path).with_encoder_args(encoder_args);
        encoder.begin(w, h, fps)?;

        for frame in 0..total_frames {
            if cancel
                .as_ref()
                .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
            {
                encoder.abort();
                return Err(WorldRenderError::Cancelled);
            }
            let image = if profile.uses_gpu_compositor() {
                renderer.render_frame_gpu(graph, frame, &asset_root).await?
            } else {
                renderer.render_frame(graph, frame, &asset_root)?
            };
            encoder.push_frame(frame, image.as_raw())?;
            let rendered_frames = frame + 1;
            if rendered_frames == total_frames
                || (progress_every_frames > 0 && rendered_frames % progress_every_frames == 0)
            {
                progress_callback(WorldRenderProgress {
                    rendered_frames,
                    total_frames,
                });
            }
        }
        encoder.finish()?;
        Ok(())
    }
}

pub async fn render_world_graph_to_png_sequence_with_progress<F>(
    graph: &WorldGraph,
    asset_root: impl AsRef<Path>,
    output_dir: &Path,
    progress_every_frames: u32,
    progress_callback: F,
) -> Result<(), WorldRenderError>
where
    F: FnMut(WorldRenderProgress),
{
    render_world_graph_to_png_sequence_with_progress_and_cancel(
        graph,
        asset_root,
        output_dir,
        progress_every_frames,
        None,
        progress_callback,
    )
    .await
}

pub async fn render_world_graph_to_png_sequence_with_progress_and_cancel<F>(
    graph: &WorldGraph,
    asset_root: impl AsRef<Path>,
    output_dir: &Path,
    progress_every_frames: u32,
    cancel: Option<Arc<AtomicBool>>,
    progress_callback: F,
) -> Result<(), WorldRenderError>
where
    F: FnMut(WorldRenderProgress),
{
    render_world_graph_to_png_sequence_internal(
        graph,
        asset_root,
        output_dir,
        SceneRenderProfile::GpuPngSequence,
        progress_every_frames,
        cancel,
        progress_callback,
    )
    .await
}

async fn render_world_graph_to_png_sequence_internal<F>(
    graph: &WorldGraph,
    asset_root: impl AsRef<Path>,
    output_dir: &Path,
    profile: SceneRenderProfile,
    progress_every_frames: u32,
    cancel: Option<Arc<AtomicBool>>,
    mut progress_callback: F,
) -> Result<(), WorldRenderError>
where
    F: FnMut(WorldRenderProgress),
{
    fs::create_dir_all(output_dir).map_err(|source| WorldRenderError::CreateOutputDir {
        path: output_dir.to_path_buf(),
        source,
    })?;

    let asset_root = asset_root.as_ref().to_path_buf();
    let fps = graph.fps.max(1.0);
    let duration_sec = (graph.duration_ms as f32 / 1000.0).max(1.0 / fps);
    let total_frames = ((duration_sec * fps).round() as u32).max(1);
    let mut renderer = WorldFrameRenderer::default();
    progress_callback(WorldRenderProgress {
        rendered_frames: 0,
        total_frames,
    });

    for frame in 0..total_frames {
        if cancel
            .as_ref()
            .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
        {
            return Err(WorldRenderError::Cancelled);
        }
        let image = if profile.uses_gpu_compositor() {
            renderer.render_frame_gpu(graph, frame, &asset_root).await?
        } else {
            renderer.render_frame(graph, frame, &asset_root)?
        };
        let path = output_dir.join(format!("frame_{frame:06}.png"));
        image
            .save(&path)
            .map_err(|source| WorldRenderError::SavePngFrame { path, source })?;

        let rendered_frames = frame + 1;
        if rendered_frames == total_frames
            || (progress_every_frames > 0 && rendered_frames % progress_every_frames == 0)
        {
            progress_callback(WorldRenderProgress {
                rendered_frames,
                total_frames,
            });
        }
    }

    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn world_encoder_args(profile: SceneRenderProfile) -> Vec<String> {
    match profile {
        SceneRenderProfile::Cpu | SceneRenderProfile::GpuProRes => world_prores_encoder_args(),
        SceneRenderProfile::Gpu => world_gpu_h264_encoder_args(),
        SceneRenderProfile::GpuProRes4444 => world_prores_4444_encoder_args(),
        SceneRenderProfile::GpuPngSequence => Vec::new(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn world_prores_encoder_args() -> Vec<String> {
    vec![
        "-vf".to_string(),
        "format=yuv422p10le".to_string(),
        "-c:v".to_string(),
        "prores_ks".to_string(),
        "-profile:v".to_string(),
        "3".to_string(),
        "-vendor".to_string(),
        "apl0".to_string(),
        "-pix_fmt".to_string(),
        "yuv422p10le".to_string(),
        "-color_primaries".to_string(),
        "bt709".to_string(),
        "-color_trc".to_string(),
        "bt709".to_string(),
        "-colorspace".to_string(),
        "bt709".to_string(),
    ]
}

#[cfg(not(target_arch = "wasm32"))]
fn world_prores_4444_encoder_args() -> Vec<String> {
    vec![
        "-vf".to_string(),
        "format=yuva444p10le".to_string(),
        "-c:v".to_string(),
        "prores_ks".to_string(),
        "-profile:v".to_string(),
        "4".to_string(),
        "-vendor".to_string(),
        "apl0".to_string(),
        "-alpha_bits".to_string(),
        "16".to_string(),
        "-vtag".to_string(),
        "ap4h".to_string(),
        "-pix_fmt".to_string(),
        "yuva444p10le".to_string(),
        "-color_primaries".to_string(),
        "bt709".to_string(),
        "-color_trc".to_string(),
        "bt709".to_string(),
        "-colorspace".to_string(),
        "bt709".to_string(),
    ]
}

#[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
fn world_gpu_h264_encoder_args() -> Vec<String> {
    vec![
        "-c:v".to_string(),
        "h264_videotoolbox".to_string(),
        "-allow_sw".to_string(),
        "1".to_string(),
        "-profile:v".to_string(),
        "high".to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-b:v".to_string(),
        "30M".to_string(),
        "-maxrate".to_string(),
        "45M".to_string(),
        "-bufsize".to_string(),
        "90M".to_string(),
        "-color_primaries".to_string(),
        "bt709".to_string(),
        "-color_trc".to_string(),
        "bt709".to_string(),
        "-colorspace".to_string(),
        "bt709".to_string(),
        "-movflags".to_string(),
        "+faststart".to_string(),
    ]
}

#[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
fn world_gpu_h264_encoder_args() -> Vec<String> {
    vec![
        "-c:v".to_string(),
        "h264_mf".to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-b:v".to_string(),
        "30M".to_string(),
        "-maxrate".to_string(),
        "45M".to_string(),
        "-bufsize".to_string(),
        "90M".to_string(),
        "-color_primaries".to_string(),
        "bt709".to_string(),
        "-color_trc".to_string(),
        "bt709".to_string(),
        "-colorspace".to_string(),
        "bt709".to_string(),
        "-movflags".to_string(),
        "+faststart".to_string(),
    ]
}

#[cfg(all(
    not(target_arch = "wasm32"),
    not(target_os = "macos"),
    not(target_os = "windows")
))]
fn world_gpu_h264_encoder_args() -> Vec<String> {
    vec![
        "-c:v".to_string(),
        "libopenh264".to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-b:v".to_string(),
        "30M".to_string(),
        "-maxrate".to_string(),
        "45M".to_string(),
        "-bufsize".to_string(),
        "90M".to_string(),
        "-color_primaries".to_string(),
        "bt709".to_string(),
        "-color_trc".to_string(),
        "bt709".to_string(),
        "-colorspace".to_string(),
        "bt709".to_string(),
        "-movflags".to_string(),
        "+faststart".to_string(),
    ]
}

fn draw_world_background(
    canvas: &mut RgbaImage,
    world: &WorldNode,
    asset_root: &Path,
    resolver: &dyn AssetResolver,
    time: WorldTime,
    image_cache: &mut HashMap<PathBuf, RgbaImage>,
) -> Result<(), WorldRenderError> {
    let background = world.background.as_ref();
    let color = background
        .map(|bg| parse_rgba(&bg.color))
        .unwrap_or(Rgba([0, 0, 0, 255]));
    fill(canvas, color);

    let Some(background) = background else {
        return Ok(());
    };
    let Some(src) = background.src.as_deref() else {
        return Ok(());
    };
    let resolved = resolve_world_asset_source(asset_root, src, WorldPathStyle::Relative, resolver)?;
    let key = resolved.key().to_path_buf();
    if matches!(resolved, ResolvedWorldAsset::Missing { .. }) {
        return Ok(());
    }
    let opacity = eval_number(&background.opacity, 1.0, time)?.clamp(0.0, 1.0);
    if !image_cache.contains_key(&key) {
        let image = load_rgba_image_from_resolved(&resolved, |path, source| {
            WorldRenderError::BackgroundImage { path, source }
        })?
        .to_rgba8();
        image_cache.insert(key.clone(), image);
    }
    if let Some(image) = image_cache.get(&key) {
        composite_background(canvas, image, &background.fit, opacity);
    }
    Ok(())
}

fn draw_directional_characters(
    canvas: &mut RgbaImage,
    world: &WorldNode,
    logical_size: (u32, u32),
    asset_root: &Path,
    resolver: &dyn AssetResolver,
    time: WorldTime,
    image_cache: &mut HashMap<PathBuf, RgbaImage>,
) -> Result<(), WorldRenderError> {
    if world.directional_characters.is_empty() {
        return Ok(());
    }
    let camera_yaw = eval_number(&world.camera.yaw, 0.0, time)?;
    let camera_pitch = eval_number(&world.camera.pitch, 0.0, time)?;
    let camera_x = eval_number(&world.camera.x, 0.0, time)?;
    let camera_y = eval_number(&world.camera.y, 0.0, time)?;
    let camera_zoom = eval_number(&world.camera.zoom, 1.0, time)?.max(0.01);
    let logical_w = logical_size.0.max(1) as f32;
    let logical_h = logical_size.1.max(1) as f32;
    let output_scale_x = canvas.width().max(1) as f32 / logical_w;
    let output_scale_y = canvas.height().max(1) as f32 / logical_h;

    for character in &world.directional_characters {
        let Some(direction) = select_direction_frame(character, camera_yaw, camera_pitch, time)?
        else {
            continue;
        };
        let Some(image_src) = direction.image.as_deref().or(character.sheet.as_deref()) else {
            continue;
        };
        let resolved =
            resolve_world_asset_source(asset_root, image_src, character.path_style, resolver)?;
        let key = resolved.key().to_path_buf();
        if matches!(resolved, ResolvedWorldAsset::Missing { .. }) {
            return Err(WorldRenderError::MissingDirectionalCharacterImage(key));
        }
        if !image_cache.contains_key(&key) {
            let image = load_rgba_image_from_resolved(&resolved, |path, source| {
                WorldRenderError::DirectionalCharacterImage { path, source }
            })?
            .to_rgba8();
            image_cache.insert(key.clone(), image);
        }
        let Some(source_image) = image_cache.get(&key) else {
            continue;
        };
        let (rect_x, rect_y, rect_w, rect_h) = if let Some(play_sprite) =
            character.play_sprite.as_ref()
        {
            let Some(rect) = play_sprite_rect(play_sprite, direction, source_image, time)? else {
                continue;
            };
            rect
        } else if let Some(rect) = direction.rect {
            let Some(clamped) = clamp_direction_rect(rect, source_image) else {
                continue;
            };
            clamped
        } else {
            (
                0,
                0,
                source_image.width().max(1),
                source_image.height().max(1),
            )
        };
        if rect_w == 0 || rect_h == 0 {
            continue;
        };
        let frame = imageops::crop_imm(source_image, rect_x, rect_y, rect_w, rect_h).to_image();
        let scale = eval_number(&character.scale, 1.0, time)?.max(0.01) * camera_zoom;
        let scale_x = (scale * output_scale_x).max(0.01);
        let scale_y = (scale * output_scale_y).max(0.01);
        let scaled_w = ((frame.width() as f32 * scale_x).round() as u32).max(1);
        let scaled_h = ((frame.height() as f32 * scale_y).round() as u32).max(1);
        let scaled = imageops::resize(&frame, scaled_w, scaled_h, imageops::FilterType::Lanczos3);
        let x = (eval_number(&character.x, 0.0, time)? - camera_x) * output_scale_x;
        let y = (eval_number(&character.y, 0.0, time)? - camera_y) * output_scale_y;
        let opacity = eval_number(&character.opacity, 1.0, time)?.clamp(0.0, 1.0);
        let anchor = direction
            .anchor
            .unwrap_or((rect_w as f32 * 0.5, rect_h as f32));
        let draw_x = (x - anchor.0 * scale_x).round() as i32;
        let draw_y = (y - anchor.1 * scale_y).round() as i32;
        blend_image_i32(canvas, &scaled, draw_x, draw_y, opacity);
    }

    Ok(())
}

fn play_sprite_rect(
    play_sprite: &WorldSpritePlayback,
    direction: &WorldDirectionFrame,
    source_image: &RgbaImage,
    time: WorldTime,
) -> Result<Option<(u32, u32, u32, u32)>, WorldRenderError> {
    let fps = eval_number(&play_sprite.fps, 12.0, time)?.max(0.01);
    let elapsed = (time.time_sec() * fps).floor().max(0.0) as u32;
    let local_frame = if play_sprite.r#loop {
        elapsed % play_sprite.frames.max(1)
    } else {
        elapsed.min(play_sprite.frames.saturating_sub(1))
    };
    let frame_index = play_sprite.start.saturating_add(local_frame);
    let column = frame_index % play_sprite.columns.max(1);
    let row = frame_index / play_sprite.columns.max(1);
    let (base_x, base_y) = direction
        .rect
        .map(|rect| (rect.0, rect.1))
        .unwrap_or((play_sprite.margin_x, play_sprite.margin_y));
    let x = base_x.saturating_add(
        column.saturating_mul(
            play_sprite
                .frame_width
                .saturating_add(play_sprite.spacing_x),
        ),
    );
    let y = base_y.saturating_add(
        row.saturating_mul(
            play_sprite
                .frame_height
                .saturating_add(play_sprite.spacing_y),
        ),
    );
    Ok(clamp_direction_rect(
        (x, y, play_sprite.frame_width, play_sprite.frame_height),
        source_image,
    ))
}

fn select_direction_frame(
    character: &WorldDirectionalCharacter,
    camera_yaw: f32,
    camera_pitch: f32,
    time: WorldTime,
) -> Result<Option<&WorldDirectionFrame>, WorldRenderError> {
    if camera_pitch.abs() >= 60.0 {
        if let Some(direction) = character
            .directions
            .iter()
            .filter(|direction| direction.camera_pitch.is_some())
            .min_by(|a, b| {
                let a_dist = (a.camera_pitch.unwrap_or(0.0) - camera_pitch).abs();
                let b_dist = (b.camera_pitch.unwrap_or(0.0) - camera_pitch).abs();
                a_dist.total_cmp(&b_dist)
            })
        {
            return Ok(Some(direction));
        }
        if camera_pitch > 0.0 {
            if let Some(direction) = character.directions.iter().find(|direction| {
                direction
                    .name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case("top"))
            }) {
                return Ok(Some(direction));
            }
        }
    }

    let yaw = eval_number(&character.yaw, 0.0, time)?;
    let view_yaw = normalize_degrees(yaw - camera_yaw);
    Ok(character
        .directions
        .iter()
        .filter(|direction| direction.angle.is_some())
        .min_by(|a, b| {
            let a_dist = angular_distance(view_yaw, a.angle.unwrap_or(0.0));
            let b_dist = angular_distance(view_yaw, b.angle.unwrap_or(0.0));
            a_dist.total_cmp(&b_dist)
        })
        .or_else(|| character.directions.first()))
}

fn clamp_direction_rect(
    rect: (u32, u32, u32, u32),
    image: &RgbaImage,
) -> Option<(u32, u32, u32, u32)> {
    let (x, y, w, h) = rect;
    let x = x.min(image.width());
    let y = y.min(image.height());
    let w = w.min(image.width().saturating_sub(x));
    let h = h.min(image.height().saturating_sub(y));
    if w == 0 || h == 0 {
        None
    } else {
        Some((x, y, w, h))
    }
}

fn normalize_degrees(value: f32) -> f32 {
    value.rem_euclid(360.0)
}

fn angular_distance(a: f32, b: f32) -> f32 {
    let diff = (normalize_degrees(a) - normalize_degrees(b)).abs();
    diff.min(360.0 - diff)
}

#[derive(Debug, Clone, Copy)]
struct CameraActorView {
    x: f32,
    y: f32,
    depth: f32,
    yaw: f32,
    pitch: f32,
}

const WORLD_DEPTH_SORT_SCALE: f32 = 0.25;

#[allow(clippy::too_many_arguments)]
fn camera_actor_view(
    actor_x: f32,
    actor_y: f32,
    actor_z: f32,
    actor_yaw: f32,
    camera_x: f32,
    camera_y: f32,
    camera_z: f32,
    camera_yaw: f32,
    camera_pitch: f32,
) -> CameraActorView {
    let dx = actor_x - camera_x;
    let dy = actor_y - camera_y;
    let dz = actor_z - camera_z;
    let yaw = camera_yaw.to_radians();
    let cos_y = yaw.cos();
    let sin_y = yaw.sin();
    let view_x = dx * cos_y + dz * sin_y;
    let yaw_depth = -dx * sin_y + dz * cos_y;
    let pitch = camera_pitch.to_radians();
    let cos_p = pitch.cos();
    let sin_p = pitch.sin();
    let view_y = dy * cos_p - yaw_depth * sin_p;
    let depth = dy * sin_p + yaw_depth * cos_p;
    CameraActorView {
        x: view_x,
        y: view_y,
        depth,
        yaw: actor_yaw - camera_yaw,
        pitch: camera_pitch,
    }
}

fn perspective_camera_view(
    world: &WorldNode,
    width: u32,
    height: u32,
    time: WorldTime,
) -> Result<PerspectiveCameraView, WorldRenderError> {
    let width_f = width.max(1) as f32;
    let height_f = height.max(1) as f32;
    let target_x = eval_number(&world.camera.target_x, 0.0, time)?;
    let target_y = eval_number(&world.camera.target_y, 1.0, time)?;
    let target_z = eval_number(&world.camera.target_z, 0.0, time)?;
    let yaw = eval_number(&world.camera.yaw, 0.0, time)?.to_radians();
    let pitch = eval_number(&world.camera.pitch, 0.0, time)?
        .clamp(-89.0, 89.0)
        .to_radians();
    let distance = eval_number(&world.camera.distance, 3.2, time)?.max(0.05);
    let fov = eval_number(&world.camera.fov, 35.0, time)?
        .clamp(10.0, 100.0)
        .to_radians();
    let yaw_sin = yaw.sin();
    let yaw_cos = yaw.cos();
    let pitch_sin = pitch.sin();
    let pitch_cos = pitch.cos();
    let target = [target_x, target_y, target_z];
    let eye = [
        target_x + yaw_sin * pitch_cos * distance,
        target_y + pitch_sin * distance,
        target_z + yaw_cos * pitch_cos * distance,
    ];
    let mut forward = normalize3([target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]]);
    if !forward[0].is_finite() || !forward[1].is_finite() || !forward[2].is_finite() {
        forward = [0.0, 0.0, -1.0];
    }
    let world_up = [0.0, 1.0, 0.0];
    let mut right = normalize3(cross3(forward, world_up));
    if !right[0].is_finite() || !right[1].is_finite() || !right[2].is_finite() {
        right = [1.0, 0.0, 0.0];
    }
    let up = normalize3(cross3(right, forward));
    let focal_px = (height_f * 0.5) / (fov * 0.5).tan().max(0.001);
    let far = distance.max(1.0) + width_f.max(height_f) / height_f * 24.0;
    Ok(PerspectiveCameraView {
        eye,
        right,
        up,
        forward,
        focal_px,
        near: 0.02,
        far,
    })
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len <= 0.000001 {
        return [f32::NAN, f32::NAN, f32::NAN];
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn draw_actor_debug_projections(
    canvas: &mut RgbaImage,
    graph: &WorldGraph,
    world: &WorldNode,
    asset_root: &Path,
    resolver: &dyn AssetResolver,
    time: WorldTime,
    mesh_cache: &mut HashMap<PathBuf, GlbMeshData>,
) -> Result<(), WorldRenderError> {
    let camera_yaw = eval_number(&world.camera.yaw, 0.0, time)?;
    let camera_pitch = eval_number(&world.camera.pitch, 0.0, time)?;
    let camera_x = eval_number(&world.camera.x, 0.0, time)?;
    let camera_y = eval_number(&world.camera.y, 0.0, time)?;
    let camera_z = eval_number(&world.camera.z, 0.0, time)?;
    let camera_zoom = eval_number(&world.camera.zoom, 1.0, time)?.max(0.05);
    let fov = eval_number(&world.camera.fov, 35.0, time)?.clamp(10.0, 100.0);
    let distance = eval_number(&world.camera.distance, 3.2, time)?.max(0.2);

    for actor in &world.actors {
        let (model_key, mesh) =
            load_glb_mesh_resolved(asset_root, &actor.model, actor.path_style, resolver)?;
        if !mesh_cache.contains_key(&model_key) {
            mesh_cache.insert(model_key.clone(), mesh);
        }
        let mesh = mesh_cache
            .get(&model_key)
            .expect("mesh cache entry inserted before render");
        let x = eval_number(&actor.x, 0.0, time)?;
        let y = eval_number(&actor.y, 0.0, time)?;
        let z = eval_number(&actor.z, 0.0, time)?;
        let yaw = eval_number(&actor.yaw, 0.0, time)?;
        let scale = eval_number(&actor.scale, 1.0, time)?.max(0.01);
        let opacity = eval_number(&actor.opacity, 1.0, time)?.clamp(0.0, 1.0);
        let view = camera_actor_view(
            x,
            y,
            z,
            yaw,
            camera_x,
            camera_y,
            camera_z,
            camera_yaw,
            camera_pitch,
        );
        if mesh.positions.is_empty() || mesh.indices.len() < 3 {
            draw_actor_placeholder(
                canvas,
                actor,
                false,
                view.x,
                view.y,
                view.yaw,
                view.pitch,
                fov,
                distance,
                scale * camera_zoom,
                opacity,
            );
        } else {
            let skinned_positions = skinned_actor_positions(graph, actor, mesh, time)?;
            let positions = skinned_positions.as_deref().unwrap_or(&mesh.positions);
            draw_actor_mesh_projection(
                canvas,
                actor,
                mesh,
                positions,
                view.x,
                view.y,
                view.depth,
                view.yaw,
                view.pitch,
                fov,
                distance,
                scale * camera_zoom,
                opacity,
            );
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_actor_gpu_draws(
    canvas: &mut RgbaImage,
    graph: &WorldGraph,
    world: &WorldNode,
    asset_root: &Path,
    resolver: &dyn AssetResolver,
    time: WorldTime,
    mesh_cache: &mut HashMap<PathBuf, GlbMeshData>,
    gpu_static_draw_cache: &mut HashMap<GpuWorldStaticPlanKey, Vec<GpuWorldStaticDraw>>,
    skinning_strategy_cache: &mut HashMap<SkinningStrategyKey, SkinningMatrixStrategy>,
) -> Result<Vec<GpuWorldDraw>, WorldRenderError> {
    let camera_zoom = eval_number(&world.camera.zoom, 1.0, time)?.max(0.05);
    let camera_view = perspective_camera_view(world, canvas.width(), canvas.height(), time)?;
    let mut draws = Vec::new();

    for actor in &world.actors {
        let (model_key, mesh) =
            load_glb_mesh_resolved(asset_root, &actor.model, actor.path_style, resolver)?;
        if !mesh_cache.contains_key(&model_key) {
            mesh_cache.insert(model_key.clone(), mesh);
        }
        let mesh = mesh_cache
            .get(&model_key)
            .expect("mesh cache entry inserted before render");
        let x = eval_number(&actor.x, 0.0, time)?;
        let y = eval_number(&actor.y, 0.0, time)?;
        let z = eval_number(&actor.z, 0.0, time)?;
        let yaw = eval_number(&actor.yaw, 0.0, time)?;
        let pitch = eval_number(&actor.pitch, 0.0, time)?;
        let roll = eval_number(&actor.roll, 0.0, time)?;
        let scale = eval_number(&actor.scale, 1.0, time)?.max(0.01);
        let opacity = eval_number(&actor.opacity, 1.0, time)?.clamp(0.0, 1.0);
        if mesh.positions.is_empty() || mesh.indices.len() < 3 {
            draw_actor_placeholder(
                canvas,
                actor,
                false,
                x,
                y,
                yaw,
                0.0,
                35.0,
                3.2,
                scale * camera_zoom,
                opacity,
            );
            continue;
        }
        let static_plan_key = GpuWorldStaticPlanKey {
            model_path: model_key.clone(),
            outline: actor
                .material
                .as_ref()
                .is_some_and(|material| material.outline),
            hide_meshes: actor.hide_meshes.clone(),
            hide_materials: actor.hide_materials.clone(),
        };
        if !gpu_static_draw_cache.contains_key(&static_plan_key) {
            let static_draws = build_actor_mesh_gpu_static_draws(actor, mesh, &model_key);
            gpu_static_draw_cache.insert(static_plan_key.clone(), static_draws);
        }
        let static_draws = gpu_static_draw_cache
            .get(&static_plan_key)
            .expect("GPU static draw cache entry inserted before render");
        let actor_draws = build_actor_mesh_gpu_draws(
            graph,
            actor,
            mesh,
            static_draws,
            canvas.width(),
            canvas.height(),
            x,
            y,
            z,
            yaw,
            pitch,
            roll,
            camera_view,
            scale * camera_zoom,
            opacity,
            time,
            &model_key,
            skinning_strategy_cache,
        )?;
        draws.extend(actor_draws);
    }
    Ok(draws)
}

fn skinned_actor_positions(
    graph: &WorldGraph,
    actor: &WorldActor,
    mesh: &GlbMeshData,
    time: WorldTime,
) -> Result<Option<Vec<[f32; 3]>>, WorldRenderError> {
    let Some(skin) = mesh.skin.as_ref() else {
        return Ok(None);
    };
    if mesh.nodes.is_empty()
        || mesh.joints.len() != mesh.positions.len()
        || mesh.weights.len() != mesh.positions.len()
    {
        return Ok(None);
    }

    let overrides = actor_bone_overrides(graph, actor, time)?;
    if overrides.is_empty() {
        return Ok(None);
    }
    if !overrides_match_nodes(mesh, &overrides) {
        return Ok(None);
    }

    let global_matrices = global_node_matrices(mesh, &overrides);
    let joint_matrices = skin
        .joints
        .iter()
        .map(|joint| {
            let global = global_matrices
                .get(joint.node_index)
                .copied()
                .unwrap_or_else(mat4_identity);
            mat4_mul(global, joint.inverse_bind_matrix)
        })
        .collect::<Vec<_>>();

    let mut out = Vec::with_capacity(mesh.positions.len());
    for (idx, position) in mesh.positions.iter().copied().enumerate() {
        let Some(joints) = mesh.joints.get(idx).copied().flatten() else {
            out.push(position);
            continue;
        };
        let Some(weights) = mesh.weights.get(idx).copied().flatten() else {
            out.push(position);
            continue;
        };
        let weight_sum = weights.iter().copied().sum::<f32>();
        if weight_sum <= f32::EPSILON {
            out.push(position);
            continue;
        }

        let mut skinned = [0.0f32; 3];
        for slot in 0..4 {
            let joint_index = joints[slot] as usize;
            let Some(matrix) = joint_matrices.get(joint_index).copied() else {
                continue;
            };
            let weight = weights[slot] / weight_sum;
            if weight <= 0.0 {
                continue;
            }
            let transformed = mat4_transform_point(matrix, position);
            for axis in 0..3 {
                skinned[axis] += transformed[axis] * weight;
            }
        }
        out.push(skinned);
    }
    Ok(Some(out))
}

fn actor_joint_matrices(
    graph: &WorldGraph,
    actor: &WorldActor,
    mesh: &GlbMeshData,
    model_path: &Path,
    mesh_node: Option<usize>,
    time: WorldTime,
    skinning_strategy_cache: &mut HashMap<SkinningStrategyKey, SkinningMatrixStrategy>,
) -> Result<Vec<[f32; 16]>, WorldRenderError> {
    let Some(skin) = mesh.skin.as_ref() else {
        return Ok(vec![mat4_identity()]);
    };
    if mesh.nodes.is_empty()
        || mesh.joints.len() != mesh.positions.len()
        || mesh.weights.len() != mesh.positions.len()
    {
        return Ok(vec![mat4_identity()]);
    }

    let overrides = actor_bone_overrides(graph, actor, time)?;
    if overrides.is_empty() || !overrides_match_nodes(mesh, &overrides) {
        return Ok(vec![mat4_identity(); skin.joints.len().max(1)]);
    }
    let global_matrices = global_node_matrices(mesh, &overrides);
    let cache_key = SkinningStrategyKey {
        model_path: model_path.to_path_buf(),
        mesh_node,
    };
    let strategy = if let Some(strategy) = skinning_strategy_cache.get(&cache_key).copied() {
        strategy
    } else {
        let candidates = skinning_matrix_candidates(mesh, skin, mesh_node, &global_matrices);
        let strategy =
            choose_skinning_matrix_candidate(mesh, mesh_node, candidates).unwrap_or_default();
        skinning_strategy_cache.insert(cache_key, strategy);
        strategy
    };
    let mut joint_matrices =
        matrices_for_skinning_strategy(mesh, skin, mesh_node, &global_matrices, strategy);
    if joint_matrices.is_empty() {
        joint_matrices.push(mat4_identity());
    }
    Ok(joint_matrices)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SkinningStrategyKey {
    model_path: PathBuf,
    mesh_node: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkinningMatrixStrategy {
    BindSpace,
    MeshLocal,
    SkeletonLocal,
    MeshParentLocal,
}

impl Default for SkinningMatrixStrategy {
    fn default() -> Self {
        Self::MeshLocal
    }
}

struct SkinningMatrixCandidate {
    strategy: SkinningMatrixStrategy,
    matrices: Vec<[f32; 16]>,
}

fn skinning_matrix_candidates(
    mesh: &GlbMeshData,
    skin: &crate::world::gltf_loader::GlbSkinData,
    mesh_node: Option<usize>,
    global_matrices: &[[f32; 16]],
) -> Vec<SkinningMatrixCandidate> {
    let mut candidates = vec![
        SkinningMatrixCandidate {
            strategy: SkinningMatrixStrategy::BindSpace,
            matrices: matrices_for_skinning_strategy(
                mesh,
                skin,
                mesh_node,
                global_matrices,
                SkinningMatrixStrategy::BindSpace,
            ),
        },
        SkinningMatrixCandidate {
            strategy: SkinningMatrixStrategy::MeshLocal,
            matrices: matrices_for_skinning_strategy(
                mesh,
                skin,
                mesh_node,
                global_matrices,
                SkinningMatrixStrategy::MeshLocal,
            ),
        },
    ];

    if let Some(skeleton) = skin.skeleton {
        if let Some(skeleton_inverse) = global_matrices
            .get(skeleton)
            .copied()
            .and_then(mat4_inverse_affine)
        {
            candidates.push(SkinningMatrixCandidate {
                strategy: SkinningMatrixStrategy::SkeletonLocal,
                matrices: matrices_for_skinning_strategy(
                    mesh,
                    skin,
                    mesh_node,
                    global_matrices,
                    SkinningMatrixStrategy::SkeletonLocal,
                ),
            });
            let _ = skeleton_inverse;
        }
    }

    if let Some(parent_inverse) = mesh_node
        .and_then(|node_index| mesh.nodes.get(node_index).and_then(|node| node.parent))
        .and_then(|parent| global_matrices.get(parent).copied())
        .and_then(mat4_inverse_affine)
    {
        candidates.push(SkinningMatrixCandidate {
            strategy: SkinningMatrixStrategy::MeshParentLocal,
            matrices: matrices_for_skinning_strategy(
                mesh,
                skin,
                mesh_node,
                global_matrices,
                SkinningMatrixStrategy::MeshParentLocal,
            ),
        });
        let _ = parent_inverse;
    }

    candidates
}

fn matrices_for_skinning_strategy(
    mesh: &GlbMeshData,
    skin: &crate::world::gltf_loader::GlbSkinData,
    mesh_node: Option<usize>,
    global_matrices: &[[f32; 16]],
    strategy: SkinningMatrixStrategy,
) -> Vec<[f32; 16]> {
    let space_inverse = match strategy {
        SkinningMatrixStrategy::BindSpace => mat4_identity(),
        SkinningMatrixStrategy::MeshLocal => mesh_node
            .and_then(|node_index| global_matrices.get(node_index).copied())
            .and_then(mat4_inverse_affine)
            .unwrap_or_else(mat4_identity),
        SkinningMatrixStrategy::SkeletonLocal => skin
            .skeleton
            .and_then(|node_index| global_matrices.get(node_index).copied())
            .and_then(mat4_inverse_affine)
            .unwrap_or_else(mat4_identity),
        SkinningMatrixStrategy::MeshParentLocal => mesh_node
            .and_then(|node_index| mesh.nodes.get(node_index).and_then(|node| node.parent))
            .and_then(|parent| global_matrices.get(parent).copied())
            .and_then(mat4_inverse_affine)
            .unwrap_or_else(mat4_identity),
    };

    skin.joints
        .iter()
        .map(|joint| {
            let global = global_matrices
                .get(joint.node_index)
                .copied()
                .unwrap_or_else(mat4_identity);
            let bind = mat4_mul(global, joint.inverse_bind_matrix);
            match strategy {
                SkinningMatrixStrategy::BindSpace => bind,
                _ => mat4_mul(space_inverse, bind),
            }
        })
        .collect()
}

fn choose_skinning_matrix_candidate(
    mesh: &GlbMeshData,
    mesh_node: Option<usize>,
    candidates: Vec<SkinningMatrixCandidate>,
) -> Option<SkinningMatrixStrategy> {
    if candidates.is_empty() {
        return None;
    }
    let sample_indices = skinning_strategy_sample_indices(mesh, mesh_node, 4096);
    if sample_indices.is_empty() {
        return candidates
            .into_iter()
            .next()
            .map(|candidate| candidate.strategy);
    }
    let (raw_min, raw_max) = bounds_for_indices(mesh, &sample_indices, None);
    let raw_extent = bounds_extent(raw_min, raw_max);
    let raw_center = bounds_center(raw_min, raw_max);

    candidates
        .into_iter()
        .map(|candidate| {
            let (min, max) = bounds_for_indices(mesh, &sample_indices, Some(&candidate.matrices));
            let extent = bounds_extent(min, max);
            let center = bounds_center(min, max);
            let score = skinning_strategy_score(raw_extent, raw_center, extent, center);
            (score, candidate)
        })
        .min_by(|(a, a_candidate), (b, b_candidate)| {
            a.total_cmp(b).then_with(|| {
                // Prefer the standard glTF mesh-local path when candidates are effectively tied.
                skinning_strategy_rank(a_candidate.strategy)
                    .cmp(&skinning_strategy_rank(b_candidate.strategy))
            })
        })
        .map(|(_, candidate)| candidate.strategy)
}

fn skinning_strategy_rank(strategy: SkinningMatrixStrategy) -> u8 {
    match strategy {
        SkinningMatrixStrategy::MeshLocal => 0,
        SkinningMatrixStrategy::MeshParentLocal => 1,
        SkinningMatrixStrategy::SkeletonLocal => 2,
        SkinningMatrixStrategy::BindSpace => 3,
    }
}

fn skinning_strategy_sample_indices(
    mesh: &GlbMeshData,
    mesh_node: Option<usize>,
    max_samples: usize,
) -> Vec<usize> {
    let mut seen = vec![false; mesh.positions.len()];
    let mut indices = Vec::with_capacity(max_samples.min(mesh.positions.len()));
    for triangle in &mesh.triangles {
        if mesh_node.is_some() && triangle.mesh_node != mesh_node {
            continue;
        }
        for index in triangle.indices {
            let index = index as usize;
            if index >= seen.len() || seen[index] {
                continue;
            }
            seen[index] = true;
            indices.push(index);
            if indices.len() >= max_samples {
                return indices;
            }
        }
    }
    if indices.is_empty() && mesh_node.is_some() {
        return skinning_strategy_sample_indices(mesh, None, max_samples);
    }
    indices
}

fn bounds_for_indices(
    mesh: &GlbMeshData,
    indices: &[usize],
    matrices: Option<&[[f32; 16]]>,
) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for index in indices {
        let Some(position) = mesh.positions.get(*index).copied() else {
            continue;
        };
        let point = matrices
            .and_then(|matrices| {
                let joints = mesh.joints.get(*index).copied().flatten()?;
                let weights = mesh.weights.get(*index).copied().flatten()?;
                Some(skinning_transform_position(
                    position, joints, weights, matrices,
                ))
            })
            .unwrap_or(position);
        if point.iter().all(|value| value.is_finite()) {
            accumulate_bounds3(&mut min, &mut max, point);
        }
    }
    (min, max)
}

fn skinning_transform_position(
    position: [f32; 3],
    joints: [u16; 4],
    weights: [f32; 4],
    matrices: &[[f32; 16]],
) -> [f32; 3] {
    let weight_sum = weights.iter().sum::<f32>();
    if weight_sum <= f32::EPSILON {
        return position;
    }
    let mut out = [0.0f32; 3];
    for slot in 0..4 {
        let weight = weights[slot] / weight_sum;
        if weight <= 0.0 {
            continue;
        }
        let Some(matrix) = matrices.get(joints[slot] as usize).copied() else {
            continue;
        };
        let transformed = mat4_transform_point(matrix, position);
        for axis in 0..3 {
            out[axis] += transformed[axis] * weight;
        }
    }
    out
}

fn bounds_extent(min: [f32; 3], max: [f32; 3]) -> [f32; 3] {
    [
        (max[0] - min[0]).abs().max(0.0001),
        (max[1] - min[1]).abs().max(0.0001),
        (max[2] - min[2]).abs().max(0.0001),
    ]
}

fn bounds_center(min: [f32; 3], max: [f32; 3]) -> [f32; 3] {
    [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ]
}

fn skinning_strategy_score(
    raw_extent: [f32; 3],
    raw_center: [f32; 3],
    extent: [f32; 3],
    center: [f32; 3],
) -> f32 {
    let mut score = 0.0;
    for axis in 0..3 {
        let scale_ratio = (extent[axis] / raw_extent[axis]).max(0.0001);
        score += scale_ratio.ln().abs() * 4.0;
        score += ((center[axis] - raw_center[axis]).abs() / raw_extent[axis].max(0.0001)).min(10.0);
    }
    score
}

fn overrides_match_nodes(mesh: &GlbMeshData, overrides: &HashMap<String, BoneOverride>) -> bool {
    mesh.nodes
        .iter()
        .filter_map(|node| node.name.as_deref())
        .any(|name| overrides.contains_key(name))
}

#[derive(Debug, Clone, Copy)]
struct BoneOverride {
    translation: [f32; 3],
    rotation_deg: [f32; 3],
    scale: f32,
}

impl BoneOverride {
    fn identity() -> Self {
        Self {
            translation: [0.0, 0.0, 0.0],
            rotation_deg: [0.0, 0.0, 0.0],
            scale: 1.0,
        }
    }

    fn add_weighted(&mut self, other: Self, weight: f32) {
        for axis in 0..3 {
            self.translation[axis] += other.translation[axis] * weight;
            self.rotation_deg[axis] += other.rotation_deg[axis] * weight;
        }
        self.scale *= 1.0 + (other.scale - 1.0) * weight;
    }

    fn is_identity(self) -> bool {
        self.translation.iter().all(|value| value.abs() <= 0.0001)
            && self.rotation_deg.iter().all(|value| value.abs() <= 0.0001)
            && (self.scale - 1.0).abs() <= 0.0001
    }
}

fn actor_bone_overrides(
    graph: &WorldGraph,
    actor: &WorldActor,
    time: WorldTime,
) -> Result<HashMap<String, BoneOverride>, WorldRenderError> {
    let profile = actor_model_profile(graph, actor);
    let axis_map = profile.and_then(|profile| profile.bone_axis_map.as_ref());
    let retarget = actor
        .retarget
        .as_deref()
        .and_then(|id| graph.retargets.iter().find(|retarget| retarget.id == id));
    let profile_retarget = profile.and_then(|profile| profile.retarget.as_ref());
    let retarget_preset = retarget
        .map(|retarget| retarget.preset.as_str())
        .or_else(|| profile_retarget.map(|retarget| retarget.preset.as_str()))
        .or_else(|| profile.map(|profile| profile.preset.as_str()));
    let bone_to_node = if let Some(retarget) = retarget {
        retarget_maps_to_node_lookup(&retarget.maps)
    } else if let Some(retarget) = profile_retarget {
        retarget_maps_to_node_lookup(&retarget.maps)
    } else {
        HashMap::new()
    };

    let mut out = HashMap::<String, BoneOverride>::new();
    if let Some(axis_map) = axis_map {
        for axis in &axis_map.axes {
            let transform = rest_pose_axis_transform(axis, axis_map, time)?;
            if transform.is_identity() {
                continue;
            }
            let node_name = bone_to_node
                .get(axis.bone.as_str())
                .copied()
                .unwrap_or(axis.bone.as_str());
            out.entry(node_name.to_string())
                .or_insert_with(BoneOverride::identity)
                .add_weighted(transform, 1.0);
        }
    }

    for apply in graph
        .apply_actions
        .iter()
        .filter(|apply| apply.target == actor.id)
    {
        let Some(action) = graph
            .actions
            .iter()
            .find(|action| action.id == apply.action)
        else {
            continue;
        };
        if let Some(retarget_preset) = retarget_preset {
            if action.skeleton != retarget_preset {
                continue;
            }
        }
        let Some(action_time) = action_local_time_sec(action, apply.at_ms, apply.r#loop, time)
        else {
            continue;
        };
        let weight = eval_number(&apply.weight, 1.0, time)?.clamp(0.0, 1.0);
        if weight <= 0.0 {
            continue;
        }

        for (bone_id, transform) in action_pose_transform(action, action_time, time, axis_map)? {
            if transform.is_identity() {
                continue;
            }
            let node_name = bone_to_node
                .get(bone_id.as_str())
                .copied()
                .unwrap_or(bone_id.as_str());
            out.entry(node_name.to_string())
                .or_insert_with(BoneOverride::identity)
                .add_weighted(transform, weight);
        }
    }
    Ok(out)
}

fn actor_model_profile<'a>(
    graph: &'a WorldGraph,
    actor: &WorldActor,
) -> Option<&'a WorldModelProfile> {
    actor
        .profile
        .as_deref()
        .and_then(|id| graph.model_profiles.iter().find(|profile| profile.id == id))
}

fn retarget_maps_to_node_lookup(maps: &[WorldRetargetMap]) -> HashMap<&str, &str> {
    maps.iter()
        .map(|map| (map.to.as_str(), map.from.as_str()))
        .collect::<HashMap<_, _>>()
}

fn rest_pose_axis_transform(
    axis: &WorldBoneAxis,
    axis_map: &WorldBoneAxisMap,
    time: WorldTime,
) -> Result<BoneOverride, WorldRenderError> {
    let bone = WorldActionBone {
        id: axis.bone.clone(),
        x: None,
        y: None,
        z: None,
        rotation: None,
        rotation_x: None,
        rotation_y: None,
        rotation_z: None,
        forward: axis.rest_forward.clone(),
        side: axis.rest_side.clone(),
        twist: axis.rest_twist.clone(),
        bend: axis.rest_bend.clone(),
        turn: axis.rest_turn.clone(),
        scale: None,
        opacity: None,
    };
    interpolate_bone(Some(&bone), Some(&bone), 0.0, time, Some(axis_map))
}

fn action_local_time_sec(
    action: &WorldAction,
    at_ms: u64,
    should_loop: bool,
    time: WorldTime,
) -> Option<f32> {
    let duration_sec = action.duration_ms as f32 / 1000.0;
    if duration_sec <= f32::EPSILON {
        return Some(0.0);
    }
    let local = time.time_sec() - at_ms as f32 / 1000.0;
    if local < 0.0 {
        return None;
    }
    if should_loop {
        Some(local % duration_sec)
    } else {
        Some(local.min(duration_sec))
    }
}

fn action_pose_transform(
    action: &WorldAction,
    action_time_sec: f32,
    time: WorldTime,
    axis_map: Option<&WorldBoneAxisMap>,
) -> Result<HashMap<String, BoneOverride>, WorldRenderError> {
    if action.poses.is_empty() {
        return Ok(HashMap::new());
    }
    let mut poses = action.poses.iter().collect::<Vec<_>>();
    poses.sort_by(|a, b| a.t.total_cmp(&b.t));
    let first = poses[0];
    let last = *poses.last().expect("poses is not empty");
    let (before, after) = if action_time_sec <= first.t {
        (first, first)
    } else if action_time_sec >= last.t {
        (last, last)
    } else {
        let mut pair = (first, first);
        for window in poses.windows(2) {
            let a = window[0];
            let b = window[1];
            if action_time_sec >= a.t && action_time_sec <= b.t {
                pair = (a, b);
                break;
            }
        }
        pair
    };
    let alpha = if (after.t - before.t).abs() <= f32::EPSILON {
        0.0
    } else {
        ((action_time_sec - before.t) / (after.t - before.t)).clamp(0.0, 1.0)
    };

    let before_bones = before
        .bones
        .iter()
        .map(|bone| (bone.id.as_str(), bone))
        .collect::<HashMap<_, _>>();
    let after_bones = after
        .bones
        .iter()
        .map(|bone| (bone.id.as_str(), bone))
        .collect::<HashMap<_, _>>();
    let mut ids = before_bones.keys().copied().collect::<Vec<_>>();
    for id in after_bones.keys().copied() {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }

    let mut out = HashMap::new();
    for id in ids {
        let transform = interpolate_bone(
            before_bones.get(id).copied(),
            after_bones.get(id).copied(),
            alpha,
            time,
            axis_map,
        )?;
        out.insert(id.to_string(), transform);
    }
    Ok(out)
}

fn interpolate_bone(
    before: Option<&WorldActionBone>,
    after: Option<&WorldActionBone>,
    alpha: f32,
    time: WorldTime,
    axis_map: Option<&WorldBoneAxisMap>,
) -> Result<BoneOverride, WorldRenderError> {
    let lerp_field =
        |a: Option<&String>, b: Option<&String>, default: f32| -> Result<f32, WorldRenderError> {
            let av = match a {
                Some(expr) => eval_number(expr, default, time)?,
                None => default,
            };
            let bv = match b {
                Some(expr) => eval_number(expr, av, time)?,
                None => av,
            };
            Ok(av + (bv - av) * alpha)
        };

    let before_rotation_z =
        before.and_then(|bone| bone.rotation_z.as_ref().or(bone.rotation.as_ref()));
    let after_rotation_z =
        after.and_then(|bone| bone.rotation_z.as_ref().or(bone.rotation.as_ref()));
    let mut rotation_deg = [
        lerp_field(
            before.and_then(|bone| bone.rotation_x.as_ref()),
            after.and_then(|bone| bone.rotation_x.as_ref()),
            0.0,
        )?,
        lerp_field(
            before.and_then(|bone| bone.rotation_y.as_ref()),
            after.and_then(|bone| bone.rotation_y.as_ref()),
            0.0,
        )?,
        lerp_field(before_rotation_z, after_rotation_z, 0.0)?,
    ];
    if let Some(bone_id) = before
        .map(|bone| bone.id.as_str())
        .or_else(|| after.map(|bone| bone.id.as_str()))
        && let Some(axis) = bone_axis(axis_map, bone_id)
    {
        apply_semantic_rotation(
            &mut rotation_deg,
            axis.forward.as_deref(),
            lerp_field(
                before.and_then(|bone| bone.forward.as_ref()),
                after.and_then(|bone| bone.forward.as_ref()),
                0.0,
            )?,
        );
        apply_semantic_rotation(
            &mut rotation_deg,
            axis.side.as_deref(),
            lerp_field(
                before.and_then(|bone| bone.side.as_ref()),
                after.and_then(|bone| bone.side.as_ref()),
                0.0,
            )?,
        );
        apply_semantic_rotation(
            &mut rotation_deg,
            axis.twist.as_deref(),
            lerp_field(
                before.and_then(|bone| bone.twist.as_ref()),
                after.and_then(|bone| bone.twist.as_ref()),
                0.0,
            )?,
        );
        apply_semantic_rotation(
            &mut rotation_deg,
            axis.bend.as_deref(),
            lerp_field(
                before.and_then(|bone| bone.bend.as_ref()),
                after.and_then(|bone| bone.bend.as_ref()),
                0.0,
            )?,
        );
        apply_semantic_rotation(
            &mut rotation_deg,
            axis.turn.as_deref(),
            lerp_field(
                before.and_then(|bone| bone.turn.as_ref()),
                after.and_then(|bone| bone.turn.as_ref()),
                0.0,
            )?,
        );
    }
    Ok(BoneOverride {
        translation: [
            lerp_field(
                before.and_then(|bone| bone.x.as_ref()),
                after.and_then(|bone| bone.x.as_ref()),
                0.0,
            )?,
            lerp_field(
                before.and_then(|bone| bone.y.as_ref()),
                after.and_then(|bone| bone.y.as_ref()),
                0.0,
            )?,
            lerp_field(
                before.and_then(|bone| bone.z.as_ref()),
                after.and_then(|bone| bone.z.as_ref()),
                0.0,
            )?,
        ],
        rotation_deg,
        scale: lerp_field(
            before.and_then(|bone| bone.scale.as_ref()),
            after.and_then(|bone| bone.scale.as_ref()),
            1.0,
        )?,
    })
}

fn bone_axis<'a>(
    axis_map: Option<&'a WorldBoneAxisMap>,
    bone_id: &str,
) -> Option<&'a WorldBoneAxis> {
    axis_map.and_then(|axis_map| axis_map.axes.iter().find(|axis| axis.bone == bone_id))
}

fn apply_semantic_rotation(rotation_deg: &mut [f32; 3], binding: Option<&str>, value: f32) {
    if value.abs() <= f32::EPSILON {
        return;
    }
    let Some((axis, scale)) = binding.and_then(parse_axis_binding) else {
        return;
    };
    rotation_deg[axis] += value * scale;
}

fn parse_axis_binding(raw: &str) -> Option<(usize, f32)> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    let (axis_raw, scale_raw) = text.split_once(':').unwrap_or((text, "1"));
    let mut axis_text = axis_raw.trim();
    let mut sign = 1.0f32;
    if let Some(stripped) = axis_text.strip_prefix('-') {
        sign = -1.0;
        axis_text = stripped.trim();
    } else if let Some(stripped) = axis_text.strip_prefix('+') {
        axis_text = stripped.trim();
    }
    let axis_key = axis_text.to_ascii_lowercase().replace(['_', '-'], "");
    let axis = match axis_key.as_str() {
        "x" | "rx" | "rotationx" => 0,
        "y" | "ry" | "rotationy" => 1,
        "z" | "rz" | "rotationz" | "rotation" => 2,
        _ => return None,
    };
    let scale = scale_raw.trim().parse::<f32>().unwrap_or(1.0);
    Some((axis, sign * scale))
}

fn global_node_matrices(
    mesh: &GlbMeshData,
    overrides: &HashMap<String, BoneOverride>,
) -> Vec<[f32; 16]> {
    let local = mesh
        .nodes
        .iter()
        .map(|node| {
            let base = node
                .matrix
                .unwrap_or_else(|| mat4_from_trs(node.translation, node.rotation, node.scale));
            let Some(name) = node.name.as_deref() else {
                return base;
            };
            let Some(override_transform) = overrides.get(name).copied() else {
                return base;
            };
            mat4_mul(base, mat4_from_override(override_transform))
        })
        .collect::<Vec<_>>();
    let mut global = vec![None; mesh.nodes.len()];
    for index in 0..mesh.nodes.len() {
        compute_global_node_matrix(index, &mesh.nodes, &local, &mut global);
    }
    global
        .into_iter()
        .map(|matrix| matrix.unwrap_or_else(mat4_identity))
        .collect()
}

fn compute_global_node_matrix(
    index: usize,
    nodes: &[crate::world::gltf_loader::GlbNodeData],
    local: &[[f32; 16]],
    global: &mut [Option<[f32; 16]>],
) -> [f32; 16] {
    if let Some(matrix) = global.get(index).copied().flatten() {
        return matrix;
    }
    let local_matrix = local.get(index).copied().unwrap_or_else(mat4_identity);
    let matrix = nodes
        .get(index)
        .and_then(|node| node.parent)
        .map(|parent| {
            mat4_mul(
                compute_global_node_matrix(parent, nodes, local, global),
                local_matrix,
            )
        })
        .unwrap_or(local_matrix);
    if let Some(slot) = global.get_mut(index) {
        *slot = Some(matrix);
    }
    matrix
}

fn mat4_from_override(transform: BoneOverride) -> [f32; 16] {
    let translation = mat4_translation(transform.translation);
    let rotation = mat4_mul(
        mat4_mul(
            mat4_rotation_z(transform.rotation_deg[2].to_radians()),
            mat4_rotation_y(transform.rotation_deg[1].to_radians()),
        ),
        mat4_rotation_x(transform.rotation_deg[0].to_radians()),
    );
    let scale = mat4_scale([transform.scale, transform.scale, transform.scale]);
    mat4_mul(mat4_mul(translation, rotation), scale)
}

fn mat4_from_trs(translation: [f32; 3], rotation: [f32; 4], scale: [f32; 3]) -> [f32; 16] {
    mat4_mul(
        mat4_mul(mat4_translation(translation), mat4_from_quat(rotation)),
        mat4_scale(scale),
    )
}

fn mat4_identity() -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]
}

fn mat4_translation(translation: [f32; 3]) -> [f32; 16] {
    [
        1.0,
        0.0,
        0.0,
        0.0, //
        0.0,
        1.0,
        0.0,
        0.0, //
        0.0,
        0.0,
        1.0,
        0.0, //
        translation[0],
        translation[1],
        translation[2],
        1.0,
    ]
}

fn mat4_scale(scale: [f32; 3]) -> [f32; 16] {
    [
        scale[0], 0.0, 0.0, 0.0, //
        0.0, scale[1], 0.0, 0.0, //
        0.0, 0.0, scale[2], 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]
}

fn mat4_rotation_x(angle: f32) -> [f32; 16] {
    let (sin, cos) = angle.sin_cos();
    [
        1.0, 0.0, 0.0, 0.0, //
        0.0, cos, sin, 0.0, //
        0.0, -sin, cos, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]
}

fn mat4_rotation_y(angle: f32) -> [f32; 16] {
    let (sin, cos) = angle.sin_cos();
    [
        cos, 0.0, -sin, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        sin, 0.0, cos, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]
}

fn mat4_rotation_z(angle: f32) -> [f32; 16] {
    let (sin, cos) = angle.sin_cos();
    [
        cos, sin, 0.0, 0.0, //
        -sin, cos, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]
}

fn mat4_from_quat(quat: [f32; 4]) -> [f32; 16] {
    let [x, y, z, w] = quat;
    let len = (x * x + y * y + z * z + w * w).sqrt();
    if len <= f32::EPSILON {
        return mat4_identity();
    }
    let x = x / len;
    let y = y / len;
    let z = z / len;
    let w = w / len;
    let x2 = x + x;
    let y2 = y + y;
    let z2 = z + z;
    let xx = x * x2;
    let xy = x * y2;
    let xz = x * z2;
    let yy = y * y2;
    let yz = y * z2;
    let zz = z * z2;
    let wx = w * x2;
    let wy = w * y2;
    let wz = w * z2;
    [
        1.0 - (yy + zz),
        xy + wz,
        xz - wy,
        0.0,
        xy - wz,
        1.0 - (xx + zz),
        yz + wx,
        0.0,
        xz + wy,
        yz - wx,
        1.0 - (xx + yy),
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

fn mat4_mul(a: [f32; 16], b: [f32; 16]) -> [f32; 16] {
    let mut out = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            out[col * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum();
        }
    }
    out
}

fn mat4_inverse_affine(matrix: [f32; 16]) -> Option<[f32; 16]> {
    let a00 = matrix[0];
    let a01 = matrix[4];
    let a02 = matrix[8];
    let a10 = matrix[1];
    let a11 = matrix[5];
    let a12 = matrix[9];
    let a20 = matrix[2];
    let a21 = matrix[6];
    let a22 = matrix[10];
    let det = a00 * (a11 * a22 - a12 * a21) - a01 * (a10 * a22 - a12 * a20)
        + a02 * (a10 * a21 - a11 * a20);
    if det.abs() <= 1.0e-8 {
        return None;
    }
    let inv_det = 1.0 / det;
    let r00 = (a11 * a22 - a12 * a21) * inv_det;
    let r01 = (a02 * a21 - a01 * a22) * inv_det;
    let r02 = (a01 * a12 - a02 * a11) * inv_det;
    let r10 = (a12 * a20 - a10 * a22) * inv_det;
    let r11 = (a00 * a22 - a02 * a20) * inv_det;
    let r12 = (a02 * a10 - a00 * a12) * inv_det;
    let r20 = (a10 * a21 - a11 * a20) * inv_det;
    let r21 = (a01 * a20 - a00 * a21) * inv_det;
    let r22 = (a00 * a11 - a01 * a10) * inv_det;
    let tx = matrix[12];
    let ty = matrix[13];
    let tz = matrix[14];
    Some([
        r00,
        r10,
        r20,
        0.0,
        r01,
        r11,
        r21,
        0.0,
        r02,
        r12,
        r22,
        0.0,
        -(r00 * tx + r01 * ty + r02 * tz),
        -(r10 * tx + r11 * ty + r12 * tz),
        -(r20 * tx + r21 * ty + r22 * tz),
        1.0,
    ])
}

fn mat4_transform_point(matrix: [f32; 16], point: [f32; 3]) -> [f32; 3] {
    [
        matrix[0] * point[0] + matrix[4] * point[1] + matrix[8] * point[2] + matrix[12],
        matrix[1] * point[0] + matrix[5] * point[1] + matrix[9] * point[2] + matrix[13],
        matrix[2] * point[0] + matrix[6] * point[1] + matrix[10] * point[2] + matrix[14],
    ]
}

#[allow(clippy::too_many_arguments)]
fn draw_actor_mesh_projection(
    canvas: &mut RgbaImage,
    actor: &WorldActor,
    mesh: &GlbMeshData,
    positions: &[[f32; 3]],
    world_x: f32,
    world_y: f32,
    world_depth: f32,
    view_yaw_deg: f32,
    camera_pitch_deg: f32,
    fov: f32,
    distance: f32,
    scale: f32,
    opacity: f32,
) {
    let width = canvas.width() as f32;
    let height = canvas.height() as f32;
    let model_height = (mesh.bounds_max[1] - mesh.bounds_min[1]).abs().max(0.001);
    let model_center_x = (mesh.bounds_min[0] + mesh.bounds_max[0]) * 0.5;
    let model_center_z = (mesh.bounds_min[2] + mesh.bounds_max[2]) * 0.5;
    let px_per_world = (height / distance) * (35.0 / fov).clamp(0.35, 2.5);
    let model_px = (height * 0.58 * scale * (3.2 / distance).clamp(0.25, 4.0)) / model_height;
    let cx = width * 0.5 + world_x * px_per_world;
    let ground_y = height * 0.82 - world_y * px_per_world;
    let yaw = view_yaw_deg.to_radians();
    let cos_y = yaw.cos();
    let sin_y = yaw.sin();
    let pitch = camera_pitch_deg.to_radians();
    let cos_p = pitch.cos();
    let sin_p = pitch.sin();

    let mut projected = Vec::<([f32; 2], f32)>::with_capacity(positions.len());
    for position in positions {
        let x = position[0] - model_center_x;
        let y = position[1] - mesh.bounds_min[1];
        let z = position[2] - model_center_z;
        let rx = x * cos_y + z * sin_y;
        let rz = -x * sin_y + z * cos_y;
        let ry = y * cos_p - rz * sin_p;
        let rz = y * sin_p + rz * cos_p + world_depth * WORLD_DEPTH_SORT_SCALE;
        projected.push(([cx + rx * model_px, ground_y - ry * model_px], rz));
    }

    let base = if actor
        .material
        .as_ref()
        .is_some_and(|material| material.outline)
    {
        [93, 126, 178]
    } else {
        [111, 145, 190]
    };
    let mut triangles = Vec::<ProjectedTriangle>::new();
    let triangle_source = mesh
        .triangles
        .iter()
        .map(|triangle| (triangle.indices, triangle.material));
    for (indices, material) in triangle_source {
        let Some(a) = projected.get(indices[0] as usize) else {
            continue;
        };
        let Some(b) = projected.get(indices[1] as usize) else {
            continue;
        };
        let Some(c) = projected.get(indices[2] as usize) else {
            continue;
        };
        let ax = b.0[0] - a.0[0];
        let ay = b.0[1] - a.0[1];
        let bx = c.0[0] - a.0[0];
        let by = c.0[1] - a.0[1];
        let screen_cross = ax * by - ay * bx;
        if screen_cross.abs() < 0.01 {
            continue;
        }
        let shade = if screen_cross < 0.0 { 0.78 } else { 1.0 };
        let uvs = [
            mesh.texcoords.get(indices[0] as usize).copied().flatten(),
            mesh.texcoords.get(indices[1] as usize).copied().flatten(),
            mesh.texcoords.get(indices[2] as usize).copied().flatten(),
        ];
        triangles.push(ProjectedTriangle {
            depth: (a.1 + b.1 + c.1) / 3.0,
            points: [a.0, b.0, c.0],
            uvs,
            material,
            shade,
        });
    }
    triangles.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    for triangle in triangles {
        if let Some((texture, material_factor, uvs)) =
            textured_triangle_source(mesh, triangle.material, triangle.uvs)
        {
            fill_textured_triangle(
                canvas,
                triangle.points,
                uvs,
                texture.width,
                texture.height,
                &texture.rgba,
                material_factor,
                triangle.shade,
                opacity,
            );
        } else {
            fill_triangle(
                canvas,
                triangle.points,
                material_color(mesh, triangle.material, base, triangle.shade, opacity),
            );
        }
    }
}

struct ProjectedTriangle {
    depth: f32,
    points: [[f32; 2]; 3],
    uvs: [Option<[f32; 2]>; 3],
    material: Option<usize>,
    shade: f32,
}

#[derive(Debug, Clone, Copy)]
struct GpuWorldVertex {
    position: [f32; 3],
    normal: [f32; 3],
    joints: [f32; 4],
    weights: [f32; 4],
    uv: [f32; 2],
    color: [f32; 4],
}

#[derive(Debug)]
struct GpuWorldDraw {
    resource_key: GpuWorldResourceKey,
    instance_key: GpuWorldInstanceKey,
    vertices: Arc<Vec<GpuWorldVertex>>,
    texture: Arc<GpuWorldTexture>,
    bone_matrices: Vec<[f32; 16]>,
    params: GpuWorldParams,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GpuWorldTexture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GpuWorldDrawKey {
    material: Option<usize>,
    texture: Option<usize>,
    mesh_node: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GpuWorldResourceKey {
    model_path: PathBuf,
    draw_key: GpuWorldDrawKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GpuWorldInstanceKey {
    actor_id: String,
    resource_key: GpuWorldResourceKey,
}

#[derive(Debug)]
struct GpuWorldDrawChunk {
    key: GpuWorldDrawKey,
    texture: GpuWorldTexture,
    vertices: Vec<GpuWorldVertex>,
    mesh_node: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GpuWorldStaticPlanKey {
    model_path: PathBuf,
    outline: bool,
    hide_meshes: Vec<String>,
    hide_materials: Vec<String>,
}

#[derive(Debug, Clone)]
struct GpuWorldStaticDraw {
    resource_key: GpuWorldResourceKey,
    vertices: Arc<Vec<GpuWorldVertex>>,
    texture: Arc<GpuWorldTexture>,
    mesh_node: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
struct GpuWorldParams {
    canvas: [f32; 4],
    model: [f32; 4],
    actor: [f32; 4],
    actor_rotation: [f32; 4],
    camera0: [f32; 4],
    camera1: [f32; 4],
    camera2: [f32; 4],
    camera3: [f32; 4],
    style: [f32; 4],
}

#[derive(Debug, Clone, Copy)]
struct GpuGroundGridParams {
    canvas: [f32; 4],
    camera0: [f32; 4],
    camera1: [f32; 4],
    camera2: [f32; 4],
    camera3: [f32; 4],
    options: [f32; 4],
}

impl GpuGroundGridParams {
    fn from_camera(width: u32, height: u32, camera_view: PerspectiveCameraView) -> Self {
        let width_f = width.max(1) as f32;
        let height_f = height.max(1) as f32;
        Self {
            canvas: [width_f, height_f, width_f * 0.5, height_f * 0.5],
            camera0: [
                camera_view.eye[0],
                camera_view.eye[1],
                camera_view.eye[2],
                camera_view.focal_px,
            ],
            camera1: [
                camera_view.right[0],
                camera_view.right[1],
                camera_view.right[2],
                camera_view.near,
            ],
            camera2: [
                camera_view.up[0],
                camera_view.up[1],
                camera_view.up[2],
                camera_view.far,
            ],
            camera3: [
                camera_view.forward[0],
                camera_view.forward[1],
                camera_view.forward[2],
                0.0,
            ],
            // x: opacity, y/z: distance fade start/end, w: base grid size.
            options: [0.95, 30.0, 70.0, 1.0],
        }
    }

    fn debug_from_camera(width: u32, height: u32, camera_view: PerspectiveCameraView) -> Self {
        let mut params = Self::from_camera(width, height, camera_view);
        // options.y < 0 acts as a debug sentinel in WGSL:
        // high contrast, thicker lines, and no distance fade.
        params.options = [1.0, -1.0, -1.0, 1.0];
        params
    }
}

#[derive(Debug, Clone, Copy)]
struct PerspectiveCameraView {
    eye: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    forward: [f32; 3],
    focal_px: f32,
    near: f32,
    far: f32,
}

#[allow(clippy::too_many_arguments)]
fn build_actor_mesh_gpu_draws(
    graph: &WorldGraph,
    actor: &WorldActor,
    mesh: &GlbMeshData,
    static_draws: &[GpuWorldStaticDraw],
    width: u32,
    height: u32,
    actor_x: f32,
    actor_y: f32,
    actor_z: f32,
    actor_yaw_deg: f32,
    actor_pitch_deg: f32,
    actor_roll_deg: f32,
    camera_view: PerspectiveCameraView,
    scale: f32,
    opacity: f32,
    time: WorldTime,
    model_path: &Path,
    skinning_strategy_cache: &mut HashMap<SkinningStrategyKey, SkinningMatrixStrategy>,
) -> Result<Vec<GpuWorldDraw>, WorldRenderError> {
    let width_f = width.max(1) as f32;
    let height_f = height.max(1) as f32;
    let model_height = (mesh.bounds_max[1] - mesh.bounds_min[1]).abs().max(0.001);
    let model_center_x = (mesh.bounds_min[0] + mesh.bounds_max[0]) * 0.5;
    let model_center_z = (mesh.bounds_min[2] + mesh.bounds_max[2]) * 0.5;
    let world_scale = scale / model_height;
    let params = GpuWorldParams {
        canvas: [width_f, height_f, width_f * 0.5, height_f * 0.5],
        model: [
            model_center_x,
            mesh.bounds_min[1],
            model_center_z,
            world_scale,
        ],
        actor: [actor_x, actor_y, actor_z, actor_yaw_deg.to_radians()],
        actor_rotation: [
            actor_pitch_deg.to_radians(),
            actor_roll_deg.to_radians(),
            0.0,
            0.0,
        ],
        camera0: [
            camera_view.eye[0],
            camera_view.eye[1],
            camera_view.eye[2],
            camera_view.focal_px,
        ],
        camera1: [
            camera_view.right[0],
            camera_view.right[1],
            camera_view.right[2],
            camera_view.near,
        ],
        camera2: [
            camera_view.up[0],
            camera_view.up[1],
            camera_view.up[2],
            camera_view.far,
        ],
        camera3: [
            camera_view.forward[0],
            camera_view.forward[1],
            camera_view.forward[2],
            0.0,
        ],
        style: [
            opacity.clamp(0.0, 1.0),
            actor_material_light_mix(actor),
            0.0,
            0.0,
        ],
    };
    let mut draws = Vec::<GpuWorldDraw>::with_capacity(static_draws.len());
    for static_draw in static_draws {
        if static_draw.vertices.is_empty() {
            continue;
        }
        let bone_matrices = actor_joint_matrices(
            graph,
            actor,
            mesh,
            model_path,
            static_draw.mesh_node,
            time,
            skinning_strategy_cache,
        )?;
        draws.push(GpuWorldDraw {
            instance_key: GpuWorldInstanceKey {
                actor_id: actor.id.clone(),
                resource_key: static_draw.resource_key.clone(),
            },
            resource_key: static_draw.resource_key.clone(),
            vertices: Arc::clone(&static_draw.vertices),
            texture: Arc::clone(&static_draw.texture),
            bone_matrices: bone_matrices.clone(),
            params,
        });
    }
    Ok(draws)
}

fn actor_material_light_mix(actor: &WorldActor) -> f32 {
    match actor.material.as_ref().map(|material| &material.style) {
        Some(WorldMaterialStyle::Pbr) | None => 1.0,
        Some(WorldMaterialStyle::Toon | WorldMaterialStyle::Unlit) => 0.0,
    }
}

fn build_actor_mesh_gpu_static_draws(
    actor: &WorldActor,
    mesh: &GlbMeshData,
    model_path: &Path,
) -> Vec<GpuWorldStaticDraw> {
    let fallback = if actor
        .material
        .as_ref()
        .is_some_and(|material| material.outline)
    {
        [93, 126, 178]
    } else {
        [111, 145, 190]
    };
    let mut chunks = Vec::<GpuWorldDrawChunk>::new();
    for triangle in &mesh.triangles {
        if actor_hides_triangle(actor, mesh, triangle) {
            continue;
        }
        let indices = triangle.indices;
        let mesh_node = triangle.mesh_node;
        let uvs = [
            mesh.texcoords.get(indices[0] as usize).copied().flatten(),
            mesh.texcoords.get(indices[1] as usize).copied().flatten(),
            mesh.texcoords.get(indices[2] as usize).copied().flatten(),
        ];
        let (key, color_factor) =
            gpu_triangle_material_factor(mesh, triangle.material, mesh_node, 1.0, 1.0);
        let chunk_index = if let Some(index) = chunks.iter().position(|chunk| chunk.key == key) {
            index
        } else {
            let texture = gpu_texture_for_material(mesh, triangle.material, key, fallback);
            let index = chunks.len();
            chunks.push(GpuWorldDrawChunk {
                key,
                texture,
                vertices: Vec::new(),
                mesh_node,
            });
            index
        };
        let chunk = chunks
            .get_mut(chunk_index)
            .expect("GPU world draw chunk inserted before vertex push");
        let indices = triangle.indices;
        let fallback_normal = triangle_normal(mesh, indices).unwrap_or([0.0, 0.0, 1.0]);
        for i in 0..3 {
            let vertex_index = indices[i] as usize;
            let Some(position) = mesh.positions.get(vertex_index).copied() else {
                continue;
            };
            let normal = mesh
                .normals
                .get(vertex_index)
                .copied()
                .flatten()
                .unwrap_or(fallback_normal);
            let joints = mesh
                .joints
                .get(vertex_index)
                .copied()
                .flatten()
                .map(|joints| {
                    [
                        joints[0] as f32,
                        joints[1] as f32,
                        joints[2] as f32,
                        joints[3] as f32,
                    ]
                })
                .unwrap_or([0.0, 0.0, 0.0, 0.0]);
            let weights = mesh
                .weights
                .get(vertex_index)
                .copied()
                .flatten()
                .unwrap_or([1.0, 0.0, 0.0, 0.0]);
            let vertex_color = mesh
                .colors
                .get(vertex_index)
                .copied()
                .flatten()
                .unwrap_or([1.0, 1.0, 1.0, 1.0]);
            let color = [
                color_factor[0] * vertex_color[0],
                color_factor[1] * vertex_color[1],
                color_factor[2] * vertex_color[2],
                color_factor[3] * vertex_color[3],
            ];
            chunk.vertices.push(GpuWorldVertex {
                position,
                normal,
                joints,
                weights,
                uv: uvs[i].unwrap_or([0.0, 0.0]),
                color,
            });
        }
    }
    let mut draws = Vec::<GpuWorldStaticDraw>::with_capacity(chunks.len());
    for chunk in chunks {
        if chunk.vertices.is_empty() {
            continue;
        }
        let resource_key = GpuWorldResourceKey {
            model_path: model_path.to_path_buf(),
            draw_key: chunk.key,
        };
        draws.push(GpuWorldStaticDraw {
            resource_key,
            vertices: Arc::new(chunk.vertices),
            texture: Arc::new(chunk.texture),
            mesh_node: chunk.mesh_node,
        });
    }
    draws
}

fn triangle_normal(mesh: &GlbMeshData, indices: [u32; 3]) -> Option<[f32; 3]> {
    let a = mesh.positions.get(indices[0] as usize).copied()?;
    let b = mesh.positions.get(indices[1] as usize).copied()?;
    let c = mesh.positions.get(indices[2] as usize).copied()?;
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    normalize_vec3([
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ])
}

fn normalize_vec3(value: [f32; 3]) -> Option<[f32; 3]> {
    let len = (value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt();
    if len <= f32::EPSILON || !len.is_finite() {
        None
    } else {
        Some([value[0] / len, value[1] / len, value[2] / len])
    }
}

fn actor_hides_triangle(actor: &WorldActor, mesh: &GlbMeshData, triangle: &GlbTriangle) -> bool {
    if actor.hide_materials.iter().any(|name| {
        triangle
            .material
            .and_then(|index| mesh.materials.get(index))
            .and_then(|material| material.name.as_deref())
            .is_some_and(|material_name| material_name.eq_ignore_ascii_case(name))
    }) {
        return true;
    }
    if actor.hide_meshes.iter().any(|name| {
        let mesh_name = triangle
            .mesh
            .and_then(|index| mesh.mesh_names.get(index))
            .and_then(|name| name.as_deref());
        let node_name = triangle
            .mesh_node
            .and_then(|index| mesh.nodes.get(index))
            .and_then(|node| node.name.as_deref());
        mesh_name.is_some_and(|mesh_name| mesh_name.eq_ignore_ascii_case(name))
            || node_name.is_some_and(|node_name| node_name.eq_ignore_ascii_case(name))
    }) {
        return true;
    }
    false
}

fn gpu_triangle_material_factor(
    mesh: &GlbMeshData,
    material_index: Option<usize>,
    mesh_node: Option<usize>,
    shade: f32,
    opacity: f32,
) -> (GpuWorldDrawKey, [f32; 4]) {
    if let Some(material) = material_index.and_then(|index| mesh.materials.get(index)) {
        if let Some(texture_index) = material.base_color_texture {
            if mesh
                .textures
                .get(texture_index)
                .and_then(Option::as_ref)
                .is_some()
            {
                return (
                    GpuWorldDrawKey {
                        material: material_index,
                        texture: Some(texture_index),
                        mesh_node,
                    },
                    [
                        material.base_color_factor[0].clamp(0.0, 1.0) * shade,
                        material.base_color_factor[1].clamp(0.0, 1.0) * shade,
                        material.base_color_factor[2].clamp(0.0, 1.0) * shade,
                        material.base_color_factor[3].clamp(0.0, 1.0) * opacity.clamp(0.0, 1.0),
                    ],
                );
            }
        }
    }

    (
        GpuWorldDrawKey {
            material: material_index,
            texture: None,
            mesh_node,
        },
        [shade, shade, shade, opacity.clamp(0.0, 1.0)],
    )
}

fn gpu_texture_for_material(
    mesh: &GlbMeshData,
    material_index: Option<usize>,
    key: GpuWorldDrawKey,
    fallback: [u8; 3],
) -> GpuWorldTexture {
    if let Some(texture_index) = key.texture {
        if let Some(texture) = mesh.textures.get(texture_index).and_then(Option::as_ref) {
            return GpuWorldTexture {
                width: texture.width,
                height: texture.height,
                rgba: texture.rgba.clone(),
            };
        }
    }

    let color = material_color(mesh, material_index, fallback, 1.0, 1.0);
    GpuWorldTexture {
        width: 1,
        height: 1,
        rgba: vec![color[0], color[1], color[2], color[3]],
    }
}

type TexturedTriangleSource<'a> = (&'a GlbTextureData, [f32; 4], [[f32; 2]; 3]);

fn textured_triangle_source(
    mesh: &GlbMeshData,
    material_index: Option<usize>,
    uvs: [Option<[f32; 2]>; 3],
) -> Option<TexturedTriangleSource<'_>> {
    let material = material_index.and_then(|index| mesh.materials.get(index))?;
    let texture_index = material.base_color_texture?;
    let texture = mesh.textures.get(texture_index)?.as_ref()?;
    let uvs = [uvs[0]?, uvs[1]?, uvs[2]?];
    Some((texture, material.base_color_factor, uvs))
}

fn sampled_texture_alpha(texture: &GlbTextureData, uv: [f32; 2], material_factor: [f32; 4]) -> f32 {
    if texture.width == 0 || texture.height == 0 || texture.rgba.len() < 4 {
        return 0.0;
    }
    let u = uv[0].clamp(0.0, 1.0);
    let v = uv[1].clamp(0.0, 1.0);
    let tx = (u * texture.width.saturating_sub(1) as f32)
        .round()
        .clamp(0.0, texture.width.saturating_sub(1) as f32) as u32;
    let ty = (v * texture.height.saturating_sub(1) as f32)
        .round()
        .clamp(0.0, texture.height.saturating_sub(1) as f32) as u32;
    let offset = ((ty * texture.width + tx) as usize).saturating_mul(4);
    texture
        .rgba
        .get(offset + 3)
        .map_or(0.0, |alpha| *alpha as f32 / 255.0)
        * material_factor[3].clamp(0.0, 1.0)
}

fn material_color(
    mesh: &GlbMeshData,
    material_index: Option<usize>,
    fallback: [u8; 3],
    shade: f32,
    opacity: f32,
) -> Rgba<u8> {
    let mut rgb = [
        fallback[0] as f32 / 255.0,
        fallback[1] as f32 / 255.0,
        fallback[2] as f32 / 255.0,
    ];
    let mut alpha = 220.0 / 255.0;
    if let Some(material) = material_index.and_then(|index| mesh.materials.get(index)) {
        let factor = material.base_color_factor;
        let has_visible_tint = (factor[0] - 1.0).abs() > 0.001
            || (factor[1] - 1.0).abs() > 0.001
            || (factor[2] - 1.0).abs() > 0.001;
        if has_visible_tint {
            rgb = [
                factor[0].clamp(0.0, 1.0),
                factor[1].clamp(0.0, 1.0),
                factor[2].clamp(0.0, 1.0),
            ];
        }
        alpha *= factor[3].clamp(0.0, 1.0);
    }
    Rgba([
        (rgb[0] * shade * 255.0).round().clamp(0.0, 255.0) as u8,
        (rgb[1] * shade * 255.0).round().clamp(0.0, 255.0) as u8,
        (rgb[2] * shade * 255.0).round().clamp(0.0, 255.0) as u8,
        (alpha * opacity.clamp(0.0, 1.0) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8,
    ])
}

#[allow(clippy::too_many_arguments)]
fn draw_actor_placeholder(
    canvas: &mut RgbaImage,
    actor: &WorldActor,
    has_skin: bool,
    world_x: f32,
    world_y: f32,
    view_yaw_deg: f32,
    camera_pitch_deg: f32,
    fov: f32,
    distance: f32,
    scale: f32,
    opacity: f32,
) {
    let width = canvas.width() as f32;
    let height = canvas.height() as f32;
    let px_per_world = (height / distance) * (35.0 / fov).clamp(0.35, 2.5);
    let cx = width * 0.5 + world_x * px_per_world;
    let ground_y = height * 0.78 - world_y * px_per_world;
    let body_h = (height * 0.34 * scale).clamp(32.0, height * 0.9);
    let head_r = body_h * 0.115;
    let yaw = view_yaw_deg.to_radians();
    let facing_width = yaw.cos().abs().mul_add(0.72, 0.28);
    let body_w = body_h * 0.20 * facing_width;
    let outline = Rgba([35, 45, 58, (255.0 * opacity) as u8]);
    let skin = if has_skin {
        Rgba([248, 229, 218, (235.0 * opacity) as u8])
    } else {
        Rgba([210, 218, 225, (235.0 * opacity) as u8])
    };
    let cloth = Rgba([94, 132, 190, (220.0 * opacity) as u8]);
    let shadow = Rgba([30, 45, 45, (70.0 * opacity) as u8]);

    fill_ellipse(
        canvas,
        cx,
        ground_y + body_h * 0.035,
        body_w * 1.5,
        body_h * 0.035,
        shadow,
    );
    fill_ellipse(
        canvas,
        cx,
        ground_y - body_h * 0.88,
        head_r * facing_width.max(0.55),
        head_r,
        outline,
    );
    fill_ellipse(
        canvas,
        cx,
        ground_y - body_h * 0.88,
        (head_r - 3.0).max(1.0) * facing_width.max(0.55),
        (head_r - 3.0).max(1.0),
        skin,
    );
    fill_ellipse(
        canvas,
        cx,
        ground_y - body_h * 0.48,
        body_w,
        body_h * 0.33,
        outline,
    );
    fill_ellipse(
        canvas,
        cx,
        ground_y - body_h * 0.48,
        (body_w - 3.0).max(1.0),
        (body_h * 0.33 - 3.0).max(1.0),
        cloth,
    );

    let nose_x = cx + yaw.sin() * head_r * 0.55 * facing_width.max(0.4);
    let nose_y = ground_y - body_h * 0.88 - camera_pitch_deg.to_radians().sin() * head_r * 0.3;
    draw_line(
        canvas,
        cx,
        ground_y - body_h * 0.88,
        nose_x,
        nose_y,
        outline,
        3.0,
    );

    let label_hint = if actor
        .play
        .as_ref()
        .and_then(|play| play.clip.as_ref())
        .is_some()
    {
        Rgba([255, 225, 120, (190.0 * opacity) as u8])
    } else {
        Rgba([180, 210, 255, (170.0 * opacity) as u8])
    };
    fill_ellipse(
        canvas,
        cx + body_w * 0.7,
        ground_y - body_h * 0.78,
        5.0,
        5.0,
        label_hint,
    );
}

fn composite_background(
    canvas: &mut RgbaImage,
    image: &RgbaImage,
    fit: &WorldBackgroundFit,
    opacity: f32,
) {
    let cw = canvas.width();
    let ch = canvas.height();
    let iw = image.width().max(1);
    let ih = image.height().max(1);
    let (scaled_w, scaled_h) = match fit {
        WorldBackgroundFit::Stretch => (cw, ch),
        WorldBackgroundFit::Contain => {
            let scale = (cw as f32 / iw as f32).min(ch as f32 / ih as f32);
            (
                (iw as f32 * scale).round() as u32,
                (ih as f32 * scale).round() as u32,
            )
        }
        WorldBackgroundFit::Cover => {
            let scale = (cw as f32 / iw as f32).max(ch as f32 / ih as f32);
            (
                (iw as f32 * scale).round() as u32,
                (ih as f32 * scale).round() as u32,
            )
        }
    };
    let scaled = imageops::resize(
        image,
        scaled_w.max(1),
        scaled_h.max(1),
        imageops::FilterType::Triangle,
    );
    let offset_x = ((cw as i64 - scaled.width() as i64) / 2)
        .min(0)
        .unsigned_abs() as u32;
    let offset_y = ((ch as i64 - scaled.height() as i64) / 2)
        .min(0)
        .unsigned_abs() as u32;
    let crop_w = cw.min(scaled.width().saturating_sub(offset_x));
    let crop_h = ch.min(scaled.height().saturating_sub(offset_y));
    let cropped =
        imageops::crop_imm(&scaled, offset_x, offset_y, crop_w.max(1), crop_h.max(1)).to_image();
    let paste_x = ((cw as i64 - cropped.width() as i64) / 2).max(0) as u32;
    let paste_y = ((ch as i64 - cropped.height() as i64) / 2).max(0) as u32;
    blend_image(canvas, &cropped, paste_x, paste_y, opacity);
}

fn blend_image(canvas: &mut RgbaImage, image: &RgbaImage, x: u32, y: u32, opacity: f32) {
    for iy in 0..image.height() {
        for ix in 0..image.width() {
            let dx = x + ix;
            let dy = y + iy;
            if dx >= canvas.width() || dy >= canvas.height() {
                continue;
            }
            let src = *image.get_pixel(ix, iy);
            blend_pixel(canvas, dx, dy, with_opacity(src, opacity));
        }
    }
}

fn blend_image_i32(canvas: &mut RgbaImage, image: &RgbaImage, x: i32, y: i32, opacity: f32) {
    for iy in 0..image.height() as i32 {
        for ix in 0..image.width() as i32 {
            let dx = x + ix;
            let dy = y + iy;
            if dx < 0 || dy < 0 {
                continue;
            }
            let dx = dx as u32;
            let dy = dy as u32;
            if dx >= canvas.width() || dy >= canvas.height() {
                continue;
            }
            let src = *image.get_pixel(ix as u32, iy as u32);
            blend_pixel(canvas, dx, dy, with_opacity(src, opacity));
        }
    }
}

fn fill(canvas: &mut RgbaImage, color: Rgba<u8>) {
    for pixel in canvas.pixels_mut() {
        *pixel = color;
    }
}

fn fill_triangle(canvas: &mut RgbaImage, points: [[f32; 2]; 3], color: Rgba<u8>) {
    let min_x = points
        .iter()
        .map(|point| point[0])
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as u32;
    let max_x = points
        .iter()
        .map(|point| point[0])
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(canvas.width() as f32 - 1.0)
        .max(0.0) as u32;
    let min_y = points
        .iter()
        .map(|point| point[1])
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as u32;
    let max_y = points
        .iter()
        .map(|point| point[1])
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(canvas.height() as f32 - 1.0)
        .max(0.0) as u32;
    let area = edge(points[0], points[1], points[2]);
    if area.abs() <= 0.0001 {
        return;
    }
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let point = [x as f32 + 0.5, y as f32 + 0.5];
            let w0 = edge(points[1], points[2], point);
            let w1 = edge(points[2], points[0], point);
            let w2 = edge(points[0], points[1], point);
            if (w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0) || (w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0) {
                blend_pixel(canvas, x, y, color);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_textured_triangle(
    canvas: &mut RgbaImage,
    points: [[f32; 2]; 3],
    uvs: [[f32; 2]; 3],
    texture_width: u32,
    texture_height: u32,
    texture_rgba: &[u8],
    material_factor: [f32; 4],
    shade: f32,
    opacity: f32,
) {
    if texture_width == 0 || texture_height == 0 || texture_rgba.len() < 4 {
        return;
    }
    let min_x = points
        .iter()
        .map(|point| point[0])
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as u32;
    let max_x = points
        .iter()
        .map(|point| point[0])
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(canvas.width() as f32 - 1.0)
        .max(0.0) as u32;
    let min_y = points
        .iter()
        .map(|point| point[1])
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as u32;
    let max_y = points
        .iter()
        .map(|point| point[1])
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(canvas.height() as f32 - 1.0)
        .max(0.0) as u32;
    let area = edge(points[0], points[1], points[2]);
    if area.abs() <= 0.0001 {
        return;
    }
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let point = [x as f32 + 0.5, y as f32 + 0.5];
            let e0 = edge(points[1], points[2], point);
            let e1 = edge(points[2], points[0], point);
            let e2 = edge(points[0], points[1], point);
            if !((e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0) || (e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0)) {
                continue;
            }
            let w0 = e0 / area;
            let w1 = e1 / area;
            let w2 = e2 / area;
            let u = (uvs[0][0] * w0 + uvs[1][0] * w1 + uvs[2][0] * w2).clamp(0.0, 1.0);
            let v = (uvs[0][1] * w0 + uvs[1][1] * w1 + uvs[2][1] * w2).clamp(0.0, 1.0);
            let tx = (u * texture_width.saturating_sub(1) as f32)
                .round()
                .clamp(0.0, texture_width.saturating_sub(1) as f32) as u32;
            let ty = (v * texture_height.saturating_sub(1) as f32)
                .round()
                .clamp(0.0, texture_height.saturating_sub(1) as f32) as u32;
            let offset = ((ty * texture_width + tx) as usize).saturating_mul(4);
            let Some(texel) = texture_rgba.get(offset..offset + 4) else {
                continue;
            };
            let alpha = texel[3] as f32 / 255.0 * material_factor[3].clamp(0.0, 1.0) * opacity;
            if alpha <= 0.0 {
                continue;
            }
            let color = Rgba([
                (texel[0] as f32 * material_factor[0].clamp(0.0, 1.0) * shade)
                    .round()
                    .clamp(0.0, 255.0) as u8,
                (texel[1] as f32 * material_factor[1].clamp(0.0, 1.0) * shade)
                    .round()
                    .clamp(0.0, 255.0) as u8,
                (texel[2] as f32 * material_factor[2].clamp(0.0, 1.0) * shade)
                    .round()
                    .clamp(0.0, 255.0) as u8,
                (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
            ]);
            blend_pixel(canvas, x, y, color);
        }
    }
}

fn edge(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (c[0] - a[0]) * (b[1] - a[1]) - (c[1] - a[1]) * (b[0] - a[0])
}

fn fill_ellipse(canvas: &mut RgbaImage, cx: f32, cy: f32, rx: f32, ry: f32, color: Rgba<u8>) {
    let rx = rx.max(0.5);
    let ry = ry.max(0.5);
    let min_x = (cx - rx).floor().max(0.0) as u32;
    let max_x = (cx + rx).ceil().min(canvas.width() as f32 - 1.0).max(0.0) as u32;
    let min_y = (cy - ry).floor().max(0.0) as u32;
    let max_y = (cy + ry).ceil().min(canvas.height() as f32 - 1.0).max(0.0) as u32;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let nx = (x as f32 + 0.5 - cx) / rx;
            let ny = (y as f32 + 0.5 - cy) / ry;
            if nx * nx + ny * ny <= 1.0 {
                blend_pixel(canvas, x, y, color);
            }
        }
    }
}

fn draw_line(
    canvas: &mut RgbaImage,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    color: Rgba<u8>,
    width: f32,
) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let steps = dx.abs().max(dy.abs()).ceil().max(1.0) as u32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = x0 + dx * t;
        let y = y0 + dy * t;
        fill_ellipse(canvas, x, y, width * 0.5, width * 0.5, color);
    }
}

fn blend_pixel(canvas: &mut RgbaImage, x: u32, y: u32, src: Rgba<u8>) {
    let dst = canvas.get_pixel_mut(x, y);
    let sa = src[3] as f32 / 255.0;
    if sa <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= f32::EPSILON {
        *dst = Rgba([0, 0, 0, 0]);
        return;
    }
    for i in 0..3 {
        let sc = src[i] as f32 / 255.0;
        let dc = dst[i] as f32 / 255.0;
        dst[i] = (((sc * sa + dc * da * (1.0 - sa)) / out_a) * 255.0).round() as u8;
    }
    dst[3] = (out_a * 255.0).round() as u8;
}

fn with_opacity(mut color: Rgba<u8>, opacity: f32) -> Rgba<u8> {
    color[3] = (color[3] as f32 * opacity.clamp(0.0, 1.0)).round() as u8;
    color
}

fn parse_rgba(raw: &str) -> Rgba<u8> {
    let text = raw.trim().trim_matches('"').trim();
    let Some(hex) = text.strip_prefix('#') else {
        return Rgba([0, 0, 0, 255]);
    };
    let parse = |range: std::ops::Range<usize>| {
        hex.get(range)
            .and_then(|part| u8::from_str_radix(part, 16).ok())
            .unwrap_or(0)
    };
    match hex.len() {
        6 => Rgba([parse(0..2), parse(2..4), parse(4..6), 255]),
        8 => Rgba([parse(0..2), parse(2..4), parse(4..6), parse(6..8)]),
        _ => Rgba([0, 0, 0, 255]),
    }
}

fn eval_number(expr: &str, default: f32, time: WorldTime) -> Result<f32, WorldRenderError> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Ok(default);
    }
    eval_time_expr(trimmed, time.time_norm(), time.time_sec()).map_err(|message| {
        WorldRenderError::Expression {
            expr: trimmed.to_string(),
            message,
        }
    })
}

fn resolve_asset_path_with_style(
    asset_root: &Path,
    src: &str,
    path_style: WorldPathStyle,
) -> PathBuf {
    let path = Path::new(src);
    match path_style {
        WorldPathStyle::Absolute => path.to_path_buf(),
        WorldPathStyle::Relative => {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                asset_root.join(path)
            }
        }
    }
}

/// Result of resolving a world asset source, used to load from either the
/// filesystem or an in-memory resolver (e.g. WASM `add_asset`).
enum ResolvedWorldAsset {
    Path(PathBuf),
    Bytes { key: PathBuf, bytes: Vec<u8> },
    Missing { key: PathBuf },
}

impl ResolvedWorldAsset {
    /// Cache key used to deduplicate loaded images. For filesystem assets this
    /// is the resolved path; for memory assets it is the original source name.
    fn key(&self) -> &Path {
        match self {
            ResolvedWorldAsset::Path(path) => path,
            ResolvedWorldAsset::Bytes { key, .. } => key,
            ResolvedWorldAsset::Missing { key } => key,
        }
    }
}

/// Resolve an asset source through the global resolver, falling back to the
/// legacy filesystem resolution when no resolver entry exists.
fn resolve_world_asset_source(
    asset_root: &Path,
    src: &str,
    path_style: WorldPathStyle,
    resolver: &dyn AssetResolver,
) -> Result<ResolvedWorldAsset, WorldRenderError> {
    match resolver.resolve(src) {
        Ok(AssetSource::Bytes(bytes)) => Ok(ResolvedWorldAsset::Bytes {
            key: PathBuf::from(src),
            bytes,
        }),
        Ok(AssetSource::Path(resolved_path)) => {
            // The resolver may return the raw source path (e.g. PathAssetResolver).
            // Resolve relative paths against the asset root and verify existence on
            // native platforms. On WASM there is no filesystem, so treat Path results
            // as missing and rely on memory assets instead.
            let path = if resolved_path.is_absolute() {
                resolved_path
            } else {
                resolve_asset_path_with_style(asset_root, src, path_style)
            };
            #[cfg(not(target_arch = "wasm32"))]
            {
                if path.exists() {
                    Ok(ResolvedWorldAsset::Path(path))
                } else {
                    Ok(ResolvedWorldAsset::Missing {
                        key: PathBuf::from(src),
                    })
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = path;
                Ok(ResolvedWorldAsset::Missing {
                    key: PathBuf::from(src),
                })
            }
        }
        Ok(AssetSource::Url(url)) => Err(WorldRenderError::Expression {
            expr: src.to_string(),
            message: format!("URL asset source is not supported for world assets: {url}"),
        }),
        Err(_) => {
            let path = resolve_asset_path_with_style(asset_root, src, path_style);
            if path.exists() {
                Ok(ResolvedWorldAsset::Path(path))
            } else {
                Ok(ResolvedWorldAsset::Missing {
                    key: PathBuf::from(src),
                })
            }
        }
    }
}

/// Load a dynamic image from a resolved world asset.
fn load_rgba_image_from_resolved(
    resolved: &ResolvedWorldAsset,
    error_ctor: impl Fn(PathBuf, image::ImageError) -> WorldRenderError,
) -> Result<image::DynamicImage, WorldRenderError> {
    match resolved {
        ResolvedWorldAsset::Path(path) => {
            image::open(path).map_err(|source| error_ctor(path.clone(), source))
        }
        ResolvedWorldAsset::Bytes { key, bytes } => {
            image::load_from_memory(bytes).map_err(|source| error_ctor(key.clone(), source))
        }
        ResolvedWorldAsset::Missing { key } => Err(error_ctor(
            key.clone(),
            image::ImageError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "asset not found",
            )),
        )),
    }
}

/// Load a GLB mesh from a resolved world asset. Returns the mesh together with a
/// stable cache key derived from the source resolution.
fn load_glb_mesh_resolved(
    asset_root: &Path,
    src: &str,
    path_style: WorldPathStyle,
    resolver: &dyn AssetResolver,
) -> Result<(PathBuf, GlbMeshData), WorldRenderError> {
    let resolved = resolve_world_asset_source(asset_root, src, path_style, resolver)?;
    let key = resolved.key().to_path_buf();
    let mesh = match resolved {
        ResolvedWorldAsset::Path(path) => load_glb_mesh_data(&path)?,
        ResolvedWorldAsset::Bytes { bytes, .. } => load_glb_mesh_data_from_bytes(&key, &bytes)?,
        ResolvedWorldAsset::Missing { .. } => {
            return Err(GlbLoadError::Io {
                path: key,
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "GLB asset not found"),
            }
            .into());
        }
    };
    Ok((key, mesh))
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use std::{fs, path::Path};

    use crate::world::{parse_world_graph_script, render_world_frame};

    #[test]
    fn world_camera_yaw_rotates_actor_world_position() {
        let front = super::camera_actor_view(1.0, 0.0, 0.0, 30.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        assert!((front.x - 1.0).abs() < 0.001);
        assert!(front.depth.abs() < 0.001);
        assert!((front.yaw - 30.0).abs() < 0.001);

        let side = super::camera_actor_view(0.0, 0.0, 1.0, 135.0, 0.0, 0.0, 0.0, 90.0, 0.0);
        assert!((side.x - 1.0).abs() < 0.001);
        assert!(side.depth.abs() < 0.001);
        assert!((side.yaw - 45.0).abs() < 0.001);
    }

    #[test]
    fn renders_world_placeholder_frame() {
        let script = r##"<Graph fps={30} duration="2s" size={[320,180]}>
  <World id="stage">
    <Background src="../scene/environments/forest_path_static.png" fit="cover" color="#87c9ff" />
    <Camera target="hero" yaw={curve("0:0:linear,2:360:linear")} distance="3" fov="35" />
    <Actor id="hero" model="../sample_assets/glb/mammuthus_primigenius_blumbach.glb" x="0" y="0" yaw="0" scale="0.001" />
  </World>
  <Present from="stage" />
</Graph>"##;
        let graph = parse_world_graph_script(script).expect("world graph");
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/motionloom/world");
        let model = root.join("../sample_assets/glb/mammuthus_primigenius_blumbach.glb");
        if !model.exists() {
            return;
        }
        let frame = pollster::block_on(render_world_frame(&graph, 0, &root)).expect("world frame");
        assert_eq!(frame.width(), 320);
        assert_eq!(frame.height(), 180);
    }

    #[test]
    fn renders_world_directional_character_by_yaw_and_pitch() {
        let root = std::env::temp_dir().join(format!(
            "motionloom_directional_character_test_{}",
            std::process::id()
        ));
        let character_dir = root.join("characters");
        fs::create_dir_all(&character_dir).expect("test character dir");
        let sheet_path = character_dir.join("hero_sheet.png");
        let mut sheet = image::RgbaImage::from_pixel(30, 10, image::Rgba([0, 0, 0, 0]));
        for y in 0..10 {
            for x in 0..10 {
                sheet.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
                sheet.put_pixel(x + 10, y, image::Rgba([0, 255, 0, 255]));
                sheet.put_pixel(x + 20, y, image::Rgba([0, 0, 255, 255]));
            }
        }
        sheet.save(&sheet_path).expect("test sheet png");

        let script = r##"<Graph fps={30} duration="1s" size={[40,20]}>
  <World id="sprite_stage">
    <Background color="#000000" />
    <Camera yaw="0" pitch="0" zoom="1" />
    <DirectionalCharacter id="hero" sheet="characters/hero_sheet.png" x="10" y="10" scale="1" yaw="90">
      <DirectionMap>
        <Direction angle="0" rect={[0,0,10,10]} anchor={[0,0]} />
        <Direction angle="90" rect={[10,0,10,10]} anchor={[0,0]} />
        <Direction name="top" cameraPitch="90" rect={[20,0,10,10]} anchor={[0,0]} />
      </DirectionMap>
    </DirectionalCharacter>
  </World>
  <Present from="sprite_stage" />
</Graph>"##;
        let graph = parse_world_graph_script(script).expect("directional graph");
        let frame =
            pollster::block_on(render_world_frame(&graph, 0, &root)).expect("directional frame");
        assert_eq!(frame.get_pixel(10, 10).0, [0, 255, 0, 255]);

        let top_script = script.replace("pitch=\"0\"", "pitch=\"90\"");
        let graph = parse_world_graph_script(&top_script).expect("top directional graph");
        let frame = pollster::block_on(render_world_frame(&graph, 0, &root))
            .expect("top directional frame");
        assert_eq!(frame.get_pixel(10, 10).0, [0, 0, 255, 255]);

        let scaled_script = script.replace("size={[40,20]}", "size={[40,20]} renderSize={[20,10]}");
        let graph = parse_world_graph_script(&scaled_script).expect("scaled directional graph");
        let frame = pollster::block_on(render_world_frame(&graph, 0, &root))
            .expect("scaled directional frame");
        assert_eq!(frame.width(), 20);
        assert_eq!(frame.height(), 10);
        assert_eq!(frame.get_pixel(5, 5).0, [0, 255, 0, 255]);
    }

    #[test]
    fn renders_directional_character_play_sprite_frames() {
        let root = std::env::temp_dir().join(format!(
            "motionloom_play_sprite_test_{}",
            std::process::id()
        ));
        let character_dir = root.join("characters");
        fs::create_dir_all(&character_dir).expect("test character dir");
        let sheet_path = character_dir.join("runner.png");
        let mut sheet = image::RgbaImage::from_pixel(30, 10, image::Rgba([0, 0, 0, 0]));
        for y in 0..10 {
            for x in 0..10 {
                sheet.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
                sheet.put_pixel(x + 10, y, image::Rgba([0, 255, 0, 255]));
                sheet.put_pixel(x + 20, y, image::Rgba([0, 0, 255, 255]));
            }
        }
        sheet.save(&sheet_path).expect("test play sprite png");

        let script = r##"<Graph fps={1} duration="3s" size={[20,20]}>
  <World id="sprite_stage">
    <Background color="#000000" />
    <Camera yaw="0" pitch="0" zoom="1" />
    <DirectionalCharacter id="hero" sheet="characters/runner.png" x="0" y="0" scale="1" yaw="0">
      <PlaySprite fps="1" loop="true" frameSize={[10,10]} columns="3" frames="3" />
      <DirectionMap>
        <Direction angle="0" rect={[0,0,10,10]} anchor={[0,0]} />
      </DirectionMap>
    </DirectionalCharacter>
  </World>
  <Present from="sprite_stage" />
</Graph>"##;
        let graph = parse_world_graph_script(script).expect("play sprite graph");
        let frame0 =
            pollster::block_on(render_world_frame(&graph, 0, &root)).expect("play sprite frame 0");
        let frame1 =
            pollster::block_on(render_world_frame(&graph, 1, &root)).expect("play sprite frame 1");
        let frame2 =
            pollster::block_on(render_world_frame(&graph, 2, &root)).expect("play sprite frame 2");

        assert_eq!(frame0.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(frame1.get_pixel(0, 0).0, [0, 255, 0, 255]);
        assert_eq!(frame2.get_pixel(0, 0).0, [0, 0, 255, 255]);
    }

    #[test]
    fn renders_split_directional_character_png_with_alpha() {
        let root = std::env::temp_dir().join(format!(
            "motionloom_directional_character_split_test_{}",
            std::process::id()
        ));
        let character_dir = root.join("characters");
        fs::create_dir_all(&character_dir).expect("test character dir");
        let image_path = character_dir.join("hero_front.png");
        let frame = image::RgbaImage::from_pixel(10, 10, image::Rgba([0, 255, 0, 128]));
        frame.save(&image_path).expect("test direction png");

        let script = r##"<Graph fps={30} duration="1s" size={[40,20]} renderSize={[20,10]}>
  <World id="sprite_stage">
    <Background color="#000000" />
    <Camera yaw="0" pitch="0" zoom="1" />
    <DirectionalCharacter id="hero" pathstyle="relative" x="10" y="10" scale="1" yaw="0">
      <DirectionMap>
        <Direction angle="0" image="characters/hero_front.png" anchor={[0,0]} />
      </DirectionMap>
    </DirectionalCharacter>
  </World>
  <Present from="sprite_stage" />
</Graph>"##;
        let graph = parse_world_graph_script(script).expect("split directional graph");
        let rendered = pollster::block_on(render_world_frame(&graph, 0, &root))
            .expect("split directional frame");
        let pixel = rendered.get_pixel(5, 5).0;
        assert_eq!(pixel[0], 0);
        assert!(
            (120..=136).contains(&pixel[1]),
            "expected alpha-blended green, got {pixel:?}"
        );
        assert_eq!(pixel[2], 0);
        assert_eq!(pixel[3], 255);
    }
}
