use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnimationGraph {
    pub id: Option<String>,
    pub version: Option<String>,
    pub fps: f32,
    pub duration_ms: u64,
    pub duration_explicit: bool,
    pub size: (u32, u32),
    pub render_size: Option<(u32, u32)>,
    pub model_profiles: Vec<AnimationModelProfile>,
    pub worlds: Vec<AnimationWorld>,
    pub retargets: Vec<AnimationRetarget>,
    pub actions: Vec<AnimationAction>,
    pub apply_actions: Vec<AnimationApplyAction>,
    pub present: AnimationPresent,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnimationWorld {
    pub id: String,
    pub background: Option<AnimationBackground>,
    pub camera: AnimationCamera,
    pub actors: Vec<AnimationActor>,
    pub directional_characters: Vec<AnimationDirectionalCharacter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnimationBackgroundFit {
    Cover,
    Contain,
    Stretch,
}

impl Default for AnimationBackgroundFit {
    fn default() -> Self {
        Self::Cover
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnimationBackground {
    pub id: Option<String>,
    pub src: Option<String>,
    pub fit: AnimationBackgroundFit,
    pub color: String,
    pub opacity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnimationCameraMode {
    Orbit,
    Free,
}

impl Default for AnimationCameraMode {
    fn default() -> Self {
        Self::Orbit
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnimationCameraProjection {
    Perspective,
    Orthographic,
}

impl Default for AnimationCameraProjection {
    fn default() -> Self {
        Self::Perspective
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnimationCamera {
    pub id: Option<String>,
    pub mode: AnimationCameraMode,
    pub projection: AnimationCameraProjection,
    pub target: Option<String>,
    pub x: String,
    pub y: String,
    pub z: String,
    pub target_x: String,
    pub target_y: String,
    pub target_z: String,
    pub yaw: String,
    pub pitch: String,
    pub roll: String,
    pub distance: String,
    pub zoom: String,
    pub fov: String,
    pub orthographic_scale: Option<String>,
}

impl Default for AnimationCamera {
    fn default() -> Self {
        Self {
            id: Some("camera".to_string()),
            mode: AnimationCameraMode::Orbit,
            projection: AnimationCameraProjection::Perspective,
            target: None,
            x: "0".to_string(),
            y: "0".to_string(),
            z: "0".to_string(),
            target_x: "0".to_string(),
            target_y: "1.0".to_string(),
            target_z: "0".to_string(),
            yaw: "0".to_string(),
            pitch: "0".to_string(),
            roll: "0".to_string(),
            distance: "3.2".to_string(),
            zoom: "1".to_string(),
            fov: "35".to_string(),
            orthographic_scale: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnimationActor {
    pub id: String,
    pub model: String,
    pub path_style: AnimationPathStyle,
    pub hide_meshes: Vec<String>,
    pub hide_materials: Vec<String>,
    pub profile: Option<String>,
    pub rig: Option<String>,
    pub retarget: Option<String>,
    pub x: String,
    pub y: String,
    pub z: String,
    pub yaw: String,
    pub pitch: String,
    pub roll: String,
    pub scale: String,
    pub opacity: String,
    pub material: Option<AnimationMaterial>,
    pub play: Option<AnimationPlay>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnimationDirectionalCharacter {
    pub id: String,
    pub sheet: Option<String>,
    pub path_style: AnimationPathStyle,
    pub x: String,
    pub y: String,
    pub scale: String,
    pub yaw: String,
    pub opacity: String,
    pub play_sprite: Option<AnimationSpritePlayback>,
    pub directions: Vec<AnimationDirectionFrame>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnimationSpritePlayback {
    pub fps: String,
    pub r#loop: bool,
    pub frames: u32,
    pub columns: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    pub start: u32,
    pub margin_x: u32,
    pub margin_y: u32,
    pub spacing_x: u32,
    pub spacing_y: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnimationDirectionFrame {
    pub name: Option<String>,
    pub angle: Option<f32>,
    pub camera_pitch: Option<f32>,
    pub image: Option<String>,
    pub rect: Option<(u32, u32, u32, u32)>,
    pub anchor: Option<(f32, f32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnimationPathStyle {
    Relative,
    Absolute,
}

impl Default for AnimationPathStyle {
    fn default() -> Self {
        Self::Relative
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnimationMaterialStyle {
    Toon,
    Pbr,
    Unlit,
}

impl Default for AnimationMaterialStyle {
    fn default() -> Self {
        Self::Toon
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnimationMaterial {
    pub style: AnimationMaterialStyle,
    pub outline: bool,
    pub outline_width: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnimationPlay {
    pub clip: Option<String>,
    pub r#loop: bool,
    pub speed: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AnimationRetarget {
    pub id: String,
    pub actor: Option<String>,
    pub preset: String,
    pub maps: Vec<AnimationRetargetMap>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AnimationRetargetMap {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnimationModelProfile {
    pub id: String,
    pub model: String,
    pub preset: String,
    pub retarget: Option<AnimationProfileRetarget>,
    pub bone_axis_map: Option<AnimationBoneAxisMap>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnimationProfileRetarget {
    pub preset: String,
    pub maps: Vec<AnimationRetargetMap>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnimationBoneAxisMap {
    pub axes: Vec<AnimationBoneAxis>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnimationBoneAxis {
    pub bone: String,
    pub forward: Option<String>,
    pub side: Option<String>,
    pub twist: Option<String>,
    pub bend: Option<String>,
    pub turn: Option<String>,
    pub rest_forward: Option<String>,
    pub rest_side: Option<String>,
    pub rest_twist: Option<String>,
    pub rest_bend: Option<String>,
    pub rest_turn: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnimationAction {
    pub id: String,
    pub skeleton: String,
    pub intent: Option<String>,
    pub duration_ms: u64,
    pub poses: Vec<AnimationActionPose>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnimationActionPose {
    pub t: f32,
    pub label: Option<String>,
    pub bones: Vec<AnimationActionBone>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnimationActionBone {
    pub id: String,
    pub x: Option<String>,
    pub y: Option<String>,
    pub z: Option<String>,
    pub rotation: Option<String>,
    pub rotation_x: Option<String>,
    pub rotation_y: Option<String>,
    pub rotation_z: Option<String>,
    pub forward: Option<String>,
    pub side: Option<String>,
    pub twist: Option<String>,
    pub bend: Option<String>,
    pub turn: Option<String>,
    pub scale: Option<String>,
    pub opacity: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AnimationApplyAction {
    pub target: String,
    pub action: String,
    pub at_ms: u64,
    pub r#loop: bool,
    pub weight: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AnimationPresent {
    pub from: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationTime {
    pub frame: u32,
    pub fps: f32,
    pub duration_ms: u64,
}

impl AnimationTime {
    pub fn time_sec(self) -> f32 {
        if self.fps <= f32::EPSILON {
            0.0
        } else {
            self.frame as f32 / self.fps
        }
    }

    pub fn time_norm(self) -> f32 {
        let duration_sec = self.duration_ms as f32 / 1000.0;
        if duration_sec <= f32::EPSILON {
            0.0
        } else {
            (self.time_sec() / duration_sec).clamp(0.0, 1.0)
        }
    }
}

impl AnimationGraph {
    pub fn presented_world(&self) -> Option<&AnimationWorld> {
        self.worlds
            .iter()
            .find(|world| world.id == self.present.from)
    }

    pub fn output_size(&self) -> (u32, u32) {
        self.render_size.unwrap_or(self.size)
    }
}
