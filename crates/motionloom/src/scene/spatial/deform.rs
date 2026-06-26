use crate::scene::drawable::Point2;
use crate::scene::model::{GroupNode, MeshTopologyNode, PinNode, PuppetNode, SceneNode};
use crate::scene::render::{MotionLoomSceneRenderError, eval_scene_number};
use std::collections::HashMap;

use super::Affine2;

#[derive(Debug, Clone)]
pub(crate) struct EvaluatedDeformGrid {
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    pub(crate) from: Vec<Point2>,
    pub(crate) to: Vec<Point2>,
    pub(crate) triangles: Vec<[usize; 3]>,
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
        triangles: Vec::new(),
    }))
}

pub(crate) fn eval_puppet_deform_grid(
    puppet: &PuppetNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<Option<EvaluatedDeformGrid>, MotionLoomSceneRenderError> {
    let mesh = puppet.mesh.trim();
    if mesh.eq_ignore_ascii_case("none") {
        return Ok(None);
    }

    let amount = eval_scene_number(&puppet.amount, time_norm, time_sec)?.clamp(0.0, 1.0);
    if amount <= 0.0001 {
        return Ok(None);
    }

    let width = eval_scene_number(&puppet.width, time_norm, time_sec)?.max(1.0);
    let height = eval_scene_number(&puppet.height, time_norm, time_sec)?.max(1.0);
    let topology = puppet_topology_mesh(puppet, time_norm, time_sec)?;
    let (cols, rows, from, triangles) = if topology.triangles.is_empty() {
        let (cols, rows) = puppet_grid_size(&puppet.density);
        (
            cols,
            rows,
            regular_grid_points(width, height, cols, rows),
            Vec::new(),
        )
    } else {
        (
            topology.vertices.len().max(1),
            1,
            topology.vertices.clone(),
            topology.triangles.clone(),
        )
    };
    let pins = puppet_pin_controls(puppet, &topology.vertex_map, amount, time_norm, time_sec)?;
    if pins.is_empty() {
        return Ok(None);
    }

    let to = from
        .iter()
        .map(|point| apply_puppet_pins_to_point(*point, &pins))
        .collect::<Vec<_>>();
    if from
        .iter()
        .zip(to.iter())
        .all(|(a, b)| (a.x - b.x).abs() <= 0.001 && (a.y - b.y).abs() <= 0.001)
    {
        return Ok(None);
    }

    Ok(Some(EvaluatedDeformGrid {
        cols,
        rows,
        from,
        to,
        triangles,
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
        triangles: grid.triangles.clone(),
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
    if !grid.triangles.is_empty() {
        for triangle in &grid.triangles {
            if triangle
                .iter()
                .any(|index| *index >= grid.from.len() || *index >= grid.to.len())
            {
                continue;
            }
            if let Some(warped) = warp_point_with_deform_triangle(
                point,
                [
                    grid.from[triangle[0]],
                    grid.from[triangle[1]],
                    grid.from[triangle[2]],
                ],
                [
                    grid.to[triangle[0]],
                    grid.to[triangle[1]],
                    grid.to[triangle[2]],
                ],
            ) {
                return warped;
            }
        }
        return point;
    }
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

fn puppet_grid_size(density: &str) -> (usize, usize) {
    match density.trim().to_ascii_lowercase().as_str() {
        "low" | "coarse" => (3, 3),
        "high" | "fine" => (7, 7),
        "ultra" | "dense" => (9, 9),
        raw => parse_deform_grid_size(raw).unwrap_or((5, 5)),
    }
}

fn regular_grid_points(width: f32, height: f32, cols: usize, rows: usize) -> Vec<Point2> {
    let mut points = Vec::with_capacity(cols * rows);
    for row in 0..rows {
        let y = if rows <= 1 {
            0.0
        } else {
            height * row as f32 / (rows - 1) as f32
        };
        for col in 0..cols {
            let x = if cols <= 1 {
                0.0
            } else {
                width * col as f32 / (cols - 1) as f32
            };
            points.push(Point2::new(x, y));
        }
    }
    points
}

#[derive(Debug, Clone, Default)]
struct EvaluatedPuppetTopology {
    vertex_map: HashMap<String, Point2>,
    vertex_indices: HashMap<String, usize>,
    vertices: Vec<Point2>,
    triangles: Vec<[usize; 3]>,
}

#[derive(Debug, Clone)]
struct EvaluatedPuppetPin {
    source: Point2,
    delta: Point2,
    radius: f32,
    strength: f32,
    falloff: String,
}

fn puppet_pin_controls(
    puppet: &PuppetNode,
    vertices: &HashMap<String, Point2>,
    amount: f32,
    time_norm: f32,
    time_sec: f32,
) -> Result<Vec<EvaluatedPuppetPin>, MotionLoomSceneRenderError> {
    let mut pins = Vec::new();
    for child in &puppet.children {
        let SceneNode::Pin(pin) = child else {
            continue;
        };
        let source = eval_pin_source(pin, vertices, time_norm, time_sec)?;
        let fixed = eval_pin_fixed(pin, time_norm, time_sec)?;
        let target_x = if fixed {
            source.x
        } else {
            pin.target_x
                .as_deref()
                .map(|expr| eval_scene_number(expr, time_norm, time_sec))
                .transpose()?
                .unwrap_or(source.x)
        };
        let target_y = if fixed {
            source.y
        } else {
            pin.target_y
                .as_deref()
                .map(|expr| eval_scene_number(expr, time_norm, time_sec))
                .transpose()?
                .unwrap_or(source.y)
        };
        let radius = eval_scene_number(&pin.radius, time_norm, time_sec)?.max(0.001);
        let strength = eval_scene_number(&pin.strength, time_norm, time_sec)?.clamp(0.0, 8.0);
        pins.push(EvaluatedPuppetPin {
            source,
            delta: Point2::new(
                (target_x - source.x) * amount,
                (target_y - source.y) * amount,
            ),
            radius,
            strength,
            falloff: pin.falloff.clone(),
        });
    }
    Ok(pins)
}

fn eval_pin_source(
    pin: &PinNode,
    vertices: &HashMap<String, Point2>,
    time_norm: f32,
    time_sec: f32,
) -> Result<Point2, MotionLoomSceneRenderError> {
    if let Some(vertex) = pin.vertex.as_deref()
        && let Some(point) = vertices.get(vertex)
    {
        return Ok(*point);
    }
    let x = pin.x.as_deref().ok_or_else(|| {
        invalid_deform_grid(
            pin.id.as_deref().unwrap_or("pin"),
            "Pin requires x/y or vertex.",
        )
    })?;
    let y = pin.y.as_deref().ok_or_else(|| {
        invalid_deform_grid(
            pin.id.as_deref().unwrap_or("pin"),
            "Pin requires x/y or vertex.",
        )
    })?;
    Ok(Point2::new(
        eval_scene_number(x, time_norm, time_sec)?,
        eval_scene_number(y, time_norm, time_sec)?,
    ))
}

fn eval_pin_fixed(
    pin: &PinNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<bool, MotionLoomSceneRenderError> {
    let raw = pin.fixed.trim();
    if raw.eq_ignore_ascii_case("true") || raw.eq_ignore_ascii_case("yes") || raw == "1" {
        return Ok(true);
    }
    if raw.eq_ignore_ascii_case("false") || raw.eq_ignore_ascii_case("no") || raw == "0" {
        return Ok(false);
    }
    Ok(eval_scene_number(raw, time_norm, time_sec)? >= 0.5)
}

fn puppet_topology_mesh(
    puppet: &PuppetNode,
    time_norm: f32,
    time_sec: f32,
) -> Result<EvaluatedPuppetTopology, MotionLoomSceneRenderError> {
    let mut topology_eval = EvaluatedPuppetTopology::default();
    for topology in puppet.children.iter().filter_map(|child| match child {
        SceneNode::MeshTopology(topology) => Some(topology),
        _ => None,
    }) {
        collect_topology_vertices(topology, &mut topology_eval, time_norm, time_sec)?;
        collect_topology_triangles(topology, &mut topology_eval);
    }
    Ok(topology_eval)
}

fn collect_topology_vertices(
    topology: &MeshTopologyNode,
    out: &mut EvaluatedPuppetTopology,
    time_norm: f32,
    time_sec: f32,
) -> Result<(), MotionLoomSceneRenderError> {
    for child in &topology.children {
        if let SceneNode::Vertex(vertex) = child {
            let point = Point2::new(
                eval_scene_number(&vertex.x, time_norm, time_sec)?,
                eval_scene_number(&vertex.y, time_norm, time_sec)?,
            );
            let index = out.vertices.len();
            out.vertex_map.insert(vertex.id.clone(), point);
            out.vertex_indices.insert(vertex.id.clone(), index);
            out.vertices.push(point);
        }
    }
    Ok(())
}

fn collect_topology_triangles(topology: &MeshTopologyNode, out: &mut EvaluatedPuppetTopology) {
    for child in &topology.children {
        if let SceneNode::Triangle(triangle) = child {
            let Some(a) = out.vertex_indices.get(&triangle.a).copied() else {
                continue;
            };
            let Some(b) = out.vertex_indices.get(&triangle.b).copied() else {
                continue;
            };
            let Some(c) = out.vertex_indices.get(&triangle.c).copied() else {
                continue;
            };
            out.triangles.push([a, b, c]);
        }
    }
}

fn apply_puppet_pins_to_point(point: Point2, pins: &[EvaluatedPuppetPin]) -> Point2 {
    let mut dx = 0.0;
    let mut dy = 0.0;
    let mut weight_sum = 0.0;
    for pin in pins {
        let distance = ((point.x - pin.source.x).powi(2) + (point.y - pin.source.y).powi(2)).sqrt();
        let mut weight = puppet_pin_falloff(distance, pin.radius, &pin.falloff) * pin.strength;
        if !weight.is_finite() {
            weight = 0.0;
        }
        dx += pin.delta.x * weight;
        dy += pin.delta.y * weight;
        weight_sum += weight;
    }
    let divisor = weight_sum.max(1.0);
    Point2::new(point.x + dx / divisor, point.y + dy / divisor)
}

fn puppet_pin_falloff(distance: f32, radius: f32, falloff: &str) -> f32 {
    if distance >= radius {
        return 0.0;
    }
    let t = (1.0 - distance / radius).clamp(0.0, 1.0);
    match falloff.trim().to_ascii_lowercase().as_str() {
        "linear" => t,
        "gaussian" | "gauss" => (-(distance / radius).powi(2) * 4.0).exp(),
        "none" | "constant" => 1.0,
        _ => t * t * (3.0 - 2.0 * t),
    }
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
