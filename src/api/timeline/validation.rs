use std::collections::HashSet;

use super::errors::{TimelineEditError, TimelineValidationError};
use super::*;

fn push_op_validation_error(
    errors: &mut Vec<String>,
    op_label: &str,
    err: TimelineValidationError,
) {
    errors.push(format!("{op_label}: {err}"));
}

fn validate_insert_clip_common(
    global: &GlobalState,
    track_type: &str,
    track_index: Option<usize>,
    media_pool_item_id: Option<usize>,
    path: Option<&str>,
    source_in_ms: Option<u64>,
    source_out_ms: Option<u64>,
    duration_ms: Option<u64>,
    errors: &mut Vec<TimelineValidationError>,
) {
    let Some(kind) = parse_track_kind(track_type) else {
        errors.push(TimelineValidationError::InvalidTrackTypeForInsertClip {
            track_type: track_type.to_string(),
        });
        return;
    };
    let has_media_id = media_pool_item_id.is_some();
    let has_path = path.is_some_and(|p| !p.trim().is_empty());
    if !has_media_id && !has_path {
        errors.push(TimelineValidationError::from(
            TimelineEditError::InsertClipRequiresMediaPoolItemIdOrPath,
        ));
    }
    if let Some(media_pool_item_id) = media_pool_item_id
        && media_pool_item_id >= global.media_pool.len()
    {
        errors.push(TimelineValidationError::from(
            TimelineEditError::MediaPoolItemOutOfRange {
                media_pool_item_id,
                media_pool_len: global.media_pool.len(),
            },
        ));
    }
    if duration_ms.is_some_and(|ms| ms == 0) {
        errors.push(TimelineValidationError::InsertClipDurationMustBePositive);
    }
    let source_in = source_in_ms.unwrap_or(0);
    if let Some(source_out) = source_out_ms
        && source_out <= source_in
    {
        errors.push(TimelineValidationError::from(
            TimelineEditError::SourceOutMustBeGreaterThanSourceIn,
        ));
    }
    match kind {
        ApiTrackKind::V1 => {
            if track_index.is_some_and(|idx| idx != 0) {
                errors.push(TimelineValidationError::V1InsertClipOnlySupportsTrackIndexZero);
            }
        }
        ApiTrackKind::Audio | ApiTrackKind::Video => {
            let Some(track_index) = track_index else {
                errors.push(TimelineValidationError::InsertClipRequiresTrackIndex {
                    track_type: track_type.to_string(),
                });
                return;
            };
            if !track_exists(global, kind, track_index) {
                errors.push(TimelineValidationError::InsertClipTargetTrackNotFound {
                    track_type: track_type.to_string(),
                    track_index,
                });
            }
        }
        ApiTrackKind::Subtitle => {
            errors.push(TimelineValidationError::InsertClipDoesNotSupportSubtitleTracks);
        }
    }
}

