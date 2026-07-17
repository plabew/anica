// =========================================
// =========================================
// crates/motionloom/src/scene/domain/skeleton.rs

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::scene::dsl::SkeletonNode;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProportionProfile {
    pub id: &'static str,
    pub head_count: f32,
    pub shoulder_to_head: f32,
    pub torso_ratio: f32,
    pub leg_ratio: f32,
    pub arm_ratio: f32,
    pub hand_ratio: f32,
    pub eye_spacing: f32,
    pub eye_line_ratio: f32,
    pub pelvis_width_ratio: f32,
}

const PROPORTION_PROFILES: [ProportionProfile; 7] = [
    ProportionProfile {
        id: "chibi_2_head",
        head_count: 2.0,
        shoulder_to_head: 1.25,
        torso_ratio: 0.27,
        leg_ratio: 0.28,
        arm_ratio: 0.33,
        hand_ratio: 0.11,
        eye_spacing: 0.78,
        eye_line_ratio: 0.48,
        pelvis_width_ratio: 0.92,
    },
    ProportionProfile {
        id: "chibi_3_head",
        head_count: 3.0,
        shoulder_to_head: 1.40,
        torso_ratio: 0.30,
        leg_ratio: 0.37,
        arm_ratio: 0.38,
        hand_ratio: 0.105,
        eye_spacing: 0.76,
        eye_line_ratio: 0.49,
        pelvis_width_ratio: 0.88,
    },
    ProportionProfile {
        id: "anime_5_head",
        head_count: 5.0,
        shoulder_to_head: 1.72,
        torso_ratio: 0.34,
        leg_ratio: 0.46,
        arm_ratio: 0.43,
        hand_ratio: 0.10,
        eye_spacing: 0.75,
        eye_line_ratio: 0.50,
        pelvis_width_ratio: 0.80,
    },
    ProportionProfile {
        id: "anime_6_head",
        head_count: 6.0,
        shoulder_to_head: 1.85,
        torso_ratio: 0.35,
        leg_ratio: 0.49,
        arm_ratio: 0.44,
        hand_ratio: 0.095,
        eye_spacing: 0.78,
        eye_line_ratio: 0.50,
        pelvis_width_ratio: 0.78,
    },
    ProportionProfile {
        id: "heroic_7_head",
        head_count: 7.0,
        shoulder_to_head: 2.15,
        torso_ratio: 0.36,
        leg_ratio: 0.51,
        arm_ratio: 0.45,
        hand_ratio: 0.09,
        eye_spacing: 0.72,
        eye_line_ratio: 0.51,
        pelvis_width_ratio: 0.73,
    },
    ProportionProfile {
        id: "realistic_7_5_head",
        head_count: 7.5,
        shoulder_to_head: 2.22,
        torso_ratio: 0.37,
        leg_ratio: 0.52,
        arm_ratio: 0.46,
        hand_ratio: 0.085,
        eye_spacing: 0.70,
        eye_line_ratio: 0.52,
        pelvis_width_ratio: 0.72,
    },
    ProportionProfile {
        id: "realistic_8_head",
        head_count: 8.0,
        shoulder_to_head: 2.32,
        torso_ratio: 0.375,
        leg_ratio: 0.525,
        arm_ratio: 0.47,
        hand_ratio: 0.082,
        eye_spacing: 0.68,
        eye_line_ratio: 0.52,
        pelvis_width_ratio: 0.70,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkeletonPosePreset {
    pub id: &'static str,
    pub description: &'static str,
    pub required_roles: &'static [&'static str],
}

const POSE_PRESETS: [SkeletonPosePreset; 6] = [
    SkeletonPosePreset {
        id: "neutral_front",
        description: "Front-facing neutral calibration pose",
        required_roles: &["root", "pelvis", "spine", "chest", "neck", "head"],
    },
    SkeletonPosePreset {
        id: "a_pose",
        description: "Relaxed humanoid A-pose",
        required_roles: &["upper_arm", "forearm", "hand"],
    },
    SkeletonPosePreset {
        id: "t_pose",
        description: "Horizontal-arm retargeting pose",
        required_roles: &["upper_arm", "forearm", "hand"],
    },
    SkeletonPosePreset {
        id: "walk_contact",
        description: "Walk-cycle heel contact pose",
        required_roles: &["thigh", "shin", "foot"],
    },
    SkeletonPosePreset {
        id: "walk_passing",
        description: "Walk-cycle passing pose",
        required_roles: &["thigh", "shin", "foot"],
    },
    SkeletonPosePreset {
        id: "face_neutral",
        description: "Symmetric neutral facial landmark pose",
        required_roles: &["head"],
    },
];

pub fn builtin_proportion_profiles() -> &'static [ProportionProfile] {
    &PROPORTION_PROFILES
}

