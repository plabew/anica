use crate::dsl::{
    GraphParseError, GraphScript, graph_root_start, is_graph_script, parse_graph_script,
    validate_graph_present_placement,
};

pub type ProcessGraph = GraphScript;

pub fn is_process_graph_script(script: &str) -> bool {
    graph_root_start(script).is_ok() && script.contains("<Process") && is_graph_script(script)
}

pub fn parse_process_graph_script(script: &str) -> Result<ProcessGraph, GraphParseError> {
    if script.contains("<Process") {
        return parse_process_block_graph_script(script);
    }

    Err(GraphParseError {
        line: 1,
        message: "Process graphs must wrap process nodes in <Process id=\"...\">...</Process>."
            .to_string(),
    })
}

fn parse_process_block_graph_script(script: &str) -> Result<ProcessGraph, GraphParseError> {
    let normalized = script.replace('＝', "=");
    validate_graph_present_placement(&normalized)?;
    let graph_start = find_open_tag(&normalized, "Graph", 0).ok_or_else(|| GraphParseError {
        line: 1,
        message: "Missing <Graph ...> root tag.".to_string(),
    })?;
    let graph_open_end = find_tag_end(&normalized, graph_start).ok_or_else(|| GraphParseError {
        line: line_of_byte(&normalized, graph_start),
        message: "Unclosed <Graph ...> opening tag.".to_string(),
    })?;
    let graph_close = normalized[graph_open_end + 1..]
        .find("</Graph>")
        .map(|offset| graph_open_end + 1 + offset)
        .ok_or_else(|| GraphParseError {
            line: line_of_byte(&normalized, graph_start),
            message: "Missing </Graph> close tag.".to_string(),
        })?;

    let process_start =
        find_open_tag(&normalized, "Process", graph_open_end + 1).ok_or_else(|| {
            GraphParseError {
                line: line_of_byte(&normalized, graph_open_end),
                message: "Missing <Process ...> block.".to_string(),
            }
        })?;
    if process_start >= graph_close {
        return Err(GraphParseError {
            line: line_of_byte(&normalized, process_start),
            message: "<Process> must be inside <Graph>.".to_string(),
        });
    }

    let process_open_end =
        find_tag_end(&normalized, process_start).ok_or_else(|| GraphParseError {
            line: line_of_byte(&normalized, process_start),
            message: "Unclosed <Process ...> opening tag.".to_string(),
        })?;
    let process_open = &normalized[process_start..=process_open_end];
    if is_self_closing_tag(process_open) {
        return Err(GraphParseError {
            line: line_of_byte(&normalized, process_start),
            message: "<Process> must contain process nodes.".to_string(),
        });
    }

    let process_close = normalized[process_open_end + 1..]
        .find("</Process>")
        .map(|offset| process_open_end + 1 + offset)
        .ok_or_else(|| GraphParseError {
            line: line_of_byte(&normalized, process_start),
            message: "Missing </Process> close tag.".to_string(),
        })?;
    if process_close > graph_close {
        return Err(GraphParseError {
            line: line_of_byte(&normalized, process_start),
            message: "<Process> must close before </Graph>.".to_string(),
        });
    }
    if let Some(second_process) = find_open_tag(&normalized, "Process", process_close + 10)
        && second_process < graph_close
    {
        return Err(GraphParseError {
            line: line_of_byte(&normalized, second_process),
            message: "parse_process_graph_script supports one <Process> block. Use the root dispatcher for mixed or multi-process graphs.".to_string(),
        });
    }

    let process_id = attr_value(process_open, "id")
        .map(|raw| strip_wrappers(&raw).to_string())
        .ok_or_else(|| GraphParseError {
            line: line_of_byte(&normalized, process_start),
            message: "Missing required attribute: id".to_string(),
        })?;
    let present_start = find_open_tag(&normalized, "Present", process_close + "</Process>".len())
        .ok_or_else(|| GraphParseError {
        line: line_of_byte(&normalized, graph_start),
        message: "Missing <Present from=\"...\" /> node.".to_string(),
    })?;
    if present_start >= graph_close {
        return Err(GraphParseError {
            line: line_of_byte(&normalized, graph_start),
            message: "Missing <Present from=\"...\" /> node.".to_string(),
        });
    }
    let present_end = find_tag_end(&normalized, present_start).ok_or_else(|| GraphParseError {
        line: line_of_byte(&normalized, present_start),
        message: "Unclosed <Present ... /> tag.".to_string(),
    })?;
    let present_tag = &normalized[present_start..=present_end];
    let present_from = attr_value(present_tag, "from")
        .map(|raw| strip_wrappers(&raw).to_string())
        .ok_or_else(|| GraphParseError {
            line: line_of_byte(&normalized, present_start),
            message: "Missing required attribute: from".to_string(),
        })?;
    if present_from != process_id {
        return Err(GraphParseError {
            line: line_of_byte(&normalized, present_start),
            message: format!(
                "Root <Present> for a process graph must reference the <Process> id \"{process_id}\", got \"{present_from}\"."
            ),
        });
    }

    let graph_open = &normalized[graph_start..=graph_open_end];
    let process_body = &normalized[process_open_end + 1..process_close];
    let present_resource = infer_process_present_resource(
        process_open,
        process_body,
        line_of_byte(&normalized, process_start),
    )?;
    let synthetic =
        format!("{graph_open}\n{process_body}\n<Present from=\"{present_resource}\" />\n</Graph>");
    let mut graph = parse_graph_script(&synthetic)?;
    graph.id = Some(process_id);
    graph.raw_script = Some(script.to_string());
    Ok(graph)
}

