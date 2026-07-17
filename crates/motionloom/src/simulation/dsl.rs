// =========================================
// =========================================
// crates/motionloom/src/simulation/dsl.rs

use crate::dsl::{attr_value, required_attr_value, strip_wrappers};
use crate::error::GraphParseError;
use crate::simulation::model::*;

pub(crate) fn parse_resource(
    tag: &str,
    line: usize,
) -> Result<SimulationResourceNode, GraphParseError> {
    let name = tag
        .trim_start()
        .trim_start_matches('<')
        .split_whitespace()
        .next()
        .unwrap_or("");
    match name {
        "Gravity" => Ok(SimulationResourceNode::Gravity(GravityNode {
            id: required_string(tag, "id", line)?,
            vector: vec2(tag, "vector", [0.0, 980.0], line)?,
        })),
        "Wind" => Ok(SimulationResourceNode::Wind(WindNode {
            id: required_string(tag, "id", line)?,
            direction: vec2(tag, "direction", [1.0, 0.0], line)?,
            strength: number(tag, "strength", 0.0, line)?,
            turbulence: number(tag, "turbulence", 0.0, line)?,
            noise_scale: number(tag, "noiseScale", 1.0, line)?,
        })),
        "Attraction" => Ok(SimulationResourceNode::Attraction(AttractionNode {
            id: required_string(tag, "id", line)?,
            target: optional_string(tag, "target"),
            point: vec2(tag, "point", [0.0, 0.0], line)?,
            strength: number(tag, "strength", 0.0, line)?,
            radius: number(tag, "radius", f32::MAX, line)?,
        })),
        "Collider" => Ok(SimulationResourceNode::Collider(ColliderNode {
            id: required_string(tag, "id", line)?,
            target: optional_string(tag, "target"),
            shape: collider_shape(tag, line)?,
            x: number(tag, "x", 0.0, line)?,
            y: number(tag, "y", 0.0, line)?,
            radius: number(tag, "radius", 0.0, line)?,
            radius_x: number(tag, "radiusX", 0.0, line)?,
            radius_y: number(tag, "radiusY", 0.0, line)?,
            from: vec2(tag, "from", [0.0, 0.0], line)?,
            to: vec2(tag, "to", [0.0, 0.0], line)?,
        })),
        _ => Err(parse_error(
            line,
            format!("unknown simulation resource <{name}>"),
        )),
    }
}

