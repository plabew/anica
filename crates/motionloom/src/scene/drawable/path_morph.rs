use std::borrow::Cow;

use crate::scene::render::MotionLoomSceneRenderError;

use super::{PathToken, tokenize_path_data};

#[derive(Debug, Clone)]
struct PathMorphKeyframe {
    time_sec: f32,
    tokens: Vec<PathMorphToken>,
}

#[derive(Debug, Clone, Copy)]
enum PathMorphToken {
    Command(char),
    Number(f32),
}

pub(crate) fn eval_path_d<'a>(
    d: &'a str,
    _time_norm: f32,
    time_sec: f32,
) -> Result<Cow<'a, str>, MotionLoomSceneRenderError> {
    let trimmed = d.trim();
    if !trimmed.starts_with("morph(") {
        return Ok(Cow::Borrowed(d));
    }

    Ok(Cow::Owned(eval_path_morph(trimmed, time_sec)?))
}

fn eval_path_morph(expr: &str, time_sec: f32) -> Result<String, MotionLoomSceneRenderError> {
    let inner = expr
        .strip_prefix("morph(")
        .and_then(|value| value.strip_suffix(')'))
        .ok_or_else(|| invalid_path_morph(expr, "expected morph(\"time:path\", ...)"))?;
    let args = split_path_morph_args(inner, expr)?;
    if args.len() < 2 {
        return Err(invalid_path_morph(
            expr,
            "morph requires at least two keyframes.",
        ));
    }

    let mut keyframes = args
        .iter()
        .map(|arg| parse_path_morph_keyframe(arg, expr))
        .collect::<Result<Vec<_>, _>>()?;
    keyframes.sort_by(|a, b| a.time_sec.total_cmp(&b.time_sec));
    for pair in keyframes.windows(2) {
        if (pair[0].time_sec - pair[1].time_sec).abs() <= f32::EPSILON {
            return Err(invalid_path_morph(
                expr,
                format!(
                    "duplicate morph keyframe time: {}",
                    format_path_morph_number(pair[0].time_sec)
                ),
            ));
        }
    }

    if time_sec <= keyframes[0].time_sec {
        return Ok(path_morph_tokens_to_d(&keyframes[0].tokens));
    }
    let last = keyframes.len() - 1;
    if time_sec >= keyframes[last].time_sec {
        return Ok(path_morph_tokens_to_d(&keyframes[last].tokens));
    }

    for pair in keyframes.windows(2) {
        let from = &pair[0];
        let to = &pair[1];
        if time_sec < from.time_sec || time_sec > to.time_sec {
            continue;
        }
        let t = ((time_sec - from.time_sec) / (to.time_sec - from.time_sec)).clamp(0.0, 1.0);
        return interpolate_path_morph_tokens(&from.tokens, &to.tokens, t, expr);
    }

    Ok(path_morph_tokens_to_d(&keyframes[last].tokens))
}

