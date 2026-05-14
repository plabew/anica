// =========================================
// =========================================
// src/ui/motionloom_templates.rs

// Keep layer templates on a single graph scaffold so future multi-effect appends
// can reuse the same `clip0 -> src -> ... -> out` structure consistently.
const LAYER_GRAPH_INPUTS: &str = "  <Input id=\"clip0\" type=\"video\" from=\"input:clip0\" />\n  <Tex id=\"src\" fmt=\"rgba16f\" from=\"clip0\" />\n  <Tex id=\"out\" fmt=\"rgba16f\" size={[1920,1080]} />\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerEffectTemplateKind {
    BlurGaussian,
    Sharpen,
    Opacity,
    Lut,
    HslaOverlay,
    TransitionFadeInOut,
}

pub const DEFAULT_GRAPH_SCRIPT: &str = r#"<Graph scope="clip" fps={60} size={[1920,1080]}>
  <Input id="clip0" type="video" from="input:clip0" />
  <Tex id="src" fmt="rgba16f" from="clip0" />
  <Tex id="out" fmt="rgba16f" size={[1920,1080]} />
  <Pass id="fx_blur" kind="compute" effect="gaussian_5tap_h"
        in={["src"]} out={["out"]}
        params={{ sigma: "10.0" }} />
  <Present from="out" />
</Graph>"#;

pub const DEFAULT_SCENE_SCRIPT: &str = r##"<Graph scope="scene" fps={60} duration="3s" size={[1920,1080]}>
  <Solid color="#000000" />
  <Text value="hello world"
        x="center"
        y="center"
        fontSize="96"
        color="#ffffff"
        opacity="min($time.sec / 1.0, 1.0)" />
  <Present from="scene" />
</Graph>"##;

fn begin_layer_graph(add_time_parameter: bool) -> String {
    let mut script = String::new();
    if add_time_parameter {
        script.push_str(
            "<Graph scope=\"layer\" fps={60} apply=\"graph\" duration=\"5s\" size={[1920,1080]}>\n",
        );
    } else {
        script.push_str("<Graph scope=\"layer\" fps={60} size={[1920,1080]}>\n");
    }
    script.push_str(LAYER_GRAPH_INPUTS);
    script
}

fn finish_graph(script: &mut String) {
    script.push_str("  <Present from=\"out\" />\n");
    script.push_str("</Graph>");
}

fn build_layer_effect_pass(
    kind: LayerEffectTemplateKind,
    input_tex: &str,
    output_tex: &str,
    add_curve_parameter: bool,
) -> String {
    match kind {
        LayerEffectTemplateKind::BlurGaussian => {
            blur_gaussian_pass(input_tex, output_tex, add_curve_parameter)
        }
        LayerEffectTemplateKind::Sharpen => {
            sharpen_pass(input_tex, output_tex, add_curve_parameter)
        }
        LayerEffectTemplateKind::Opacity => {
            opacity_pass(input_tex, output_tex, add_curve_parameter)
        }
        LayerEffectTemplateKind::Lut => lut_pass(input_tex, output_tex, add_curve_parameter),
        LayerEffectTemplateKind::HslaOverlay => {
            hsla_overlay_pass(input_tex, output_tex, add_curve_parameter)
        }
        LayerEffectTemplateKind::TransitionFadeInOut => String::new(),
    }
}

fn build_transition_passes(
    input_tex: &str,
    output_tex: &str,
    tmp_tex: &str,
    suffix: usize,
    add_curve_parameter: bool,
) -> String {
    let mut script = String::new();
    script.push_str(&format!(
        "  <Pass id=\"fade_in_{suffix}\" kind=\"render\" role=\"transition\"\n"
    ));
    script.push_str("        effect=\"fade_in\"\n");
    script.push_str(&format!(
        "        in={{[\"{input_tex}\"]}} out={{[\"{tmp_tex}\"]}}\n"
    ));
    script.push_str("        params={{\n");
    script.push_str("          durationSec: \"2.0\"");
    if add_curve_parameter {
        script.push_str(",\n");
        script.push_str("          opacity: curve(\"0.00:0.0:linear, 2.00:1.0:ease_in_out\")\n");
    } else {
        script.push('\n');
    }
    script.push_str("        }} />\n");
    script.push_str(&format!(
        "  <Pass id=\"fade_out_{suffix}\" kind=\"render\" role=\"transition\"\n"
    ));
    script.push_str("        effect=\"fade_out\"\n");
    script.push_str(&format!(
        "        in={{[\"{tmp_tex}\"]}} out={{[\"{output_tex}\"]}}\n"
    ));
    script.push_str("        params={{\n");
    script.push_str("          durationSec: \"2.0\"");
    if add_curve_parameter {
        script.push_str(",\n");
        script.push_str("          opacity: curve(\"0.00:1.0:linear, 2.00:0.0:ease_in_out\")\n");
    } else {
        script.push('\n');
    }
    script.push_str("        }} />\n");
    script
}