pub(crate) fn parse_binding(
    tag: &str,
    line: usize,
) -> Result<SimulationBindingNode, GraphParseError> {
    let name = tag
        .trim_start()
        .trim_start_matches('<')
        .split_whitespace()
        .next()
        .unwrap_or("");
    match name {
        "SpringChain" => {
            let (gravity, gravity_ref) = vec2_or_ref(tag, "gravity", [0.0, 520.0], line)?;
            Ok(SimulationBindingNode::SpringChain(SpringChainNode {
                id: optional_string(tag, "id"),
                target: required_string(tag, "target", line)?,
                pin: string(tag, "pin", "start"),
                segments: usize_number(tag, "segments", 16, line)?,
                stiffness: number(tag, "stiffness", 0.75, line)?,
                damping: number(tag, "damping", 0.18, line)?,
                gravity,
                gravity_ref,
                wind: optional_string(tag, "wind"),
                attraction: optional_string(tag, "attraction"),
                colliders: string_list(tag, "colliders"),
                collision_radius: number(tag, "collisionRadius", 0.0, line)?,
            }))
        }
        "DynamicCurve" => Ok(SimulationBindingNode::DynamicCurve(DynamicCurveNode {
            id: optional_string(tag, "id"),
            target: required_string(tag, "target", line)?,
            simulation: string(tag, "simulation", "spring"),
        })),
        "DistanceConstraint" => Ok(SimulationBindingNode::DistanceConstraint(
            DistanceConstraintNode {
                id: optional_string(tag, "id"),
                a: required_string(tag, "a", line)?,
                b: required_string(tag, "b", line)?,
                distance: number(tag, "distance", 0.0, line)?,
                stiffness: number(tag, "stiffness", 1.0, line)?,
            },
        )),
        "Hinge" => Ok(SimulationBindingNode::Hinge(HingeNode {
            id: optional_string(tag, "id"),
            a: required_string(tag, "a", line)?,
            b: required_string(tag, "b", line)?,
            anchor: vec2(tag, "anchor", [0.0, 0.0], line)?,
            stiffness: number(tag, "stiffness", 1.0, line)?,
        })),
        "RigidBody2D" => Ok(SimulationBindingNode::RigidBody2D(RigidBody2DNode {
            id: required_string(tag, "id", line)?,
            target: required_string(tag, "target", line)?,
            mass: number(tag, "mass", 1.0, line)?,
            velocity: vec2(tag, "velocity", [0.0, 0.0], line)?,
            angular_velocity: number(tag, "angularVelocity", 0.0, line)?,
        })),
        "ParticleEmitter" => Ok(SimulationBindingNode::ParticleEmitter(
            ParticleEmitterNode {
                id: required_string(tag, "id", line)?,
                target: optional_string(tag, "target"),
                x: number(tag, "x", 0.0, line)?,
                y: number(tag, "y", 0.0, line)?,
                rate: number(tag, "rate", 24.0, line)?,
                lifetime: number(tag, "lifetime", 2.0, line)?,
                velocity: vec2(tag, "velocity", [0.0, -120.0], line)?,
                gravity: vec2(tag, "gravity", [0.0, 300.0], line)?,
                radius: number(tag, "radius", 5.0, line)?,
                color: string(tag, "color", "#D8FF2F"),
            },
        )),
        "Cloth" => Ok(SimulationBindingNode::Cloth(ClothNode {
            id: required_string(tag, "id", line)?,
            target: required_string(tag, "target", line)?,
            columns: usize_number(tag, "columns", 12, line)?,
            rows: usize_number(tag, "rows", 8, line)?,
            stiffness: number(tag, "stiffness", 0.75, line)?,
            damping: number(tag, "damping", 0.2, line)?,
            amplitude: number(tag, "amplitude", 28.0, line)?,
            frequency: number(tag, "frequency", 1.8, line)?,
        })),
        "HairStrandField" => Ok(SimulationBindingNode::HairStrandField(
            HairStrandFieldNode {
                id: required_string(tag, "id", line)?,
                target: required_string(tag, "target", line)?,
                strands: usize_number(tag, "strands", 32, line)?,
                segments: usize_number(tag, "segments", 12, line)?,
                stiffness: number(tag, "stiffness", 0.72, line)?,
                damping: number(tag, "damping", 0.2, line)?,
            },
        )),
        "CacheBake" => Ok(SimulationBindingNode::CacheBake(CacheBakeNode {
            id: required_string(tag, "id", line)?,
            target: required_string(tag, "target", line)?,
            from_frame: usize_number(tag, "fromFrame", 0, line)? as u32,
            to_frame: usize_number(tag, "toFrame", 0, line)? as u32,
        })),
        _ => Err(parse_error(
            line,
            format!("unknown simulation binding <{name}>"),
        )),
    }
}

