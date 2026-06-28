// =========================================
// =========================================
// crates/motionloom/src/scene/editor_keyframes.rs

use crate::dsl::{AnimationKeyNode, AnimationTargetNode, parse_graph_script, parse_time_seconds};
use crate::error::GraphParseError;
use std::error::Error;
use std::fmt;

/// Editor-facing keyframe timeline extracted from MotionLoom graph DSL.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableAnimationTimeline {
    pub fps: f32,
    pub targets: Vec<EditableAnimationTarget>,
}

/// One editable `AnimationTarget` channel for a node/property pair.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableAnimationTarget {
    pub node: String,
    pub property: String,
    pub keys: Vec<EditableAnimationKey>,
}

/// One timed key for an editable `AnimationTarget` channel.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableAnimationKey {
    pub frame: u32,
    #[serde(default)]
    pub time: Option<String>,
    pub value: String,
    pub ease: String,
}

/// Typed errors for editor keyframe extraction and write-back.
#[derive(Debug, Clone, PartialEq)]
pub enum AnimationKeyframeEditError {
    Parse(GraphParseError),
    MissingGraphClose,
    MissingGraphPresent,
    InvalidTarget {
        node: String,
        property: String,
        reason: &'static str,
    },
    InvalidKey {
        node: String,
        property: String,
        frame: u32,
        reason: &'static str,
    },
}

impl fmt::Display for AnimationKeyframeEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(err) => write!(f, "{err}"),
            Self::MissingGraphClose => write!(f, "MotionLoom graph is missing </Graph>."),
            Self::MissingGraphPresent => {
                write!(
                    f,
                    "MotionLoom graph is missing a final <Present ... /> node."
                )
            }
            Self::InvalidTarget {
                node,
                property,
                reason,
            } => write!(
                f,
                "Invalid AnimationTarget node={node:?} property={property:?}: {reason}"
            ),
            Self::InvalidKey {
                node,
                property,
                frame,
                reason,
            } => write!(
                f,
                "Invalid AnimationTarget key node={node:?} property={property:?} frame={frame}: {reason}"
            ),
        }
    }
}

impl Error for AnimationKeyframeEditError {}

impl From<GraphParseError> for AnimationKeyframeEditError {
    fn from(value: GraphParseError) -> Self {
        Self::Parse(value)
    }
}

impl From<&AnimationTargetNode> for EditableAnimationTarget {
    fn from(value: &AnimationTargetNode) -> Self {
        Self {
            node: value.node.clone(),
            property: value.property.clone(),
            keys: value.keys.iter().map(EditableAnimationKey::from).collect(),
        }
    }
}

impl From<&AnimationKeyNode> for EditableAnimationKey {
    fn from(value: &AnimationKeyNode) -> Self {
        Self {
            frame: value.frame,
            time: value.time.clone(),
            value: value.value.clone(),
            ease: value.ease.clone(),
        }
    }
}

/// Parse a MotionLoom script and return the UI-editable AnimationTarget model.
pub fn extract_editable_animation_timeline(
    script: &str,
) -> Result<EditableAnimationTimeline, AnimationKeyframeEditError> {
    let graph = parse_graph_script(script)?;
    Ok(EditableAnimationTimeline {
        fps: graph.fps,
        targets: graph
            .animation_targets
            .iter()
            .map(EditableAnimationTarget::from)
            .collect(),
    })
}

/// Replace every existing AnimationTarget block with the supplied UI model.
pub fn replace_editable_animation_targets(
    script: &str,
    targets: &[EditableAnimationTarget],
) -> Result<String, AnimationKeyframeEditError> {
    for target in targets {
        validate_editable_target(target)?;
    }

    let stripped = strip_animation_target_blocks(script);
    let insertion = render_animation_target_blocks(targets);
    let output = insert_before_final_present(&stripped, &insertion)?;

    // Re-parse the generated script so UI write-back cannot emit invalid DSL.
    parse_graph_script(&output)?;
    Ok(output)
}

/// Replace or add a single node/property channel while preserving other channels.
pub fn upsert_editable_animation_target(
    script: &str,
    target: EditableAnimationTarget,
) -> Result<String, AnimationKeyframeEditError> {
    validate_editable_target(&target)?;
    let mut timeline = extract_editable_animation_timeline(script)?;
    if let Some(existing) = timeline
        .targets
        .iter_mut()
        .find(|existing| existing.node == target.node && existing.property == target.property)
    {
        *existing = target;
    } else {
        timeline.targets.push(target);
    }
    replace_editable_animation_targets(script, &timeline.targets)
}