fn extract_attr_value(block: &str, start_marker: &str, end_marker: &str) -> Option<String> {
    let start = block.find(start_marker)? + start_marker.len();
    let end = block[start..].find(end_marker)? + start;
    Some(block[start..end].to_string())
}

fn replace_range_with(
    existing_script: &str,
    start_idx: usize,
    end_idx: usize,
    replacement: &str,
) -> String {
    let mut updated = existing_script.to_string();
    updated.replace_range(start_idx..end_idx, replacement);
    updated
}

// Build multiple selected effect templates into a single chained layer graph so
// the picker can confirm several selections at once instead of inserting them one by one.
pub fn build_layer_effect_chain_script(
    kinds: &[LayerEffectTemplateKind],
    add_time_parameter: bool,
    add_curve_parameter: bool,
) -> Option<String> {
    let mut script = begin_layer_graph(add_time_parameter);
    let mut tex_defs = String::new();
    let mut pass_defs = String::new();
    let mut input_tex = "src".to_string();
    let mut stage_idx = 0usize;
    let mut transition_idx = 0usize;
    for (idx, kind) in kinds.iter().enumerate() {
        let output_tex = if idx + 1 == kinds.len() {
            "out".to_string()
        } else {
            stage_idx += 1;
            let stage_tex = format!("stage{stage_idx}");
            tex_defs.push_str(&format!(
                "  <Tex id=\"{stage_tex}\" fmt=\"rgba16f\" size={{[1920,1080]}} />\n"
            ));
            stage_tex
        };
        if *kind == LayerEffectTemplateKind::TransitionFadeInOut {
            transition_idx += 1;
            let tmp_tex = format!("transition_tmp{transition_idx}");
            tex_defs.push_str(&format!(
                "  <Tex id=\"{tmp_tex}\" fmt=\"rgba16f\" size={{[1920,1080]}} />\n"
            ));
            pass_defs.push_str(&build_transition_passes(
                &input_tex,
                &output_tex,
                &tmp_tex,
                transition_idx,
                add_curve_parameter,
            ));
        } else {
            pass_defs.push_str(&build_layer_effect_pass(
                *kind,
                &input_tex,
                &output_tex,
                add_curve_parameter,
            ));
        }
        input_tex = output_tex;
    }
    script.push_str(&tex_defs);
    script.push_str(&pass_defs);
    finish_graph(&mut script);
    Some(script)
}

