use serde::{Deserialize, Serialize};

use super::selector::{
    TextSelectionIndex, TextSelectionRange, TextSelectorKind, parse_selector_range,
};
use super::style::{TextEffectNode, TextStyleOverrideNode, TextTransformNode};

fn default_mode() -> String {
    "normal".to_string()
}

fn default_order() -> String {
    "forward".to_string()
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextAnimatorNode {
    pub id: Option<String>,
    pub selector: TextSelectorKind,
    #[serde(default = "default_mode")]
    pub mode: String,
    pub from_ms: i64,
    pub duration_ms: Option<u64>,
    pub stagger_ms: i64,
    #[serde(default = "default_order")]
    pub order: String,
    pub pre_roll_ms: i64,
    pub post_roll_ms: i64,
    #[serde(default)]
    pub active_word: Option<String>,
    #[serde(default)]
    pub random_seed: Option<u64>,
    #[serde(default)]
    pub range: Option<String>,
    #[serde(default)]
    pub transform: Option<TextTransformNode>,
    #[serde(default)]
    pub style: Option<TextStyleOverrideNode>,
    #[serde(default)]
    pub effects: Vec<TextEffectNode>,
}

impl TextAnimatorNode {
    pub fn is_karaoke(&self) -> bool {
        self.mode == "karaoke"
    }

    pub fn target_ranges(&self, selections: &TextSelectionIndex) -> Vec<TextSelectionRange> {
        match self.selector {
            TextSelectorKind::Range => self
                .range
                .as_deref()
                .and_then(|range| parse_selector_range(range, selections.chars.len()))
                .into_iter()
                .collect(),
            selector => selections.ranges_for(selector).to_vec(),
        }
    }

    pub fn target_states(&self, selections: &TextSelectionIndex) -> Vec<TextAnimatorTargetState> {
        let ranges = self.target_ranges(selections);
        let ordered_indices = animator_order_indices(&ranges, &self.order, self.random_seed);
        ordered_indices
            .into_iter()
            .enumerate()
            .map(|(order_index, source_index)| {
                let range = ranges[source_index].clone();
                TextAnimatorTargetState {
                    source_index,
                    order_index,
                    start_char: range.start_char,
                    end_char: range.end_char,
                    start_ms: self.from_ms + self.stagger_ms * order_index as i64,
                    duration_ms: self.duration_ms,
                    pre_roll_ms: self.pre_roll_ms,
                    post_roll_ms: self.post_roll_ms,
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextAnimatorTargetState {
    pub source_index: usize,
    pub order_index: usize,
    pub start_char: usize,
    pub end_char: usize,
    pub start_ms: i64,
    pub duration_ms: Option<u64>,
    pub pre_roll_ms: i64,
    pub post_roll_ms: i64,
}

impl TextAnimatorTargetState {
    pub fn end_ms(&self) -> Option<i64> {
        self.duration_ms
            .map(|duration| self.start_ms + duration as i64)
    }

    pub fn local_time_ms(&self, global_time_ms: i64) -> i64 {
        global_time_ms - self.start_ms
    }
}

fn animator_order_indices(
    ranges: &[TextSelectionRange],
    order: &str,
    random_seed: Option<u64>,
) -> Vec<usize> {
    let mut indices = (0..ranges.len()).collect::<Vec<_>>();
    match order {
        "reverse" => indices.reverse(),
        "random" => {
            let seed = random_seed.unwrap_or(0);
            indices.sort_by_key(|index| deterministic_order_key(seed, *index as u64));
        }
        _ => {}
    }
    indices
}

fn deterministic_order_key(seed: u64, value: u64) -> u64 {
    let mut x = seed ^ value.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

#[cfg(test)]
mod tests {
    use super::TextAnimatorNode;
    use crate::scene::text::{TextSelectorKind, build_text_selection_index, style::TextEffectNode};

    fn animator(selector: TextSelectorKind, order: &str, stagger_ms: i64) -> TextAnimatorNode {
        TextAnimatorNode {
            id: Some("anim".to_string()),
            selector,
            mode: "normal".to_string(),
            from_ms: 120,
            duration_ms: Some(450),
            stagger_ms,
            order: order.to_string(),
            pre_roll_ms: 10,
            post_roll_ms: 20,
            active_word: None,
            random_seed: Some(7),
            range: None,
            transform: None,
            style: None,
            effects: Vec::<TextEffectNode>::new(),
        }
    }

    #[test]
    fn animator_target_states_apply_forward_stagger() {
        let selections = build_text_selection_index("AI edits video");
        let states = animator(TextSelectorKind::Word, "forward", 80).target_states(&selections);

        assert_eq!(states.len(), 3);
        assert_eq!(states[0].source_index, 0);
        assert_eq!(states[0].start_ms, 120);
        assert_eq!(states[1].source_index, 1);
        assert_eq!(states[1].start_ms, 200);
        assert_eq!(states[2].end_ms(), Some(730));
    }

    #[test]
    fn animator_target_states_support_reverse_order() {
        let selections = build_text_selection_index("AI edits video");
        let states = animator(TextSelectorKind::Word, "reverse", 50).target_states(&selections);

        assert_eq!(states[0].source_index, 2);
        assert_eq!(states[0].start_char, 9);
        assert_eq!(states[1].source_index, 1);
        assert_eq!(states[2].source_index, 0);
        assert_eq!(states[2].start_ms, 220);
    }

    #[test]
    fn animator_target_states_support_range_selector() {
        let selections = build_text_selection_index("AI edits video");
        let mut range_animator = animator(TextSelectorKind::Range, "forward", 0);
        range_animator.range = Some("3..8".to_string());
        let states = range_animator.target_states(&selections);

        assert_eq!(states.len(), 1);
        assert_eq!(states[0].start_char, 3);
        assert_eq!(states[0].end_char, 8);
    }
}
