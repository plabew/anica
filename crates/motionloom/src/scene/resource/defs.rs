use std::collections::HashMap;

use crate::dsl::GraphScript;
use crate::scene::model::{
    FilterDef, FontDef, GradientDef, MaskNode, PaletteNode, PrecomposeNode, SceneNode,
};

pub(crate) fn collect_graph_gradient_defs(
    graph: &GraphScript,
    out: &mut HashMap<String, GradientDef>,
) {
    collect_scene_gradient_defs(&graph.scene_nodes, out);
    for scene in &graph.scenes {
        collect_scene_gradient_defs(&scene.children, out);
    }
}

pub(crate) fn collect_graph_palette_defs(
    graph: &GraphScript,
    out: &mut HashMap<String, PaletteNode>,
) {
    collect_scene_palette_defs(&graph.scene_nodes, out);
    for scene in &graph.scenes {
        collect_scene_palette_defs(&scene.children, out);
    }
}

pub(crate) fn collect_graph_font_defs(graph: &GraphScript, out: &mut HashMap<String, FontDef>) {
    collect_scene_font_defs(&graph.scene_nodes, out);
    for scene in &graph.scenes {
        collect_scene_font_defs(&scene.children, out);
    }
}

pub(crate) fn collect_graph_filter_defs(graph: &GraphScript, out: &mut HashMap<String, FilterDef>) {
    collect_scene_filter_defs(&graph.scene_nodes, out);
    for scene in &graph.scenes {
        collect_scene_filter_defs(&scene.children, out);
    }
}

pub(crate) fn collect_graph_component_defs(
    graph: &GraphScript,
    out: &mut HashMap<String, Vec<SceneNode>>,
) {
    collect_scene_component_defs(&graph.scene_nodes, out);
    for scene in &graph.scenes {
        collect_scene_component_defs(&scene.children, out);
    }
}

pub(crate) fn collect_graph_mask_defs(graph: &GraphScript, out: &mut HashMap<String, MaskNode>) {
    collect_scene_mask_defs(&graph.scene_nodes, out);
    for scene in &graph.scenes {
        collect_scene_mask_defs(&scene.children, out);
    }
}

pub(crate) fn collect_graph_precompose_defs(graph: &GraphScript) -> Vec<PrecomposeNode> {
    let mut out = Vec::new();
    collect_scene_precompose_defs(&graph.scene_nodes, &mut out);
    for scene in &graph.scenes {
        collect_scene_precompose_defs(&scene.children, &mut out);
    }
    out
}

fn collect_scene_gradient_defs(nodes: &[SceneNode], out: &mut HashMap<String, GradientDef>) {
    for node in nodes {
        match node {
            SceneNode::Defs(defs) => {
                for gradient in &defs.gradients {
                    let id = match gradient {
                        GradientDef::Linear(linear) => &linear.id,
                        GradientDef::Radial(radial) => &radial.id,
                    };
                    out.insert(id.clone(), gradient.clone());
                }
                for mask in &defs.masks {
                    collect_scene_gradient_defs(&mask.children, out);
                }
                for precompose in &defs.precomposes {
                    collect_scene_gradient_defs(&precompose.children, out);
                }
                for component in &defs.components {
                    collect_scene_gradient_defs(&component.children, out);
                }
            }
            SceneNode::Timeline(timeline) => collect_scene_gradient_defs(&timeline.children, out),
            SceneNode::Track(track) => collect_scene_gradient_defs(&track.children, out),
            SceneNode::Sequence(sequence) => collect_scene_gradient_defs(&sequence.children, out),
            SceneNode::Chain(chain) => collect_scene_gradient_defs(&chain.children, out),
            SceneNode::Group(group) => collect_scene_gradient_defs(&group.children, out),
            SceneNode::Part(part) => collect_scene_gradient_defs(&part.children, out),
            SceneNode::Repeat(repeat) => collect_scene_gradient_defs(&repeat.children, out),
            SceneNode::Mask(mask) => collect_scene_gradient_defs(&mask.children, out),
            SceneNode::Precompose(precompose) => {
                collect_scene_gradient_defs(&precompose.children, out)
            }
            SceneNode::Layer(layer) => collect_scene_gradient_defs(&layer.children, out),
            SceneNode::Camera(camera) => collect_scene_gradient_defs(&camera.children, out),
            SceneNode::Character(character) => {
                collect_scene_gradient_defs(&character.children, out)
            }
            _ => {}
        }
    }
}