// Append a new effect pass by rerouting the previous terminal pass into a fresh
// stage texture, then placing the new pass at the end of the graph.
pub fn append_layer_effect_template_script(
    existing_script: &str,
    kind: LayerEffectTemplateKind,
    add_curve_parameter: bool,
) -> Option<String> {
    match kind {
        LayerEffectTemplateKind::BlurGaussian => {
            if let Some(start_idx) = existing_script.find("  <Pass id=\"fx_blur\"") {
                let rel_end = existing_script[start_idx..].find("/>\n")?;
                let end_idx = start_idx + rel_end + 3;
                let block = &existing_script[start_idx..end_idx];
                let input_tex = extract_attr_value(block, "in={[\"", "\"]}")?;
                let output_tex = extract_attr_value(block, "out={[\"", "\"]}")?;
                return Some(replace_range_with(
                    existing_script,
                    start_idx,
                    end_idx,
                    &blur_gaussian_pass(&input_tex, &output_tex, add_curve_parameter),
                ));
            }
        }
        LayerEffectTemplateKind::Sharpen => {
            if let Some(start_idx) = existing_script.find("  <Pass id=\"fx_sharpen\"") {
                let rel_end = existing_script[start_idx..].find("/>\n")?;
                let end_idx = start_idx + rel_end + 3;
                let block = &existing_script[start_idx..end_idx];
                let input_tex = extract_attr_value(block, "in={[\"", "\"]}")?;
                let output_tex = extract_attr_value(block, "out={[\"", "\"]}")?;
                return Some(replace_range_with(
                    existing_script,
                    start_idx,
                    end_idx,
                    &sharpen_pass(&input_tex, &output_tex, add_curve_parameter),
                ));
            }
        }
        LayerEffectTemplateKind::Opacity => {
            if let Some(start_idx) = existing_script.find("  <Pass id=\"fx_opacity\"") {
                let rel_end = existing_script[start_idx..].find("/>\n")?;
                let end_idx = start_idx + rel_end + 3;
                let block = &existing_script[start_idx..end_idx];
                let input_tex = extract_attr_value(block, "in={[\"", "\"]}")?;
                let output_tex = extract_attr_value(block, "out={[\"", "\"]}")?;
                return Some(replace_range_with(
                    existing_script,
                    start_idx,
                    end_idx,
                    &opacity_pass(&input_tex, &output_tex, add_curve_parameter),
                ));
            }
        }
        LayerEffectTemplateKind::Lut => {
            if let Some(start_idx) = existing_script.find("  <Pass id=\"fx_lut\"") {
                let rel_end = existing_script[start_idx..].find("/>\n")?;
                let end_idx = start_idx + rel_end + 3;
                let block = &existing_script[start_idx..end_idx];
                let input_tex = extract_attr_value(block, "in={[\"", "\"]}")?;
                let output_tex = extract_attr_value(block, "out={[\"", "\"]}")?;
                return Some(replace_range_with(
                    existing_script,
                    start_idx,
                    end_idx,
                    &lut_pass(&input_tex, &output_tex, add_curve_parameter),
                ));
            }
        }
        LayerEffectTemplateKind::HslaOverlay => {
            if let Some(start_idx) = existing_script.find("  <Pass id=\"fx_hsla_overlay\"") {
                let rel_end = existing_script[start_idx..].find("/>\n")?;
                let end_idx = start_idx + rel_end + 3;
                let block = &existing_script[start_idx..end_idx];
                let input_tex = extract_attr_value(block, "in={[\"", "\"]}")?;
                let output_tex = extract_attr_value(block, "out={[\"", "\"]}")?;
                return Some(replace_range_with(
                    existing_script,
                    start_idx,
                    end_idx,
                    &hsla_overlay_pass(&input_tex, &output_tex, add_curve_parameter),
                ));
            }
        }
        LayerEffectTemplateKind::TransitionFadeInOut => {
            if let Some(start_idx) = existing_script.find("  <Pass id=\"fade_in_") {
                let fade_in_rel_end = existing_script[start_idx..].find("/>\n")?;
                let fade_in_end = start_idx + fade_in_rel_end + 3;
                let fade_in_block = &existing_script[start_idx..fade_in_end];
                let suffix = extract_attr_value(fade_in_block, "id=\"fade_in_", "\"")?;
                let fade_out_marker = format!("  <Pass id=\"fade_out_{suffix}\"");
                let fade_out_start = existing_script.find(&fade_out_marker)?;
                let fade_out_rel_end = existing_script[fade_out_start..].find("/>\n")?;
                let fade_out_end = fade_out_start + fade_out_rel_end + 3;
                let fade_out_block = &existing_script[fade_out_start..fade_out_end];
                let input_tex = extract_attr_value(fade_in_block, "in={[\"", "\"]}")?;
                let tmp_tex = extract_attr_value(fade_in_block, "out={[\"", "\"]}")?;
                let output_tex = extract_attr_value(fade_out_block, "out={[\"", "\"]}")?;
                let replacement = build_transition_passes(
                    &input_tex,
                    &output_tex,
                    &tmp_tex,
                    suffix.parse().ok()?,
                    add_curve_parameter,
                );
                let replace_end = fade_out_end.max(fade_in_end);
                return Some(replace_range_with(
                    existing_script,
                    start_idx,
                    replace_end,
                    &replacement,
                ));
            }
        }
    }

    if !existing_script.contains("<Graph scope=\"layer\"")
        || !existing_script.contains("<Tex id=\"src\"")
        || !existing_script.contains("<Tex id=\"out\"")
    {
        return None;
    }

    let present_idx = existing_script.find("  <Present from=\"out\" />")?;
    let first_pass_idx = existing_script.find("  <Pass ")?;
    let pass_count = existing_script.matches("\n  <Pass ").count();

    if pass_count == 0 {
        let mut updated = existing_script.to_string();
        if kind == LayerEffectTemplateKind::TransitionFadeInOut {
            let tmp_tex = "transition_tmp1";
            let insert_tex_at = updated.find("  <Present from=\"out\" />")?;
            updated.insert_str(
                insert_tex_at,
                &format!("  <Tex id=\"{tmp_tex}\" fmt=\"rgba16f\" size={{[1920,1080]}} />\n"),
            );
            let insert_at = updated.find("  <Present from=\"out\" />")?;
            updated.insert_str(
                insert_at,
                &build_transition_passes("src", "out", tmp_tex, 1, add_curve_parameter),
            );
        } else {
            updated.insert_str(
                present_idx,
                &build_layer_effect_pass(kind, "src", "out", add_curve_parameter),
            );
        }
        return Some(updated);
    }

    let mut stage_idx = 1usize;
    while existing_script.contains(&format!("id=\"stage{stage_idx}\"")) {
        stage_idx += 1;
    }
    let stage_tex = format!("stage{stage_idx}");
    let last_out_marker = "out={[\"out\"]}";
    let (last_out_idx, _) = existing_script.rmatch_indices(last_out_marker).next()?;

    let mut updated = existing_script.to_string();
    let mut inserted_defs =
        format!("  <Tex id=\"{stage_tex}\" fmt=\"rgba16f\" size={{[1920,1080]}} />\n");
    let transition_suffix = if kind == LayerEffectTemplateKind::TransitionFadeInOut {
        let mut transition_idx = 1usize;
        while existing_script.contains(&format!("id=\"transition_tmp{transition_idx}\"")) {
            transition_idx += 1;
        }
        inserted_defs.push_str(&format!(
            "  <Tex id=\"transition_tmp{transition_idx}\" fmt=\"rgba16f\" size={{[1920,1080]}} />\n"
        ));
        Some(transition_idx)
    } else {
        None
    };
    // Declare intermediate stage textures before pass nodes so resources stay grouped.
    updated.insert_str(first_pass_idx, &inserted_defs);
    let adjusted_last_out_idx = last_out_idx + inserted_defs.len();
    updated.replace_range(
        adjusted_last_out_idx..adjusted_last_out_idx + last_out_marker.len(),
        &format!("out={{[\"{stage_tex}\"]}}"),
    );

    let insert_at = updated.find("  <Present from=\"out\" />")?;
    if let Some(transition_idx) = transition_suffix {
        let tmp_tex = format!("transition_tmp{transition_idx}");
        updated.insert_str(
            insert_at,
            &build_transition_passes(
                &stage_tex,
                "out",
                &tmp_tex,
                transition_idx,
                add_curve_parameter,
            ),
        );
    } else {
        updated.insert_str(
            insert_at,
            &build_layer_effect_pass(kind, &stage_tex, "out", add_curve_parameter),
        );
    }
    Some(updated)
}

