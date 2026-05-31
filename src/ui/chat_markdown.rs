#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatMarkdownSegment {
    Markdown(String),
    Code { language: String, code: String },
}

pub fn split_markdown_code_blocks(text: &str) -> Vec<ChatMarkdownSegment> {
    let mut segments = Vec::new();
    let mut markdown_lines = Vec::<String>::new();
    let mut code_lines = Vec::<String>::new();
    let mut code_language = String::new();
    let mut in_code = false;

    for line in text.lines() {
        let trimmed = line.trim_start();
        if !in_code && trimmed.starts_with("```") {
            flush_markdown_segment(&mut segments, &mut markdown_lines);
            code_language = trimmed.trim_start_matches("```").trim().to_string();
            code_lines.clear();
            in_code = true;
            continue;
        }

        if in_code && trimmed.starts_with("```") {
            segments.push(ChatMarkdownSegment::Code {
                language: code_language.clone(),
                code: code_lines.join("\n"),
            });
            code_lines.clear();
            code_language.clear();
            in_code = false;
            continue;
        }

        if in_code {
            code_lines.push(line.to_string());
        } else {
            markdown_lines.push(line.to_string());
        }
    }

    if in_code {
        segments.push(ChatMarkdownSegment::Code {
            language: code_language,
            code: code_lines.join("\n"),
        });
    } else {
        flush_markdown_segment(&mut segments, &mut markdown_lines);
    }

    if segments.is_empty() {
        segments.push(ChatMarkdownSegment::Markdown(text.to_string()));
    }

    segments
}

fn flush_markdown_segment(
    segments: &mut Vec<ChatMarkdownSegment>,
    markdown_lines: &mut Vec<String>,
) {
    if markdown_lines.is_empty() {
        return;
    }
    let markdown = markdown_lines.join("\n");
    if !markdown.trim().is_empty() {
        segments.push(ChatMarkdownSegment::Markdown(markdown));
    }
    markdown_lines.clear();
}

#[cfg(test)]
mod tests {
    use super::{ChatMarkdownSegment, split_markdown_code_blocks};

    #[test]
    fn splits_fenced_code_blocks_for_copy() {
        let segments = split_markdown_code_blocks("Intro\n```code\nA\nB\n```\nOutro");
        assert_eq!(
            segments,
            vec![
                ChatMarkdownSegment::Markdown("Intro".to_string()),
                ChatMarkdownSegment::Code {
                    language: "code".to_string(),
                    code: "A\nB".to_string(),
                },
                ChatMarkdownSegment::Markdown("Outro".to_string()),
            ]
        );
    }

    #[test]
    fn keeps_unclosed_code_block_as_code_for_streaming() {
        let segments = split_markdown_code_blocks("```motionloom\n<Graph>");
        assert_eq!(
            segments,
            vec![ChatMarkdownSegment::Code {
                language: "motionloom".to_string(),
                code: "<Graph>".to_string(),
            }]
        );
    }
}