fn validate_editable_target(
    target: &EditableAnimationTarget,
) -> Result<(), AnimationKeyframeEditError> {
    if target.node.trim().is_empty() {
        return Err(AnimationKeyframeEditError::InvalidTarget {
            node: target.node.clone(),
            property: target.property.clone(),
            reason: "node id is empty",
        });
    }
    if !matches!(
        target.property.as_str(),
        "x" | "y"
            | "rotation"
            | "scale"
            | "scaleX"
            | "scaleY"
            | "skewX"
            | "skewY"
            | "transformOriginX"
            | "transformOriginY"
            | "opacity"
            | "d"
    ) {
        return Err(AnimationKeyframeEditError::InvalidTarget {
            node: target.node.clone(),
            property: target.property.clone(),
            reason: "unsupported property",
        });
    }
    if contains_unsafe_attr_text(&target.node) || contains_unsafe_attr_text(&target.property) {
        return Err(AnimationKeyframeEditError::InvalidTarget {
            node: target.node.clone(),
            property: target.property.clone(),
            reason: "node or property contains unsupported attribute characters",
        });
    }
    if target.keys.is_empty() {
        return Err(AnimationKeyframeEditError::InvalidTarget {
            node: target.node.clone(),
            property: target.property.clone(),
            reason: "target requires at least one key",
        });
    }
    for key in &target.keys {
        validate_editable_key(target, key)?;
    }
    Ok(())
}

fn validate_editable_key(
    target: &EditableAnimationTarget,
    key: &EditableAnimationKey,
) -> Result<(), AnimationKeyframeEditError> {
    if key.ease.trim().is_empty()
        || contains_unsafe_attr_text(&key.ease)
        || contains_unsafe_attr_text(&key.value)
        || key
            .time
            .as_ref()
            .is_some_and(|time| contains_unsafe_attr_text(time))
    {
        return Err(AnimationKeyframeEditError::InvalidKey {
            node: target.node.clone(),
            property: target.property.clone(),
            frame: key.frame,
            reason: "value or ease contains unsupported attribute characters",
        });
    }
    if let Some(time) = key.time.as_ref() {
        parse_time_seconds(time, 0, "Key.time").map_err(|_| {
            AnimationKeyframeEditError::InvalidKey {
                node: target.node.clone(),
                property: target.property.clone(),
                frame: key.frame,
                reason: "time must be a valid non-negative time value",
            }
        })?;
    }
    if target.property != "d" && key.value.parse::<f32>().is_err() {
        return Err(AnimationKeyframeEditError::InvalidKey {
            node: target.node.clone(),
            property: target.property.clone(),
            frame: key.frame,
            reason: "numeric properties require numeric key values",
        });
    }
    Ok(())
}

fn contains_unsafe_attr_text(value: &str) -> bool {
    value.contains('"') || value.contains('\n') || value.contains('\r')
}

fn strip_animation_target_blocks(script: &str) -> String {
    let mut output = Vec::<String>::new();
    let mut skipping = false;
    for line in script.lines() {
        let trimmed = line.trim_start();
        if !skipping && trimmed.starts_with("<AnimationTarget") {
            skipping = !trimmed.contains("</AnimationTarget>");
            continue;
        }
        if skipping {
            if trimmed.contains("</AnimationTarget>") {
                skipping = false;
            }
            continue;
        }
        output.push(line.to_string());
    }

    let mut result = output.join("\n");
    if script.ends_with('\n') {
        result.push('\n');
    }
    result
}

fn insert_before_final_present(
    script: &str,
    insertion: &str,
) -> Result<String, AnimationKeyframeEditError> {
    if !script.contains("</Graph>") {
        return Err(AnimationKeyframeEditError::MissingGraphClose);
    }
    if insertion.trim().is_empty() {
        return Ok(script.to_string());
    }

    let Some(index) = script.rfind("<Present") else {
        return Err(AnimationKeyframeEditError::MissingGraphPresent);
    };
    let before = script[..index].trim_end();
    let after = &script[index..];
    Ok(format!("{before}\n\n{insertion}{after}"))
}

fn render_animation_target_blocks(targets: &[EditableAnimationTarget]) -> String {
    let mut sorted = targets.to_vec();
    sorted.sort_by(|a, b| {
        a.node
            .cmp(&b.node)
            .then_with(|| a.property.cmp(&b.property))
    });

    let mut output = String::new();
    for target in sorted {
        let mut keys = target.keys;
        keys.sort_by(|a, b| {
            editable_key_sort_seconds(a)
                .partial_cmp(&editable_key_sort_seconds(b))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.frame.cmp(&b.frame))
        });
        output.push_str(&format!(
            "  <AnimationTarget node=\"{}\" property=\"{}\">\n",
            target.node, target.property
        ));
        for key in keys {
            if let Some(time) = key.time.as_ref() {
                output.push_str(&format!(
                    "    <Key time=\"{}\" value=\"{}\" ease=\"{}\" />\n",
                    time, key.value, key.ease
                ));
            } else {
                output.push_str(&format!(
                    "    <Key frame=\"{}\" value=\"{}\" ease=\"{}\" />\n",
                    key.frame, key.value, key.ease
                ));
            }
        }
        output.push_str("  </AnimationTarget>\n");
    }
    output.push('\n');
    output
}

