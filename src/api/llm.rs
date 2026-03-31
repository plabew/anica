use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmSrtSimilarClip {
    pub clip_id: u64,
    pub track_index: usize,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmDecisionMakingSrtSimilarSerachRequest {
    #[serde(default)]
    pub user_goal: String,
    #[serde(default)]
    pub first_pass_rule_analysis: Value,
    #[serde(default)]
    pub subtitle_rows: Vec<LlmSrtSimilarClip>,
    #[serde(default = "default_second_check_min_range_ms")]
    pub min_range_ms: u64,
    #[serde(default = "default_second_check_max_rows")]
    pub max_rows: usize,
    #[serde(default)]
    pub llm_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmDecisionMakingSrtSimilarSerachResponse {
    pub analysis_source: String,
    pub prompt: String,
    pub expected_json_schema: Value,
    pub subtitle_row_count: usize,
    pub warnings: Vec<String>,
}

fn default_second_check_min_range_ms() -> u64 {
    400
}

fn default_second_check_max_rows() -> usize {
    420
}

fn compact_first_pass_for_prompt(value: &Value) -> Value {
    let cut_candidates = value
        .get("cut_candidates")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().take(128).cloned().collect::<Vec<_>>())
        .unwrap_or_default();

    let repeat_groups = value
        .get("repeat_groups")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .take(64)
                .map(|group| {
                    json!({
                        "group_id": group.get("group_id").cloned().unwrap_or(Value::Null),
                        "keep_clip_id": group.get("keep_clip_id").cloned().unwrap_or(Value::Null),
                        "confidence": group.get("confidence").cloned().unwrap_or(Value::Null),
                        "members": group.get("members").cloned().unwrap_or(Value::Array(Vec::new())),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    json!({
        "analysis_source": value.get("analysis_source").cloned().unwrap_or(Value::Null),
        "window_ms": value.get("window_ms").cloned().unwrap_or(Value::Null),
        "similarity_threshold": value.get("similarity_threshold").cloned().unwrap_or(Value::Null),
        "cut_candidates": cut_candidates,
        "repeat_groups": repeat_groups,
        "warnings": value.get("warnings").cloned().unwrap_or(Value::Array(Vec::new())),
    })
}

pub fn llm_decision_making_srt_similar_serach(
    mut request: LlmDecisionMakingSrtSimilarSerachRequest,
) -> LlmDecisionMakingSrtSimilarSerachResponse {
    let mut warnings: Vec<String> = Vec::new();
    let min_range_ms = request.min_range_ms.max(200);

    if request.max_rows == 0 {
        request.max_rows = default_second_check_max_rows();
    }

    if request.user_goal.trim().is_empty() {
        request.user_goal =
            "Find conservative missed repeated-speech cuts after first pass.".to_string();
        warnings.push("empty_user_goal_filled_with_default".to_string());
    }

    let mut subtitle_rows = request
        .subtitle_rows
        .into_iter()
        .filter(|row| row.end_ms > row.start_ms && !row.text.trim().is_empty())
        .collect::<Vec<_>>();
    subtitle_rows.sort_by_key(|row| (row.start_ms, row.end_ms, row.clip_id));

    if subtitle_rows.len() > request.max_rows {
        subtitle_rows.truncate(request.max_rows);
        warnings.push(format!(
            "subtitle_rows_truncated_to_max_rows_{}",
            request.max_rows
        ));
    }
    if subtitle_rows.len() < 2 {
        warnings.push("subtitle_rows_too_few_for_meaningful_second_check".to_string());
    }

    let first_pass_compact = compact_first_pass_for_prompt(&request.first_pass_rule_analysis);
    let first_pass_json =
        serde_json::to_string_pretty(&first_pass_compact).unwrap_or_else(|_| "{}".to_string());
    let subtitle_rows_json =
        serde_json::to_string_pretty(&subtitle_rows).unwrap_or_else(|_| "[]".to_string());
    let first_pass_section = if request.llm_only {
        "First-pass rule analysis JSON:\n\
        {\"analysis_source\":\"llm_only\",\"notes\":[\"Do not use rule-based prefilters for final decisions.\"]}\n"
            .to_string()
    } else {
        format!("First-round rule analysis JSON:\n{first_pass_json}\n")
    };

    let expected_json_schema = json!({
        "category_assessments": [
            {
                "category": "exact_repeat",
                "decision": "has_candidate|no_candidate",
                "reason": "...",
                "candidate_count": 0
            },
            {
                "category": "same_topic_consecutive_restart",
                "decision": "has_candidate|no_candidate",
                "reason": "...",
                "candidate_count": 0
            },
            {
                "category": "near_synonym_semantic_repeat",
                "decision": "has_candidate|no_candidate",
                "reason": "...",
                "candidate_count": 0
            },
            {
                "category": "prefix_or_continuation_restart",
                "decision": "has_candidate|no_candidate",
                "reason": "...",
                "candidate_count": 0
            }
        ],
        "missed_cut_ranges": [
            {
                "start_ms": 0,
                "end_ms": 0,
                "category": "exact_repeat",
                "confidence": 0.0,
                "reason": "short explanation",
                "source_clip_ids": [1, 2]
            }
        ],
        "notes": ["optional notes"]
    });

    let mode_line = if request.llm_only {
        "Do a pure LLM judgment for repeated speech from subtitle rows only.\n\
        Do NOT use deterministic similarity formulas or rule-engine thresholds as decision authority.\n"
    } else {
        "First round was rule-based. Now do a conservative LLM second-check for missed repeated speech.\n"
    };
    let prompt = format!(
        "You are an editing second-check analyzer.\n\
        {mode_line}\
        Analyze repeated lines by these categories only:\n\
        - exact_repeat\n\
        - same_topic_consecutive_restart\n\
        - near_synonym_semantic_repeat\n\
        - prefix_or_continuation_restart\n\
        \n\
        Requirements:\n\
        - HARD RULE: you must analyze ALL FOUR categories above. Do not skip any category.\n\
        - HARD RULE: return a `category_assessments` array containing exactly these 4 category keys.\n\
        - Keep the final/best phrasing, prefer deleting earlier duplicate attempts.\n\
        - Decide cuts from current subtitle clips.\n\
        - Ignore ranges shorter than {min_range_ms}ms.\n\
        - Be conservative; avoid over-cutting emphasis that is likely intentional.\n\
        - Return STRICT JSON only.\n\
        \n\
        JSON schema:\n\
        {expected_json_schema}\n\
        \n\
        User request:\n\
        {user_goal}\n\
        \n\
        {first_pass_section}\
        \n\
        Subtitle clips JSON:\n\
        {subtitle_rows_json}\n",
        mode_line = mode_line,
        first_pass_section = first_pass_section,
        expected_json_schema = expected_json_schema,
        user_goal = request.user_goal,
    );

    LlmDecisionMakingSrtSimilarSerachResponse {
        analysis_source: "llm_prompt_builder".to_string(),
        prompt,
        expected_json_schema,
        subtitle_row_count: subtitle_rows.len(),
        warnings,
    }
}