fn infer_process_present_resource(
    process_open: &str,
    process_body: &str,
    line: usize,
) -> Result<String, GraphParseError> {
    if let Some(raw) =
        attr_value(process_open, "output").or_else(|| attr_value(process_open, "present"))
    {
        let id = strip_wrappers(&raw).to_string();
        if !id.is_empty() {
            return Ok(id);
        }
    }
    if let Some(id) = last_self_closing_attr(process_body, "Output", "id")? {
        return Ok(id);
    }
    if let Some(out) = last_pass_output(process_body)? {
        return Ok(out);
    }
    if let Some(id) = last_self_closing_attr(process_body, "Tex", "id")? {
        return Ok(id);
    }
    Err(GraphParseError {
        line,
        message: "<Process> must declare output=\"...\" or contain an <Output>, <Pass out={...}>, or <Tex> that can be presented.".to_string(),
    })
}

fn last_self_closing_attr(
    input: &str,
    tag_name: &str,
    attr: &str,
) -> Result<Option<String>, GraphParseError> {
    let mut cursor = 0usize;
    let mut last = None;
    while let Some(start) = find_open_tag(input, tag_name, cursor) {
        let tag_end = find_tag_end(input, start).ok_or_else(|| GraphParseError {
            line: line_of_byte(input, start),
            message: format!("Unclosed <{tag_name} ... /> tag."),
        })?;
        let tag = &input[start..=tag_end];
        if let Some(raw) = attr_value(tag, attr) {
            let id = strip_wrappers(&raw).to_string();
            if !id.is_empty() {
                last = Some(id);
            }
        }
        cursor = tag_end + 1;
    }
    Ok(last)
}

fn last_pass_output(input: &str) -> Result<Option<String>, GraphParseError> {
    let mut cursor = 0usize;
    let mut last = None;
    while let Some(start) = find_open_tag(input, "Pass", cursor) {
        let tag_end = find_tag_end(input, start).ok_or_else(|| GraphParseError {
            line: line_of_byte(input, start),
            message: "Unclosed <Pass ... /> tag.".to_string(),
        })?;
        let tag = &input[start..=tag_end];
        if let Some(raw) = attr_value(tag, "out")
            && let Some(id) = last_resource_id(&raw)
        {
            last = Some(id);
        }
        cursor = tag_end + 1;
    }
    Ok(last)
}

