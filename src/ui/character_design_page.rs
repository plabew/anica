use std::collections::{HashMap, HashSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use gpui::{
    App, ClipboardItem, Context, Element, Entity, FocusHandle, Focusable, GlobalElementId,
    InspectorElementId, IntoElement, KeyDownEvent, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathPromptOptions, Render, RenderImage, ScrollWheelEvent, Style,
    Subscription, Timer, Window, div, prelude::*, px, relative, rgb, rgba,
};
use gpui_component::{
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    white,
};
use image::{ImageBuffer, Rgba};
use motionloom::{
    CharacterDesignGpuViewport, WorldGpuDiagnostics, WorldGraph, diagnose_world_glb_gpu_plan,
    load_glb_mesh_data, parse_world_graph_script,
};
use smallvec::SmallVec;

use crate::core::global_state::GlobalState;

const DEFAULT_MODEL: &str = "";
const DEFAULT_FPS: u32 = 30;
const DEFAULT_DURATION_SEC: f32 = 2.0;
const DEFAULT_GRAPH_WIDTH: u32 = 1920;
const DEFAULT_GRAPH_HEIGHT: u32 = 1080;
const DEFAULT_RENDER_WIDTH: u32 = 1920;
const DEFAULT_RENDER_HEIGHT: u32 = 1080;
const MAX_PREVIEW_PIXELS: u32 = 540 * 720;
const MAX_UNDO_STEPS: usize = 80;

type PreviewKey = (u64, u32, usize);

struct CharacterPreviewRequest {
    script: String,
    frame: u32,
    actor_id: String,
    asset_root: PathBuf,
    response_tx: Sender<Result<CharacterPreviewResponse, String>>,
}

struct CharacterPreviewResponse {
    width: u32,
    height: u32,
    bgra: Vec<u8>,
    diagnostics: Option<WorldGpuDiagnostics>,
}

#[derive(Clone, Debug, PartialEq)]
struct RawBoneInfo {
    index: usize,
    name: String,
    position: [f32; 3],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RetargetBoneAxis {
    turn: Option<String>,
    bend: Option<String>,
    forward: Option<String>,
    side: Option<String>,
    twist: Option<String>,
    rest_turn: Option<String>,
    rest_bend: Option<String>,
    rest_forward: Option<String>,
    rest_side: Option<String>,
    rest_twist: Option<String>,
}

#[derive(Clone, Debug)]
struct LoadedModelInspection {
    retarget_maps: Vec<(String, String)>,
    raw_bones: Vec<RawBoneInfo>,
    mesh_names: Vec<String>,
    material_names: Vec<String>,
    hidden_meshes: HashSet<String>,
    hidden_materials: HashSet<String>,
    diagnostics: Option<WorldGpuDiagnostics>,
    bounds_height: f32,
    source_rest_pose: SourceRestPose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RotationAxis {
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AxisBinding {
    axis: RotationAxis,
    sign: f32,
}

impl AxisBinding {
    fn new(axis: RotationAxis, sign: f32) -> Self {
        Self { axis, sign }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct CharacterProfileControls {
    upper_arm_r_forward: AxisBinding,
    upper_arm_r_side: AxisBinding,
    upper_arm_r_twist: AxisBinding,
    forearm_r_bend: AxisBinding,
    forearm_r_twist: AxisBinding,
    hand_r_twist: AxisBinding,
    forearm_r_rest_bend: f32,
}

impl Default for CharacterProfileControls {
    fn default() -> Self {
        // Calibrated draft generated from the current humanoid GLB. Keep this
        // editable so the page can export a model-specific profile.
        Self {
            upper_arm_r_forward: AxisBinding::new(RotationAxis::Z, -1.0),
            upper_arm_r_side: AxisBinding::new(RotationAxis::X, 1.0),
            upper_arm_r_twist: AxisBinding::new(RotationAxis::Y, 1.0),
            forearm_r_bend: AxisBinding::new(RotationAxis::X, -1.0),
            forearm_r_twist: AxisBinding::new(RotationAxis::Y, 1.0),
            hand_r_twist: AxisBinding::new(RotationAxis::Y, -1.0),
            forearm_r_rest_bend: 10.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct WaveActionControls {
    raise_forward: f32,
    raise_side: f32,
    elbow_bend: f32,
    wave_twist: f32,
    left_leg_forward: f32,
    left_knee_bend: f32,
    right_leg_forward: f32,
    right_knee_bend: f32,
    chest_turn: f32,
    head_turn: f32,
}

impl Default for WaveActionControls {
    fn default() -> Self {
        Self {
            raise_forward: 0.0,
            raise_side: 0.0,
            elbow_bend: 8.0,
            wave_twist: 0.0,
            left_leg_forward: 0.0,
            left_knee_bend: 0.0,
            right_leg_forward: 0.0,
            right_knee_bend: 0.0,
            chest_turn: 0.0,
            head_turn: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CharacterDragHandle {
    RightHand,
    RightElbow,
    LeftFoot,
    RightFoot,
    Head,
    Chest,
    RawBone,
}

impl CharacterDragHandle {
    fn label(self) -> &'static str {
        match self {
            Self::RightHand => "hand_r",
            Self::RightElbow => "forearm_r",
            Self::LeftFoot => "foot_l",
            Self::RightFoot => "foot_r",
            Self::Head => "head",
            Self::Chest => "chest",
            Self::RawBone => "Raw bone",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetargetPosePreset {
    Original,
    ArmsDown,
    APose,
    TPose,
    Walk,
    Jump,
    WaveHand,
    SidePlus20,
    SideMinus20,
    ForwardPlus20,
    BendPlus20,
    TwistPlus20,
}

impl RetargetPosePreset {
    fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::ArmsDown => "Arms Down",
            Self::APose => "A-Pose",
            Self::TPose => "T-Pose",
            Self::Walk => "Walk",
            Self::Jump => "Jump",
            Self::WaveHand => "Wave Hand",
            Self::SidePlus20 => "Side +20",
            Self::SideMinus20 => "Side -20",
            Self::ForwardPlus20 => "Forward +20",
            Self::BendPlus20 => "Bend +20",
            Self::TwistPlus20 => "Twist +20",
        }
    }

    fn action_duration(self) -> &'static str {
        match self {
            Self::Walk => "1s",
            Self::Jump => "1.6s",
            Self::WaveHand => "2s",
            _ => "2s",
        }
    }

    fn preview_frame(self) -> u32 {
        match self {
            Self::Walk => 0,
            Self::Jump => 24,
            Self::WaveHand => 24,
            _ => 0,
        }
    }

    fn action_poses(self) -> Option<&'static str> {
        match self {
            Self::Original => None,
            Self::ArmsDown => Some(
                r#"    <Pose t="0.0" label="Arms Down">
      <Bone id="hips" turn="0" />
      <Bone id="spine" turn="0" bend="0" />
      <Bone id="chest" turn="0" bend="0" />
      <Bone id="head" turn="0" />
      <Bone id="upper_arm_l" forward="0" side="0" twist="0" />
      <Bone id="forearm_l" bend="0" twist="0" />
      <Bone id="hand_l" twist="0" />
      <Bone id="upper_arm_r" forward="0" side="0" twist="0" />
      <Bone id="forearm_r" bend="0" twist="0" />
      <Bone id="hand_r" twist="0" />
      <Bone id="upper_leg_l" forward="0" side="0" />
      <Bone id="lower_leg_l" bend="0" />
      <Bone id="upper_leg_r" forward="0" side="0" />
      <Bone id="lower_leg_r" bend="0" />
    </Pose>"#,
            ),
            Self::APose => Some(
                r#"    <Pose t="0.0" label="A-Pose">
      <Bone id="hips" turn="0" />
      <Bone id="spine" turn="0" bend="0" />
      <Bone id="chest" turn="0" bend="0" />
      <Bone id="head" turn="0" />
      <Bone id="upper_arm_l" forward="0" side="35" twist="0" />
      <Bone id="forearm_l" bend="0" twist="0" />
      <Bone id="hand_l" twist="0" />
      <Bone id="upper_arm_r" forward="0" side="35" twist="0" />
      <Bone id="forearm_r" bend="0" twist="0" />
      <Bone id="hand_r" twist="0" />
    </Pose>"#,
            ),
            Self::TPose => Some(
                r#"    <Pose t="0.0" label="T-Pose">
      <Bone id="hips" turn="0" />
      <Bone id="spine" turn="0" bend="0" />
      <Bone id="chest" turn="0" bend="0" />
      <Bone id="head" turn="0" />
      <Bone id="upper_arm_l" forward="0" side="90" twist="0" />
      <Bone id="forearm_l" bend="0" twist="0" />
      <Bone id="hand_l" twist="0" />
      <Bone id="upper_arm_r" forward="0" side="90" twist="0" />
      <Bone id="forearm_r" bend="0" twist="0" />
      <Bone id="hand_r" twist="0" />
    </Pose>"#,
            ),
            Self::Walk => Some(
                r#"    <Pose t="0.0" label="walk_left">
      <Bone id="upper_leg_l" forward="25" />
      <Bone id="lower_leg_l" bend="35" />
      <Bone id="upper_leg_r" forward="-25" />
      <Bone id="lower_leg_r" bend="-15" />
    </Pose>

    <Pose t="0.5" label="walk_right">
      <Bone id="upper_leg_l" forward="-25" />
      <Bone id="lower_leg_l" bend="-15" />
      <Bone id="upper_leg_r" forward="25" />
      <Bone id="lower_leg_r" bend="35" />
    </Pose>

    <Pose t="1.0" label="walk_loop">
      <Bone id="upper_leg_l" forward="25" />
      <Bone id="lower_leg_l" bend="35" />
      <Bone id="upper_leg_r" forward="-25" />
      <Bone id="lower_leg_r" bend="-15" />
    </Pose>"#,
            ),
            Self::Jump => Some(
                r#"    <Pose t="0.0" label="jump_start">
      <Bone id="hips" y="0" rotationZ="0" />
      <Bone id="spine" rotationZ="0" />
      <Bone id="chest" rotationZ="0" />
      <Bone id="head" rotationZ="0" />
      <Bone id="upper_leg_l" rotationZ="0" />
      <Bone id="lower_leg_l" rotationZ="0" />
      <Bone id="foot_l" rotationZ="0" />
      <Bone id="upper_leg_r" rotationZ="0" />
      <Bone id="lower_leg_r" rotationZ="0" />
      <Bone id="foot_r" rotationZ="0" />
      <Bone id="upper_arm_l" rotationZ="-8" />
      <Bone id="forearm_l" rotationZ="6" />
      <Bone id="upper_arm_r" rotationZ="8" />
      <Bone id="forearm_r" rotationZ="-6" />
    </Pose>

    <Pose t="0.22" label="jump_crouch">
      <Bone id="hips" y="-0.08" rotationZ="0" />
      <Bone id="spine" rotationZ="4" />
      <Bone id="chest" rotationZ="5" />
      <Bone id="head" rotationZ="-3" />
      <Bone id="upper_leg_l" rotationZ="18" />
      <Bone id="lower_leg_l" rotationZ="-48" />
      <Bone id="foot_l" rotationZ="16" />
      <Bone id="upper_leg_r" rotationZ="-18" />
      <Bone id="lower_leg_r" rotationZ="48" />
      <Bone id="foot_r" rotationZ="-16" />
      <Bone id="upper_arm_l" rotationZ="42" />
      <Bone id="forearm_l" rotationZ="-28" />
      <Bone id="upper_arm_r" rotationZ="-42" />
      <Bone id="forearm_r" rotationZ="28" />
    </Pose>

    <Pose t="0.45" label="jump_takeoff">
      <Bone id="hips" y="0.24" rotationZ="0" />
      <Bone id="spine" rotationZ="-2" />
      <Bone id="chest" rotationZ="-3" />
      <Bone id="head" rotationZ="2" />
      <Bone id="upper_leg_l" rotationZ="-8" />
      <Bone id="lower_leg_l" rotationZ="14" />
      <Bone id="foot_l" rotationZ="-8" />
      <Bone id="upper_leg_r" rotationZ="8" />
      <Bone id="lower_leg_r" rotationZ="-14" />
      <Bone id="foot_r" rotationZ="8" />
      <Bone id="upper_arm_l" rotationZ="-58" />
      <Bone id="forearm_l" rotationZ="28" />
      <Bone id="upper_arm_r" rotationZ="58" />
      <Bone id="forearm_r" rotationZ="-28" />
    </Pose>

    <Pose t="0.80" label="jump_air">
      <Bone id="hips" y="0.34" rotationZ="0" />
      <Bone id="spine" rotationZ="0" />
      <Bone id="chest" rotationZ="0" />
      <Bone id="head" rotationZ="0" />
      <Bone id="upper_leg_l" rotationZ="-14" />
      <Bone id="lower_leg_l" rotationZ="24" />
      <Bone id="foot_l" rotationZ="-10" />
      <Bone id="upper_leg_r" rotationZ="14" />
      <Bone id="lower_leg_r" rotationZ="-24" />
      <Bone id="foot_r" rotationZ="10" />
      <Bone id="upper_arm_l" rotationZ="-68" />
      <Bone id="forearm_l" rotationZ="35" />
      <Bone id="upper_arm_r" rotationZ="68" />
      <Bone id="forearm_r" rotationZ="-35" />
    </Pose>

    <Pose t="1.08" label="jump_land">
      <Bone id="hips" y="0.08" rotationZ="0" />
      <Bone id="spine" rotationZ="3" />
      <Bone id="chest" rotationZ="4" />
      <Bone id="head" rotationZ="-2" />
      <Bone id="upper_leg_l" rotationZ="12" />
      <Bone id="lower_leg_l" rotationZ="-34" />
      <Bone id="foot_l" rotationZ="12" />
      <Bone id="upper_leg_r" rotationZ="-12" />
      <Bone id="lower_leg_r" rotationZ="34" />
      <Bone id="foot_r" rotationZ="-12" />
      <Bone id="upper_arm_l" rotationZ="-22" />
      <Bone id="forearm_l" rotationZ="12" />
      <Bone id="upper_arm_r" rotationZ="22" />
      <Bone id="forearm_r" rotationZ="-12" />
    </Pose>

    <Pose t="1.28" label="jump_recover">
      <Bone id="hips" y="-0.04" rotationZ="0" />
      <Bone id="spine" rotationZ="-2" />
      <Bone id="chest" rotationZ="-3" />
      <Bone id="head" rotationZ="2" />
      <Bone id="upper_leg_l" rotationZ="8" />
      <Bone id="lower_leg_l" rotationZ="-22" />
      <Bone id="foot_l" rotationZ="8" />
      <Bone id="upper_leg_r" rotationZ="-8" />
      <Bone id="lower_leg_r" rotationZ="22" />
      <Bone id="foot_r" rotationZ="-8" />
      <Bone id="upper_arm_l" rotationZ="-6" />
      <Bone id="forearm_l" rotationZ="4" />
      <Bone id="upper_arm_r" rotationZ="6" />
      <Bone id="forearm_r" rotationZ="-4" />
    </Pose>

    <Pose t="1.6" label="jump_loop">
      <Bone id="hips" y="0" rotationZ="0" />
      <Bone id="spine" rotationZ="0" />
      <Bone id="chest" rotationZ="0" />
      <Bone id="head" rotationZ="0" />
      <Bone id="upper_leg_l" rotationZ="0" />
      <Bone id="lower_leg_l" rotationZ="0" />
      <Bone id="foot_l" rotationZ="0" />
      <Bone id="upper_leg_r" rotationZ="0" />
      <Bone id="lower_leg_r" rotationZ="0" />
      <Bone id="foot_r" rotationZ="0" />
      <Bone id="upper_arm_l" rotationZ="-8" />
      <Bone id="forearm_l" rotationZ="6" />
      <Bone id="upper_arm_r" rotationZ="8" />
      <Bone id="forearm_r" rotationZ="-6" />
    </Pose>"#,
            ),
            Self::WaveHand => Some(
                r#"    <Pose t="0.0" label="wave_start">
      <Bone id="hips" turn="0" />
      <Bone id="spine" turn="0" bend="0" />
      <Bone id="chest" turn="0" bend="0" />
      <Bone id="head" turn="0" />
      <Bone id="upper_arm_r" forward="0" side="12" twist="0" />
      <Bone id="forearm_r" bend="8" twist="0" />
      <Bone id="hand_r" twist="0" />
      <Bone id="upper_arm_l" forward="0" side="8" twist="0" />
      <Bone id="forearm_l" bend="4" twist="0" />
    </Pose>

    <Pose t="0.30" label="wave_raise">
      <Bone id="hips" turn="-1" />
      <Bone id="spine" turn="-1" bend="0" />
      <Bone id="chest" turn="-3" bend="-2" />
      <Bone id="head" turn="3" />
      <Bone id="upper_arm_r" forward="0" side="78" twist="0" />
      <Bone id="forearm_r" bend="62" twist="-6" />
      <Bone id="hand_r" twist="-12" />
      <Bone id="upper_arm_l" forward="0" side="8" twist="0" />
      <Bone id="forearm_l" bend="4" twist="0" />
    </Pose>

    <Pose t="0.55" label="wave_out">
      <Bone id="hips" turn="-1" />
      <Bone id="spine" turn="-1" bend="0" />
      <Bone id="chest" turn="-3" bend="-2" />
      <Bone id="head" turn="3" />
      <Bone id="upper_arm_r" forward="0" side="84" twist="0" />
      <Bone id="forearm_r" bend="42" twist="16" />
      <Bone id="hand_r" twist="22" />
      <Bone id="upper_arm_l" forward="0" side="8" twist="0" />
      <Bone id="forearm_l" bend="4" twist="0" />
    </Pose>

    <Pose t="0.80" label="wave_in">
      <Bone id="hips" turn="-1" />
      <Bone id="spine" turn="-1" bend="0" />
      <Bone id="chest" turn="-4" bend="-2" />
      <Bone id="head" turn="4" />
      <Bone id="upper_arm_r" forward="0" side="86" twist="0" />
      <Bone id="forearm_r" bend="76" twist="-18" />
      <Bone id="hand_r" twist="-24" />
      <Bone id="upper_arm_l" forward="0" side="8" twist="0" />
      <Bone id="forearm_l" bend="4" twist="0" />
    </Pose>

    <Pose t="1.05" label="wave_out_2">
      <Bone id="hips" turn="-1" />
      <Bone id="spine" turn="-1" bend="0" />
      <Bone id="chest" turn="-3" bend="-2" />
      <Bone id="head" turn="3" />
      <Bone id="upper_arm_r" forward="0" side="84" twist="0" />
      <Bone id="forearm_r" bend="44" twist="18" />
      <Bone id="hand_r" twist="24" />
      <Bone id="upper_arm_l" forward="0" side="8" twist="0" />
      <Bone id="forearm_l" bend="4" twist="0" />
    </Pose>

    <Pose t="1.30" label="wave_in_2">
      <Bone id="hips" turn="-1" />
      <Bone id="spine" turn="-1" bend="0" />
      <Bone id="chest" turn="-4" bend="-2" />
      <Bone id="head" turn="4" />
      <Bone id="upper_arm_r" forward="0" side="86" twist="0" />
      <Bone id="forearm_r" bend="72" twist="-16" />
      <Bone id="hand_r" twist="-22" />
      <Bone id="upper_arm_l" forward="0" side="8" twist="0" />
      <Bone id="forearm_l" bend="4" twist="0" />
    </Pose>

    <Pose t="1.65" label="wave_lower">
      <Bone id="hips" turn="0" />
      <Bone id="spine" turn="0" bend="0" />
      <Bone id="chest" turn="-1" bend="-1" />
      <Bone id="head" turn="1" />
      <Bone id="upper_arm_r" forward="0" side="34" twist="0" />
      <Bone id="forearm_r" bend="22" twist="4" />
      <Bone id="hand_r" twist="4" />
      <Bone id="upper_arm_l" forward="0" side="8" twist="0" />
      <Bone id="forearm_l" bend="4" twist="0" />
    </Pose>

    <Pose t="2.0" label="wave_loop">
      <Bone id="hips" turn="0" />
      <Bone id="spine" turn="0" bend="0" />
      <Bone id="chest" turn="0" bend="0" />
      <Bone id="head" turn="0" />
      <Bone id="upper_arm_r" forward="0" side="12" twist="0" />
      <Bone id="forearm_r" bend="8" twist="0" />
      <Bone id="hand_r" twist="0" />
      <Bone id="upper_arm_l" forward="0" side="8" twist="0" />
      <Bone id="forearm_l" bend="4" twist="0" />
    </Pose>"#,
            ),
            Self::SidePlus20 => Some(
                r#"    <Pose t="0.0" label="Side +20">
      <Bone id="hips" turn="0" />
      <Bone id="spine" turn="0" bend="0" />
      <Bone id="chest" turn="0" bend="0" />
      <Bone id="head" turn="0" />
      <Bone id="upper_arm_l" forward="0" side="20" twist="0" />
      <Bone id="forearm_l" bend="0" twist="0" />
      <Bone id="hand_l" twist="0" />
      <Bone id="upper_arm_r" forward="0" side="20" twist="0" />
      <Bone id="forearm_r" bend="0" twist="0" />
      <Bone id="hand_r" twist="0" />
    </Pose>"#,
            ),
            Self::SideMinus20 => Some(
                r#"    <Pose t="0.0" label="Side -20">
      <Bone id="hips" turn="0" />
      <Bone id="spine" turn="0" bend="0" />
      <Bone id="chest" turn="0" bend="0" />
      <Bone id="head" turn="0" />
      <Bone id="upper_arm_l" forward="0" side="-20" twist="0" />
      <Bone id="forearm_l" bend="0" twist="0" />
      <Bone id="hand_l" twist="0" />
      <Bone id="upper_arm_r" forward="0" side="-20" twist="0" />
      <Bone id="forearm_r" bend="0" twist="0" />
      <Bone id="hand_r" twist="0" />
    </Pose>"#,
            ),
            Self::ForwardPlus20 => Some(
                r#"    <Pose t="0.0" label="Forward +20">
      <Bone id="hips" turn="0" />
      <Bone id="spine" turn="0" bend="0" />
      <Bone id="chest" turn="0" bend="0" />
      <Bone id="head" turn="0" />
      <Bone id="upper_arm_l" forward="20" side="0" twist="0" />
      <Bone id="upper_arm_r" forward="20" side="0" twist="0" />
      <Bone id="upper_leg_l" forward="20" side="0" />
      <Bone id="upper_leg_r" forward="20" side="0" />
    </Pose>"#,
            ),
            Self::BendPlus20 => Some(
                r#"    <Pose t="0.0" label="Bend +20">
      <Bone id="hips" turn="0" />
      <Bone id="spine" turn="0" bend="0" />
      <Bone id="chest" turn="0" bend="0" />
      <Bone id="head" turn="0" />
      <Bone id="forearm_l" bend="20" twist="0" />
      <Bone id="forearm_r" bend="20" twist="0" />
      <Bone id="lower_leg_l" bend="20" />
      <Bone id="lower_leg_r" bend="20" />
    </Pose>"#,
            ),
            Self::TwistPlus20 => Some(
                r#"    <Pose t="0.0" label="Twist +20">
      <Bone id="hips" turn="0" />
      <Bone id="spine" turn="0" bend="0" />
      <Bone id="chest" turn="0" bend="0" />
      <Bone id="head" turn="0" />
      <Bone id="upper_arm_l" forward="0" side="0" twist="20" />
      <Bone id="forearm_l" bend="0" twist="20" />
      <Bone id="hand_l" twist="20" />
      <Bone id="upper_arm_r" forward="0" side="0" twist="20" />
      <Bone id="forearm_r" bend="0" twist="20" />
      <Bone id="hand_r" twist="20" />
    </Pose>"#,
            ),
        }
    }

    fn is_axis_debug(self) -> bool {
        matches!(
            self,
            Self::SidePlus20
                | Self::SideMinus20
                | Self::ForwardPlus20
                | Self::BendPlus20
                | Self::TwistPlus20
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceRestPose {
    TPose,
    APose,
    ArmsDown,
}

impl SourceRestPose {
    fn label(self) -> &'static str {
        match self {
            Self::TPose => "T-Pose",
            Self::APose => "A-Pose",
            Self::ArmsDown => "Arms Down",
        }
    }

    fn arm_side_correction(self) -> f32 {
        match self {
            Self::TPose => -90.0,
            Self::APose => -35.0,
            Self::ArmsDown => 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct RestPoseSemanticCorrection {
    forward: f32,
    side: f32,
    twist: f32,
    bend: f32,
    turn: f32,
}

impl RestPoseSemanticCorrection {
    fn is_identity(self) -> bool {
        self.forward.abs() < 0.001
            && self.side.abs() < 0.001
            && self.twist.abs() < 0.001
            && self.bend.abs() < 0.001
            && self.turn.abs() < 0.001
    }

    fn add(self, other: Self) -> Self {
        Self {
            forward: self.forward + other.forward,
            side: self.side + other.side,
            twist: self.twist + other.twist,
            bend: self.bend + other.bend,
            turn: self.turn + other.turn,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct CharacterDesignSnapshot {
    profile: CharacterProfileControls,
    bone_axes: HashMap<String, RetargetBoneAxis>,
    source_rest_pose: SourceRestPose,
    rest_pose_corrections: HashMap<String, RestPoseSemanticCorrection>,
    retarget_maps: Vec<(String, String)>,
    raw_bone_rotations: HashMap<String, [f32; 3]>,
    hidden_meshes: HashSet<String>,
    hidden_materials: HashSet<String>,
    action: WaveActionControls,
    additional_actions: Vec<WaveActionControls>,
    actor_x: f32,
    actor_y: f32,
    actor_z: f32,
    actor_scale: f32,
    actor_yaw: f32,
    actor_pitch: f32,
    actor_roll: f32,
    graph_fps: u32,
    graph_width: u32,
    graph_height: u32,
    render_width: u32,
    render_height: u32,
    background_image_path: String,
    camera_x: f32,
    camera_y: f32,
    camera_z: f32,
    camera_yaw: f32,
    camera_pitch: f32,
    camera_distance: f32,
    additional_model_paths: Vec<String>,
    additional_actor_positions: Vec<[f32; 3]>,
    additional_actor_rotations: Vec<[f32; 3]>,
    additional_retarget_maps: Vec<Vec<(String, String)>>,
    additional_bone_axes: Vec<HashMap<String, RetargetBoneAxis>>,
    additional_source_rest_poses: Vec<SourceRestPose>,
    additional_rest_pose_corrections: Vec<HashMap<String, RestPoseSemanticCorrection>>,
    additional_raw_bones: Vec<Vec<RawBoneInfo>>,
    additional_selected_raw_bones: Vec<Option<usize>>,
    additional_selected_canonical_bones: Vec<Option<String>>,
    additional_raw_bone_rotations: Vec<HashMap<String, [f32; 3]>>,
    additional_mesh_names: Vec<Vec<String>>,
    additional_material_names: Vec<Vec<String>>,
    additional_hidden_meshes: Vec<HashSet<String>>,
    additional_hidden_materials: Vec<HashSet<String>>,
    selected_actor_slot: usize,
    duration_sec: f32,
    action_dsl: String,
    additional_action_dsls: Vec<String>,
    selected_action_keyframe_t: Option<f32>,
    additional_selected_action_keyframe_ts: Vec<Option<f32>>,
    test_action_preset: RetargetPosePreset,
    additional_test_action_presets: Vec<RetargetPosePreset>,
}

#[derive(Clone, Debug)]
struct ActionPoseBoneAttrs {
    id: String,
    attrs: HashMap<String, String>,
}

#[derive(Clone, Debug)]
struct CharacterDragState {
    handle: CharacterDragHandle,
    snapshot: CharacterDesignSnapshot,
    start_x: f32,
    start_y: f32,
    raise_forward: f32,
    raise_side: f32,
    elbow_bend: f32,
    wave_twist: f32,
    left_leg_forward: f32,
    left_knee_bend: f32,
    right_leg_forward: f32,
    right_knee_bend: f32,
    chest_turn: f32,
    head_turn: f32,
    raw_rotation: [f32; 3],
    rest_correction: RestPoseSemanticCorrection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ViewportDragMode {
    Pan,
    Orbit,
}

#[derive(Clone, Copy, Debug)]
struct ViewportDragState {
    mode: ViewportDragMode,
    start_x: f32,
    start_y: f32,
    camera_yaw: f32,
    camera_pitch: f32,
    camera_x: f32,
    camera_y: f32,
}

// Fit-to-container preview image element. This is intentionally local to the
// page so Character Design does not depend on MotionLoomPage internals.
struct FitPreviewImageElement {
    image: Arc<RenderImage>,
    width: u32,
    height: u32,
}

impl FitPreviewImageElement {
    fn new(image: Arc<RenderImage>, width: u32, height: u32) -> Self {
        Self {
            image,
            width,
            height,
        }
    }

    fn fitted_bounds(&self, bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
        let container_w: f32 = bounds.size.width.into();
        let container_h: f32 = bounds.size.height.into();
        let frame_w = self.width as f32;
        let frame_h = self.height as f32;
        if frame_w == 0.0 || frame_h == 0.0 {
            return bounds;
        }

        let fit_scale = (container_w / frame_w).min(container_h / frame_h);
        let dest_w = frame_w * fit_scale;
        let dest_h = frame_h * fit_scale;
        let offset_x = (container_w - dest_w) * 0.5;
        let offset_y = (container_h - dest_h) * 0.5;

        gpui::Bounds::new(
            gpui::point(
                bounds.origin.x + gpui::px(offset_x),
                bounds.origin.y + gpui::px(offset_y),
            ),
            gpui::size(gpui::px(dest_w), gpui::px(dest_h)),
        )
    }
}

impl Element for FitPreviewImageElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = Style {
            size: gpui::Size {
                width: gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0)),
                height: gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0)),
            },
            ..Default::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: gpui::Bounds<gpui::Pixels>,
        _state: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut gpui::App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        _layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut gpui::App,
    ) {
        let dest_bounds = self.fitted_bounds(bounds);
        let _ = window.paint_image(
            dest_bounds,
            gpui::Corners::default(),
            self.image.clone(),
            0,
            false,
        );
    }
}

impl IntoElement for FitPreviewImageElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct CharacterDesignPage {
    #[allow(dead_code)]
    global: Entity<GlobalState>,
    focus_handle: FocusHandle,
    model_path: String,
    profile: CharacterProfileControls,
    bone_axes: HashMap<String, RetargetBoneAxis>,
    source_rest_pose: SourceRestPose,
    rest_pose_corrections: HashMap<String, RestPoseSemanticCorrection>,
    retarget_maps: Vec<(String, String)>,
    raw_bones: Vec<RawBoneInfo>,
    selected_raw_bone: Option<usize>,
    selected_canonical_bone: Option<String>,
    raw_bone_rotations: HashMap<String, [f32; 3]>,
    mesh_names: Vec<String>,
    material_names: Vec<String>,
    hidden_meshes: HashSet<String>,
    hidden_materials: HashSet<String>,
    gpu_diagnostics: Option<WorldGpuDiagnostics>,
    action: WaveActionControls,
    additional_actions: Vec<WaveActionControls>,
    actor_x: f32,
    actor_y: f32,
    actor_z: f32,
    actor_scale: f32,
    actor_yaw: f32,
    actor_pitch: f32,
    actor_roll: f32,
    graph_fps: u32,
    graph_width: u32,
    graph_height: u32,
    render_width: u32,
    render_height: u32,
    duration_sec: f32,
    background_image_path: String,
    camera_x: f32,
    camera_y: f32,
    camera_z: f32,
    camera_yaw: f32,
    camera_pitch: f32,
    camera_distance: f32,
    additional_model_paths: Vec<String>,
    additional_actor_positions: Vec<[f32; 3]>,
    additional_actor_rotations: Vec<[f32; 3]>,
    additional_retarget_maps: Vec<Vec<(String, String)>>,
    additional_bone_axes: Vec<HashMap<String, RetargetBoneAxis>>,
    additional_source_rest_poses: Vec<SourceRestPose>,
    additional_rest_pose_corrections: Vec<HashMap<String, RestPoseSemanticCorrection>>,
    additional_raw_bones: Vec<Vec<RawBoneInfo>>,
    additional_selected_raw_bones: Vec<Option<usize>>,
    additional_selected_canonical_bones: Vec<Option<String>>,
    additional_raw_bone_rotations: Vec<HashMap<String, [f32; 3]>>,
    additional_mesh_names: Vec<Vec<String>>,
    additional_material_names: Vec<Vec<String>>,
    additional_hidden_meshes: Vec<HashSet<String>>,
    additional_hidden_materials: Vec<HashSet<String>>,
    selected_actor_slot: usize,
    frame: u32,
    playing: bool,
    play_token: u64,
    preview_key: Option<PreviewKey>,
    preview_image: Option<(Arc<RenderImage>, u32, u32)>,
    preview_pending: bool,
    preview_dirty: bool,
    preview_error: Option<String>,
    preview_token: u64,
    graph_input_token: u64,
    preview_tx: Sender<CharacterPreviewRequest>,
    drag_state: Option<CharacterDragState>,
    viewport_drag_state: Option<ViewportDragState>,
    show_canonical_pose: bool,
    undo_stack: Vec<CharacterDesignSnapshot>,
    redo_stack: Vec<CharacterDesignSnapshot>,
    action_dsl: String,
    additional_action_dsls: Vec<String>,
    selected_action_keyframe_t: Option<f32>,
    additional_selected_action_keyframe_ts: Vec<Option<f32>>,
    test_action_preset: RetargetPosePreset,
    additional_test_action_presets: Vec<RetargetPosePreset>,
    pose_calibration_preset: Option<RetargetPosePreset>,
    action_dsl_input: Option<Entity<InputState>>,
    action_dsl_input_sub: Option<Subscription>,
    action_dsl_input_syncing: bool,
    frame_input: Option<Entity<InputState>>,
    frame_input_sub: Option<Subscription>,
    duration_input: Option<Entity<InputState>>,
    duration_input_sub: Option<Subscription>,
    graph_fps_input: Option<Entity<InputState>>,
    graph_fps_input_sub: Option<Subscription>,
    graph_width_input: Option<Entity<InputState>>,
    graph_width_input_sub: Option<Subscription>,
    graph_height_input: Option<Entity<InputState>>,
    graph_height_input_sub: Option<Subscription>,
    render_width_input: Option<Entity<InputState>>,
    render_width_input_sub: Option<Subscription>,
    render_height_input: Option<Entity<InputState>>,
    render_height_input_sub: Option<Subscription>,
    timeline_input_syncing: bool,
    status_line: String,
}

impl CharacterDesignPage {
    pub fn new(global: Entity<GlobalState>, cx: &mut Context<Self>) -> Self {
        Self {
            global,
            focus_handle: cx.focus_handle(),
            model_path: DEFAULT_MODEL.to_string(),
            profile: CharacterProfileControls::default(),
            bone_axes: Self::default_bone_axes(),
            source_rest_pose: SourceRestPose::ArmsDown,
            rest_pose_corrections: HashMap::new(),
            retarget_maps: Vec::new(),
            raw_bones: Vec::new(),
            selected_raw_bone: None,
            selected_canonical_bone: None,
            raw_bone_rotations: HashMap::new(),
            mesh_names: Vec::new(),
            material_names: Vec::new(),
            hidden_meshes: HashSet::new(),
            hidden_materials: HashSet::new(),
            gpu_diagnostics: None,
            action: WaveActionControls::default(),
            additional_actions: Vec::new(),
            actor_x: 0.0,
            actor_y: 0.0,
            actor_z: 0.0,
            actor_scale: 1.0,
            actor_yaw: 5.0,
            actor_pitch: 0.0,
            actor_roll: 0.0,
            graph_fps: DEFAULT_FPS,
            graph_width: DEFAULT_GRAPH_WIDTH,
            graph_height: DEFAULT_GRAPH_HEIGHT,
            render_width: DEFAULT_RENDER_WIDTH,
            render_height: DEFAULT_RENDER_HEIGHT,
            duration_sec: DEFAULT_DURATION_SEC,
            background_image_path: String::new(),
            camera_x: 0.0,
            camera_y: 0.0,
            camera_z: 0.0,
            camera_yaw: 0.0,
            camera_pitch: 0.0,
            camera_distance: 3.4,
            additional_model_paths: Vec::new(),
            additional_actor_positions: Vec::new(),
            additional_actor_rotations: Vec::new(),
            additional_retarget_maps: Vec::new(),
            additional_bone_axes: Vec::new(),
            additional_source_rest_poses: Vec::new(),
            additional_rest_pose_corrections: Vec::new(),
            additional_raw_bones: Vec::new(),
            additional_selected_raw_bones: Vec::new(),
            additional_selected_canonical_bones: Vec::new(),
            additional_raw_bone_rotations: Vec::new(),
            additional_mesh_names: Vec::new(),
            additional_material_names: Vec::new(),
            additional_hidden_meshes: Vec::new(),
            additional_hidden_materials: Vec::new(),
            selected_actor_slot: 0,
            frame: 0,
            playing: false,
            play_token: 0,
            preview_key: None,
            preview_image: None,
            preview_pending: false,
            preview_dirty: false,
            preview_error: None,
            preview_token: 0,
            graph_input_token: 0,
            preview_tx: Self::spawn_preview_worker(),
            drag_state: None,
            viewport_drag_state: None,
            show_canonical_pose: true,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            action_dsl: String::new(),
            additional_action_dsls: Vec::new(),
            selected_action_keyframe_t: None,
            additional_selected_action_keyframe_ts: Vec::new(),
            test_action_preset: RetargetPosePreset::Original,
            additional_test_action_presets: Vec::new(),
            pose_calibration_preset: None,
            action_dsl_input: None,
            action_dsl_input_sub: None,
            action_dsl_input_syncing: false,
            frame_input: None,
            frame_input_sub: None,
            duration_input: None,
            duration_input_sub: None,
            graph_fps_input: None,
            graph_fps_input_sub: None,
            graph_width_input: None,
            graph_width_input_sub: None,
            graph_height_input: None,
            graph_height_input_sub: None,
            render_width_input: None,
            render_width_input_sub: None,
            render_height_input: None,
            render_height_input_sub: None,
            timeline_input_syncing: false,
            status_line: "Load a GLB to start 3D Layout.".to_string(),
        }
    }

    fn capture_snapshot(&self) -> CharacterDesignSnapshot {
        CharacterDesignSnapshot {
            profile: self.profile.clone(),
            bone_axes: self.bone_axes.clone(),
            source_rest_pose: self.source_rest_pose,
            rest_pose_corrections: self.rest_pose_corrections.clone(),
            retarget_maps: self.retarget_maps.clone(),
            raw_bone_rotations: self.raw_bone_rotations.clone(),
            hidden_meshes: self.hidden_meshes.clone(),
            hidden_materials: self.hidden_materials.clone(),
            action: self.action.clone(),
            additional_actions: self.additional_actions.clone(),
            actor_x: self.actor_x,
            actor_y: self.actor_y,
            actor_z: self.actor_z,
            actor_scale: self.actor_scale,
            actor_yaw: self.actor_yaw,
            actor_pitch: self.actor_pitch,
            actor_roll: self.actor_roll,
            graph_fps: self.graph_fps,
            graph_width: self.graph_width,
            graph_height: self.graph_height,
            render_width: self.render_width,
            render_height: self.render_height,
            background_image_path: self.background_image_path.clone(),
            camera_x: self.camera_x,
            camera_y: self.camera_y,
            camera_z: self.camera_z,
            camera_yaw: self.camera_yaw,
            camera_pitch: self.camera_pitch,
            camera_distance: self.camera_distance,
            additional_model_paths: self.additional_model_paths.clone(),
            additional_actor_positions: self.additional_actor_positions.clone(),
            additional_actor_rotations: self.additional_actor_rotations.clone(),
            additional_retarget_maps: self.additional_retarget_maps.clone(),
            additional_bone_axes: self.additional_bone_axes.clone(),
            additional_source_rest_poses: self.additional_source_rest_poses.clone(),
            additional_rest_pose_corrections: self.additional_rest_pose_corrections.clone(),
            additional_raw_bones: self.additional_raw_bones.clone(),
            additional_selected_raw_bones: self.additional_selected_raw_bones.clone(),
            additional_selected_canonical_bones: self.additional_selected_canonical_bones.clone(),
            additional_raw_bone_rotations: self.additional_raw_bone_rotations.clone(),
            additional_mesh_names: self.additional_mesh_names.clone(),
            additional_material_names: self.additional_material_names.clone(),
            additional_hidden_meshes: self.additional_hidden_meshes.clone(),
            additional_hidden_materials: self.additional_hidden_materials.clone(),
            selected_actor_slot: self.selected_actor_slot,
            duration_sec: self.duration_sec,
            action_dsl: self.action_dsl.clone(),
            additional_action_dsls: self.additional_action_dsls.clone(),
            selected_action_keyframe_t: self.selected_action_keyframe_t,
            additional_selected_action_keyframe_ts: self
                .additional_selected_action_keyframe_ts
                .clone(),
            test_action_preset: self.test_action_preset,
            additional_test_action_presets: self.additional_test_action_presets.clone(),
        }
    }

    fn restore_snapshot(&mut self, snapshot: CharacterDesignSnapshot) {
        self.profile = snapshot.profile;
        self.bone_axes = snapshot.bone_axes;
        self.source_rest_pose = snapshot.source_rest_pose;
        self.rest_pose_corrections = snapshot.rest_pose_corrections;
        self.retarget_maps = snapshot.retarget_maps;
        self.raw_bone_rotations = snapshot.raw_bone_rotations;
        self.hidden_meshes = snapshot.hidden_meshes;
        self.hidden_materials = snapshot.hidden_materials;
        self.action = snapshot.action;
        self.additional_actions = snapshot.additional_actions;
        self.actor_x = snapshot.actor_x;
        self.actor_y = snapshot.actor_y;
        self.actor_z = snapshot.actor_z;
        self.actor_scale = snapshot.actor_scale;
        self.actor_yaw = snapshot.actor_yaw;
        self.actor_pitch = snapshot.actor_pitch;
        self.actor_roll = snapshot.actor_roll;
        self.graph_fps = snapshot.graph_fps;
        self.graph_width = snapshot.graph_width;
        self.graph_height = snapshot.graph_height;
        self.render_width = snapshot.render_width;
        self.render_height = snapshot.render_height;
        self.background_image_path = snapshot.background_image_path;
        self.camera_x = snapshot.camera_x;
        self.camera_y = snapshot.camera_y;
        self.camera_z = snapshot.camera_z;
        self.camera_yaw = snapshot.camera_yaw;
        self.camera_pitch = snapshot.camera_pitch;
        self.camera_distance = snapshot.camera_distance;
        self.additional_model_paths = snapshot.additional_model_paths;
        self.additional_actor_positions = snapshot.additional_actor_positions;
        self.additional_actor_rotations = snapshot.additional_actor_rotations;
        self.additional_retarget_maps = snapshot.additional_retarget_maps;
        self.additional_bone_axes = snapshot.additional_bone_axes;
        self.additional_source_rest_poses = snapshot.additional_source_rest_poses;
        self.additional_rest_pose_corrections = snapshot.additional_rest_pose_corrections;
        self.additional_raw_bones = snapshot.additional_raw_bones;
        self.additional_selected_raw_bones = snapshot.additional_selected_raw_bones;
        self.additional_selected_canonical_bones = snapshot.additional_selected_canonical_bones;
        self.additional_raw_bone_rotations = snapshot.additional_raw_bone_rotations;
        self.additional_mesh_names = snapshot.additional_mesh_names;
        self.additional_material_names = snapshot.additional_material_names;
        self.additional_hidden_meshes = snapshot.additional_hidden_meshes;
        self.additional_hidden_materials = snapshot.additional_hidden_materials;
        self.selected_actor_slot = snapshot.selected_actor_slot;
        self.duration_sec = snapshot.duration_sec;
        self.action_dsl = snapshot.action_dsl;
        self.additional_action_dsls = snapshot.additional_action_dsls;
        self.selected_action_keyframe_t = snapshot.selected_action_keyframe_t;
        self.additional_selected_action_keyframe_ts =
            snapshot.additional_selected_action_keyframe_ts;
        self.test_action_preset = snapshot.test_action_preset;
        self.additional_test_action_presets = snapshot.additional_test_action_presets;
        self.normalize_additional_model_state();
        self.invalidate_preview();
    }

    fn push_undo_snapshot_if_changed(&mut self, before: CharacterDesignSnapshot) {
        if before == self.capture_snapshot() {
            return;
        }
        self.undo_stack.push(before);
        if self.undo_stack.len() > MAX_UNDO_STEPS {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    fn undo_pose_edit(&mut self) {
        let Some(previous) = self.undo_stack.pop() else {
            self.status_line = "Nothing to undo.".to_string();
            return;
        };
        let current = self.capture_snapshot();
        self.redo_stack.push(current);
        self.restore_snapshot(previous);
        self.status_line = "Undo pose edit.".to_string();
    }

    fn redo_pose_edit(&mut self) {
        let Some(next) = self.redo_stack.pop() else {
            self.status_line = "Nothing to redo.".to_string();
            return;
        };
        let current = self.capture_snapshot();
        self.undo_stack.push(current);
        self.restore_snapshot(next);
        self.status_line = "Redo pose edit.".to_string();
    }

    fn spawn_preview_worker() -> Sender<CharacterPreviewRequest> {
        let (request_tx, request_rx) = mpsc::channel::<CharacterPreviewRequest>();
        std::thread::spawn(move || {
            let mut viewport = CharacterDesignGpuViewport::new();
            let mut cached_script = String::new();
            let mut cached_graph: Option<WorldGraph> = None;
            while let Ok(mut request) = request_rx.recv() {
                while let Ok(next_request) = request_rx.try_recv() {
                    request = next_request;
                }
                let result = (|| {
                    if cached_graph.is_none() || cached_script != request.script {
                        cached_graph = Some(
                            parse_world_graph_script(&request.script)
                                .map_err(|err| format!("Character DSL parse error: {err}"))?,
                        );
                        cached_script = request.script.clone();
                    }
                    let graph = cached_graph
                        .as_ref()
                        .expect("cached graph is parsed before Character Design preview render");
                    pollster::block_on(viewport.render_frame(
                        graph,
                        request.frame,
                        &request.asset_root,
                        &request.actor_id,
                    ))
                    .map_err(|err| format!("Character GPU preview error: {err}"))
                    .map(|frame| (frame.image, frame.diagnostics))
                })()
                .map(|(rgba, diagnostics)| {
                    let (w, h) = rgba.dimensions();
                    let mut bgra = rgba.into_raw();
                    for px in bgra.chunks_mut(4) {
                        px.swap(0, 2);
                    }
                    CharacterPreviewResponse {
                        width: w,
                        height: h,
                        bgra,
                        diagnostics,
                    }
                });
                let _ = request.response_tx.send(result);
            }
        });
        request_tx
    }

    fn world_asset_root() -> PathBuf {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        for candidate in [
            cwd.join("examples/motionloom/world"),
            cwd.join("anica/examples/motionloom/world"),
            PathBuf::from("examples/motionloom/world"),
            PathBuf::from("anica/examples/motionloom/world"),
        ] {
            if candidate.exists() {
                return candidate;
            }
        }
        PathBuf::from("examples/motionloom/world")
    }

    fn resolve_current_model_path(&self) -> PathBuf {
        let path = Path::new(&self.model_path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            Self::world_asset_root().join(path)
        }
    }

    fn model_path_style(&self) -> &'static str {
        Self::path_style_for(&self.model_path)
    }

    fn path_style_for(path: &str) -> &'static str {
        if Path::new(path).is_absolute() {
            "absolute"
        } else {
            "relative"
        }
    }

    fn display_model_path(path: PathBuf) -> String {
        let asset_root = Self::world_asset_root();
        if let Ok(relative) = path.strip_prefix(&asset_root) {
            relative.to_string_lossy().to_string()
        } else if let Some(motionloom_root) = asset_root.parent() {
            path.strip_prefix(motionloom_root)
                .map(|relative| format!("../{}", relative.to_string_lossy()))
                .unwrap_or_else(|_| path.to_string_lossy().to_string())
        } else {
            path.to_string_lossy().to_string()
        }
    }

    fn display_model_name(path: &str) -> String {
        Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(path)
            .to_string()
    }

    fn resolve_model_path_value(path: &str) -> PathBuf {
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            Self::world_asset_root().join(path)
        }
    }

    fn inspect_model(path: &str) -> Result<LoadedModelInspection, String> {
        let path = Self::resolve_model_path_value(path);
        let mesh = load_glb_mesh_data(&path).map_err(|err| err.to_string())?;
        let global = Self::global_node_matrices(&mesh.nodes);
        let node_position = |node_index: usize| {
            Self::mat4_transform_point(
                global
                    .get(node_index)
                    .copied()
                    .unwrap_or_else(Self::mat4_identity),
                [0.0, 0.0, 0.0],
            )
        };
        let mesh_names = mesh
            .mesh_names
            .iter()
            .filter_map(Clone::clone)
            .collect::<Vec<_>>();
        let material_names = mesh
            .materials
            .iter()
            .filter_map(|material| material.name.clone())
            .collect::<Vec<_>>();
        let hidden_meshes = mesh_names
            .iter()
            .filter(|name| Self::is_default_hidden_asset_name(name))
            .cloned()
            .collect::<HashSet<_>>();
        let hidden_materials = material_names
            .iter()
            .filter(|name| Self::is_default_hidden_asset_name(name))
            .cloned()
            .collect::<HashSet<_>>();
        let joint_names = mesh
            .skin
            .as_ref()
            .map(|skin| {
                skin.joints
                    .iter()
                    .filter_map(|joint| joint.name.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let raw_bones = mesh
            .skin
            .as_ref()
            .map(|skin| {
                skin.joints
                    .iter()
                    .enumerate()
                    .map(|(index, joint)| RawBoneInfo {
                        index,
                        name: joint
                            .name
                            .clone()
                            .unwrap_or_else(|| format!("node_{}", joint.node_index)),
                        position: node_position(joint.node_index),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let retarget_maps =
            Self::sort_retarget_maps(&Self::guess_humanoid_retarget_maps(&joint_names));
        let source_rest_pose = Self::estimate_source_rest_pose_for_maps(&raw_bones, &retarget_maps);
        let bounds_height = (mesh.bounds_max[1] - mesh.bounds_min[1]).abs();
        Ok(LoadedModelInspection {
            retarget_maps,
            raw_bones,
            mesh_names,
            material_names,
            hidden_meshes,
            hidden_materials,
            diagnostics: Some(diagnose_world_glb_gpu_plan(&mesh)),
            bounds_height,
            source_rest_pose,
        })
    }

    fn normalize_additional_model_state(&mut self) {
        let len = self.additional_model_paths.len();
        self.additional_actions
            .resize_with(len, WaveActionControls::default);
        while self.additional_actor_positions.len() < len {
            let index = self.additional_actor_positions.len();
            self.additional_actor_positions
                .push(Self::additional_actor_default_position(index));
        }
        self.additional_actor_positions.truncate(len);
        self.additional_actor_rotations
            .resize_with(len, Self::additional_actor_default_rotation);
        self.additional_actor_rotations.truncate(len);
        self.additional_retarget_maps.resize_with(len, Vec::new);
        self.additional_bone_axes
            .resize_with(len, Self::default_bone_axes);
        self.additional_bone_axes.truncate(len);
        self.additional_source_rest_poses
            .resize_with(len, || SourceRestPose::ArmsDown);
        self.additional_source_rest_poses.truncate(len);
        self.additional_rest_pose_corrections
            .resize_with(len, HashMap::new);
        self.additional_rest_pose_corrections.truncate(len);
        self.additional_raw_bones.resize_with(len, Vec::new);
        self.additional_selected_raw_bones.resize_with(len, || None);
        self.additional_selected_canonical_bones
            .resize_with(len, || None);
        self.additional_raw_bone_rotations
            .resize_with(len, HashMap::new);
        self.additional_mesh_names.resize_with(len, Vec::new);
        self.additional_material_names.resize_with(len, Vec::new);
        self.additional_hidden_meshes.resize_with(len, HashSet::new);
        self.additional_hidden_materials
            .resize_with(len, HashSet::new);
        self.additional_action_dsls.resize_with(len, String::new);
        self.additional_selected_action_keyframe_ts
            .resize_with(len, || None);
        self.additional_test_action_presets
            .resize_with(len, || RetargetPosePreset::Original);
        if self.selected_actor_slot > len {
            self.selected_actor_slot = 0;
        }
    }

    fn actor_id_for_slot(slot: usize) -> String {
        if slot == 0 {
            "design_actor".to_string()
        } else {
            format!("loaded_glb_actor_{slot}")
        }
    }

    fn profile_id_for_slot(slot: usize) -> String {
        if slot == 0 {
            "character_design_profile".to_string()
        } else {
            format!("character_design_profile_{slot}")
        }
    }

    fn action_id_for_slot(slot: usize) -> String {
        if slot == 0 {
            "designed_pose".to_string()
        } else {
            format!("designed_pose_{slot}")
        }
    }

    fn actor_label_for_slot(&self, slot: usize) -> String {
        if slot == 0 {
            if self.model_path.trim().is_empty() {
                "No GLB loaded".to_string()
            } else {
                format!("#1 {}", Self::display_model_name(&self.model_path))
            }
        } else {
            self.additional_model_paths
                .get(slot - 1)
                .map(|path| format!("#{} {}", slot + 1, Self::display_model_name(path)))
                .unwrap_or_else(|| format!("#{} missing GLB", slot + 1))
        }
    }

    fn selected_actor_id(&self) -> String {
        Self::actor_id_for_slot(self.selected_actor_slot)
    }

    fn selected_actor_label(&self) -> String {
        self.actor_label_for_slot(self.selected_actor_slot)
    }

    fn select_actor_slot(&mut self, slot: usize) {
        self.normalize_additional_model_state();
        self.selected_actor_slot = slot.min(self.additional_model_paths.len());
        self.apply_action_dsl_pose_to_controls();
        self.invalidate_preview();
        self.status_line = format!("Editing action target: {}.", self.selected_actor_label());
    }

    fn action_for_slot(&self, slot: usize) -> &WaveActionControls {
        if slot == 0 {
            &self.action
        } else {
            self.additional_actions
                .get(slot - 1)
                .unwrap_or(&self.action)
        }
    }

    fn active_action(&self) -> &WaveActionControls {
        self.action_for_slot(self.selected_actor_slot)
    }

    fn active_action_mut(&mut self) -> &mut WaveActionControls {
        self.normalize_additional_model_state();
        if self.selected_actor_slot == 0 {
            &mut self.action
        } else {
            &mut self.additional_actions[self.selected_actor_slot - 1]
        }
    }

    fn action_dsl_for_slot(&self, slot: usize) -> &str {
        if slot == 0 {
            self.action_dsl.as_str()
        } else {
            self.additional_action_dsls
                .get(slot - 1)
                .map(String::as_str)
                .unwrap_or("")
        }
    }

    fn active_action_dsl(&self) -> &str {
        self.action_dsl_for_slot(self.selected_actor_slot)
    }

    fn active_action_dsl_mut(&mut self) -> &mut String {
        self.normalize_additional_model_state();
        if self.selected_actor_slot == 0 {
            &mut self.action_dsl
        } else {
            &mut self.additional_action_dsls[self.selected_actor_slot - 1]
        }
    }

    fn active_selected_action_keyframe_t(&self) -> Option<f32> {
        if self.selected_actor_slot == 0 {
            self.selected_action_keyframe_t
        } else {
            self.additional_selected_action_keyframe_ts
                .get(self.selected_actor_slot - 1)
                .copied()
                .flatten()
        }
    }

    fn set_active_selected_action_keyframe_t(&mut self, value: Option<f32>) {
        self.normalize_additional_model_state();
        if self.selected_actor_slot == 0 {
            self.selected_action_keyframe_t = value;
        } else {
            self.additional_selected_action_keyframe_ts[self.selected_actor_slot - 1] = value;
        }
    }

    fn active_test_action_preset(&self) -> RetargetPosePreset {
        if self.selected_actor_slot == 0 {
            self.test_action_preset
        } else {
            self.additional_test_action_presets
                .get(self.selected_actor_slot - 1)
                .copied()
                .unwrap_or(RetargetPosePreset::Original)
        }
    }

    fn set_active_test_action_preset(&mut self, preset: RetargetPosePreset) {
        self.normalize_additional_model_state();
        if self.selected_actor_slot == 0 {
            self.test_action_preset = preset;
        } else {
            self.additional_test_action_presets[self.selected_actor_slot - 1] = preset;
        }
    }

    fn active_retarget_maps(&self) -> &[(String, String)] {
        if self.selected_actor_slot == 0 {
            &self.retarget_maps
        } else {
            self.additional_retarget_maps
                .get(self.selected_actor_slot - 1)
                .map(Vec::as_slice)
                .unwrap_or(&[])
        }
    }

    fn active_retarget_maps_mut(&mut self) -> &mut Vec<(String, String)> {
        self.normalize_additional_model_state();
        if self.selected_actor_slot == 0 {
            &mut self.retarget_maps
        } else {
            &mut self.additional_retarget_maps[self.selected_actor_slot - 1]
        }
    }

    fn bone_axes_for_slot(&self, slot: usize) -> &HashMap<String, RetargetBoneAxis> {
        if slot == 0 {
            &self.bone_axes
        } else {
            self.additional_bone_axes
                .get(slot - 1)
                .unwrap_or(&self.bone_axes)
        }
    }

    fn active_bone_axes(&self) -> &HashMap<String, RetargetBoneAxis> {
        self.bone_axes_for_slot(self.selected_actor_slot)
    }

    fn active_bone_axes_mut(&mut self) -> &mut HashMap<String, RetargetBoneAxis> {
        self.normalize_additional_model_state();
        if self.selected_actor_slot == 0 {
            &mut self.bone_axes
        } else {
            &mut self.additional_bone_axes[self.selected_actor_slot - 1]
        }
    }

    fn source_rest_pose_for_slot(&self, slot: usize) -> SourceRestPose {
        if slot == 0 {
            self.source_rest_pose
        } else {
            self.additional_source_rest_poses
                .get(slot - 1)
                .copied()
                .unwrap_or(SourceRestPose::ArmsDown)
        }
    }

    fn active_source_rest_pose(&self) -> SourceRestPose {
        self.source_rest_pose_for_slot(self.selected_actor_slot)
    }

    fn rest_pose_corrections_for_slot(
        &self,
        slot: usize,
    ) -> &HashMap<String, RestPoseSemanticCorrection> {
        if slot == 0 {
            &self.rest_pose_corrections
        } else {
            self.additional_rest_pose_corrections
                .get(slot - 1)
                .unwrap_or(&self.rest_pose_corrections)
        }
    }

    fn active_rest_pose_corrections(&self) -> &HashMap<String, RestPoseSemanticCorrection> {
        self.rest_pose_corrections_for_slot(self.selected_actor_slot)
    }

    fn active_rest_pose_corrections_mut(
        &mut self,
    ) -> &mut HashMap<String, RestPoseSemanticCorrection> {
        self.normalize_additional_model_state();
        if self.selected_actor_slot == 0 {
            &mut self.rest_pose_corrections
        } else {
            &mut self.additional_rest_pose_corrections[self.selected_actor_slot - 1]
        }
    }

    fn active_raw_bones(&self) -> &[RawBoneInfo] {
        if self.selected_actor_slot == 0 {
            &self.raw_bones
        } else {
            self.additional_raw_bones
                .get(self.selected_actor_slot - 1)
                .map(Vec::as_slice)
                .unwrap_or(&[])
        }
    }

    fn active_selected_raw_bone(&self) -> Option<usize> {
        if self.selected_actor_slot == 0 {
            self.selected_raw_bone
        } else {
            self.additional_selected_raw_bones
                .get(self.selected_actor_slot - 1)
                .copied()
                .flatten()
        }
    }

    fn set_active_selected_raw_bone(&mut self, value: Option<usize>) {
        self.normalize_additional_model_state();
        if self.selected_actor_slot == 0 {
            self.selected_raw_bone = value;
        } else {
            self.additional_selected_raw_bones[self.selected_actor_slot - 1] = value;
        }
    }

    fn active_selected_canonical_bone(&self) -> Option<&str> {
        if self.selected_actor_slot == 0 {
            self.selected_canonical_bone.as_deref()
        } else {
            self.additional_selected_canonical_bones
                .get(self.selected_actor_slot - 1)
                .and_then(|value| value.as_deref())
        }
    }

    fn set_active_selected_canonical_bone(&mut self, value: Option<String>) {
        self.normalize_additional_model_state();
        if self.selected_actor_slot == 0 {
            self.selected_canonical_bone = value;
        } else {
            self.additional_selected_canonical_bones[self.selected_actor_slot - 1] = value;
        }
    }

    fn active_raw_bone_rotations(&self) -> &HashMap<String, [f32; 3]> {
        if self.selected_actor_slot == 0 {
            &self.raw_bone_rotations
        } else {
            self.additional_raw_bone_rotations
                .get(self.selected_actor_slot - 1)
                .unwrap_or(&self.raw_bone_rotations)
        }
    }

    fn active_raw_bone_rotations_mut(&mut self) -> &mut HashMap<String, [f32; 3]> {
        self.normalize_additional_model_state();
        if self.selected_actor_slot == 0 {
            &mut self.raw_bone_rotations
        } else {
            &mut self.additional_raw_bone_rotations[self.selected_actor_slot - 1]
        }
    }

    fn active_mesh_names(&self) -> &[String] {
        if self.selected_actor_slot == 0 {
            &self.mesh_names
        } else {
            self.additional_mesh_names
                .get(self.selected_actor_slot - 1)
                .map(Vec::as_slice)
                .unwrap_or(&[])
        }
    }

    fn active_material_names(&self) -> &[String] {
        if self.selected_actor_slot == 0 {
            &self.material_names
        } else {
            self.additional_material_names
                .get(self.selected_actor_slot - 1)
                .map(Vec::as_slice)
                .unwrap_or(&[])
        }
    }

    fn active_hidden_meshes_contains(&self, name: &str) -> bool {
        if self.selected_actor_slot == 0 {
            self.hidden_meshes.contains(name)
        } else {
            self.additional_hidden_meshes
                .get(self.selected_actor_slot - 1)
                .is_some_and(|hidden| hidden.contains(name))
        }
    }

    fn active_hidden_materials_contains(&self, name: &str) -> bool {
        if self.selected_actor_slot == 0 {
            self.hidden_materials.contains(name)
        } else {
            self.additional_hidden_materials
                .get(self.selected_actor_slot - 1)
                .is_some_and(|hidden| hidden.contains(name))
        }
    }

    fn toggle_active_hidden_mesh(&mut self, name: String) {
        self.normalize_additional_model_state();
        let hidden = if self.selected_actor_slot == 0 {
            &mut self.hidden_meshes
        } else {
            &mut self.additional_hidden_meshes[self.selected_actor_slot - 1]
        };
        if !hidden.remove(&name) {
            hidden.insert(name);
        }
    }

    fn toggle_active_hidden_material(&mut self, name: String) {
        self.normalize_additional_model_state();
        let hidden = if self.selected_actor_slot == 0 {
            &mut self.hidden_materials
        } else {
            &mut self.additional_hidden_materials[self.selected_actor_slot - 1]
        };
        if !hidden.remove(&name) {
            hidden.insert(name);
        }
    }

    fn refresh_retarget_from_model(&mut self) {
        if self.model_path.trim().is_empty() {
            self.actor_scale = 1.0;
            self.gpu_diagnostics = None;
            self.mesh_names.clear();
            self.material_names.clear();
            self.hidden_meshes.clear();
            self.hidden_materials.clear();
            self.raw_bones.clear();
            self.selected_raw_bone = None;
            self.selected_canonical_bone = None;
            self.raw_bone_rotations.clear();
            self.rest_pose_corrections.clear();
            self.retarget_maps.clear();
            self.source_rest_pose = SourceRestPose::ArmsDown;
            self.status_line = "Load a GLB to start 3D Layout.".to_string();
            return;
        }
        let path = self.resolve_current_model_path();
        match Self::inspect_model(&path.to_string_lossy()) {
            Ok(inspection) => {
                self.actor_scale = 1.0;
                self.gpu_diagnostics = inspection.diagnostics;
                self.mesh_names = inspection.mesh_names;
                self.material_names = inspection.material_names;
                self.hidden_meshes = inspection.hidden_meshes;
                self.hidden_materials = inspection.hidden_materials;
                self.raw_bones = inspection.raw_bones;
                self.source_rest_pose = inspection.source_rest_pose;
                self.selected_raw_bone = (!self.raw_bones.is_empty()).then_some(0);
                self.selected_canonical_bone = None;
                self.raw_bone_rotations.clear();
                self.rest_pose_corrections.clear();
                let maps = inspection.retarget_maps;
                self.retarget_maps = maps.clone();
                if maps.is_empty() {
                    self.status_line = format!(
                        "Loaded GLB but no humanoid retarget guess was found: {}",
                        self.model_path
                    );
                } else {
                    let mapped = maps.len();
                    self.status_line = format!(
                        "Loaded GLB with auto retarget: {} humanoid bone(s) mapped. Bounds height {:.2}. Source rest {}.",
                        mapped,
                        inspection.bounds_height,
                        self.source_rest_pose.label()
                    );
                }
            }
            Err(err) => {
                self.gpu_diagnostics = None;
                self.mesh_names.clear();
                self.material_names.clear();
                self.hidden_meshes.clear();
                self.hidden_materials.clear();
                self.raw_bones.clear();
                self.selected_raw_bone = None;
                self.selected_canonical_bone = None;
                self.raw_bone_rotations.clear();
                self.rest_pose_corrections.clear();
                self.retarget_maps.clear();
                self.source_rest_pose = SourceRestPose::ArmsDown;
                self.status_line = format!("Loaded GLB path, but retarget inspect failed: {err}");
            }
        }
    }

    fn is_default_hidden_asset_name(name: &str) -> bool {
        let normalized = name.to_ascii_lowercase();
        normalized == "wall"
            || normalized.contains("_wall")
            || normalized.contains("wall_")
            || normalized == "outline"
            || normalized.contains("_outline")
            || normalized.contains("outline_")
            || normalized.starts_with("outline")
    }

    fn gpu_diagnostics_text(&self) -> String {
        let Some(diag) = self.gpu_diagnostics.as_ref() else {
            return "GPU diagnostics: no GLB loaded.".to_string();
        };
        let mut lines = vec![
            format!("mesh loaded: {}", diag.mesh_loaded),
            format!(
                "vertices: {}  triangles: {}  skin joints: {}",
                diag.vertex_count, diag.triangle_count, diag.skin_joint_count
            ),
            format!(
                "materials: {}  textures: {}/{} decoded",
                diag.material_count, diag.decoded_texture_count, diag.texture_count
            ),
            format!(
                "gpu draw count: {}  gpu vertices: {}",
                diag.gpu_draw_count, diag.gpu_vertex_count
            ),
            format!("bone overrides: {}", diag.bone_override_count),
            format!(
                "projected inside: {}  nonfinite: {}",
                diag.projected_inside_count, diag.projected_nonfinite_count
            ),
        ];
        if let Some(bounds) = &diag.projected_bounds {
            lines.push(format!("screen bbox: {bounds}"));
        }
        if let Some(bounds) = &diag.raw_draw_bounds {
            lines.push(format!("raw draw bounds: {bounds}"));
        }
        if let Some(bounds) = &diag.shader_local_bounds {
            lines.push(format!("shader local bounds: {bounds}"));
        }
        if let Some(bounds) = &diag.shader_projected_bounds {
            lines.push(format!(
                "shader bbox: {bounds}  inside: {}",
                diag.shader_projected_inside_count
            ));
        }
        if diag.shader_projected_nonfinite_count > 0 || diag.shader_joint_oob_count > 0 {
            lines.push(format!(
                "shader nonfinite/joint-oob: {}/{}",
                diag.shader_projected_nonfinite_count, diag.shader_joint_oob_count
            ));
        }
        if let Some(z_range) = &diag.ndc_z_range {
            lines.push(format!(
                "depth z: {z_range}  pass/reject: {}/{}",
                diag.depth_pass_estimate_count, diag.depth_reject_estimate_count
            ));
        }
        lines.push(format!(
            "alpha samples visible/zero: {}/{} of {}",
            diag.alpha_visible_sample_count, diag.alpha_zero_sample_count, diag.alpha_sample_count
        ));
        if let Some(alpha_range) = &diag.alpha_range {
            lines.push(format!("alpha range: {alpha_range}"));
        }
        if diag.uv_outside_sample_count > 0 {
            lines.push(format!(
                "uv outside samples: {}",
                diag.uv_outside_sample_count
            ));
        }
        if diag.skipped_reasons.is_empty() {
            lines.push("skipped reason: none reported".to_string());
        } else {
            lines.push("skipped / inspect:".to_string());
            lines.extend(
                diag.skipped_reasons
                    .iter()
                    .map(|reason| format!("- {reason}")),
            );
        }
        lines.join("\n")
    }

    fn humanoid_bones() -> &'static [&'static str] {
        &[
            "hips",
            "spine",
            "chest",
            "neck",
            "head",
            "shoulder_l",
            "upper_arm_l",
            "forearm_l",
            "hand_l",
            "shoulder_r",
            "upper_arm_r",
            "forearm_r",
            "hand_r",
            "upper_leg_l",
            "lower_leg_l",
            "foot_l",
            "toe_l",
            "upper_leg_r",
            "lower_leg_r",
            "foot_r",
            "toe_r",
        ]
    }

    fn default_bone_axes() -> HashMap<String, RetargetBoneAxis> {
        let mut axes = HashMap::new();
        axes.insert(
            "hips".to_string(),
            RetargetBoneAxis {
                turn: Some("rotationY:1".to_string()),
                ..Default::default()
            },
        );
        for bone in ["spine", "chest"] {
            axes.insert(
                bone.to_string(),
                RetargetBoneAxis {
                    turn: Some("rotationY:1".to_string()),
                    bend: Some("rotationX:-1".to_string()),
                    ..Default::default()
                },
            );
        }
        axes.insert(
            "head".to_string(),
            RetargetBoneAxis {
                turn: Some("rotationY:1".to_string()),
                ..Default::default()
            },
        );
        axes.insert(
            "upper_arm_l".to_string(),
            RetargetBoneAxis {
                forward: Some("rotationZ:1".to_string()),
                side: Some("rotationX:1".to_string()),
                twist: Some("rotationY:1".to_string()),
                ..Default::default()
            },
        );
        axes.insert(
            "upper_arm_r".to_string(),
            RetargetBoneAxis {
                forward: Some("rotationZ:-1".to_string()),
                side: Some("rotationX:1".to_string()),
                twist: Some("rotationY:1".to_string()),
                ..Default::default()
            },
        );
        for bone in ["forearm_l", "forearm_r"] {
            axes.insert(
                bone.to_string(),
                RetargetBoneAxis {
                    bend: Some("rotationX:-1".to_string()),
                    twist: Some("rotationY:1".to_string()),
                    ..Default::default()
                },
            );
        }
        for bone in ["hand_l", "hand_r"] {
            axes.insert(
                bone.to_string(),
                RetargetBoneAxis {
                    twist: Some("rotationY:-1".to_string()),
                    ..Default::default()
                },
            );
        }
        axes.insert(
            "upper_leg_l".to_string(),
            RetargetBoneAxis {
                forward: Some("rotationX:1".to_string()),
                side: Some("rotationZ:-1".to_string()),
                ..Default::default()
            },
        );
        axes.insert(
            "upper_leg_r".to_string(),
            RetargetBoneAxis {
                forward: Some("rotationX:1".to_string()),
                side: Some("rotationZ:1".to_string()),
                ..Default::default()
            },
        );
        for bone in ["lower_leg_l", "lower_leg_r"] {
            axes.insert(
                bone.to_string(),
                RetargetBoneAxis {
                    bend: Some("rotationX:1".to_string()),
                    ..Default::default()
                },
            );
        }
        for bone in ["foot_l", "foot_r"] {
            axes.insert(
                bone.to_string(),
                RetargetBoneAxis {
                    bend: Some("rotationX:-1".to_string()),
                    ..Default::default()
                },
            );
        }
        axes
    }

    fn sort_retarget_maps(maps: &[(String, String)]) -> Vec<(String, String)> {
        let order = Self::humanoid_bones()
            .iter()
            .enumerate()
            .map(|(index, bone)| (*bone, index))
            .collect::<HashMap<_, _>>();
        let mut sorted = maps.to_vec();
        sorted.sort_by(|(_, a), (_, b)| {
            order
                .get(a.as_str())
                .copied()
                .unwrap_or(usize::MAX)
                .cmp(&order.get(b.as_str()).copied().unwrap_or(usize::MAX))
                .then_with(|| a.cmp(b))
        });
        sorted
    }

    fn guess_humanoid_retarget_maps(joint_names: &[String]) -> Vec<(String, String)> {
        let candidates: &[(&str, &[&str])] = &[
            ("hips", &["hips", "pelvis"]),
            ("spine", &["spine"]),
            ("chest", &["chest", "upperchest", "thorax", "spine2"]),
            ("neck", &["neck"]),
            ("head", &["head"]),
            ("shoulder_l", &["leftshoulder", "lshoulder", "shoulderl"]),
            (
                "upper_arm_l",
                &[
                    "leftarm",
                    "leftupperarm",
                    "lupperarm",
                    "upperarml",
                    "upperarmleft",
                ],
            ),
            (
                "forearm_l",
                &[
                    "leftelbow",
                    "leftforearm",
                    "leftlowerarm",
                    "llowerarm",
                    "lowerarml",
                    "forearml",
                ],
            ),
            ("hand_l", &["leftwrist", "lefthand", "wristl", "handl"]),
            ("shoulder_r", &["rightshoulder", "rshoulder", "shoulderr"]),
            (
                "upper_arm_r",
                &[
                    "rightarm",
                    "rightupperarm",
                    "rupperarm",
                    "upperarmr",
                    "upperarmright",
                ],
            ),
            (
                "forearm_r",
                &[
                    "rightelbow",
                    "rightforearm",
                    "rightlowerarm",
                    "rlowerarm",
                    "lowerarmr",
                    "forearmr",
                ],
            ),
            ("hand_r", &["rightwrist", "righthand", "wristr", "handr"]),
            (
                "upper_leg_l",
                &[
                    "leftupleg",
                    "leftupperleg",
                    "leftthigh",
                    "upperlegl",
                    "thighl",
                ],
            ),
            (
                "lower_leg_l",
                &[
                    "leftleg",
                    "leftknee",
                    "leftlowerleg",
                    "leftshin",
                    "lowerlegl",
                    "calfl",
                ],
            ),
            ("foot_l", &["leftankle", "leftfoot", "anklel", "footl"]),
            ("toe_l", &["lefttoe", "toel"]),
            (
                "upper_leg_r",
                &[
                    "rightupleg",
                    "rightupperleg",
                    "rightthigh",
                    "upperlegr",
                    "thighr",
                ],
            ),
            (
                "lower_leg_r",
                &[
                    "rightleg",
                    "rightknee",
                    "rightlowerleg",
                    "rightshin",
                    "lowerlegr",
                    "calfr",
                ],
            ),
            ("foot_r", &["rightankle", "rightfoot", "ankler", "footr"]),
            ("toe_r", &["righttoe", "toer"]),
        ];
        let normalized = joint_names
            .iter()
            .map(|name| (name, Self::normalize_joint_name(name)))
            .collect::<Vec<_>>();
        let mut used = HashSet::<String>::new();
        let mut out = Vec::<(String, String)>::new();
        for (bone, patterns) in candidates {
            let Some((name, _)) = normalized
                .iter()
                .filter(|(name, normalized)| {
                    !used.contains(*name)
                        && !Self::is_likely_accessory_joint(normalized)
                        && Self::joint_name_matches_humanoid_bone(normalized, patterns)
                })
                .min_by_key(|(_, normalized)| Self::joint_match_score(normalized, patterns))
            else {
                continue;
            };
            used.insert((*name).clone());
            out.push(((*name).clone(), (*bone).to_string()));
        }
        out
    }

    fn joint_name_matches_humanoid_bone(normalized: &str, patterns: &[&str]) -> bool {
        patterns.iter().any(|pattern| normalized.contains(pattern))
    }

    fn joint_match_score(normalized: &str, patterns: &[&str]) -> usize {
        patterns
            .iter()
            .filter(|pattern| normalized.contains(**pattern))
            .map(|pattern| {
                if normalized == *pattern {
                    0
                } else if normalized.ends_with(*pattern) || normalized.starts_with(*pattern) {
                    1
                } else {
                    2
                }
            })
            .min()
            .unwrap_or(usize::MAX)
    }

    fn is_likely_accessory_joint(normalized: &str) -> bool {
        [
            "ribon", "ribbon", "twist", "hair", "skirt", "wing", "tail", "eye", "front", "kata",
            "sleeve",
        ]
        .iter()
        .any(|needle| normalized.contains(needle))
    }

    fn normalize_joint_name(name: &str) -> String {
        name.chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    }

    fn estimate_source_rest_pose_for_maps(
        raw_bones: &[RawBoneInfo],
        maps: &[(String, String)],
    ) -> SourceRestPose {
        let Some(shoulder_l) = Self::raw_position_for_canonical_in(raw_bones, maps, "shoulder_l")
        else {
            return SourceRestPose::ArmsDown;
        };
        let Some(shoulder_r) = Self::raw_position_for_canonical_in(raw_bones, maps, "shoulder_r")
        else {
            return SourceRestPose::ArmsDown;
        };
        let Some(hand_l) = Self::raw_position_for_canonical_in(raw_bones, maps, "hand_l") else {
            return SourceRestPose::ArmsDown;
        };
        let Some(hand_r) = Self::raw_position_for_canonical_in(raw_bones, maps, "hand_r") else {
            return SourceRestPose::ArmsDown;
        };
        let shoulder_y = (shoulder_l[1] + shoulder_r[1]) * 0.5;
        let hand_y = (hand_l[1] + hand_r[1]) * 0.5;
        let hand_drop = shoulder_y - hand_y;
        let shoulder_width = (shoulder_r[0] - shoulder_l[0]).abs().max(0.001);
        let hand_width = (hand_r[0] - hand_l[0]).abs();
        if hand_drop.abs() <= 0.10 && hand_width > shoulder_width * 1.8 {
            SourceRestPose::TPose
        } else if hand_drop <= 0.35 && hand_width > shoulder_width * 1.25 {
            SourceRestPose::APose
        } else {
            SourceRestPose::ArmsDown
        }
    }

    fn raw_position_for_canonical_in(
        raw_bones: &[RawBoneInfo],
        maps: &[(String, String)],
        canonical: &str,
    ) -> Option<[f32; 3]> {
        let raw = maps
            .iter()
            .find(|(_, to)| to == canonical)
            .map(|(from, _)| from)?;
        raw_bones
            .iter()
            .find(|bone| &bone.name == raw)
            .map(|bone| bone.position)
    }

    fn script_hash(script: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        script.hash(&mut hasher);
        hasher.finish()
    }

    fn render_image_from_bgra(width: u32, height: u32, bgra: Vec<u8>) -> Option<Arc<RenderImage>> {
        let image_buffer = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, bgra)?;
        let frames = SmallVec::from_elem(image::Frame::new(image_buffer), 1);
        Some(Arc::new(RenderImage::new(frames)))
    }

    fn generated_dsl(&self) -> String {
        self.generated_dsl_with_size(
            self.graph_width,
            self.graph_height,
            self.render_width,
            self.render_height,
        )
    }

    fn generated_preview_dsl(&self) -> String {
        let (width, height) = self.preview_render_size();
        self.generated_dsl_with_size(width, height, width, height)
    }

    fn preview_render_size(&self) -> (u32, u32) {
        let width = self.render_width.max(1);
        let height = self.render_height.max(1);
        let pixels = width.saturating_mul(height);
        if pixels <= MAX_PREVIEW_PIXELS {
            return (width, height);
        }

        let scale = (MAX_PREVIEW_PIXELS as f32 / pixels as f32).sqrt();
        let preview_width = ((width as f32 * scale).round() as u32).max(1);
        let preview_height = ((height as f32 * scale).round() as u32).max(1);
        (preview_width, preview_height)
    }

    fn generated_dsl_with_size(
        &self,
        graph_width: u32,
        graph_height: u32,
        render_width: u32,
        render_height: u32,
    ) -> String {
        let retarget = self.retarget_dsl();
        let bone_axis = self.bone_axis_dsl_for_axes_with_rest(
            &self.bone_axes,
            self.source_rest_pose,
            &self.rest_pose_corrections,
        );
        let additional_profile_blocks = self.additional_profile_blocks();
        let action_blocks = self.action_blocks_for_graph();
        let apply_action_blocks = self.apply_actions_for_graph();
        let model = Self::xml_attr_escape(&self.model_path);
        let hide_meshes = Self::xml_attr_escape(
            &self
                .sorted_hidden_meshes()
                .into_iter()
                .collect::<Vec<_>>()
                .join(","),
        );
        let hide_materials = Self::xml_attr_escape(
            &self
                .sorted_hidden_materials()
                .into_iter()
                .collect::<Vec<_>>()
                .join(","),
        );
        let background_block = self.background_dsl();
        let additional_actor_blocks = self.additional_actor_blocks();
        format!(
            r##"<Graph fps={{{fps}}} duration="{duration}s" size={{[{graph_width},{graph_height}]}} renderSize={{[{render_width},{render_height}]}}>
  <ModelProfile id="character_design_profile"
                model="{model}"
                preset="humanoid_v1">
{retarget}

{bone_axis}
  </ModelProfile>
{additional_profile_blocks}

  <World id="character_design_stage">
{background_block}

    <Camera id="design_camera"
            mode="orbit"
            projection="perspective"
            target="design_actor"
            x="{camera_x}"
            y="{camera_y}"
            z="{camera_z}"
            targetX="{camera_x}"
            targetY="{camera_y}"
            targetZ="{camera_z}"
            yaw="{camera_yaw}"
            pitch="{camera_pitch}"
            distance="{camera_distance}"
            fov="35" />

    <Actor id="design_actor"
           model="{model}"
           pathstyle="{pathstyle}"
           hideMeshes="{hide_meshes}"
           hideMaterials="{hide_materials}"
           profile="character_design_profile"
           x="{actor_x}"
           y="{actor_y}"
           z="{actor_z}"
           yaw="{actor_yaw}"
           pitch="{actor_pitch}"
           roll="{actor_roll}"
           scale="{actor_scale}"
           opacity="1">
      <Material style="toon" outline="true" outlineWidth="2" />
      <Play clip="Idle" loop="true" speed="1" />
    </Actor>
{additional_actor_blocks}
  </World>

{action_blocks}

{apply_action_blocks}

  <Present from="character_design_stage" />
</Graph>
"##,
            model = model,
            pathstyle = self.model_path_style(),
            hide_meshes = hide_meshes,
            hide_materials = hide_materials,
            background_block = background_block,
            retarget = retarget,
            bone_axis = bone_axis,
            additional_profile_blocks = additional_profile_blocks,
            action_blocks = action_blocks,
            apply_action_blocks = apply_action_blocks,
            fps = self.graph_fps,
            graph_width = graph_width,
            graph_height = graph_height,
            render_width = render_width,
            render_height = render_height,
            camera_x = Self::fmt(self.camera_x),
            camera_y = Self::fmt(self.camera_y),
            camera_z = Self::fmt(self.camera_z),
            camera_yaw = Self::fmt(self.camera_yaw),
            camera_pitch = Self::fmt(self.camera_pitch),
            camera_distance = Self::fmt(self.camera_distance),
            actor_x = Self::fmt(self.actor_x),
            actor_y = Self::fmt(self.actor_y),
            actor_z = Self::fmt(self.actor_z),
            actor_yaw = Self::fmt(self.actor_yaw),
            actor_pitch = Self::fmt(self.actor_pitch),
            actor_roll = Self::fmt(self.actor_roll),
            actor_scale = Self::fmt(self.actor_scale),
            additional_actor_blocks = additional_actor_blocks,
            duration = Self::fmt(self.duration_sec),
        )
    }

    fn background_dsl(&self) -> String {
        if self.background_image_path.trim().is_empty() {
            "    <Background id=\"design_bg\" color=\"#101827\" opacity=\"1\" />".to_string()
        } else {
            format!(
                "    <Background id=\"design_bg\"\n                src=\"{}\"\n                fit=\"cover\"\n                color=\"#101827\"\n                opacity=\"1\" />",
                Self::xml_attr_escape(&self.background_image_path)
            )
        }
    }

    fn additional_actor_blocks(&self) -> String {
        let mut blocks = String::new();
        for (index, path) in self.additional_model_paths.iter().enumerate() {
            if path.trim().is_empty() {
                continue;
            }
            let slot = index + 1;
            let actor_id = Self::actor_id_for_slot(slot);
            let profile_id = Self::profile_id_for_slot(slot);
            let model = Self::xml_attr_escape(path);
            let pathstyle = Self::path_style_for(path);
            let [x, y, z] = self
                .additional_actor_positions
                .get(index)
                .copied()
                .unwrap_or_else(|| Self::additional_actor_default_position(index));
            let x = Self::fmt(x);
            let y = Self::fmt(y);
            let z = Self::fmt(z);
            let [yaw, pitch, roll] = self
                .additional_actor_rotations
                .get(index)
                .copied()
                .unwrap_or_else(Self::additional_actor_default_rotation);
            let yaw = Self::fmt(yaw);
            let pitch = Self::fmt(pitch);
            let roll = Self::fmt(roll);
            let mut hidden_meshes = self
                .additional_hidden_meshes
                .get(index)
                .map(|hidden| hidden.iter().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            hidden_meshes.sort();
            let mut hidden_materials = self
                .additional_hidden_materials
                .get(index)
                .map(|hidden| hidden.iter().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            hidden_materials.sort();
            let hide_meshes = Self::xml_attr_escape(&hidden_meshes.join(","));
            let hide_materials = Self::xml_attr_escape(&hidden_materials.join(","));
            blocks.push_str(&format!(
                r#"

    <Actor id="{actor_id}"
           model="{model}"
           pathstyle="{pathstyle}"
           hideMeshes="{hide_meshes}"
           hideMaterials="{hide_materials}"
           profile="{profile_id}"
           x="{x}"
           y="{y}"
           z="{z}"
           yaw="{yaw}"
           pitch="{pitch}"
           roll="{roll}"
           scale="1"
           opacity="1">
      <Material style="toon" outline="true" outlineWidth="2" />
      <Play clip="Idle" loop="true" speed="1" />
    </Actor>"#
            ));
        }
        blocks
    }

    fn additional_actor_default_position(index: usize) -> [f32; 3] {
        let x_slots = [0.78, -0.78, 0.0];
        let x = x_slots[index % x_slots.len()];
        let z = -0.55 * ((index / x_slots.len()) as f32 + 1.0);
        [x, 0.0, z]
    }

    fn additional_actor_default_rotation() -> [f32; 3] {
        [-5.0, 0.0, 0.0]
    }

    fn additional_profile_blocks(&self) -> String {
        let mut blocks = String::new();
        for (index, path) in self.additional_model_paths.iter().enumerate() {
            let slot = index + 1;
            let profile_id = Self::profile_id_for_slot(slot);
            let model = Self::xml_attr_escape(path);
            let retarget = self
                .additional_retarget_maps
                .get(index)
                .map(|maps| Self::retarget_dsl_for_maps(maps))
                .unwrap_or_else(|| Self::retarget_dsl_for_maps(&[]));
            let axes = self
                .additional_bone_axes
                .get(index)
                .unwrap_or(&self.bone_axes);
            let source_rest_pose = self.source_rest_pose_for_slot(slot);
            let rest_pose_corrections = self.rest_pose_corrections_for_slot(slot);
            blocks.push_str(&self.model_profile_block(
                &profile_id,
                &model,
                &retarget,
                axes,
                source_rest_pose,
                rest_pose_corrections,
            ));
        }
        blocks
    }

    fn model_profile_block(
        &self,
        profile_id: &str,
        model: &str,
        retarget: &str,
        bone_axes: &HashMap<String, RetargetBoneAxis>,
        source_rest_pose: SourceRestPose,
        rest_pose_corrections: &HashMap<String, RestPoseSemanticCorrection>,
    ) -> String {
        let bone_axis = self.bone_axis_dsl_for_axes_with_rest(
            bone_axes,
            source_rest_pose,
            rest_pose_corrections,
        );
        format!(
            r##"
  <ModelProfile id="{profile_id}"
                model="{model}"
                preset="humanoid_v1">
{retarget}

{bone_axis}
  </ModelProfile>"##,
            profile_id = Self::xml_attr_escape(profile_id),
            model = model,
            retarget = retarget,
            bone_axis = bone_axis,
        )
    }

    fn generated_action_block_for(
        &self,
        action: &WaveActionControls,
        raw_pose: &str,
        action_id: &str,
    ) -> String {
        let a = action;
        format!(
            r##"  <Action id="{action_id}" skeleton="humanoid_v1" duration="{duration}s" intent="pose">
    <Pose t="0.0" label="pose">
      <Bone id="hips" turn="0" />
      <Bone id="chest" turn="{chest_turn}" bend="0" />
      <Bone id="head" turn="{head_turn}" />
      <Bone id="upper_arm_r" forward="{raise_forward}" side="{raise_side}" twist="0" />
      <Bone id="forearm_r" bend="{elbow_bend}" twist="{wave_twist}" />
      <Bone id="hand_r" twist="{wave_twist}" />
      <Bone id="upper_leg_l" forward="{left_leg_forward}" />
      <Bone id="lower_leg_l" bend="{left_knee_bend}" />
      <Bone id="upper_leg_r" forward="{right_leg_forward}" />
      <Bone id="lower_leg_r" bend="{right_knee_bend}" />
{raw_pose}    </Pose>
  </Action>"##,
            action_id = Self::xml_attr_escape(action_id),
            raw_pose = raw_pose,
            raise_forward = Self::fmt(a.raise_forward),
            raise_side = Self::fmt(a.raise_side),
            elbow_bend = Self::fmt(a.elbow_bend),
            wave_twist = Self::fmt(a.wave_twist),
            left_leg_forward = Self::fmt(a.left_leg_forward),
            left_knee_bend = Self::fmt(a.left_knee_bend),
            right_leg_forward = Self::fmt(a.right_leg_forward),
            right_knee_bend = Self::fmt(a.right_knee_bend),
            chest_turn = Self::fmt(a.chest_turn),
            head_turn = Self::fmt(a.head_turn),
            duration = Self::fmt(self.duration_sec),
        )
    }

    fn rewrite_action_id(action: &str, action_id: &str) -> String {
        let Some(action_start) = action.find("<Action") else {
            return action.to_string();
        };
        let Some(open_end_rel) = action[action_start..].find('>') else {
            return action.to_string();
        };
        let open_end = action_start + open_end_rel;
        let opening = &action[action_start..=open_end];
        let escaped_id = Self::xml_attr_escape(action_id);
        let updated_opening = if let Some(id_rel) = opening.find("id=\"") {
            let id_start = id_rel + 4;
            if let Some(id_end_rel) = opening[id_start..].find('"') {
                let id_end = id_start + id_end_rel;
                format!(
                    "{}{}{}",
                    &opening[..id_start],
                    escaped_id,
                    &opening[id_end..]
                )
            } else {
                opening.to_string()
            }
        } else {
            format!(
                "{} id=\"{}\"{}",
                &opening[..opening.len() - 1],
                escaped_id,
                ">"
            )
        };

        format!(
            "{}{}{}",
            &action[..action_start],
            updated_opening,
            &action[open_end + 1..]
        )
    }

    fn action_block_for_slot(&self, slot: usize) -> String {
        let action_id = Self::action_id_for_slot(slot);
        let custom = self.action_dsl_for_slot(slot).trim();
        if custom.is_empty() {
            let raw_pose = self.raw_pose_dsl_for_slot(slot);
            self.generated_action_block_for(self.action_for_slot(slot), &raw_pose, &action_id)
        } else {
            Self::rewrite_action_id(custom, &action_id)
        }
    }

    fn action_block_for_test_preset(preset: RetargetPosePreset, action_id: &str) -> Option<String> {
        let poses = preset.action_poses()?;
        Some(format!(
            r#"  <Action id="{action_id}" skeleton="humanoid_v1" duration="{duration}" intent="pose">
{poses}
  </Action>"#,
            action_id = Self::xml_attr_escape(action_id),
            duration = preset.action_duration(),
            poses = poses
        ))
    }

    fn apply_test_action_preset(
        &mut self,
        preset: RetargetPosePreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.capture_snapshot();
        self.pose_calibration_preset = None;
        self.set_active_test_action_preset(preset);
        self.frame = preset
            .preview_frame()
            .min(self.total_frames().saturating_sub(1));
        let axis_bone = self.axis_bone_for_new_preset(preset);
        self.set_active_selected_canonical_bone(Some(axis_bone.clone()));
        let action_id = Self::action_id_for_slot(self.selected_actor_slot);
        let text = Self::action_block_for_test_preset(preset, &action_id).unwrap_or_default();
        *self.active_action_dsl_mut() = text.clone();
        self.set_active_selected_action_keyframe_t(None);
        self.apply_action_dsl_pose_to_controls();
        if let Some(input) = self.action_dsl_input.as_ref() {
            self.action_dsl_input_syncing = true;
            input.update(cx, |this, cx| {
                this.set_value(text, window, cx);
            });
            self.action_dsl_input_syncing = false;
        }
        self.status_line = if preset.is_axis_debug() {
            format!(
                "{} raw axis test: {}. Editing BoneAxis {}.",
                self.selected_actor_label(),
                preset.label(),
                axis_bone
            )
        } else if preset == RetargetPosePreset::Original {
            format!(
                "{} uses current generated pose/action. Editing BoneAxis {}.",
                self.selected_actor_label(),
                axis_bone
            )
        } else {
            format!(
                "{} test action: {}. Editing BoneAxis {}.",
                self.selected_actor_label(),
                preset.label(),
                axis_bone
            )
        };
        self.invalidate_preview();
        self.push_undo_snapshot_if_changed(before);
    }

    fn action_blocks_for_graph(&self) -> String {
        let mut blocks = Vec::with_capacity(self.additional_model_paths.len() + 1);
        for slot in 0..=self.additional_model_paths.len() {
            blocks.push(self.action_block_for_slot(slot));
        }
        blocks.join("\n\n")
    }

    fn apply_actions_for_graph(&self) -> String {
        let mut blocks = Vec::with_capacity(self.additional_model_paths.len() + 1);
        for slot in 0..=self.additional_model_paths.len() {
            blocks.push(format!(
                r#"  <ApplyAction target="{target}"
               action="{action_id}"
               at="0s"
               loop="true"
               weight="1" />"#,
                target = Self::xml_attr_escape(&Self::actor_id_for_slot(slot)),
                action_id = Self::xml_attr_escape(&Self::action_id_for_slot(slot)),
            ));
        }
        blocks.join("\n\n")
    }

    fn tag_attr_value(tag: &str, name: &str) -> Option<String> {
        for quote in ['"', '\''] {
            let needle = format!("{name}={quote}");
            let Some(start) = tag.find(&needle).map(|start| start + needle.len()) else {
                continue;
            };
            let rest = &tag[start..];
            let Some(end) = rest.find(quote) else {
                continue;
            };
            let value = rest[..end].trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
        None
    }

    fn tag_attr_f32(tag: &str, name: &str) -> Option<f32> {
        Self::tag_attr_value(tag, name)?.parse::<f32>().ok()
    }

    fn action_bone_numeric_attrs() -> &'static [&'static str] {
        &[
            "x",
            "y",
            "z",
            "rotation",
            "rotationX",
            "rotationY",
            "rotationZ",
            "forward",
            "side",
            "twist",
            "bend",
            "turn",
            "scale",
            "opacity",
        ]
    }

    fn parse_action_duration_seconds(action: &str) -> Option<f32> {
        let action_start = action.find("<Action")?;
        let action_rest = &action[action_start..];
        let tag_end = action_rest.find('>')?;
        let tag = &action_rest[..tag_end];
        let duration = Self::tag_attr_value(tag, "duration")?;
        let seconds = duration.strip_suffix('s').unwrap_or(duration.as_str());
        seconds.trim().parse::<f32>().ok()
    }

    fn action_pose_blocks(action: &str) -> Vec<(f32, String)> {
        let mut poses = Vec::new();
        let mut search_from = 0;
        while let Some(rel_start) = action[search_from..].find("<Pose") {
            let start = search_from + rel_start;
            let Some(open_end_rel) = action[start..].find('>') else {
                break;
            };
            let open_end = start + open_end_rel;
            let pose_tag = &action[start..=open_end];
            let t = Self::tag_attr_f32(pose_tag, "t").unwrap_or(0.0);
            let body_start = open_end + 1;
            let Some(close_rel) = action[body_start..].find("</Pose>") else {
                break;
            };
            let body_end = body_start + close_rel;
            poses.push((t, action[body_start..body_end].to_string()));
            search_from = body_end + "</Pose>".len();
        }
        poses
    }

    fn action_keyframes(&self) -> Vec<f32> {
        let mut times = Self::action_pose_blocks(self.active_action_dsl())
            .into_iter()
            .map(|(t, _)| t)
            .collect::<Vec<_>>();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        times.dedup_by(|a, b| (*a - *b).abs() < 0.001);
        times
    }

    fn total_frames(&self) -> u32 {
        ((self.duration_sec.max(0.03) * self.graph_fps.max(1) as f32).round() as u32).max(1)
    }

    fn clamp_frame_to_duration(&mut self) {
        self.frame = self.frame.min(self.total_frames().saturating_sub(1));
    }

    fn frame_time_seconds(&self) -> f32 {
        self.frame as f32 / self.graph_fps.max(1) as f32
    }

    fn set_frame_from_input(&mut self, raw: &str) {
        let Some(frame) = raw.trim().parse::<u32>().ok() else {
            self.status_line = "Frame input must be an integer.".to_string();
            return;
        };
        self.frame = frame.min(self.total_frames().saturating_sub(1));
        self.set_active_selected_action_keyframe_t(None);
        self.apply_action_dsl_pose_to_controls();
        self.invalidate_preview();
        self.status_line = format!("Jumped to frame {}.", self.frame);
    }

    fn replace_action_duration(action: &str, duration_sec: f32) -> String {
        let Some(action_start) = action.find("<Action") else {
            return action.to_string();
        };
        let Some(tag_end_rel) = action[action_start..].find('>') else {
            return action.to_string();
        };
        let tag_end = action_start + tag_end_rel;
        let tag = &action[action_start..tag_end];
        let duration = format!("duration=\"{}s\"", Self::fmt(duration_sec));

        if let Some(attr_start_rel) = tag.find("duration=") {
            let attr_start = action_start + attr_start_rel;
            let attr_value_start = attr_start + "duration=".len();
            let quote = action[attr_value_start..]
                .chars()
                .next()
                .filter(|quote| *quote == '"' || *quote == '\'');
            if let Some(quote) = quote {
                if let Some(attr_end_rel) = action[attr_value_start + 1..].find(quote) {
                    let attr_end = attr_value_start + 1 + attr_end_rel + 1;
                    let mut updated = String::with_capacity(action.len() + 8);
                    updated.push_str(&action[..attr_start]);
                    updated.push_str(&duration);
                    updated.push_str(&action[attr_end..]);
                    return updated;
                }
            }
        }

        let mut updated = String::with_capacity(action.len() + duration.len() + 1);
        updated.push_str(&action[..tag_end]);
        updated.push(' ');
        updated.push_str(&duration);
        updated.push_str(&action[tag_end..]);
        updated
    }

    fn set_duration_from_input(&mut self, raw: &str) {
        let Some(seconds) = raw.trim().parse::<f32>().ok() else {
            self.status_line = "Duration input must be a number of seconds.".to_string();
            return;
        };
        let seconds = ((seconds.clamp(0.03, 600.0) * 100.0).round()) / 100.0;
        self.duration_sec = seconds;
        self.clamp_frame_to_duration();
        if !self.action_dsl.trim().is_empty() {
            self.action_dsl = Self::replace_action_duration(&self.action_dsl, seconds);
        }
        for action_dsl in &mut self.additional_action_dsls {
            if !action_dsl.trim().is_empty() {
                *action_dsl = Self::replace_action_duration(action_dsl, seconds);
            }
        }
        self.set_active_selected_action_keyframe_t(None);
        self.apply_action_dsl_pose_to_controls();
        self.invalidate_preview();
        self.status_line = format!("Set action duration to {}s.", Self::fmt(seconds));
    }

    fn set_graph_fps_from_input(&mut self, raw: &str) {
        let Some(fps) = raw.trim().parse::<u32>().ok() else {
            self.status_line = "FPS input must be an integer.".to_string();
            return;
        };
        self.graph_fps = fps.clamp(1, 240);
        self.clamp_frame_to_duration();
        self.set_active_selected_action_keyframe_t(None);
        self.apply_action_dsl_pose_to_controls();
        self.invalidate_preview();
        self.status_line = format!("Set Graph FPS to {}.", self.graph_fps);
    }

    fn set_graph_dimension_from_input(&mut self, name: &str, raw: &str) {
        let Some(value) = raw.trim().parse::<u32>().ok() else {
            self.status_line = format!("{name} input must be an integer.");
            return;
        };
        let value = value.clamp(1, 8192);
        match name {
            "size width" => self.graph_width = value,
            "size height" => self.graph_height = value,
            "render width" => self.render_width = value,
            "render height" => self.render_height = value,
            _ => return,
        }
        self.invalidate_preview();
        self.status_line = format!("Set Graph {name} to {value}.");
    }

    fn schedule_graph_input_apply(
        &mut self,
        field: &'static str,
        text: String,
        cx: &mut Context<Self>,
    ) {
        self.graph_input_token = self.graph_input_token.wrapping_add(1);
        let token = self.graph_input_token;
        cx.spawn(async move |view, cx| {
            Timer::after(Duration::from_millis(1500)).await;
            let _ = view.update(cx, |this, cx| {
                if this.graph_input_token != token {
                    return;
                }
                match field {
                    "fps" => this.set_graph_fps_from_input(&text),
                    "size width" | "size height" | "render width" | "render height" => {
                        this.set_graph_dimension_from_input(field, &text)
                    }
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn current_action_time(&self, action: &str) -> f32 {
        if let Some(t) = self.active_selected_action_keyframe_t() {
            return t;
        }
        let mut target_t = self.frame_time_seconds();
        if let Some(duration) = Self::parse_action_duration_seconds(action)
            && duration > 0.001
            && target_t > duration
        {
            target_t %= duration;
        }
        target_t
    }

    fn nearest_pose_body_range(action: &str, target_t: f32) -> Option<(f32, usize, usize)> {
        let mut search_from = 0;
        let mut best: Option<(f32, f32, usize, usize)> = None;
        while let Some(rel_start) = action[search_from..].find("<Pose") {
            let start = search_from + rel_start;
            let Some(open_end_rel) = action[start..].find('>') else {
                break;
            };
            let open_end = start + open_end_rel;
            let pose_tag = &action[start..=open_end];
            let t = Self::tag_attr_f32(pose_tag, "t").unwrap_or(0.0);
            let body_start = open_end + 1;
            let Some(close_rel) = action[body_start..].find("</Pose>") else {
                break;
            };
            let body_end = body_start + close_rel;
            let distance = (t - target_t).abs();
            if best
                .as_ref()
                .map(|(best_distance, _, _, _)| distance < *best_distance)
                .unwrap_or(true)
            {
                best = Some((distance, t, body_start, body_end));
            }
            search_from = body_end + "</Pose>".len();
        }
        best.map(|(_, t, body_start, body_end)| (t, body_start, body_end))
    }

    fn parse_action_pose_bones(body: &str) -> Vec<ActionPoseBoneAttrs> {
        let mut bones = Vec::new();
        let mut search_from = 0;
        while let Some(rel_start) = body[search_from..].find("<Bone") {
            let start = search_from + rel_start;
            let Some(end_rel) = body[start..].find("/>") else {
                break;
            };
            let end = start + end_rel + 2;
            let tag = &body[start..end];
            let Some(id) = Self::tag_attr_value(tag, "id") else {
                search_from = end;
                continue;
            };
            let attrs = Self::action_bone_numeric_attrs()
                .iter()
                .filter_map(|attr| {
                    Self::tag_attr_value(tag, attr).map(|value| ((*attr).to_string(), value))
                })
                .collect::<HashMap<_, _>>();
            bones.push(ActionPoseBoneAttrs { id, attrs });
            search_from = end;
        }
        bones
    }

    fn interpolated_action_pose_body(action: &str, target_t: f32) -> Option<String> {
        let mut poses = Self::action_pose_blocks(action);
        if poses.is_empty() {
            return None;
        }
        poses.sort_by(|(a, _), (b, _)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let first = poses.first()?.clone();
        let last = poses.last()?.clone();
        let (before, after) = if target_t <= first.0 {
            (first.clone(), first)
        } else if target_t >= last.0 {
            (last.clone(), last)
        } else {
            let mut pair = (first.clone(), first);
            for window in poses.windows(2) {
                let a = window[0].clone();
                let b = window[1].clone();
                if target_t >= a.0 && target_t <= b.0 {
                    pair = (a, b);
                    break;
                }
            }
            pair
        };

        let alpha = if (after.0 - before.0).abs() <= f32::EPSILON {
            0.0
        } else {
            ((target_t - before.0) / (after.0 - before.0)).clamp(0.0, 1.0)
        };
        Some(Self::interpolate_action_pose_bodies(
            &before.1, &after.1, alpha,
        ))
    }

    fn interpolate_action_pose_bodies(before_body: &str, after_body: &str, alpha: f32) -> String {
        let before_bones = Self::parse_action_pose_bones(before_body);
        let after_bones = Self::parse_action_pose_bones(after_body);
        let before_lookup = before_bones
            .iter()
            .map(|bone| (bone.id.as_str(), bone))
            .collect::<HashMap<_, _>>();
        let after_lookup = after_bones
            .iter()
            .map(|bone| (bone.id.as_str(), bone))
            .collect::<HashMap<_, _>>();

        let mut ids = before_bones
            .iter()
            .map(|bone| bone.id.as_str())
            .collect::<Vec<_>>();
        for bone in &after_bones {
            if !ids.contains(&bone.id.as_str()) {
                ids.push(bone.id.as_str());
            }
        }

        let mut out = String::new();
        for id in ids {
            let before = before_lookup.get(id).copied();
            let after = after_lookup.get(id).copied();
            out.push_str("\n      <Bone id=\"");
            out.push_str(&Self::xml_attr_escape(id));
            out.push('"');

            for attr in Self::action_bone_numeric_attrs() {
                let before_value = before.and_then(|bone| bone.attrs.get(*attr));
                let after_value = after.and_then(|bone| bone.attrs.get(*attr));
                let Some(value) = Self::interpolate_action_attr(before_value, after_value, alpha)
                else {
                    continue;
                };
                out.push(' ');
                out.push_str(attr);
                out.push_str("=\"");
                out.push_str(&Self::xml_attr_escape(&value));
                out.push('"');
            }

            out.push_str(" />");
        }
        out.push('\n');
        out
    }

    fn interpolate_action_attr(
        before: Option<&String>,
        after: Option<&String>,
        alpha: f32,
    ) -> Option<String> {
        match (before, after) {
            (Some(a), Some(b)) => match (a.parse::<f32>(), b.parse::<f32>()) {
                (Ok(av), Ok(bv)) => Some(Self::fmt(av + (bv - av) * alpha)),
                _ if alpha < 0.5 => Some(a.clone()),
                _ => Some(b.clone()),
            },
            (Some(value), None) | (None, Some(value)) => Some(value.clone()),
            (None, None) => None,
        }
    }

    fn ensure_action_dsl_for_keyframes(&mut self) {
        let action = self.active_action_dsl().to_string();
        if action.trim().is_empty() || !action.contains("</Action>") {
            let generated = self.action_block_for_slot(self.selected_actor_slot);
            *self.active_action_dsl_mut() = generated;
        }
    }

    fn add_keyframe_at_current_frame(&mut self) {
        self.ensure_action_dsl_for_keyframes();
        let target_t = self.frame_time_seconds();
        self.set_active_selected_action_keyframe_t(Some(target_t));
        let action = self.active_action_dsl().to_string();
        if let Some((existing_t, _, _)) = Self::nearest_pose_body_range(&action, target_t)
            && (existing_t - target_t).abs() < 0.001
        {
            self.sync_action_dsl_from_current_pose_if_active();
            self.status_line = format!(
                "Updated existing keyframe t={} from current pose.",
                Self::fmt(existing_t)
            );
            return;
        }

        let Some(action_close) = action.find("</Action>") else {
            let generated = self.action_block_for_slot(self.selected_actor_slot);
            *self.active_action_dsl_mut() = generated;
            self.status_line = "Created a new Action DSL from current pose.".to_string();
            return;
        };
        let pose_label = format!("key_{}", Self::fmt(target_t).replace('.', "_"));
        let pose_block = format!(
            "\n    <Pose t=\"{}\" label=\"{}\">{}    </Pose>\n",
            Self::fmt(target_t),
            pose_label,
            self.current_pose_body_dsl(),
        );

        let mut insert_at = action_close;
        let mut search_from = 0;
        while let Some(rel_start) = action[search_from..].find("<Pose") {
            let start = search_from + rel_start;
            let Some(open_end_rel) = action[start..].find('>') else {
                break;
            };
            let open_end = start + open_end_rel;
            let pose_tag = &action[start..=open_end];
            if Self::tag_attr_f32(pose_tag, "t").unwrap_or(0.0) > target_t {
                insert_at = start;
                break;
            }
            let Some(close_rel) = action[open_end + 1..].find("</Pose>") else {
                break;
            };
            search_from = open_end + 1 + close_rel + "</Pose>".len();
        }

        let mut updated = action;
        updated.insert_str(insert_at, &pose_block);
        *self.active_action_dsl_mut() = updated;
        self.status_line = format!(
            "Added keyframe t={} from current pose.",
            Self::fmt(target_t)
        );
    }

    fn actor_position_for_slot(&self, slot: usize) -> [f32; 3] {
        if slot == 0 {
            [self.actor_x, self.actor_y, self.actor_z]
        } else {
            self.additional_actor_positions
                .get(slot - 1)
                .copied()
                .unwrap_or_else(|| Self::additional_actor_default_position(slot - 1))
        }
    }

    fn active_actor_position(&self) -> [f32; 3] {
        self.actor_position_for_slot(self.selected_actor_slot)
    }

    fn actor_rotation_for_slot(&self, slot: usize) -> [f32; 3] {
        if slot == 0 {
            [self.actor_yaw, self.actor_pitch, self.actor_roll]
        } else {
            self.additional_actor_rotations
                .get(slot - 1)
                .copied()
                .unwrap_or_else(Self::additional_actor_default_rotation)
        }
    }

    fn active_actor_rotation(&self) -> [f32; 3] {
        self.actor_rotation_for_slot(self.selected_actor_slot)
    }

    fn adjust_active_actor_position(&mut self, axis: usize, delta: f32) {
        self.normalize_additional_model_state();
        let axis = axis.min(2);
        if self.selected_actor_slot == 0 {
            let value = match axis {
                0 => &mut self.actor_x,
                1 => &mut self.actor_y,
                _ => &mut self.actor_z,
            };
            *value = (*value + delta).clamp(-20.0, 20.0);
        } else {
            let position = &mut self.additional_actor_positions[self.selected_actor_slot - 1];
            position[axis] = (position[axis] + delta).clamp(-20.0, 20.0);
        }
        self.status_line = format!(
            "Moved {} to x {}, y {}, z {}.",
            self.selected_actor_label(),
            Self::fmt(self.active_actor_position()[0]),
            Self::fmt(self.active_actor_position()[1]),
            Self::fmt(self.active_actor_position()[2])
        );
    }

    fn adjust_active_actor_rotation(&mut self, axis: usize, delta: f32) {
        self.normalize_additional_model_state();
        let axis = axis.min(2);
        if self.selected_actor_slot == 0 {
            let value = match axis {
                0 => &mut self.actor_yaw,
                1 => &mut self.actor_pitch,
                _ => &mut self.actor_roll,
            };
            *value = (*value + delta).rem_euclid(360.0);
        } else {
            let rotation = &mut self.additional_actor_rotations[self.selected_actor_slot - 1];
            rotation[axis] = (rotation[axis] + delta).rem_euclid(360.0);
        }
        let rotation = self.active_actor_rotation();
        self.status_line = format!(
            "Rotated {} to yaw {}, pitch {}, roll {}.",
            self.selected_actor_label(),
            Self::fmt(rotation[0]),
            Self::fmt(rotation[1]),
            Self::fmt(rotation[2])
        );
    }

    fn select_action_keyframe(&mut self, t: f32) {
        self.set_active_selected_action_keyframe_t(Some(t));
        self.frame = ((t * self.graph_fps.max(1) as f32).round() as u32)
            .min(self.total_frames().saturating_sub(1));
        self.apply_action_dsl_pose_to_controls();
        self.status_line = format!(
            "Selected keyframe t={}. Drag handles then Update Keyframe.",
            Self::fmt(t)
        );
        self.invalidate_preview();
    }

    fn update_selected_action_keyframe(&mut self) {
        self.ensure_action_dsl_for_keyframes();
        if self.active_selected_action_keyframe_t().is_none() {
            let target_t = self.frame_time_seconds();
            let nearest = Self::nearest_pose_body_range(self.active_action_dsl(), target_t)
                .map(|(t, _, _)| t)
                .or(Some(target_t));
            self.set_active_selected_action_keyframe_t(nearest);
        }
        self.sync_action_dsl_from_current_pose_if_active();
        let t = self
            .active_selected_action_keyframe_t()
            .map(Self::fmt)
            .unwrap_or_else(|| Self::fmt(self.frame_time_seconds()));
        self.status_line = format!("Updated keyframe t={t} from current pose.");
    }

    fn current_pose_body_dsl(&self) -> String {
        let raw_pose = self.raw_pose_dsl();
        let generated = self.generated_action_block_for(
            self.active_action(),
            &raw_pose,
            &Self::action_id_for_slot(self.selected_actor_slot),
        );
        Self::action_pose_blocks(&generated)
            .into_iter()
            .next()
            .map(|(_, body)| body)
            .unwrap_or_else(|| {
                "\n      <!-- Failed to generate current pose. -->\n    ".to_string()
            })
    }

    fn apply_bone_tag_to_controls(&mut self, tag: &str) {
        let Some(id) = Self::tag_attr_value(tag, "id") else {
            return;
        };
        let action = self.active_action_mut();
        match id.as_str() {
            "chest" => {
                if let Some(value) = Self::tag_attr_f32(tag, "turn") {
                    action.chest_turn = value;
                }
            }
            "head" => {
                if let Some(value) = Self::tag_attr_f32(tag, "turn") {
                    action.head_turn = value;
                }
            }
            "upper_arm_r" => {
                if let Some(value) = Self::tag_attr_f32(tag, "forward") {
                    action.raise_forward = value;
                }
                if let Some(value) = Self::tag_attr_f32(tag, "side") {
                    action.raise_side = value;
                }
            }
            "forearm_r" => {
                if let Some(value) = Self::tag_attr_f32(tag, "bend") {
                    action.elbow_bend = value;
                }
                if let Some(value) = Self::tag_attr_f32(tag, "twist") {
                    action.wave_twist = value;
                }
            }
            "hand_r" => {
                if let Some(value) = Self::tag_attr_f32(tag, "twist") {
                    action.wave_twist = value;
                }
            }
            "upper_leg_l" => {
                if let Some(value) = Self::tag_attr_f32(tag, "forward") {
                    action.left_leg_forward = value;
                }
            }
            "lower_leg_l" => {
                if let Some(value) = Self::tag_attr_f32(tag, "bend") {
                    action.left_knee_bend = value;
                }
            }
            "upper_leg_r" => {
                if let Some(value) = Self::tag_attr_f32(tag, "forward") {
                    action.right_leg_forward = value;
                }
            }
            "lower_leg_r" => {
                if let Some(value) = Self::tag_attr_f32(tag, "bend") {
                    action.right_knee_bend = value;
                }
            }
            _ => {}
        }

        let rotation_x = Self::tag_attr_f32(tag, "rotationX");
        let rotation_y = Self::tag_attr_f32(tag, "rotationY");
        let rotation_z = Self::tag_attr_f32(tag, "rotationZ");
        if rotation_x.is_none() && rotation_y.is_none() && rotation_z.is_none() {
            return;
        }
        let raw_name = self
            .mapped_raw_for_canonical(&id)
            .map(str::to_string)
            .unwrap_or(id);
        let rotation = self
            .active_raw_bone_rotations_mut()
            .entry(raw_name)
            .or_insert([0.0, 0.0, 0.0]);
        if let Some(value) = rotation_x {
            rotation[0] = value;
        }
        if let Some(value) = rotation_y {
            rotation[1] = value;
        }
        if let Some(value) = rotation_z {
            rotation[2] = value;
        }
    }

    fn apply_action_dsl_pose_to_controls(&mut self) {
        let action = self.active_action_dsl().trim().to_string();
        if !action.contains("</Action>") {
            return;
        }
        let poses = Self::action_pose_blocks(&action);
        if poses.is_empty() {
            return;
        }
        let target_t = self.current_action_time(&action);
        let Some(body) = Self::interpolated_action_pose_body(&action, target_t) else {
            return;
        };

        *self.active_action_mut() = WaveActionControls::default();
        self.active_raw_bone_rotations_mut().clear();

        let mut search_from = 0;
        while let Some(rel_start) = body[search_from..].find("<Bone") {
            let start = search_from + rel_start;
            let Some(end_rel) = body[start..].find("/>") else {
                break;
            };
            let end = start + end_rel + 2;
            self.apply_bone_tag_to_controls(&body[start..end]);
            search_from = end;
        }
    }

    fn sync_action_dsl_from_current_pose_if_active(&mut self) {
        let action = self.active_action_dsl().to_string();
        if action.trim().is_empty() {
            return;
        }

        let target_t = self.current_action_time(&action);

        let Some((t, body_start, body_end)) = Self::nearest_pose_body_range(&action, target_t)
        else {
            let generated = self.action_block_for_slot(self.selected_actor_slot);
            *self.active_action_dsl_mut() = generated;
            return;
        };
        if self.active_selected_action_keyframe_t().is_none() {
            self.set_active_selected_action_keyframe_t(Some(t));
        }

        let mut updated = String::with_capacity(action.len() + 512);
        updated.push_str(&action[..body_start]);
        updated.push_str(&self.current_pose_body_dsl());
        updated.push_str(&action[body_end..]);
        *self.active_action_dsl_mut() = updated;
    }

    fn ensure_action_dsl_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.action_dsl_input.is_some() {
            return;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("xml")
                .rows(14)
                .line_number(true)
                .soft_wrap(true)
                .placeholder("<Action id=\"custom_action\" skeleton=\"humanoid_v1\">...</Action>")
        });
        let initial = self.active_action_dsl().to_string();
        input.update(cx, |this, cx| {
            this.set_value(initial, window, cx);
        });
        let sub = cx.subscribe(&input, |this, input, ev, cx| match ev {
            InputEvent::Change | InputEvent::PressEnter { .. } => {
                if this.action_dsl_input_syncing {
                    return;
                }
                *this.active_action_dsl_mut() = input.read(cx).value().to_string();
                this.set_active_selected_action_keyframe_t(None);
                this.apply_action_dsl_pose_to_controls();
                this.invalidate_preview();
                cx.notify();
            }
            _ => {}
        });
        self.action_dsl_input = Some(input);
        self.action_dsl_input_sub = Some(sub);
    }

    fn sync_action_dsl_input_if_needed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.action_dsl_input.as_ref() else {
            return;
        };
        let text = self.active_action_dsl().to_string();
        if input.read(cx).value().to_string() == text {
            return;
        }
        self.action_dsl_input_syncing = true;
        input.update(cx, |this, cx| {
            this.set_value(text, window, cx);
        });
        self.action_dsl_input_syncing = false;
    }

    fn ensure_timeline_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.frame_input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("frame"));
            let initial = self.frame.to_string();
            input.update(cx, |this, cx| {
                this.set_value(initial, window, cx);
            });
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::PressEnter { .. }) || this.timeline_input_syncing {
                    return;
                }
                let text = input.read(cx).value().to_string();
                this.set_frame_from_input(&text);
                cx.notify();
            });
            self.frame_input = Some(input);
            self.frame_input_sub = Some(sub);
        }

        if self.duration_input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("seconds"));
            let initial = Self::fmt(self.duration_sec);
            input.update(cx, |this, cx| {
                this.set_value(initial, window, cx);
            });
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::PressEnter { .. }) || this.timeline_input_syncing {
                    return;
                }
                let text = input.read(cx).value().to_string();
                this.set_duration_from_input(&text);
                cx.notify();
            });
            self.duration_input = Some(input);
            self.duration_input_sub = Some(sub);
        }

        if self.graph_fps_input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("fps"));
            let initial = self.graph_fps.to_string();
            input.update(cx, |this, cx| {
                this.set_value(initial, window, cx);
            });
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change | InputEvent::PressEnter { .. })
                    || this.timeline_input_syncing
                {
                    return;
                }
                let text = input.read(cx).value().to_string();
                this.schedule_graph_input_apply("fps", text, cx);
            });
            self.graph_fps_input = Some(input);
            self.graph_fps_input_sub = Some(sub);
        }

        if self.graph_width_input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("width"));
            let initial = self.graph_width.to_string();
            input.update(cx, |this, cx| {
                this.set_value(initial, window, cx);
            });
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change | InputEvent::PressEnter { .. })
                    || this.timeline_input_syncing
                {
                    return;
                }
                let text = input.read(cx).value().to_string();
                this.schedule_graph_input_apply("size width", text, cx);
            });
            self.graph_width_input = Some(input);
            self.graph_width_input_sub = Some(sub);
        }

        if self.graph_height_input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("height"));
            let initial = self.graph_height.to_string();
            input.update(cx, |this, cx| {
                this.set_value(initial, window, cx);
            });
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change | InputEvent::PressEnter { .. })
                    || this.timeline_input_syncing
                {
                    return;
                }
                let text = input.read(cx).value().to_string();
                this.schedule_graph_input_apply("size height", text, cx);
            });
            self.graph_height_input = Some(input);
            self.graph_height_input_sub = Some(sub);
        }

        if self.render_width_input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("render w"));
            let initial = self.render_width.to_string();
            input.update(cx, |this, cx| {
                this.set_value(initial, window, cx);
            });
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change | InputEvent::PressEnter { .. })
                    || this.timeline_input_syncing
                {
                    return;
                }
                let text = input.read(cx).value().to_string();
                this.schedule_graph_input_apply("render width", text, cx);
            });
            self.render_width_input = Some(input);
            self.render_width_input_sub = Some(sub);
        }

        if self.render_height_input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("render h"));
            let initial = self.render_height.to_string();
            input.update(cx, |this, cx| {
                this.set_value(initial, window, cx);
            });
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change | InputEvent::PressEnter { .. })
                    || this.timeline_input_syncing
                {
                    return;
                }
                let text = input.read(cx).value().to_string();
                this.schedule_graph_input_apply("render height", text, cx);
            });
            self.render_height_input = Some(input);
            self.render_height_input_sub = Some(sub);
        }
    }

    fn sync_timeline_inputs_if_needed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.timeline_input_syncing = true;

        if let Some(input) = self.frame_input.as_ref() {
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            let value = self.frame.to_string();
            if !focused && input.read(cx).value() != value {
                input.update(cx, |this, cx| {
                    this.set_value(value, window, cx);
                });
            }
        }

        if let Some(input) = self.duration_input.as_ref() {
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            let value = Self::fmt(self.duration_sec);
            if !focused && input.read(cx).value() != value {
                input.update(cx, |this, cx| {
                    this.set_value(value, window, cx);
                });
            }
        }

        if let Some(input) = self.graph_fps_input.as_ref() {
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            let value = self.graph_fps.to_string();
            if !focused && input.read(cx).value() != value {
                input.update(cx, |this, cx| {
                    this.set_value(value, window, cx);
                });
            }
        }

        if let Some(input) = self.graph_width_input.as_ref() {
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            let value = self.graph_width.to_string();
            if !focused && input.read(cx).value() != value {
                input.update(cx, |this, cx| {
                    this.set_value(value, window, cx);
                });
            }
        }

        if let Some(input) = self.graph_height_input.as_ref() {
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            let value = self.graph_height.to_string();
            if !focused && input.read(cx).value() != value {
                input.update(cx, |this, cx| {
                    this.set_value(value, window, cx);
                });
            }
        }

        if let Some(input) = self.render_width_input.as_ref() {
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            let value = self.render_width.to_string();
            if !focused && input.read(cx).value() != value {
                input.update(cx, |this, cx| {
                    this.set_value(value, window, cx);
                });
            }
        }

        if let Some(input) = self.render_height_input.as_ref() {
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            let value = self.render_height.to_string();
            if !focused && input.read(cx).value() != value {
                input.update(cx, |this, cx| {
                    this.set_value(value, window, cx);
                });
            }
        }

        self.timeline_input_syncing = false;
    }

    fn set_action_dsl_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let before = self.capture_snapshot();
        *self.active_action_dsl_mut() = text.clone();
        self.set_active_selected_action_keyframe_t(None);
        self.apply_action_dsl_pose_to_controls();
        if let Some(input) = self.action_dsl_input.as_ref() {
            self.action_dsl_input_syncing = true;
            input.update(cx, |this, cx| {
                this.set_value(text, window, cx);
            });
            self.action_dsl_input_syncing = false;
        }
        self.invalidate_preview();
        self.push_undo_snapshot_if_changed(before);
    }

    fn sorted_hidden_meshes(&self) -> Vec<String> {
        let mut names = self.hidden_meshes.iter().cloned().collect::<Vec<_>>();
        names.sort();
        names
    }

    fn sorted_hidden_materials(&self) -> Vec<String> {
        let mut names = self.hidden_materials.iter().cloned().collect::<Vec<_>>();
        names.sort();
        names
    }

    fn retarget_dsl(&self) -> String {
        Self::retarget_dsl_for_maps(&self.retarget_maps)
    }

    fn retarget_dsl_for_maps(maps: &[(String, String)]) -> String {
        if maps.is_empty() {
            return "    <!-- No canonical retarget map assigned yet. Raw bone Action IDs still work for this GLB only. -->".to_string();
        }
        let mut out = String::from("    <Retarget>\n");
        for (from, to) in maps {
            out.push_str(&format!(
                "      <Map from=\"{}\" to=\"{}\" />\n",
                Self::xml_attr_escape(from),
                Self::xml_attr_escape(to)
            ));
        }
        out.push_str("    </Retarget>");
        out
    }

    fn bone_axis_dsl_for_axes_with_rest(
        &self,
        axes: &HashMap<String, RetargetBoneAxis>,
        source_rest_pose: SourceRestPose,
        custom_corrections: &HashMap<String, RestPoseSemanticCorrection>,
    ) -> String {
        let rest_offsets = self.rest_offsets_for_pose(source_rest_pose, custom_corrections);
        let mut lines = Vec::new();
        for bone in Self::humanoid_bones() {
            let Some(axis) = axes.get(*bone) else {
                continue;
            };
            let mut attrs = Vec::new();
            if let Some(value) = axis.turn.as_deref() {
                attrs.push(format!("turn=\"{}\"", Self::xml_attr_escape(value)));
            }
            if let Some(value) = axis.bend.as_deref() {
                attrs.push(format!("bend=\"{}\"", Self::xml_attr_escape(value)));
            }
            if let Some(value) = axis.forward.as_deref() {
                attrs.push(format!("forward=\"{}\"", Self::xml_attr_escape(value)));
            }
            if let Some(value) = axis.side.as_deref() {
                attrs.push(format!("side=\"{}\"", Self::xml_attr_escape(value)));
            }
            if let Some(value) = axis.twist.as_deref() {
                attrs.push(format!("twist=\"{}\"", Self::xml_attr_escape(value)));
            }
            let rest = rest_offsets.get(*bone).copied().unwrap_or_default();
            let rest_turn = axis
                .rest_turn
                .clone()
                .or_else(|| (rest.turn.abs() >= 0.001).then(|| Self::fmt(rest.turn)));
            let rest_bend = axis
                .rest_bend
                .clone()
                .or_else(|| (rest.bend.abs() >= 0.001).then(|| Self::fmt(rest.bend)));
            let rest_forward = axis
                .rest_forward
                .clone()
                .or_else(|| (rest.forward.abs() >= 0.001).then(|| Self::fmt(rest.forward)));
            let rest_side = axis
                .rest_side
                .clone()
                .or_else(|| (rest.side.abs() >= 0.001).then(|| Self::fmt(rest.side)));
            let rest_twist = axis
                .rest_twist
                .clone()
                .or_else(|| (rest.twist.abs() >= 0.001).then(|| Self::fmt(rest.twist)));
            if let Some(value) = rest_turn.as_deref() {
                attrs.push(format!("restTurn=\"{}\"", Self::xml_attr_escape(value)));
            }
            if let Some(value) = rest_bend.as_deref() {
                attrs.push(format!("restBend=\"{}\"", Self::xml_attr_escape(value)));
            }
            if let Some(value) = rest_forward.as_deref() {
                attrs.push(format!("restForward=\"{}\"", Self::xml_attr_escape(value)));
            }
            if let Some(value) = rest_side.as_deref() {
                attrs.push(format!("restSide=\"{}\"", Self::xml_attr_escape(value)));
            }
            if let Some(value) = rest_twist.as_deref() {
                attrs.push(format!("restTwist=\"{}\"", Self::xml_attr_escape(value)));
            }
            if !attrs.is_empty() {
                lines.push(format!(
                    "      <Axis bone=\"{}\" {} />",
                    bone,
                    attrs.join(" ")
                ));
            }
        }
        format!(
            "    <BoneAxisMap>\n{}\n    </BoneAxisMap>",
            lines.join("\n")
        )
    }

    fn rest_offsets_for_pose(
        &self,
        source_rest_pose: SourceRestPose,
        custom_corrections: &HashMap<String, RestPoseSemanticCorrection>,
    ) -> HashMap<String, RestPoseSemanticCorrection> {
        let mut corrections = HashMap::<String, RestPoseSemanticCorrection>::new();
        let side = source_rest_pose.arm_side_correction();
        if side.abs() > f32::EPSILON {
            corrections
                .entry("upper_arm_l".to_string())
                .or_default()
                .side += side;
            corrections
                .entry("upper_arm_r".to_string())
                .or_default()
                .side += side;
        }
        corrections.entry("forearm_r".to_string()).or_default().bend +=
            self.profile.forearm_r_rest_bend;
        corrections.entry("forearm_l".to_string()).or_default().bend += 10.0;
        for (bone, correction) in custom_corrections {
            let entry = corrections.entry(bone.clone()).or_default();
            *entry = entry.add(*correction);
        }
        corrections
    }

    fn xml_attr_escape(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn fmt(value: f32) -> String {
        let mut text = format!("{value:.2}");
        while text.contains('.') && text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
        text
    }

    fn display_bone_name(raw_name: &str) -> String {
        raw_name.rsplit(':').next().unwrap_or(raw_name).to_string()
    }

    fn selected_raw_bone_name(&self) -> Option<&str> {
        self.active_selected_raw_bone()
            .and_then(|index| self.active_raw_bones().get(index))
            .map(|bone| bone.name.as_str())
    }

    fn selected_raw_bone_rotation(&self) -> [f32; 3] {
        self.selected_raw_bone_name()
            .and_then(|name| self.active_raw_bone_rotations().get(name).copied())
            .unwrap_or([0.0, 0.0, 0.0])
    }

    fn selected_rest_pose_correction(&self) -> RestPoseSemanticCorrection {
        self.active_selected_canonical_bone()
            .and_then(|bone| self.active_rest_pose_corrections().get(bone).copied())
            .unwrap_or_default()
    }

    fn mapped_canonical_for_raw(&self, raw_name: &str) -> Option<&str> {
        self.active_retarget_maps()
            .iter()
            .find(|(from, _)| from == raw_name)
            .map(|(_, to)| to.as_str())
    }

    fn mapped_raw_for_canonical(&self, canonical: &str) -> Option<&str> {
        self.active_retarget_maps()
            .iter()
            .find(|(_, to)| to == canonical)
            .map(|(from, _)| from.as_str())
    }

    fn selected_axis_bone(&self) -> String {
        self.active_selected_canonical_bone()
            .map(str::to_string)
            .unwrap_or_else(|| {
                Self::suggested_axis_bone_for_pose(self.active_test_action_preset()).to_string()
            })
    }

    fn axis_bone_for_new_preset(&self, preset: RetargetPosePreset) -> String {
        let Some(selected) = self.active_selected_canonical_bone() else {
            return Self::suggested_axis_bone_for_pose(preset).to_string();
        };
        if Self::axis_bone_allowed_for_preset(selected, preset) {
            selected.to_string()
        } else {
            Self::suggested_axis_bone_for_pose(preset).to_string()
        }
    }

    fn axis_bone_allowed_for_preset(bone: &str, preset: RetargetPosePreset) -> bool {
        match preset {
            RetargetPosePreset::Original => true,
            RetargetPosePreset::ArmsDown
            | RetargetPosePreset::APose
            | RetargetPosePreset::TPose => matches!(bone, "upper_arm_l" | "upper_arm_r"),
            RetargetPosePreset::WaveHand => {
                matches!(bone, "upper_arm_r" | "forearm_r" | "hand_r")
            }
            RetargetPosePreset::Walk => matches!(
                bone,
                "upper_leg_l" | "lower_leg_l" | "foot_l" | "upper_leg_r" | "lower_leg_r" | "foot_r"
            ),
            RetargetPosePreset::Jump => matches!(
                bone,
                "hips"
                    | "spine"
                    | "chest"
                    | "head"
                    | "upper_arm_l"
                    | "forearm_l"
                    | "upper_arm_r"
                    | "forearm_r"
                    | "upper_leg_l"
                    | "lower_leg_l"
                    | "foot_l"
                    | "upper_leg_r"
                    | "lower_leg_r"
                    | "foot_r"
            ),
            RetargetPosePreset::SidePlus20 | RetargetPosePreset::SideMinus20 => matches!(
                bone,
                "upper_arm_l" | "upper_arm_r" | "upper_leg_l" | "upper_leg_r"
            ),
            RetargetPosePreset::ForwardPlus20 => matches!(
                bone,
                "upper_arm_l" | "upper_arm_r" | "upper_leg_l" | "upper_leg_r"
            ),
            RetargetPosePreset::BendPlus20 => matches!(
                bone,
                "forearm_l" | "forearm_r" | "lower_leg_l" | "lower_leg_r" | "foot_l" | "foot_r"
            ),
            RetargetPosePreset::TwistPlus20 => matches!(
                bone,
                "upper_arm_l" | "forearm_l" | "hand_l" | "upper_arm_r" | "forearm_r" | "hand_r"
            ),
        }
    }

    fn suggested_axis_bone_for_pose(preset: RetargetPosePreset) -> &'static str {
        match preset {
            RetargetPosePreset::SidePlus20
            | RetargetPosePreset::SideMinus20
            | RetargetPosePreset::ForwardPlus20
            | RetargetPosePreset::APose
            | RetargetPosePreset::TPose
            | RetargetPosePreset::TwistPlus20 => "upper_arm_l",
            RetargetPosePreset::BendPlus20 => "forearm_l",
            RetargetPosePreset::Walk => "upper_leg_l",
            RetargetPosePreset::Jump => "upper_leg_l",
            RetargetPosePreset::WaveHand => "upper_arm_r",
            RetargetPosePreset::Original => "upper_arm_l",
            RetargetPosePreset::ArmsDown => "upper_arm_l",
        }
    }

    fn counterpart_bone(bone: &str) -> Option<&'static str> {
        match bone {
            "shoulder_l" => Some("shoulder_r"),
            "upper_arm_l" => Some("upper_arm_r"),
            "forearm_l" => Some("forearm_r"),
            "hand_l" => Some("hand_r"),
            "upper_leg_l" => Some("upper_leg_r"),
            "lower_leg_l" => Some("lower_leg_r"),
            "foot_l" => Some("foot_r"),
            "toe_l" => Some("toe_r"),
            "shoulder_r" => Some("shoulder_l"),
            "upper_arm_r" => Some("upper_arm_l"),
            "forearm_r" => Some("forearm_l"),
            "hand_r" => Some("hand_l"),
            "upper_leg_r" => Some("upper_leg_l"),
            "lower_leg_r" => Some("lower_leg_l"),
            "foot_r" => Some("foot_l"),
            "toe_r" => Some("toe_l"),
            _ => None,
        }
    }

    fn axis_role_value(&self, bone: &str, role: &str) -> String {
        let axis = self.active_bone_axes().get(bone);
        let value = match role {
            "turn" => axis.and_then(|axis| axis.turn.as_deref()),
            "bend" => axis.and_then(|axis| axis.bend.as_deref()),
            "forward" => axis.and_then(|axis| axis.forward.as_deref()),
            "side" => axis.and_then(|axis| axis.side.as_deref()),
            "twist" => axis.and_then(|axis| axis.twist.as_deref()),
            _ => None,
        };
        value.unwrap_or("unset").to_string()
    }

    fn axis_rest_role_value(&self, bone: &str, role: &str) -> String {
        let axis = self.active_bone_axes().get(bone);
        let explicit = match role {
            "turn" => axis.and_then(|axis| axis.rest_turn.as_deref()),
            "bend" => axis.and_then(|axis| axis.rest_bend.as_deref()),
            "forward" => axis.and_then(|axis| axis.rest_forward.as_deref()),
            "side" => axis.and_then(|axis| axis.rest_side.as_deref()),
            "twist" => axis.and_then(|axis| axis.rest_twist.as_deref()),
            _ => None,
        };
        if let Some(value) = explicit {
            return value.to_string();
        }
        let rest_offsets = self.rest_offsets_for_pose(
            self.active_source_rest_pose(),
            self.active_rest_pose_corrections(),
        );
        let correction = rest_offsets.get(bone).copied().unwrap_or_default();
        let value = match role {
            "turn" => correction.turn,
            "bend" => correction.bend,
            "forward" => correction.forward,
            "side" => correction.side,
            "twist" => correction.twist,
            _ => 0.0,
        };
        if value.abs() >= 0.001 {
            Self::fmt(value)
        } else {
            "unset".to_string()
        }
    }

    fn set_axis_role(&mut self, bone: &str, role: &str, binding: &str) {
        let before = self.capture_snapshot();
        let axis = self
            .active_bone_axes_mut()
            .entry(bone.to_string())
            .or_default();
        let slot = match role {
            "turn" => &mut axis.turn,
            "bend" => &mut axis.bend,
            "forward" => &mut axis.forward,
            "side" => &mut axis.side,
            "twist" => &mut axis.twist,
            _ => return,
        };
        *slot = Some(binding.to_string());
        self.status_line = format!(
            "Set {} BoneAxis {bone}.{role} = {binding}.",
            self.selected_actor_label()
        );
        self.invalidate_preview();
        self.push_undo_snapshot_if_changed(before);
    }

    fn clear_axis_role(&mut self, bone: &str, role: &str) {
        let before = self.capture_snapshot();
        let Some(axis) = self.active_bone_axes_mut().get_mut(bone) else {
            return;
        };
        match role {
            "turn" => axis.turn = None,
            "bend" => axis.bend = None,
            "forward" => axis.forward = None,
            "side" => axis.side = None,
            "twist" => axis.twist = None,
            _ => return,
        }
        self.status_line = format!(
            "Cleared {} BoneAxis {bone}.{role}.",
            self.selected_actor_label()
        );
        self.invalidate_preview();
        self.push_undo_snapshot_if_changed(before);
    }

    fn set_axis_rest_role(&mut self, bone: &str, role: &str, value: f32) {
        let before = self.capture_snapshot();
        let formatted = Self::fmt(value);
        let axis = self
            .active_bone_axes_mut()
            .entry(bone.to_string())
            .or_default();
        let slot = match role {
            "turn" => &mut axis.rest_turn,
            "bend" => &mut axis.rest_bend,
            "forward" => &mut axis.rest_forward,
            "side" => &mut axis.rest_side,
            "twist" => &mut axis.rest_twist,
            _ => return,
        };
        *slot = Some(formatted.clone());
        self.status_line = format!(
            "Set {} BoneAxis {bone}.rest{} = {}.",
            self.selected_actor_label(),
            Self::rest_role_attr_suffix(role),
            formatted
        );
        self.invalidate_preview();
        self.push_undo_snapshot_if_changed(before);
    }

    fn adjust_axis_rest_role(&mut self, bone: &str, role: &str, delta: f32) {
        let current = self
            .axis_rest_role_value(bone, role)
            .parse::<f32>()
            .unwrap_or(0.0);
        self.set_axis_rest_role(bone, role, current + delta);
    }

    fn clear_axis_rest_role(&mut self, bone: &str, role: &str) {
        self.set_axis_rest_role(bone, role, 0.0);
    }

    fn rest_role_attr_suffix(role: &str) -> &'static str {
        match role {
            "turn" => "Turn",
            "bend" => "Bend",
            "forward" => "Forward",
            "side" => "Side",
            "twist" => "Twist",
            _ => "",
        }
    }

    fn copy_axis_to_counterpart(&mut self, bone: &str, mirrored: bool) {
        let Some(counterpart) = Self::counterpart_bone(bone) else {
            self.status_line = format!("{bone} has no left/right counterpart.");
            return;
        };
        let Some(mut axis) = self.active_bone_axes().get(bone).cloned() else {
            self.status_line = format!("{bone} has no BoneAxis values to copy.");
            return;
        };
        let before = self.capture_snapshot();
        if mirrored {
            axis.turn = axis.turn.as_deref().map(Self::invert_axis_binding);
            axis.bend = axis.bend.as_deref().map(Self::invert_axis_binding);
            axis.forward = axis.forward.as_deref().map(Self::invert_axis_binding);
            axis.side = axis.side.as_deref().map(Self::invert_axis_binding);
            axis.twist = axis.twist.as_deref().map(Self::invert_axis_binding);
            axis.rest_turn = axis.rest_turn.as_deref().map(Self::invert_numeric_string);
            axis.rest_bend = axis.rest_bend.as_deref().map(Self::invert_numeric_string);
            axis.rest_forward = axis
                .rest_forward
                .as_deref()
                .map(Self::invert_numeric_string);
            axis.rest_side = axis.rest_side.as_deref().map(Self::invert_numeric_string);
            axis.rest_twist = axis.rest_twist.as_deref().map(Self::invert_numeric_string);
        }
        self.active_bone_axes_mut()
            .insert(counterpart.to_string(), axis);
        self.status_line = if mirrored {
            format!("Copied mirrored BoneAxis {bone} -> {counterpart}.")
        } else {
            format!("Copied exact BoneAxis {bone} -> {counterpart}.")
        };
        self.invalidate_preview();
        self.push_undo_snapshot_if_changed(before);
    }

    fn invert_axis_binding(binding: &str) -> String {
        let (axis, scale) = binding.split_once(':').unwrap_or((binding, "1"));
        let value = scale.trim().parse::<f32>().unwrap_or(1.0) * -1.0;
        format!("{}:{}", axis.trim(), Self::fmt(value))
    }

    fn invert_numeric_string(value: &str) -> String {
        Self::fmt(value.trim().parse::<f32>().unwrap_or(0.0) * -1.0)
    }

    fn assign_selected_raw_to(&mut self, canonical: &str) {
        let Some(raw_name) = self.selected_raw_bone_name().map(str::to_string) else {
            self.status_line = "Select a raw bone before assigning it.".to_string();
            return;
        };
        let before = self.capture_snapshot();
        let maps = self.active_retarget_maps_mut();
        maps.retain(|(from, to)| from != &raw_name && to != canonical);
        maps.push((raw_name.clone(), canonical.to_string()));
        *maps = Self::sort_retarget_maps(maps);
        self.status_line = format!(
            "Assigned raw bone {} to canonical {}.",
            Self::display_bone_name(&raw_name),
            canonical
        );
        self.invalidate_preview();
        self.push_undo_snapshot_if_changed(before);
    }

    fn clear_canonical_assignment(&mut self, canonical: &str) {
        let before = self.capture_snapshot();
        self.active_retarget_maps_mut()
            .retain(|(_, to)| to != canonical);
        self.status_line = format!("Cleared canonical assignment for {canonical}.");
        self.invalidate_preview();
        self.push_undo_snapshot_if_changed(before);
    }

    fn adjust_selected_raw_rotation(&mut self, axis: usize, delta: f32) {
        let Some(raw_name) = self.selected_raw_bone_name().map(str::to_string) else {
            self.status_line = "Select a raw bone before rotating it.".to_string();
            return;
        };
        let rotations = self.active_raw_bone_rotations_mut();
        let rotation = rotations.entry(raw_name.clone()).or_insert([0.0, 0.0, 0.0]);
        rotation[axis] = (rotation[axis] + delta).clamp(-180.0, 180.0);
        if rotation.iter().all(|value| value.abs() < 0.001) {
            rotations.remove(&raw_name);
        }
        self.status_line = format!(
            "Rotated raw bone {}. Assign it to a canonical bone for reusable DSL.",
            Self::display_bone_name(&raw_name)
        );
        self.sync_action_dsl_from_current_pose_if_active();
        self.invalidate_preview();
    }

    fn raw_pose_dsl(&self) -> String {
        self.raw_pose_dsl_for_slot(self.selected_actor_slot)
    }

    fn raw_pose_dsl_for_slot(&self, slot: usize) -> String {
        let mut lines = Vec::<String>::new();
        let raw_bones = if slot == 0 {
            self.raw_bones.as_slice()
        } else {
            self.additional_raw_bones
                .get(slot - 1)
                .map(Vec::as_slice)
                .unwrap_or(&[])
        };
        let rotations = if slot == 0 {
            &self.raw_bone_rotations
        } else {
            self.additional_raw_bone_rotations
                .get(slot - 1)
                .unwrap_or(&self.raw_bone_rotations)
        };
        let maps = if slot == 0 {
            self.retarget_maps.as_slice()
        } else {
            self.additional_retarget_maps
                .get(slot - 1)
                .map(Vec::as_slice)
                .unwrap_or(&[])
        };
        for bone in raw_bones {
            let Some(rotation) = rotations.get(&bone.name) else {
                continue;
            };
            if rotation.iter().all(|value| value.abs() < 0.001) {
                continue;
            }
            let bone_id = maps
                .iter()
                .find(|(from, _)| from == &bone.name)
                .map(|(_, to)| to.as_str())
                .unwrap_or(bone.name.as_str());
            let mut attrs = Vec::new();
            if rotation[0].abs() >= 0.001 {
                attrs.push(format!("rotationX=\"{}\"", Self::fmt(rotation[0])));
            }
            if rotation[1].abs() >= 0.001 {
                attrs.push(format!("rotationY=\"{}\"", Self::fmt(rotation[1])));
            }
            if rotation[2].abs() >= 0.001 {
                attrs.push(format!("rotationZ=\"{}\"", Self::fmt(rotation[2])));
            }
            lines.push(format!(
                "      <Bone id=\"{}\" {} />",
                Self::xml_attr_escape(bone_id),
                attrs.join(" ")
            ));
        }
        if lines.is_empty() {
            String::new()
        } else {
            format!(
                "      <!-- Raw bone overrides. Mapped bones export through canonical ids; unmapped ids are model-specific. -->\n{}\n",
                lines.join("\n")
            )
        }
    }

    fn ensure_preview_requested(&mut self, cx: &mut Context<Self>) {
        if self.model_path.trim().is_empty() {
            self.preview_pending = false;
            self.preview_dirty = false;
            self.preview_error = None;
            self.preview_key = None;
            return;
        }
        let script = self.generated_preview_dsl();
        let key = (
            Self::script_hash(&script),
            self.frame,
            self.selected_actor_slot,
        );
        if self.preview_pending {
            if self.preview_key != Some(key) {
                self.preview_dirty = true;
            }
            return;
        }
        if self.preview_key == Some(key) && (self.preview_image.is_some() || self.preview_pending) {
            return;
        }
        self.preview_key = Some(key);
        self.preview_pending = true;
        self.preview_dirty = false;
        self.preview_error = None;
        self.preview_token = self.preview_token.wrapping_add(1);
        let token = self.preview_token;
        let frame = self.frame;
        let actor_id = self.selected_actor_id();
        let asset_root = Self::world_asset_root();
        let (tx, rx) = mpsc::channel::<Result<CharacterPreviewResponse, String>>();
        let request = CharacterPreviewRequest {
            script,
            frame,
            actor_id,
            asset_root,
            response_tx: tx,
        };
        if let Err(err) = self.preview_tx.send(request) {
            self.preview_tx = Self::spawn_preview_worker();
            let _ = self.preview_tx.send(err.0);
        }
        cx.spawn(async move |view, cx| {
            loop {
                Timer::after(Duration::from_millis(16)).await;
                let mut done = false;
                let _ = view.update(cx, |this, cx| {
                    if this.preview_token != token {
                        done = true;
                        return;
                    }
                    match rx.try_recv() {
                        Ok(Ok(response)) => {
                            this.preview_pending = false;
                            if let Some(diagnostics) = response.diagnostics {
                                this.gpu_diagnostics = Some(diagnostics);
                            }
                            if let Some(image) = Self::render_image_from_bgra(
                                response.width,
                                response.height,
                                response.bgra,
                            ) {
                                this.preview_image = Some((image, response.width, response.height));
                            }
                            if this.preview_dirty {
                                this.preview_dirty = false;
                                this.preview_key = None;
                                this.ensure_preview_requested(cx);
                            }
                            done = true;
                            cx.notify();
                        }
                        Ok(Err(err)) => {
                            this.preview_pending = false;
                            this.preview_error = Some(err);
                            done = true;
                            cx.notify();
                        }
                        Err(mpsc::TryRecvError::Empty) => {}
                        Err(mpsc::TryRecvError::Disconnected) => {
                            this.preview_pending = false;
                            this.preview_error =
                                Some("Character preview worker disconnected.".to_string());
                            done = true;
                            cx.notify();
                        }
                    }
                });
                if done {
                    break;
                }
            }
        })
        .detach();
    }

    fn schedule_play_tick(&mut self, cx: &mut Context<Self>) {
        let token = self.play_token;
        let frame_ms = (1000 / self.graph_fps.max(1) as u64).max(1);
        cx.spawn(async move |view, cx| {
            Timer::after(Duration::from_millis(frame_ms)).await;
            let _ = view.update(cx, |this, cx| {
                if !this.playing || this.play_token != token {
                    return;
                }
                this.frame = (this.frame + 1) % this.total_frames();
                this.set_active_selected_action_keyframe_t(None);
                this.apply_action_dsl_pose_to_controls();
                this.preview_key = None;
                this.ensure_preview_requested(cx);
                this.schedule_play_tick(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn invalidate_preview(&mut self) {
        if self.preview_pending {
            self.preview_dirty = true;
        } else {
            self.preview_key = None;
        }
        self.preview_error = None;
    }

    fn set_model_path(&mut self, path: PathBuf) {
        self.model_path = Self::display_model_path(path);
        self.frame = 0;
        self.selected_actor_slot = 0;
        self.selected_action_keyframe_t = None;
        self.actor_x = 0.0;
        self.actor_y = 0.0;
        self.actor_z = 0.0;
        self.actor_yaw = 5.0;
        self.actor_pitch = 0.0;
        self.actor_roll = 0.0;
        self.bone_axes = Self::default_bone_axes();
        self.rest_pose_corrections.clear();
        self.test_action_preset = RetargetPosePreset::Original;
        self.pose_calibration_preset = None;
        self.playing = false;
        self.play_token = self.play_token.wrapping_add(1);
        self.preview_token = self.preview_token.wrapping_add(1);
        self.preview_image = None;
        self.preview_key = None;
        self.preview_pending = false;
        self.preview_dirty = false;
        self.invalidate_preview();
        self.refresh_retarget_from_model();
    }

    fn add_model_path(&mut self, path: PathBuf) {
        if self.model_path.trim().is_empty() {
            self.set_model_path(path);
            self.status_line = format!("Loaded GLB #1: {}", self.model_path);
        } else {
            self.add_additional_model_path(path);
        }
    }

    fn set_static_background_path(&mut self, path: PathBuf) {
        self.background_image_path = Self::display_model_path(path);
        self.invalidate_preview();
        self.status_line = format!("Loaded static background: {}", self.background_image_path);
    }

    fn add_additional_model_path(&mut self, path: PathBuf) {
        let path = Self::display_model_path(path);
        let inspection = match Self::inspect_model(&path) {
            Ok(inspection) => inspection,
            Err(err) => {
                self.status_line = format!("Loaded GLB, but retarget inspect failed: {err}");
                LoadedModelInspection {
                    retarget_maps: Vec::new(),
                    raw_bones: Vec::new(),
                    mesh_names: Vec::new(),
                    material_names: Vec::new(),
                    hidden_meshes: HashSet::new(),
                    hidden_materials: HashSet::new(),
                    diagnostics: None,
                    bounds_height: 0.0,
                    source_rest_pose: SourceRestPose::ArmsDown,
                }
            }
        };
        let next_index = self.additional_model_paths.len();
        self.additional_model_paths.push(path.clone());
        self.additional_actor_positions
            .push(Self::additional_actor_default_position(next_index));
        self.additional_actor_rotations
            .push(Self::additional_actor_default_rotation());
        self.additional_actions.push(WaveActionControls::default());
        self.additional_retarget_maps
            .push(inspection.retarget_maps.clone());
        self.additional_bone_axes.push(Self::default_bone_axes());
        self.additional_source_rest_poses
            .push(inspection.source_rest_pose);
        self.additional_rest_pose_corrections.push(HashMap::new());
        self.additional_raw_bones.push(inspection.raw_bones.clone());
        self.additional_selected_raw_bones
            .push((!inspection.raw_bones.is_empty()).then_some(0));
        self.additional_selected_canonical_bones.push(None);
        self.additional_raw_bone_rotations.push(HashMap::new());
        self.additional_mesh_names.push(inspection.mesh_names);
        self.additional_material_names
            .push(inspection.material_names);
        self.additional_hidden_meshes.push(inspection.hidden_meshes);
        self.additional_hidden_materials
            .push(inspection.hidden_materials);
        self.additional_action_dsls.push(String::new());
        self.additional_selected_action_keyframe_ts.push(None);
        self.additional_test_action_presets
            .push(RetargetPosePreset::Original);
        self.selected_actor_slot = self.additional_model_paths.len();
        self.invalidate_preview();
        self.status_line = format!(
            "Loaded GLB #{} with {} retarget map(s), source rest {}: {}",
            self.additional_model_paths.len() + 1,
            inspection.retarget_maps.len(),
            inspection.source_rest_pose.label(),
            path
        );
    }

    fn remove_additional_model_path(&mut self, index: usize) {
        if index >= self.additional_model_paths.len() {
            self.status_line = "Additional GLB was already removed.".to_string();
            return;
        }
        let path = self.additional_model_paths.remove(index);
        self.additional_actor_positions.remove(index);
        self.additional_actor_rotations.remove(index);
        self.additional_actions.remove(index);
        self.additional_retarget_maps.remove(index);
        self.additional_bone_axes.remove(index);
        self.additional_source_rest_poses.remove(index);
        self.additional_rest_pose_corrections.remove(index);
        self.additional_raw_bones.remove(index);
        self.additional_selected_raw_bones.remove(index);
        self.additional_selected_canonical_bones.remove(index);
        self.additional_raw_bone_rotations.remove(index);
        self.additional_mesh_names.remove(index);
        self.additional_material_names.remove(index);
        self.additional_hidden_meshes.remove(index);
        self.additional_hidden_materials.remove(index);
        self.additional_action_dsls.remove(index);
        self.additional_selected_action_keyframe_ts.remove(index);
        self.additional_test_action_presets.remove(index);
        if self.selected_actor_slot == index + 1 {
            self.selected_actor_slot = 0;
        } else if self.selected_actor_slot > index + 1 {
            self.selected_actor_slot -= 1;
        }
        self.invalidate_preview();
        self.status_line = format!("Removed GLB: {path}");
    }

    fn remove_model_slot(&mut self, slot: usize) {
        if slot == 0 {
            let removed = self.model_path.clone();
            if self.additional_model_paths.is_empty() {
                self.model_path.clear();
                self.retarget_maps.clear();
                self.bone_axes = Self::default_bone_axes();
                self.source_rest_pose = SourceRestPose::ArmsDown;
                self.rest_pose_corrections.clear();
                self.raw_bones.clear();
                self.selected_raw_bone = None;
                self.selected_canonical_bone = None;
                self.raw_bone_rotations.clear();
                self.mesh_names.clear();
                self.material_names.clear();
                self.hidden_meshes.clear();
                self.hidden_materials.clear();
                self.gpu_diagnostics = None;
                self.action = WaveActionControls::default();
                self.action_dsl.clear();
                self.selected_action_keyframe_t = None;
                self.test_action_preset = RetargetPosePreset::Original;
                self.actor_x = 0.0;
                self.actor_y = 0.0;
                self.actor_z = 0.0;
                self.actor_yaw = 5.0;
                self.actor_pitch = 0.0;
                self.actor_roll = 0.0;
                self.selected_actor_slot = 0;
            } else {
                self.model_path = self.additional_model_paths.remove(0);
                let [x, y, z] = self.additional_actor_positions.remove(0);
                self.actor_x = x;
                self.actor_y = y;
                self.actor_z = z;
                let [yaw, pitch, roll] = self.additional_actor_rotations.remove(0);
                self.actor_yaw = yaw;
                self.actor_pitch = pitch;
                self.actor_roll = roll;
                self.retarget_maps = self.additional_retarget_maps.remove(0);
                self.bone_axes = self.additional_bone_axes.remove(0);
                self.source_rest_pose = self.additional_source_rest_poses.remove(0);
                self.rest_pose_corrections = self.additional_rest_pose_corrections.remove(0);
                self.raw_bones = self.additional_raw_bones.remove(0);
                self.selected_raw_bone = self.additional_selected_raw_bones.remove(0);
                self.selected_canonical_bone = self.additional_selected_canonical_bones.remove(0);
                self.raw_bone_rotations = self.additional_raw_bone_rotations.remove(0);
                self.mesh_names = self.additional_mesh_names.remove(0);
                self.material_names = self.additional_material_names.remove(0);
                self.hidden_meshes = self.additional_hidden_meshes.remove(0);
                self.hidden_materials = self.additional_hidden_materials.remove(0);
                self.action = self.additional_actions.remove(0);
                self.action_dsl = self.additional_action_dsls.remove(0);
                self.selected_action_keyframe_t =
                    self.additional_selected_action_keyframe_ts.remove(0);
                self.test_action_preset = self.additional_test_action_presets.remove(0);
                if self.selected_actor_slot > 0 {
                    self.selected_actor_slot -= 1;
                }
            }
            self.normalize_additional_model_state();
            self.invalidate_preview();
            self.status_line = format!("Removed GLB: {removed}");
            return;
        }
        self.remove_additional_model_path(slot - 1);
    }

    fn begin_drag(&mut self, handle: CharacterDragHandle, evt: &MouseDownEvent) {
        let action = self.active_action().clone();
        self.drag_state = Some(CharacterDragState {
            handle,
            snapshot: self.capture_snapshot(),
            start_x: f32::from(evt.position.x),
            start_y: f32::from(evt.position.y),
            raise_forward: action.raise_forward,
            raise_side: action.raise_side,
            elbow_bend: action.elbow_bend,
            wave_twist: action.wave_twist,
            left_leg_forward: action.left_leg_forward,
            left_knee_bend: action.left_knee_bend,
            right_leg_forward: action.right_leg_forward,
            right_knee_bend: action.right_knee_bend,
            chest_turn: action.chest_turn,
            head_turn: action.head_turn,
            raw_rotation: self.selected_raw_bone_rotation(),
            rest_correction: self.selected_rest_pose_correction(),
        });
        self.status_line = if self.pose_calibration_preset.is_some() {
            format!(
                "Dragging {} for rest offset editing. This writes semantic rest* values inside BoneAxisMap.",
                handle.label()
            )
        } else {
            format!(
                "Dragging canonical {}. Copy Formal DSL when the pose feels right.",
                handle.label()
            )
        };
    }

    fn update_drag(&mut self, evt: &MouseMoveEvent) {
        let Some(drag) = self.drag_state.as_ref().cloned() else {
            return;
        };
        if !evt.dragging() {
            return;
        }
        let dx = f32::from(evt.position.x) - drag.start_x;
        let dy = f32::from(evt.position.y) - drag.start_y;
        match drag.handle {
            CharacterDragHandle::RightHand => {
                let action = self.active_action_mut();
                action.raise_side = (drag.raise_side + dx * 0.32).clamp(-180.0, 180.0);
                action.raise_forward = (drag.raise_forward - dy * 0.32).clamp(-180.0, 180.0);
            }
            CharacterDragHandle::RightElbow => {
                let action = self.active_action_mut();
                action.elbow_bend = (drag.elbow_bend - dy * 0.34).clamp(-120.0, 140.0);
                action.wave_twist = (drag.wave_twist + dx * 0.22).clamp(-90.0, 90.0);
            }
            CharacterDragHandle::LeftFoot => {
                let action = self.active_action_mut();
                action.left_leg_forward = (drag.left_leg_forward + dx * 0.22).clamp(-85.0, 85.0);
                action.left_knee_bend = (drag.left_knee_bend - dy * 0.24).clamp(-85.0, 120.0);
            }
            CharacterDragHandle::RightFoot => {
                let action = self.active_action_mut();
                action.right_leg_forward = (drag.right_leg_forward + dx * 0.22).clamp(-85.0, 85.0);
                action.right_knee_bend = (drag.right_knee_bend - dy * 0.24).clamp(-85.0, 120.0);
            }
            CharacterDragHandle::Head => {
                self.active_action_mut().head_turn =
                    (drag.head_turn + dx * 0.18).clamp(-45.0, 45.0);
            }
            CharacterDragHandle::Chest => {
                self.active_action_mut().chest_turn =
                    (drag.chest_turn + dx * 0.16).clamp(-60.0, 60.0);
            }
            CharacterDragHandle::RawBone => {
                if self.pose_calibration_preset.is_some() {
                    if let Some(canonical) =
                        self.active_selected_canonical_bone().map(str::to_string)
                    {
                        let mut correction = drag.rest_correction;
                        match canonical.as_str() {
                            "upper_arm_l" | "upper_arm_r" | "upper_leg_l" | "upper_leg_r" => {
                                correction.side =
                                    (drag.rest_correction.side + dx * 0.32).clamp(-180.0, 180.0);
                                correction.forward =
                                    (drag.rest_correction.forward - dy * 0.32).clamp(-180.0, 180.0);
                            }
                            "forearm_l" | "forearm_r" | "lower_leg_l" | "lower_leg_r" => {
                                correction.bend =
                                    (drag.rest_correction.bend - dy * 0.34).clamp(-180.0, 180.0);
                                correction.twist =
                                    (drag.rest_correction.twist + dx * 0.22).clamp(-180.0, 180.0);
                            }
                            "hand_l" | "hand_r" | "foot_l" | "foot_r" | "toe_l" | "toe_r" => {
                                correction.bend =
                                    (drag.rest_correction.bend - dy * 0.24).clamp(-180.0, 180.0);
                                correction.twist =
                                    (drag.rest_correction.twist + dx * 0.22).clamp(-180.0, 180.0);
                            }
                            "hips" | "spine" | "chest" | "neck" | "head" => {
                                correction.turn =
                                    (drag.rest_correction.turn + dx * 0.18).clamp(-180.0, 180.0);
                                correction.bend =
                                    (drag.rest_correction.bend - dy * 0.18).clamp(-180.0, 180.0);
                            }
                            _ => {}
                        }

                        if correction.is_identity() {
                            self.active_rest_pose_corrections_mut().remove(&canonical);
                        } else {
                            self.active_rest_pose_corrections_mut()
                                .insert(canonical.clone(), correction);
                        }
                        self.status_line = format!(
                            "Editing {} {canonical} rest* offset inside BoneAxisMap.",
                            self.selected_actor_label()
                        );
                    }
                } else if let Some(raw_name) = self.selected_raw_bone_name().map(str::to_string) {
                    let rotation = self
                        .active_raw_bone_rotations_mut()
                        .entry(raw_name.clone())
                        .or_insert(drag.raw_rotation);
                    rotation[1] = (drag.raw_rotation[1] + dx * 0.35).clamp(-180.0, 180.0);
                    rotation[0] = (drag.raw_rotation[0] - dy * 0.35).clamp(-180.0, 180.0);
                    self.status_line = format!(
                        "Dragging raw bone {}. Assign it to canonical for reusable DSL.",
                        Self::display_bone_name(&raw_name)
                    );
                }
            }
        }
        if self.pose_calibration_preset.is_none() {
            self.sync_action_dsl_from_current_pose_if_active();
        }
        self.invalidate_preview();
    }

    fn end_drag(&mut self) {
        if let Some(drag) = self.drag_state.take() {
            self.push_undo_snapshot_if_changed(drag.snapshot);
            self.status_line = if self.pose_calibration_preset.is_some() {
                format!(
                    "Updated {} semantic rest correction. Test Walk/Jump/Wave to verify.",
                    drag.handle.label()
                )
            } else {
                format!(
                    "Updated {} from direct manipulation. Copy Formal DSL to export.",
                    drag.handle.label()
                )
            };
        }
    }

    fn begin_viewport_drag(&mut self, mode: ViewportDragMode, evt: &MouseDownEvent) {
        self.viewport_drag_state = Some(ViewportDragState {
            mode,
            start_x: f32::from(evt.position.x),
            start_y: f32::from(evt.position.y),
            camera_yaw: self.camera_yaw,
            camera_pitch: self.camera_pitch,
            camera_x: self.camera_x,
            camera_y: self.camera_y,
        });
        self.status_line = match mode {
            ViewportDragMode::Pan => {
                "Camera pan: Option/Shift-drag empty preview space.".to_string()
            }
            ViewportDragMode::Orbit => {
                "Camera orbit: drag empty preview space to change yaw/pitch.".to_string()
            }
        };
    }

    fn update_viewport_drag(&mut self, evt: &MouseMoveEvent) {
        let Some(drag) = self.viewport_drag_state else {
            return;
        };
        if !evt.dragging() {
            return;
        }
        let dx = f32::from(evt.position.x) - drag.start_x;
        let dy = f32::from(evt.position.y) - drag.start_y;
        match drag.mode {
            ViewportDragMode::Pan => {
                let pan_scale = (self.camera_distance * 0.0025).clamp(0.0015, 0.03);
                self.camera_x = (drag.camera_x - dx * pan_scale).clamp(-8.0, 8.0);
                self.camera_y = (drag.camera_y + dy * pan_scale).clamp(-8.0, 8.0);
            }
            ViewportDragMode::Orbit => {
                self.camera_yaw = (drag.camera_yaw + dx * 0.25).rem_euclid(360.0);
                self.camera_pitch = (drag.camera_pitch + dy * 0.18).clamp(-80.0, 80.0);
            }
        }
        self.invalidate_preview();
    }

    fn end_viewport_drag(&mut self) {
        if let Some(drag) = self.viewport_drag_state.take() {
            self.status_line = match drag.mode {
                ViewportDragMode::Pan => "Camera pan updated.".to_string(),
                ViewportDragMode::Orbit => "Camera yaw/pitch updated.".to_string(),
            };
        }
    }

    fn scroll_viewport(&mut self, evt: &ScrollWheelEvent) {
        let delta = evt.delta.pixel_delta(px(10.0));
        let delta_x = delta.x / px(1.0);
        let delta_y = delta.y / px(1.0);
        if delta_x.abs() <= f32::EPSILON && delta_y.abs() <= f32::EPSILON {
            return;
        }

        // Trackpad-friendly controls:
        // - vertical two-finger scroll/pinch wheel events zoom
        // - horizontal two-finger scroll pans the preview.
        let pan_scroll = delta_x.abs() > delta_y.abs() * 1.15 && delta_x.abs() > 0.1;
        if pan_scroll {
            let pan_scale = (self.camera_distance * 0.0025).clamp(0.0015, 0.03);
            self.camera_x = (self.camera_x - delta_x * pan_scale).clamp(-8.0, 8.0);
            self.status_line = format!(
                "Camera pan updated. Camera x {}, y {}.",
                Self::fmt(self.camera_x),
                Self::fmt(self.camera_y)
            );
            self.invalidate_preview();
            return;
        }

        let factor = if delta_y > 0.0 { 1.08 } else { 0.92 };
        self.camera_distance = (self.camera_distance * factor).clamp(0.6, 14.0);
        self.status_line = format!(
            "Camera zoom updated. Camera distance {}.",
            Self::fmt(self.camera_distance)
        );
        self.invalidate_preview();
    }

    fn reset_viewport(&mut self) {
        self.camera_x = 0.0;
        self.camera_y = 0.0;
        self.camera_z = 0.0;
        self.camera_yaw = 0.0;
        self.camera_pitch = 0.0;
        self.camera_distance = 3.4;
        self.viewport_drag_state = None;
        self.status_line = "Camera view reset.".to_string();
        self.invalidate_preview();
    }

    fn control_button(label: &'static str) -> gpui::Div {
        div()
            .h(px(30.0))
            .flex_shrink_0()
            .px_3()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.15))
            .bg(white().opacity(0.06))
            .hover(|s| s.bg(white().opacity(0.11)))
            .cursor_pointer()
            .text_xs()
            .text_color(white().opacity(0.9))
            .flex()
            .items_center()
            .justify_center()
            .child(label)
    }

    fn dynamic_button(label: String, active: bool) -> gpui::Div {
        div()
            .h(px(30.0))
            .flex_shrink_0()
            .px_3()
            .rounded_md()
            .border_1()
            .border_color(if active {
                rgba(0x79c7ffcc)
            } else {
                rgba(0xffffff26)
            })
            .bg(if active {
                rgba(0x1d4f7acc)
            } else {
                rgba(0xffffff10)
            })
            .hover(|s| s.bg(white().opacity(0.12)))
            .cursor_pointer()
            .text_xs()
            .text_color(white().opacity(if active { 0.98 } else { 0.82 }))
            .flex()
            .items_center()
            .justify_center()
            .child(label)
    }

    fn section_label(label: &'static str) -> gpui::Div {
        div()
            .h(px(30.0))
            .flex_shrink_0()
            .px_2()
            .text_xs()
            .text_color(white().opacity(0.52))
            .flex()
            .items_center()
            .child(label)
    }

    fn value_pill(value: String) -> gpui::Div {
        div()
            .h(px(30.0))
            .flex_shrink_0()
            .px_3()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.10))
            .bg(rgba(0x050914cc))
            .text_xs()
            .text_color(white().opacity(0.76))
            .flex()
            .items_center()
            .justify_center()
            .child(value)
    }

    fn input_pill(
        input: &Entity<InputState>,
        width: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let input_entity = input.clone();
        div()
            .h(px(34.0))
            .w(px(width))
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.14))
            .bg(rgb(0x080b12))
            .overflow_hidden()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, window, cx| {
                    input_entity.read(cx).focus_handle(cx).focus(window);
                    cx.stop_propagation();
                }),
            )
            .child(Input::new(input).h(px(34.0)).w(px(width)))
            .into_any_element()
    }

    fn global_node_matrices(nodes: &[motionloom::GlbNodeData]) -> Vec<[f32; 16]> {
        let local = nodes
            .iter()
            .map(|node| {
                node.matrix.unwrap_or_else(|| {
                    Self::mat4_from_trs(node.translation, node.rotation, node.scale)
                })
            })
            .collect::<Vec<_>>();
        let mut global = vec![None; nodes.len()];
        for index in 0..nodes.len() {
            Self::compute_global_node_matrix(index, nodes, &local, &mut global);
        }
        global
            .into_iter()
            .map(|matrix| matrix.unwrap_or_else(Self::mat4_identity))
            .collect()
    }

    fn compute_global_node_matrix(
        index: usize,
        nodes: &[motionloom::GlbNodeData],
        local: &[[f32; 16]],
        global: &mut [Option<[f32; 16]>],
    ) -> [f32; 16] {
        if let Some(matrix) = global.get(index).copied().flatten() {
            return matrix;
        }
        let local_matrix = local
            .get(index)
            .copied()
            .unwrap_or_else(Self::mat4_identity);
        let matrix = nodes
            .get(index)
            .and_then(|node| node.parent)
            .map(|parent| {
                Self::mat4_mul(
                    Self::compute_global_node_matrix(parent, nodes, local, global),
                    local_matrix,
                )
            })
            .unwrap_or(local_matrix);
        if let Some(slot) = global.get_mut(index) {
            *slot = Some(matrix);
        }
        matrix
    }

    fn mat4_from_trs(translation: [f32; 3], rotation: [f32; 4], scale: [f32; 3]) -> [f32; 16] {
        Self::mat4_mul(
            Self::mat4_mul(
                Self::mat4_translation(translation),
                Self::mat4_from_quat(rotation),
            ),
            Self::mat4_scale(scale),
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

    fn mat4_from_quat(quat: [f32; 4]) -> [f32; 16] {
        let [x, y, z, w] = quat;
        let len = (x * x + y * y + z * z + w * w).sqrt();
        if len <= f32::EPSILON {
            return Self::mat4_identity();
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
        let mut out = [0.0; 16];
        for col in 0..4 {
            for row in 0..4 {
                out[col * 4 + row] = a[row] * b[col * 4]
                    + a[4 + row] * b[col * 4 + 1]
                    + a[8 + row] * b[col * 4 + 2]
                    + a[12 + row] * b[col * 4 + 3];
            }
        }
        out
    }

    fn mat4_transform_point(matrix: [f32; 16], point: [f32; 3]) -> [f32; 3] {
        let x = point[0];
        let y = point[1];
        let z = point[2];
        [
            matrix[0] * x + matrix[4] * y + matrix[8] * z + matrix[12],
            matrix[1] * x + matrix[5] * y + matrix[9] * z + matrix[13],
            matrix[2] * x + matrix[6] * y + matrix[10] * z + matrix[14],
        ]
    }

    fn axis_role_row(
        bone: String,
        role: &'static str,
        value: String,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let button = |label: &'static str, binding: &'static str, cx: &mut Context<Self>| {
            let bone = bone.clone();
            Self::control_button(label).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.set_axis_role(&bone, role, binding);
                    cx.notify();
                }),
            )
        };
        let clear_bone = bone.clone();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(px(58.0))
                    .text_xs()
                    .text_color(white().opacity(0.62))
                    .child(role),
            )
            .child(Self::dynamic_button(value, false))
            .child(button("X+", "rotationX:1", cx))
            .child(button("X-", "rotationX:-1", cx))
            .child(button("Y+", "rotationY:1", cx))
            .child(button("Y-", "rotationY:-1", cx))
            .child(button("Z+", "rotationZ:1", cx))
            .child(button("Z-", "rotationZ:-1", cx))
            .child(Self::control_button("Clear").on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.clear_axis_role(&clear_bone, role);
                    cx.notify();
                }),
            ))
    }

    fn axis_rest_role_row(
        bone: String,
        role: &'static str,
        value: String,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let set_button = |label: &'static str, value: f32, cx: &mut Context<Self>| {
            let bone = bone.clone();
            Self::control_button(label).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.set_axis_rest_role(&bone, role, value);
                    cx.notify();
                }),
            )
        };
        let adjust_button = |label: &'static str, delta: f32, cx: &mut Context<Self>| {
            let bone = bone.clone();
            Self::control_button(label).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.adjust_axis_rest_role(&bone, role, delta);
                    cx.notify();
                }),
            )
        };
        let clear_bone = bone.clone();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(px(78.0))
                    .text_xs()
                    .text_color(white().opacity(0.62))
                    .child(format!(
                        "rest{suffix}",
                        suffix = Self::rest_role_attr_suffix(role)
                    )),
            )
            .child(Self::dynamic_button(value, false))
            .child(adjust_button("-10", -10.0, cx))
            .child(adjust_button("-1", -1.0, cx))
            .child(adjust_button("+1", 1.0, cx))
            .child(adjust_button("+10", 10.0, cx))
            .child(set_button("-90", -90.0, cx))
            .child(set_button("-35", -35.0, cx))
            .child(set_button("0", 0.0, cx))
            .child(set_button("+35", 35.0, cx))
            .child(set_button("+90", 90.0, cx))
            .child(Self::control_button("Clear").on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.clear_axis_rest_role(&clear_bone, role);
                    cx.notify();
                }),
            ))
    }

    fn number_row(
        label: &'static str,
        value: f32,
        minus_large: impl Fn(&mut Self) + 'static,
        minus_small: impl Fn(&mut Self) + 'static,
        plus_small: impl Fn(&mut Self) + 'static,
        plus_large: impl Fn(&mut Self) + 'static,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        Self::number_row_custom(
            label,
            value,
            "-10",
            "-1",
            "+1",
            "+10",
            minus_large,
            minus_small,
            plus_small,
            plus_large,
            cx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn number_row_custom(
        label: &'static str,
        value: f32,
        minus_large_label: &'static str,
        minus_small_label: &'static str,
        plus_small_label: &'static str,
        plus_large_label: &'static str,
        minus_large: impl Fn(&mut Self) + 'static,
        minus_small: impl Fn(&mut Self) + 'static,
        plus_small: impl Fn(&mut Self) + 'static,
        plus_large: impl Fn(&mut Self) + 'static,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(Self::section_label(label))
            .child(Self::control_button(minus_large_label).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    let before = this.capture_snapshot();
                    minus_large(this);
                    this.invalidate_preview();
                    this.push_undo_snapshot_if_changed(before);
                    cx.notify();
                }),
            ))
            .child(Self::control_button(minus_small_label).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    let before = this.capture_snapshot();
                    minus_small(this);
                    this.invalidate_preview();
                    this.push_undo_snapshot_if_changed(before);
                    cx.notify();
                }),
            ))
            .child(Self::value_pill(Self::fmt(value)))
            .child(Self::control_button(plus_small_label).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    let before = this.capture_snapshot();
                    plus_small(this);
                    this.invalidate_preview();
                    this.push_undo_snapshot_if_changed(before);
                    cx.notify();
                }),
            ))
            .child(Self::control_button(plus_large_label).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    let before = this.capture_snapshot();
                    plus_large(this);
                    this.invalidate_preview();
                    this.push_undo_snapshot_if_changed(before);
                    cx.notify();
                }),
            ))
    }

    fn handle_position(&self, handle: CharacterDragHandle) -> (f32, f32) {
        let action = self.active_action();
        match handle {
            CharacterDragHandle::RightHand => (
                (0.50 + action.raise_side / 420.0).clamp(0.08, 0.88),
                (0.58 - action.raise_forward / 390.0).clamp(0.08, 0.88),
            ),
            CharacterDragHandle::RightElbow => (
                (0.50 + action.raise_side / 560.0).clamp(0.10, 0.88),
                (0.53 - action.raise_forward / 760.0 + action.elbow_bend / 900.0).clamp(0.10, 0.88),
            ),
            CharacterDragHandle::LeftFoot => (
                (0.45 + action.left_leg_forward / 430.0).clamp(0.12, 0.82),
                (0.78 - action.left_knee_bend / 520.0).clamp(0.50, 0.94),
            ),
            CharacterDragHandle::RightFoot => (
                (0.55 + action.right_leg_forward / 430.0).clamp(0.18, 0.88),
                (0.78 - action.right_knee_bend / 520.0).clamp(0.50, 0.94),
            ),
            CharacterDragHandle::Head => {
                ((0.50 + action.head_turn / 220.0).clamp(0.25, 0.75), 0.30)
            }
            CharacterDragHandle::Chest => {
                ((0.50 + action.chest_turn / 260.0).clamp(0.25, 0.75), 0.43)
            }
            CharacterDragHandle::RawBone => {
                let rotation = self.selected_raw_bone_rotation();
                (
                    (0.50 + rotation[1] / 420.0).clamp(0.12, 0.88),
                    (0.50 - rotation[0] / 420.0).clamp(0.12, 0.88),
                )
            }
        }
    }

    fn should_show_canonical_marker(&self, label: &str) -> bool {
        match self.active_selected_canonical_bone() {
            Some(selected) => selected == label,
            None => true,
        }
    }

    fn canonical_marker(
        &self,
        label: &'static str,
        x: f32,
        y: f32,
        accent: u32,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        if !self.should_show_canonical_marker(label) {
            return div();
        }
        let canonical = label.to_string();
        let canonical_for_move = canonical.clone();
        div()
            .absolute()
            .left(relative(x))
            .top(relative(y))
            .flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .w(px(16.0))
                    .h(px(16.0))
                    .rounded_full()
                    .border_2()
                    .border_color(white().opacity(0.82))
                    .bg(rgb(accent))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, evt: &MouseDownEvent, window, cx| {
                            this.focus_handle.focus(window);
                            if let Some(raw_name) = this
                                .mapped_raw_for_canonical(&canonical)
                                .map(str::to_string)
                            {
                                let selected_raw = this
                                    .active_raw_bones()
                                    .iter()
                                    .position(|bone| bone.name == raw_name);
                                this.set_active_selected_canonical_bone(Some(canonical.clone()));
                                this.set_active_selected_raw_bone(selected_raw);
                                this.begin_drag(CharacterDragHandle::RawBone, evt);
                                this.status_line = if this.pose_calibration_preset.is_some() {
                                    format!(
                                        "Dragging {canonical}. This writes semantic rest* values inside BoneAxisMap."
                                    )
                                } else {
                                    format!(
                                        "Dragging canonical {} through raw {}.",
                                        canonical,
                                        Self::display_bone_name(&raw_name),
                                    )
                                };
                            } else {
                                this.status_line =
                                    format!("Canonical {canonical} is not assigned to a raw bone.");
                            }
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(move |this, evt: &MouseMoveEvent, _, cx| {
                        let dragging = this.drag_state.is_some();
                        this.update_drag(evt);
                        if dragging {
                            this.ensure_preview_requested(cx);
                            cx.stop_propagation();
                        } else {
                            this.status_line =
                                format!("Canonical {canonical_for_move} is ready to drag.");
                        }
                        cx.notify();
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                            this.end_drag();
                            this.ensure_preview_requested(cx);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .px_2()
                    .h(px(20.0))
                    .rounded_md()
                    .border_1()
                    .border_color(white().opacity(0.12))
                    .bg(rgba(0x05091499))
                    .text_xs()
                    .text_color(white().opacity(0.70))
                    .flex()
                    .items_center()
                    .child(label),
            )
    }

    fn canonical_pose_overlay(&self, cx: &mut Context<Self>) -> gpui::Div {
        let right_hand = self.handle_position(CharacterDragHandle::RightHand);
        let right_elbow = self.handle_position(CharacterDragHandle::RightElbow);
        let left_foot = self.handle_position(CharacterDragHandle::LeftFoot);
        let right_foot = self.handle_position(CharacterDragHandle::RightFoot);

        div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .size_full()
            .child(self.canonical_marker("hips", 0.50, 0.56, 0xfbbf24, cx))
            .child(self.canonical_marker("spine", 0.50, 0.50, 0xfbbf24, cx))
            .child(self.canonical_marker(
                "chest",
                self.handle_position(CharacterDragHandle::Chest).0,
                self.handle_position(CharacterDragHandle::Chest).1,
                0x22c55e,
                cx,
            ))
            .child(self.canonical_marker("neck", 0.50, 0.36, 0xfbbf24, cx))
            .child(self.canonical_marker(
                "head",
                self.handle_position(CharacterDragHandle::Head).0,
                self.handle_position(CharacterDragHandle::Head).1,
                0xf97316,
                cx,
            ))
            .child(self.canonical_marker("shoulder_l", 0.42, 0.43, 0x34d399, cx))
            .child(self.canonical_marker("upper_arm_l", 0.37, 0.49, 0x34d399, cx))
            .child(self.canonical_marker("forearm_l", 0.33, 0.56, 0x34d399, cx))
            .child(self.canonical_marker("hand_l", 0.30, 0.63, 0x34d399, cx))
            .child(self.canonical_marker("shoulder_r", 0.58, 0.43, 0x60a5fa, cx))
            .child(self.canonical_marker(
                "upper_arm_r",
                ((right_hand.0 + right_elbow.0) * 0.5).clamp(0.08, 0.88),
                ((right_hand.1 + right_elbow.1) * 0.5 - 0.04).clamp(0.08, 0.88),
                0x60a5fa,
                cx,
            ))
            .child(self.canonical_marker("forearm_r", right_elbow.0, right_elbow.1, 0xa78bfa, cx))
            .child(self.canonical_marker("hand_r", right_hand.0, right_hand.1, 0x60a5fa, cx))
            .child(self.canonical_marker("upper_leg_l", 0.44, 0.63, 0x34d399, cx))
            .child(self.canonical_marker(
                "lower_leg_l",
                (left_foot.0 + 0.01).clamp(0.08, 0.88),
                (left_foot.1 - 0.10).clamp(0.08, 0.94),
                0x34d399,
                cx,
            ))
            .child(self.canonical_marker("foot_l", left_foot.0, left_foot.1, 0x34d399, cx))
            .child(self.canonical_marker(
                "toe_l",
                (left_foot.0 - 0.03).clamp(0.08, 0.88),
                (left_foot.1 + 0.05).clamp(0.08, 0.96),
                0x34d399,
                cx,
            ))
            .child(self.canonical_marker("upper_leg_r", 0.56, 0.63, 0xf59e0b, cx))
            .child(self.canonical_marker(
                "lower_leg_r",
                (right_foot.0 - 0.01).clamp(0.08, 0.88),
                (right_foot.1 - 0.10).clamp(0.08, 0.94),
                0xf59e0b,
                cx,
            ))
            .child(self.canonical_marker("foot_r", right_foot.0, right_foot.1, 0xf59e0b, cx))
            .child(self.canonical_marker(
                "toe_r",
                (right_foot.0 + 0.03).clamp(0.08, 0.88),
                (right_foot.1 + 0.05).clamp(0.08, 0.96),
                0xf59e0b,
                cx,
            ))
    }

    fn preview_card(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if let Some(err) = &self.preview_error {
            return div()
                .size_full()
                .rounded_lg()
                .border_1()
                .border_color(rgba(0xff6655cc))
                .bg(rgb(0x12080a))
                .p_4()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(white().opacity(0.8))
                .child(err.clone())
                .into_any_element();
        }
        if let Some((image, w, h)) = self.preview_image.as_ref() {
            let mut card = div()
                .size_full()
                .relative()
                .rounded_lg()
                .border_1()
                .border_color(white().opacity(0.14))
                .bg(rgb(0x05070c))
                .overflow_hidden()
                .on_scroll_wheel(cx.listener(|this, evt: &ScrollWheelEvent, _, cx| {
                    this.scroll_viewport(evt);
                    this.ensure_preview_requested(cx);
                    cx.stop_propagation();
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, evt: &MouseDownEvent, window, cx| {
                        this.focus_handle.focus(window);
                        if evt.modifiers.alt || evt.modifiers.shift {
                            this.begin_viewport_drag(ViewportDragMode::Pan, evt);
                        } else {
                            this.begin_viewport_drag(ViewportDragMode::Orbit, evt);
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, evt: &MouseDownEvent, window, cx| {
                        this.focus_handle.focus(window);
                        this.begin_viewport_drag(ViewportDragMode::Orbit, evt);
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .on_mouse_move(cx.listener(|this, evt: &MouseMoveEvent, _, cx| {
                    let pose_dragging = this.drag_state.is_some();
                    let viewport_dragging = this.viewport_drag_state.is_some();
                    this.update_drag(evt);
                    this.update_viewport_drag(evt);
                    if pose_dragging || viewport_dragging {
                        this.ensure_preview_requested(cx);
                        cx.stop_propagation();
                    }
                    cx.notify();
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                        this.end_drag();
                        this.end_viewport_drag();
                        this.ensure_preview_requested(cx);
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .on_mouse_up(
                    MouseButton::Right,
                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                        this.end_viewport_drag();
                        this.ensure_preview_requested(cx);
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .child(FitPreviewImageElement::new(image.clone(), *w, *h));
            if self.show_canonical_pose {
                card = card.child(self.canonical_pose_overlay(cx));
            }
            return card
                .child(
                    div()
                        .absolute()
                        .left(px(12.0))
                        .bottom(px(12.0))
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(rgba(0x050914cc))
                        .text_xs()
                        .text_color(white().opacity(0.70))
                        .child(if self.show_canonical_pose {
                            "Drag bone dots to pose. Left-drag empty space changes Camera yaw/pitch. Option/Shift-left-drag or horizontal two-finger scroll pans Camera x/y. Vertical two-finger scroll zooms Camera distance."
                        } else {
                            "Bone dots hidden. Left-drag empty space changes Camera yaw/pitch. Option/Shift-left-drag or horizontal two-finger scroll pans Camera x/y. Vertical two-finger scroll zooms Camera distance."
                        }),
                )
                .into_any_element();
        }
        div()
            .size_full()
            .rounded_lg()
            .border_1()
            .border_color(white().opacity(0.14))
            .bg(rgb(0x05070c))
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(white().opacity(0.62))
            .child(if self.model_path.trim().is_empty() {
                "Load a GLB to begin 3D layout."
            } else if self.preview_pending {
                "Preview rendering..."
            } else {
                "Waiting for GLB preview..."
            })
            .into_any_element()
    }
}

impl Focusable for CharacterDesignPage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CharacterDesignPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.normalize_additional_model_state();
        self.ensure_action_dsl_input(window, cx);
        self.sync_action_dsl_input_if_needed(window, cx);
        self.ensure_timeline_inputs(window, cx);
        self.sync_timeline_inputs_if_needed(window, cx);
        self.ensure_preview_requested(cx);
        let model_label = if self.model_path.trim().is_empty() {
            "No GLB loaded".to_string()
        } else {
            format!(
                "{} GLB(s) loaded · editing {}",
                self.additional_model_paths.len() + 1,
                self.selected_actor_label()
            )
        };
        let status_line = if self.preview_pending {
            "GPU preview rendering with cached renderer...".to_string()
        } else {
            self.status_line.clone()
        };

        let keyframes = self.action_keyframes();
        let active_action_dsl = self.active_action_dsl().to_string();
        let action_time = if active_action_dsl.trim().is_empty() {
            self.frame_time_seconds()
        } else {
            self.current_action_time(&active_action_dsl)
        };
        let mut keyframe_row = div().flex().flex_wrap().gap_2();
        if keyframes.is_empty() {
            keyframe_row = keyframe_row.child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.46))
                    .child("No keyframes yet. Add Current Frame creates an <Action> keyframe."),
            );
        } else {
            for t in keyframes {
                let selected = self
                    .active_selected_action_keyframe_t()
                    .map(|selected| (selected - t).abs() < 0.001)
                    .unwrap_or_else(|| (action_time - t).abs() < 0.001);
                keyframe_row = keyframe_row.child(
                    Self::dynamic_button(format!("t={}", Self::fmt(t)), selected).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.select_action_keyframe(t);
                            cx.notify();
                        }),
                    ),
                );
            }
        }

        let frame_input_elem = if let Some(input) = self.frame_input.as_ref() {
            let input_entity = input.clone();
            div()
                .h(px(34.0))
                .w(px(92.0))
                .rounded_md()
                .border_1()
                .border_color(white().opacity(0.14))
                .bg(rgb(0x080b12))
                .overflow_hidden()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |_, _, window, cx| {
                        input_entity.read(cx).focus_handle(cx).focus(window);
                        cx.stop_propagation();
                    }),
                )
                .child(Input::new(input).h(px(34.0)).w(px(92.0)))
                .into_any_element()
        } else {
            Self::value_pill(format!("Frame {}", self.frame)).into_any_element()
        };

        let duration_input_elem = if let Some(input) = self.duration_input.as_ref() {
            let input_entity = input.clone();
            div()
                .h(px(34.0))
                .w(px(86.0))
                .rounded_md()
                .border_1()
                .border_color(white().opacity(0.14))
                .bg(rgb(0x080b12))
                .overflow_hidden()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |_, _, window, cx| {
                        input_entity.read(cx).focus_handle(cx).focus(window);
                        cx.stop_propagation();
                    }),
                )
                .child(Input::new(input).h(px(34.0)).w(px(86.0)))
                .into_any_element()
        } else {
            Self::value_pill(Self::fmt(self.duration_sec)).into_any_element()
        };

        let graph_fps_input_elem = if let Some(input) = self.graph_fps_input.as_ref() {
            Self::input_pill(input, 76.0, cx)
        } else {
            Self::value_pill(self.graph_fps.to_string()).into_any_element()
        };
        let graph_width_input_elem = if let Some(input) = self.graph_width_input.as_ref() {
            Self::input_pill(input, 86.0, cx)
        } else {
            Self::value_pill(self.graph_width.to_string()).into_any_element()
        };
        let graph_height_input_elem = if let Some(input) = self.graph_height_input.as_ref() {
            Self::input_pill(input, 86.0, cx)
        } else {
            Self::value_pill(self.graph_height.to_string()).into_any_element()
        };
        let render_width_input_elem = if let Some(input) = self.render_width_input.as_ref() {
            Self::input_pill(input, 86.0, cx)
        } else {
            Self::value_pill(self.render_width.to_string()).into_any_element()
        };
        let render_height_input_elem = if let Some(input) = self.render_height_input.as_ref() {
            Self::input_pill(input, 86.0, cx)
        } else {
            Self::value_pill(self.render_height.to_string()).into_any_element()
        };

        let graph_settings_panel = div()
            .mt_3()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.12))
            .bg(rgb(0x090d15))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(white().opacity(0.92))
                    .child("Graph"),
            )
            .child(div().text_xs().text_color(white().opacity(0.58)).child(
                "Auto-applies 1.5s after typing. Exports to <Graph fps duration size renderSize>.",
            ))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(Self::value_pill("fps".to_string()))
                    .child(graph_fps_input_elem)
                    .child(Self::value_pill("duration".to_string()))
                    .child(duration_input_elem),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(Self::value_pill("size w".to_string()))
                    .child(graph_width_input_elem)
                    .child(Self::value_pill("h".to_string()))
                    .child(graph_height_input_elem),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(Self::value_pill("render w".to_string()))
                    .child(render_width_input_elem)
                    .child(Self::value_pill("h".to_string()))
                    .child(render_height_input_elem),
            )
            .child(div().text_xs().text_color(white().opacity(0.54)).child(
                if self.background_image_path.trim().is_empty() {
                    "Background: solid #101827".to_string()
                } else {
                    format!("Background: {}", self.background_image_path)
                },
            ));

        let frame_limit_label = format!("/ {}", self.total_frames().saturating_sub(1));
        let keyframe_controls = div()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.12))
            .bg(rgb(0x090d15))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .flex_wrap()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(white().opacity(0.92))
                                    .child("Keyframes"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.54))
                                    .child("linear interpolation"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(Self::control_button("F-1").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.frame = this.frame.saturating_sub(1);
                                    this.set_active_selected_action_keyframe_t(None);
                                    this.apply_action_dsl_pose_to_controls();
                                    this.invalidate_preview();
                                    cx.notify();
                                }),
                            ))
                            .child(frame_input_elem)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.50))
                                    .child(frame_limit_label),
                            )
                            .child(Self::control_button("F+1").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.frame = (this.frame + 1) % this.total_frames();
                                    this.set_active_selected_action_keyframe_t(None);
                                    this.apply_action_dsl_pose_to_controls();
                                    this.invalidate_preview();
                                    cx.notify();
                                }),
                            ))
                            .child(
                                div()
                                    .ml_2()
                                    .text_xs()
                                    .text_color(white().opacity(0.58))
                                    .child("seconds"),
                            )
                            .child(Self::value_pill(Self::fmt(self.duration_sec)))
                            .child(Self::control_button("Add Current Frame").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    let before = this.capture_snapshot();
                                    this.add_keyframe_at_current_frame();
                                    this.invalidate_preview();
                                    this.sync_action_dsl_input_if_needed(window, cx);
                                    this.push_undo_snapshot_if_changed(before);
                                    cx.notify();
                                }),
                            ))
                            .child(Self::control_button("Update Keyframe").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    let before = this.capture_snapshot();
                                    this.update_selected_action_keyframe();
                                    this.invalidate_preview();
                                    this.sync_action_dsl_input_if_needed(window, cx);
                                    this.push_undo_snapshot_if_changed(before);
                                    cx.notify();
                                }),
                            )),
                    ),
            )
            .child(keyframe_row);

        let preview_panel = div()
            .flex_1()
            .max_w(relative(0.65))
            .min_w_0()
            .min_h_0()
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(white().opacity(0.95))
                                    .child("3D Layout"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.62))
                                    .child(model_label),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(
                                Self::control_button(if self.playing { "Pause" } else { "Play" })
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.playing = !this.playing;
                                            this.play_token = this.play_token.wrapping_add(1);
                                            if this.playing {
                                                this.schedule_play_tick(cx);
                                            }
                                            cx.notify();
                                        }),
                                    ),
                            )
                            .child(Self::control_button("Load GLB").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_this, _, win, cx| {
                                    let rx = cx.prompt_for_paths(PathPromptOptions {
                                        files: true,
                                        directories: false,
                                        multiple: false,
                                        prompt: Some("Load GLB".into()),
                                    });
                                    cx.spawn_in(win, async move |view, window| {
                                        let Ok(result) = rx.await else {
                                            return;
                                        };
                                        let Some(paths) = result.ok().flatten() else {
                                            return;
                                        };
                                        let Some(path) = paths.into_iter().next() else {
                                            return;
                                        };
                                        let _ = view.update_in(window, |this, _window, cx| {
                                            let before = this.capture_snapshot();
                                            this.add_model_path(path);
                                            this.push_undo_snapshot_if_changed(before);
                                            cx.notify();
                                        });
                                    })
                                    .detach();
                                }),
                            ))
                            .child(
                                Self::control_button("Load Static Background").on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|_this, _, win, cx| {
                                        let rx = cx.prompt_for_paths(PathPromptOptions {
                                            files: true,
                                            directories: false,
                                            multiple: false,
                                            prompt: Some("Load Static Background Image".into()),
                                        });
                                        cx.spawn_in(win, async move |view, window| {
                                            let Ok(result) = rx.await else {
                                                return;
                                            };
                                            let Some(paths) = result.ok().flatten() else {
                                                return;
                                            };
                                            let Some(path) = paths.into_iter().next() else {
                                                return;
                                            };
                                            let _ = view.update_in(window, |this, _window, cx| {
                                                let before = this.capture_snapshot();
                                                this.set_static_background_path(path);
                                                this.push_undo_snapshot_if_changed(before);
                                                cx.notify();
                                            });
                                        })
                                        .detach();
                                    }),
                                ),
                            )
                            .child(
                                Self::control_button(if self.show_canonical_pose {
                                    "Hide Bone Dots"
                                } else {
                                    "Show Bone Dots"
                                })
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        this.show_canonical_pose = !this.show_canonical_pose;
                                        cx.notify();
                                    }),
                                ),
                            )
                            .child(Self::control_button("Reset View").on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.reset_viewport();
                                    cx.notify();
                                }),
                            )),
                    ),
            )
            .child(div().flex_1().min_h_0().child(self.preview_card(cx)))
            .child(keyframe_controls)
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.58))
                    .child(status_line),
            );

        let mut canonical_selector =
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(white().opacity(0.92))
                        .child("Retarget"),
                )
                .child(div().text_xs().text_color(white().opacity(0.58)).child(
                    "Map raw GLB bones to humanoid_v1 ids. This exports the <Retarget> tag.",
                ));
        let mut canonical_selector_grid = div().flex().flex_wrap().gap_2();
        for canonical in Self::humanoid_bones() {
            let raw = self.mapped_raw_for_canonical(canonical).map(str::to_string);
            let mapped_label = raw
                .as_deref()
                .map(Self::display_bone_name)
                .unwrap_or_else(|| "unassigned".to_string());
            let selected = self.active_selected_canonical_bone() == Some(*canonical);
            let label = format!("{canonical}: {mapped_label}");
            let canonical = (*canonical).to_string();
            canonical_selector_grid = canonical_selector_grid.child(
                Self::dynamic_button(label, selected).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if this.active_selected_canonical_bone() == Some(canonical.as_str()) {
                            this.set_active_selected_canonical_bone(None);
                            this.status_line = "Showing all canonical bones in preview.".to_string();
                            cx.notify();
                            return;
                        }

                        this.set_active_selected_canonical_bone(Some(canonical.clone()));
                        if let Some(raw_name) = this
                            .mapped_raw_for_canonical(&canonical)
                            .map(str::to_string)
                        {
                            let selected_raw = this
                                .active_raw_bones()
                                .iter()
                                .position(|bone| bone.name == raw_name);
                            this.set_active_selected_raw_bone(selected_raw);
                            this.status_line = format!(
                                "Selected canonical {} via raw {}. Drag its preview dot to pose.",
                                canonical,
                                Self::display_bone_name(&raw_name),
                            );
                        } else {
                            this.status_line = format!(
                                "Selected unassigned canonical {canonical}. Select a raw bone below and assign it first."
                            );
                        }
                        cx.notify();
                    }),
                ),
            );
        }
        canonical_selector = canonical_selector.child(canonical_selector_grid);

        let active_actor_position = self.active_actor_position();
        let active_actor_rotation = self.active_actor_rotation();
        let model_transform_controls = div()
            .mt_3()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.12))
            .bg(rgb(0x090d15))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(white().opacity(0.92))
                    .child("Model Transform"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.58))
                    .child(format!(
                        "Editing {}. These values export to the selected <Actor x y z yaw pitch roll>.",
                        self.selected_actor_label()
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(Self::value_pill(format!(
                        "x {}",
                        Self::fmt(active_actor_position[0])
                    )))
                    .child(Self::value_pill(format!(
                        "y {}",
                        Self::fmt(active_actor_position[1])
                    )))
                    .child(Self::value_pill(format!(
                        "z {}",
                        Self::fmt(active_actor_position[2])
                    )))
                    .child(Self::value_pill(format!(
                        "yaw {}",
                        Self::fmt(active_actor_rotation[0])
                    )))
                    .child(Self::value_pill(format!(
                        "pitch {}",
                        Self::fmt(active_actor_rotation[1])
                    )))
                    .child(Self::value_pill(format!(
                        "roll {}",
                        Self::fmt(active_actor_rotation[2])
                    ))),
            )
            .child(Self::number_row_custom(
                "model x",
                active_actor_position[0],
                "-1",
                "-0.1",
                "+0.1",
                "+1",
                |this| this.adjust_active_actor_position(0, -1.0),
                |this| this.adjust_active_actor_position(0, -0.1),
                |this| this.adjust_active_actor_position(0, 0.1),
                |this| this.adjust_active_actor_position(0, 1.0),
                cx,
            ))
            .child(Self::number_row_custom(
                "model y",
                active_actor_position[1],
                "-1",
                "-0.1",
                "+0.1",
                "+1",
                |this| this.adjust_active_actor_position(1, -1.0),
                |this| this.adjust_active_actor_position(1, -0.1),
                |this| this.adjust_active_actor_position(1, 0.1),
                |this| this.adjust_active_actor_position(1, 1.0),
                cx,
            ))
            .child(Self::number_row_custom(
                "model z",
                active_actor_position[2],
                "-1",
                "-0.1",
                "+0.1",
                "+1",
                |this| this.adjust_active_actor_position(2, -1.0),
                |this| this.adjust_active_actor_position(2, -0.1),
                |this| this.adjust_active_actor_position(2, 0.1),
                |this| this.adjust_active_actor_position(2, 1.0),
                cx,
            ))
            .child(Self::number_row_custom(
                "yaw",
                active_actor_rotation[0],
                "-45",
                "-5",
                "+5",
                "+45",
                |this| this.adjust_active_actor_rotation(0, -45.0),
                |this| this.adjust_active_actor_rotation(0, -5.0),
                |this| this.adjust_active_actor_rotation(0, 5.0),
                |this| this.adjust_active_actor_rotation(0, 45.0),
                cx,
            ))
            .child(Self::number_row_custom(
                "pitch",
                active_actor_rotation[1],
                "-45",
                "-5",
                "+5",
                "+45",
                |this| this.adjust_active_actor_rotation(1, -45.0),
                |this| this.adjust_active_actor_rotation(1, -5.0),
                |this| this.adjust_active_actor_rotation(1, 5.0),
                |this| this.adjust_active_actor_rotation(1, 45.0),
                cx,
            ))
            .child(Self::number_row_custom(
                "roll",
                active_actor_rotation[2],
                "-45",
                "-5",
                "+5",
                "+45",
                |this| this.adjust_active_actor_rotation(2, -45.0),
                |this| this.adjust_active_actor_rotation(2, -5.0),
                |this| this.adjust_active_actor_rotation(2, 5.0),
                |this| this.adjust_active_actor_rotation(2, 45.0),
                cx,
            ));

        let camera_controls = div()
            .mt_3()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.12))
            .bg(rgb(0x090d15))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(white().opacity(0.92))
                    .child("Camera"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.58))
                    .child("Left-drag empty preview space changes Camera yaw/pitch only; actor yaw/model transform is not edited. Shift/Option-drag pans Camera x/y. Copy Formal DSL exports these <Camera> values."),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(Self::value_pill(format!("x {}", Self::fmt(self.camera_x))))
                    .child(Self::value_pill(format!("y {}", Self::fmt(self.camera_y))))
                    .child(Self::value_pill(format!("z {}", Self::fmt(self.camera_z))))
                    .child(Self::value_pill(format!("yaw {}", Self::fmt(self.camera_yaw))))
                    .child(Self::value_pill(format!("pitch {}", Self::fmt(self.camera_pitch))))
                    .child(Self::value_pill(format!(
                        "distance {}",
                        Self::fmt(self.camera_distance)
                    ))),
            )
            .child(Self::number_row_custom(
                "cam x",
                self.camera_x,
                "-1",
                "-0.1",
                "+0.1",
                "+1",
                |this| this.camera_x = (this.camera_x - 1.0).clamp(-8.0, 8.0),
                |this| this.camera_x = (this.camera_x - 0.1).clamp(-8.0, 8.0),
                |this| this.camera_x = (this.camera_x + 0.1).clamp(-8.0, 8.0),
                |this| this.camera_x = (this.camera_x + 1.0).clamp(-8.0, 8.0),
                cx,
            ))
            .child(Self::number_row_custom(
                "cam y",
                self.camera_y,
                "-1",
                "-0.1",
                "+0.1",
                "+1",
                |this| this.camera_y = (this.camera_y - 1.0).clamp(-8.0, 8.0),
                |this| this.camera_y = (this.camera_y - 0.1).clamp(-8.0, 8.0),
                |this| this.camera_y = (this.camera_y + 0.1).clamp(-8.0, 8.0),
                |this| this.camera_y = (this.camera_y + 1.0).clamp(-8.0, 8.0),
                cx,
            ))
            .child(Self::number_row_custom(
                "cam z",
                self.camera_z,
                "-1",
                "-0.1",
                "+0.1",
                "+1",
                |this| this.camera_z = (this.camera_z - 1.0).clamp(-8.0, 8.0),
                |this| this.camera_z = (this.camera_z - 0.1).clamp(-8.0, 8.0),
                |this| this.camera_z = (this.camera_z + 0.1).clamp(-8.0, 8.0),
                |this| this.camera_z = (this.camera_z + 1.0).clamp(-8.0, 8.0),
                cx,
            ))
            .child(Self::number_row_custom(
                "yaw",
                self.camera_yaw,
                "-45",
                "-5",
                "+5",
                "+45",
                |this| this.camera_yaw = (this.camera_yaw - 45.0).rem_euclid(360.0),
                |this| this.camera_yaw = (this.camera_yaw - 5.0).rem_euclid(360.0),
                |this| this.camera_yaw = (this.camera_yaw + 5.0).rem_euclid(360.0),
                |this| this.camera_yaw = (this.camera_yaw + 45.0).rem_euclid(360.0),
                cx,
            ))
            .child(Self::number_row_custom(
                "pitch",
                self.camera_pitch,
                "-15",
                "-1",
                "+1",
                "+15",
                |this| this.camera_pitch = (this.camera_pitch - 15.0).clamp(-80.0, 80.0),
                |this| this.camera_pitch = (this.camera_pitch - 1.0).clamp(-80.0, 80.0),
                |this| this.camera_pitch = (this.camera_pitch + 1.0).clamp(-80.0, 80.0),
                |this| this.camera_pitch = (this.camera_pitch + 15.0).clamp(-80.0, 80.0),
                cx,
            ))
            .child(Self::number_row_custom(
                "distance",
                self.camera_distance,
                "-1",
                "-0.1",
                "+0.1",
                "+1",
                |this| this.camera_distance = (this.camera_distance - 1.0).clamp(0.6, 14.0),
                |this| this.camera_distance = (this.camera_distance - 0.1).clamp(0.6, 14.0),
                |this| this.camera_distance = (this.camera_distance + 0.1).clamp(0.6, 14.0),
                |this| this.camera_distance = (this.camera_distance + 1.0).clamp(0.6, 14.0),
                cx,
            ));

        let mut loaded_glb_list = div().flex().flex_col().gap_2();
        if self.model_path.trim().is_empty() {
            loaded_glb_list = loaded_glb_list.child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.46))
                    .child("No GLB loaded."),
            );
        } else {
            for slot in 0..=self.additional_model_paths.len() {
                let selected = self.selected_actor_slot == slot;
                let name = self.actor_label_for_slot(slot);
                let path = if slot == 0 {
                    self.model_path.clone()
                } else {
                    self.additional_model_paths[slot - 1].clone()
                };
                let position = self.actor_position_for_slot(slot);
                let rotation = self.actor_rotation_for_slot(slot);
                loaded_glb_list = loaded_glb_list.child(
                    div()
                        .rounded_md()
                        .border_1()
                        .border_color(if selected {
                            rgba(0x70d6ff99)
                        } else {
                            rgba(0xffffff1f)
                        })
                        .bg(rgb(0x0b1020))
                        .p_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(white().opacity(0.82))
                                        .child(name),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(white().opacity(0.46))
                                        .child(path),
                                )
                                .child(div().text_xs().text_color(white().opacity(0.46)).child(
                                    format!(
                                        "x {} · y {} · z {}",
                                        Self::fmt(position[0]),
                                        Self::fmt(position[1]),
                                        Self::fmt(position[2])
                                    ),
                                ))
                                .child(div().text_xs().text_color(white().opacity(0.46)).child(
                                    format!(
                                        "yaw {} · pitch {} · roll {}",
                                        Self::fmt(rotation[0]),
                                        Self::fmt(rotation[1]),
                                        Self::fmt(rotation[2])
                                    ),
                                )),
                        )
                        .child(
                            Self::dynamic_button(
                                if selected { "Editing" } else { "Select" }.to_string(),
                                selected,
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.select_actor_slot(slot);
                                    this.sync_action_dsl_input_if_needed(window, cx);
                                    cx.notify();
                                }),
                            ),
                        )
                        .child(Self::control_button("Delete").on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                let before = this.capture_snapshot();
                                this.remove_model_slot(slot);
                                this.push_undo_snapshot_if_changed(before);
                                cx.notify();
                            }),
                        )),
                );
            }
        }

        let loaded_glb_panel = div()
            .mt_3()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.12))
            .bg(rgb(0x090d15))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(white().opacity(0.92))
                    .child("Loaded GLBs"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.58))
                    .child(format!(
                        "{} GLB(s) loaded. Use Load GLB again to add more.",
                        if self.model_path.trim().is_empty() {
                            0
                        } else {
                            self.additional_model_paths.len() + 1
                        }
                    )),
            )
            .child(loaded_glb_list);

        let selected_raw_label = self
            .selected_raw_bone_name()
            .map(Self::display_bone_name)
            .unwrap_or_else(|| "none".to_string());
        let selected_raw_rotation = self.selected_raw_bone_rotation();
        let mut raw_bone_list = div()
            .h(px(190.0))
            .overflow_y_scrollbar()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.12))
            .bg(rgb(0x090d15))
            .p_2()
            .flex()
            .flex_col()
            .gap_1();
        if self.active_raw_bones().is_empty() {
            raw_bone_list = raw_bone_list.child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.46))
                    .child("Load a GLB to show raw bones."),
            );
        } else {
            for (list_index, bone) in self.active_raw_bones().iter().enumerate() {
                let selected = self.active_selected_raw_bone() == Some(list_index);
                let mapped = self
                    .mapped_canonical_for_raw(&bone.name)
                    .map(|canonical| format!(" -> {canonical}"))
                    .unwrap_or_default();
                let label = format!(
                    "#{:03} {}{}",
                    bone.index,
                    Self::display_bone_name(&bone.name),
                    mapped
                );
                raw_bone_list =
                    raw_bone_list.child(Self::dynamic_button(label, selected).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.set_active_selected_raw_bone(Some(list_index));
                            this.status_line =
                                "Selected raw bone for direct rotate/drag.".to_string();
                            cx.notify();
                        }),
                    ));
            }
        }

        let mut canonical_grid = div().flex().flex_wrap().gap_2();
        for canonical in Self::humanoid_bones() {
            let raw_display = self
                .mapped_raw_for_canonical(canonical)
                .map(Self::display_bone_name)
                .unwrap_or_else(|| "unassigned".to_string());
            let selected_raw_maps_here = self
                .selected_raw_bone_name()
                .and_then(|raw| self.mapped_canonical_for_raw(raw))
                .map(|mapped| mapped == *canonical)
                .unwrap_or(false);
            let label = format!("{canonical}: {raw_display}");
            let canonical = (*canonical).to_string();
            canonical_grid = canonical_grid.child(
                Self::dynamic_button(label, selected_raw_maps_here).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.assign_selected_raw_to(&canonical);
                        cx.notify();
                    }),
                ),
            );
        }

        let raw_bone_controls = div()
            .mt_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(white().opacity(0.92))
                    .child("Raw Bone Control"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.58))
                    .child(format!("Selected: {selected_raw_label}")),
            )
            .child(raw_bone_list)
            .child(Self::number_row(
                "raw rot X",
                selected_raw_rotation[0],
                |this| this.adjust_selected_raw_rotation(0, -10.0),
                |this| this.adjust_selected_raw_rotation(0, -1.0),
                |this| this.adjust_selected_raw_rotation(0, 1.0),
                |this| this.adjust_selected_raw_rotation(0, 10.0),
                cx,
            ))
            .child(Self::number_row(
                "raw rot Y",
                selected_raw_rotation[1],
                |this| this.adjust_selected_raw_rotation(1, -10.0),
                |this| this.adjust_selected_raw_rotation(1, -1.0),
                |this| this.adjust_selected_raw_rotation(1, 1.0),
                |this| this.adjust_selected_raw_rotation(1, 10.0),
                cx,
            ))
            .child(Self::number_row(
                "raw rot Z",
                selected_raw_rotation[2],
                |this| this.adjust_selected_raw_rotation(2, -10.0),
                |this| this.adjust_selected_raw_rotation(2, -1.0),
                |this| this.adjust_selected_raw_rotation(2, 1.0),
                |this| this.adjust_selected_raw_rotation(2, 10.0),
                cx,
            ))
            .child(
                div()
                    .mt_2()
                    .text_xs()
                    .text_color(white().opacity(0.58))
                    .child("Assign selected raw bone to canonical bone:"),
            )
            .child(canonical_grid)
            .child(
                Self::control_button("Clear Selected Canonical").on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        if let Some(raw) = this.selected_raw_bone_name().map(str::to_string)
                            && let Some(canonical) =
                                this.mapped_canonical_for_raw(&raw).map(str::to_string)
                        {
                            this.clear_canonical_assignment(&canonical);
                        } else {
                            this.status_line = "Selected raw bone is not assigned.".to_string();
                        }
                        cx.notify();
                    }),
                ),
            );

        let mut test_action_buttons = div().flex().flex_wrap().gap_2();
        let active_test_action_preset = self.active_test_action_preset();
        for preset in [
            RetargetPosePreset::Original,
            RetargetPosePreset::ArmsDown,
            RetargetPosePreset::APose,
            RetargetPosePreset::TPose,
            RetargetPosePreset::Walk,
            RetargetPosePreset::Jump,
            RetargetPosePreset::WaveHand,
            RetargetPosePreset::SidePlus20,
            RetargetPosePreset::SideMinus20,
            RetargetPosePreset::ForwardPlus20,
            RetargetPosePreset::BendPlus20,
            RetargetPosePreset::TwistPlus20,
        ] {
            test_action_buttons = test_action_buttons.child(
                Self::dynamic_button(
                    preset.label().to_string(),
                    active_test_action_preset == preset,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.apply_test_action_preset(preset, window, cx);
                        cx.notify();
                    }),
                ),
            );
        }
        let test_action_controls = div()
            .mt_4()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.12))
            .bg(rgb(0x090d15))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(white().opacity(0.92))
                    .child("Test Action"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.58))
                    .child("Applies a temporary Action DSL to the selected GLB. +20 tests are for BoneAxis direction; Walk/Jump/Wave test reusable humanoid_v1 actions."),
            )
            .child(test_action_buttons);

        let axis_bone = self.selected_axis_bone();
        let counterpart_label = Self::counterpart_bone(&axis_bone)
            .map(|bone| format!("Copy Exact To {bone}"))
            .unwrap_or_else(|| "No Counterpart".to_string());
        let mirrored_label = Self::counterpart_bone(&axis_bone)
            .map(|bone| format!("Copy Mirrored To {bone}"))
            .unwrap_or_else(|| "No Counterpart".to_string());
        let copy_axis_bone = axis_bone.clone();
        let mirror_axis_bone = axis_bone.clone();
        let mut quick_axis_buttons = div().flex().flex_wrap().gap_2();
        for (label, bone) in [
            ("R Arm", "upper_arm_r"),
            ("R Forearm", "forearm_r"),
            ("R Hand", "hand_r"),
            ("L Arm", "upper_arm_l"),
            ("L Forearm", "forearm_l"),
            ("L Hand", "hand_l"),
            ("R Thigh", "upper_leg_r"),
            ("R Knee", "lower_leg_r"),
            ("R Foot", "foot_r"),
            ("L Thigh", "upper_leg_l"),
            ("L Knee", "lower_leg_l"),
            ("L Foot", "foot_l"),
        ] {
            let bone = bone.to_string();
            quick_axis_buttons = quick_axis_buttons.child(
                Self::dynamic_button(label.to_string(), axis_bone == bone).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.set_active_selected_canonical_bone(Some(bone.clone()));
                        this.status_line =
                            format!("Editing {} BoneAxis {}.", this.selected_actor_label(), bone);
                        cx.notify();
                    }),
                ),
            );
        }
        let bone_axis_panel = div()
            .mt_4()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.12))
            .bg(rgb(0x090d15))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(white().opacity(0.92))
                    .child("BoneAxis"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.58))
                    .child(format!(
                        "Editing {} bone: {}. Axis rows export turn/bend/forward/side/twist; rest rows export restTurn/restBend/restForward/restSide/restTwist on the same <Axis> tag.",
                        self.selected_actor_label(),
                        axis_bone
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(Self::dynamic_button(counterpart_label, false).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.copy_axis_to_counterpart(&copy_axis_bone, false);
                            cx.notify();
                        }),
                    ))
                    .child(Self::dynamic_button(mirrored_label, false).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.copy_axis_to_counterpart(&mirror_axis_bone, true);
                            cx.notify();
                        }),
                    )),
            )
            .child(quick_axis_buttons)
            .child(
                div()
                    .pt_2()
                    .text_xs()
                    .text_color(white().opacity(0.72))
                    .child("Action semantic axis mapping"),
            )
            .child(Self::axis_role_row(
                axis_bone.clone(),
                "turn",
                self.axis_role_value(&axis_bone, "turn"),
                cx,
            ))
            .child(Self::axis_role_row(
                axis_bone.clone(),
                "bend",
                self.axis_role_value(&axis_bone, "bend"),
                cx,
            ))
            .child(Self::axis_role_row(
                axis_bone.clone(),
                "forward",
                self.axis_role_value(&axis_bone, "forward"),
                cx,
            ))
            .child(Self::axis_role_row(
                axis_bone.clone(),
                "side",
                self.axis_role_value(&axis_bone, "side"),
                cx,
            ))
            .child(Self::axis_role_row(
                axis_bone.clone(),
                "twist",
                self.axis_role_value(&axis_bone, "twist"),
                cx,
            ))
            .child(
                div()
                    .pt_3()
                    .text_xs()
                    .text_color(white().opacity(0.72))
                    .child("Rest offsets inside BoneAxisMap"),
            )
            .child(Self::axis_rest_role_row(
                axis_bone.clone(),
                "turn",
                self.axis_rest_role_value(&axis_bone, "turn"),
                cx,
            ))
            .child(Self::axis_rest_role_row(
                axis_bone.clone(),
                "bend",
                self.axis_rest_role_value(&axis_bone, "bend"),
                cx,
            ))
            .child(Self::axis_rest_role_row(
                axis_bone.clone(),
                "forward",
                self.axis_rest_role_value(&axis_bone, "forward"),
                cx,
            ))
            .child(Self::axis_rest_role_row(
                axis_bone.clone(),
                "side",
                self.axis_rest_role_value(&axis_bone, "side"),
                cx,
            ))
            .child(Self::axis_rest_role_row(
                axis_bone.clone(),
                "twist",
                self.axis_rest_role_value(&axis_bone, "twist"),
                cx,
            ));

        let action_dsl_input_elem = if let Some(input) = self.action_dsl_input.as_ref() {
            let input_entity = input.clone();
            div()
                .h(px(420.0))
                .w(px(520.0))
                .min_w(px(520.0))
                .flex_shrink_0()
                .rounded_md()
                .border_1()
                .border_color(white().opacity(0.18))
                .bg(rgb(0x0b1020))
                .overflow_hidden()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |_, _, window, cx| {
                        input_entity.read(cx).focus_handle(cx).focus(window);
                        cx.stop_propagation();
                    }),
                )
                .child(Input::new(input).h(px(420.0)).w(px(520.0)).flex_shrink_0())
                .into_any_element()
        } else {
            div()
                .h(px(420.0))
                .w(px(520.0))
                .min_w(px(520.0))
                .flex_shrink_0()
                .rounded_md()
                .border_1()
                .border_color(white().opacity(0.12))
                .bg(rgb(0x090d15))
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .text_color(white().opacity(0.46))
                .child("Preparing Action DSL editor...")
                .into_any_element()
        };
        let action_dsl_editor = div()
            .mt_4()
            .w(px(520.0))
            .min_w(px(520.0))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(white().opacity(0.92))
                    .child("Action DSL"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.58))
                    .child("Paste only an <Action> block here to test motion. Empty editor uses the current canonical/raw pose."),
            )
            .child(action_dsl_input_elem)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(Self::control_button("Clear Action DSL").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.set_action_dsl_text(String::new(), window, cx);
                            this.status_line =
                                "Action DSL cleared; preview uses current pose.".to_string();
                            cx.notify();
                        }),
                    )),
            );

        let mut hide_controls = div()
            .mt_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(white().opacity(0.92))
                    .child("Mesh / Material Hide"),
            )
            .child(div().text_xs().text_color(white().opacity(0.58)).child(
                "Hide Sketchfab leftovers such as outline shells or wall/background meshes.",
            ));
        if self.active_material_names().is_empty() && self.active_mesh_names().is_empty() {
            hide_controls = hide_controls.child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.46))
                    .child("Load a GLB to inspect mesh/material names."),
            );
        } else {
            let mut material_row = div().flex().flex_wrap().gap_2().items_center().child(
                div()
                    .w(px(70.0))
                    .text_xs()
                    .text_color(white().opacity(0.52))
                    .child("material"),
            );
            for name in self.active_material_names().iter().take(18).cloned() {
                let hidden = self.active_hidden_materials_contains(&name);
                let button_label = if hidden {
                    format!("hide {name}")
                } else {
                    name.clone()
                };
                material_row =
                    material_row.child(Self::dynamic_button(button_label, hidden).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            let before = this.capture_snapshot();
                            this.toggle_active_hidden_material(name.clone());
                            this.invalidate_preview();
                            this.push_undo_snapshot_if_changed(before);
                            cx.notify();
                        }),
                    ));
            }
            let mut mesh_row = div().flex().flex_wrap().gap_2().items_center().child(
                div()
                    .w(px(70.0))
                    .text_xs()
                    .text_color(white().opacity(0.52))
                    .child("mesh"),
            );
            for name in self.active_mesh_names().iter().take(24).cloned() {
                let hidden = self.active_hidden_meshes_contains(&name);
                let button_label = if hidden {
                    format!("hide {name}")
                } else {
                    name.clone()
                };
                mesh_row =
                    mesh_row.child(Self::dynamic_button(button_label, hidden).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            let before = this.capture_snapshot();
                            this.toggle_active_hidden_mesh(name.clone());
                            this.invalidate_preview();
                            this.push_undo_snapshot_if_changed(before);
                            cx.notify();
                        }),
                    ));
            }
            hide_controls = hide_controls.child(material_row).child(mesh_row);
        }

        let mut diagnostics_panel = div()
            .mt_2()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(white().opacity(0.12))
            .bg(rgb(0x090d15))
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .text_color(white().opacity(0.88))
                    .child("GPU Diagnostics"),
            );
        for line in self.gpu_diagnostics_text().lines() {
            diagnostics_panel = diagnostics_panel.child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.62))
                    .child(line.to_string()),
            );
        }

        let sidebar = div()
            .w(px(520.0))
            .max_w(px(520.0))
            .flex_shrink_0()
            .h_full()
            .border_l_1()
            .border_color(white().opacity(0.12))
            .bg(rgb(0x0d111a))
            .p_4()
            .overflow_y_scrollbar()
            .flex()
            .flex_col()
            .gap_3()
            .child(canonical_selector)
            .child(loaded_glb_panel)
            .child(model_transform_controls)
            .child(graph_settings_panel)
            .child(camera_controls)
            .child(raw_bone_controls)
            .child(test_action_controls)
            .child(bone_axis_panel)
            .child(action_dsl_editor)
            .child(hide_controls)
            .child(diagnostics_panel)
            .child(
                div()
                    .mt_4()
                    .flex()
                    .gap_2()
                    .child(Self::control_button("Copy Formal DSL").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            if this.model_path.trim().is_empty() {
                                this.status_line = "Load a GLB before copying formal DSL.".to_string();
                                cx.notify();
                                return;
                            }
                            let export = this.generated_dsl();
                            cx.write_to_clipboard(ClipboardItem::new_string(export));
                            this.status_line = "Copied formal MotionLoom DSL.".to_string();
                            cx.notify();
                        }),
                    ))
                    .child(Self::control_button("Reset Pose").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            let before = this.capture_snapshot();
                            *this.active_action_mut() = WaveActionControls::default();
                            this.active_raw_bone_rotations_mut().clear();
                            this.sync_action_dsl_from_current_pose_if_active();
                            this.status_line = format!(
                                "Reset pose for {}.",
                                this.selected_actor_label()
                            );
                            this.invalidate_preview();
                            this.push_undo_snapshot_if_changed(before);
                            cx.notify();
                        }),
                    )),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.58))
                    .child("Drag canonical handles for pose blocking. Raw bones can be assigned to canonical bones; Copy Formal DSL exports ModelProfile + Action as source of truth."),
            );

        div()
            .size_full()
            .track_focus(&self.focus_handle)
            .bg(rgb(0x080b12))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, _cx| {
                    this.focus_handle.focus(window);
                }),
            )
            .on_key_down(cx.listener(|this, evt: &KeyDownEvent, window, cx| {
                let input_focused = [
                    &this.action_dsl_input,
                    &this.frame_input,
                    &this.duration_input,
                    &this.graph_fps_input,
                    &this.graph_width_input,
                    &this.graph_height_input,
                    &this.render_width_input,
                    &this.render_height_input,
                ]
                .into_iter()
                .any(|input| {
                    input
                        .as_ref()
                        .map(|input| input.read(cx).focus_handle(cx).is_focused(window))
                        .unwrap_or(false)
                });
                if input_focused {
                    return;
                }

                let key = evt.keystroke.key.as_str();
                let modifiers = evt.keystroke.modifiers;
                let accel = modifiers.platform || modifiers.control;
                if accel && !modifiers.shift && key.eq_ignore_ascii_case("z") {
                    this.undo_pose_edit();
                    cx.stop_propagation();
                    cx.notify();
                } else if (accel && modifiers.shift && key.eq_ignore_ascii_case("z"))
                    || (modifiers.control && key.eq_ignore_ascii_case("y"))
                {
                    this.redo_pose_edit();
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .flex()
            .child(preview_panel)
            .child(sidebar)
    }
}
