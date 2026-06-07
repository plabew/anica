use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WorldGraph {
    pub id: Option<String>,
    pub version: Option<String>,
    pub fps: f32,
    pub duration_ms: u64,
    pub duration_explicit: bool,
    pub size: (u32, u32),
    pub render_size: Option<(u32, u32)>,
    pub model_profiles: Vec<WorldModelProfile>,
    pub worlds: Vec<WorldNode>,
    pub retargets: Vec<WorldRetarget>,
    pub actions: Vec<WorldAction>,
    pub apply_actions: Vec<WorldApplyAction>,
    pub present: WorldPresent,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WorldNode {
    pub id: String,
    pub background: Option<WorldBackground>,
    pub camera: WorldCamera,
    pub actors: Vec<WorldActor>,
    pub directional_characters: Vec<WorldDirectionalCharacter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldBackgroundFit {
    Cover,
    Contain,
    Stretch,
}

impl Default for WorldBackgroundFit {
    fn default() -> Self {
        Self::Cover
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WorldBackground {
    pub id: Option<String>,
    pub src: Option<String>,
    pub fit: WorldBackgroundFit,
    pub color: String,
    pub opacity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldCameraControl {
    Orbit,
    Free,
}

impl Default for WorldCameraControl {
    fn default() -> Self {
        Self::Orbit
    }
}

pub type WorldCameraMode = WorldCameraControl;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldCameraProjection {
    Perspective,
    Orthographic,
}

impl Default for WorldCameraProjection {
    fn default() -> Self {
        Self::Perspective
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WorldCamera {
    pub id: Option<String>,
    #[serde(default, alias = "mode")]
    pub control: WorldCameraControl,
    pub projection: WorldCameraProjection,
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

impl Default for WorldCamera {
    fn default() -> Self {
        Self {
            id: Some("camera".to_string()),
            control: WorldCameraControl::Orbit,
            projection: WorldCameraProjection::Perspective,
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
pub struct WorldActor {
    pub id: String,
    pub model: String,
    pub path_style: WorldPathStyle,
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
    pub material: Option<WorldMaterial>,
    pub play: Option<WorldPlay>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WorldDirectionalCharacter {
    pub id: String,
    pub sheet: Option<String>,
    pub path_style: WorldPathStyle,
    pub x: String,
    pub y: String,
    pub scale: String,
    pub yaw: String,
    pub opacity: String,
    pub play_sprite: Option<WorldSpritePlayback>,
    pub directions: Vec<WorldDirectionFrame>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WorldSpritePlayback {
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
pub struct WorldDirectionFrame {
    pub name: Option<String>,
    pub angle: Option<f32>,
    pub camera_pitch: Option<f32>,
    pub image: Option<String>,
    pub rect: Option<(u32, u32, u32, u32)>,
    pub anchor: Option<(f32, f32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldPathStyle {
    Relative,
    Absolute,
}

impl Default for WorldPathStyle {
    fn default() -> Self {
        Self::Relative
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldMaterialStyle {
    Toon,
    Pbr,
    Unlit,
}

impl Default for WorldMaterialStyle {
    fn default() -> Self {
        Self::Toon
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WorldMaterial {
    pub style: WorldMaterialStyle,
    pub outline: bool,
    pub outline_width: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WorldPlay {
    pub clip: Option<String>,
    pub r#loop: bool,
    pub speed: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct WorldRetarget {
    pub id: String,
    pub actor: Option<String>,
    pub preset: String,
    pub maps: Vec<WorldRetargetMap>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct WorldRetargetMap {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WorldModelProfile {
    pub id: String,
    pub model: String,
    pub preset: String,
    pub retarget: Option<WorldProfileRetarget>,
    pub bone_axis_map: Option<WorldBoneAxisMap>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WorldProfileRetarget {
    pub preset: String,
    pub maps: Vec<WorldRetargetMap>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WorldBoneAxisMap {
    pub axes: Vec<WorldBoneAxis>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WorldBoneAxis {
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
pub struct WorldAction {
    pub id: String,
    pub skeleton: String,
    pub intent: Option<String>,
    pub duration_ms: u64,
    pub poses: Vec<WorldActionPose>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WorldActionPose {
    pub t: f32,
    pub label: Option<String>,
    pub bones: Vec<WorldActionBone>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WorldActionBone {
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
pub struct WorldApplyAction {
    pub target: String,
    pub action: String,
    pub at_ms: u64,
    pub r#loop: bool,
    pub weight: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct WorldPresent {
    pub from: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldTime {
    pub frame: u32,
    pub fps: f32,
    pub duration_ms: u64,
}

impl WorldTime {
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

impl WorldGraph {
    pub fn presented_world(&self) -> Option<&WorldNode> {
        self.worlds
            .iter()
            .find(|world| world.id == self.present.from)
    }

    pub fn output_size(&self) -> (u32, u32) {
        self.render_size.unwrap_or(self.size)
    }
}