fn last_resource_id(raw: &str) -> Option<String> {
    let text = strip_wrappers(raw).trim();
    let mut quoted = Vec::<String>::new();
    let mut in_quote: Option<char> = None;
    let mut current = String::new();
    for ch in text.chars() {
        if let Some(quote) = in_quote {
            if ch == quote {
                if !current.trim().is_empty() {
                    quoted.push(current.trim().to_string());
                }
                current.clear();
                in_quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            in_quote = Some(ch);
        }
    }
    if let Some(id) = quoted
        .into_iter()
        .rev()
        .find(|value| value != "tex" && value != "buf" && value != "id")
    {
        return Some(id);
    }

    text.trim_matches(|ch| matches!(ch, '[' | ']' | '{' | '}' | '"' | '\'' | ' '))
        .split(',')
        .filter_map(|part| {
            let token = part
                .trim()
                .trim_start_matches("tex:")
                .trim_start_matches("buf:")
                .trim_matches(|ch| matches!(ch, '"' | '\'' | ' '));
            (!token.is_empty()).then(|| token.to_string())
        })
        .last()
}

fn find_open_tag(input: &str, tag_name: &str, start: usize) -> Option<usize> {
    let pattern = format!("<{tag_name}");
    let mut cursor = start.min(input.len());
    while let Some(offset) = input[cursor..].find(&pattern) {
        let ix = cursor + offset;
        let next_ix = ix + pattern.len();
        let next = input[next_ix..].chars().next();
        if matches!(next, Some(ch) if ch.is_whitespace() || ch == '>' || ch == '/') {
            return Some(ix);
        }
        cursor = next_ix;
    }
    None
}

fn find_tag_end(input: &str, start: usize) -> Option<usize> {
    let mut in_quote = false;
    let mut brace_depth = 0usize;
    for (offset, ch) in input[start..].char_indices() {
        match ch {
            '"' if brace_depth == 0 => in_quote = !in_quote,
            '{' if !in_quote => brace_depth += 1,
            '}' if !in_quote => brace_depth = brace_depth.saturating_sub(1),
            '>' if !in_quote && brace_depth == 0 => return Some(start + offset),
            _ => {}
        }
    }
    None
}

fn is_self_closing_tag(block: &str) -> bool {
    block.trim_end().ends_with("/>")
}

fn attr_value(block: &str, key: &str) -> Option<String> {
    let start = find_attr_start(block, key)?;
    let mut rest = block[start..].trim_start();
    if !rest.starts_with('=') {
        return None;
    }
    rest = rest[1..].trim_start();
    if let Some(stripped) = rest.strip_prefix('"') {
        let end = stripped.find('"')?;
        return Some(stripped[..end].to_string());
    }
    if let Some(stripped) = rest.strip_prefix('{') {
        let mut depth = 1usize;
        let mut out = String::new();
        for ch in stripped.chars() {
            if ch == '{' {
                depth += 1;
                out.push(ch);
                continue;
            }
            if ch == '}' {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(out);
                }
                out.push(ch);
                continue;
            }
            out.push(ch);
        }
        return None;
    }
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

fn find_attr_start(block: &str, key: &str) -> Option<usize> {
    let bytes = block.as_bytes();
    let key_bytes = key.as_bytes();
    if key_bytes.is_empty() || bytes.len() < key_bytes.len() + 1 {
        return None;
    }
    let mut in_double_quote = false;
    let mut i = 0usize;
    while i + key_bytes.len() < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            in_double_quote = !in_double_quote;
            i += 1;
            continue;
        }
        if in_double_quote {
            i += 1;
            continue;
        }
        if &bytes[i..i + key_bytes.len()] == key_bytes {
            let prev_ok = i == 0
                || bytes[i - 1].is_ascii_whitespace()
                || bytes[i - 1] == b'<'
                || bytes[i - 1] == b'\n';
            let mut j = i + key_bytes.len();
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if prev_ok && j < bytes.len() && bytes[j] == b'=' {
                return Some(i + key_bytes.len());
            }
        }
        i += 1;
    }
    None
}

