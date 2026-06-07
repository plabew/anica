// =========================================
// crates/motionloom/src/lib.rs

pub mod common;
pub mod dsl;
pub mod error;
pub mod process;
pub mod root;
pub mod scene;
pub mod world;

// Compatibility module aliases. Public API names stay stable while the source
// tree is split by product domain.
pub use common::backend;
pub use common::curve as eval;
pub use common::keyframe;
pub use process::adapters::clip::export_adapter;
pub use process::adapters::clip::model;
pub use process::adapters::clip::preview_adapter;
pub use process::adapters::clip::transitions;
pub use process::effect as effects;
pub use process::graph;
pub use process::pass as effect_kernel_map;
pub use process::process_catalog;
pub use process::runtime;
pub use root as graph_root;
pub use scene::model as scene_model;
pub use scene::render as scene_render;

pub use common::keyframe::ScalarKeyframe;
pub use dsl::{
    ActionBoneNode, ActionNode, ActionPoseNode, ApplyActionNode, BackgroundNode, GraphScript,
    ImageNode, ModelProfileBoneAxisMapNode, ModelProfileBoneAxisNode, ModelProfileNode,
    ModelProfileRetargetMapNode, ModelProfileRetargetNode, SkeletonBoneNode, SkeletonNode, SvgNode,
    is_graph_script, parse_graph_script,
};
pub use error::{GraphParseError, MotionLoomError, RootGraphError, RuntimeCompileError};
pub use eval::sample_anim_f32;
pub use process::adapters::clip::model::{
    AnimF32, ClipZoomSpec, ColorRgba, LayerEffectClip, LocalMaskLayer, MAX_LOCAL_MASK_LAYERS,
    SlideDirection, TextStyle, VideoEffect, ZoomStyle,
};
pub use process::effect::{
    LayerColorBlurEffects, PerClipColorBlurEffects, combine_clip_with_layer,
};
pub use process::error::{
    MotionLoomProcessError, ProcessError, ProcessGraphError, ProcessParseError, ProcessRuntimeError,
};
pub use process::model::{
    AlphaMode, BlendMode, BufferElemType, BufferNode, BufferUsage, ColorSpace, EffectNode,
    GraphApplyScope, InputNode, InputType, LayerNode, LoadOp, OutputNode, OutputTarget, PassCache,
    PassKind, PassNode, PassParam, PassRole, PassTransitionClips, PassTransitionEasing,
    PassTransitionFallback, PassTransitionMode, PresentNode, PresentTarget, Quality, ResourceRef,
    SampleAddress, SampleConfig, SampleFilter, StoreOp, TexNode, TexUsage, TextureFormat,
};
pub use process::parser::{ProcessGraph, is_process_graph_script, parse_process_graph_script};
pub use process::pass::{default_kernel_for_effect, resolve_pass_kernel};
pub use process::process_catalog::{
    PROCESS_CATEGORIES, PROCESS_EFFECTS, ProcessCategory, ProcessEffectDefinition,
    is_known_process_kernel, kernel_source_by_name, process_effect_for_id, process_effects,
    process_effects_for_category,
};
pub use process::runtime::{
    BlurSharpenMode, RuntimeFrameOutput, RuntimeProgram, compile_runtime_program, eval_time_expr,
};
pub use root::{
    MotionLoomDocument, MotionLoomRenderProgress, RootGraphDomain, RootGraphShell,
    inspect_root_graph, parse_motionloom_document,
    render_motionloom_document_to_video_with_progress,
};
pub use scene::{
    BrushDef, CameraNode, CharacterNode, CircleNode, ComponentNode, DefsNode, FaceJawNode,
    FilterDef, FilterStepDef, FontDef, GradientDef, GradientStop, GroupNode, LineNode,
    LinearGradientDef, MaskNode, PaletteColorDef, PaletteNode, PartNode, PathNode, PixelGridNode,
    PolylineNode, PrecomposeNode, RadialGradientDef, RectNode, RepeatNode, SceneChainNode,
    SceneLayerNode, SceneNode, SceneRootNode, SceneSequenceNode, SceneTimelineNode, SceneTrackNode,
    ShadowNode, TextAlignMode, TextAnimatorNode, TextEffectNode, TextGlowEffectNode,
    TextLayoutNode, TextNode, TextOverflowMode, TextSelectorKind, TextStyleOverrideNode,
    TextTransformNode, TextWrapMode, UseNode,
};
pub use scene_render::{
    MotionLoomSceneRenderError, SceneRenderError, SceneRenderProfile, SceneRenderProgress,
    SceneRenderer, clear_scene_asset_roots, next_scene_output_path,
    next_scene_output_path_for_profile, render_scene_frame, render_scene_graph_frame,
    render_scene_graph_to_video, render_scene_graph_to_video_with_progress, set_scene_asset_roots,
};
pub use world::error::{MotionLoomWorldError, WorldAssetError, WorldError, WorldParseError};
pub use world::{
    CharacterDesignGpuViewport, CharacterDesignViewportFrame, GlbLoadError, GlbMeshData,
    GlbMetadata, WorldAction, WorldActionBone, WorldActionPose, WorldActor, WorldApplyAction,
    WorldBackground, WorldBackgroundFit, WorldBoneAxis, WorldBoneAxisMap, WorldCamera,
    WorldCameraControl, WorldCameraMode, WorldCameraProjection, WorldFrameRenderer,
    WorldGpuDiagnostics, WorldGraph, WorldMaterial, WorldMaterialStyle, WorldModelProfile,
    WorldNode, WorldPathStyle, WorldPlay, WorldPresent, WorldProfileRetarget, WorldRenderError,
    WorldRenderProgress, WorldRetarget, WorldRetargetMap, WorldSpritePlayback, WorldTime,
    diagnose_world_glb_gpu_plan, diagnose_world_graph_actor_gpu_frame, is_world_graph_script,
    load_glb_mesh_data, load_glb_metadata, parse_glb_mesh_data, parse_glb_metadata,
    parse_world_graph_script, render_world_frame, render_world_graph_to_video_with_progress,
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
