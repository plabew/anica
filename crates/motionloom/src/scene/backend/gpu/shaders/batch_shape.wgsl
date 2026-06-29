
struct BatchParams {
    canvas: vec4<f32>,
    count: vec4<f32>,
};

struct Primitive {
    info: vec4<f32>,
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

struct PrimitiveBuffer {
    items: array<Primitive>,
};

struct TileRangeBuffer {
    items: array<vec4<u32>>,
};

struct TileIndexBuffer {
    items: array<u32>,
};

@group(0) @binding(0) var base_tex: texture_2d<f32>;
@group(0) @binding(1) var out_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> params: BatchParams;
@group(0) @binding(3) var<storage, read> primitive_buffer: PrimitiveBuffer;
@group(0) @binding(4) var<storage, read> tile_range_buffer: TileRangeBuffer;
@group(0) @binding(5) var<storage, read> tile_index_buffer: TileIndexBuffer;

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

fn encode_pick_id(id_f: f32) -> vec4<f32> {
    let id = u32(round(id_f));
    let r = f32(id & 255u) / 255.0;
    let g = f32((id >> 8u) & 255u) / 255.0;
    let b = f32((id >> 16u) & 255u) / 255.0;
    return vec4<f32>(r, g, b, 1.0);
}

fn stop_offset(p: Primitive, index: i32) -> f32 {
    if (index == 0) { return p.stop_offsets0.x; }
    if (index == 1) { return p.stop_offsets0.y; }
    if (index == 2) { return p.stop_offsets0.z; }
    if (index == 3) { return p.stop_offsets0.w; }
    if (index == 4) { return p.stop_offsets1.x; }
    if (index == 5) { return p.stop_offsets1.y; }
    if (index == 6) { return p.stop_offsets1.z; }
    return p.stop_offsets1.w;
}

fn stop_color(p: Primitive, index: i32) -> vec4<f32> {
    if (index == 0) { return p.stop_color0; }
    if (index == 1) { return p.stop_color1; }
    if (index == 2) { return p.stop_color2; }
    if (index == 3) { return p.stop_color3; }
    if (index == 4) { return p.stop_color4; }
    if (index == 5) { return p.stop_color5; }
    if (index == 6) { return p.stop_color6; }
    return p.stop_color7;
}

fn sample_gradient_stops(p: Primitive, t_in: f32) -> vec4<f32> {
    let count = i32(clamp(round(p.paint.z), 0.0, 8.0));
    if (count <= 0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let t = clamp(t_in, 0.0, 1.0);
    if (t <= stop_offset(p, 0)) {
        return stop_color(p, 0);
    }
    for (var i = 1; i < 8; i = i + 1) {
        if (i < count && t <= stop_offset(p, i)) {
            let a_off = stop_offset(p, i - 1);
            let b_off = stop_offset(p, i);
            let span = max(b_off - a_off, 0.000001);
            let local_t = clamp((t - a_off) / span, 0.0, 1.0);
            let a = stop_color(p, i - 1);
            let b = stop_color(p, i);
            return a + (b - a) * local_t;
        }
    }
    return stop_color(p, max(count - 1, 0));
}

fn sample_shape_paint(p: Primitive, local: vec2<f32>) -> vec4<f32> {
    let paint_kind = i32(round(p.paint.x));
    if (paint_kind == 0) {
        return p.color;
    }

    let units = i32(round(p.paint.y));
    let min_p = p.paint_bounds.xy;
    let max_p = p.paint_bounds.zw;
    let size = max(max_p - min_p, vec2<f32>(0.0001, 0.0001));
    var sample_p = local;
    if (units == 0) {
        sample_p = (local - min_p) / size;
    }

    if (paint_kind == 1) {
        let start = p.gradient.xy;
        let end = p.gradient.zw;
        let dir = end - start;
        let len2 = max(dot(dir, dir), 0.000001);
        return sample_gradient_stops(p, dot(sample_p - start, dir) / len2);
    }

    if (paint_kind == 2) {
        let center = p.gradient.xy;
        var delta = sample_p - center;
        if (units == 0) {
            let aspect = select(1.0, size.y / size.x, size.x > size.y);
            delta.x = delta.x / max(aspect, 0.0001);
        }
        return sample_gradient_stops(p, length(delta) / max(p.gradient.z, 0.0001));
    }

    return p.color;
}

fn line_taper_pressure(p: Primitive, t: f32) -> f32 {
    var pressure = 1.0;
    if (p.line.z > 0.0001) {
        pressure = min(pressure, clamp(t / p.line.z, 0.0, 1.0));
    }
    if (p.line.w > 0.0001) {
        pressure = min(pressure, clamp((1.0 - t) / p.line.w, 0.0, 1.0));
    }
    return pressure;
}

fn primitive_coverage(p: Primitive, local: vec2<f32>) -> vec2<f32> {
    let shape_kind = i32(round(p.info.x));
    var coverage = 0.0;
    var replace = 0.0;

    if (shape_kind == 7) {
        coverage = 1.0;
        replace = 1.0;
    } else if (shape_kind == 1) {
        let dist = rounded_rect_sdf(local, p.shape, p.style.x);
        coverage = inside_coverage(dist);
    } else if (shape_kind == 2) {
        let outer = rounded_rect_sdf(local, p.shape, p.style.x);
        let sw = max(p.style.y, 0.0);
        let inner = rounded_rect_sdf(
            local,
            vec4<f32>(p.shape.x + sw, p.shape.y + sw, max(p.shape.z - sw * 2.0, 0.0), max(p.shape.w - sw * 2.0, 0.0)),
            max(p.style.x - sw, 0.0)
        );
        coverage = inside_coverage(outer) * (1.0 - inside_coverage(inner));
    } else if (shape_kind == 3) {
        let dist = length(local - p.shape.xy) - p.shape.z;
        coverage = inside_coverage(dist);
    } else if (shape_kind == 4) {
        let dist = length(local - p.shape.xy) - p.shape.z;
        let sw = max(p.style.y, 0.0);
        coverage = inside_coverage(dist) * (1.0 - inside_coverage(dist + sw));
    } else if (shape_kind == 5) {
        let dist = rounded_rect_sdf(local, p.shape, p.style.x);
        let outside = max(dist, 0.0);
        let blur = max(p.style.z, 1.0);
        let sigma = max(blur * 0.42, 1.0);
        let inside = smoothstep(0.0, blur * 0.7, max(-dist, 0.0));
        let outside_falloff = exp(-(outside * outside) / (2.0 * sigma * sigma));
        coverage = min(max(inside, outside_falloff), 0.86);
    } else if (shape_kind == 6) {
        let dist = length(local - p.shape.xy) - p.shape.z;
        let outside = max(dist, 0.0);
        let blur = max(p.style.z, 1.0);
        let sigma = max(blur * 0.42, 1.0);
        let inside = smoothstep(0.0, blur * 0.7, max(-dist, 0.0));
        let outside_falloff = exp(-(outside * outside) / (2.0 * sigma * sigma));
        coverage = min(max(inside, outside_falloff), 0.86);
    } else if (shape_kind == 8) {
        let ab = p.shape.zw - p.shape.xy;
        let h = clamp(dot(local - p.shape.xy, ab) / max(dot(ab, ab), 0.0001), 0.0, 1.0);
        let global_t = p.line.x + (p.line.y - p.line.x) * h;
        let pressure = line_taper_pressure(p, global_t);
        let half_width = max(p.style.y * pressure * 0.5, 0.01);
        let dist = segment_distance(local, p.shape.xy, p.shape.zw) - half_width;
        coverage = inside_coverage(dist);
    } else if (shape_kind == 9) {
        coverage = triangle_coverage(local, p.shape.xy, p.shape.zw, p.style.xy);
    }

    return vec2<f32>(clamp(coverage, 0.0, 1.0), replace);
}

@compute @workgroup_size(16, 16, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= u32(params.canvas.x) || y >= u32(params.canvas.y)) {
        return;
    }

    let pos = vec2<i32>(i32(x), i32(y));
    let px = f32(x) + 0.5;
    let py = f32(y) + 0.5;
    var out_color = textureLoad(base_tex, pos, 0);
    let primitive_count = u32(round(params.count.x));
    let tile_size = max(u32(round(params.count.y)), 1u);
    let tiles_x = max(u32(round(params.count.z)), 1u);
    let tiles_y = max(u32(round(params.count.w)), 1u);
    let tile_x = min(x / tile_size, tiles_x - 1u);
    let tile_y = min(y / tile_size, tiles_y - 1u);
    let tile = tile_range_buffer.items[tile_y * tiles_x + tile_x];
    let tile_start = tile.x;
    let tile_count = tile.y;
    let pick_mode = params.canvas.z > 0.5;

    for (var local_i = 0u; local_i < tile_count; local_i = local_i + 1u) {
        let primitive_index = tile_index_buffer.items[tile_start + local_i];
        if (primitive_index >= primitive_count) {
            continue;
        }
        let p = primitive_buffer.items[primitive_index];
        if (px < p.bounds.x || py < p.bounds.y || px >= p.bounds.x + p.bounds.z || py >= p.bounds.y + p.bounds.w) {
            continue;
        }
        let local = vec2<f32>(
            p.inv0.x * px + p.inv0.y * py + p.inv0.z,
            p.inv1.x * px + p.inv1.y * py + p.inv1.z
        );
        let cover_replace = primitive_coverage(p, local);
        let coverage = cover_replace.x;
        if (coverage <= 0.0) {
            continue;
        }
        if (pick_mode) {
            if (p.info.z > 0.5) {
                out_color = encode_pick_id(p.info.z);
            }
            continue;
        }
        let paint_color = sample_shape_paint(p, local);
        if (cover_replace.y > 0.5) {
            out_color = vec4<f32>(paint_color.rgb, paint_color.a * p.style.w);
        } else {
            out_color = blend_over(out_color, paint_color.rgb, paint_color.a * p.style.w * coverage, p.info.y);
        }
    }

    textureStore(out_tex, pos, out_color);
}
