use std::borrow::Cow;

use crate::scene::render::MotionLoomSceneRenderError;

use super::{PathToken, Point2, parse_path_subpaths, point_distance, tokenize_path_data};

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
    if path_morph_tokens_are_compatible(from, to) {
        return interpolate_compatible_path_morph_tokens(from, to, t, source);
    }

    interpolate_normalized_path_morph(from, to, t, source)
}

fn path_morph_tokens_are_compatible(from: &[PathMorphToken], to: &[PathMorphToken]) -> bool {
    from.len() == to.len()
        && from.iter().zip(to).all(|(a, b)| match (*a, *b) {
            (PathMorphToken::Command(a), PathMorphToken::Command(b)) => a == b,
            (PathMorphToken::Number(_), PathMorphToken::Number(_)) => true,
            _ => false,
        })
}

fn interpolate_compatible_path_morph_tokens(
    from: &[PathMorphToken],
    to: &[PathMorphToken],
    t: f32,
    source: &str,
) -> Result<String, MotionLoomSceneRenderError> {
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

fn interpolate_normalized_path_morph(
    from: &[PathMorphToken],
    to: &[PathMorphToken],
    t: f32,
    source: &str,
) -> Result<String, MotionLoomSceneRenderError> {
    let from_d = path_morph_tokens_to_d(from);
    let to_d = path_morph_tokens_to_d(to);
    let from_subpaths = parse_path_subpaths(&from_d)?;
    let to_subpaths = parse_path_subpaths(&to_d)?;
    if from_subpaths.len() != to_subpaths.len() {
        return Err(invalid_path_morph(
            source,
            format!(
                "incompatible path topology: keyframes contain {} and {} subpaths; add matching subpaths before morphing.",
                from_subpaths.len(),
                to_subpaths.len()
            ),
        ));
    }

    let mut output = Vec::with_capacity(from_subpaths.len());
    for (from_path, to_path) in from_subpaths.iter().zip(&to_subpaths) {
        let from_closed = subpath_is_closed(from_path);
        let to_closed = subpath_is_closed(to_path);
        if from_closed != to_closed {
            return Err(invalid_path_morph(
                source,
                "incompatible path topology: cannot morph an open subpath into a closed subpath.",
            ));
        }

        let sample_count = morph_sample_count(from_path, to_path, from_closed);
        let from_samples = resample_subpath(from_path, sample_count, from_closed);
        let mut to_samples = resample_subpath(to_path, sample_count, to_closed);
        if from_closed {
            match_subpath_winding(&from_samples, &mut to_samples);
            align_closed_subpath_start(&from_samples, &mut to_samples);
        }
        output.push(
            from_samples
                .iter()
                .zip(&to_samples)
                .map(|(a, b)| a.lerp(*b, t))
                .collect::<Vec<_>>(),
        );
    }

    Ok(sampled_subpaths_to_d(&output, &from_subpaths))
}

fn subpath_is_closed(points: &[Point2]) -> bool {
    points.len() > 2 && point_distance(points[0], points[points.len() - 1]) <= 0.001
}

fn morph_sample_count(from: &[Point2], to: &[Point2], closed: bool) -> usize {
    let from_len = from.len().saturating_sub(usize::from(closed));
    let to_len = to.len().saturating_sub(usize::from(closed));
    from_len
        .max(to_len)
        .max(if closed { 24 } else { 2 })
        .min(512)
}

fn resample_subpath(points: &[Point2], count: usize, closed: bool) -> Vec<Point2> {
    let usable = if closed && points.len() > 1 {
        &points[..points.len() - 1]
    } else {
        points
    };
    if usable.len() <= 1 {
        return vec![usable.first().copied().unwrap_or(Point2::new(0.0, 0.0)); count];
    }

    let segment_count = if closed {
        usable.len()
    } else {
        usable.len() - 1
    };
    let mut lengths = Vec::with_capacity(segment_count + 1);
    lengths.push(0.0);
    for index in 0..segment_count {
        let next = if index + 1 == usable.len() {
            0
        } else {
            index + 1
        };
        let value = lengths[index] + point_distance(usable[index], usable[next]);
        lengths.push(value);
    }
    let total = *lengths.last().unwrap_or(&0.0);
    if total <= 0.0001 {
        return vec![usable[0]; count];
    }

    (0..count)
        .map(|sample| {
            let denominator = if closed {
                count
            } else {
                count.saturating_sub(1).max(1)
            };
            let distance = total * sample as f32 / denominator as f32;
            let segment = lengths
                .windows(2)
                .position(|range| distance <= range[1])
                .unwrap_or(segment_count - 1);
            let start = usable[segment];
            let next_index = if segment + 1 == usable.len() {
                0
            } else {
                segment + 1
            };
            let segment_len = lengths[segment + 1] - lengths[segment];
            let local_t = if segment_len <= 0.0001 {
                0.0
            } else {
                (distance - lengths[segment]) / segment_len
            };
            start.lerp(usable[next_index], local_t.clamp(0.0, 1.0))
        })
        .collect()
}

fn signed_area(points: &[Point2]) -> f32 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum::<f32>()
        * 0.5
}

fn match_subpath_winding(reference: &[Point2], candidate: &mut [Point2]) {
    if signed_area(reference).signum() != signed_area(candidate).signum() {
        candidate.reverse();
    }
}

fn align_closed_subpath_start(reference: &[Point2], candidate: &mut [Point2]) {
    if reference.is_empty() || candidate.is_empty() {
        return;
    }
    let best_shift = (0..candidate.len())
        .min_by(|a, b| {
            let cost_a = reference
                .iter()
                .enumerate()
                .map(|(index, point)| {
                    let other = candidate[(index + a) % candidate.len()];
                    (point.x - other.x).powi(2) + (point.y - other.y).powi(2)
                })
                .sum::<f32>();
            let cost_b = reference
                .iter()
                .enumerate()
                .map(|(index, point)| {
                    let other = candidate[(index + b) % candidate.len()];
                    (point.x - other.x).powi(2) + (point.y - other.y).powi(2)
                })
                .sum::<f32>();
            cost_a.total_cmp(&cost_b)
        })
        .unwrap_or(0);
    candidate.rotate_left(best_shift);
}

fn sampled_subpaths_to_d(subpaths: &[Vec<Point2>], originals: &[Vec<Point2>]) -> String {
    let mut out = String::new();
    for (points, original) in subpaths.iter().zip(originals) {
        let Some(first) = points.first() else {
            continue;
        };
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!(
            "M {} {}",
            format_path_morph_number(first.x),
            format_path_morph_number(first.y)
        ));
        for point in &points[1..] {
            out.push_str(&format!(
                " L {} {}",
                format_path_morph_number(point.x),
                format_path_morph_number(point.y)
            ));
        }
        if subpath_is_closed(original) {
            out.push_str(" Z");
        }
    }
    out
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
