// =========================================
// =========================================
// crates/motionloom/src/simulation/model.rs

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SimulationResourceNode {
    Gravity(GravityNode),
    Wind(WindNode),
    Attraction(AttractionNode),
    Collider(ColliderNode),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SimulationBindingNode {
    SpringChain(SpringChainNode),
    DynamicCurve(DynamicCurveNode),
    DistanceConstraint(DistanceConstraintNode),
    Hinge(HingeNode),
    RigidBody2D(RigidBody2DNode),
    ParticleEmitter(ParticleEmitterNode),
    Cloth(ClothNode),
    HairStrandField(HairStrandFieldNode),
    CacheBake(CacheBakeNode),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GravityNode {
    pub id: String,
    pub vector: [f32; 2],
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindNode {
    pub id: String,
    pub direction: [f32; 2],
    pub strength: f32,
    pub turbulence: f32,
    pub noise_scale: f32,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttractionNode {
    pub id: String,
    pub target: Option<String>,
    pub point: [f32; 2],
    pub strength: f32,
    pub radius: f32,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColliderNode {
    pub id: String,
    pub target: Option<String>,
    pub shape: ColliderShape,
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub radius_x: f32,
    pub radius_y: f32,
    pub from: [f32; 2],
    pub to: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ColliderShape {
    Circle,
    Ellipse,
    Capsule,
    Box,
    ConvexHull,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpringChainNode {
    pub id: Option<String>,
    pub target: String,
    pub pin: String,
    pub segments: usize,
    pub stiffness: f32,
    pub damping: f32,
    pub gravity: [f32; 2],
    pub gravity_ref: Option<String>,
    pub wind: Option<String>,
    pub attraction: Option<String>,
    pub colliders: Vec<String>,
    pub collision_radius: f32,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DynamicCurveNode {
    pub id: Option<String>,
    pub target: String,
    pub simulation: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DistanceConstraintNode {
    pub id: Option<String>,
    pub a: String,
    pub b: String,
    pub distance: f32,
    pub stiffness: f32,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HingeNode {
    pub id: Option<String>,
    pub a: String,
    pub b: String,
    pub anchor: [f32; 2],
    pub stiffness: f32,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RigidBody2DNode {
    pub id: String,
    pub target: String,
    pub mass: f32,
    pub velocity: [f32; 2],
    pub angular_velocity: f32,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticleEmitterNode {
    pub id: String,
    pub target: Option<String>,
    pub x: f32,
    pub y: f32,
    pub rate: f32,
    pub lifetime: f32,
    pub velocity: [f32; 2],
    pub gravity: [f32; 2],
    pub radius: f32,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClothNode {
    pub id: String,
    pub target: String,
    pub columns: usize,
    pub rows: usize,
    pub stiffness: f32,
    pub damping: f32,
    pub amplitude: f32,
    pub frequency: f32,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HairStrandFieldNode {
    pub id: String,
    pub target: String,
    pub strands: usize,
    pub segments: usize,
    pub stiffness: f32,
    pub damping: f32,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheBakeNode {
    pub id: String,
    pub target: String,
    pub from_frame: u32,
    pub to_frame: u32,
}
