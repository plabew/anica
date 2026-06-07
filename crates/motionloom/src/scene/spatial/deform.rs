use crate::scene::drawable::Point2;
use crate::scene::model::GroupNode;
use crate::scene::render::{MotionLoomSceneRenderError, eval_scene_number};

use super::Affine2;

#[derive(Debug, Clone)]
pub(crate) struct EvaluatedDeformGrid {
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    pub(crate) from: Vec<Point2>,
    pub(crate) to: Vec<Point2>,
}

pub(crate) fn eval_group_deform_grid(
    group: &GroupNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<Option<EvaluatedDeformGrid>, MotionLoomSceneRenderError> {
    let Some(size_raw) = group.deform_grid.as_deref() else {
        return Ok(None);
    };
    let size_raw = size_raw.trim();
    if size_raw.is_empty() || size_raw.eq_ignore_ascii_case("none") {
        return Ok(None);
    }

    let amount = eval_scene_number(&group.deform_amount, time_norm, time_sec)?.clamp(0.0, 1.0);
    if amount <= 0.0001 {
        return Ok(None);
    }

    let (cols, rows) = parse_deform_grid_size(size_raw)?;
    let expected = cols * rows;
    let grid_from_raw = group.grid_from.as_deref().ok_or_else(|| {
        invalid_deform_grid(size_raw, "deformGrid requires gridFrom=\"x,y ...\".")
    })?;
    let grid_to_raw = group
        .grid_to
        .as_deref()
        .ok_or_else(|| invalid_deform_grid(size_raw, "deformGrid requires gridTo=\"x,y ...\"."))?;
    let from = parse_deform_grid_points(grid_from_raw, cols, rows, "gridFrom")?;
    let target = parse_deform_grid_points(grid_to_raw, cols, rows, "gridTo")?;
    if from.len() != expected || target.len() != expected {
        return Err(invalid_deform_grid(
            size_raw,
            format!("expected {expected} control points."),
        ));
    }

    let to = from
        .iter()
        .zip(target.iter())
        .map(|(from, target)| from.lerp(*target, amount))
        .collect();

    Ok(Some(EvaluatedDeformGrid {
        cols,
        rows,
        from,
        to,
    }))
}

pub(crate) fn transform_deform_grid(
    grid: &EvaluatedDeformGrid,
    transform: Affine2,
) -> EvaluatedDeformGrid {
    EvaluatedDeformGrid {
        cols: grid.cols,
        rows: grid.rows,
        from: grid
            .from
            .iter()
            .map(|point| transform_point2(transform, *point))
            .collect(),
        to: grid
            .to
            .iter()
            .map(|point| transform_point2(transform, *point))
            .collect(),
    }
}

fn transform_point2(transform: Affine2, point: Point2) -> Point2 {
    let (x, y) = transform.transform_point(point.x, point.y);
    Point2::new(x, y)
}

pub(crate) fn transform_and_deform_point(
    transform: Affine2,
    point: Point2,
    deform: Option<&EvaluatedDeformGrid>,
) -> Point2 {
    let transformed = transform_point2(transform, point);
    deform
        .map(|grid| warp_point_with_deform_grid(transformed, grid))
        .unwrap_or(transformed)
}

pub(crate) fn transform_and_deform_subpaths(
    subpaths: &[Vec<Point2>],
    transform: Affine2,
    deform: &EvaluatedDeformGrid,
) -> Vec<Vec<Point2>> {
    subpaths
        .iter()
        .map(|subpath| {
            subpath
                .iter()
                .map(|point| transform_and_deform_point(transform, *point, Some(deform)))
                .collect()
        })
        .collect()
}

fn warp_point_with_deform_grid(point: Point2, grid: &EvaluatedDeformGrid) -> Point2 {
    for row in 0..grid.rows - 1 {
        for col in 0..grid.cols - 1 {
            let i00 = row * grid.cols + col;
            let i10 = i00 + 1;
            let i01 = (row + 1) * grid.cols + col;
            let i11 = i01 + 1;
            if let Some(warped) = warp_point_with_deform_triangle(
                point,
                [grid.from[i00], grid.from[i10], grid.from[i11]],
                [grid.to[i00], grid.to[i10], grid.to[i11]],
            ) {
                return warped;
            }
            if let Some(warped) = warp_point_with_deform_triangle(
                point,
                [grid.from[i00], grid.from[i11], grid.from[i01]],
                [grid.to[i00], grid.to[i11], grid.to[i01]],
            ) {
                return warped;
            }
        }
    }
    point
}

fn warp_point_with_deform_triangle(
    point: Point2,
    src: [Point2; 3],
    dst: [Point2; 3],
) -> Option<Point2> {
    let denom = triangle_barycentric_denominator(src);
    let (w0, w1, w2) = triangle_barycentric(point, src, denom)?;
    if w0 < -0.001 || w1 < -0.001 || w2 < -0.001 {
        return None;
    }
    Some(Point2::new(
        dst[0].x * w0 + dst[1].x * w1 + dst[2].x * w2,
        dst[0].y * w0 + dst[1].y * w1 + dst[2].y * w2,
    ))
}

fn parse_deform_grid_size(size: &str) -> Result<(usize, usize), MotionLoomSceneRenderError> {
    let normalized = size.trim().to_ascii_lowercase().replace(' ', "");
    let Some((cols_raw, rows_raw)) = normalized.split_once('x') else {
        return Err(invalid_deform_grid(
            size,
            "deformGrid must use the form \"colsxrows\", for example \"3x3\".",
        ));
    };
    let cols = cols_raw
        .parse::<usize>()
        .map_err(|_| invalid_deform_grid(size, format!("invalid column count: {cols_raw}")))?;
    let rows = rows_raw
        .parse::<usize>()
        .map_err(|_| invalid_deform_grid(size, format!("invalid row count: {rows_raw}")))?;
    if cols < 2 || rows < 2 || cols > 16 || rows > 16 {
        return Err(invalid_deform_grid(
            size,
            "deformGrid supports 2..16 columns and 2..16 rows.",
        ));
    }
    Ok((cols, rows))
}

fn parse_deform_grid_points(
    value: &str,
    cols: usize,
    rows: usize,
    label: &str,
) -> Result<Vec<Point2>, MotionLoomSceneRenderError> {
    let mut points = Vec::new();
    let row_chunks: Vec<&str> = if value.contains(';') {
        value.split(';').collect()
    } else {
        vec![value]
    };
    if row_chunks.len() != 1 && row_chunks.len() != rows {
        return Err(invalid_deform_grid(
            value,
            format!("{label} expected {rows} rows separated by ';'."),
        ));
    }

    for (row_index, row) in row_chunks.iter().enumerate() {
        let row_points = row
            .split_whitespace()
            .map(|raw| parse_deform_grid_point(raw, value))
            .collect::<Result<Vec<_>, _>>()?;
        if row_chunks.len() != 1 && row_points.len() != cols {
            return Err(invalid_deform_grid(
                value,
                format!(
                    "{label} row {} expected {cols} points, got {}.",
                    row_index + 1,
                    row_points.len()
                ),
            ));
        }
        points.extend(row_points);
    }

    let expected = cols * rows;
    if points.len() != expected {
        return Err(invalid_deform_grid(
            value,
            format!("{label} expected {expected} points, got {}.", points.len()),
        ));
    }
    Ok(points)
}

fn parse_deform_grid_point(raw: &str, source: &str) -> Result<Point2, MotionLoomSceneRenderError> {
    let Some((x_raw, y_raw)) = raw.split_once(',') else {
        return Err(invalid_deform_grid(
            source,
            format!("control point must be \"x,y\": {raw}"),
        ));
    };
    let x = x_raw
        .trim()
        .parse::<f32>()
        .map_err(|_| invalid_deform_grid(source, format!("invalid x value: {x_raw}")))?;
    let y = y_raw
        .trim()
        .parse::<f32>()
        .map_err(|_| invalid_deform_grid(source, format!("invalid y value: {y_raw}")))?;
    if !x.is_finite() || !y.is_finite() {
        return Err(invalid_deform_grid(
            source,
            format!("control point must be finite: {raw}"),
        ));
    }
    Ok(Point2::new(x, y))
}

pub(crate) fn triangle_barycentric_denominator(tri: [Point2; 3]) -> f32 {
    (tri[1].y - tri[2].y) * (tri[0].x - tri[2].x) + (tri[2].x - tri[1].x) * (tri[0].y - tri[2].y)
}

pub(crate) fn triangle_barycentric(
    point: Point2,
    tri: [Point2; 3],
    denom: f32,
) -> Option<(f32, f32, f32)> {
    if denom.abs() <= 0.00001 {
        return None;
    }
    let w0 = ((tri[1].y - tri[2].y) * (point.x - tri[2].x)
        + (tri[2].x - tri[1].x) * (point.y - tri[2].y))
        / denom;
    let w1 = ((tri[2].y - tri[0].y) * (point.x - tri[2].x)
        + (tri[0].x - tri[2].x) * (point.y - tri[2].y))
        / denom;
    let w2 = 1.0 - w0 - w1;
    Some((w0, w1, w2))
}

fn invalid_deform_grid(value: &str, message: impl Into<String>) -> MotionLoomSceneRenderError {
    MotionLoomSceneRenderError::InvalidDeformGrid {
        value: value.to_string(),
        message: message.into(),
    }
}