// Append a whole selection of effect templates into an existing standardized layer graph.
pub fn append_layer_effect_template_chain_script(
    existing_script: &str,
    kinds: &[LayerEffectTemplateKind],
    add_curve_parameter: bool,
) -> Option<String> {
    let mut updated = existing_script.to_string();
    for kind in kinds {
        updated = append_layer_effect_template_script(&updated, *kind, add_curve_parameter)?;
    }
    Some(updated)
}

// Build pass blocks against explicit input/output textures so the same pass text
// can later be reused for stacked effect graphs.
fn blur_gaussian_pass(input_tex: &str, output_tex: &str, add_curve_parameter: bool) -> String {
    let mut script = String::new();
    script.push_str("  <Pass id=\"fx_blur\" kind=\"compute\"\n");
    script.push_str("        effect=\"gaussian_5tap_h\"\n");
    script.push_str(&format!(
        "        in={{[\"{input_tex}\"]}} out={{[\"{output_tex}\"]}}\n"
    ));
    if add_curve_parameter {
        script.push_str(
            "        params={{\n          sigma: curve(\"0.00:2.0:linear, 2.00:10.0:ease_in_out\") // range: 0.0-64.0\n        }} />\n",
        );
    } else {
        script.push_str(
            "        params={{\n          sigma: \"10.0\" // range: 0.0-64.0\n        }} />\n",
        );
    }
    script
}

