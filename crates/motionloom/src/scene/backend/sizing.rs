use image::{Rgba, RgbaImage, imageops::FilterType};

use crate::dsl::GraphScript;
use crate::scene::composition::draw_rgba_image;
use crate::scene::spatial::Affine2;

pub(crate) fn format_scene_number(value: f32) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let mut text = format!("{value:.4}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" {
        text = "0".to_string();
    }
    text
}

pub(crate) fn graph_logical_render_size(graph: &GraphScript) -> (u32, u32) {
    graph.size
}

pub(crate) fn graph_output_size(graph: &GraphScript) -> (u32, u32) {
    graph.render_size.unwrap_or(graph.size)
}

pub(crate) fn render_size_root_transform(
    output_size: (u32, u32),
    logical_size: (u32, u32),
) -> Affine2 {
    let output_w = output_size.0.max(1) as f32;
    let output_h = output_size.1.max(1) as f32;
    let logical_w = logical_size.0.max(1) as f32;
    let logical_h = logical_size.1.max(1) as f32;
    let scale = (output_w / logical_w).min(output_h / logical_h);
    let x = (output_w - logical_w * scale) * 0.5;
    let y = (output_h - logical_h * scale) * 0.5;
    Affine2::translate(x, y).mul(Affine2::scale(scale))
}

pub(crate) fn fit_logical_canvas_to_output(
    image: &RgbaImage,
    output_size: (u32, u32),
) -> RgbaImage {
    let output_w = output_size.0.max(1);
    let output_h = output_size.1.max(1);
    if image.width() == output_w && image.height() == output_h {
        return image.clone();
    }

    let logical_w = image.width().max(1) as f32;
    let logical_h = image.height().max(1) as f32;
    let scale = (output_w as f32 / logical_w).min(output_h as f32 / logical_h);
    let target_w = (logical_w * scale).round().max(1.0) as u32;
    let target_h = (logical_h * scale).round().max(1.0) as u32;
    let x = (output_w as f32 - target_w as f32) * 0.5;
    let y = (output_h as f32 - target_h as f32) * 0.5;
    let mut output = RgbaImage::from_pixel(output_w, output_h, Rgba([0, 0, 0, 255]));
    let scaled = image::imageops::resize(image, target_w, target_h, FilterType::Lanczos3);
    draw_rgba_image(&mut output, &scaled, x, y, 1.0);
    output
}
