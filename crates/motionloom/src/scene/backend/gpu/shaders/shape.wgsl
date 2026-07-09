
struct ShapeParams {
    canvas: vec4<f32>,
    bounds: vec4<f32>,
    shape: vec4<f32>,
    style: vec4<f32>,
    color: vec4<f32>,
    inv0: vec4<f32>,
    inv1: vec4<f32>,
    paint: vec4<f32>,
    paint_bounds: vec4<f32>,
    gradient: vec4<f32>,
    stop_offsets0: vec4<f32>,
    stop_offsets1: vec4<f32>,
    stop_color0: vec4<f32>,
    stop_color1: vec4<f32>,
    stop_color2: vec4<f32>,
    stop_color3: vec4<f32>,
    stop_color4: vec4<f32>,
    stop_color5: vec4<f32>,
    stop_color6: vec4<f32>,
    stop_color7: vec4<f32>,
    line: vec4<f32>,
};

@group(0) @binding(0) var base_tex: texture_2d<f32>;
@group(0) @binding(1) var out_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> params: ShapeParams;

fn rounded_rect_sdf(p: vec2<f32>, rect: vec4<f32>, radius: f32) -> f32 {
    let half_size = max(rect.zw * 0.5, vec2<f32>(0.0001, 0.0001));
    let center = rect.xy + half_size;
    let r = clamp(radius, 0.0, min(half_size.x, half_size.y));
    let q = abs(p - center) - half_size + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn segment_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let h = clamp(dot(p - a, ab) / max(dot(ab, ab), 0.0001), 0.0, 1.0);
    return length(p - (a + ab * h));
}

fn cross2(a: vec2<f32>, b: vec2<f32>) -> f32 {
    return a.x * b.y - a.y * b.x;
}

fn triangle_coverage(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>) -> f32 {
    let area = cross2(b - a, c - a);
    if (abs(area) <= 0.0001) {
        return 0.0;
    }
    let e0 = cross2(b - a, p - a);
    let e1 = cross2(c - b, p - b);
    let e2 = cross2(a - c, p - c);
    var same_side = false;
    if (area > 0.0) {
        same_side = e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0;
    } else {
        same_side = e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0;
    }
    if (!same_side) {
        return 0.0;
    }
    // Filled paths are triangulated before reaching this shader. Applying AA to
    // every triangle edge makes internal triangulation seams visible as moire /
    // wave bands on flat anime fills. Keep triangle interiors hard-filled here;
    // visible contour quality should be handled by the path outline/stroke.
    return 1.0;
}

fn inside_coverage(dist: f32) -> f32 {
    return clamp(0.5 - dist, 0.0, 1.0);
}

fn over(base: vec4<f32>, src_rgb: vec3<f32>, src_a: f32) -> vec4<f32> {
    let a = clamp(src_a, 0.0, 1.0);
    let out_a = a + base.a * (1.0 - a);
    if (out_a <= 0.000001) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let rgb = (src_rgb * a + base.rgb * base.a * (1.0 - a)) / out_a;
    return vec4<f32>(rgb, out_a);
}

fn blend_over(base: vec4<f32>, src_rgb: vec3<f32>, src_a: f32, mode: f32) -> vec4<f32> {
    let blend_mode = i32(round(mode));
    if (blend_mode == 0) {
        return over(base, src_rgb, src_a);
    }

    let a = clamp(src_a, 0.0, 1.0);
    if (a <= 0.0) {
        return base;
    }

    var blended = src_rgb;
    if (blend_mode == 1) {
        blended = src_rgb * base.rgb;
    } else if (blend_mode == 2) {
        let one = vec3<f32>(1.0, 1.0, 1.0);
        blended = one - (one - src_rgb) * (one - base.rgb);
    } else if (blend_mode == 3) {
        blended = min(src_rgb + base.rgb, vec3<f32>(1.0, 1.0, 1.0));
    }

    let out_a = a + base.a * (1.0 - a);
    if (out_a <= 0.000001) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let rgb = (blended * a + base.rgb * base.a * (1.0 - a)) / out_a;
    return vec4<f32>(rgb, out_a);
}