fn split_path_morph_args(
    inner: &str,
    source: &str,
) -> Result<Vec<String>, MotionLoomSceneRenderError> {
    let mut args = Vec::new();
    let mut start = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut depth = 0usize;

    for (index, ch) in inner.char_indices() {
        if let Some(quote_ch) = quote {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == quote_ch {
                quote = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => quote = Some(ch),
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return Err(invalid_path_morph(
                        source,
                        "unexpected ')' in morph arguments.",
                    ));
                }
                depth -= 1;
            }
            ',' if depth == 0 => {
                let arg = inner[start..index].trim();
                if !arg.is_empty() {
                    args.push(arg.to_string());
                }
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }

    if quote.is_some() {
        return Err(invalid_path_morph(
            source,
            "unterminated string in morph arguments.",
        ));
    }
    if depth != 0 {
        return Err(invalid_path_morph(
            source,
            "unclosed nested expression in morph arguments.",
        ));
    }

    let tail = inner[start..].trim();
    if !tail.is_empty() {
        args.push(tail.to_string());
    }
    Ok(args)
}

fn parse_path_morph_keyframe(
    arg: &str,
    source: &str,
) -> Result<PathMorphKeyframe, MotionLoomSceneRenderError> {
    let arg = unquote_path_morph_arg(arg, source)?;
    let Some((time_raw, path_raw)) = arg.split_once(':') else {
        return Err(invalid_path_morph(
            source,
            "each morph keyframe must be \"seconds:path data\".",
        ));
    };
    let time_sec = time_raw
        .trim()
        .parse::<f32>()
        .map_err(|_| invalid_path_morph(source, format!("invalid keyframe time: {time_raw}")))?;
    if !time_sec.is_finite() {
        return Err(invalid_path_morph(
            source,
            format!("invalid keyframe time: {time_raw}"),
        ));
    }
    let path = path_raw.trim();
    if path.is_empty() {
        return Err(invalid_path_morph(
            source,
            "morph keyframe path data is empty.",
        ));
    }

    let tokens = tokenize_path_data(path)?
        .into_iter()
        .map(|token| match token {
            PathToken::Command(command) => PathMorphToken::Command(command),
            PathToken::Number(value) => PathMorphToken::Number(value),
        })
        .collect();

    Ok(PathMorphKeyframe { time_sec, tokens })
}

fn unquote_path_morph_arg(arg: &str, source: &str) -> Result<String, MotionLoomSceneRenderError> {
    let arg = arg.trim();
    let Some(first) = arg.chars().next() else {
        return Err(invalid_path_morph(source, "empty morph keyframe."));
    };
    if first != '"' && first != '\'' {
        return Ok(arg.to_string());
    }
    if !arg.ends_with(first) || arg.len() < 2 {
        return Err(invalid_path_morph(
            source,
            "unterminated morph keyframe string.",
        ));
    }

    let body = &arg[first.len_utf8()..arg.len() - first.len_utf8()];
    let mut out = String::new();
    let mut escaped = false;
    for ch in body.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        out.push(ch);
    }
    if escaped {
        out.push('\\');
    }
    Ok(out)
}

fn interpolate_path_morph_tokens(
    from: &[PathMorphToken],
    to: &[PathMorphToken],
    t: f32,
    source: &str,
) -> Result<String, MotionLoomSceneRenderError> {
    if from.len() != to.len() {
        return Err(invalid_path_morph(
            source,
            "incompatible path data: keyframes have different token counts.",
        ));
    }

    let mut out = Vec::with_capacity(from.len());
    for (from_token, to_token) in from.iter().zip(to.iter()) {
        match (*from_token, *to_token) {
            (PathMorphToken::Command(a), PathMorphToken::Command(b)) if a == b => {
                out.push(PathMorphToken::Command(a));
            }
            (PathMorphToken::Number(a), PathMorphToken::Number(b)) => {
                out.push(PathMorphToken::Number(a + (b - a) * t));
            }
            (PathMorphToken::Command(a), PathMorphToken::Command(b)) => {
                return Err(invalid_path_morph(
                    source,
                    format!("incompatible path data: command '{a}' does not match '{b}'."),
                ));
            }
            _ => {
                return Err(invalid_path_morph(
                    source,
                    "incompatible path data: command/number layout differs.",
                ));
            }
        }
    }

    Ok(path_morph_tokens_to_d(&out))
}

fn path_morph_tokens_to_d(tokens: &[PathMorphToken]) -> String {
    let mut out = String::new();
    for token in tokens {
        if !out.is_empty() {
            out.push(' ');
        }
        match *token {
            PathMorphToken::Command(command) => out.push(command),
            PathMorphToken::Number(value) => out.push_str(&format_path_morph_number(value)),
        }
    }
    out
}

fn format_path_morph_number(value: f32) -> String {
    let mut text = format!("{value:.3}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" { "0".to_string() } else { text }
}

fn invalid_path_morph(source: &str, message: impl Into<String>) -> MotionLoomSceneRenderError {
    MotionLoomSceneRenderError::InvalidPathData {
        value: source.to_string(),
        message: message.into(),
    }
}
