// =========================================
// =========================================
// crates/motionloom/src/simulation/collision/shapes.rs

use crate::simulation::model::{ColliderNode, ColliderShape};

pub fn project_out(point: &mut [f32; 2], collider: &ColliderNode, padding: f32) {
    match collider.shape {
        ColliderShape::Circle => {
            project_circle(point, [collider.x, collider.y], collider.radius + padding)
        }
        ColliderShape::Ellipse => project_ellipse(
            point,
            [collider.x, collider.y],
            collider.radius_x + padding,
            collider.radius_y + padding,
        ),
        ColliderShape::Capsule => {
            project_capsule(point, collider.from, collider.to, collider.radius + padding)
        }
        ColliderShape::Box | ColliderShape::ConvexHull => {}
    }
}

fn project_circle(point: &mut [f32; 2], center: [f32; 2], radius: f32) {
    let dx = point[0] - center[0];
    let dy = point[1] - center[1];
    let length = (dx * dx + dy * dy).sqrt();
    if length < radius && length > 0.000_1 {
        point[0] = center[0] + dx / length * radius;
        point[1] = center[1] + dy / length * radius;
    }
}

fn project_ellipse(point: &mut [f32; 2], center: [f32; 2], rx: f32, ry: f32) {
    let dx = point[0] - center[0];
    let dy = point[1] - center[1];
    let normalized = (dx * dx / (rx * rx).max(0.000_1) + dy * dy / (ry * ry).max(0.000_1)).sqrt();
    if normalized < 1.0 && normalized > 0.000_1 {
        point[0] = center[0] + dx / normalized;
        point[1] = center[1] + dy / normalized;
    }
}

fn project_capsule(point: &mut [f32; 2], from: [f32; 2], to: [f32; 2], radius: f32) {
    let ab = [to[0] - from[0], to[1] - from[1]];
    let length2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if length2 > 0.000_1 {
        (((point[0] - from[0]) * ab[0] + (point[1] - from[1]) * ab[1]) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    project_circle(point, [from[0] + ab[0] * t, from[1] + ab[1] * t], radius);
}
