// =========================================
// =========================================
// src/ui/inspector_panel/semantic_layer.rs

use super::*;

impl InspectorPanel {
    pub(super) fn semantic_video_provider_uses_1280x720(provider: &str, mode: &str) -> bool {
        if Self::normalize_semantic_schema_mode(mode) != "video" {
            return false;
        }
        let provider = provider.trim().to_ascii_lowercase();
        matches!(
            provider.as_str(),
            "veo_3_1"
                | "google/veo_3_1"
                | "google/veo-3.1-generate-preview"
                | "openai/sora-2"
                | "sora-2"
                | "sora_2"
                | "openai/sora-2-pro"
                | "sora-2-pro"
                | "sora_2_pro"
        )
    }

    pub(super) fn semantic_default_video_size_for_provider(
        provider: &str,
        mode: &str,
    ) -> Option<(u32, u32)> {
        if Self::semantic_video_provider_uses_1280x720(provider, mode) {
            Some((1280, 720))
        } else {
            None
        }
    }

    pub(super) fn semantic_schema_status_text(
        fallback: String,
        validation: Option<SemanticSchemaValidation>,
    ) -> String {
        if let Some(validation) = validation {
            if !validation.errors.is_empty() {
                return format!("Error: {}", validation.errors.join(" "));
            }
            if !validation.warnings.is_empty() {
                return format!("Warning: {}", validation.warnings.join(" "));
            }
        }
        fallback
    }