fn collect_scene_palette_defs(nodes: &[SceneNode], out: &mut HashMap<String, PaletteNode>) {
    for node in nodes {
        match node {
            SceneNode::Palette(palette) => {
                out.insert(palette.id.clone(), palette.clone());
            }
            SceneNode::Defs(defs) => {
                for palette in &defs.palettes {
                    out.insert(palette.id.clone(), palette.clone());
                }
                for mask in &defs.masks {
                    collect_scene_palette_defs(&mask.children, out);
                }
                for precompose in &defs.precomposes {
                    collect_scene_palette_defs(&precompose.children, out);
                }
                for component in &defs.components {
                    collect_scene_palette_defs(&component.children, out);
                }
            }
            SceneNode::Timeline(timeline) => collect_scene_palette_defs(&timeline.children, out),
            SceneNode::Track(track) => collect_scene_palette_defs(&track.children, out),
            SceneNode::Sequence(sequence) => collect_scene_palette_defs(&sequence.children, out),
            SceneNode::Chain(chain) => collect_scene_palette_defs(&chain.children, out),
            SceneNode::Group(group) => collect_scene_palette_defs(&group.children, out),
            SceneNode::Part(part) => collect_scene_palette_defs(&part.children, out),
            SceneNode::Repeat(repeat) => collect_scene_palette_defs(&repeat.children, out),
            SceneNode::Mask(mask) => collect_scene_palette_defs(&mask.children, out),
            SceneNode::Precompose(precompose) => {
                collect_scene_palette_defs(&precompose.children, out)
            }
            SceneNode::Layer(layer) => collect_scene_palette_defs(&layer.children, out),
            SceneNode::Camera(camera) => collect_scene_palette_defs(&camera.children, out),
            SceneNode::Character(character) => collect_scene_palette_defs(&character.children, out),
            _ => {}
        }
    }
}

fn collect_scene_font_defs(nodes: &[SceneNode], out: &mut HashMap<String, FontDef>) {
    for node in nodes {
        match node {
            SceneNode::Defs(defs) => {
                for font in &defs.fonts {
                    out.insert(font.id.clone(), font.clone());
                }
                for mask in &defs.masks {
                    collect_scene_font_defs(&mask.children, out);
                }
                for precompose in &defs.precomposes {
                    collect_scene_font_defs(&precompose.children, out);
                }
                for component in &defs.components {
                    collect_scene_font_defs(&component.children, out);
                }
            }
            SceneNode::Timeline(timeline) => collect_scene_font_defs(&timeline.children, out),
            SceneNode::Track(track) => collect_scene_font_defs(&track.children, out),
            SceneNode::Sequence(sequence) => collect_scene_font_defs(&sequence.children, out),
            SceneNode::Chain(chain) => collect_scene_font_defs(&chain.children, out),
            SceneNode::Group(group) => collect_scene_font_defs(&group.children, out),
            SceneNode::Part(part) => collect_scene_font_defs(&part.children, out),
            SceneNode::Repeat(repeat) => collect_scene_font_defs(&repeat.children, out),
            SceneNode::Mask(mask) => collect_scene_font_defs(&mask.children, out),
            SceneNode::Precompose(precompose) => collect_scene_font_defs(&precompose.children, out),
            SceneNode::Layer(layer) => collect_scene_font_defs(&layer.children, out),
            SceneNode::Camera(camera) => collect_scene_font_defs(&camera.children, out),
            SceneNode::Character(character) => collect_scene_font_defs(&character.children, out),
            _ => {}
        }
    }
}

fn collect_scene_filter_defs(nodes: &[SceneNode], out: &mut HashMap<String, FilterDef>) {
    for node in nodes {
        match node {
            SceneNode::Defs(defs) => {
                for filter in &defs.filters {
                    out.insert(filter.id.clone(), filter.clone());
                }
                for mask in &defs.masks {
                    collect_scene_filter_defs(&mask.children, out);
                }
                for precompose in &defs.precomposes {
                    collect_scene_filter_defs(&precompose.children, out);
                }
                for component in &defs.components {
                    collect_scene_filter_defs(&component.children, out);
                }
            }
            SceneNode::Timeline(timeline) => collect_scene_filter_defs(&timeline.children, out),
            SceneNode::Track(track) => collect_scene_filter_defs(&track.children, out),
            SceneNode::Sequence(sequence) => collect_scene_filter_defs(&sequence.children, out),
            SceneNode::Chain(chain) => collect_scene_filter_defs(&chain.children, out),
            SceneNode::Group(group) => collect_scene_filter_defs(&group.children, out),
            SceneNode::Part(part) => collect_scene_filter_defs(&part.children, out),
            SceneNode::Repeat(repeat) => collect_scene_filter_defs(&repeat.children, out),
            SceneNode::Mask(mask) => collect_scene_filter_defs(&mask.children, out),
            SceneNode::Precompose(precompose) => {
                collect_scene_filter_defs(&precompose.children, out)
            }
            SceneNode::Layer(layer) => collect_scene_filter_defs(&layer.children, out),
            SceneNode::Camera(camera) => collect_scene_filter_defs(&camera.children, out),
            SceneNode::Character(character) => collect_scene_filter_defs(&character.children, out),
            _ => {}
        }
    }
}