pub fn validate_edit_plan(
    global: &GlobalState,
    request: &TimelineEditPlanRequest,
) -> TimelineEditValidationResponse {
    let before_revision = timeline_revision(global);
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut estimated_removed_ms = 0_u64;
    let mut affected_clip_ids = HashSet::new();
    let mut raw_ripple_ranges: Vec<(u64, u64)> = Vec::new();

    if let Some(expected) = request.based_on_revision.as_deref()
        && expected != before_revision
    {
        errors.push(
            TimelineValidationError::RevisionMismatch {
                expected: expected.to_string(),
                actual: before_revision.clone(),
            }
            .to_string(),
        );
    }

    if request.operations.is_empty() {
        errors.push(TimelineValidationError::NoOperationsProvided.to_string());
    }

    for (op_index, op) in request.operations.iter().enumerate() {
        let op_label = format!("op#{}", op_index.saturating_add(1));
        match op {
            TimelineEditOperation::AddTrack { track_type, .. } => {
                let Some(kind) = parse_track_kind(track_type) else {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::InvalidTrackTypeForAddTrack {
                            track_type: track_type.clone(),
                        },
                    );
                    continue;
                };
                if matches!(kind, ApiTrackKind::V1) {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::from(TimelineEditError::V1CannotBeAddedOrRemoved),
                    );
                }
            }
            TimelineEditOperation::AddAudioTrack { .. }
            | TimelineEditOperation::AddVideoTrack { .. }
            | TimelineEditOperation::AddSubtitleTrack { .. } => {}
            TimelineEditOperation::RemoveTrack { track_type, index } => {
                let Some(kind) = parse_track_kind(track_type) else {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::InvalidTrackTypeForRemoveTrack {
                            track_type: track_type.clone(),
                        },
                    );
                    continue;
                };
                if matches!(kind, ApiTrackKind::V1) {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::from(TimelineEditError::V1CannotBeAddedOrRemoved),
                    );
                    continue;
                }
                if !track_exists(global, kind, *index) {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::RemoveTrackTargetNotFound {
                            track_type: track_type.clone(),
                            index: *index,
                        },
                    );
                }
            }
            TimelineEditOperation::RemoveAudioTrack { index } => {
                if !track_exists(global, ApiTrackKind::Audio, *index) {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::RemoveAudioTrackTargetNotFound { index: *index },
                    );
                }
            }
            TimelineEditOperation::RemoveVideoTrack { index } => {
                if !track_exists(global, ApiTrackKind::Video, *index) {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::RemoveVideoTrackTargetNotFound { index: *index },
                    );
                }
            }
            TimelineEditOperation::RemoveSubtitleTrack { index } => {
                if !track_exists(global, ApiTrackKind::Subtitle, *index) {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::RemoveSubtitleTrackTargetNotFound {
                            index: *index,
                        },
                    );
                }
            }
            TimelineEditOperation::SetTrackVisibility {
                track_type, index, ..
            }
            | TimelineEditOperation::SetTrackLock {
                track_type, index, ..
            } => {
                let Some(kind) = parse_track_kind(track_type) else {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::InvalidTrackTypeForTrackStateUpdate {
                            track_type: track_type.clone(),
                        },
                    );
                    continue;
                };
                if !track_exists(global, kind, *index) {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::TrackNotFound {
                            track_type: track_type.clone(),
                            index: *index,
                        },
                    );
                }
            }
            TimelineEditOperation::SetTrackMute {
                track_type, index, ..
            } => {
                let Some(kind) = parse_track_kind(track_type) else {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::InvalidTrackTypeForTrackMute {
                            track_type: track_type.clone(),
                        },
                    );
                    continue;
                };
                if matches!(kind, ApiTrackKind::V1) {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::from(TimelineEditError::V1DoesNotSupportMute),
                    );
                    continue;
                }
                if !track_exists(global, kind, *index) {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::TrackNotFound {
                            track_type: track_type.clone(),
                            index: *index,
                        },
                    );
                }
            }
            TimelineEditOperation::InsertClip {
                track_type,
                track_index,
                media_pool_item_id,
                path,
                source_in_ms,
                source_out_ms,
                duration_ms,
                ..
            } => {
                let mut op_errors = Vec::new();
                validate_insert_clip_common(
                    global,
                    track_type,
                    *track_index,
                    *media_pool_item_id,
                    path.as_deref(),
                    *source_in_ms,
                    *source_out_ms,
                    *duration_ms,
                    &mut op_errors,
                );
                for err in op_errors {
                    push_op_validation_error(&mut errors, &op_label, err);
                }
            }
            TimelineEditOperation::InsertFromMediaPool {
                track_type,
                track_index,
                media_pool_item_id,
                source_in_ms,
                source_out_ms,
                duration_ms,
                ..
            } => {
                let mut op_errors = Vec::new();
                validate_insert_clip_common(
                    global,
                    track_type,
                    *track_index,
                    Some(*media_pool_item_id),
                    None,
                    *source_in_ms,
                    *source_out_ms,
                    *duration_ms,
                    &mut op_errors,
                );
                for err in op_errors {
                    push_op_validation_error(&mut errors, &op_label, err);
                }
            }
            TimelineEditOperation::SetSourceInOut {
                clip_id,
                source_in_ms,
                source_out_ms,
                duration_ms,
            } => {
                if find_clip_ref(global, *clip_id).is_none() {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::from(TimelineEditError::ClipNotFound {
                            clip_id: *clip_id,
                        }),
                    );
                    continue;
                }
                if duration_ms.is_some_and(|ms| ms == 0) {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::SetSourceInOutDurationMustBePositive,
                    );
                }
                if let Some(source_out_ms) = source_out_ms
                    && *source_out_ms <= *source_in_ms
                {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::SetSourceInOutSourceOutMustBeGreaterThanSourceIn,
                    );
                }
                affected_clip_ids.insert(*clip_id);
            }
            TimelineEditOperation::DeleteClip { clip_id, ripple } => {
                let Some(clip) = find_clip_ref(global, *clip_id) else {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::from(TimelineEditError::ClipNotFound {
                            clip_id: *clip_id,
                        }),
                    );
                    continue;
                };
                affected_clip_ids.insert(*clip_id);
                if ripple.unwrap_or(false) {
                    raw_ripple_ranges
                        .push((duration_to_ms(clip.start), duration_to_ms(clip.end())));
                }
            }
            TimelineEditOperation::DeleteTrackClips {
                track_type,
                track_index,
                with_linked,
            } => {
                let Some(kind) = parse_track_kind(track_type) else {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::InvalidTrackTypeForDeleteTrackClips {
                            track_type: track_type.clone(),
                        },
                    );
                    continue;
                };
                let resolved_index =
                    match resolve_indexed_track_target(track_type, kind, *track_index) {
                        Ok(index) => index,
                        Err(err) => {
                            push_op_validation_error(
                                &mut errors,
                                &op_label,
                                TimelineValidationError::from(err),
                            );
                            continue;
                        }
                    };
                if !track_exists(global, kind, resolved_index) {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::from(
                            TimelineEditError::DeleteTrackClipsTargetNotFound {
                                track_type: track_type.clone(),
                                track_index: resolved_index,
                            },
                        ),
                    );
                    continue;
                }
                match kind {
                    ApiTrackKind::Subtitle => {
                        for clip_id in subtitle_ids_on_track(global, resolved_index) {
                            affected_clip_ids.insert(clip_id);
                        }
                    }
                    ApiTrackKind::V1 | ApiTrackKind::Audio | ApiTrackKind::Video => {
                        let base_ids: HashSet<u64> =
                            clip_ids_on_track(global, kind, resolved_index)
                                .into_iter()
                                .collect();
                        let clip_ids = if *with_linked {
                            expand_clip_ids_by_link_group(global, &base_ids)
                        } else {
                            base_ids
                        };
                        for clip_id in clip_ids {
                            affected_clip_ids.insert(clip_id);
                        }
                    }
                }
            }
            TimelineEditOperation::RippleDeleteRange {
                start_ms,
                end_ms,
                mode,
            } => {
                if end_ms <= start_ms {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::InvalidRippleDeleteRange {
                            start_ms: *start_ms,
                            end_ms: *end_ms,
                        },
                    );
                    continue;
                }
                if let Some(mode) = mode {
                    let normalized = mode.trim().to_ascii_lowercase();
                    if normalized != "all_tracks" && normalized != "all" {
                        push_op_validation_error(
                            &mut errors,
                            &op_label,
                            TimelineValidationError::UnsupportedRippleDeleteRangeMode {
                                mode: mode.clone(),
                            },
                        );
                        continue;
                    }
                }
                raw_ripple_ranges.push((*start_ms, *end_ms));
            }
            TimelineEditOperation::TrimClip {
                clip_id,
                new_duration_ms,
                ..
            } => {
                if find_clip_ref(global, *clip_id).is_none() {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::from(TimelineEditError::ClipNotFound {
                            clip_id: *clip_id,
                        }),
                    );
                    continue;
                }
                if *new_duration_ms == 0 {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::TrimClipDurationMustBePositive,
                    );
                } else {
                    affected_clip_ids.insert(*clip_id);
                }
            }
            TimelineEditOperation::MoveClip { clip_id, .. } => {
                if find_clip_ref(global, *clip_id).is_none() {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::from(TimelineEditError::ClipNotFound {
                            clip_id: *clip_id,
                        }),
                    );
                } else {
                    affected_clip_ids.insert(*clip_id);
                }
            }
            TimelineEditOperation::SplitClip { clip_id, at_ms } => {
                let Some(clip) = find_clip_ref(global, *clip_id) else {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::from(TimelineEditError::ClipNotFound {
                            clip_id: *clip_id,
                        }),
                    );
                    continue;
                };
                let at = ms_to_duration(*at_ms);
                if at <= clip.start || at >= clip.end() {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::SplitClipAtMustBeStrictlyInsideRange,
                    );
                } else {
                    affected_clip_ids.insert(*clip_id);
                }
            }
            TimelineEditOperation::ShiftSubtitlesRange {
                start_ms, end_ms, ..
            } => {
                if end_ms <= start_ms {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::InvalidShiftSubtitlesRange {
                            start_ms: *start_ms,
                            end_ms: *end_ms,
                        },
                    );
                }
            }
            TimelineEditOperation::GenerateSubtitles {
                track_index,
                entries,
            } => {
                if let Some(track_index) = track_index
                    && !track_exists(global, ApiTrackKind::Subtitle, *track_index)
                {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::from(TimelineEditError::SubtitleTrackNotFound {
                            track_index: *track_index,
                        }),
                    );
                }
                if entries.is_empty() {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::GenerateSubtitlesRequiresNonEmptyEntries,
                    );
                }
                for (entry_index, entry) in entries.iter().enumerate() {
                    if entry.duration_ms == 0 {
                        push_op_validation_error(
                            &mut errors,
                            &op_label,
                            TimelineValidationError::GenerateSubtitleEntryDurationMustBePositive {
                                entry_index,
                            },
                        );
                    }
                    if entry.text.trim().is_empty() {
                        warnings.push(format!(
                            "{op_label}: entries[{entry_index}] has empty text."
                        ));
                    }
                }
            }
            TimelineEditOperation::MoveSubtitle {
                clip_id,
                to_track_index,
                ..
            } => {
                if find_subtitle_ref(global, *clip_id).is_none() {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::from(TimelineEditError::SubtitleClipNotFound {
                            clip_id: *clip_id,
                        }),
                    );
                    continue;
                }
                if let Some(track_index) = to_track_index
                    && !track_exists(global, ApiTrackKind::Subtitle, *track_index)
                {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::from(
                            TimelineEditError::SubtitleTargetTrackNotFound {
                                track_index: *track_index,
                            },
                        ),
                    );
                }
            }
            TimelineEditOperation::BatchUpdateSubtitles { updates } => {
                if updates.is_empty() {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::BatchUpdateSubtitlesRequiresNonEmptyUpdates,
                    );
                    continue;
                }
                for (update_index, patch) in updates.iter().enumerate() {
                    if find_subtitle_ref(global, patch.clip_id).is_none() {
                        push_op_validation_error(
                            &mut errors,
                            &op_label,
                            TimelineValidationError::BatchUpdateSubtitleClipNotFound {
                                update_index,
                                clip_id: patch.clip_id,
                            },
                        );
                        continue;
                    }
                    if patch.duration_ms.is_some_and(|ms| ms == 0) {
                        push_op_validation_error(
                            &mut errors,
                            &op_label,
                            TimelineValidationError::BatchUpdateSubtitleDurationMustBePositive {
                                update_index,
                            },
                        );
                    }
                    if let Some(track_index) = patch.track_index
                        && !track_exists(global, ApiTrackKind::Subtitle, track_index)
                    {
                        push_op_validation_error(
                            &mut errors,
                            &op_label,
                            TimelineValidationError::BatchUpdateSubtitleTargetTrackNotFound {
                                update_index,
                                track_index,
                            },
                        );
                    }
                }
            }
            TimelineEditOperation::DeleteSubtitleRange {
                start_ms,
                end_ms,
                track_indices,
            } => {
                if end_ms <= start_ms {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::InvalidDeleteSubtitleRange {
                            start_ms: *start_ms,
                            end_ms: *end_ms,
                        },
                    );
                }
                if let Some(indexes) = track_indices
                    && let Some(missing) =
                        first_missing_track_index(global, ApiTrackKind::Subtitle, indexes)
                {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::from(TimelineEditError::SubtitleTrackNotFound {
                            track_index: missing,
                        }),
                    );
                }
            }
            TimelineEditOperation::ApplyEffect {
                clip_id, effect, ..
            }
            | TimelineEditOperation::UpdateEffectParams {
                clip_id, effect, ..
            }
            | TimelineEditOperation::RemoveEffect { clip_id, effect } => {
                if find_clip_ref(global, *clip_id).is_none() {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::from(TimelineEditError::ClipNotFound {
                            clip_id: *clip_id,
                        }),
                    );
                    continue;
                }
                if !effect_name_supported(effect) {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::from(TimelineEditError::UnsupportedEffect {
                            effect: effect.clone(),
                        }),
                    );
                } else {
                    affected_clip_ids.insert(*clip_id);
                }
            }
            TimelineEditOperation::ApplyTransition {
                clip_id,
                transition,
            }
            | TimelineEditOperation::UpdateTransition {
                clip_id,
                transition,
                ..
            } => {
                if find_clip_ref(global, *clip_id).is_none() {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::from(TimelineEditError::ClipNotFound {
                            clip_id: *clip_id,
                        }),
                    );
                    continue;
                }
                if !transition_name_supported(transition) {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::from(TimelineEditError::UnsupportedTransition {
                            transition: transition.clone(),
                        }),
                    );
                } else {
                    affected_clip_ids.insert(*clip_id);
                }
            }
            TimelineEditOperation::RemoveTransition {
                clip_id,
                transition,
            } => {
                if find_clip_ref(global, *clip_id).is_none() {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::from(TimelineEditError::ClipNotFound {
                            clip_id: *clip_id,
                        }),
                    );
                    continue;
                }
                if let Some(transition) = transition {
                    let normalized = transition.trim().to_ascii_lowercase();
                    if normalized != "all" && !transition_name_supported(&normalized) {
                        push_op_validation_error(
                            &mut errors,
                            &op_label,
                            TimelineValidationError::from(
                                TimelineEditError::UnsupportedTransition {
                                    transition: transition.clone(),
                                },
                            ),
                        );
                        continue;
                    }
                }
                affected_clip_ids.insert(*clip_id);
            }
            // Validate semantic layer marker insertion (B-roll planning annotations).
            TimelineEditOperation::InsertSemanticClip {
                duration_ms,
                semantic_type,
                prompt_schema,
                ..
            } => {
                if *duration_ms == 0 {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::InsertSemanticClipDurationMustBePositive,
                    );
                }
                if let Some(semantic_type) = semantic_type
                    && semantic_type.trim().is_empty()
                {
                    push_op_validation_error(
                        &mut errors,
                        &op_label,
                        TimelineValidationError::InsertSemanticClipSemanticTypeMustBeNonEmpty,
                    );
                }
                if let Some(prompt_schema) = prompt_schema {
                    if !prompt_schema.is_object() {
                        push_op_validation_error(
                            &mut errors,
                            &op_label,
                            TimelineValidationError::InsertSemanticClipPromptSchemaMustBeObject,
                        );
                    } else {
                        let mode = semantic_schema_asset_mode(prompt_schema);
                        let provider = semantic_schema_provider(prompt_schema);
                        if mode == "video" && provider == "veo_3_1" {
                            let max_sec =
                                semantic_schema_provider_limit_sec(prompt_schema, "veo_3_1")
                                    .unwrap_or(8.0);
                            let duration_sec = (*duration_ms as f64) / 1000.0;
                            if duration_sec > max_sec + 0.0001 {
                                push_op_validation_error(
                                    &mut errors,
                                    &op_label,
                                    TimelineValidationError::SemanticDurationExceedsVeoLimit {
                                        duration_sec,
                                        max_sec,
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    let (merged_ranges, input_ripple_count) = canonicalize_ripple_delete_ranges(&raw_ripple_ranges);
    if input_ripple_count > merged_ranges.len() && !merged_ranges.is_empty() {
        warnings.push(format!(
            "Merged {} ripple_delete_range ops into {} non-overlapping ranges.",
            input_ripple_count,
            merged_ranges.len()
        ));
    }

    for (start_ms, end_ms) in &merged_ranges {
        estimated_removed_ms = estimated_removed_ms.saturating_add(end_ms - start_ms);
        let start = Duration::from_millis(*start_ms);
        let end = Duration::from_millis(*end_ms);
        for clip in global
            .v1_clips
            .iter()
            .chain(global.audio_tracks.iter().flat_map(|t| t.clips.iter()))
            .chain(global.video_tracks.iter().flat_map(|t| t.clips.iter()))
        {
            if clip.start < end && clip.end() > start {
                affected_clip_ids.insert(clip.id);
            }
        }
    }

    let mut affected_clip_ids: Vec<u64> = affected_clip_ids.into_iter().collect();
    affected_clip_ids.sort_unstable();

    TimelineEditValidationResponse {
        ok: errors.is_empty(),
        before_revision,
        errors,
        warnings,
        estimated_removed_ms,
        affected_clip_ids,
    }
}