fn sharpen_pass(input_tex: &str, output_tex: &str, add_curve_parameter: bool) -> String {
    let mut script = String::new();
    script.push_str("  <Pass id=\"fx_sharpen\" kind=\"compute\"\n");
    script.push_str("        effect=\"sharpen\"\n");
    script.push_str(&format!(
        "        in={{[\"{input_tex}\"]}} out={{[\"{output_tex}\"]}}\n"
    ));
    if add_curve_parameter {
        script.push_str(
            "        params={{\n          sigma: curve(\"0.00:0.0:linear, 2.00:22.0:ease_in_out\") // range: 0.0-64.0\n        }} />\n",
        );
    } else {
        script.push_str(
            "        params={{\n          sigma: \"22.0\" // range: 0.0-64.0\n        }} />\n",
        );
    }
    script
}

fn opacity_pass(input_tex: &str, output_tex: &str, add_curve_parameter: bool) -> String {
    let mut script = String::new();
    script.push_str("  <Pass id=\"fx_opacity\" kind=\"compute\"\n");
    script.push_str("        effect=\"opacity\"\n");
    script.push_str(&format!(
        "        in={{[\"{input_tex}\"]}} out={{[\"{output_tex}\"]}}\n"
    ));
    if add_curve_parameter {
        script.push_str(
            "        params={{ opacity: curve(\"0.00:1.0:linear, 2.00:0.7:ease_in_out\") // range: 0.0-1.0 }} />\n",
        );
    } else {
        script.push_str("        params={{ opacity: \"0.7\" // range: 0.0-1.0 }} />\n");
    }
    script
}

fn lut_pass(input_tex: &str, output_tex: &str, add_curve_parameter: bool) -> String {
    let mut script = String::new();
    script.push_str("  <Pass id=\"fx_lut\" kind=\"compute\"\n");
    script.push_str("        effect=\"lut\"\n");
    script.push_str(&format!(
        "        in={{[\"{input_tex}\"]}} out={{[\"{output_tex}\"]}}\n"
    ));
    if add_curve_parameter {
        script.push_str(
            "        params={{ mix: curve(\"0.00:0.0:linear, 2.00:0.6:ease_in_out\") // range: 0.0-1.0 }} />\n",
        );
    } else {
        script.push_str("        params={{ mix: \"0.6\" // range: 0.0-1.0 }} />\n");
    }
    script
}

fn hsla_overlay_pass(input_tex: &str, output_tex: &str, add_curve_parameter: bool) -> String {
    let mut script = String::new();
    script.push_str("  <Pass id=\"fx_hsla_overlay\" kind=\"compute\"\n");
    script.push_str("        effect=\"hsla_overlay\"\n");
    script.push_str(&format!(
        "        in={{[\"{input_tex}\"]}} out={{[\"{output_tex}\"]}}\n"
    ));
    if add_curve_parameter {
        script.push_str(
            "        params={{\n          hue: \"210.0\", // range: 0.0-360.0 (degrees)\n          saturation: \"0.70\", // range: 0.0-1.0\n          lightness: \"0.41\", // range: 0.0-1.0\n          alpha: curve(\"0.00:0.0:linear, 2.00:0.45:ease_in_out\") // range: 0.0-1.0\n        }} />\n",
        );
    } else {
        script.push_str(
            "        params={{\n          hue: \"210.0\", // range: 0.0-360.0 (degrees)\n          saturation: \"0.70\", // range: 0.0-1.0\n          lightness: \"0.41\", // range: 0.0-1.0\n          alpha: \"0.45\" // range: 0.0-1.0\n        }} />\n",
        );
    }
    script
}