fn required_string(tag: &str, key: &str, line: usize) -> Result<String, GraphParseError> {
    Ok(strip_wrappers(&required_attr_value(tag, key, line)?).to_string())
}
fn optional_string(tag: &str, key: &str) -> Option<String> {
    attr_value(tag, key).map(|value| strip_wrappers(&value).to_string())
}
fn string(tag: &str, key: &str, default: &str) -> String {
    optional_string(tag, key).unwrap_or_else(|| default.to_string())
}
fn number(tag: &str, key: &'static str, default: f32, line: usize) -> Result<f32, GraphParseError> {
    optional_string(tag, key).map_or(Ok(default), |value| {
        value
            .parse()
            .map_err(|_| parse_error(line, format!("{key} must be numeric")))
    })
}
fn usize_number(
    tag: &str,
    key: &'static str,
    default: usize,
    line: usize,
) -> Result<usize, GraphParseError> {
    optional_string(tag, key).map_or(Ok(default), |value| {
        value
            .parse()
            .map_err(|_| parse_error(line, format!("{key} must be an unsigned integer")))
    })
}
fn vec2(
    tag: &str,
    key: &'static str,
    default: [f32; 2],
    line: usize,
) -> Result<[f32; 2], GraphParseError> {
    let Some(raw) = optional_string(tag, key) else {
        return Ok(default);
    };
    let values = raw
        .trim_matches(|c| matches!(c, '[' | ']' | '{' | '}'))
        .split(',')
        .map(str::trim)
        .map(str::parse::<f32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| parse_error(line, format!("{key} must contain two numbers")))?;
    if values.len() != 2 {
        return Err(parse_error(line, format!("{key} must contain two numbers")));
    }
    Ok([values[0], values[1]])
}
fn vec2_or_ref(
    tag: &str,
    key: &'static str,
    default: [f32; 2],
    line: usize,
) -> Result<([f32; 2], Option<String>), GraphParseError> {
    let Some(raw) = optional_string(tag, key) else {
        return Ok((default, None));
    };
    if !raw.contains(',') {
        return Ok((default, Some(raw)));
    }
    Ok((vec2(tag, key, default, line)?, None))
}
fn string_list(tag: &str, key: &str) -> Vec<String> {
    optional_string(tag, key)
        .map(|raw| {
            raw.trim_matches(|c| matches!(c, '[' | ']' | '{' | '}'))
                .split(',')
                .map(|item| item.trim().trim_matches('"').to_string())
                .filter(|item| !item.is_empty())
                .collect()
        })
        .unwrap_or_default()
}
fn collider_shape(tag: &str, line: usize) -> Result<ColliderShape, GraphParseError> {
    match string(tag, "shape", "circle").as_str() {
        "circle" => Ok(ColliderShape::Circle),
        "ellipse" => Ok(ColliderShape::Ellipse),
        "capsule" => Ok(ColliderShape::Capsule),
        "box" => Ok(ColliderShape::Box),
        "convexHull" | "convex_hull" => Ok(ColliderShape::ConvexHull),
        value => Err(parse_error(
            line,
            format!("unsupported collider shape '{value}'"),
        )),
    }
}
fn parse_error(line: usize, message: String) -> GraphParseError {
    GraphParseError { line, message }
}

#[cfg(test)]
mod tests {
    use crate::dsl::parse_graph_script;
    use crate::scene::model::SceneNode;
    use crate::simulation::model::{SimulationBindingNode, SimulationResourceNode};

    #[test]
    fn parses_resources_and_spring_chain_without_wrapper() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[320,240]}>
  <Scene id="hair">
    <Defs>
      <Wind id="wind" direction={[1,0]} strength="18" />
      <Collider id="head" shape="circle" x="160" y="100" radius="40" />
    </Defs>
    <Timeline>
      <Track>
        <Sequence from="0s" duration="1s">
          <Layer>
      <Curve id="strand" points="100,40 110,90 120,150" stroke="#fff" />
      <SpringChain target="strand" pin="start" segments="8" wind="wind" colliders={["head"]} />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>
  <Present from="hair" />
</Graph>
"##,
        )
        .expect("simulation DSL should parse");
        let defs = graph.scenes[0]
            .children
            .iter()
            .find_map(|node| match node {
                SceneNode::Defs(defs) => Some(defs),
                _ => None,
            })
            .expect("defs");
        assert!(matches!(
            defs.simulation[0],
            SimulationResourceNode::Wind(_)
        ));
        fn has_binding(nodes: &[SceneNode]) -> bool {
            nodes.iter().any(|node| match node {
                SceneNode::Simulation(SimulationBindingNode::SpringChain(_)) => true,
                SceneNode::Timeline(node) => has_binding(&node.children),
                SceneNode::Track(node) => has_binding(&node.children),
                SceneNode::Sequence(node) => has_binding(&node.children),
                SceneNode::Layer(node) => has_binding(&node.children),
                _ => false,
            })
        }
        assert!(has_binding(&graph.scenes[0].children));
    }
}