fn collect_scene_component_defs(nodes: &[SceneNode], out: &mut HashMap<String, Vec<SceneNode>>) {
    for node in nodes {
        match node {
            SceneNode::Defs(defs) => {
                for component in &defs.components {
                    out.insert(component.id.clone(), component.children.clone());
                    collect_scene_component_defs(&component.children, out);
                }
                for mask in &defs.masks {
                    collect_scene_component_defs(&mask.children, out);
                }
                for precompose in &defs.precomposes {
                    collect_scene_component_defs(&precompose.children, out);
                }
            }
            SceneNode::Timeline(timeline) => collect_scene_component_defs(&timeline.children, out),
            SceneNode::Track(track) => collect_scene_component_defs(&track.children, out),
            SceneNode::Sequence(sequence) => collect_scene_component_defs(&sequence.children, out),
            SceneNode::Chain(chain) => collect_scene_component_defs(&chain.children, out),
            SceneNode::Group(group) => collect_scene_component_defs(&group.children, out),
            SceneNode::Part(part) => collect_scene_component_defs(&part.children, out),
            SceneNode::Repeat(repeat) => collect_scene_component_defs(&repeat.children, out),
            SceneNode::Mask(mask) => collect_scene_component_defs(&mask.children, out),
            SceneNode::Precompose(precompose) => {
                collect_scene_component_defs(&precompose.children, out)
            }
            SceneNode::Layer(layer) => collect_scene_component_defs(&layer.children, out),
            SceneNode::Camera(camera) => collect_scene_component_defs(&camera.children, out),
            SceneNode::Character(character) => {
                collect_scene_component_defs(&character.children, out)
            }
            _ => {}
        }
    }
}

fn collect_scene_mask_defs(nodes: &[SceneNode], out: &mut HashMap<String, MaskNode>) {
    for node in nodes {
        match node {
            SceneNode::Defs(defs) => {
                for mask in &defs.masks {
                    if let Some(id) = mask.id.as_deref() {
                        out.insert(id.to_string(), mask.clone());
                    }
                    collect_scene_mask_defs(&mask.children, out);
                }
                for precompose in &defs.precomposes {
                    collect_scene_mask_defs(&precompose.children, out);
                }
                for component in &defs.components {
                    collect_scene_mask_defs(&component.children, out);
                }
            }
            SceneNode::Timeline(timeline) => collect_scene_mask_defs(&timeline.children, out),
            SceneNode::Track(track) => collect_scene_mask_defs(&track.children, out),
            SceneNode::Sequence(sequence) => collect_scene_mask_defs(&sequence.children, out),
            SceneNode::Chain(chain) => collect_scene_mask_defs(&chain.children, out),
            SceneNode::Group(group) => collect_scene_mask_defs(&group.children, out),
            SceneNode::Part(part) => collect_scene_mask_defs(&part.children, out),
            SceneNode::Repeat(repeat) => collect_scene_mask_defs(&repeat.children, out),
            SceneNode::Mask(mask) => collect_scene_mask_defs(&mask.children, out),
            SceneNode::Precompose(precompose) => collect_scene_mask_defs(&precompose.children, out),
            SceneNode::Layer(layer) => collect_scene_mask_defs(&layer.children, out),
            SceneNode::Camera(camera) => collect_scene_mask_defs(&camera.children, out),
            SceneNode::Character(character) => collect_scene_mask_defs(&character.children, out),
            _ => {}
        }
    }
}

fn collect_scene_precompose_defs(nodes: &[SceneNode], out: &mut Vec<PrecomposeNode>) {
    for node in nodes {
        match node {
            SceneNode::Defs(defs) => {
                for precompose in &defs.precomposes {
                    out.push(precompose.clone());
                    collect_scene_precompose_defs(&precompose.children, out);
                }
                for mask in &defs.masks {
                    collect_scene_precompose_defs(&mask.children, out);
                }
                for component in &defs.components {
                    collect_scene_precompose_defs(&component.children, out);
                }
            }
            SceneNode::Timeline(timeline) => collect_scene_precompose_defs(&timeline.children, out),
            SceneNode::Track(track) => collect_scene_precompose_defs(&track.children, out),
            SceneNode::Sequence(sequence) => collect_scene_precompose_defs(&sequence.children, out),
            SceneNode::Chain(chain) => collect_scene_precompose_defs(&chain.children, out),
            SceneNode::Group(group) => collect_scene_precompose_defs(&group.children, out),
            SceneNode::Part(part) => collect_scene_precompose_defs(&part.children, out),
            SceneNode::Repeat(repeat) => collect_scene_precompose_defs(&repeat.children, out),
            SceneNode::Mask(mask) => collect_scene_precompose_defs(&mask.children, out),
            SceneNode::Precompose(precompose) => {
                collect_scene_precompose_defs(&precompose.children, out)
            }
            SceneNode::Layer(layer) => collect_scene_precompose_defs(&layer.children, out),
            SceneNode::Camera(camera) => collect_scene_precompose_defs(&camera.children, out),
            SceneNode::Character(character) => {
                collect_scene_precompose_defs(&character.children, out)
            }
            _ => {}
        }
    }
}
