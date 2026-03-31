use thiserror::Error;

#[derive(Debug, Error)]
pub(super) enum TimelineEditError {
    #[error("invalid track_type `{track_type}` for {operation}")]
    InvalidTrackTypeForOperation {
        track_type: String,
        operation: &'static str,
    },
    #[error("invalid track_type `{track_type}` for insert_clip")]
    InvalidTrackTypeForInsertClip { track_type: String },
    #[error("V1 cannot be added or removed.")]
    V1CannotBeAddedOrRemoved,
    #[error("V1 does not support mute.")]
    V1DoesNotSupportMute,
    #[error("media_pool_item_id {media_pool_item_id} out of range (len={media_pool_len}).")]
    MediaPoolItemOutOfRange {
        media_pool_item_id: usize,
        media_pool_len: usize,
    },
    #[error("insert_clip requires `media_pool_item_id` or `path`.")]
    InsertClipRequiresMediaPoolItemIdOrPath,
    #[error("source_out_ms must be greater than source_in_ms.")]
    SourceOutMustBeGreaterThanSourceIn,
    #[error("source window is empty after clamping.")]
    SourceWindowEmptyAfterClamping,
    #[error("resolved insert duration is zero.")]
    ResolvedInsertDurationIsZero,
    #[error("resolved insert duration is outside media range.")]
    ResolvedInsertDurationOutsideMediaRange,
    #[error("insert_clip requires track_index for audio.")]
    InsertClipRequiresAudioTrackIndex,
    #[error("insert_clip requires track_index for video.")]
    InsertClipRequiresVideoTrackIndex,
    #[error("insert_clip does not support subtitle tracks.")]
    InsertClipDoesNotSupportSubtitleTracks,
    #[error("clip_id {clip_id} not found.")]
    ClipNotFound { clip_id: u64 },
    #[error("subtitle clip_id {clip_id} not found.")]
    SubtitleClipNotFound { clip_id: u64 },
    #[error("Unsupported effect `{effect}`.")]
    UnsupportedEffect { effect: String },
    #[error("Unsupported transition `{transition}`.")]
    UnsupportedTransition { transition: String },
    #[error("Failed to apply transition `{transition}` to clip {clip_id}.")]
    FailedToApplyTransition { transition: String, clip_id: u64 },
    #[error("delete_track_clips target not found ({track_type}[{track_index}]).")]
    DeleteTrackClipsTargetNotFound {
        track_type: String,
        track_index: usize,
    },
    #[error("subtitle track not found (subtitle[{track_index}]).")]
    SubtitleTrackNotFound { track_index: usize },
    #[error("subtitle target track not found (subtitle[{track_index}]).")]
    SubtitleTargetTrackNotFound { track_index: usize },
    #[error("{message}")]
    SplitClipFailed { message: String },
    #[error("track_type `{track_type}` only supports track_index 0 or null (got {index}).")]
    TrackTypeOnlySupportsTrackIndexZero { track_type: String, index: usize },
    #[error("track_type `{track_type}` requires track_index.")]
    TrackTypeRequiresTrackIndex { track_type: String },
}