fn strip_wrappers(raw: &str) -> &str {
    let mut text = raw.trim();
    loop {
        if text.starts_with('{') && text.ends_with('}') && text.len() >= 2 {
            text = text[1..text.len() - 1].trim();
            continue;
        }
        if text.starts_with('"') && text.ends_with('"') && text.len() >= 2 {
            text = text[1..text.len() - 1].trim();
            continue;
        }
        break;
    }
    text
}

fn line_of_byte(input: &str, byte_ix: usize) -> usize {
    input[..byte_ix.min(input.len())]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1
}

#[cfg(test)]
mod tests {
    use super::{is_process_graph_script, parse_process_graph_script};

    #[test]
    fn parses_process_block_graph() {
        let script = r#"
<Graph fps={60} duration="8s" size={[1920,1080]}>
  <Process id="final_grade">
    <Input id="clip0" type="video" from="input:clip0" />
    <Tex id="src" fmt="rgba16f" from="clip0" />
    <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
    <Pass id="fx" kind="compute" effect="hsla_overlay"
          in={["src"]} out={["out"]}
          params={{ hue: "210", saturation: "0.7", lightness: "0.41", alpha: "0.45" }} />
  </Process>
  <Present from="final_grade" />
</Graph>
"#;
        assert!(is_process_graph_script(script));
        let graph = parse_process_graph_script(script).expect("process block should parse");
        assert_eq!(graph.id.as_deref(), Some("final_grade"));
        assert_eq!(graph.inputs.len(), 1);
        assert_eq!(graph.textures.len(), 2);
        assert_eq!(graph.passes.len(), 1);
        assert_eq!(graph.present.from, "out");
        assert_eq!(graph.duration_ms, 8000);
    }

    #[test]
    fn rejects_root_level_process_shorthand() {
        let script = r#"
<Graph fps={60} size={[1920,1080]}>
  <Input id="clip0" type="video" from="input:clip0" />
  <Tex id="src" fmt="rgba16f" from="clip0" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="fx" kind="compute" effect="gaussian_5tap_h"
        in={["src"]} out={["out"]} params={{ sigma: "10" }} />
  <Present from="out" />
</Graph>
"#;
        assert!(!is_process_graph_script(script));
        let err = parse_process_graph_script(script).expect_err("legacy shorthand should fail");
        assert!(err.message.contains("<Process"));
    }

    #[test]
    fn rejects_multiple_process_blocks_for_direct_process_parse() {
        let script = r#"
<Graph fps={60} size={[1920,1080]}>
  <Process id="a">
    <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
    <Pass id="fx_a" kind="compute" effect="gaussian_5tap_h"
          in={["out"]} out={["out"]} params={{ sigma: "1" }} />
  </Process>
  <Process id="b">
    <Tex id="out_b" fmt="rgba16f" size={[1920,1080]} />
    <Pass id="fx_b" kind="compute" effect="gaussian_5tap_h"
          in={["out_b"]} out={["out_b"]} params={{ sigma: "1" }} />
  </Process>
  <Present from="b" />
</Graph>
"#;
        let err = parse_process_graph_script(script).expect_err("multiple process blocks fail");
        assert!(err.message.contains("one <Process> block"));
    }

    #[test]
    fn rejects_present_inside_process_block() {
        let script = r#"
<Graph fps={60} size={[1920,1080]}>
  <Process id="final_grade">
    <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
    <Present from="out" />
  </Process>
  <Present from="final_grade" />
</Graph>
"#;
        let err = parse_process_graph_script(script).expect_err("nested Present should fail");
        assert!(err.message.contains("direct child of <Graph>"));
    }
}
