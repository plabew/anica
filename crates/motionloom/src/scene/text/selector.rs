use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextSelectorKind {
    Char,
    Word,
    Line,
    Range,
}

impl TextSelectorKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "char" | "chars" | "character" | "characters" => Some(Self::Char),
            "word" | "words" => Some(Self::Word),
            "line" | "lines" => Some(Self::Line),
            "range" => Some(Self::Range),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSelectionRange {
    pub index: usize,
    pub start_char: usize,
    pub end_char: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSelectionIndex {
    pub chars: Vec<TextSelectionRange>,
    pub words: Vec<TextSelectionRange>,
    pub lines: Vec<TextSelectionRange>,
}

pub fn build_text_selection_index(value: &str) -> TextSelectionIndex {
    build_text_selection_index_with_lines(value, explicit_line_ranges(value))
}

pub fn build_text_selection_index_with_lines(
    value: &str,
    line_ranges: Vec<(usize, usize)>,
) -> TextSelectionIndex {
    let chars = value
        .chars()
        .enumerate()
        .map(|(index, _)| TextSelectionRange {
            index,
            start_char: index,
            end_char: index + 1,
        })
        .collect::<Vec<_>>();

    let mut words = Vec::new();
    let mut active_start: Option<usize> = None;
    let mut word_index = 0usize;
    let mut char_len = 0usize;
    for (char_ix, ch) in value.chars().enumerate() {
        char_len = char_ix + 1;
        if ch.is_whitespace() {
            if let Some(start) = active_start.take()
                && start < char_ix
            {
                words.push(TextSelectionRange {
                    index: word_index,
                    start_char: start,
                    end_char: char_ix,
                });
                word_index += 1;
            }
        } else if active_start.is_none() {
            active_start = Some(char_ix);
        }
    }
    if let Some(start) = active_start
        && start < char_len
    {
        words.push(TextSelectionRange {
            index: word_index,
            start_char: start,
            end_char: char_len,
        });
    }

    let char_count = value.chars().count();
    let lines = line_ranges
        .into_iter()
        .enumerate()
        .filter_map(|(index, (start_char, end_char))| {
            let start_char = start_char.min(char_count);
            let end_char = end_char.min(char_count);
            (start_char < end_char).then_some(TextSelectionRange {
                index,
                start_char,
                end_char,
            })
        })
        .collect::<Vec<_>>();

    TextSelectionIndex {
        chars,
        words,
        lines,
    }
}

impl TextSelectionIndex {
    pub fn ranges_for(&self, selector: TextSelectorKind) -> &[TextSelectionRange] {
        match selector {
            TextSelectorKind::Char => &self.chars,
            TextSelectorKind::Word => &self.words,
            TextSelectorKind::Line => &self.lines,
            TextSelectorKind::Range => &[],
        }
    }
}

pub fn explicit_line_ranges(value: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0usize;
    let mut char_ix = 0usize;
    for ch in value.chars() {
        if ch == '\n' {
            if start < char_ix {
                ranges.push((start, char_ix));
            }
            start = char_ix + 1;
        }
        char_ix += 1;
    }
    if start < char_ix {
        ranges.push((start, char_ix));
    }
    if ranges.is_empty() && !value.is_empty() {
        ranges.push((0, value.chars().count()));
    }
    ranges
}

pub fn parse_selector_range(value: &str, char_count: usize) -> Option<TextSelectionRange> {
    let trimmed = value.trim();
    let (start, end) = trimmed
        .split_once("..")
        .or_else(|| trimmed.split_once(':'))
        .or_else(|| trimmed.split_once(','))?;
    let start_char = start.trim().parse::<usize>().ok()?.min(char_count);
    let end_char = end.trim().parse::<usize>().ok()?.min(char_count);
    (start_char < end_char).then_some(TextSelectionRange {
        index: 0,
        start_char,
        end_char,
    })
}

#[cfg(test)]
mod tests {
    use super::{TextSelectorKind, build_text_selection_index, parse_selector_range};

    #[test]
    fn text_selection_index_builds_char_word_and_line_ranges() {
        let index = build_text_selection_index("AI edits\nyour video");

        assert_eq!(index.chars.len(), 19);
        assert_eq!(index.words.len(), 4);
        assert_eq!(index.words[0].start_char, 0);
        assert_eq!(index.words[0].end_char, 2);
        assert_eq!(index.words[3].start_char, 14);
        assert_eq!(index.words[3].end_char, 19);
        assert_eq!(index.lines.len(), 2);
        assert_eq!(index.lines[0].start_char, 0);
        assert_eq!(index.lines[0].end_char, 8);
        assert_eq!(index.lines[1].start_char, 9);
        assert_eq!(index.lines[1].end_char, 19);
        assert_eq!(index.ranges_for(TextSelectorKind::Word).len(), 4);
    }

    #[test]
    fn selector_range_parser_clamps_to_text_length() {
        let range = parse_selector_range("2..99", 8).expect("range");
        assert_eq!(range.start_char, 2);
        assert_eq!(range.end_char, 8);
    }
}