    pub(super) fn normalize_semantic_schema_mode(raw: &str) -> &'static str {
        if raw.trim().eq_ignore_ascii_case("image") {
            "image"
        } else {
            "video"
        }
    }

    pub(super) fn semantic_schema_mode_label(mode: &str) -> &'static str {
        if Self::normalize_semantic_schema_mode(mode) == "image" {
            "IMAGE"
        } else {
            "VIDEO"
        }
    }

    pub(super) fn parse_optional_positive_u32(raw: &str) -> Result<Option<u32>, ()> {
        let text = raw.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let parsed = text.parse::<u32>().map_err(|_| ())?;
        if parsed == 0 {
            return Err(());
        }
        Ok(Some(parsed))
    }

    fn semantic_catalog_candidate_keys(model_route_key: &str) -> Vec<String> {
        let normalized = model_route_key.trim().to_ascii_lowercase();
        let mut keys = vec![normalized.clone()];
        match normalized.as_str() {
            "google/gemini-3.1-flash-image-preview" => {
                keys.push("nanobanana2".to_string());
                keys.push("nano-banana-2".to_string());
                keys.push("nano_banana_2".to_string());
            }
            "google/gemini-3-pro-image-preview" => {
                keys.push("nanobanana_pro".to_string());
                keys.push("nano-banana-pro".to_string());
                keys.push("nano_banana_pro".to_string());
            }
            "google/gemini-2.5-flash-image" => {
                keys.push("google/nanobanana".to_string());
                keys.push("nanobanana".to_string());
                keys.push("nano-banana".to_string());
                keys.push("nano_banana".to_string());
            }
            "google/veo_3_1" => {
                keys.push("google/veo-3.1-generate-preview".to_string());
                keys.push("veo_3_1".to_string());
                keys.push("veo3.1".to_string());
            }
            "google/veo-3.1-generate-preview" => {
                keys.push("google/veo_3_1".to_string());
                keys.push("veo_3_1".to_string());
                keys.push("veo3.1".to_string());
            }
            _ => {}
        }
        let mut seen = HashSet::new();
        keys.into_iter()
            .filter(|key| seen.insert(key.clone()))
            .collect()
    }

    fn semantic_lookup_resolution_labels(
        catalog: &ModelResolutionCatalog,
        mode: &str,
        model_route_key: &str,
    ) -> Vec<String> {
        let bucket = if mode.eq_ignore_ascii_case("image") {
            &catalog.image
        } else {
            &catalog.video
        };
        for key in Self::semantic_catalog_candidate_keys(model_route_key) {
            if let Some(labels) = bucket.get(key.as_str()) {
                return labels.clone();
            }
        }
        Vec::new()
    }

    fn semantic_lookup_image_aspect_map(
        catalog: &ModelResolutionCatalog,
        model_route_key: &str,
    ) -> Option<AspectRatioResolutionMap> {
        for key in Self::semantic_catalog_candidate_keys(model_route_key) {
            if let Some(table) = catalog.image_aspect_resolution_map.get(key.as_str()) {
                return Some(table.clone());
            }
        }
        None
    }

    fn semantic_lookup_video_constraints(
        catalog: &ModelResolutionCatalog,
        model_route_key: &str,
    ) -> Option<VideoResolutionConstraintMap> {
        for key in Self::semantic_catalog_candidate_keys(model_route_key) {
            if let Some(constraints) = catalog.video_resolution_constraints.get(key.as_str()) {
                return Some(constraints.clone());
            }
        }
        None
    }

    fn semantic_resolution_select_signature(
        mode: &str,
        model_route_key: &str,
        labels: &[String],
    ) -> String {
        format!("{mode}|{model_route_key}|{}", labels.join(","))
    }

    fn semantic_parse_size_text(raw: &str) -> Option<(u32, u32)> {
        let normalized = raw.trim();
        let (width, height) = normalized
            .split_once('x')
            .or_else(|| normalized.split_once('X'))?;
        let width = width.trim().parse::<u32>().ok()?;
        let height = height.trim().parse::<u32>().ok()?;
        if width == 0 || height == 0 {
            return None;
        }
        Some((width, height))
    }

    fn semantic_parse_ratio_text(raw: &str) -> Option<(u32, u32)> {
        let normalized = raw.trim();
        let separator = if normalized.contains(':') { ':' } else { '/' };
        let (left, right) = normalized.split_once(separator)?;
        let left = left.trim().parse::<u32>().ok()?;
        let right = right.trim().parse::<u32>().ok()?;
        if left == 0 || right == 0 {
            return None;
        }
        Some((left, right))
    }

    fn semantic_current_output_size(&self) -> Option<(u32, u32)> {
        let width = Self::parse_optional_positive_u32(self.semantic_output_width.as_str())
            .ok()
            .flatten()?;
        let height = Self::parse_optional_positive_u32(self.semantic_output_height.as_str())
            .ok()
            .flatten()?;
        Some((width, height))
    }

    fn semantic_pick_best_aspect_ratio(
        target_size: Option<(u32, u32)>,
        table: &AspectRatioResolutionMap,
    ) -> Option<String> {
        if table.is_empty() {
            return None;
        }
        let Some((target_w, target_h)) = target_size else {
            if table.contains_key("16:9") {
                return Some("16:9".to_string());
            }
            return table.keys().next().cloned();
        };
        let target_ratio = target_w as f64 / target_h as f64;
        let mut best: Option<(f64, String)> = None;
        for ratio in table.keys() {
            let Some((left, right)) = Self::semantic_parse_ratio_text(ratio.as_str()) else {
                continue;
            };
            let ratio_value = left as f64 / right as f64;
            let diff = (target_ratio - ratio_value).abs();
            match best.as_ref() {
                Some((best_diff, _)) if diff >= *best_diff => {}
                _ => best = Some((diff, ratio.clone())),
            }
        }
        best.map(|(_, ratio)| ratio)
            .or_else(|| table.keys().next().cloned())
    }

    fn semantic_find_image_preset(
        table: &AspectRatioResolutionMap,
        resolution_label: &str,
        target_size: Option<(u32, u32)>,
    ) -> Option<(String, ImageResolutionPreset)> {
        let normalized_label = resolution_label.trim().to_ascii_uppercase();
        let preferred_ratio = Self::semantic_pick_best_aspect_ratio(target_size, table);
        if let Some(preferred_ratio) = preferred_ratio.as_ref()
            && let Some(row) = table.get(preferred_ratio.as_str())
            && let Some(preset) = row.get(normalized_label.as_str()).cloned()
        {
            return Some((preferred_ratio.clone(), preset));
        }
        for (ratio, row) in table {
            if let Some(preset) = row.get(normalized_label.as_str()).cloned() {
                return Some((ratio.clone(), preset));
            }
        }
        None
    }

    fn semantic_video_resolution_to_size(
        resolution_label: &str,
        portrait: bool,
    ) -> Option<(u32, u32)> {
        let normalized = resolution_label.trim().to_ascii_lowercase();
        let mut size = match normalized.as_str() {
            "720p" => Some((1280, 720)),
            "1080p" => Some((1920, 1080)),
            "4k" => Some((3840, 2160)),
            _ => Self::semantic_parse_size_text(normalized.as_str()),
        }?;
        if portrait {
            size = (size.1, size.0);
        }
        Some(size)
    }

    fn semantic_find_video_constraint(
        constraints: &VideoResolutionConstraintMap,
        resolution_label: &str,
    ) -> Option<VideoResolutionConstraint> {
        let trimmed = resolution_label.trim();
        if let Some(rule) = constraints.get(trimmed).cloned() {
            return Some(rule);
        }
        let upper = trimmed.to_ascii_uppercase();
        if let Some(rule) = constraints.get(upper.as_str()).cloned() {
            return Some(rule);
        }
        let lower = trimmed.to_ascii_lowercase();
        constraints.get(lower.as_str()).cloned()
    }

    fn set_semantic_output_size(
        &mut self,
        width: u32,
        height: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.semantic_output_width = width.to_string();
        self.semantic_output_height = height.to_string();
        if let Some(input) = self.semantic_output_width_input.as_ref() {
            let width_text = self.semantic_output_width.clone();
            input.update(cx, |input, cx| {
                input.set_value(width_text.clone(), window, cx);
            });
        }
        if let Some(input) = self.semantic_output_height_input.as_ref() {
            let height_text = self.semantic_output_height.clone();
            input.update(cx, |input, cx| {
                input.set_value(height_text.clone(), window, cx);
            });
        }
        self.global.update(cx, |gs, cx| {
            gs.set_selected_semantic_image_size(Some(width), Some(height));
            cx.notify();
        });
    }

    fn ensure_semantic_resolution_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (mode, model) = {
            let gs = self.global.read(cx);
            (
                gs.get_selected_semantic_asset_mode()
                    .unwrap_or_else(|| "video".to_string()),
                gs.get_selected_semantic_model()
                    .unwrap_or_else(|| "veo_3_1".to_string()),
            )
        };
        let mode = Self::normalize_semantic_schema_mode(mode.as_str()).to_string();
        let Some(model_route_key) = Self::semantic_media_route_key(model.as_str()) else {
            self.semantic_resolution_select = None;
            self.semantic_resolution_select_sub = None;
            self.semantic_resolution_select_sig.clear();
            self.semantic_selected_resolution.clear();
            self.semantic_resolution_apply_pending = false;
            return;
        };

        let catalog = model_resolution_catalog();
        let labels = Self::semantic_lookup_resolution_labels(
            &catalog,
            mode.as_str(),
            model_route_key.as_str(),
        );
        if labels.is_empty() {
            self.semantic_resolution_select = None;
            self.semantic_resolution_select_sub = None;
            self.semantic_resolution_select_sig.clear();
            self.semantic_selected_resolution.clear();
            self.semantic_resolution_apply_pending = false;
            return;
        }

        let signature = Self::semantic_resolution_select_signature(
            mode.as_str(),
            model_route_key.as_str(),
            &labels,
        );
        let selected = labels
            .iter()
            .find(|label| label.eq_ignore_ascii_case(self.semantic_selected_resolution.as_str()))
            .cloned()
            .unwrap_or_else(|| labels[0].clone());
        if self.semantic_selected_resolution != selected {
            self.semantic_selected_resolution = selected.clone();
            self.semantic_resolution_apply_pending = true;
        }

        let recreate = self.semantic_resolution_select.is_none()
            || self.semantic_resolution_select_sig != signature;
        if recreate {
            let items = SearchableVec::new(
                labels
                    .iter()
                    .map(|label| SemanticSelectOption {
                        label: label.clone(),
                        value: label.clone(),
                    })
                    .collect::<Vec<_>>(),
            );
            let state = cx.new(|cx| SelectState::new(items, None, window, cx).searchable(false));
            let selected_value = selected.clone();
            state.update(cx, |state, cx| {
                state.set_selected_value(&selected_value, window, cx);
            });
            let sub = cx.subscribe(
                &state,
                |this, _, ev: &SelectEvent<SearchableVec<SemanticSelectOption>>, cx| {
                    let SelectEvent::Confirm(value) = ev;
                    let Some(value) = value else {
                        return;
                    };
                    this.semantic_selected_resolution = value.clone();
                    this.semantic_resolution_apply_pending = true;
                    cx.notify();
                },
            );
            self.semantic_resolution_select = Some(state);
            self.semantic_resolution_select_sub = Some(sub);
            self.semantic_resolution_select_sig = signature;
            return;
        }

        if let Some(select) = self.semantic_resolution_select.as_ref() {
            let current = select
                .read(cx)
                .selected_value()
                .cloned()
                .unwrap_or_default();
            if current != selected {
                let selected_value = selected.clone();
                select.update(cx, |state, cx| {
                    state.set_selected_value(&selected_value, window, cx);
                });
            }
        }
        self.semantic_resolution_select_sig = signature;
    }

    fn apply_selected_semantic_resolution_preset(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.semantic_resolution_apply_pending {
            return;
        }
        self.semantic_resolution_apply_pending = false;
        let selected_resolution = self.semantic_selected_resolution.trim().to_string();
        if selected_resolution.is_empty() {
            return;
        }

        let (mode, model) = {
            let gs = self.global.read(cx);
            (
                gs.get_selected_semantic_asset_mode()
                    .unwrap_or_else(|| "video".to_string()),
                gs.get_selected_semantic_model()
                    .unwrap_or_else(|| "veo_3_1".to_string()),
            )
        };
        let mode = Self::normalize_semantic_schema_mode(mode.as_str()).to_string();
        let Some(model_route_key) = Self::semantic_media_route_key(model.as_str()) else {
            return;
        };
        let current_size = self.semantic_current_output_size();

        if mode.eq_ignore_ascii_case("image") {
            let catalog = model_resolution_catalog();
            if let Some((width, height)) =
                Self::semantic_parse_size_text(selected_resolution.as_str())
            {
                self.set_semantic_output_size(width, height, window, cx);
                self.semantic_generate_status = format!(
                    "Resolution preset {} -> {}x{}",
                    selected_resolution, width, height
                );
                return;
            }

            let Some(aspect_table) =
                Self::semantic_lookup_image_aspect_map(&catalog, model_route_key.as_str())
            else {
                return;
            };
            let Some((aspect_ratio, preset)) = Self::semantic_find_image_preset(
                &aspect_table,
                selected_resolution.as_str(),
                current_size,
            ) else {
                return;
            };
            let Some((width, height)) = Self::semantic_parse_size_text(preset.size.as_str()) else {
                return;
            };
            self.set_semantic_output_size(width, height, window, cx);
            self.semantic_generate_status = format!(
                "Resolution preset {} @ {} -> {}x{} ({} tokens)",
                selected_resolution, aspect_ratio, width, height, preset.token_count
            );
            return;
        }

        let portrait = current_size.map(|(w, h)| h > w).unwrap_or(false);
        let Some((width, height)) =
            Self::semantic_video_resolution_to_size(selected_resolution.as_str(), portrait)
        else {
            return;
        };
        self.set_semantic_output_size(width, height, window, cx);
        self.semantic_generate_status = format!(
            "Resolution preset {} -> {}x{}",
            selected_resolution, width, height
        );
    }

    pub(super) fn ensure_object_child<'a>(
        root: &'a mut Map<String, Value>,
        key: &str,
    ) -> &'a mut Map<String, Value> {
        let entry = root
            .entry(key.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        entry
            .as_object_mut()
            .expect("semantic schema child must be object")
    }

    pub(super) fn semantic_schema_view_for_mode(full_schema: &Value, mode: &str) -> Value {
        let normalized_mode = Self::normalize_semantic_schema_mode(mode);
        let mut out = Map::new();
        if let Some(root) = full_schema.as_object() {
            for key in ["semantic_goal", "duration_sec"] {
                if let Some(value) = root.get(key) {
                    out.insert(key.to_string(), value.clone());
                }
            }
            out.insert(
                "asset_mode".to_string(),
                Value::String(normalized_mode.to_string()),
            );
            if let Some(value) = root.get("provider") {
                out.insert("provider".to_string(), value.clone());
            }
            if let Some(prompts) = root.get("prompts").and_then(Value::as_object) {
                let mut mode_prompts = Map::new();
                let prompt_key = if normalized_mode == "image" {
                    "image_prompt"
                } else {
                    "video_prompt"
                };
                if let Some(value) = prompts.get(prompt_key) {
                    mode_prompts.insert(prompt_key.to_string(), value.clone());
                }
                if !mode_prompts.is_empty() {
                    out.insert("prompts".to_string(), Value::Object(mode_prompts));
                }
            }
            if normalized_mode == "video" {
                let provider = root
                    .get("provider")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let width = root
                    .get("image_options")
                    .and_then(Value::as_object)
                    .and_then(|options| options.get("width"))
                    .and_then(Value::as_u64)
                    .filter(|v| *v > 0 && *v <= u32::MAX as u64)
                    .map(|v| v as u32)
                    .or_else(|| {
                        Self::semantic_default_video_size_for_provider(provider, normalized_mode)
                            .map(|(w, _)| w)
                    });
                let height = root
                    .get("image_options")
                    .and_then(Value::as_object)
                    .and_then(|options| options.get("height"))
                    .and_then(Value::as_u64)
                    .filter(|v| *v > 0 && *v <= u32::MAX as u64)
                    .map(|v| v as u32)
                    .or_else(|| {
                        Self::semantic_default_video_size_for_provider(provider, normalized_mode)
                            .map(|(_, h)| h)
                    });
                if let Some(width) = width {
                    out.insert("width".to_string(), json!(width));
                }
                if let Some(height) = height {
                    out.insert("height".to_string(), json!(height));
                }
            }
            if normalized_mode == "image"
                && let Some(value) = root.get("image_options")
            {
                out.insert("image_options".to_string(), value.clone());
            }
        }
        if out.is_empty() {
            out.insert(
                "asset_mode".to_string(),
                Value::String(normalized_mode.to_string()),
            );
            out.insert("prompts".to_string(), Value::Object(Map::new()));
        }
        Value::Object(out)
    }

    pub(super) fn merge_mode_schema_into_full(
        full_schema: &mut Value,
        mode: &str,
        mode_schema: &Value,
    ) -> Result<(), String> {
        let normalized_mode = Self::normalize_semantic_schema_mode(mode);
        let mode_root = mode_schema
            .as_object()
            .ok_or_else(|| "Schema JSON must be an object.".to_string())?;
        if !full_schema.is_object() {
            *full_schema = Value::Object(Map::new());
        }
        let full_root = full_schema
            .as_object_mut()
            .ok_or_else(|| "Failed to normalize semantic schema root object.".to_string())?;

        for key in ["semantic_goal", "duration_sec"] {
            if let Some(value) = mode_root.get(key) {
                full_root.insert(key.to_string(), value.clone());
            }
        }

        full_root.insert(
            "asset_mode".to_string(),
            Value::String(normalized_mode.to_string()),
        );
        if let Some(provider) = mode_root.get("provider") {
            full_root.insert("provider".to_string(), provider.clone());
        }

        let prompt_key = if normalized_mode == "image" {
            "image_prompt"
        } else {
            "video_prompt"
        };
        if let Some(prompt) = mode_root.get("prompt").and_then(Value::as_str) {
            let prompts = Self::ensure_object_child(full_root, "prompts");
            prompts.insert(prompt_key.to_string(), Value::String(prompt.to_string()));
        }
        if let Some(mode_prompts) = mode_root.get("prompts").and_then(Value::as_object) {
            let prompts = Self::ensure_object_child(full_root, "prompts");
            if let Some(prompt) = mode_prompts.get(prompt_key) {
                prompts.insert(prompt_key.to_string(), prompt.clone());
            }
            prompts.remove("fallback_search_query");
        }

        if normalized_mode == "image" {
            if let Some(image_options) = mode_root.get("image_options").and_then(Value::as_object) {
                let mut cleaned = Map::new();
                if let Some(width) = image_options.get("width").and_then(Value::as_u64)
                    && width > 0
                    && width <= u32::MAX as u64
                {
                    cleaned.insert("width".to_string(), json!(width as u32));
                }
                if let Some(height) = image_options.get("height").and_then(Value::as_u64)
                    && height > 0
                    && height <= u32::MAX as u64
                {
                    cleaned.insert("height".to_string(), json!(height as u32));
                }
                if cleaned.is_empty() {
                    full_root.remove("image_options");
                } else {
                    full_root.insert("image_options".to_string(), Value::Object(cleaned));
                }
            } else {
                full_root.remove("image_options");
            }
        }
        if normalized_mode == "video" {
            let width = mode_root
                .get("width")
                .and_then(Value::as_u64)
                .filter(|v| *v > 0 && *v <= u32::MAX as u64)
                .map(|v| v as u32)
                .or_else(|| {
                    mode_root
                        .get("image_options")
                        .and_then(Value::as_object)
                        .and_then(|opts| opts.get("width"))
                        .and_then(Value::as_u64)
                        .filter(|v| *v > 0 && *v <= u32::MAX as u64)
                        .map(|v| v as u32)
                });
            let height = mode_root
                .get("height")
                .and_then(Value::as_u64)
                .filter(|v| *v > 0 && *v <= u32::MAX as u64)
                .map(|v| v as u32)
                .or_else(|| {
                    mode_root
                        .get("image_options")
                        .and_then(Value::as_object)
                        .and_then(|opts| opts.get("height"))
                        .and_then(Value::as_u64)
                        .filter(|v| *v > 0 && *v <= u32::MAX as u64)
                        .map(|v| v as u32)
                });
            if width.is_none() && height.is_none() {
                full_root.remove("image_options");
            } else {
                let image_options = Self::ensure_object_child(full_root, "image_options");
                if let Some(width) = width {
                    image_options.insert("width".to_string(), json!(width));
                } else {
                    image_options.remove("width");
                }
                if let Some(height) = height {
                    image_options.insert("height".to_string(), json!(height));
                } else {
                    image_options.remove("height");
                }
                if image_options.is_empty() {
                    full_root.remove("image_options");
                }
            }
            full_root.remove("width");
            full_root.remove("height");
        }

        full_root.remove("provider_prompts");
        full_root.remove("meta");
        full_root.remove("schema_version");
        full_root.remove("provider_limits");

        Ok(())
    }

    pub(super) fn ensure_semantic_schema_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.semantic_schema_input.is_some() {
            return;
        }
        // Build an inline JSON editor so semantic prompt schema can be edited directly in Inspector.
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("json")
                .rows(7)
                .line_number(true)
                .soft_wrap(true)
                .placeholder("{ \"provider\": \"gpt-image-1\" }")
        });
        let initial = self.semantic_schema_text.clone();
        input.update(cx, |this, cx| {
            this.set_value(initial.clone(), window, cx);
        });
        let sub = cx.subscribe(&input, |this, input, ev, _cx| {
            if matches!(ev, InputEvent::Change | InputEvent::PressEnter { .. }) {
                this.semantic_schema_text = input.read(_cx).value().to_string();
            }
        });
        self.semantic_schema_input = Some(input);
        self.semantic_schema_input_sub = Some(sub);
    }

    pub(super) fn sync_semantic_schema_from_selected_clip(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (selected_id, schema_json, validation, asset_mode) = {
            let gs = self.global.read(cx);
            let selected_id = gs.selected_semantic_clip_id();
            let asset_mode = gs
                .get_selected_semantic_asset_mode()
                .unwrap_or_else(|| "video".to_string());
            (
                selected_id,
                selected_id.and_then(|_| gs.get_selected_semantic_schema_json()),
                gs.validate_selected_semantic_schema(),
                asset_mode,
            )
        };
        let active_mode = Self::normalize_semantic_schema_mode(asset_mode.as_str()).to_string();
        let mode_label = Self::semantic_schema_mode_label(active_mode.as_str());
        let Some(selected_id) = selected_id else {
            if self.semantic_schema_clip_id.take().is_some() {
                self.semantic_schema_text.clear();
                self.semantic_schema_mode = "video".to_string();
                if let Some(input) = self.semantic_schema_input.as_ref() {
                    input.update(cx, |this, cx| {
                        this.set_value("", window, cx);
                    });
                }
            }
            self.semantic_schema_status = "No semantic clip selected.".to_string();
            return;
        };

        let full_schema = schema_json
            .as_deref()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .unwrap_or_else(|| Value::Object(Map::new()));
        let mode_schema = Self::semantic_schema_view_for_mode(&full_schema, active_mode.as_str());
        let next_json =
            serde_json::to_string_pretty(&mode_schema).unwrap_or_else(|_| "{}".to_string());
        let mode_changed = self.semantic_schema_mode != active_mode;
        if self.semantic_schema_clip_id == Some(selected_id) {
            let focused = self
                .semantic_schema_input
                .as_ref()
                .map(|input| input.read(cx).focus_handle(cx).is_focused(window))
                .unwrap_or(false);
            if mode_changed || (!focused && self.semantic_schema_text != next_json) {
                self.semantic_schema_text = next_json.clone();
                self.semantic_schema_mode = active_mode.clone();
                if let Some(input) = self.semantic_schema_input.as_ref() {
                    input.update(cx, |this, cx| {
                        this.set_value(next_json.clone(), window, cx);
                    });
                }
            }
            self.semantic_schema_status = Self::semantic_schema_status_text(
                format!("Editing {mode_label} schema for clip #{}.", selected_id),
                validation,
            );
            return;
        }

        self.semantic_schema_clip_id = Some(selected_id);
        self.semantic_schema_mode = active_mode;
        self.semantic_schema_text = next_json.clone();
        if let Some(input) = self.semantic_schema_input.as_ref() {
            input.update(cx, |this, cx| {
                this.set_value(next_json.clone(), window, cx);
            });
        }
        self.semantic_schema_status = Self::semantic_schema_status_text(
            format!("Editing {mode_label} schema for clip #{}.", selected_id),
            validation,
        );
    }

    pub(super) fn apply_semantic_schema_json(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.semantic_schema_clip_id.is_none() {
            self.semantic_schema_status = "No semantic clip selected.".to_string();
            return;
        }
        let raw_json = self.semantic_schema_text.trim().to_string();
        if raw_json.is_empty() {
            self.semantic_schema_status = "Semantic schema JSON cannot be empty.".to_string();
            return;
        }
        let mode =
            Self::normalize_semantic_schema_mode(self.semantic_schema_mode.as_str()).to_string();
        let mode_label = Self::semantic_schema_mode_label(mode.as_str());
        let parsed_mode_schema = match serde_json::from_str::<Value>(raw_json.as_str()) {
            Ok(value) => value,
            Err(err) => {
                self.semantic_schema_status = format!("Invalid {mode_label} schema JSON: {err}");
                return;
            }
        };

        // Apply only the active mode schema and merge back into full semantic schema.
        let result: Result<(), String> = self.global.update(cx, |gs, cx| {
            let base_full_json = gs
                .get_selected_semantic_schema_json()
                .unwrap_or_else(|| "{}".to_string());
            let mut full_schema = serde_json::from_str::<Value>(base_full_json.as_str())
                .unwrap_or_else(|_| Value::Object(Map::new()));
            Self::merge_mode_schema_into_full(
                &mut full_schema,
                mode.as_str(),
                &parsed_mode_schema,
            )?;
            let merged_json = serde_json::to_string_pretty(&full_schema)
                .map_err(|err| format!("Failed to serialize merged schema: {err}"))?;
            let result = gs
                .set_selected_semantic_schema_json(merged_json)
                .map_err(|err| err.to_string());
            cx.notify();
            result
        });
        match result {
            Ok(()) => {
                let validation = self.global.read(cx).validate_selected_semantic_schema();
                self.semantic_schema_status = Self::semantic_schema_status_text(
                    format!("{mode_label} schema applied."),
                    validation,
                );
                self.sync_semantic_schema_from_selected_clip(window, cx);
            }
            Err(err) => {
                self.semantic_schema_status = err;
            }
        }
    }

    pub(super) fn sanitize_file_token(raw: &str) -> String {
        raw.chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                    ch
                } else {
                    '_'
                }
            })
            .collect()
    }

    pub(super) fn next_semantic_generated_image_path(
        output_dir: &Path,
        clip_id: u64,
        model: &str,
    ) -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let model_tag = Self::sanitize_file_token(model);
        output_dir.join(format!("semantic_{clip_id}_{model_tag}_{ts}.png"))
    }

    pub(super) fn next_semantic_generated_video_path(
        output_dir: &Path,
        clip_id: u64,
        model: &str,
    ) -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let model_tag = Self::sanitize_file_token(model);
        output_dir.join(format!("semantic_{clip_id}_{model_tag}_{ts}.mp4"))
    }

    pub(super) fn next_semantic_generated_mask_path(output_dir: &Path, clip_id: u64) -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        output_dir
            .join("painting_masks")
            .join(format!("semantic_mask_{clip_id}_{ts}.png"))
    }

    pub(super) fn download_binary(url: &str) -> Result<Vec<u8>, String> {
        let response = ureq::get(url)
            .set("User-Agent", "anica-semantic-image/1.0")
            .call()
            .map_err(|err| format!("Failed to download generated image URL: {err}"))?;
        let mut reader = response.into_reader();
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .map_err(|err| format!("Failed to read generated image bytes: {err}"))?;
        Ok(bytes)
    }

    pub(super) fn output_url_to_local_path(output_url: &str) -> Option<PathBuf> {
        match Url::parse(output_url) {
            Ok(parsed) if parsed.scheme() == "file" => parsed.to_file_path().ok(),
            Ok(_) => None,
            Err(_) => Some(PathBuf::from(output_url)),
        }
    }

    pub(super) fn format_media_protocol_error(err: &MediaGenProtocolError) -> String {
        let mut out = format!("{:?}: {}", err.code, err.message);
        if let Some(provider) = err.provider.as_deref() {
            out.push_str(format!(" [provider={provider}]").as_str());
        }
        if let Some(provider_code) = err.provider_code.as_deref() {
            out.push_str(format!(" [provider_code={provider_code}]").as_str());
        }
        if let Some(status) = err.provider_http_status {
            out.push_str(format!(" [status={status}]").as_str());
        }
        out
    }

    pub(super) fn semantic_media_route_key(model: &str) -> Option<String> {
        let normalized = model.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return None;
        }
        if normalized.starts_with("openai/") || normalized.starts_with("google/") {
            return match normalized.as_str() {
                "openai/gpt-image-1"
                | "openai/gpt-image-1.5"
                | "openai/gpt-image-1-mini"
                | "openai/sora-2"
                | "openai/sora-2-pro"
                | "google/veo_3_1"
                | "google/veo-3.1-generate-preview"
                | "google/gemini-3.1-flash-image-preview"
                | "google/gemini-3-pro-image-preview"
                | "google/gemini-2.5-flash-image" => Some(normalized),
                // Keep compatibility for previous Nanobanana route key.
                "google/nanobanana" => Some("google/gemini-2.5-flash-image".to_string()),
                _ => None,
            };
        }

        match normalized.as_str() {
            "gpt-image-1" => Some("openai/gpt-image-1".to_string()),
            "gpt-image-1.5" => Some("openai/gpt-image-1.5".to_string()),
            "gpt-image-1-mini" => Some("openai/gpt-image-1-mini".to_string()),
            "sora_2" | "sora-2" => Some("openai/sora-2".to_string()),
            "sora_2_pro" | "sora-2-pro" => Some("openai/sora-2-pro".to_string()),
            "veo_3_1" | "veo-3.1-generate-preview" => Some("google/veo_3_1".to_string()),
            "nanobanana" => Some("google/gemini-2.5-flash-image".to_string()),
            "gemini-3.1-flash-image-preview" => {
                Some("google/gemini-3.1-flash-image-preview".to_string())
            }
            "gemini-3-pro-image-preview" => Some("google/gemini-3-pro-image-preview".to_string()),
            "gemini-2.5-flash-image" => Some("google/gemini-2.5-flash-image".to_string()),
            _ => None,
        }
    }

    pub(super) fn semantic_route_supports_image_generation(model_route_key: &str) -> bool {
        matches!(
            model_route_key,
            "openai/gpt-image-1"
                | "openai/gpt-image-1.5"
                | "openai/gpt-image-1-mini"
                | "google/gemini-3.1-flash-image-preview"
                | "google/gemini-3-pro-image-preview"
                | "google/gemini-2.5-flash-image"
        )
    }

    pub(super) fn semantic_route_supports_openai_image_edit(model_route_key: &str) -> bool {
        model_route_key.eq_ignore_ascii_case("openai/gpt-image-1")
    }

    pub(super) fn semantic_route_supports_video_generation(model_route_key: &str) -> bool {
        model_route_key.eq_ignore_ascii_case("openai/sora-2")
            || model_route_key.eq_ignore_ascii_case("openai/sora-2-pro")
            || model_route_key.eq_ignore_ascii_case("google/veo_3_1")
            || model_route_key.eq_ignore_ascii_case("google/veo-3.1-generate-preview")
    }

    pub(super) fn request_image_via_media_protocol(
        api_key: &str,
        model_route_key: &str,
        prompt: &str,
        output_path: &Path,
        image_size: &str,
        input_image_path: Option<String>,
        mask_path: Option<String>,
    ) -> Result<PathBuf, String> {
        let key_slot = if model_route_key.starts_with("google/") {
            "google"
        } else {
            "openai"
        };
        let mut model_registry = ModelRegistry::new();
        model_registry
            .insert(ModelSpec {
                route_key: model_route_key.to_string(),
                label: model_route_key.to_string(),
                supported_assets: vec![ProtocolAssetKind::Image],
                enabled: true,
                api_key_slot: Some(key_slot.to_string()),
            })
            .map_err(|err| {
                format!(
                    "Failed to register semantic model '{model_route_key}': {}",
                    Self::format_media_protocol_error(&err)
                )
            })?;

        let mut key_resolver = StaticKeyResolver::new();
        key_resolver.insert(key_slot, api_key.trim());
        let output_path = output_path.to_path_buf();
        let output_uploader = Arc::new(SemanticFileOutputUploader::new(output_path.clone()));
        let context = ProtocolGatewayContext::new(
            Arc::new(model_registry),
            Arc::new(key_resolver),
            output_uploader,
        );
        let mut gateway = ProtocolGatewayService::new(context, Arc::new(InMemoryJobStore::new()));
        gateway.register_adapter(Arc::new(OpenAiAdapter));
        gateway.register_adapter(Arc::new(GoogleGenAiAdapter));

        let mut provider_options = Map::new();
        provider_options.insert("size".to_string(), Value::String(image_size.to_string()));
        let mut input_assets = Vec::new();
        if let Some(path) = input_image_path.as_deref()
            && !path.trim().is_empty()
        {
            input_assets.push(ProtocolInputAsset {
                kind: ProtocolInputAssetKind::Image,
                url: path.trim().to_string(),
                role: Some("image".to_string()),
            });
        }
        if let Some(path) = mask_path.as_deref()
            && !path.trim().is_empty()
        {
            input_assets.push(ProtocolInputAsset {
                kind: ProtocolInputAssetKind::Mask,
                url: path.trim().to_string(),
                role: Some("mask".to_string()),
            });
        }
        let request = ProtocolGenerateRequest {
            model: model_route_key.to_string(),
            asset_kind: ProtocolAssetKind::Image,
            prompt: prompt.to_string(),
            negative_prompt: None,
            inputs: input_assets,
            duration_sec: None,
            aspect_ratio: Some("1:1".to_string()),
            provider_options,
            callback_url: None,
            idempotency_key: None,
            metadata: Map::new(),
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| format!("Failed to create semantic image runtime: {err}"))?;

        let generated_output_url = runtime.block_on(async move {
            let accepted = gateway.submit(request).await.map_err(|err| {
                format!(
                    "Semantic image request failed: {}",
                    Self::format_media_protocol_error(&err)
                )
            })?;

            let snapshot = gateway.poll(&accepted.job_id).await.map_err(|err| {
                format!(
                    "Semantic image poll failed: {}",
                    Self::format_media_protocol_error(&err)
                )
            })?;

            if snapshot.status != media_gen_protocol::JobStatus::Succeeded {
                let details = snapshot
                    .error
                    .as_ref()
                    .map(Self::format_media_protocol_error)
                    .unwrap_or_else(|| format!("status={:?}", snapshot.status));
                return Err(format!("Semantic image generation failed: {details}"));
            }

            let output_url = snapshot
                .result
                .as_ref()
                .and_then(|result| result.outputs.first())
                .map(|output| output.url.clone())
                .ok_or_else(|| "Semantic image generation returned no output asset.".to_string())?;

            Ok::<String, String>(output_url)
        })?;

        if let Some(local_path) = Self::output_url_to_local_path(generated_output_url.as_str()) {
            if local_path != output_path {
                let bytes = fs::read(&local_path).map_err(|err| {
                    format!(
                        "Failed to read generated semantic image '{}': {err}",
                        local_path.display()
                    )
                })?;
                fs::write(&output_path, bytes).map_err(|err| {
                    format!(
                        "Failed to write generated image '{}': {err}",
                        output_path.display()
                    )
                })?;
            }
            return Ok(output_path);
        }

        let bytes = Self::download_binary(generated_output_url.as_str())?;
        fs::write(&output_path, bytes).map_err(|err| {
            format!(
                "Failed to write generated image '{}': {err}",
                output_path.display()
            )
        })?;
        Ok(output_path)
    }

    pub(super) fn request_video_via_media_protocol(
        api_key: &str,
        model_route_key: &str,
        prompt: &str,
        output_path: &Path,
        video_size: &str,
        duration_sec: f64,
    ) -> Result<PathBuf, String> {
        let key_slot = if model_route_key.starts_with("google/") {
            "google"
        } else {
            "openai"
        };
        let mut model_registry = ModelRegistry::new();
        model_registry
            .insert(ModelSpec {
                route_key: model_route_key.to_string(),
                label: model_route_key.to_string(),
                supported_assets: vec![ProtocolAssetKind::Video],
                enabled: true,
                api_key_slot: Some(key_slot.to_string()),
            })
            .map_err(|err| {
                format!(
                    "Failed to register semantic model '{model_route_key}': {}",
                    Self::format_media_protocol_error(&err)
                )
            })?;

        let key_resolver = StaticKeyResolver::new().with_key(key_slot, api_key.trim());
        let output_path = output_path.to_path_buf();
        let output_uploader = Arc::new(SemanticFileOutputUploader::new(output_path.clone()));
        let context = ProtocolGatewayContext::new(
            Arc::new(model_registry),
            Arc::new(key_resolver),
            output_uploader,
        );
        let mut gateway = ProtocolGatewayService::new(context, Arc::new(InMemoryJobStore::new()));
        gateway.register_adapter(Arc::new(OpenAiAdapter));
        gateway.register_adapter(Arc::new(GoogleGenAiAdapter));

        let mut provider_options = Map::new();
        provider_options.insert("size".to_string(), Value::String(video_size.to_string()));
        let request = ProtocolGenerateRequest {
            model: model_route_key.to_string(),
            asset_kind: ProtocolAssetKind::Video,
            prompt: prompt.to_string(),
            negative_prompt: None,
            inputs: Vec::new(),
            duration_sec: Some(duration_sec),
            aspect_ratio: None,
            provider_options,
            callback_url: None,
            idempotency_key: None,
            metadata: Map::new(),
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| format!("Failed to create semantic video runtime: {err}"))?;

        let generated_output_url = runtime.block_on(async move {
            let accepted = gateway.submit(request).await.map_err(|err| {
                format!(
                    "Semantic video request failed: {}",
                    Self::format_media_protocol_error(&err)
                )
            })?;
            let mut last_status = accepted.status;

            // Poll provider job until terminal state because video providers are async.
            for _ in 0..120 {
                let snapshot = gateway.poll(&accepted.job_id).await.map_err(|err| {
                    format!(
                        "Semantic video poll failed: {}",
                        Self::format_media_protocol_error(&err)
                    )
                })?;
                last_status = snapshot.status;

                if snapshot.status == media_gen_protocol::JobStatus::Succeeded {
                    let output_url = snapshot
                        .result
                        .as_ref()
                        .and_then(|result| result.outputs.first())
                        .map(|output| output.url.clone())
                        .ok_or_else(|| {
                            "Semantic video generation returned no output asset.".to_string()
                        })?;
                    return Ok::<String, String>(output_url);
                }

                if snapshot.status.is_terminal() {
                    let details = snapshot
                        .error
                        .as_ref()
                        .map(Self::format_media_protocol_error)
                        .unwrap_or_else(|| format!("status={:?}", snapshot.status));
                    return Err(format!("Semantic video generation failed: {details}"));
                }

                tokio::time::sleep(Duration::from_secs(2)).await;
            }

            Err(format!(
                "Semantic video generation timed out while polling (last status: {last_status:?})."
            ))
        })?;

        if let Some(local_path) = Self::output_url_to_local_path(generated_output_url.as_str()) {
            if local_path != output_path {
                let bytes = fs::read(&local_path).map_err(|err| {
                    format!(
                        "Failed to read generated semantic video '{}': {err}",
                        local_path.display()
                    )
                })?;
                fs::write(&output_path, bytes).map_err(|err| {
                    format!(
                        "Failed to write generated video '{}': {err}",
                        output_path.display()
                    )
                })?;
            }
            return Ok(output_path);
        }

        let bytes = Self::download_binary(generated_output_url.as_str())?;
        fs::write(&output_path, bytes).map_err(|err| {
            format!(
                "Failed to write generated video '{}': {err}",
                output_path.display()
            )
        })?;
        Ok(output_path)
    }

    pub(super) fn semantic_model_uses_google_api_key(model: &str) -> bool {
        let normalized = model.trim().to_ascii_lowercase();
        if let Some(model_route_key) = Self::semantic_media_route_key(normalized.as_str()) {
            return model_route_key.starts_with("google/");
        }
        matches!(
            normalized.as_str(),
            "veo_3_1"
                | "nanobanana"
                | "google/veo_3_1"
                | "google/veo-3.1-generate-preview"
                | "google/nanobanana"
                | "gemini-3.1-flash-image-preview"
                | "gemini-3-pro-image-preview"
                | "gemini-2.5-flash-image"
        )
    }

    pub(super) fn semantic_api_key_label_for_model(model: &str) -> &'static str {
        if Self::semantic_model_uses_google_api_key(model) {
            "Google API Key"
        } else {
            "OpenAI API Key"
        }
    }

    pub(super) fn semantic_api_key_placeholder_for_model(model: &str) -> &'static str {
        if Self::semantic_model_uses_google_api_key(model) {
            "GOOGLE_API_KEY"
        } else {
            "OPENAI_API_KEY"
        }
    }

    pub(super) fn semantic_api_key_required_message_for_model(model: &str) -> &'static str {
        if Self::semantic_model_uses_google_api_key(model) {
            "Google API key is required."
        } else {
            "OpenAI API key is required."
        }
    }

    pub(super) fn ensure_semantic_image_api_key_input(
        &mut self,
        window: &mut Window,
        model: &str,
        cx: &mut Context<Self>,
    ) {
        let placeholder = Self::semantic_api_key_placeholder_for_model(model);
        let placeholder_unchanged = self
            .semantic_image_api_key_placeholder
            .eq_ignore_ascii_case(placeholder);
        if self.semantic_image_api_key_input.is_some() && placeholder_unchanged {
            return;
        }

        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .masked(true)
        });
        let sub = cx.subscribe(&input, |this, input, ev, cx| {
            if !matches!(ev, InputEvent::Change) {
                return;
            }
            this.semantic_image_api_key = input.read(cx).value().to_string();
            cx.notify();
        });
        let existing_value = self.semantic_image_api_key.clone();
        input.update(cx, move |state, cx| {
            state.set_value(existing_value.clone(), window, cx);
        });

        self.semantic_image_api_key_input = Some(input);
        self.semantic_image_api_key_input_sub = Some(sub);
        self.semantic_image_api_key_placeholder = placeholder.to_string();
    }

    pub(super) fn ensure_semantic_render_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Lazy-init semantic type input (single-line text field for marker category)
        if self.semantic_type_input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Semantic type…"));
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                if this.semantic_editing_id.is_none() {
                    return;
                }
                let text = input.read(cx).value().to_string();
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_semantic_type(text);
                    cx.notify();
                });
                cx.notify();
            });
            self.semantic_type_input = Some(input);
            self.semantic_type_input_sub = Some(sub);
        }

        // Lazy-init semantic label input (single-line text field for B-roll notes / markers)
        if self.semantic_label_input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Semantic label…"));
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                if this.semantic_editing_id.is_none() {
                    return;
                }
                let text = input.read(cx).value().to_string();
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_semantic_label(text);
                    cx.notify();
                });
                cx.notify();
            });
            self.semantic_label_input = Some(input);
            self.semantic_label_input_sub = Some(sub);
        }

        // Lazy-init semantic prompt input (multi-line prompt block for image/video generation).
        if self.semantic_prompt_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .multi_line(true)
                    .rows(4)
                    .placeholder("Type generation prompt…")
            });
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                if this.semantic_editing_id.is_none() {
                    return;
                }
                let text = input.read(cx).value().to_string();
                this.global.update(cx, |gs, cx| {
                    gs.set_selected_semantic_prompt_text(text);
                    cx.notify();
                });
                cx.notify();
            });
            self.semantic_prompt_input = Some(input);
            self.semantic_prompt_input_sub = Some(sub);
        }

        let semantic_model_for_api_key_input = self
            .global
            .read(cx)
            .get_selected_semantic_model()
            .unwrap_or_else(|| "veo_3_1".to_string());
        self.ensure_semantic_image_api_key_input(
            window,
            semantic_model_for_api_key_input.as_str(),
            cx,
        );
        if self.semantic_input_image_path_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx).placeholder("Input Image File (optional, local path)")
            });
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                this.semantic_input_image_path = input.read(cx).value().to_string();
                cx.notify();
            });
            self.semantic_input_image_path_input = Some(input);
            self.semantic_input_image_path_input_sub = Some(sub);
        }
        if self.semantic_input_mask_path_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Mask File (optional, requires Input Image File)")
            });
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                this.semantic_input_mask_path = input.read(cx).value().to_string();
                cx.notify();
            });
            self.semantic_input_mask_path_input = Some(input);
            self.semantic_input_mask_path_input_sub = Some(sub);
        }
        if self.semantic_output_width_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Width (optional, default Display Settings)")
            });
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                this.semantic_output_width = input.read(cx).value().to_string();
                if this.semantic_editing_id.is_none() {
                    cx.notify();
                    return;
                }
                let width = Self::parse_optional_positive_u32(this.semantic_output_width.as_str());
                let height =
                    Self::parse_optional_positive_u32(this.semantic_output_height.as_str());
                if let (Ok(width), Ok(height)) = (width, height) {
                    this.global.update(cx, |gs, cx| {
                        gs.set_selected_semantic_image_size(width, height);
                        cx.notify();
                    });
                }
                cx.notify();
            });
            self.semantic_output_width_input = Some(input);
            self.semantic_output_width_input_sub = Some(sub);
        }
        if self.semantic_output_height_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Height (optional, default Display Settings)")
            });
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                this.semantic_output_height = input.read(cx).value().to_string();
                if this.semantic_editing_id.is_none() {
                    cx.notify();
                    return;
                }
                let width = Self::parse_optional_positive_u32(this.semantic_output_width.as_str());
                let height =
                    Self::parse_optional_positive_u32(this.semantic_output_height.as_str());
                if let (Ok(width), Ok(height)) = (width, height) {
                    this.global.update(cx, |gs, cx| {
                        gs.set_selected_semantic_image_size(width, height);
                        cx.notify();
                    });
                }
                cx.notify();
            });
            self.semantic_output_height_input = Some(input);
            self.semantic_output_height_input_sub = Some(sub);
        }

        self.ensure_semantic_resolution_select(window, cx);
        self.apply_selected_semantic_resolution_preset(window, cx);
    }

    pub(super) fn sync_semantic_inputs_with_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected_semantic_id = self.global.read(cx).selected_semantic_clip_id;
        let selected_semantic_type = self.global.read(cx).get_selected_semantic_type();
        let selected_semantic_label = self.global.read(cx).get_selected_semantic_label();
        let selected_semantic_prompt = self.global.read(cx).get_selected_semantic_prompt_text();
        let (selected_semantic_width_text, selected_semantic_height_text) = self
            .global
            .read(cx)
            .get_selected_semantic_image_size()
            .map(|(width, height)| {
                (
                    width.map(|v| v.to_string()).unwrap_or_default(),
                    height.map(|v| v.to_string()).unwrap_or_default(),
                )
            })
            .unwrap_or_else(|| (String::new(), String::new()));
        if let Some(id) = selected_semantic_id {
            if self.semantic_editing_id != Some(id) {
                self.semantic_editing_id = Some(id);
                if let Some(input) = self.semantic_type_input.as_ref() {
                    let text = selected_semantic_type.clone().unwrap_or_default();
                    input.update(cx, |input, cx| {
                        input.set_value(text, window, cx);
                    });
                }
                if let Some(input) = self.semantic_label_input.as_ref() {
                    let text = selected_semantic_label.clone().unwrap_or_default();
                    input.update(cx, |input, cx| {
                        input.set_value(text, window, cx);
                    });
                }
                if let Some(input) = self.semantic_prompt_input.as_ref() {
                    let text = selected_semantic_prompt.clone().unwrap_or_default();
                    input.update(cx, |input, cx| {
                        input.set_value(text, window, cx);
                    });
                }
                self.semantic_output_width = selected_semantic_width_text.clone();
                self.semantic_output_height = selected_semantic_height_text.clone();
                if let Some(input) = self.semantic_output_width_input.as_ref() {
                    let width_text = self.semantic_output_width.clone();
                    input.update(cx, |input, cx| {
                        input.set_value(width_text.clone(), window, cx);
                    });
                }
                if let Some(input) = self.semantic_output_height_input.as_ref() {
                    let height_text = self.semantic_output_height.clone();
                    input.update(cx, |input, cx| {
                        input.set_value(height_text.clone(), window, cx);
                    });
                }
                self.semantic_input_image_path.clear();
                self.semantic_input_mask_path.clear();
                if let Some(input) = self.semantic_input_image_path_input.as_ref() {
                    input.update(cx, |input, cx| {
                        input.set_value("", window, cx);
                    });
                }
                if let Some(input) = self.semantic_input_mask_path_input.as_ref() {
                    input.update(cx, |input, cx| {
                        input.set_value("", window, cx);
                    });
                }
                self.semantic_mask_painter.close();
            } else {
                if let (Some(input), Some(text)) = (
                    self.semantic_type_input.as_ref(),
                    selected_semantic_type.as_ref(),
                ) {
                    let focused = input.read(cx).focus_handle(cx).is_focused(window);
                    if !focused {
                        let current = input.read(cx).value();
                        if current.as_ref() != text {
                            let text = text.clone();
                            input.update(cx, |input, cx| {
                                input.set_value(text, window, cx);
                            });
                        }
                    }
                }
                if let (Some(input), Some(text)) = (
                    self.semantic_label_input.as_ref(),
                    selected_semantic_label.as_ref(),
                ) {
                    let focused = input.read(cx).focus_handle(cx).is_focused(window);
                    if !focused {
                        let current = input.read(cx).value();
                        if current.as_ref() != text {
                            let text = text.clone();
                            input.update(cx, |input, cx| {
                                input.set_value(text, window, cx);
                            });
                        }
                    }
                }
                if let (Some(input), Some(text)) = (
                    self.semantic_prompt_input.as_ref(),
                    selected_semantic_prompt.as_ref(),
                ) {
                    let focused = input.read(cx).focus_handle(cx).is_focused(window);
                    if !focused {
                        let current = input.read(cx).value();
                        if current.as_ref() != text {
                            let text = text.clone();
                            input.update(cx, |input, cx| {
                                input.set_value(text, window, cx);
                            });
                        }
                    }
                }
                if let Some(input) = self.semantic_output_width_input.as_ref() {
                    let focused = input.read(cx).focus_handle(cx).is_focused(window);
                    if !focused {
                        let current = input.read(cx).value().to_string();
                        if current != selected_semantic_width_text {
                            self.semantic_output_width = selected_semantic_width_text.clone();
                            let width_text = selected_semantic_width_text.clone();
                            input.update(cx, |input, cx| {
                                input.set_value(width_text.clone(), window, cx);
                            });
                        }
                    }
                }
                if let Some(input) = self.semantic_output_height_input.as_ref() {
                    let focused = input.read(cx).focus_handle(cx).is_focused(window);
                    if !focused {
                        let current = input.read(cx).value().to_string();
                        if current != selected_semantic_height_text {
                            self.semantic_output_height = selected_semantic_height_text.clone();
                            let height_text = selected_semantic_height_text.clone();
                            input.update(cx, |input, cx| {
                                input.set_value(height_text.clone(), window, cx);
                            });
                        }
                    }
                }
            }
        } else if self.semantic_editing_id.is_some() {
            self.semantic_editing_id = None;
            if let Some(input) = self.semantic_type_input.as_ref() {
                input.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                });
            }
            if let Some(input) = self.semantic_label_input.as_ref() {
                input.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                });
            }
            if let Some(input) = self.semantic_prompt_input.as_ref() {
                input.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                });
            }
            self.semantic_output_width.clear();
            self.semantic_output_height.clear();
            if let Some(input) = self.semantic_output_width_input.as_ref() {
                input.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                });
            }
            if let Some(input) = self.semantic_output_height_input.as_ref() {
                input.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                });
            }
            self.semantic_input_image_path.clear();
            self.semantic_input_mask_path.clear();
            if let Some(input) = self.semantic_input_image_path_input.as_ref() {
                input.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                });
            }
            if let Some(input) = self.semantic_input_mask_path_input.as_ref() {
                input.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                });
            }
            self.semantic_mask_painter.close();
            self.semantic_selected_resolution.clear();
            self.semantic_resolution_apply_pending = false;
        }
    }

    pub(super) fn prompt_semantic_local_file_path(
        &mut self,
        pick_mask: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prompt = if pick_mask {
            "Select mask image".to_string()
        } else {
            "Select input image".to_string()
        };
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(prompt.into()),
        });

        cx.spawn_in(window, async move |view, window| {
            let Ok(result) = rx.await else { return };
            let Some(paths) = result.ok().flatten() else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let path_str = path.to_string_lossy().to_string();

            let _ = view.update_in(window, |this, window, cx| {
                if pick_mask {
                    this.semantic_input_mask_path = path_str.clone();
                    if let Some(input) = this.semantic_input_mask_path_input.as_ref() {
                        input.update(cx, |state, cx| {
                            state.set_value(path_str.clone(), window, cx);
                        });
                    }
                } else {
                    this.semantic_input_image_path = path_str.clone();
                    if let Some(input) = this.semantic_input_image_path_input.as_ref() {
                        input.update(cx, |state, cx| {
                            state.set_value(path_str.clone(), window, cx);
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn open_semantic_mask_painter(&mut self, cx: &mut Context<Self>) {
        let (clip_id, canvas_w, canvas_h) = {
            let gs = self.global.read(cx);
            (
                gs.selected_semantic_clip_id().unwrap_or(0),
                gs.canvas_w.round().max(1.0) as u32,
                gs.canvas_h.round().max(1.0) as u32,
            )
        };
        if clip_id == 0 {
            self.semantic_generate_status = "No semantic clip selected.".to_string();
            return;
        }
        let width = self
            .semantic_output_width
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|v| *v > 0)
            .unwrap_or(canvas_w);
        let height = self
            .semantic_output_height
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|v| *v > 0)
            .unwrap_or(canvas_h);
        let input_image_path = self.semantic_input_image_path.trim().to_string();
        self.semantic_mask_painter
            .open_for_semantic(input_image_path.as_str(), width, height);
    }

    pub(super) fn save_semantic_mask_from_painter(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (clip_id, output_dir) = {
            let gs = self.global.read(cx);
            (
                gs.selected_semantic_clip_id().unwrap_or(0),
                gs.generated_media_dir_for_semantic_mode("image"),
            )
        };
        if clip_id == 0 {
            self.semantic_generate_status = "No semantic clip selected.".to_string();
            return;
        }
        let output_path = Self::next_semantic_generated_mask_path(&output_dir, clip_id);
        match self.semantic_mask_painter.save_mask_png(&output_path) {
            Ok(()) => {
                let path_str = output_path.to_string_lossy().to_string();
                self.semantic_input_mask_path = path_str.clone();
                if let Some(input) = self.semantic_input_mask_path_input.as_ref() {
                    input.update(cx, |state, cx| {
                        state.set_value(path_str.clone(), window, cx);
                    });
                }
                if self.semantic_input_image_path.trim().is_empty() {
                    self.semantic_generate_status = format!(
                        "Mask saved: {}. Set Input Image File before generation.",
                        output_path.display()
                    );
                } else {
                    self.semantic_generate_status =
                        format!("Mask saved: {}.", output_path.display());
                }
                self.semantic_mask_painter.close();
            }
            Err(err) => {
                self.semantic_generate_status = err;
            }
        }
    }

    pub(super) fn render_semantic_editor_panel(
        &self,
        has_clip_selection: bool,
        has_subtitle_selection: bool,
        selected_semantic_id: Option<u64>,
        cx: &Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let has_semantic_selection = selected_semantic_id.is_some();
        if has_semantic_selection && !has_clip_selection && !has_subtitle_selection {
            let semantic_duration_sec = self
                .global
                .read(cx)
                .get_selected_semantic_duration_sec()
                .unwrap_or(0.0);
            let semantic_asset_mode = self
                .global
                .read(cx)
                .get_selected_semantic_asset_mode()
                .unwrap_or_else(|| "video".to_string());
            let semantic_model = self
                .global
                .read(cx)
                .get_selected_semantic_model()
                .unwrap_or_else(|| "veo_3_1".to_string());
            let semantic_model_route_key = Self::semantic_media_route_key(semantic_model.as_str());
            let semantic_model_supports_mask_edit = semantic_model_route_key
                .as_deref()
                .map(Self::semantic_route_supports_openai_image_edit)
                .unwrap_or(false);
            let semantic_model_is_google_image = semantic_model_route_key
                .as_deref()
                .map(|route| {
                    route.starts_with("google/")
                        && Self::semantic_route_supports_image_generation(route)
                })
                .unwrap_or(false);
            let semantic_schema_mode_label =
                Self::semantic_schema_mode_label(semantic_asset_mode.as_str());
            let semantic_api_key_label =
                Self::semantic_api_key_label_for_model(semantic_model.as_str());
            let semantic_status = {
                let validation = self.global.read(cx).validate_selected_semantic_schema();
                Self::semantic_schema_status_text(self.semantic_schema_status.clone(), validation)
            };
            let semantic_type_elem = if let Some(input) = self.semantic_type_input.as_ref() {
                div()
                    .w_full()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .child(Input::new(input).h(px(30.0)).w_full())
                    .into_any_element()
            } else {
                div()
                    .h(px(30.0))
                    .w_full()
                    .rounded_sm()
                    .bg(white().opacity(0.05))
                    .into_any_element()
            };
            let semantic_input_elem = if let Some(input) = self.semantic_label_input.as_ref() {
                div()
                    .w_full()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .child(Input::new(input).h(px(30.0)).w_full())
                    .into_any_element()
            } else {
                div()
                    .h(px(30.0))
                    .w_full()
                    .rounded_sm()
                    .bg(white().opacity(0.05))
                    .into_any_element()
            };
            let semantic_prompt_elem = if let Some(input) = self.semantic_prompt_input.as_ref() {
                div()
                    .w_full()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .child(Input::new(input).h(px(88.0)).w_full())
                    .into_any_element()
            } else {
                div()
                    .h(px(88.0))
                    .w_full()
                    .rounded_sm()
                    .bg(white().opacity(0.05))
                    .into_any_element()
            };
            let asset_image_active = semantic_asset_mode.eq_ignore_ascii_case("image");
            let asset_video_active = !asset_image_active;
            let asset_image_btn = div()
                .h(px(26.0))
                .px_2()
                .rounded_sm()
                .border_1()
                .border_color(if asset_image_active {
                    rgba(0x4f8fffeb)
                } else {
                    rgba(0xffffff33)
                })
                .bg(if asset_image_active {
                    rgba(0x253c62c7)
                } else {
                    rgba(0xffffff14)
                })
                .text_xs()
                .text_color(white().opacity(0.9))
                .cursor_pointer()
                .child("IMAGE")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.global.update(cx, |gs, cx| {
                            gs.set_selected_semantic_asset_mode("image".to_string());
                            cx.notify();
                        });
                        cx.notify();
                    }),
                );
            let asset_video_btn = div()
                .h(px(26.0))
                .px_2()
                .rounded_sm()
                .border_1()
                .border_color(if asset_video_active {
                    rgba(0x4f8fffeb)
                } else {
                    rgba(0xffffff33)
                })
                .bg(if asset_video_active {
                    rgba(0x253c62c7)
                } else {
                    rgba(0xffffff14)
                })
                .text_xs()
                .text_color(white().opacity(0.9))
                .cursor_pointer()
                .child("VIDEO")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.global.update(cx, |gs, cx| {
                            gs.set_selected_semantic_asset_mode("video".to_string());
                            cx.notify();
                        });
                        cx.notify();
                    }),
                );
            let semantic_model_button = |label: &'static str,
                                         model_id: &'static str,
                                         active: bool| {
                div()
                    .h(px(26.0))
                    .px_2()
                    .rounded_sm()
                    .border_1()
                    .border_color(if active {
                        rgba(0x4f8fffeb)
                    } else {
                        rgba(0xffffff33)
                    })
                    .bg(if active {
                        rgba(0x253c62c7)
                    } else {
                        rgba(0xffffff14)
                    })
                    .text_xs()
                    .text_color(white().opacity(0.9))
                    .cursor_pointer()
                    .child(label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.global.update(cx, |gs, cx| {
                                if let Some(model_route_key) =
                                    Self::semantic_media_route_key(model_id)
                                {
                                    if Self::semantic_route_supports_image_generation(
                                        model_route_key.as_str(),
                                    ) {
                                        gs.set_selected_semantic_asset_mode("image".to_string());
                                    } else if Self::semantic_route_supports_video_generation(
                                        model_route_key.as_str(),
                                    ) {
                                        gs.set_selected_semantic_asset_mode("video".to_string());
                                    }
                                    gs.set_selected_semantic_model(model_route_key);
                                } else {
                                    gs.set_selected_semantic_asset_mode("image".to_string());
                                    gs.set_selected_semantic_model(model_id.to_string());
                                }
                                cx.notify();
                            });
                            cx.notify();
                        }),
                    )
            };
            let model_buttons = if asset_image_active {
                div()
                    .flex()
                    .items_center()
                    .justify_start()
                    .flex_wrap()
                    .gap_2()
                    .child(semantic_model_button(
                        "GPT-IMAGE 1",
                        "openai/gpt-image-1",
                        semantic_model.eq_ignore_ascii_case("gpt-image-1")
                            || semantic_model.eq_ignore_ascii_case("openai/gpt-image-1"),
                    ))
                    .child(semantic_model_button(
                        "GPT-IMAGE 1.5",
                        "openai/gpt-image-1.5",
                        semantic_model.eq_ignore_ascii_case("gpt-image-1.5")
                            || semantic_model.eq_ignore_ascii_case("openai/gpt-image-1.5"),
                    ))
                    .child(semantic_model_button(
                        "GPT-IMAGE 1 MINI",
                        "openai/gpt-image-1-mini",
                        semantic_model.eq_ignore_ascii_case("gpt-image-1-mini")
                            || semantic_model.eq_ignore_ascii_case("openai/gpt-image-1-mini"),
                    ))
                    .child(semantic_model_button(
                        "NANO BANANA 2",
                        "google/gemini-3.1-flash-image-preview",
                        semantic_model
                            .eq_ignore_ascii_case("google/gemini-3.1-flash-image-preview")
                            || semantic_model
                                .eq_ignore_ascii_case("gemini-3.1-flash-image-preview"),
                    ))
                    .child(semantic_model_button(
                        "NANO BANANA PRO",
                        "google/gemini-3-pro-image-preview",
                        semantic_model.eq_ignore_ascii_case("google/gemini-3-pro-image-preview")
                            || semantic_model.eq_ignore_ascii_case("gemini-3-pro-image-preview"),
                    ))
                    .child(semantic_model_button(
                        "NANO BANANA",
                        "google/gemini-2.5-flash-image",
                        semantic_model.eq_ignore_ascii_case("google/gemini-2.5-flash-image")
                            || semantic_model.eq_ignore_ascii_case("gemini-2.5-flash-image")
                            || semantic_model.eq_ignore_ascii_case("nanobanana")
                            || semantic_model.eq_ignore_ascii_case("google/nanobanana"),
                    ))
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .items_center()
                    .justify_start()
                    .flex_wrap()
                    .gap_2()
                    .child(semantic_model_button(
                        "SORA 2",
                        "openai/sora-2",
                        semantic_model.eq_ignore_ascii_case("openai/sora-2")
                            || semantic_model.eq_ignore_ascii_case("sora_2")
                            || semantic_model.eq_ignore_ascii_case("sora-2"),
                    ))
                    .child(semantic_model_button(
                        "SORA 2 PRO",
                        "openai/sora-2-pro",
                        semantic_model.eq_ignore_ascii_case("openai/sora-2-pro")
                            || semantic_model.eq_ignore_ascii_case("sora_2_pro")
                            || semantic_model.eq_ignore_ascii_case("sora-2-pro"),
                    ))
                    .child(semantic_model_button(
                        "VEO 3.1",
                        "veo_3_1",
                        semantic_model.eq_ignore_ascii_case("veo_3_1")
                            || semantic_model.eq_ignore_ascii_case("google/veo_3_1")
                            || semantic_model
                                .eq_ignore_ascii_case("google/veo-3.1-generate-preview"),
                    ))
                    .into_any_element()
            };
            let semantic_api_key_elem =
                if let Some(input) = self.semantic_image_api_key_input.as_ref() {
                    div()
                        .w_full()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|_, _, _, cx| {
                                cx.stop_propagation();
                            }),
                        )
                        .child(Input::new(input).h(px(30.0)).w_full().mask_toggle())
                        .into_any_element()
                } else {
                    div()
                        .h(px(30.0))
                        .w_full()
                        .rounded_sm()
                        .bg(white().opacity(0.05))
                        .into_any_element()
                };
            let semantic_input_image_path_elem =
                if let Some(input) = self.semantic_input_image_path_input.as_ref() {
                    div()
                        .w_full()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|_, _, _, cx| {
                                cx.stop_propagation();
                            }),
                        )
                        .child(Input::new(input).h(px(30.0)).w_full())
                        .into_any_element()
                } else {
                    div()
                        .h(px(30.0))
                        .w_full()
                        .rounded_sm()
                        .bg(white().opacity(0.05))
                        .into_any_element()
                };
            let semantic_input_mask_path_elem =
                if let Some(input) = self.semantic_input_mask_path_input.as_ref() {
                    div()
                        .w_full()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|_, _, _, cx| {
                                cx.stop_propagation();
                            }),
                        )
                        .child(Input::new(input).h(px(30.0)).w_full())
                        .into_any_element()
                } else {
                    div()
                        .h(px(30.0))
                        .w_full()
                        .rounded_sm()
                        .bg(white().opacity(0.05))
                        .into_any_element()
                };
            let semantic_output_width_elem =
                if let Some(input) = self.semantic_output_width_input.as_ref() {
                    div()
                        .w_full()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|_, _, _, cx| {
                                cx.stop_propagation();
                            }),
                        )
                        .child(Input::new(input).h(px(30.0)).w_full())
                        .into_any_element()
                } else {
                    div()
                        .h(px(30.0))
                        .w_full()
                        .rounded_sm()
                        .bg(white().opacity(0.05))
                        .into_any_element()
                };
            let semantic_output_height_elem =
                if let Some(input) = self.semantic_output_height_input.as_ref() {
                    div()
                        .w_full()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|_, _, _, cx| {
                                cx.stop_propagation();
                            }),
                        )
                        .child(Input::new(input).h(px(30.0)).w_full())
                        .into_any_element()
                } else {
                    div()
                        .h(px(30.0))
                        .w_full()
                        .rounded_sm()
                        .bg(white().opacity(0.05))
                        .into_any_element()
                };
            let semantic_resolution_select_elem =
                if let Some(select) = self.semantic_resolution_select.as_ref() {
                    Select::new(select)
                        .placeholder("Select resolution")
                        .menu_width(px(220.0))
                        .into_any_element()
                } else {
                    div()
                        .h(px(30.0))
                        .w_full()
                        .rounded_sm()
                        .bg(white().opacity(0.05))
                        .text_xs()
                        .text_color(white().opacity(0.6))
                        .px_2()
                        .child("No presets for current model")
                        .into_any_element()
                };
            let semantic_resolution_hint =
                semantic_model_route_key
                    .as_ref()
                    .and_then(|model_route_key| {
                        let selected_resolution = self.semantic_selected_resolution.trim();
                        if selected_resolution.is_empty() {
                            return None;
                        }
                        let catalog = model_resolution_catalog();
                        if asset_image_active {
                            let aspect_table = Self::semantic_lookup_image_aspect_map(
                                &catalog,
                                model_route_key.as_str(),
                            )?;
                            let (aspect_ratio, preset) = Self::semantic_find_image_preset(
                                &aspect_table,
                                selected_resolution,
                                self.semantic_current_output_size(),
                            )?;
                            return Some(format!(
                                "Preset {} @ {} -> {} ({} tokens)",
                                selected_resolution, aspect_ratio, preset.size, preset.token_count
                            ));
                        }
                        let constraints = Self::semantic_lookup_video_constraints(
                            &catalog,
                            model_route_key.as_str(),
                        )?;
                        let rule = Self::semantic_find_video_constraint(
                            &constraints,
                            selected_resolution,
                        )?;
                        Some(format!(
                            "{} duration(s): {}",
                            selected_resolution,
                            rule.duration_sec
                                .iter()
                                .map(|sec| sec.to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ))
                    });
            let semantic_pick_file_button = |pick_mask: bool| {
                div()
                    .h(px(26.0))
                    .px_2()
                    .rounded_sm()
                    .border_1()
                    .border_color(white().opacity(0.2))
                    .bg(white().opacity(0.06))
                    .text_xs()
                    .text_color(white().opacity(0.85))
                    .cursor_pointer()
                    .child("Browse")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.prompt_semantic_local_file_path(pick_mask, window, cx);
                            cx.notify();
                        }),
                    )
            };
            let semantic_draw_mask_button = div()
                .h(px(26.0))
                .px_2()
                .rounded_sm()
                .border_1()
                .border_color(white().opacity(0.2))
                .bg(white().opacity(0.1))
                .text_xs()
                .text_color(white().opacity(0.92))
                .cursor_pointer()
                .child("Draw Mask")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.open_semantic_mask_painter(cx);
                        cx.notify();
                    }),
                );
            let default_canvas_size = {
                let gs = self.global.read(cx);
                format!(
                    "{}x{}",
                    gs.canvas_w.round().max(1.0) as u32,
                    gs.canvas_h.round().max(1.0) as u32
                )
            };
            let semantic_generation_inputs = if asset_image_active {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.7))
                            .child("Resolution Preset"),
                    )
                    .child(semantic_resolution_select_elem)
                    .child(div().text_xs().text_color(white().opacity(0.58)).child(
                        semantic_resolution_hint.clone().unwrap_or_else(|| {
                            "Select a preset to auto-fill Width/Height.".to_string()
                        }),
                    ))
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.7))
                            .child("Output Size (Optional Override)"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(white().opacity(0.62))
                                            .child("Width"),
                                    )
                                    .child(semantic_output_width_elem),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(white().opacity(0.62))
                                            .child("Height"),
                                    )
                                    .child(semantic_output_height_elem),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.58))
                            .child(format!(
                                "Default follows Display Settings: {default_canvas_size}"
                            )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.7))
                            .child("Input Image File (Optional)"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().child(semantic_input_image_path_elem))
                            .child(semantic_pick_file_button(false)),
                    )
                    .child(if semantic_model_supports_mask_edit {
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.7))
                                    .child("Mask File (Optional)"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(div().flex_1().child(semantic_input_mask_path_elem))
                                    .child(semantic_pick_file_button(true))
                                    .child(semantic_draw_mask_button),
                            )
                    } else if semantic_model_is_google_image {
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.62))
                            .child("Please use text instructions as semantic masking edits.")
                    } else {
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.62))
                            .child("Mask file editing is currently available only for GPT-IMAGE 1.")
                    })
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.7))
                            .child("Resolution Preset"),
                    )
                    .child(semantic_resolution_select_elem)
                    .child(div().text_xs().text_color(white().opacity(0.58)).child(
                        semantic_resolution_hint.unwrap_or_else(|| {
                            "Select a preset to auto-fill Width/Height.".to_string()
                        }),
                    ))
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.7))
                            .child("Output Size"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(white().opacity(0.62))
                                            .child("Width"),
                                    )
                                    .child(semantic_output_width_elem),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(white().opacity(0.62))
                                            .child("Height"),
                                    )
                                    .child(semantic_output_height_elem),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.58))
                            .child(format!(
                                "Default follows Display Settings: {default_canvas_size}"
                            )),
                    )
                    .into_any_element()
            };
            let semantic_generate_controls = div()
                .flex()
                .items_center()
                .justify_start()
                .gap_2()
                .child(
                    div()
                        .h(px(26.0))
                        .px_2()
                        .rounded_sm()
                        .border_1()
                        .border_color(white().opacity(0.2))
                        .bg(white().opacity(0.1))
                        .text_xs()
                        .text_color(white().opacity(0.9))
                        .cursor_pointer()
                        .child(if asset_image_active {
                            "Generate Image -> Media Pool"
                        } else {
                            "Generate Video -> Media Pool"
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.generate_semantic_image_to_media_pool(cx);
                                cx.notify();
                            }),
                        ),
                )
                .into_any_element();
            let semantic_schema_elem = if let Some(input) = self.semantic_schema_input.as_ref() {
                div()
                    .w_full()
                    .h(px(140.0))
                    .rounded_sm()
                    .border_1()
                    .border_color(white().opacity(0.12))
                    .bg(rgb(0x0b1020))
                    .overflow_hidden()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .child(Input::new(input).h_full().w_full())
                    .into_any_element()
            } else {
                div()
                    .h(px(140.0))
                    .w_full()
                    .rounded_sm()
                    .bg(white().opacity(0.05))
                    .into_any_element()
            };
            let semantic_schema_controls = div()
                .flex()
                .items_center()
                .justify_start()
                .gap_2()
                .child(
                    div()
                        .h(px(26.0))
                        .px_2()
                        .rounded_sm()
                        .border_1()
                        .border_color(white().opacity(0.2))
                        .bg(white().opacity(0.1))
                        .text_xs()
                        .text_color(white().opacity(0.9))
                        .cursor_pointer()
                        .child("Apply JSON")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.apply_semantic_schema_json(window, cx);
                                cx.notify();
                            }),
                        ),
                )
                .child(
                    div()
                        .h(px(26.0))
                        .px_2()
                        .rounded_sm()
                        .border_1()
                        .border_color(white().opacity(0.2))
                        .bg(white().opacity(0.06))
                        .text_xs()
                        .text_color(white().opacity(0.82))
                        .cursor_pointer()
                        .child("Expand")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.semantic_schema_modal_open = true;
                                cx.notify();
                            }),
                        ),
                )
                .into_any_element();
            return Some(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(white().opacity(0.5))
                            .child("SEMANTIC LAYER"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.7))
                                    .child("Type"),
                            )
                            .child(semantic_type_elem)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.7))
                                    .child("Label"),
                            )
                            .child(semantic_input_elem)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.7))
                                    .child("Prompt"),
                            )
                            .child(semantic_prompt_elem)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.7))
                                    .child("Asset Type"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_start()
                                    .gap_2()
                                    .child(asset_image_btn)
                                    .child(asset_video_btn),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.7))
                                    .child("Model"),
                            )
                            .child(model_buttons)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.7))
                                    .child(semantic_api_key_label),
                            )
                            .child(semantic_api_key_elem)
                            .child(semantic_generation_inputs)
                            .child(semantic_generate_controls)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.62))
                                    .child(self.semantic_generate_status.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.7))
                                    .child(format!(
                                        "Prompt Schema JSON ({semantic_schema_mode_label})"
                                    )),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.6))
                                    .child(format!(
                                        "duration_sec follows semantic layer length: {:.2}s",
                                        semantic_duration_sec
                                    )),
                            )
                            .child(semantic_schema_elem)
                            .child(semantic_schema_controls)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.62))
                                    .child(semantic_status),
                            ),
                    )
                    .into_any_element(),
            );
        }
        None
    }

    pub(super) fn generate_semantic_image_to_media_pool(&mut self, cx: &mut Context<Self>) {
        let (
            clip_id,
            asset_mode,
            model,
            prompt,
            output_dir,
            canvas_w,
            canvas_h,
            semantic_duration_sec,
        ) = {
            let gs = self.global.read(cx);
            let clip_id = gs.selected_semantic_clip_id().unwrap_or(0);
            let asset_mode = gs
                .get_selected_semantic_asset_mode()
                .unwrap_or_else(|| "video".to_string());
            let model = gs
                .get_selected_semantic_model()
                .unwrap_or_else(|| "gpt-image-1".to_string());
            let prompt = gs.get_selected_semantic_prompt_text().unwrap_or_default();
            let output_dir = gs.generated_media_dir_for_semantic_mode(asset_mode.as_str());
            (
                clip_id,
                asset_mode,
                model,
                prompt,
                output_dir,
                gs.canvas_w,
                gs.canvas_h,
                gs.get_selected_semantic_duration_sec().unwrap_or(2.0),
            )
        };
        let api_key = self.semantic_image_api_key.trim().to_string();
        let input_image_path = self.semantic_input_image_path.trim().to_string();
        let mut mask_path = self.semantic_input_mask_path.trim().to_string();
        let width_raw = self.semantic_output_width.trim().to_string();
        let height_raw = self.semantic_output_height.trim().to_string();
        if api_key.is_empty() {
            self.semantic_generate_status =
                Self::semantic_api_key_required_message_for_model(model.as_str()).to_string();
            return;
        }

        if clip_id == 0 {
            self.semantic_generate_status = "No semantic clip selected.".to_string();
            return;
        }
        if prompt.trim().is_empty() {
            self.semantic_generate_status = "Prompt cannot be empty.".to_string();
            return;
        }
        let Some(model_route_key) = Self::semantic_media_route_key(model.as_str()) else {
            self.semantic_generate_status = if asset_mode.eq_ignore_ascii_case("image") {
                "Model must be GPT-IMAGE 1 / 1.5 / 1 MINI / NANO BANANA 2 / NANO BANANA PRO / NANO BANANA for this action.".to_string()
            } else {
                "Model must be SORA 2 / SORA 2 PRO / VEO 3.1 for this action.".to_string()
            };
            return;
        };

        // Route to image or video generation path without changing the existing inspector UI.
        if asset_mode.eq_ignore_ascii_case("image") {
            if !Self::semantic_route_supports_image_generation(model_route_key.as_str()) {
                self.semantic_generate_status =
                    "Selected model does not support IMAGE generation in this action.".to_string();
                return;
            }
            if !Self::semantic_route_supports_openai_image_edit(model_route_key.as_str()) {
                // Ignore stale mask paths for semantic-mask models (for example Nano Banana).
                mask_path.clear();
            }
            if !mask_path.is_empty() && input_image_path.is_empty() {
                self.semantic_generate_status = "Mask File requires Input Image File.".to_string();
                return;
            }
            if !input_image_path.is_empty() && !Path::new(&input_image_path).is_file() {
                self.semantic_generate_status =
                    "Input Image File does not exist (or is not a file).".to_string();
                return;
            }
            if !mask_path.is_empty() && !Path::new(&mask_path).is_file() {
                self.semantic_generate_status =
                    "Mask File does not exist (or is not a file).".to_string();
                return;
            }

            let output_path =
                Self::next_semantic_generated_image_path(&output_dir, clip_id, &model_route_key);
            let default_width = canvas_w.round().max(1.0) as u32;
            let default_height = canvas_h.round().max(1.0) as u32;
            let image_width = if width_raw.is_empty() {
                default_width
            } else {
                match width_raw.parse::<u32>() {
                    Ok(value) if value > 0 => value,
                    _ => {
                        self.semantic_generate_status =
                            "Width must be a positive integer.".to_string();
                        return;
                    }
                }
            };
            let image_height = if height_raw.is_empty() {
                default_height
            } else {
                match height_raw.parse::<u32>() {
                    Ok(value) if value > 0 => value,
                    _ => {
                        self.semantic_generate_status =
                            "Height must be a positive integer.".to_string();
                        return;
                    }
                }
            };
            let image_size = format!("{}x{}", image_width, image_height);
            self.global.update(cx, |gs, cx| {
                gs.set_selected_semantic_image_size(Some(image_width), Some(image_height));
                cx.notify();
            });
            self.semantic_generate_status = format!("Generating image ({image_size})...");
            let global = self.global.clone();
            let input_image_path = if input_image_path.is_empty() {
                None
            } else {
                Some(input_image_path)
            };
            let mask_path = if mask_path.is_empty() {
                None
            } else {
                Some(mask_path)
            };

            // Run network + file IO in background, then push generated image into Media Pool.
            cx.spawn(async move |view, cx| {
                let task = cx
                    .background_spawn(async move {
                        if let Some(parent) = output_path.parent() {
                            fs::create_dir_all(parent).map_err(|err| {
                                format!(
                                    "Failed to create output directory '{}': {err}",
                                    parent.display()
                                )
                            })?;
                        }
                        Self::request_image_via_media_protocol(
                            api_key.as_str(),
                            model_route_key.as_str(),
                            prompt.as_str(),
                            &output_path,
                            image_size.as_str(),
                            input_image_path,
                            mask_path,
                        )
                    })
                    .await;

                let _ = view.update(cx, |this, cx| {
                    match task {
                        Ok(path) => {
                            let path_str = path.to_string_lossy().to_string();
                            let duration = get_media_duration(&path_str);
                            let insert_result = global.update(cx, |gs, cx| {
                                let result = gs.add_generated_asset_to_semantic_timeline(
                                    clip_id,
                                    path.clone(),
                                    duration,
                                );
                                if result.is_ok() {
                                    cx.emit(MediaPoolUiEvent::StateChanged);
                                    cx.notify();
                                }
                                result
                            });
                            match insert_result {
                                Ok(()) => {
                                    this.semantic_generate_status = format!(
                                        "Generated and placed on semantic timeline: {}",
                                        path.file_name()
                                            .and_then(|name| name.to_str())
                                            .unwrap_or("generated.png")
                                    );
                                }
                                Err(err) => {
                                    this.semantic_generate_status = err.to_string();
                                }
                            }
                        }
                        Err(err) => {
                            this.semantic_generate_status = err;
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
            return;
        }

        if !Self::semantic_route_supports_video_generation(model_route_key.as_str()) {
            self.semantic_generate_status =
                "Selected model does not support VIDEO generation in this action.".to_string();
            return;
        }

        let output_path =
            Self::next_semantic_generated_video_path(&output_dir, clip_id, &model_route_key);
        let (default_width, default_height) =
            Self::semantic_default_video_size_for_provider(model_route_key.as_str(), "video")
                .unwrap_or_else(|| {
                    (
                        canvas_w.round().max(1.0) as u32,
                        canvas_h.round().max(1.0) as u32,
                    )
                });
        let video_width = if width_raw.is_empty() {
            default_width
        } else {
            match width_raw.parse::<u32>() {
                Ok(value) if value > 0 => value,
                _ => {
                    self.semantic_generate_status = "Width must be a positive integer.".to_string();
                    return;
                }
            }
        };
        let video_height = if height_raw.is_empty() {
            default_height
        } else {
            match height_raw.parse::<u32>() {
                Ok(value) if value > 0 => value,
                _ => {
                    self.semantic_generate_status =
                        "Height must be a positive integer.".to_string();
                    return;
                }
            }
        };
        let video_size = format!("{}x{}", video_width, video_height);
        let video_duration_sec = semantic_duration_sec.max(0.1);
        self.semantic_generate_status = format!(
            "Generating video ({video_size}, {:.2}s)...",
            video_duration_sec
        );
        let global = self.global.clone();

        cx.spawn(async move |view, cx| {
            let task = cx
                .background_spawn(async move {
                    if let Some(parent) = output_path.parent() {
                        fs::create_dir_all(parent).map_err(|err| {
                            format!(
                                "Failed to create output directory '{}': {err}",
                                parent.display()
                            )
                        })?;
                    }
                    Self::request_video_via_media_protocol(
                        api_key.as_str(),
                        model_route_key.as_str(),
                        prompt.as_str(),
                        &output_path,
                        video_size.as_str(),
                        video_duration_sec,
                    )
                })
                .await;

            let _ = view.update(cx, |this, cx| {
                match task {
                    Ok(path) => {
                        let path_str = path.to_string_lossy().to_string();
                        let duration = get_media_duration(&path_str);
                        let insert_result = global.update(cx, |gs, cx| {
                            let result = gs.add_generated_asset_to_semantic_timeline(
                                clip_id,
                                path.clone(),
                                duration,
                            );
                            if result.is_ok() {
                                cx.emit(MediaPoolUiEvent::StateChanged);
                                cx.notify();
                            }
                            result
                        });
                        match insert_result {
                            Ok(()) => {
                                this.semantic_generate_status = format!(
                                    "Generated and placed on semantic timeline: {}",
                                    path.file_name()
                                        .and_then(|name| name.to_str())
                                        .unwrap_or("generated.mp4")
                                );
                            }
                            Err(err) => {
                                this.semantic_generate_status = err.to_string();
                            }
                        }
                    }
                    Err(err) => {
                        this.semantic_generate_status = err;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_semantic_mask_painter_modal_overlay(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        if !self.semantic_mask_painter.is_open() {
            return div();
        }

        let layout = self
            .semantic_mask_painter
            .compute_layout(window.viewport_size());
        let card_x = f32::from(layout.card_bounds.origin.x);
        let card_y = f32::from(layout.card_bounds.origin.y);
        let card_w = f32::from(layout.card_bounds.size.width);
        let card_h = f32::from(layout.card_bounds.size.height);
        let slot_x = f32::from(layout.canvas_slot_bounds.origin.x) - card_x;
        let slot_y = f32::from(layout.canvas_slot_bounds.origin.y) - card_y;
        let slot_w = f32::from(layout.canvas_slot_bounds.size.width);
        let slot_h = f32::from(layout.canvas_slot_bounds.size.height);
        let draw_x = f32::from(layout.draw_bounds.origin.x) - card_x;
        let draw_y = f32::from(layout.draw_bounds.origin.y) - card_y;
        let draw_w = f32::from(layout.draw_bounds.size.width);
        let draw_h = f32::from(layout.draw_bounds.size.height);
        let painter_snapshot = self.semantic_mask_painter.clone();
        let (mask_w, mask_h) = self.semantic_mask_painter.mask_size();
        let brush_size = self.semantic_mask_painter.brush_radius_px();
        let source_text = if self.semantic_mask_painter.has_source_image() {
            format!("Source: {}", self.semantic_mask_painter.source_image_path())
        } else {
            "Source: none (mask will use output size)".to_string()
        };
        let brush_active = matches!(
            self.semantic_mask_painter.active_tool(),
            MaskPaintTool::Brush
        );
        let eraser_active = matches!(
            self.semantic_mask_painter.active_tool(),
            MaskPaintTool::Eraser
        );

        let tool_btn = |label: &'static str, active: bool| {
            div()
                .h(px(28.0))
                .px_2()
                .rounded_sm()
                .border_1()
                .border_color(if active {
                    rgba(0x4f8fffeb)
                } else {
                    rgba(0xffffff33)
                })
                .bg(if active {
                    rgba(0x253c62c7)
                } else {
                    rgba(0xffffff12)
                })
                .text_xs()
                .text_color(white().opacity(0.92))
                .cursor_pointer()
                .child(label)
        };

        let small_btn = |label: &'static str| {
            div()
                .h(px(28.0))
                .px_2()
                .rounded_sm()
                .border_1()
                .border_color(white().opacity(0.2))
                .bg(white().opacity(0.08))
                .text_xs()
                .text_color(white().opacity(0.9))
                .cursor_pointer()
                .child(label)
        };

        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right_0()
            .bg(gpui_component::black().opacity(0.55))
            .on_mouse_move(cx.listener(|this, evt: &gpui::MouseMoveEvent, window, cx| {
                if !this.semantic_mask_painter.has_active_stroke() {
                    return;
                }
                let layout = this
                    .semantic_mask_painter
                    .compute_layout(window.viewport_size());
                if this
                    .semantic_mask_painter
                    .append_stroke(&layout, evt.position)
                {
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &gpui::MouseUpEvent, _, cx| {
                    if this.semantic_mask_painter.has_active_stroke() {
                        this.semantic_mask_painter.end_stroke();
                        cx.notify();
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.semantic_mask_painter.has_active_stroke() {
                        this.semantic_mask_painter.end_stroke();
                    }
                    this.semantic_mask_painter.close();
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(card_x))
                    .top(px(card_y))
                    .w(px(card_w))
                    .h(px(card_h))
                    .rounded_md()
                    .bg(rgb(0x1f1f23))
                    .border_1()
                    .border_color(white().opacity(0.18))
                    .overflow_hidden()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(14.0))
                            .right(px(14.0))
                            .top(px(10.0))
                            .h(px(28.0))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(white().opacity(0.92))
                                    .child("Mask Painter"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.7))
                                    .child(format!("Mask: {}x{}", mask_w, mask_h)),
                            ),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(slot_x))
                            .top(px(slot_y))
                            .w(px(slot_w))
                            .h(px(slot_h))
                            .rounded_sm()
                            .border_1()
                            .border_color(white().opacity(0.12))
                            .bg(rgb(0x13151b)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(draw_x))
                            .top(px(draw_y))
                            .w(px(draw_w))
                            .h(px(draw_h))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, evt: &MouseDownEvent, window, cx| {
                                    let layout = this
                                        .semantic_mask_painter
                                        .compute_layout(window.viewport_size());
                                    if this
                                        .semantic_mask_painter
                                        .begin_stroke(&layout, evt.position)
                                    {
                                        cx.notify();
                                    }
                                }),
                            )
                            .child(
                                gpui::canvas(
                                    move |_bounds, _window, _cx| painter_snapshot.clone(),
                                    move |bounds, painter, window, _cx| {
                                        painter.paint_canvas(bounds, window);
                                    },
                                )
                                .w_full()
                                .h_full(),
                            ),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(14.0))
                            .right(px(14.0))
                            .bottom(px(12.0))
                            .h(px(118.0))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_start()
                                    .gap_2()
                                    .child(tool_btn("Brush", brush_active).on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.semantic_mask_painter
                                                .set_active_tool(MaskPaintTool::Brush);
                                            cx.notify();
                                        }),
                                    ))
                                    .child(tool_btn("Eraser", eraser_active).on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.semantic_mask_painter
                                                .set_active_tool(MaskPaintTool::Eraser);
                                            cx.notify();
                                        }),
                                    ))
                                    .child(small_btn("Brush -").on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            let next =
                                                this.semantic_mask_painter.brush_radius_px() - 2.0;
                                            this.semantic_mask_painter.set_brush_radius_px(next);
                                            cx.notify();
                                        }),
                                    ))
                                    .child(small_btn("Brush +").on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            let next =
                                                this.semantic_mask_painter.brush_radius_px() + 2.0;
                                            this.semantic_mask_painter.set_brush_radius_px(next);
                                            cx.notify();
                                        }),
                                    ))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(white().opacity(0.7))
                                            .child(format!("Radius: {:.0}px", brush_size)),
                                    )
                                    .child(small_btn("Undo").on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.semantic_mask_painter.undo();
                                            cx.notify();
                                        }),
                                    ))
                                    .child(small_btn("Clear").on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.semantic_mask_painter.clear();
                                            cx.notify();
                                        }),
                                    )),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(white().opacity(0.62))
                                    .child(source_text),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .child(div().text_xs().text_color(white().opacity(0.62)).child(
                                        self.semantic_mask_painter.status_text().to_string(),
                                    ))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_end()
                                            .gap_2()
                                            .child(small_btn("Close").on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(|this, _, _, cx| {
                                                    this.semantic_mask_painter.close();
                                                    cx.notify();
                                                }),
                                            ))
                                            .child(small_btn("Save Mask").on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(|this, _, window, cx| {
                                                    this.save_semantic_mask_from_painter(
                                                        window, cx,
                                                    );
                                                    cx.notify();
                                                }),
                                            )),
                                    ),
                            ),
                    ),
            )
    }

    pub(super) fn render_semantic_schema_modal_overlay(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        if !self.semantic_schema_modal_open {
            return div();
        }

        let modal_editor_elem = if let Some(input) = self.semantic_schema_input.as_ref() {
            div()
                .w_full()
                .h(px(470.0))
                .rounded_sm()
                .border_1()
                .border_color(white().opacity(0.16))
                .bg(rgb(0x0b1020))
                .overflow_hidden()
                .child(Input::new(input).h_full().w_full())
                .into_any_element()
        } else {
            div()
                .w_full()
                .h(px(470.0))
                .rounded_sm()
                .bg(white().opacity(0.05))
                .into_any_element()
        };
        let modal_controls = div()
            .flex()
            .items_center()
            .flex_wrap()
            .justify_start()
            .gap_2()
            .child(
                div()
                    .h(px(28.0))
                    .px_2()
                    .rounded_sm()
                    .border_1()
                    .border_color(white().opacity(0.2))
                    .bg(white().opacity(0.1))
                    .text_xs()
                    .text_color(white().opacity(0.9))
                    .cursor_pointer()
                    .child("Apply JSON")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.apply_semantic_schema_json(window, cx);
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .h(px(28.0))
                    .px_2()
                    .rounded_sm()
                    .border_1()
                    .border_color(white().opacity(0.2))
                    .bg(white().opacity(0.06))
                    .text_xs()
                    .text_color(white().opacity(0.82))
                    .cursor_pointer()
                    .child("Close")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.semantic_schema_modal_open = false;
                            cx.notify();
                        }),
                    ),
            )
            .into_any_element();
        let modal_title = format!(
            "SEMANTIC {} SCHEMA (Expanded)",
            Self::semantic_schema_mode_label(self.semantic_schema_mode.as_str())
        );

        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right_0()
            .bg(gpui_component::black().opacity(0.55))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.semantic_schema_modal_open = false;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(920.0))
                    .h(px(640.0))
                    .rounded_md()
                    .bg(rgb(0x1f1f23))
                    .border_1()
                    .border_color(white().opacity(0.16))
                    .p_3()
                    .overflow_hidden()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        div()
                            .w_full()
                            .h_full()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .overflow_y_scrollbar()
                            .child(Self::layer_fx_script_editor_wrap(
                                modal_title.as_str(),
                                modal_editor_elem,
                                modal_controls,
                                self.semantic_schema_status.clone(),
                            )),
                    ),
            )
    }
}