#[derive(Debug, Error)]
pub(super) enum TimelineValidationError {
    #[error(transparent)]
    Edit(#[from] TimelineEditError),
    #[error("Revision mismatch. expected={expected}, actual={actual}")]
    RevisionMismatch { expected: String, actual: String },
    #[error("No operations provided.")]
    NoOperationsProvided,
    #[error("invalid track_type `{track_type}` for add_track.")]
    InvalidTrackTypeForAddTrack { track_type: String },
    #[error("invalid track_type `{track_type}` for remove_track.")]
    InvalidTrackTypeForRemoveTrack { track_type: String },
    #[error("remove_track target not found ({track_type}[{index}]).")]
    RemoveTrackTargetNotFound { track_type: String, index: usize },
    #[error("remove_audio_track target not found (audio[{index}]).")]
    RemoveAudioTrackTargetNotFound { index: usize },
    #[error("remove_video_track target not found (video[{index}]).")]
    RemoveVideoTrackTargetNotFound { index: usize },
    #[error("remove_subtitle_track target not found (subtitle[{index}]).")]
    RemoveSubtitleTrackTargetNotFound { index: usize },
    #[error("invalid track_type `{track_type}` for track state update.")]
    InvalidTrackTypeForTrackStateUpdate { track_type: String },
    #[error("track not found ({track_type}[{index}]).")]
    TrackNotFound { track_type: String, index: usize },
    #[error("invalid track_type `{track_type}` for track mute.")]
    InvalidTrackTypeForTrackMute { track_type: String },
    #[error("invalid track_type `{track_type}` for insert_clip.")]
    InvalidTrackTypeForInsertClip { track_type: String },
    #[error("insert_clip duration_ms must be > 0.")]
    InsertClipDurationMustBePositive,
    #[error("V1 insert_clip only supports track_index 0 or null.")]
    V1InsertClipOnlySupportsTrackIndexZero,
    #[error("insert_clip requires track_index for `{track_type}`.")]
    InsertClipRequiresTrackIndex { track_type: String },
    #[error("insert_clip target track not found ({track_type}[{track_index}]).")]
    InsertClipTargetTrackNotFound {
        track_type: String,
        track_index: usize,
    },
    #[error("insert_clip does not support subtitle tracks. Use generate_subtitles.")]
    InsertClipDoesNotSupportSubtitleTracks,
    #[error("set_source_in_out duration_ms must be > 0.")]
    SetSourceInOutDurationMustBePositive,
    #[error("set_source_in_out source_out_ms must be greater than source_in_ms.")]
    SetSourceInOutSourceOutMustBeGreaterThanSourceIn,
    #[error("invalid track_type `{track_type}` for delete_track_clips.")]
    InvalidTrackTypeForDeleteTrackClips { track_type: String },
    #[error("invalid ripple_delete_range: end_ms ({end_ms}) must be > start_ms ({start_ms}).")]
    InvalidRippleDeleteRange { start_ms: u64, end_ms: u64 },
    #[error(
        "unsupported ripple_delete_range mode `{mode}`. Use anica.timeline/build_subtitle_gap_cut_plan for subtitle_only/subtitle_audio_aligned strategies."
    )]
    UnsupportedRippleDeleteRangeMode { mode: String },
    #[error("trim_clip new_duration_ms must be > 0.")]
    TrimClipDurationMustBePositive,
    #[error("split_clip at_ms must be strictly inside clip range.")]
    SplitClipAtMustBeStrictlyInsideRange,
    #[error("invalid shift_subtitles_range: end_ms ({end_ms}) must be > start_ms ({start_ms}).")]
    InvalidShiftSubtitlesRange { start_ms: u64, end_ms: u64 },
    #[error("generate_subtitles requires non-empty entries.")]
    GenerateSubtitlesRequiresNonEmptyEntries,
    #[error("entries[{entry_index}].duration_ms must be > 0.")]
    GenerateSubtitleEntryDurationMustBePositive { entry_index: usize },
    #[error("batch_update_subtitles requires non-empty updates.")]
    BatchUpdateSubtitlesRequiresNonEmptyUpdates,
    #[error("updates[{update_index}] subtitle clip_id {clip_id} not found.")]
    BatchUpdateSubtitleClipNotFound { update_index: usize, clip_id: u64 },
    #[error("updates[{update_index}].duration_ms must be > 0.")]
    BatchUpdateSubtitleDurationMustBePositive { update_index: usize },
    #[error("updates[{update_index}] target track not found (subtitle[{track_index}]).")]
    BatchUpdateSubtitleTargetTrackNotFound {
        update_index: usize,
        track_index: usize,
    },
    #[error("invalid delete_subtitle_range: end_ms ({end_ms}) must be > start_ms ({start_ms}).")]
    InvalidDeleteSubtitleRange { start_ms: u64, end_ms: u64 },
    #[error("insert_semantic_clip duration_ms must be > 0.")]
    InsertSemanticClipDurationMustBePositive,
    #[error("insert_semantic_clip semantic_type must be non-empty when provided.")]
    InsertSemanticClipSemanticTypeMustBeNonEmpty,
    #[error("insert_semantic_clip prompt_schema must be a JSON object.")]
    InsertSemanticClipPromptSchemaMustBeObject,
    #[error(
        "semantic duration {duration_sec:.2}s exceeds VEO 3.1 limit {max_sec:.2}s. Shorten the semantic clip first."
    )]
    SemanticDurationExceedsVeoLimit { duration_sec: f64, max_sec: f64 },
}
