// =========================================
// crates/motionloom/src/lib.rs

pub mod animation;
pub mod clip;
pub mod common;
pub mod dsl;
pub mod scene;

// Compatibility module aliases. Public API names stay stable while the source
// tree is split by product domain.
pub use clip::export_adapter;
pub use clip::model;
pub use clip::preview_adapter;
pub use clip::transitions;
pub use common::backend;
pub use common::curve as eval;
pub use common::effect as effects;
pub use common::error;
pub use common::graph;
pub use common::keyframe;
pub use common::pass as effect_kernel_map;
pub use common::process_catalog;
pub use common::runtime;
pub use scene::model as scene_model;
pub use scene::render as scene_render;

pub use animation::{
    AnimationAction, AnimationActionBone, AnimationActionPose, AnimationActor,
    AnimationApplyAction, AnimationBackground, AnimationBackgroundFit, AnimationBoneAxis,
    AnimationBoneAxisMap, AnimationCamera, AnimationCameraMode, AnimationCameraProjection,
    AnimationFrameRenderer, AnimationGpuDiagnostics, AnimationGraph, AnimationMaterial,
    AnimationMaterialStyle, AnimationModelProfile, AnimationPathStyle, AnimationPlay,
    AnimationPresent, AnimationProfileRetarget, AnimationRenderError, AnimationRenderProgress,
    AnimationRetarget, AnimationRetargetMap, AnimationSpritePlayback, AnimationTime,
    AnimationWorld, CharacterDesignGpuViewport, CharacterDesignViewportFrame, GlbLoadError,
    GlbMeshData, GlbMetadata, diagnose_animation_glb_gpu_plan,
    diagnose_animation_graph_actor_gpu_frame, is_animation_graph_script, load_glb_mesh_data,
    load_glb_metadata, parse_animation_graph_script, parse_glb_mesh_data, parse_glb_metadata,
    render_animation_frame, render_animation_graph_to_video_with_progress,
};
pub use common::effect::{LayerColorBlurEffects, PerClipColorBlurEffects, combine_clip_with_layer};
pub use common::error::{GraphParseError, RuntimeCompileError};
pub use common::keyframe::ScalarKeyframe;
pub use common::pass::{default_kernel_for_effect, resolve_pass_kernel};
pub use common::process_catalog::{
    PROCESS_CATEGORIES, PROCESS_EFFECTS, ProcessCategory, ProcessEffectDefinition,
    is_known_process_kernel, kernel_source_by_name, process_effect_for_id, process_effects,
    process_effects_for_category,
};
pub use common::runtime::{
    BlurSharpenMode, RuntimeFrameOutput, RuntimeProgram, compile_runtime_program, eval_time_expr,
};
pub use dsl::{
    ActionBoneNode, ActionNode, ActionPoseNode, AlphaMode, ApplyActionNode, BackgroundNode,
    BlendMode, BufferElemType, BufferNode, BufferUsage, ColorSpace, EffectNode, GraphApplyScope,
    GraphScript, ImageNode, InputNode, InputType, LayerNode, LoadOp, ModelProfileBoneAxisMapNode,
    ModelProfileBoneAxisNode, ModelProfileNode, ModelProfileRetargetMapNode,
    ModelProfileRetargetNode, OutputNode, OutputTarget, PassCache, PassKind, PassNode, PassParam,
    PassRole, PassTransitionClips, PassTransitionEasing, PassTransitionFallback,
    PassTransitionMode, PresentNode, PresentTarget, Quality, ResourceRef, SampleAddress,
    SampleConfig, SampleFilter, SkeletonBoneNode, SkeletonNode, StoreOp, SvgNode, TexNode,
    TexUsage, TextNode, TextureFormat, is_graph_script, parse_graph_script,
};
pub use eval::sample_anim_f32;
pub use model::{
    AnimF32, ClipZoomSpec, ColorRgba, LayerEffectClip, LocalMaskLayer, MAX_LOCAL_MASK_LAYERS,
    SlideDirection, TextStyle, VideoEffect, ZoomStyle,
};
pub use scene::{
    BrushDef, CameraNode, CharacterNode, CircleNode, DefsNode, FaceJawNode, GradientDef,
    GradientStop, GroupNode, LineNode, LinearGradientDef, MaskNode, PaletteColorDef, PaletteNode,
    PartNode, PathNode, PixelGridNode, PolylineNode, PrecomposeNode, RadialGradientDef, RectNode,
    RepeatNode, SceneLayerNode, SceneNode, SceneRootNode, ShadowNode,
};
pub use scene_render::{
    MotionLoomSceneRenderError, SceneRenderError, SceneRenderProfile, SceneRenderProgress,
    SceneRenderer, next_scene_output_path, next_scene_output_path_for_profile, render_scene_frame,
    render_scene_graph_frame, render_scene_graph_to_video,
    render_scene_graph_to_video_with_progress,
};

/// FrameContext carries the minimum timeline state needed for effect evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameContext {
    pub time_ms: u64,
    pub fps: f32,
}

impl FrameContext {
    /// Build a frame context from timeline time in milliseconds.
    pub const fn new(time_ms: u64, fps: f32) -> Self {
        Self { time_ms, fps }
    }

    /// Convert the timeline position to seconds for curve math.
    pub fn time_seconds(self) -> f32 {
        self.time_ms as f32 / 1000.0
    }
}

/// Lerp is the base interpolation primitive used by keyframe curves.
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// Sample a simple linear segment between two keyed values in milliseconds.
pub fn sample_linear_segment(
    now_ms: u64,
    start_ms: u64,
    end_ms: u64,
    start_value: f32,
    end_value: f32,
) -> f32 {
    if end_ms <= start_ms {
        return end_value;
    }
    let span = (end_ms - start_ms) as f32;
    let elapsed = now_ms.saturating_sub(start_ms) as f32;
    lerp(start_value, end_value, elapsed / span)
}

#[cfg(test)]
mod tests {
    use super::{FrameContext, lerp, sample_linear_segment};

    #[test]
    fn context_seconds_conversion() {
        let ctx = FrameContext::new(1500, 30.0);
        assert!((ctx.time_seconds() - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn lerp_clamps_input_t() {
        assert!((lerp(0.0, 10.0, -1.0) - 0.0).abs() < f32::EPSILON);
        assert!((lerp(0.0, 10.0, 2.0) - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn linear_segment_samples_midpoint() {
        let value = sample_linear_segment(500, 0, 1000, 0.0, 1.0);
        assert!((value - 0.5).abs() < 0.001);
    }
}