fn stop_offset(index: i32) -> f32 {
    if (index == 0) { return params.stop_offsets0.x; }
    if (index == 1) { return params.stop_offsets0.y; }
    if (index == 2) { return params.stop_offsets0.z; }
    if (index == 3) { return params.stop_offsets0.w; }
    if (index == 4) { return params.stop_offsets1.x; }
    if (index == 5) { return params.stop_offsets1.y; }
    if (index == 6) { return params.stop_offsets1.z; }
    return params.stop_offsets1.w;
}

fn stop_color(index: i32) -> vec4<f32> {
    if (index == 0) { return params.stop_color0; }
    if (index == 1) { return params.stop_color1; }
    if (index == 2) { return params.stop_color2; }
    if (index == 3) { return params.stop_color3; }
    if (index == 4) { return params.stop_color4; }
    if (index == 5) { return params.stop_color5; }
    if (index == 6) { return params.stop_color6; }
    return params.stop_color7;
}

fn sample_gradient_stops(t_in: f32) -> vec4<f32> {
    let count = i32(clamp(round(params.paint.z), 0.0, 8.0));
    if (count <= 0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let t = clamp(t_in, 0.0, 1.0);
    if (t <= stop_offset(0)) {
        return stop_color(0);
    }
    for (var i = 1; i < 8; i = i + 1) {
        if (i < count && t <= stop_offset(i)) {
            let a_off = stop_offset(i - 1);
            let b_off = stop_offset(i);
            let span = max(b_off - a_off, 0.000001);
            let local_t = clamp((t - a_off) / span, 0.0, 1.0);
            let a = stop_color(i - 1);
            let b = stop_color(i);
            return a + (b - a) * local_t;
        }
    }
    return stop_color(max(count - 1, 0));
}

fn sample_shape_paint(local: vec2<f32>) -> vec4<f32> {
    let paint_kind = i32(round(params.paint.x));
    if (paint_kind == 0) {
        return params.color;
    }

    let units = i32(round(params.paint.y));
    let min_p = params.paint_bounds.xy;
    let max_p = params.paint_bounds.zw;
    let size = max(max_p - min_p, vec2<f32>(0.0001, 0.0001));
    var p = local;
    if (units == 0) {
        p = (local - min_p) / size;
    }

    if (paint_kind == 1) {
        let start = params.gradient.xy;
        let end = params.gradient.zw;
        let dir = end - start;
        let len2 = max(dot(dir, dir), 0.000001);
        return sample_gradient_stops(dot(p - start, dir) / len2);
    }

    if (paint_kind == 2) {
        let center = params.gradient.xy;
        var delta = p - center;
        if (units == 0) {
            let aspect = select(1.0, size.y / size.x, size.x > size.y);
            delta.x = delta.x / max(aspect, 0.0001);
        }
        return sample_gradient_stops(length(delta) / max(params.gradient.z, 0.0001));
    }

    return params.color;
}

fn line_taper_pressure(t: f32) -> f32 {
    var pressure = 1.0;
    if (params.line.z > 0.0001) {
        pressure = min(pressure, clamp(t / params.line.z, 0.0, 1.0));
    }
    if (params.line.w > 0.0001) {
        pressure = min(pressure, clamp((1.0 - t) / params.line.w, 0.0, 1.0));
    }
    return pressure;
}

@compute @workgroup_size(16, 16, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= u32(params.bounds.z) || gid.y >= u32(params.bounds.w)) {
        return;
    }

    let px_u = u32(params.bounds.x) + gid.x;
    let py_u = u32(params.bounds.y) + gid.y;
    if (px_u >= u32(params.canvas.x) || py_u >= u32(params.canvas.y)) {
        return;
    }

    let pos = vec2<i32>(i32(px_u), i32(py_u));
    let base = textureLoad(base_tex, pos, 0);
    let px = f32(px_u) + 0.5;
    let py = f32(py_u) + 0.5;
    let local = vec2<f32>(
        params.inv0.x * px + params.inv0.y * py + params.inv0.z,
        params.inv1.x * px + params.inv1.y * py + params.inv1.z
    );

    let shape_kind = i32(round(params.canvas.z));
    var coverage = 0.0;
    var replace = false;

    if (shape_kind == 7) {
        coverage = 1.0;
        replace = true;
    } else if (shape_kind == 1) {
        let dist = rounded_rect_sdf(local, params.shape, params.style.x);
        coverage = inside_coverage(dist);
    } else if (shape_kind == 2) {
        let outer = rounded_rect_sdf(local, params.shape, params.style.x);
        let sw = max(params.style.y, 0.0);
        let inner = rounded_rect_sdf(
            local,
            vec4<f32>(params.shape.x + sw, params.shape.y + sw, max(params.shape.z - sw * 2.0, 0.0), max(params.shape.w - sw * 2.0, 0.0)),
            max(params.style.x - sw, 0.0)
        );
        coverage = inside_coverage(outer) * (1.0 - inside_coverage(inner));
    } else if (shape_kind == 3) {
        let dist = length(local - params.shape.xy) - params.shape.z;
        coverage = inside_coverage(dist);
    } else if (shape_kind == 4) {
        let dist = length(local - params.shape.xy) - params.shape.z;
        let sw = max(params.style.y, 0.0);
        coverage = inside_coverage(dist) * (1.0 - inside_coverage(dist + sw));
    } else if (shape_kind == 5) {
        let dist = rounded_rect_sdf(local, params.shape, params.style.x);
        let outside = max(dist, 0.0);
        let blur = max(params.style.z, 1.0);
        let sigma = max(blur * 0.42, 1.0);
        let inside = smoothstep(0.0, blur * 0.7, max(-dist, 0.0));
        let outside_falloff = exp(-(outside * outside) / (2.0 * sigma * sigma));
        coverage = min(max(inside, outside_falloff), 0.86);
    } else if (shape_kind == 6) {
        let dist = length(local - params.shape.xy) - params.shape.z;
        let outside = max(dist, 0.0);
        let blur = max(params.style.z, 1.0);
        let sigma = max(blur * 0.42, 1.0);
        let inside = smoothstep(0.0, blur * 0.7, max(-dist, 0.0));
        let outside_falloff = exp(-(outside * outside) / (2.0 * sigma * sigma));
        coverage = min(max(inside, outside_falloff), 0.86);
    } else if (shape_kind == 8) {
        let ab = params.shape.zw - params.shape.xy;
        let h = clamp(dot(local - params.shape.xy, ab) / max(dot(ab, ab), 0.0001), 0.0, 1.0);
        let global_t = params.line.x + (params.line.y - params.line.x) * h;
        let pressure = line_taper_pressure(global_t);
        let half_width = max(params.style.y * pressure * 0.5, 0.01);
        let dist = segment_distance(local, params.shape.xy, params.shape.zw) - half_width;
        coverage = inside_coverage(dist);
    } else if (shape_kind == 9) {
        coverage = triangle_coverage(local, params.shape.xy, params.shape.zw, params.style.xy);
    } else if (shape_kind == 10) {
        let radii = max(params.shape.zw, vec2<f32>(0.0001, 0.0001));
        let q = (local - params.shape.xy) / radii;
        let dist = (length(q) - 1.0) * min(radii.x, radii.y);
        coverage = inside_coverage(dist);
    } else if (shape_kind == 11) {
        let radii = max(params.shape.zw, vec2<f32>(0.0001, 0.0001));
        let q = (local - params.shape.xy) / radii;
        let dist = (length(q) - 1.0) * min(radii.x, radii.y);
        let sw = max(params.style.y, 0.0);
        coverage = inside_coverage(dist) * (1.0 - inside_coverage(dist + sw));
    }

    coverage = clamp(coverage, 0.0, 1.0);
    let paint_color = sample_shape_paint(local);
    let src_a = paint_color.a * params.style.w * coverage;
    let out_color = select(over(base, paint_color.rgb, src_a), vec4<f32>(paint_color.rgb, paint_color.a * params.style.w), replace);
    textureStore(out_tex, pos, out_color);
}