fn editable_key_sort_seconds(key: &EditableAnimationKey) -> f32 {
    key.time
        .as_ref()
        .and_then(|time| parse_time_seconds(time, 0, "Key.time").ok())
        .unwrap_or(key.frame as f32)
}

#[cfg(test)]
mod tests {
    use super::{
        EditableAnimationKey, EditableAnimationTarget, extract_editable_animation_timeline,
        replace_editable_animation_targets, upsert_editable_animation_target,
    };

    const SCRIPT: &str = r##"<Graph fps={30} duration="1s" size={[100,100]}>
  <Scene id="scene0">
    <Timeline>
      <Track>
        <Sequence duration="1s">
          <Layer>
            <Group id="card" x="0" y="0">
              <Rect x="0" y="0" width="10" height="10" color="#fff" />
            </Group>
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>

  <AnimationTarget node="card" property="rotation">
    <Key frame="0" value="0" ease="linear" />
    <Key frame="15" value="18" ease="ease_in_out" />
  </AnimationTarget>

  <Present from="scene0" />
</Graph>
"##;

    #[test]
    fn extracts_editable_animation_timeline() {
        let timeline = extract_editable_animation_timeline(SCRIPT).unwrap();
        assert_eq!(timeline.fps, 30.0);
        assert_eq!(timeline.targets.len(), 1);
        assert_eq!(timeline.targets[0].node, "card");
        assert_eq!(timeline.targets[0].property, "rotation");
        assert_eq!(timeline.targets[0].keys[1].frame, 15);
    }

    #[test]
    fn replaces_animation_targets_and_keeps_script_parseable() {
        let output = replace_editable_animation_targets(
            SCRIPT,
            &[EditableAnimationTarget {
                node: "card".to_string(),
                property: "x".to_string(),
                keys: vec![
                    EditableAnimationKey {
                        frame: 20,
                        time: None,
                        value: "50".to_string(),
                        ease: "ease_out".to_string(),
                    },
                    EditableAnimationKey {
                        frame: 0,
                        time: None,
                        value: "0".to_string(),
                        ease: "linear".to_string(),
                    },
                ],
            }],
        )
        .unwrap();
        let timeline = extract_editable_animation_timeline(&output).unwrap();
        assert_eq!(timeline.targets.len(), 1);
        assert_eq!(timeline.targets[0].property, "x");
        assert_eq!(timeline.targets[0].keys[0].frame, 0);
        assert!(output.contains("<Present from=\"scene0\" />"));
    }

    #[test]
    fn upserts_one_target_and_preserves_other_channels() {
        let output = upsert_editable_animation_target(
            SCRIPT,
            EditableAnimationTarget {
                node: "card".to_string(),
                property: "x".to_string(),
                keys: vec![EditableAnimationKey {
                    frame: 0,
                    time: None,
                    value: "12".to_string(),
                    ease: "linear".to_string(),
                }],
            },
        )
        .unwrap();
        let timeline = extract_editable_animation_timeline(&output).unwrap();
        assert_eq!(timeline.targets.len(), 2);
        assert!(
            timeline
                .targets
                .iter()
                .any(|target| target.property == "rotation")
        );
        assert!(timeline.targets.iter().any(|target| target.property == "x"));
    }

    #[test]
    fn accepts_extended_transform_properties() {
        let output = upsert_editable_animation_target(
            SCRIPT,
            EditableAnimationTarget {
                node: "card".to_string(),
                property: "skewX".to_string(),
                keys: vec![EditableAnimationKey {
                    frame: 0,
                    time: Some("0s".to_string()),
                    value: "-30".to_string(),
                    ease: "linear".to_string(),
                }],
            },
        )
        .unwrap();
        assert!(output.contains("<Key time=\"0s\" value=\"-30\" ease=\"linear\" />"));
        let timeline = extract_editable_animation_timeline(&output).unwrap();
        assert!(
            timeline
                .targets
                .iter()
                .any(|target| target.property == "skewX")
        );
    }

    #[test]
    fn extracts_time_keys_with_compat_frame() {
        let script = r##"<Graph fps={60} duration="1s" size={[100,100]}>
  <Scene id="scene0">
    <Timeline>
      <Track>
        <Sequence duration="1s">
          <Layer>
            <Rect id="card" x="0" y="0" width="10" height="10" color="#fff" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>

  <AnimationTarget node="card" property="x">
    <Key time="0.5s" value="20" ease="linear" />
  </AnimationTarget>

  <Present from="scene0" />
</Graph>
"##;
        let timeline = extract_editable_animation_timeline(script).unwrap();
        assert_eq!(timeline.targets[0].keys[0].time.as_deref(), Some("0.5s"));
        assert_eq!(timeline.targets[0].keys[0].frame, 30);
    }
}