pub fn builtin_proportion_profile(id: &str) -> Option<&'static ProportionProfile> {
    PROPORTION_PROFILES.iter().find(|profile| profile.id == id)
}

pub fn builtin_skeleton_pose_presets() -> &'static [SkeletonPosePreset] {
    &POSE_PRESETS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkeletonDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkeletonDiagnostic {
    pub severity: SkeletonDiagnosticSeverity,
    pub code: String,
    pub node: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkeletonValidationReport {
    pub diagnostics: Vec<SkeletonDiagnostic>,
    pub corrected_nodes: Vec<String>,
}

impl SkeletonValidationReport {
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|item| item.severity == SkeletonDiagnosticSeverity::Error)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SkeletonOverlayPrimitive {
    Bone {
        id: String,
        from: [f32; 2],
        to: [f32; 2],
    },
    Landmark {
        id: String,
        position: [f32; 2],
    },
    Guide {
        id: String,
        from: [f32; 2],
        to: [f32; 2],
    },
    EllipseRegion {
        id: String,
        center: [f32; 2],
        radius: [f32; 2],
    },
    CapsuleRegion {
        id: String,
        from: [f32; 2],
        to: [f32; 2],
        width: f32,
    },
    Control {
        id: String,
        position: [f32; 2],
    },
}

pub(crate) fn prepare_skeleton(skeleton: &mut SkeletonNode) {
    // Semantic inference keeps legacy skeletons useful without changing their DSL.
    for bone in &mut skeleton.bones {
        if bone.side.is_none() {
            bone.side = infer_side(&bone.id).map(str::to_string);
        }
        if bone.role.is_none() {
            bone.role = Some(infer_role(&bone.id).to_string());
        }
    }

    let _ = auto_correct_skeleton(skeleton);
}

pub fn auto_correct_skeleton(skeleton: &mut SkeletonNode) -> SkeletonValidationReport {
    let mut report = validate_skeleton(skeleton);
    if skeleton.auto_correct.as_deref() == Some("proportions") {
        report.corrected_nodes = correct_symmetric_landmarks(skeleton);
        report = validate_skeleton(skeleton);
        report.corrected_nodes = skeleton
            .constraints
            .iter()
            .filter(|item| item.kind == "symmetry")
            .flat_map(|item| [item.left.clone(), item.right.clone()])
            .flatten()
            .collect();
    }
    report
}

pub fn validate_skeleton(skeleton: &SkeletonNode) -> SkeletonValidationReport {
    let mut report = SkeletonValidationReport::default();
    let profile = skeleton
        .profile
        .as_deref()
        .and_then(builtin_proportion_profile);
    if let Some(profile_id) = skeleton.profile.as_deref()
        && profile.is_none()
    {
        report.diagnostics.push(diagnostic(
            SkeletonDiagnosticSeverity::Error,
            "unknown_profile",
            Some(&skeleton.id),
            format!("Skeleton references unknown proportion profile '{profile_id}'."),
        ));
    }

    validate_unique_ids(skeleton, &mut report);
    let bone_ids = skeleton
        .bones
        .iter()
        .map(|bone| bone.id.as_str())
        .collect::<HashSet<_>>();
    let landmark_ids = skeleton
        .landmarks
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    let measure_ids = skeleton
        .measures
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();

    validate_bone_hierarchy(skeleton, &bone_ids, &mut report);

    for landmark in &skeleton.landmarks {
        if !bone_ids.contains(landmark.bone.as_str()) {
            report.diagnostics.push(diagnostic(
                SkeletonDiagnosticSeverity::Error,
                "missing_landmark_bone",
                Some(&landmark.id),
                format!(
                    "Landmark '{}' references missing bone '{}'.",
                    landmark.id, landmark.bone
                ),
            ));
        }
    }
    for measure in &skeleton.measures {
        for endpoint in [&measure.from, &measure.to] {
            if !landmark_ids.contains(endpoint.as_str()) && !bone_ids.contains(endpoint.as_str()) {
                report.diagnostics.push(diagnostic(
                    SkeletonDiagnosticSeverity::Error,
                    "missing_measure_endpoint",
                    Some(&measure.id),
                    format!(
                        "Measure '{}' references missing endpoint '{}'.",
                        measure.id, endpoint
                    ),
                ));
            }
        }
    }
    for ratio in &skeleton.ratios {
        if !measure_ids.contains(ratio.measure.as_str()) && !is_builtin_measure(&ratio.measure) {
            report.diagnostics.push(diagnostic(
                SkeletonDiagnosticSeverity::Error,
                "missing_ratio_measure",
                Some(&ratio.measure),
                format!("Ratio references missing measure '{}'.", ratio.measure),
            ));
        }
        if let Ok(value) = ratio.value.parse::<f32>()
            && (!value.is_finite() || value <= 0.0)
        {
            report.diagnostics.push(diagnostic(
                SkeletonDiagnosticSeverity::Error,
                "invalid_ratio",
                Some(&ratio.measure),
                "Ratio value must be positive.".to_string(),
            ));
        }
    }
    validate_constraints(skeleton, &bone_ids, &landmark_ids, &mut report);
    validate_regions_guides_and_controls(skeleton, &bone_ids, &landmark_ids, &mut report);
    validate_profile_expectations(skeleton, profile, &mut report);

    if skeleton
        .bones
        .iter()
        .all(|bone| bone.role.as_deref().unwrap_or_default().is_empty())
    {
        report.diagnostics.push(diagnostic(SkeletonDiagnosticSeverity::Warning, "missing_semantics", Some(&skeleton.id), "Skeleton has no semantic bone roles; LLM generation and retargeting will be less reliable.".to_string()));
    }
    report
}

fn validate_bone_hierarchy(
    skeleton: &SkeletonNode,
    bone_ids: &HashSet<&str>,
    report: &mut SkeletonValidationReport,
) {
    let parents = skeleton
        .bones
        .iter()
        .map(|bone| (bone.id.as_str(), bone.parent.as_deref()))
        .collect::<HashMap<_, _>>();
    for bone in &skeleton.bones {
        if let Some(parent) = bone.parent.as_deref()
            && !bone_ids.contains(parent)
        {
            report.diagnostics.push(diagnostic(
                SkeletonDiagnosticSeverity::Error,
                "missing_bone_parent",
                Some(&bone.id),
                format!("Bone '{}' references missing parent '{parent}'.", bone.id),
            ));
        }

        let mut seen = HashSet::new();
        let mut current = Some(bone.id.as_str());
        while let Some(id) = current {
            if !seen.insert(id) {
                report.diagnostics.push(diagnostic(
                    SkeletonDiagnosticSeverity::Error,
                    "bone_parent_cycle",
                    Some(&bone.id),
                    format!("Bone '{}' belongs to a cyclic parent chain.", bone.id),
                ));
                break;
            }
            current = parents.get(id).copied().flatten();
        }
    }
}

fn validate_regions_guides_and_controls(
    skeleton: &SkeletonNode,
    bone_ids: &HashSet<&str>,
    landmark_ids: &HashSet<&str>,
    report: &mut SkeletonValidationReport,
) {
    let reference_exists = |id: &str| bone_ids.contains(id) || landmark_ids.contains(id);
    for region in &skeleton.regions {
        let references = match region.kind.as_str() {
            "ellipse" => vec![region.center.as_deref()],
            "capsule" => vec![region.from.as_deref(), region.to.as_deref()],
            _ => {
                report.diagnostics.push(diagnostic(
                    SkeletonDiagnosticSeverity::Error,
                    "unknown_region_type",
                    Some(&region.id),
                    format!(
                        "Region '{}' uses unsupported type '{}'.",
                        region.id, region.kind
                    ),
                ));
                Vec::new()
            }
        };
        for reference in references {
            match reference {
                Some(id) if reference_exists(id) => {}
                Some(id) => report.diagnostics.push(diagnostic(
                    SkeletonDiagnosticSeverity::Error,
                    "missing_region_target",
                    Some(&region.id),
                    format!("Region '{}' references missing node '{id}'.", region.id),
                )),
                None => report.diagnostics.push(diagnostic(
                    SkeletonDiagnosticSeverity::Error,
                    "incomplete_region",
                    Some(&region.id),
                    format!(
                        "Region '{}' is missing required geometry references.",
                        region.id
                    ),
                )),
            }
        }
    }

    for guide in &skeleton.guides {
        if guide.kind != "line" {
            report.diagnostics.push(diagnostic(
                SkeletonDiagnosticSeverity::Error,
                "unknown_guide_type",
                Some(&guide.id),
                format!(
                    "Guide '{}' uses unsupported type '{}'.",
                    guide.id, guide.kind
                ),
            ));
        }
        if !reference_exists(&guide.through) {
            report.diagnostics.push(diagnostic(
                SkeletonDiagnosticSeverity::Error,
                "missing_guide_target",
                Some(&guide.id),
                format!(
                    "Guide '{}' references missing node '{}'.",
                    guide.id, guide.through
                ),
            ));
        }
    }

    for control in &skeleton.controls {
        if !matches!(control.kind.as_str(), "ik" | "aim") {
            report.diagnostics.push(diagnostic(
                SkeletonDiagnosticSeverity::Error,
                "unknown_control_type",
                Some(&control.id),
                format!(
                    "Control '{}' uses unsupported type '{}'.",
                    control.id, control.kind
                ),
            ));
        }
        let references = control
            .target
            .iter()
            .chain(control.targets.iter())
            .collect::<Vec<_>>();
        if references.is_empty() {
            report.diagnostics.push(diagnostic(
                SkeletonDiagnosticSeverity::Error,
                "incomplete_control",
                Some(&control.id),
                format!("Control '{}' has no target.", control.id),
            ));
        }
        for target in references {
            if !reference_exists(target) {
                report.diagnostics.push(diagnostic(
                    SkeletonDiagnosticSeverity::Error,
                    "missing_control_target",
                    Some(&control.id),
                    format!(
                        "Control '{}' references missing node '{target}'.",
                        control.id
                    ),
                ));
            }
        }
    }
}

pub fn build_skeleton_overlay(skeleton: &SkeletonNode) -> Vec<SkeletonOverlayPrimitive> {
    let bone_positions = bone_world_positions(skeleton);
    let landmarks = landmark_positions(skeleton, &bone_positions);
    let mut overlay = Vec::new();

    for bone in &skeleton.bones {
        let Some(&to) = bone_positions.get(bone.id.as_str()) else {
            continue;
        };
        let from = bone
            .parent
            .as_deref()
            .and_then(|parent| bone_positions.get(parent).copied())
            .unwrap_or(to);
        overlay.push(SkeletonOverlayPrimitive::Bone {
            id: bone.id.clone(),
            from,
            to,
        });
    }
    for (id, position) in &landmarks {
        overlay.push(SkeletonOverlayPrimitive::Landmark {
            id: id.clone(),
            position: *position,
        });
    }
    for guide in &skeleton.guides {
        if let Some(&center) = landmarks.get(&guide.through) {
            let angle = parse_number(&guide.angle).unwrap_or(0.0).to_radians();
            let direction = [angle.cos() * 1000.0, angle.sin() * 1000.0];
            overlay.push(SkeletonOverlayPrimitive::Guide {
                id: guide.id.clone(),
                from: [center[0] - direction[0], center[1] - direction[1]],
                to: [center[0] + direction[0], center[1] + direction[1]],
            });
        }
    }
    for region in &skeleton.regions {
        match region.kind.as_str() {
            "ellipse" => {
                if let Some(center) = region
                    .center
                    .as_ref()
                    .and_then(|id| landmarks.get(id).or_else(|| bone_positions.get(id)))
                    .copied()
                {
                    overlay.push(SkeletonOverlayPrimitive::EllipseRegion {
                        id: region.id.clone(),
                        center,
                        radius: [
                            region
                                .radius_x
                                .as_deref()
                                .and_then(parse_number)
                                .unwrap_or(0.0),
                            region
                                .radius_y
                                .as_deref()
                                .and_then(parse_number)
                                .unwrap_or(0.0),
                        ],
                    });
                }
            }
            "capsule" => {
                let from = region
                    .from
                    .as_ref()
                    .and_then(|id| landmarks.get(id).or_else(|| bone_positions.get(id)))
                    .copied();
                let to = region
                    .to
                    .as_ref()
                    .and_then(|id| landmarks.get(id).or_else(|| bone_positions.get(id)))
                    .copied();
                if let (Some(from), Some(to)) = (from, to) {
                    overlay.push(SkeletonOverlayPrimitive::CapsuleRegion {
                        id: region.id.clone(),
                        from,
                        to,
                        width: region
                            .width
                            .as_deref()
                            .and_then(parse_number)
                            .unwrap_or(0.0),
                    });
                }
            }
            _ => {}
        }
    }
    for control in &skeleton.controls {
        let target = control
            .target
            .as_ref()
            .and_then(|id| landmarks.get(id).or_else(|| bone_positions.get(id)))
            .copied();
        if let Some(position) = target {
            overlay.push(SkeletonOverlayPrimitive::Control {
                id: control.id.clone(),
                position,
            });
        }
    }
    overlay
}

fn validate_unique_ids(skeleton: &SkeletonNode, report: &mut SkeletonValidationReport) {
    let mut ids = HashSet::new();
    for (kind, id) in skeleton
        .bones
        .iter()
        .map(|item| ("Bone", item.id.as_str()))
        .chain(
            skeleton
                .landmarks
                .iter()
                .map(|item| ("Landmark", item.id.as_str())),
        )
        .chain(
            skeleton
                .measures
                .iter()
                .map(|item| ("Measure", item.id.as_str())),
        )
        .chain(
            skeleton
                .regions
                .iter()
                .map(|item| ("Region", item.id.as_str())),
        )
        .chain(
            skeleton
                .guides
                .iter()
                .map(|item| ("Guide", item.id.as_str())),
        )
        .chain(
            skeleton
                .controls
                .iter()
                .map(|item| ("Control", item.id.as_str())),
        )
    {
        if !ids.insert(id) {
            report.diagnostics.push(diagnostic(
                SkeletonDiagnosticSeverity::Error,
                "duplicate_skeleton_node",
                Some(id),
                format!("Duplicate {kind} id '{id}' in skeleton '{}'.", skeleton.id),
            ));
        }
    }
}

fn validate_constraints(
    skeleton: &SkeletonNode,
    bone_ids: &HashSet<&str>,
    landmark_ids: &HashSet<&str>,
    report: &mut SkeletonValidationReport,
) {
    let reference_exists = |id: &str| bone_ids.contains(id) || landmark_ids.contains(id);
    for constraint in &skeleton.constraints {
        let required = match constraint.kind.as_str() {
            "symmetry" => vec![
                constraint.left.as_deref(),
                constraint.right.as_deref(),
                constraint.axis.as_deref(),
            ],
            "distance" => vec![constraint.from.as_deref(), constraint.to.as_deref()],
            "anglelimit" | "lengthratio" => vec![constraint.bone.as_deref()],
            _ => {
                report.diagnostics.push(diagnostic(
                    SkeletonDiagnosticSeverity::Error,
                    "unknown_constraint",
                    None,
                    format!(
                        "Unsupported skeleton constraint type '{}'.",
                        constraint.kind
                    ),
                ));
                Vec::new()
            }
        };
        for reference in required {
            match reference {
                Some(id) if reference_exists(id) => {}
                Some(id) => report.diagnostics.push(diagnostic(
                    SkeletonDiagnosticSeverity::Error,
                    "missing_constraint_target",
                    Some(id),
                    format!(
                        "Constraint '{}' references missing node '{id}'.",
                        constraint.kind
                    ),
                )),
                None => report.diagnostics.push(diagnostic(
                    SkeletonDiagnosticSeverity::Error,
                    "incomplete_constraint",
                    None,
                    format!(
                        "Constraint '{}' is missing a required target.",
                        constraint.kind
                    ),
                )),
            }
        }
    }
}

fn validate_profile_expectations(
    skeleton: &SkeletonNode,
    profile: Option<&ProportionProfile>,
    report: &mut SkeletonValidationReport,
) {
    let Some(profile) = profile else { return };
    let expected = [
        ("head_height", 1.0 / profile.head_count),
        ("shoulder_width", profile.shoulder_to_head),
        ("leg_length", profile.leg_ratio),
    ];
    for (measure, expected_value) in expected {
        let Some(ratio) = skeleton
            .ratios
            .iter()
            .find(|ratio| ratio.measure == measure)
        else {
            continue;
        };
        let Ok(actual) = ratio.value.parse::<f32>() else {
            continue;
        };
        let tolerance = ratio
            .tolerance
            .as_deref()
            .and_then(parse_number)
            .unwrap_or(0.12);
        if (actual - expected_value).abs() > tolerance {
            report.diagnostics.push(diagnostic(SkeletonDiagnosticSeverity::Warning, "profile_ratio_mismatch", Some(measure), format!("Ratio {measure}={actual:.4} differs from profile {} expectation {expected_value:.4}.", profile.id)));
        }
    }
}

fn correct_symmetric_landmarks(skeleton: &mut SkeletonNode) -> Vec<String> {
    let mut corrected = Vec::new();
    let constraints = skeleton.constraints.clone();
    for constraint in constraints.iter().filter(|item| item.kind == "symmetry") {
        let (Some(left_id), Some(right_id)) =
            (constraint.left.as_deref(), constraint.right.as_deref())
        else {
            continue;
        };
        let left = skeleton
            .landmarks
            .iter()
            .position(|item| item.id == left_id);
        let right = skeleton
            .landmarks
            .iter()
            .position(|item| item.id == right_id);
        let (Some(left), Some(right)) = (left, right) else {
            continue;
        };
        let (Some(lx), Some(ly), Some(rx), Some(ry)) = (
            parse_number(&skeleton.landmarks[left].offset.0),
            parse_number(&skeleton.landmarks[left].offset.1),
            parse_number(&skeleton.landmarks[right].offset.0),
            parse_number(&skeleton.landmarks[right].offset.1),
        ) else {
            continue;
        };
        let extent = (lx.abs() + rx.abs()) * 0.5;
        let y = (ly + ry) * 0.5;
        skeleton.landmarks[left].offset = (format_number(-extent), format_number(y));
        skeleton.landmarks[right].offset = (format_number(extent), format_number(y));
        corrected.push(left_id.to_string());
        corrected.push(right_id.to_string());
    }
    corrected
}

fn bone_world_positions(skeleton: &SkeletonNode) -> HashMap<String, [f32; 2]> {
    let mut positions = HashMap::new();
    for _ in 0..skeleton.bones.len().max(1) {
        for bone in &skeleton.bones {
            if positions.contains_key(&bone.id) {
                continue;
            }
            let local = [
                parse_number(&bone.x).unwrap_or(0.0),
                parse_number(&bone.y).unwrap_or(0.0),
            ];
            let parent = match bone.parent.as_deref() {
                Some(parent) => match positions.get(parent) {
                    Some(value) => *value,
                    None => continue,
                },
                None => [0.0, 0.0],
            };
            positions.insert(
                bone.id.clone(),
                [parent[0] + local[0], parent[1] + local[1]],
            );
        }
    }
    positions
}

fn landmark_positions(
    skeleton: &SkeletonNode,
    bones: &HashMap<String, [f32; 2]>,
) -> HashMap<String, [f32; 2]> {
    skeleton
        .landmarks
        .iter()
        .filter_map(|landmark| {
            let bone = bones.get(&landmark.bone)?;
            Some((
                landmark.id.clone(),
                [
                    bone[0] + parse_number(&landmark.offset.0).unwrap_or(0.0),
                    bone[1] + parse_number(&landmark.offset.1).unwrap_or(0.0),
                ],
            ))
        })
        .collect()
}

fn infer_side(id: &str) -> Option<&'static str> {
    let lower = id.to_ascii_lowercase();
    if lower.starts_with("left_") || lower.ends_with("_l") {
        Some("left")
    } else if lower.starts_with("right_") || lower.ends_with("_r") {
        Some("right")
    } else {
        None
    }
}

