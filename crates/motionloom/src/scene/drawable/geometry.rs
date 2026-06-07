use crate::scene::render::MotionLoomSceneRenderError;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Point2 {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

impl Point2 {
    pub(crate) fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub(crate) fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            x: self.x + (other.x - self.x) * t,
            y: self.y + (other.y - self.y) * t,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PathToken {
    Command(char),
    Number(f32),
}

pub(crate) fn parse_polyline_points(
    points: &str,
) -> Result<Vec<Point2>, MotionLoomSceneRenderError> {
    let values = points
        .replace(',', " ")
        .split_whitespace()
        .map(|raw| {
            raw.parse::<f32>()
                .map_err(|_| MotionLoomSceneRenderError::InvalidPathData {
                    value: points.to_string(),
                    message: format!("invalid point number: {raw}"),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() < 4 || values.len() % 2 != 0 {
        return Err(MotionLoomSceneRenderError::InvalidPathData {
            value: points.to_string(),
            message: "Polyline points must contain at least two x,y pairs.".to_string(),
        });
    }
    Ok(values
        .chunks_exact(2)
        .map(|pair| Point2::new(pair[0], pair[1]))
        .collect())
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TrimmedSegment {
    pub(crate) p0: Point2,
    pub(crate) p1: Point2,
    pub(crate) t0: f32,
    pub(crate) t1: f32,
}

pub(crate) fn trimmed_polyline_segments_with_progress(
    subpaths: &[Vec<Point2>],
    trim: (f32, f32),
) -> Vec<TrimmedSegment> {
    if trim.1 <= trim.0 {
        return Vec::new();
    }
    let total = polyline_total_length(subpaths);
    if total <= 0.0001 {
        return Vec::new();
    }
    let start_distance = trim.0 * total;
    let end_distance = trim.1 * total;
    let mut cursor = 0.0;
    let mut out = Vec::new();

    for subpath in subpaths {
        for segment in subpath.windows(2) {
            let p0 = segment[0];
            let p1 = segment[1];
            let len = point_distance(p0, p1);
            if len <= 0.0001 {
                continue;
            }
            let seg_start = cursor;
            let seg_end = cursor + len;
            let draw_start = start_distance.max(seg_start);
            let draw_end = end_distance.min(seg_end);
            if draw_end > draw_start {
                let t0 = (draw_start - seg_start) / len;
                let t1 = (draw_end - seg_start) / len;
                out.push(TrimmedSegment {
                    p0: p0.lerp(p1, t0),
                    p1: p0.lerp(p1, t1),
                    t0: draw_start / total,
                    t1: draw_end / total,
                });
            }
            cursor = seg_end;
        }
    }
    out
}

pub(crate) fn polyline_total_length(subpaths: &[Vec<Point2>]) -> f32 {
    subpaths
        .iter()
        .flat_map(|subpath| subpath.windows(2))
        .map(|segment| point_distance(segment[0], segment[1]))
        .sum()
}

pub(crate) fn point_distance(a: Point2, b: Point2) -> f32 {
    ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt()
}

pub(crate) fn parse_path_subpaths(
    data: &str,
) -> Result<Vec<Vec<Point2>>, MotionLoomSceneRenderError> {
    let tokens = tokenize_path_data(data)?;
    let mut i = 0usize;
    let mut command: Option<char> = None;
    let mut current = Point2::new(0.0, 0.0);
    let mut subpath_start = current;
    let mut active = Vec::<Point2>::new();
    let mut subpaths = Vec::<Vec<Point2>>::new();

    while i < tokens.len() {
        if let Some(PathToken::Command(cmd)) = tokens.get(i).copied() {
            command = Some(cmd);
            i += 1;
        }
        let cmd = command.ok_or_else(|| MotionLoomSceneRenderError::InvalidPathData {
            value: data.to_string(),
            message: "path data must start with a command.".to_string(),
        })?;

        match cmd {
            'M' | 'm' => {
                flush_active_subpath(&mut active, &mut subpaths);
                let relative = cmd == 'm';
                let first = consume_path_point(&tokens, &mut i, current, relative, data)?;
                current = first;
                subpath_start = first;
                active.push(first);
                let line_cmd = if relative { 'l' } else { 'L' };
                while next_path_token_is_number(&tokens, i) {
                    current = consume_path_point(&tokens, &mut i, current, relative, data)?;
                    active.push(current);
                }
                command = Some(line_cmd);
            }
            'L' | 'l' => {
                let relative = cmd == 'l';
                while next_path_token_is_number(&tokens, i) {
                    current = consume_path_point(&tokens, &mut i, current, relative, data)?;
                    active.push(current);
                }
            }
            'H' | 'h' => {
                let relative = cmd == 'h';
                while next_path_token_is_number(&tokens, i) {
                    let x = consume_path_number(&tokens, &mut i, data)?;
                    current = if relative {
                        Point2::new(current.x + x, current.y)
                    } else {
                        Point2::new(x, current.y)
                    };
                    active.push(current);
                }
            }
            'V' | 'v' => {
                let relative = cmd == 'v';
                while next_path_token_is_number(&tokens, i) {
                    let y = consume_path_number(&tokens, &mut i, data)?;
                    current = if relative {
                        Point2::new(current.x, current.y + y)
                    } else {
                        Point2::new(current.x, y)
                    };
                    active.push(current);
                }
            }
            'C' | 'c' => {
                let relative = cmd == 'c';
                while next_path_token_is_number(&tokens, i) {
                    let c1 = consume_path_point(&tokens, &mut i, current, relative, data)?;
                    let c2 = consume_path_point(&tokens, &mut i, current, relative, data)?;
                    let end = consume_path_point(&tokens, &mut i, current, relative, data)?;
                    sample_cubic(current, c1, c2, end, &mut active);
                    current = end;
                }
            }
            'Q' | 'q' => {
                let relative = cmd == 'q';
                while next_path_token_is_number(&tokens, i) {
                    let c = consume_path_point(&tokens, &mut i, current, relative, data)?;
                    let end = consume_path_point(&tokens, &mut i, current, relative, data)?;
                    sample_quadratic(current, c, end, &mut active);
                    current = end;
                }
            }
            'Z' | 'z' => {
                if !active.is_empty() {
                    active.push(subpath_start);
                    current = subpath_start;
                    flush_active_subpath(&mut active, &mut subpaths);
                }
                command = None;
            }
            _ => {
                return Err(MotionLoomSceneRenderError::InvalidPathData {
                    value: data.to_string(),
                    message: format!("unsupported path command: {cmd}"),
                });
            }
        }
    }

    flush_active_subpath(&mut active, &mut subpaths);
    if subpaths.is_empty() {
        return Err(MotionLoomSceneRenderError::InvalidPathData {
            value: data.to_string(),
            message: "path does not contain drawable segments.".to_string(),
        });
    }
    Ok(subpaths)
}

pub(crate) fn tokenize_path_data(data: &str) -> Result<Vec<PathToken>, MotionLoomSceneRenderError> {
    let bytes = data.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if ch.is_ascii_whitespace() || ch == ',' {
            i += 1;
            continue;
        }
        if is_path_command(ch) {
            tokens.push(PathToken::Command(ch));
            i += 1;
            continue;
        }
        if is_path_number_start(ch) {
            let start = i;
            i += 1;
            while i < bytes.len() {
                let ch = bytes[i] as char;
                if ch.is_ascii_digit() || ch == '.' {
                    i += 1;
                    continue;
                }
                if ch == 'e' || ch == 'E' {
                    i += 1;
                    if i < bytes.len() {
                        let sign = bytes[i] as char;
                        if sign == '+' || sign == '-' {
                            i += 1;
                        }
                    }
                    continue;
                }
                break;
            }
            let raw = &data[start..i];
            let value =
                raw.parse::<f32>()
                    .map_err(|_| MotionLoomSceneRenderError::InvalidPathData {
                        value: data.to_string(),
                        message: format!("invalid path number: {raw}"),
                    })?;
            tokens.push(PathToken::Number(value));
            continue;
        }
        return Err(MotionLoomSceneRenderError::InvalidPathData {
            value: data.to_string(),
            message: format!("unexpected path character: {ch}"),
        });
    }
    Ok(tokens)
}

fn is_path_command(ch: char) -> bool {
    matches!(
        ch,
        'M' | 'm' | 'L' | 'l' | 'H' | 'h' | 'V' | 'v' | 'C' | 'c' | 'Q' | 'q' | 'Z' | 'z'
    )
}

fn is_path_number_start(ch: char) -> bool {
    ch.is_ascii_digit() || ch == '-' || ch == '+' || ch == '.'
}

fn next_path_token_is_number(tokens: &[PathToken], index: usize) -> bool {
    matches!(tokens.get(index), Some(PathToken::Number(_)))
}

fn consume_path_number(
    tokens: &[PathToken],
    index: &mut usize,
    source: &str,
) -> Result<f32, MotionLoomSceneRenderError> {
    match tokens.get(*index).copied() {
        Some(PathToken::Number(value)) => {
            *index += 1;
            Ok(value)
        }
        _ => Err(MotionLoomSceneRenderError::InvalidPathData {
            value: source.to_string(),
            message: "path command is missing a numeric parameter.".to_string(),
        }),
    }
}

fn consume_path_point(
    tokens: &[PathToken],
    index: &mut usize,
    current: Point2,
    relative: bool,
    source: &str,
) -> Result<Point2, MotionLoomSceneRenderError> {
    let x = consume_path_number(tokens, index, source)?;
    let y = consume_path_number(tokens, index, source)?;
    if relative {
        Ok(Point2::new(current.x + x, current.y + y))
    } else {
        Ok(Point2::new(x, y))
    }
}

fn flush_active_subpath(active: &mut Vec<Point2>, subpaths: &mut Vec<Vec<Point2>>) {
    if active.len() >= 2 {
        subpaths.push(std::mem::take(active));
    } else {
        active.clear();
    }
}

fn sample_cubic(p0: Point2, c1: Point2, c2: Point2, p1: Point2, out: &mut Vec<Point2>) {
    const STEPS: usize = 28;
    for step in 1..=STEPS {
        let t = step as f32 / STEPS as f32;
        let mt = 1.0 - t;
        out.push(Point2::new(
            mt.powi(3) * p0.x
                + 3.0 * mt.powi(2) * t * c1.x
                + 3.0 * mt * t.powi(2) * c2.x
                + t.powi(3) * p1.x,
            mt.powi(3) * p0.y
                + 3.0 * mt.powi(2) * t * c1.y
                + 3.0 * mt * t.powi(2) * c2.y
                + t.powi(3) * p1.y,
        ));
    }
}

fn sample_quadratic(p0: Point2, c: Point2, p1: Point2, out: &mut Vec<Point2>) {
    const STEPS: usize = 20;
    for step in 1..=STEPS {
        let t = step as f32 / STEPS as f32;
        let mt = 1.0 - t;
        out.push(Point2::new(
            mt.powi(2) * p0.x + 2.0 * mt * t * c.x + t.powi(2) * p1.x,
            mt.powi(2) * p0.y + 2.0 * mt * t * c.y + t.powi(2) * p1.y,
        ));
    }
}

pub(crate) fn polyline_bounds(subpaths: &[Vec<Point2>]) -> Option<(f32, f32, f32, f32)> {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut any = false;
    for point in subpaths.iter().flatten() {
        any = true;
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    any.then_some((min_x, min_y, max_x, max_y))
}

pub(crate) fn point_in_subpaths_even_odd(point: Point2, subpaths: &[Vec<Point2>]) -> bool {
    let mut inside = false;
    for subpath in subpaths {
        if subpath.len() < 3 {
            continue;
        }
        let mut prev = *subpath.last().unwrap_or(&subpath[0]);
        for current in subpath {
            let denom = prev.y - current.y;
            if ((current.y > point.y) != (prev.y > point.y))
                && (point.x
                    < (prev.x - current.x) * (point.y - current.y)
                        / if denom.abs() <= 0.000001 {
                            0.000001
                        } else {
                            denom
                        }
                        + current.x)
            {
                inside = !inside;
            }
            prev = *current;
        }
    }
    inside
}