fn infer_role(id: &str) -> &str {
    let lower = id.to_ascii_lowercase();
    for role in [
        "root",
        "pelvis",
        "spine",
        "chest",
        "neck",
        "head",
        "upper_arm",
        "forearm",
        "hand",
        "thigh",
        "shin",
        "foot",
    ] {
        if lower.contains(role) {
            return role;
        }
    }
    "custom"
}

fn is_builtin_measure(id: &str) -> bool {
    matches!(
        id,
        "body_height"
            | "head_height"
            | "head_width"
            | "shoulder_width"
            | "leg_length"
            | "arm_length"
            | "pelvis_width"
    )
}

fn parse_number(value: &str) -> Option<f32> {
    value
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
}

fn format_number(value: f32) -> String {
    let mut text = format!("{value:.4}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

fn diagnostic(
    severity: SkeletonDiagnosticSeverity,
    code: &str,
    node: Option<&str>,
    message: String,
) -> SkeletonDiagnostic {
    SkeletonDiagnostic {
        severity,
        code: code.to_string(),
        node: node.map(str::to_string),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_graph_script;

    #[test]
    fn profile_catalog_covers_chibi_anime_and_realistic_bodies() {
        assert!(builtin_proportion_profile("chibi_2_head").is_some());
        assert!(builtin_proportion_profile("anime_6_head").is_some());
        assert!(builtin_proportion_profile("realistic_8_head").is_some());
    }

    #[test]
    fn proportion_autocorrect_mirrors_landmarks() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[100,100]}>
  <Skeleton id="face" profile="anime_6_head" autoCorrect="proportions">
    <Bone id="head" role="head" />
    <Landmark id="axis" bone="head" offset={[0,0]} />
    <Landmark id="left_eye" bone="head" offset={[-20,-4]} />
    <Landmark id="right_eye" bone="head" offset={[28,-8]} />
    <Constraint type="symmetry" left="left_eye" right="right_eye" axis="axis" />
  </Skeleton>
  <Background color="#fff" />
  <Present from="scene" />
</Graph>
"##,
        )
        .expect("parse profile skeleton");
        assert_eq!(
            graph.skeletons[0].landmarks[1].offset,
            ("-24".to_string(), "-6".to_string())
        );
        assert_eq!(
            graph.skeletons[0].landmarks[2].offset,
            ("24".to_string(), "-6".to_string())
        );
    }

    #[test]
    fn full_profile_rig_parses_validates_and_builds_editor_overlay() {
        let graph = parse_graph_script(
            r##"
<Graph fps={30} duration="1s" size={[320,240]}>
  <Skeleton id="hero" profile="anime_6_head" height="720"
            symmetryAxis="body_center" validation="strict"
            autoCorrect="proportions" overlay="true">
    <Bone id="root" role="root" x="0" y="0" />
    <Bone id="chest" role="chest" parent="root" x="0" y="-80" />
    <Bone id="head" role="head" parent="chest" x="0" y="-80" />
    <Bone id="left_hand" role="hand" side="left" parent="chest" x="-80" y="40" />
    <Landmark id="body_center" bone="root" offset={[0,0]} />
    <Landmark id="face_center" bone="head" offset={[0,0]} />
    <Landmark id="eye_line" bone="head" offset={[0,-10]} />
    <Landmark id="left_eye" bone="head" offset={[-24,-10]} />
    <Landmark id="right_eye" bone="head" offset={[24,-10]} />
    <Landmark id="head_top" bone="head" offset={[0,-60]} />
    <Landmark id="chin" bone="head" offset={[0,60]} />
    <Measure id="head_height" from="head_top" to="chin" />
    <Ratio measure="head_height" relativeTo="body_height" value="0.1667" />
    <Region id="head_volume" role="head" type="ellipse"
            center="face_center" radiusX="60" radiusY="72" />
    <Constraint type="symmetry" left="left_eye" right="right_eye" axis="body_center" />
    <Constraint type="angleLimit" bone="left_hand" min="-90" max="90" />
    <Guide id="eye_horizontal" type="line" through="eye_line" angle="0" />
    <Control id="left_hand_control" type="ik" target="left_hand" chainLength="1" />
    <Control id="look_control" type="aim" targets={["left_eye","right_eye"]} />
  </Skeleton>
  <Background color="#ffffff" />
  <Present from="scene" />
</Graph>
"##,
        )
        .expect("parse complete profile rig");
        let skeleton = &graph.skeletons[0];
        assert_eq!(skeleton.profile.as_deref(), Some("anime_6_head"));
        assert_eq!(skeleton.landmarks.len(), 7);
        assert_eq!(skeleton.regions.len(), 1);
        assert_eq!(skeleton.controls.len(), 2);
        assert!(!validate_skeleton(skeleton).has_errors());
        assert!(build_skeleton_overlay(skeleton).len() >= 10);
    }
}
