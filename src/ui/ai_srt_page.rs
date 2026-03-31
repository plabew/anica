// =========================================
// =========================================
// src/ui/ai_srt_page.rs
use gpui::{
    Context, Entity, IntoElement, MouseButton, PathPromptOptions, Render, SharedString,
    Subscription, Window, div, prelude::*, px, rgba,
};
use gpui_component::{
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
    slider::{Slider, SliderEvent, SliderState},
    white,
};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::global_state::GlobalState;
use crate::core::project_state::default_project_dir;

#[derive(Clone, Debug)]
struct AiModelOption {
    id: String,
    label: String,
    model_dir: PathBuf,
    encoder_path: PathBuf,
    decoder_path: PathBuf,
    tokenizer_path: PathBuf,
}

#[derive(Clone, Debug)]
struct AiLanguageOption {
    code: String,
    label: String,
}

#[derive(Clone, Debug)]
struct AiEngineOption {
    id: String,
    label: String,
}

#[derive(Clone, Debug)]
struct AiProviderOption {
    id: String,
    label: String,
}

#[derive(Clone, Debug)]
struct AiRemoteModelPreset {
    folder_id: String,
    label: String,
    repo: String,
}

// Keep extra manifest metadata optional so model packs can carry provenance notes without affecting runtime logic.
#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
struct AiModelManifest {
    id: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    runtime_kind: Option<String>,
    encoder: String,
    decoder: String,
    tokenizer: String,
    #[serde(default = "default_model_config_ref")]
    model_config: String,
    #[serde(default = "default_preprocessor_config_ref")]
    preprocessor_config: String,
    #[serde(default)]
    model_author: Option<String>,
    #[serde(default)]
    model_repo: Option<String>,
    #[serde(default)]
    precision: Option<String>,
    #[serde(default)]
    variant: Option<String>,
    #[serde(default)]
    decoder_variant: Option<String>,
}

fn default_model_config_ref() -> String {
    "config.json".to_string()
}

fn default_preprocessor_config_ref() -> String {
    "preprocessor_config.json".to_string()
}

// Keep model picker strict to runtime kinds currently supported by the subtitle engine binary.
fn is_supported_runtime_kind(runtime_kind: Option<&str>) -> bool {
    let normalized = runtime_kind
        .unwrap_or("whisper_seq2seq_v1")
        .trim()
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "whisper_seq2seq_v1" | "whisper_seq2seq" | "whisper"
    )
}

impl SelectItem for AiModelOption {
    type Value = String;

    fn title(&self) -> SharedString {
        SharedString::from(self.label.clone())
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

impl SelectItem for AiLanguageOption {
    type Value = String;

    fn title(&self) -> SharedString {
        SharedString::from(self.label.clone())
    }

    fn value(&self) -> &Self::Value {
        &self.code
    }
}

impl SelectItem for AiEngineOption {
    type Value = String;

    fn title(&self) -> SharedString {
        SharedString::from(self.label.clone())
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

impl SelectItem for AiProviderOption {
    type Value = String;

    fn title(&self) -> SharedString {
        SharedString::from(self.label.clone())
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

pub struct AiSrtPage {
    pub global: Entity<GlobalState>,
    input_path: Option<String>,
    output_srt_path: String,
    output_txt_path: String,
    selected_provider_id: String,
    provider_options: Vec<AiProviderOption>,
    provider_select: Option<Entity<SelectState<SearchableVec<AiProviderOption>>>>,
    provider_select_sub: Option<Subscription>,
    selected_engine_id: String,
    cloud_engine_provider_id: String,
    engine_options: Vec<AiEngineOption>,
    engine_select: Option<Entity<SelectState<SearchableVec<AiEngineOption>>>>,
    engine_select_sub: Option<Subscription>,
    openai_api_key: String,
    gemini_api_key: String,
    assemblyai_api_key: String,
    openai_key_input: Option<Entity<InputState>>,
    openai_key_input_sub: Option<Subscription>,
    gemini_key_input: Option<Entity<InputState>>,
    gemini_key_input_sub: Option<Subscription>,
    assemblyai_key_input: Option<Entity<InputState>>,
    assemblyai_key_input_sub: Option<Subscription>,
    lang_code: String,
    language_model_id: String,
    language_options: Vec<AiLanguageOption>,
    language_select: Option<Entity<SelectState<SearchableVec<AiLanguageOption>>>>,
    language_select_sub: Option<Subscription>,
    import_after_generate: bool,
    is_running: bool,
    status_text: String,
    last_runtime_backend: String,
    selected_model_id: String,
    model_options: Vec<AiModelOption>,
    model_select: Option<Entity<SelectState<SearchableVec<AiModelOption>>>>,
    model_select_sub: Option<Subscription>,
    max_subtitle_duration_sec: f32,
    max_subtitle_chars: f32,
    timing_max_window_sec: f32,
    vad_db_offset: f32,
    vad_merge_gap_sec: f32,
    max_subtitle_duration_slider: Option<Entity<SliderState>>,
    max_subtitle_duration_sub: Option<Subscription>,
    max_subtitle_chars_slider: Option<Entity<SliderState>>,
    max_subtitle_chars_sub: Option<Subscription>,
    timing_max_window_slider: Option<Entity<SliderState>>,
    timing_max_window_sub: Option<Subscription>,
    vad_db_offset_slider: Option<Entity<SliderState>>,
    vad_db_offset_sub: Option<Subscription>,
    vad_merge_gap_slider: Option<Entity<SliderState>>,
    vad_merge_gap_sub: Option<Subscription>,
    remote_model_presets: Vec<AiRemoteModelPreset>,
    is_downloading_model: bool,
    download_status_text: String,
}

impl AiSrtPage {
    // Keep UI defaults aligned with engine defaults so first-run behavior is unchanged.
    const DEFAULT_MAX_SUBTITLE_DURATION_SEC: f32 = 6.0;
    const DEFAULT_MAX_SUBTITLE_CHARS: f32 = 42.0;
    const DEFAULT_TIMING_MAX_WINDOW_SEC: f32 = 15.0;
    const DEFAULT_VAD_DB_OFFSET: f32 = 8.0;
    const DEFAULT_VAD_MERGE_GAP_SEC: f32 = 0.30;

    pub fn new(global: Entity<GlobalState>) -> Self {
        let (default_srt, default_txt) = Self::default_output_paths();
        // Discover available model packs once at page startup.
        let model_options = Self::discover_model_options();
        let selected_model_id = Self::default_selected_model_id(&model_options);
        let provider_options = Self::provider_options();
        let selected_engine_id = "local_onnx".to_string();
        let selected_provider_id = Self::provider_for_engine(&selected_engine_id);
        Self {
            global,
            input_path: None,
            output_srt_path: default_srt,
            output_txt_path: default_txt,
            selected_provider_id: selected_provider_id.clone(),
            provider_options,
            provider_select: None,
            provider_select_sub: None,
            selected_engine_id,
            cloud_engine_provider_id: String::new(),
            engine_options: Self::engine_options(),
            engine_select: None,
            engine_select_sub: None,
            openai_api_key: String::new(),
            gemini_api_key: String::new(),
            assemblyai_api_key: String::new(),
            openai_key_input: None,
            openai_key_input_sub: None,
            gemini_key_input: None,
            gemini_key_input_sub: None,
            assemblyai_key_input: None,
            assemblyai_key_input_sub: None,
            lang_code: "auto".to_string(),
            language_model_id: String::new(),
            language_options: Vec::new(),
            language_select: None,
            language_select_sub: None,
            import_after_generate: true,
            is_running: false,
            status_text: "Pick a WAV file to start.".to_string(),
            last_runtime_backend: "N/A".to_string(),
            selected_model_id,
            model_options,
            model_select: None,
            model_select_sub: None,
            max_subtitle_duration_sec: Self::DEFAULT_MAX_SUBTITLE_DURATION_SEC,
            max_subtitle_chars: Self::DEFAULT_MAX_SUBTITLE_CHARS,
            timing_max_window_sec: Self::DEFAULT_TIMING_MAX_WINDOW_SEC,
            vad_db_offset: Self::DEFAULT_VAD_DB_OFFSET,
            vad_merge_gap_sec: Self::DEFAULT_VAD_MERGE_GAP_SEC,
            max_subtitle_duration_slider: None,
            max_subtitle_duration_sub: None,
            max_subtitle_chars_slider: None,
            max_subtitle_chars_sub: None,
            timing_max_window_slider: None,
            timing_max_window_sub: None,
            vad_db_offset_slider: None,
            vad_db_offset_sub: None,
            vad_merge_gap_slider: None,
            vad_merge_gap_sub: None,
            remote_model_presets: Self::remote_model_presets(),
            is_downloading_model: false,
            download_status_text: "No download running.".to_string(),
        }
    }

    // Build one timestamped SRT/TXT path pair in a target folder and avoid accidental overwrite by checking collisions.
    fn timestamped_output_paths_in(base_dir: &Path) -> (String, String) {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let mut seq: u32 = 0;
        loop {
            let suffix = if seq == 0 {
                stamp.to_string()
            } else {
                format!("{stamp}_{seq}")
            };
            let stem = format!("ai_transcript_{suffix}");
            let srt = base_dir.join(format!("{stem}.srt"));
            let txt = base_dir.join(format!("{stem}.txt"));
            if !srt.exists() && !txt.exists() {
                return (
                    srt.to_string_lossy().to_string(),
                    txt.to_string_lossy().to_string(),
                );
            }
            seq = seq.saturating_add(1);
        }
    }

    // Keep default export location under AnicaProjects while generating unique names.
    fn default_output_paths() -> (String, String) {
        let base_dir = default_project_dir().join("ai_srt");
        Self::timestamped_output_paths_in(&base_dir)
    }

    // Refresh output file names before each generation while preserving the currently selected output folder.
    fn refresh_output_paths_for_new_generation(&mut self) {
        let current_srt_dir = PathBuf::from(&self.output_srt_path)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| default_project_dir().join("ai_srt"));
        let current_txt_dir = PathBuf::from(&self.output_txt_path)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| default_project_dir().join("ai_srt"));
        if current_srt_dir == current_txt_dir {
            let (srt, txt) = Self::timestamped_output_paths_in(&current_srt_dir);
            self.output_srt_path = srt;
            self.output_txt_path = txt;
            return;
        }
        let (srt, _) = Self::timestamped_output_paths_in(&current_srt_dir);
        let (_, txt) = Self::timestamped_output_paths_in(&current_txt_dir);
        self.output_srt_path = srt;
        self.output_txt_path = txt;
    }

    // Resolve the shared ONNX model root where model packs are stored by folder name.
    fn model_root_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("crates")
            .join("ai-subtitle-engine")
            .join("src")
            .join("model")
            .join("onnx")
    }

    // Define built-in downloadable model packs so users can install Whisper models without manual file copying.
    fn remote_model_presets() -> Vec<AiRemoteModelPreset> {
        vec![
            AiRemoteModelPreset {
                folder_id: "whisper_large_v3_turbo".to_string(),
                label: "Whisper Large V3 Turbo FP16 (ONNX Community)".to_string(),
                repo: "onnx-community/whisper-large-v3-turbo".to_string(),
            },
            AiRemoteModelPreset {
                folder_id: "whisper_large_v3".to_string(),
                label: "Whisper Large V3 FP16 (ONNX Community)".to_string(),
                repo: "onnx-community/whisper-large-v3-ONNX".to_string(),
            },
        ]
    }

    // Build one raw file URL from Hugging Face repo and relative path.
    fn hf_resolve_url(repo: &str, relative_path: &str) -> String {
        format!(
            "https://huggingface.co/{repo}/resolve/main/{}",
            relative_path.trim_start_matches('/')
        )
    }

    // Download one required file and write it atomically so interrupted downloads never leave corrupted model files.
    fn download_required_file(
        repo: &str,
        relative_path: &str,
        dest_path: &Path,
    ) -> Result<(), String> {
        let url = Self::hf_resolve_url(repo, relative_path);
        let response = ureq::get(&url)
            .set("User-Agent", "anica-model-manager/1.0")
            .call()
            .map_err(|err| format!("Download failed for '{relative_path}': {err}"))?;
        let mut reader = response.into_reader();
        let tmp_path = dest_path.with_extension("download");
        let mut file = fs::File::create(&tmp_path)
            .map_err(|err| format!("Failed to create '{}': {err}", tmp_path.display()))?;
        std::io::copy(&mut reader, &mut file)
            .map_err(|err| format!("Failed to write '{}': {err}", tmp_path.display()))?;
        fs::rename(&tmp_path, dest_path).map_err(|err| {
            format!(
                "Failed to finalize '{}' -> '{}': {err}",
                tmp_path.display(),
                dest_path.display()
            )
        })?;
        Ok(())
    }

    // Try multiple candidate files and keep the first one that exists in the remote repo.
    fn download_first_existing_file(
        repo: &str,
        candidate_paths: &[&str],
        dest_dir: &Path,
    ) -> Result<String, String> {
        let mut last_error = String::new();
        for candidate in candidate_paths {
            let file_name = Path::new(candidate)
                .file_name()
                .and_then(|x| x.to_str())
                .ok_or_else(|| format!("Invalid candidate file path: '{candidate}'"))?;
            let dest_path = dest_dir.join(file_name);
            match Self::download_required_file(repo, candidate, &dest_path) {
                Ok(()) => return Ok(file_name.to_string()),
                Err(err) => {
                    last_error = err;
                }
            }
        }
        Err(format!(
            "None of the candidate files could be downloaded for repo '{repo}'. Last error: {last_error}"
        ))
    }

    // Try downloading optional ONNX sidecar data files used by external-data graphs; ignore misses safely.
    fn download_optional_sidecars(repo: &str, onnx_file_name: &str, dest_dir: &Path) {
        let sidecar_candidates = [
            format!("onnx/{onnx_file_name}_data"),
            format!("onnx/{onnx_file_name}.data"),
        ];
        for relative in sidecar_candidates {
            let file_name = Path::new(&relative)
                .file_name()
                .and_then(|x| x.to_str())
                .unwrap_or_default()
                .to_string();
            if file_name.is_empty() {
                continue;
            }
            let dest_path = dest_dir.join(file_name);
            let _ = Self::download_required_file(repo, &relative, &dest_path);
        }
    }

    // Build a runtime manifest that points local ONNX execution to the downloaded files.
    fn write_downloaded_manifest(
        preset: &AiRemoteModelPreset,
        model_dir: &Path,
        encoder_file: &str,
        decoder_file: &str,
    ) -> Result<(), String> {
        let precision = if encoder_file.contains("fp16") || decoder_file.contains("fp16") {
            "fp16"
        } else {
            "fp32"
        };
        // Keep decoder variant optional so UI only highlights special merged decoders.
        let is_merged_decoder = decoder_file.contains("merged");
        let mut manifest = serde_json::json!({
            "id": preset.folder_id,
            "display_name": preset.label,
            "runtime_kind": "whisper_seq2seq_v1",
            // Keep required Whisper frontend metadata in manifest so the runtime never depends on implicit defaults.
            "architecture": "whisper_seq2seq",
            "frontend": "whisper_log_mel",
            "sample_rate": 16000,
            "n_fft": 400,
            "hop_length": 160,
            "n_mels": 128,
            "chunk_length_sec": 30,
            "overlap_frames": 150,
            "max_decode_steps": 448,
            "model_author": "onnx-community",
            "model_repo": preset.repo,
            "precision": precision,
            "encoder": encoder_file,
            "decoder": decoder_file,
            "tokenizer": "tokenizer.json",
            "model_config": "config.json",
            "preprocessor_config": "preprocessor_config.json",
        });
        if is_merged_decoder {
            manifest["decoder_variant"] = serde_json::json!("merged");
        }
        let manifest_path = model_dir.join("manifest.json");
        let text = serde_json::to_string_pretty(&manifest)
            .map_err(|err| format!("Failed to serialize manifest JSON: {err}"))?;
        fs::write(&manifest_path, text)
            .map_err(|err| format!("Failed to write '{}': {err}", manifest_path.display()))?;
        Ok(())
    }

    // Download one complete ONNX model pack from Hugging Face and make it immediately selectable in local ONNX mode.
    fn download_remote_model_pack(preset: &AiRemoteModelPreset) -> Result<String, String> {
        let model_root = Self::model_root_dir();
        fs::create_dir_all(&model_root).map_err(|err| {
            format!(
                "Failed to create model root '{}': {err}",
                model_root.display()
            )
        })?;
        let model_dir = model_root.join(&preset.folder_id);
        fs::create_dir_all(&model_dir).map_err(|err| {
            format!(
                "Failed to create model dir '{}': {err}",
                model_dir.display()
            )
        })?;

        // Download common model metadata/tokenizer files expected by the local engine.
        Self::download_required_file(&preset.repo, "config.json", &model_dir.join("config.json"))?;
        Self::download_required_file(
            &preset.repo,
            "preprocessor_config.json",
            &model_dir.join("preprocessor_config.json"),
        )?;
        Self::download_required_file(
            &preset.repo,
            "tokenizer.json",
            &model_dir.join("tokenizer.json"),
        )?;

        // Prefer standard fp16/fp32 encoder variants and avoid merge-specific selection in the quick-download flow.
        let encoder_file = Self::download_first_existing_file(
            &preset.repo,
            &["onnx/encoder_model_fp16.onnx", "onnx/encoder_model.onnx"],
            &model_dir,
        )?;
        // Force standard (non-merged) decoder variants for now to validate stability before exposing merge variants again.
        let decoder_file = Self::download_first_existing_file(
            &preset.repo,
            &[
                "onnx/decoder_model_fp16.onnx",
                "onnx/decoder_model.onnx",
            ],
            &model_dir,
        )
        .map_err(|_| {
            format!(
                "No standard decoder found for '{}'. Expected one of: onnx/decoder_model_fp16.onnx or onnx/decoder_model.onnx",
                preset.repo
            )
        })?;

        // Download optional sidecar data files when present, required by some external-data ONNX exports.
        Self::download_optional_sidecars(&preset.repo, &encoder_file, &model_dir);
        Self::download_optional_sidecars(&preset.repo, &decoder_file, &model_dir);
        Self::write_downloaded_manifest(preset, &model_dir, &encoder_file, &decoder_file)?;

        Ok(format!(
            "Downloaded model pack '{}' to '{}'.",
            preset.label,
            model_dir.display()
        ))
    }

    // Build one model option from manifest; invalid/missing packs are skipped from UI.
    fn build_model_option(model_dir: &Path) -> Option<AiModelOption> {
        let manifest_path = model_dir.join("manifest.json");
        let manifest_text = fs::read_to_string(&manifest_path).ok()?;
        let manifest: AiModelManifest = serde_json::from_str(&manifest_text).ok()?;
        // Skip packs that target unsupported runtime kinds so users never select unusable models.
        if !is_supported_runtime_kind(manifest.runtime_kind.as_deref()) {
            return None;
        }
        if manifest.encoder.trim().is_empty()
            || manifest.decoder.trim().is_empty()
            || manifest.tokenizer.trim().is_empty()
        {
            return None;
        }
        let encoder_path = model_dir.join(&manifest.encoder);
        let mut decoder_path = model_dir.join(&manifest.decoder);
        // Prefer standard decoder files when both standard and merged are present in the same pack.
        let mut decoder_variant_for_label = manifest.decoder_variant.clone();
        if manifest.decoder.contains("_merged_fp16") {
            let standard_path = model_dir.join(manifest.decoder.replace("_merged_fp16", "_fp16"));
            if standard_path.exists() {
                decoder_path = standard_path;
                decoder_variant_for_label = None;
            }
        } else if manifest.decoder.contains("_merged") {
            let standard_path = model_dir.join(manifest.decoder.replace("_merged", ""));
            if standard_path.exists() {
                decoder_path = standard_path;
                decoder_variant_for_label = None;
            }
        }
        let tokenizer_path = model_dir.join(&manifest.tokenizer);
        let model_config_path = model_dir.join(manifest.model_config.trim());
        if !(encoder_path.exists()
            && decoder_path.exists()
            && tokenizer_path.exists()
            && model_config_path.exists())
        {
            return None;
        }
        // Validate optional preprocessor config reference when explicitly requested by manifest.
        let pre_ref = manifest.preprocessor_config.trim();
        if !pre_ref.is_empty() && !pre_ref.eq_ignore_ascii_case("none") {
            let pre_path = model_dir.join(pre_ref);
            if !pre_path.exists() {
                return None;
            }
        }
        let folder_name = model_dir.file_name()?.to_str()?.to_string();
        let id = manifest
            .id
            .filter(|x| !x.trim().is_empty())
            .unwrap_or(folder_name);
        let mut label = manifest
            .display_name
            .filter(|x| !x.trim().is_empty())
            .unwrap_or_else(|| id.clone());
        // Surface precision in the UI label so users can distinguish fp16/fp32 packs quickly.
        if let Some(precision) = manifest.precision.as_ref().map(|x| x.trim().to_uppercase())
            && !precision.is_empty()
            && !label.to_uppercase().contains(&precision)
        {
            label = format!("{label} {precision}");
        }
        if let Some(variant) = decoder_variant_for_label
            && !variant.trim().is_empty()
        {
            label = format!("{label} ({variant})");
        }
        Some(AiModelOption {
            id,
            label,
            model_dir: model_dir.to_path_buf(),
            encoder_path,
            decoder_path,
            tokenizer_path,
        })
    }

    // Discover installed model folders from disk; each valid folder means one selectable model.
    fn discover_model_options() -> Vec<AiModelOption> {
        let model_root = Self::model_root_dir();
        let mut out = Vec::new();

        if let Ok(entries) = fs::read_dir(&model_root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                if let Some(model) = Self::build_model_option(&path) {
                    out.push(model);
                }
            }
        }

        out.sort_by(|a, b| a.label.cmp(&b.label));
        out
    }

    // Build select menu items from discovered model options.
    fn build_model_items(options: &[AiModelOption]) -> SearchableVec<AiModelOption> {
        SearchableVec::new(options.to_vec())
    }

    // Build select menu items from tokenizer-derived language options.
    fn build_language_items(options: &[AiLanguageOption]) -> SearchableVec<AiLanguageOption> {
        SearchableVec::new(options.to_vec())
    }

    // List top-level providers so UI selects company first, then model.
    fn provider_options() -> Vec<AiProviderOption> {
        vec![
            AiProviderOption {
                id: "local_onnx".to_string(),
                label: "Local ONNX".to_string(),
            },
            AiProviderOption {
                id: "openai".to_string(),
                label: "OpenAI".to_string(),
            },
            AiProviderOption {
                id: "google".to_string(),
                label: "Google".to_string(),
            },
            AiProviderOption {
                id: "assemblyai".to_string(),
                label: "AssemblyAI".to_string(),
            },
        ]
    }

    // Build select menu items for provider/company selection.
    fn build_provider_items(options: &[AiProviderOption]) -> SearchableVec<AiProviderOption> {
        SearchableVec::new(options.to_vec())
    }

    // Resolve provider id from a concrete backend engine id.
    fn provider_for_engine(engine_id: &str) -> String {
        match engine_id {
            "local_onnx" => "local_onnx".to_string(),
            "openai_whisper_1"
            | "openai_whisper_1_plus_4o_merge"
            | "gpt4o_transcribe"
            | "gpt4o_transcribe_diarize"
            | "gpt4o_mini_transcribe"
            | "gpt4o_mini_tts" => "openai".to_string(),
            "gemini_25_pro" | "gemini_25_flash" => "google".to_string(),
            "assemblyai" => "assemblyai".to_string(),
            _ => "local_onnx".to_string(),
        }
    }

    // Predict local runtime backend hint so "Generating..." status can show expected device immediately.
    fn runtime_hint_for_engine(engine_id: &str) -> String {
        if engine_id != "local_onnx" {
            return "Cloud API".to_string();
        }
        #[cfg(target_os = "macos")]
        {
            return "CoreML/CPU".to_string();
        }
        #[cfg(target_os = "windows")]
        {
            return "CUDA/DirectML/CPU".to_string();
        }
        #[cfg(target_os = "linux")]
        {
            return "CUDA/CPU".to_string();
        }
        #[allow(unreachable_code)]
        "CPU".to_string()
    }

    // Parse machine-readable EP line emitted by local backend, e.g. "[EP_ACTIVE] CUDA".
    fn parse_runtime_backend_from_stdout(stdout: &str) -> Option<String> {
        for line in stdout.lines() {
            if let Some(rest) = line.strip_prefix("[EP_ACTIVE] ") {
                let parsed = rest.trim();
                if !parsed.is_empty() {
                    return Some(parsed.to_string());
                }
            }
        }
        None
    }

    // Pick default backend engine for each provider when switching company.
    fn default_engine_for_provider(provider_id: &str) -> String {
        match provider_id {
            "local_onnx" => "local_onnx".to_string(),
            "openai" => "openai_whisper_1".to_string(),
            "google" => "gemini_25_flash".to_string(),
            "assemblyai" => "assemblyai".to_string(),
            _ => "local_onnx".to_string(),
        }
    }

    // Keep cloud model selector limited to models under selected company.
    fn cloud_engine_options_for_provider(provider_id: &str) -> Vec<AiEngineOption> {
        Self::engine_options()
            .into_iter()
            .filter(|opt| {
                Self::provider_for_engine(&opt.id) == provider_id && opt.id != "local_onnx"
            })
            .collect()
    }

    // List available transcription engines so backend model routing stays explicit.
    fn engine_options() -> Vec<AiEngineOption> {
        vec![
            AiEngineOption {
                id: "local_onnx".to_string(),
                label: "Local ONNX".to_string(),
            },
            AiEngineOption {
                id: "openai_whisper_1".to_string(),
                label: "OpenAI Whisper-1  (~$0.006/min, native SRT capable)".to_string(),
            },
            AiEngineOption {
                id: "openai_whisper_1_plus_4o_merge".to_string(),
                label: "Whisper-1 timeline + 4o text (merge, no timing change)".to_string(),
            },
            AiEngineOption {
                id: "gpt4o_transcribe".to_string(),
                label: "GPT-4o Transcribe  (~$0.006/min)".to_string(),
            },
            AiEngineOption {
                id: "gpt4o_transcribe_diarize".to_string(),
                label: "GPT-4o Transcribe Diarize  (~$0.006/min)".to_string(),
            },
            AiEngineOption {
                id: "gpt4o_mini_transcribe".to_string(),
                label: "GPT-4o Mini Transcribe  (~$0.003/min)".to_string(),
            },
            AiEngineOption {
                id: "gpt4o_mini_tts".to_string(),
                label: "GPT-4o Mini TTS  (~$0.015/min, TTS only)".to_string(),
            },
            AiEngineOption {
                id: "gemini_25_pro".to_string(),
                label: "Gemini 2.5 Pro".to_string(),
            },
            AiEngineOption {
                id: "gemini_25_flash".to_string(),
                label: "Gemini 2.5 Flash".to_string(),
            },
            AiEngineOption {
                id: "assemblyai".to_string(),
                label: "AssemblyAI".to_string(),
            },
        ]
    }

    // Build select menu items for backend engine selection.
    fn build_engine_items(options: &[AiEngineOption]) -> SearchableVec<AiEngineOption> {
        SearchableVec::new(options.to_vec())
    }

    // Pick a stable default model id, preferring v3 turbo when available.
    fn default_selected_model_id(options: &[AiModelOption]) -> String {
        if let Some(m) = options.iter().find(|m| m.id == "whisper_large_v3_turbo") {
            return m.id.clone();
        }
        // Prefer turbo-family packs (e.g. fp16/fp32 variants with suffixed ids) when exact id is absent.
        if let Some(m) = options
            .iter()
            .find(|m| m.id.starts_with("whisper_large_v3_turbo"))
        {
            return m.id.clone();
        }
        if let Some(m) = options.first() {
            return m.id.clone();
        }
        String::new()
    }

    // Read current model config from selected id.
    fn selected_model_option(&self) -> Option<&AiModelOption> {
        self.model_options
            .iter()
            .find(|m| m.id == self.selected_model_id)
    }

    // Parse one tokenizer special token into a plain language code if it matches Whisper language style.
    fn parse_language_code_token(raw: &str) -> Option<String> {
        let code = raw.strip_prefix("<|")?.strip_suffix("|>")?;
        if code.len() < 2 || code.len() > 3 {
            return None;
        }
        if code.chars().all(|c| c.is_ascii_lowercase()) {
            Some(code.to_string())
        } else {
            None
        }
    }

    // Read supported language codes from tokenizer.json so UI options always match the selected model pack.
    fn discover_model_languages(model: &AiModelOption) -> Vec<AiLanguageOption> {
        let mut out = vec![AiLanguageOption {
            code: "auto".to_string(),
            label: "Auto Detect".to_string(),
        }];

        let tokenizer_text = match fs::read_to_string(&model.tokenizer_path) {
            Ok(text) => text,
            Err(_) => return out,
        };
        let tokenizer_json: serde_json::Value = match serde_json::from_str(&tokenizer_text) {
            Ok(value) => value,
            Err(_) => return out,
        };
        let Some(tokens) = tokenizer_json
            .get("added_tokens")
            .and_then(|v| v.as_array())
        else {
            return out;
        };

        let mut language_codes = BTreeSet::new();
        for token in tokens {
            let Some(content) = token.get("content").and_then(|v| v.as_str()) else {
                continue;
            };
            if let Some(code) = Self::parse_language_code_token(content) {
                language_codes.insert(code);
            }
        }

        for code in language_codes {
            let label = code.to_ascii_uppercase();
            out.push(AiLanguageOption { code, label });
        }
        out
    }

    // Derive a safe filename to preserve names when the output folder changes.
    fn file_name_or(path: &str, fallback: &str) -> String {
        PathBuf::from(path)
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| fallback.to_string())
    }

    fn run_btn(label: &'static str) -> gpui::Div {
        div()
            .h(px(32.0))
            .px_3()
            .rounded_lg()
            .border_1()
            .border_color(white().opacity(0.14))
            .bg(white().opacity(0.08))
            .text_color(white().opacity(0.9))
            .hover(|s| s.bg(white().opacity(0.12)))
            .cursor_pointer()
            .flex()
            .items_center()
            .justify_center()
            .child(label)
    }

    // Render a compact tuning slider row with label, slider, and live value.
    fn tuning_slider_row(label: &str, slider: &Entity<SliderState>, value: String) -> gpui::Div {
        div()
            .flex()
            .items_center()
            .justify_between()
            .h_8()
            .child(
                div()
                    .w(px(190.0))
                    .text_sm()
                    .text_color(white().opacity(0.8))
                    .child(label.to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .mx_2()
                    .child(Slider::new(slider).horizontal().h(px(20.0)).w_full()),
            )
            .child(
                div()
                    .w(px(90.0))
                    .flex()
                    .justify_end()
                    .text_sm()
                    .text_color(white().opacity(0.8))
                    .child(value),
            )
    }

    // Validate WAV by extension plus RIFF/WAVE header to avoid importing mislabeled files.
    fn is_valid_wav_file(path: &Path) -> bool {
        let ext_ok = path
            .extension()
            .and_then(|x| x.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("wav"))
            .unwrap_or(false);
        if !ext_ok {
            return false;
        }

        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            Err(_) => return false,
        };

        let mut header = [0u8; 12];
        if file.read_exact(&mut header).is_err() {
            return false;
        }

        let riff_or_rf64 = &header[0..4] == b"RIFF" || &header[0..4] == b"RF64";
        let wave = &header[8..12] == b"WAVE";
        riff_or_rf64 && wave
    }

    // Restrict media import to known audio/video file types.
    // Probe first (ffprobe), then fallback to extension whitelist when probe is unavailable.
    fn detect_media_kind(path: &Path, ffmpeg_path: &str) -> Option<&'static str> {
        let ffprobe_path = if ffmpeg_path.to_ascii_lowercase().ends_with("ffmpeg.exe") {
            ffmpeg_path[..ffmpeg_path.len() - "ffmpeg.exe".len()].to_string() + "ffprobe.exe"
        } else if ffmpeg_path.to_ascii_lowercase().ends_with("ffmpeg") {
            ffmpeg_path[..ffmpeg_path.len() - "ffmpeg".len()].to_string() + "ffprobe"
        } else {
            "ffprobe".to_string()
        };

        // Ask ffprobe to report stream codec types and classify by the first known media stream.
        if let Ok(output) = Command::new(&ffprobe_path)
            .arg("-v")
            .arg("error")
            .arg("-show_entries")
            .arg("stream=codec_type")
            .arg("-of")
            .arg("default=nw=1:nk=1")
            .arg(path)
            .output()
            && output.status.success()
        {
            let report = String::from_utf8_lossy(&output.stdout);
            if report
                .lines()
                .any(|line| line.trim().eq_ignore_ascii_case("video"))
            {
                return Some("video");
            }
            if report
                .lines()
                .any(|line| line.trim().eq_ignore_ascii_case("audio"))
            {
                return Some("audio");
            }
        }

        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        let audio_exts = [
            "wav", "mp3", "aac", "m4a", "flac", "ogg", "opus", "aif", "aiff", "amr", "wma",
        ];
        let video_exts = [
            "mp4", "mov", "mkv", "avi", "webm", "m4v", "ts", "mts", "m2ts", "wmv", "flv", "3gp",
            "mpg", "mpeg",
        ];
        if audio_exts.contains(&ext.as_str()) {
            Some("audio")
        } else if video_exts.contains(&ext.as_str()) {
            Some("video")
        } else {
            None
        }
    }

    fn run_ai_subtitle_engine(
        wav_path: &str,
        output_srt: &str,
        output_txt: &str,
        lang_code: &str,
        engine_id: &str,
        model: Option<&AiModelOption>,
        openai_api_key: &str,
        gemini_api_key: &str,
        assemblyai_api_key: &str,
        max_subtitle_duration_sec: f32,
        max_subtitle_chars: f32,
        timing_max_window_sec: f32,
        vad_db_offset: f32,
        vad_merge_gap_sec: f32,
    ) -> Result<String, String> {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // Validate local model files only when local ONNX backend is selected.
        if engine_id == "local_onnx" {
            let Some(model) = model else {
                return Err("No ONNX model selected for local backend.".to_string());
            };
            if !model.encoder_path.exists()
                || !model.decoder_path.exists()
                || !model.tokenizer_path.exists()
            {
                return Err(format!(
                    "Model files missing for '{}'. Expected:\n- {}\n- {}\n- {}",
                    model.label,
                    model.encoder_path.display(),
                    model.decoder_path.display(),
                    model.tokenizer_path.display()
                ));
            }
        }
        if let Some(parent) = PathBuf::from(output_srt).parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create output directory: {e}"))?;
        }
        if let Some(parent) = PathBuf::from(output_txt).parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create output directory: {e}"))?;
        }

        let make_engine_cmd = |bin: &mut Command| {
            bin.arg("--wav")
                .arg(wav_path)
                .arg("--engine")
                .arg(engine_id)
                .arg("--out")
                .arg(output_srt)
                .arg("--txt-out")
                .arg(output_txt)
                .arg("--lang")
                .arg(lang_code)
                // Pass subtitle timing/segmentation tuning from UI sliders to the engine.
                .arg("--max-subtitle-duration-sec")
                .arg(format!("{max_subtitle_duration_sec:.3}"))
                .arg("--max-subtitle-chars")
                .arg(max_subtitle_chars.round().max(1.0).to_string())
                .arg("--timing-max-window-sec")
                .arg(format!("{timing_max_window_sec:.3}"))
                .arg("--vad-db-offset")
                .arg(format!("{vad_db_offset:.3}"))
                .arg("--vad-merge-gap-sec")
                .arg(format!("{vad_merge_gap_sec:.3}"));
            if engine_id == "local_onnx"
                && let Some(model) = model
            {
                // Pass model_dir + files so local backend stays fully manifest-driven.
                bin.arg("--model-dir")
                    .arg(&model.model_dir)
                    .arg("--encoder")
                    .arg(&model.encoder_path)
                    .arg("--decoder")
                    .arg(&model.decoder_path)
                    .arg("--tokenizer")
                    .arg(&model.tokenizer_path);
            }
            // Forward per-provider API keys from UI into subprocess env without persisting defaults.
            if !openai_api_key.trim().is_empty() {
                bin.env("OPENAI_API_KEY", openai_api_key);
            }
            if !gemini_api_key.trim().is_empty() {
                bin.env("GEMINI_API_KEY", gemini_api_key);
            }
            if !assemblyai_api_key.trim().is_empty() {
                bin.env("ASSEMBLYAI_API_KEY", assemblyai_api_key);
            }
        };

        // Default to `cargo run` so subtitle generation always uses the latest source code.
        // Set ANICA_AI_SRT_USE_DIRECT_BIN=1 only when you explicitly want to skip rebuild checks.
        let bin_name = if cfg!(target_os = "windows") {
            "ai-subtitle-engine.exe"
        } else {
            "ai-subtitle-engine"
        };
        let direct_bin = manifest_dir.join("target").join("debug").join(bin_name);
        let use_direct_bin = std::env::var("ANICA_AI_SRT_USE_DIRECT_BIN")
            .map(|v| v == "1")
            .unwrap_or(false);

        let mut cmd = if use_direct_bin && direct_bin.exists() {
            let mut c = Command::new(direct_bin);
            make_engine_cmd(&mut c);
            c
        } else {
            let mut c = Command::new("cargo");
            c.current_dir(&manifest_dir)
                .arg("run")
                .arg("-p")
                .arg("ai-subtitle-engine")
                .arg("--");
            make_engine_cmd(&mut c);
            c
        };

        let output = cmd
            .output()
            .map_err(|e| format!("Failed to launch ai-subtitle-engine: {e}"))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            // Print both streams to terminal so deep backend failures are debuggable without UI truncation.
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if !stdout.trim().is_empty() {
                eprintln!("[AI-SRT][stdout]\n{stdout}");
            }
            if !stderr.trim().is_empty() {
                eprintln!("[AI-SRT][stderr]\n{stderr}");
            }
            let exit_code = output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "terminated by signal".to_string());
            Err(format!(
                "ai-subtitle-engine failed (exit: {exit_code}). Check terminal logs for stdout/stderr."
            ))
        }
    }

    // Run ffmpeg to convert non-WAV media into 16k mono PCM WAV for the model.
    fn transcode_media_to_wav(
        ffmpeg_path: &str,
        input_path: &str,
        output_wav_path: &Path,
    ) -> Result<(), String> {
        if let Some(parent) = output_wav_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create temp wav directory: {e}"))?;
        }

        let output = Command::new(ffmpeg_path)
            .arg("-y")
            .arg("-hide_banner")
            .arg("-i")
            .arg(input_path)
            .arg("-vn")
            .arg("-ac")
            .arg("1")
            .arg("-ar")
            .arg("16000")
            // Apply speech-focused denoise/compression chain before ASR inference.
            .arg("-af")
            .arg("highpass=f=80,lowpass=f=5000,afftdn=nf=-25,acompressor=threshold=-20dB:ratio=3:attack=5:release=80,dynaudnorm=f=200:g=12")
            .arg("-c:a")
            .arg("pcm_s16le")
            .arg(output_wav_path)
            .output()
            .map_err(|e| format!("Failed to launch ffmpeg: {e}"))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).to_string())
        }
    }

    // Build a deterministic temp WAV path near output files so cleanup stays simple.
    fn temp_wav_path(output_srt_path: &str) -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let base_dir = PathBuf::from(output_srt_path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| default_project_dir().join("ai_srt"));
        base_dir.join(format!("ai_input_{ts}.wav"))
    }

    // Accept WAV directly; for media files, convert first then run the model.
    fn run_ai_pipeline(
        input_path: &str,
        output_srt: &str,
        output_txt: &str,
        lang_code: &str,
        engine_id: &str,
        ffmpeg_path: &str,
        model: Option<&AiModelOption>,
        openai_api_key: &str,
        gemini_api_key: &str,
        assemblyai_api_key: &str,
        max_subtitle_duration_sec: f32,
        max_subtitle_chars: f32,
        timing_max_window_sec: f32,
        vad_db_offset: f32,
        vad_merge_gap_sec: f32,
    ) -> Result<String, String> {
        let is_wav = Path::new(input_path)
            .extension()
            .and_then(|x| x.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("wav"))
            .unwrap_or(false);

        let (wav_path, temp_wav) = if is_wav {
            (input_path.to_string(), None)
        } else {
            let tmp = Self::temp_wav_path(output_srt);
            Self::transcode_media_to_wav(ffmpeg_path, input_path, &tmp)?;
            (tmp.to_string_lossy().to_string(), Some(tmp))
        };

        let result = Self::run_ai_subtitle_engine(
            &wav_path,
            output_srt,
            output_txt,
            lang_code,
            engine_id,
            model,
            openai_api_key,
            gemini_api_key,
            assemblyai_api_key,
            max_subtitle_duration_sec,
            max_subtitle_chars,
            timing_max_window_sec,
            vad_db_offset,
            vad_merge_gap_sec,
        );

        // On macOS, clear Gatekeeper quarantine so generated text files open normally.
        if result.is_ok() {
            let _ = Self::clear_quarantine_for_generated_outputs([output_srt, output_txt]);
        }

        if let Some(tmp) = temp_wav {
            let _ = fs::remove_file(tmp);
        }

        result
    }

    // Remove macOS quarantine attribute from generated files to avoid "Not Opened" warnings.
    fn clear_quarantine_for_generated_outputs<'a, I>(paths: I) -> Result<(), String>
    where
        I: IntoIterator<Item = &'a str>,
    {
        #[cfg(target_os = "macos")]
        {
            let mut errs = Vec::new();
            for path in paths {
                let output = Command::new("xattr")
                    .arg("-d")
                    .arg("com.apple.quarantine")
                    .arg(path)
                    .output();

                match output {
                    Ok(out) => {
                        if !out.status.success() {
                            let stderr = String::from_utf8_lossy(&out.stderr);
                            let not_found = stderr.contains("No such xattr")
                                || stderr.contains("No such file")
                                || stderr.contains("No such attribute");
                            // Missing attribute is acceptable because the file is already trusted.
                            if !not_found {
                                errs.push(format!("{path}: {}", stderr.trim()));
                            }
                        }
                    }
                    Err(err) => errs.push(format!("{path}: {err}")),
                }
            }

            if errs.is_empty() {
                Ok(())
            } else {
                Err(format!("Failed to clear quarantine: {}", errs.join(" | ")))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = paths;
            Ok(())
        }
    }
}

impl Render for AiSrtPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let input_text = self.input_path.as_deref().unwrap_or("No input selected");
        let selected_model_label = self
            .selected_model_option()
            .map(|m| m.label.clone())
            .unwrap_or_else(|| "No model".to_string());
        let selected_provider_label = self
            .provider_options
            .iter()
            .find(|p| p.id == self.selected_provider_id)
            .map(|p| p.label.clone())
            .unwrap_or_else(|| "Unknown provider".to_string());
        let selected_engine_label = self
            .engine_options
            .iter()
            .find(|e| e.id == self.selected_engine_id)
            .map(|e| e.label.clone())
            .unwrap_or_else(|| "Unknown engine".to_string());
        // Keep local-only controls hidden when a cloud engine is selected.
        let is_local_engine = self.selected_provider_id == "local_onnx";
        // Treat all OpenAI cloud engine variants as one key scope.
        let is_openai_engine = self.selected_provider_id == "openai";
        // Show only the API key field required by the selected cloud provider.
        let show_openai_key = is_openai_engine;
        let show_gemini_key = self.selected_provider_id == "google";
        let show_assemblyai_key = self.selected_provider_id == "assemblyai";
        let run_disabled = self.is_running || self.input_path.is_none();
        let global_for_pick = self.global.clone();
        let global_for_media_pick = self.global.clone();
        let global_for_download = self.global.clone();

        // Initialize provider dropdown so users choose company first.
        if self.provider_select.is_none() {
            let items = Self::build_provider_items(&self.provider_options);
            let state = cx.new(|cx| SelectState::new(items, None, window, cx).searchable(false));
            let selected_id = self.selected_provider_id.clone();
            state.update(cx, |this, cx| {
                this.set_selected_value(&selected_id, window, cx);
            });
            let sub = cx.subscribe(
                &state,
                |this, _, ev: &SelectEvent<SearchableVec<AiProviderOption>>, cx| {
                    let SelectEvent::Confirm(value) = ev;
                    let Some(value) = value else {
                        return;
                    };
                    this.selected_provider_id = value.clone();
                    if value == "local_onnx" {
                        this.selected_engine_id = "local_onnx".to_string();
                    } else if Self::provider_for_engine(&this.selected_engine_id) != *value {
                        this.selected_engine_id = Self::default_engine_for_provider(value);
                    }
                    this.status_text = format!("Provider set to {}.", value.to_ascii_uppercase());
                    cx.notify();
                },
            );
            self.provider_select = Some(state);
            self.provider_select_sub = Some(sub);
        }

        // Rebuild cloud model selector whenever provider changes, and hide it for local ONNX.
        if !is_local_engine
            && (self.engine_select.is_none()
                || self.cloud_engine_provider_id != self.selected_provider_id)
        {
            let cloud_options = Self::cloud_engine_options_for_provider(&self.selected_provider_id);
            let items = Self::build_engine_items(&cloud_options);
            let state = cx.new(|cx| SelectState::new(items, None, window, cx).searchable(false));

            let selected_is_valid = cloud_options
                .iter()
                .any(|x| x.id == self.selected_engine_id);
            if !selected_is_valid {
                self.selected_engine_id =
                    Self::default_engine_for_provider(&self.selected_provider_id);
            }
            let selected_id = self.selected_engine_id.clone();
            state.update(cx, |this, cx| {
                this.set_selected_value(&selected_id, window, cx);
            });

            let sub = cx.subscribe(
                &state,
                |this, _, ev: &SelectEvent<SearchableVec<AiEngineOption>>, cx| {
                    let SelectEvent::Confirm(value) = ev;
                    let Some(value) = value else {
                        return;
                    };
                    this.selected_engine_id = value.clone();
                    this.status_text = format!("Cloud model set to {value}.");
                    cx.notify();
                },
            );
            self.engine_select = Some(state);
            self.engine_select_sub = Some(sub);
            self.cloud_engine_provider_id = self.selected_provider_id.clone();
        }

        // Initialize API key inputs once; values stay in-memory and are passed to subprocess env on run.
        if self.openai_key_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("OPENAI_API_KEY")
                    .masked(true)
            });
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                this.openai_api_key = input.read(cx).value().to_string();
                cx.notify();
            });
            self.openai_key_input = Some(input);
            self.openai_key_input_sub = Some(sub);
        }
        if self.gemini_key_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("GEMINI_API_KEY")
                    .masked(true)
            });
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                this.gemini_api_key = input.read(cx).value().to_string();
                cx.notify();
            });
            self.gemini_key_input = Some(input);
            self.gemini_key_input_sub = Some(sub);
        }
        if self.assemblyai_key_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("ASSEMBLYAI_API_KEY")
                    .masked(true)
            });
            let sub = cx.subscribe(&input, |this, input, ev, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                this.assemblyai_api_key = input.read(cx).value().to_string();
                cx.notify();
            });
            self.assemblyai_key_input = Some(input);
            self.assemblyai_key_input_sub = Some(sub);
        }

        // Initialize the model dropdown once so users can choose from discovered model packs.
        if self.model_select.is_none() {
            let items = Self::build_model_items(&self.model_options);
            let state = cx.new(|cx| SelectState::new(items, None, window, cx).searchable(false));
            if !self.selected_model_id.is_empty() {
                let selected_id = self.selected_model_id.clone();
                state.update(cx, |this, cx| {
                    this.set_selected_value(&selected_id, window, cx);
                });
            }
            let sub = cx.subscribe(
                &state,
                |this, _, ev: &SelectEvent<SearchableVec<AiModelOption>>, cx| {
                    let SelectEvent::Confirm(value) = ev;
                    let Some(value) = value else {
                        return;
                    };
                    this.selected_model_id = value.clone();
                    if let Some(selected) = this.selected_model_option() {
                        this.status_text = format!("Model set to {}.", selected.label);
                    }
                    cx.notify();
                },
            );
            self.model_select = Some(state);
            self.model_select_sub = Some(sub);
        }

        // Rebuild language options whenever the selected model changes so choices match tokenizer support.
        if self.language_select.is_none() || self.language_model_id != self.selected_model_id {
            let options = self
                .selected_model_option()
                .map(Self::discover_model_languages)
                .unwrap_or_else(|| {
                    vec![AiLanguageOption {
                        code: "auto".to_string(),
                        label: "Auto Detect".to_string(),
                    }]
                });
            self.language_options = options;
            self.language_model_id = self.selected_model_id.clone();

            let items = Self::build_language_items(&self.language_options);
            let state = cx.new(|cx| SelectState::new(items, None, window, cx).searchable(false));
            let has_current_lang = self
                .language_options
                .iter()
                .any(|opt| opt.code == self.lang_code);
            if !has_current_lang {
                self.lang_code = "auto".to_string();
            }
            let selected_code = self.lang_code.clone();
            state.update(cx, |this, cx| {
                this.set_selected_value(&selected_code, window, cx);
            });
            let sub = cx.subscribe(
                &state,
                |this, _, ev: &SelectEvent<SearchableVec<AiLanguageOption>>, cx| {
                    let SelectEvent::Confirm(value) = ev;
                    let Some(value) = value else {
                        return;
                    };
                    this.lang_code = value.clone();
                    this.status_text = if value == "auto" {
                        "Language set to auto detect.".to_string()
                    } else {
                        format!("Language set to {}.", value.to_ascii_uppercase())
                    };
                    cx.notify();
                },
            );
            self.language_select = Some(state);
            self.language_select_sub = Some(sub);
        }

        // Lazily initialize tuning sliders once, then keep values synced through subscriptions.
        if self.max_subtitle_duration_slider.is_none() {
            let slider = cx.new(|_| {
                SliderState::new()
                    .min(2.0)
                    .max(15.0)
                    .default_value(Self::DEFAULT_MAX_SUBTITLE_DURATION_SEC)
                    .step(0.1)
            });
            let sub = cx.subscribe(&slider, |this, _, ev, cx| {
                let SliderEvent::Change(v) = ev;
                this.max_subtitle_duration_sec = v.start();
                cx.notify();
            });
            self.max_subtitle_duration_slider = Some(slider);
            self.max_subtitle_duration_sub = Some(sub);
        }
        if self.max_subtitle_chars_slider.is_none() {
            let slider = cx.new(|_| {
                SliderState::new()
                    .min(20.0)
                    .max(90.0)
                    .default_value(Self::DEFAULT_MAX_SUBTITLE_CHARS)
                    .step(1.0)
            });
            let sub = cx.subscribe(&slider, |this, _, ev, cx| {
                let SliderEvent::Change(v) = ev;
                this.max_subtitle_chars = v.start().round();
                cx.notify();
            });
            self.max_subtitle_chars_slider = Some(slider);
            self.max_subtitle_chars_sub = Some(sub);
        }
        if self.timing_max_window_slider.is_none() {
            let slider = cx.new(|_| {
                SliderState::new()
                    .min(4.0)
                    .max(30.0)
                    .default_value(Self::DEFAULT_TIMING_MAX_WINDOW_SEC)
                    .step(0.5)
            });
            let sub = cx.subscribe(&slider, |this, _, ev, cx| {
                let SliderEvent::Change(v) = ev;
                this.timing_max_window_sec = v.start();
                cx.notify();
            });
            self.timing_max_window_slider = Some(slider);
            self.timing_max_window_sub = Some(sub);
        }
        if self.vad_db_offset_slider.is_none() {
            let slider = cx.new(|_| {
                SliderState::new()
                    .min(2.0)
                    .max(20.0)
                    .default_value(Self::DEFAULT_VAD_DB_OFFSET)
                    .step(0.5)
            });
            let sub = cx.subscribe(&slider, |this, _, ev, cx| {
                let SliderEvent::Change(v) = ev;
                this.vad_db_offset = v.start();
                cx.notify();
            });
            self.vad_db_offset_slider = Some(slider);
            self.vad_db_offset_sub = Some(sub);
        }
        if self.vad_merge_gap_slider.is_none() {
            let slider = cx.new(|_| {
                SliderState::new()
                    .min(0.05)
                    .max(1.5)
                    .default_value(Self::DEFAULT_VAD_MERGE_GAP_SEC)
                    .step(0.01)
            });
            let sub = cx.subscribe(&slider, |this, _, ev, cx| {
                let SliderEvent::Change(v) = ev;
                this.vad_merge_gap_sec = v.start();
                cx.notify();
            });
            self.vad_merge_gap_slider = Some(slider);
            self.vad_merge_gap_sub = Some(sub);
        }

        // Build model-download controls dynamically so users can install ONNX packs directly from Hugging Face.
        let mut model_download_manager_section = div()
            .mt_1()
            .pt_2()
            .border_t_1()
            .border_color(rgba(0x60a5fa40))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(white().opacity(0.9))
                    .child("Model Download Manager"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.6))
                    .child("Download Whisper ONNX packs from onnx-community Hugging Face repos."),
            );
        for preset in &self.remote_model_presets {
            let is_installed =
                Self::build_model_option(&Self::model_root_dir().join(&preset.folder_id)).is_some();
            let preset_for_download = preset.clone();
            let global_for_download_btn = global_for_download.clone();

            let mut download_btn = Self::run_btn(if is_installed {
                "Reinstall"
            } else {
                "Download"
            });
            if self.is_downloading_model {
                download_btn = download_btn
                    .bg(white().opacity(0.03))
                    .text_color(white().opacity(0.45))
                    .cursor_default();
            } else {
                download_btn = download_btn.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if this.is_downloading_model {
                            return;
                        }

                        // Lock download actions so only one large model download runs at a time.
                        this.is_downloading_model = true;
                        this.download_status_text = format!(
                            "Downloading '{}' ... this can take a while.",
                            preset_for_download.label
                        );
                        cx.notify();

                        let preset_for_job = preset_for_download.clone();
                        let preset_folder_id = preset_for_download.folder_id.clone();
                        let global_for_job = global_for_download_btn.clone();
                        cx.spawn(async move |view, cx| {
                            let result = cx
                                .background_spawn(async move {
                                    Self::download_remote_model_pack(&preset_for_job)
                                })
                                .await;
                            let _ = view.update(cx, |this, cx| {
                                this.is_downloading_model = false;
                                match result {
                                    Ok(msg) => {
                                        // Refresh model/language selectors so newly installed pack is immediately available.
                                        this.model_options = Self::discover_model_options();
                                        if let Some(installed) = this
                                            .model_options
                                            .iter()
                                            .find(|x| x.id == preset_folder_id)
                                        {
                                            this.selected_model_id = installed.id.clone();
                                        } else if !this
                                            .model_options
                                            .iter()
                                            .any(|x| x.id == this.selected_model_id)
                                        {
                                            this.selected_model_id =
                                                Self::default_selected_model_id(
                                                    &this.model_options,
                                                );
                                        }
                                        this.model_select = None;
                                        this.model_select_sub = None;
                                        this.language_select = None;
                                        this.language_select_sub = None;
                                        this.download_status_text = msg.clone();
                                        this.status_text = "Model pack downloaded.".to_string();
                                        global_for_job.update(cx, |gs, cx| {
                                            gs.ui_notice = Some(msg);
                                            cx.notify();
                                        });
                                    }
                                    Err(err) => {
                                        this.download_status_text =
                                            format!("Download failed: {err}");
                                        this.status_text = "Model download failed.".to_string();
                                        global_for_job.update(cx, |gs, cx| {
                                            gs.ui_notice =
                                                Some(format!("Model download failed: {err}"));
                                            cx.notify();
                                        });
                                    }
                                }
                                cx.notify();
                            });
                        })
                        .detach();
                    }),
                );
            }

            model_download_manager_section = model_download_manager_section.child(
                div()
                    .rounded_sm()
                    .border_1()
                    .border_color(rgba(0x67e8f94a))
                    .bg(rgba(0x0f172a99))
                    .p_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w(px(0.0))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(white().opacity(0.88))
                                    .child(preset.label.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(if is_installed {
                                        white().opacity(0.72)
                                    } else {
                                        white().opacity(0.5)
                                    })
                                    .child(if is_installed {
                                        "Installed"
                                    } else {
                                        "Not installed"
                                    }),
                            ),
                    )
                    .child(download_btn),
            );
        }
        model_download_manager_section = model_download_manager_section.child(
            div()
                .text_xs()
                .text_color(white().opacity(0.72))
                .child(format!("Download status: {}", self.download_status_text)),
        );

        div()
            .size_full()
            .min_h_0()
            .bg(rgba(0x0b1220ff))
            .child(
                // Keep the whole AI SRT page vertically scrollable so lower controls are always reachable.
                div()
                    .size_full()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p_6()
                    .pb_8()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .text_xl()
                            .text_color(white().opacity(0.85))
                            .child("AI SRT Workspace"),
                    )
            .child(
                div()
                    .text_sm()
                    .text_color(white().opacity(0.55))
                    .child("Generate subtitles with local ONNX or cloud providers and import to S track."),
            )
            .child(
                div()
                    .rounded_md()
                    .border_1()
                    .border_color(rgba(0x38bdf866))
                    .bg(rgba(0x0f172acc))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.9))
                            .child(format!("Input Source: {input_text}"))
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.9))
                            .child(format!("Output SRT: {}", self.output_srt_path))
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.9))
                            .child(format!("Output TXT: {}", self.output_txt_path))
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.9))
                            .child(format!("Provider: {}", selected_provider_label))
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(white().opacity(0.8))
                                    .child("Provider Selector"),
                            )
                            .child(
                                if let Some(select) = self.provider_select.as_ref() {
                                    Select::new(select)
                                        .placeholder("Select provider")
                                        .menu_width(px(320.0))
                                        .into_any_element()
                                } else {
                                    div()
                                        .h(px(28.0))
                                        .rounded_sm()
                                        .bg(white().opacity(0.06))
                                        .text_color(white().opacity(0.6))
                                        .px_2()
                                        .child("No provider options")
                                        .into_any_element()
                                },
                            )
                    )
                    .when(!is_local_engine, |d| {
                        d.child(
                            // Render cloud model selection only after provider is chosen.
                            div()
                                .text_sm()
                                .text_color(white().opacity(0.9))
                                .child(format!("Cloud Model: {}", selected_engine_label)),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(white().opacity(0.8))
                                        .child("Cloud Model Selector"),
                                )
                                .child(
                                    if let Some(select) = self.engine_select.as_ref() {
                                        Select::new(select)
                                            .placeholder("Select cloud model")
                                            .menu_width(px(420.0))
                                            .into_any_element()
                                    } else {
                                        div()
                                            .h(px(28.0))
                                            .rounded_sm()
                                            .bg(white().opacity(0.06))
                                            .text_color(white().opacity(0.6))
                                            .px_2()
                                            .child("No cloud model options")
                                            .into_any_element()
                                    },
                                ),
                        )
                    })
                    .when(show_openai_key, |d| {
                        d.child(
                            // Render OpenAI key input only for OpenAI provider.
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(white().opacity(0.8))
                                        .w(px(140.0))
                                        .child("OpenAI API Key"),
                                )
                                .child(
                                    if let Some(input) = self.openai_key_input.as_ref() {
                                        Input::new(input)
                                            .h(px(32.0))
                                            .w(px(360.0))
                                            .mask_toggle()
                                            .into_any_element()
                                    } else {
                                        div().h(px(32.0)).w(px(360.0)).into_any_element()
                                    },
                                ),
                        )
                    })
                    .when(show_gemini_key, |d| {
                        d.child(
                            // Render Gemini key input only when Gemini backend is selected.
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(white().opacity(0.8))
                                        .w(px(140.0))
                                        .child("Gemini API Key"),
                                )
                                .child(
                                    if let Some(input) = self.gemini_key_input.as_ref() {
                                        Input::new(input)
                                            .h(px(32.0))
                                            .w(px(360.0))
                                            .mask_toggle()
                                            .into_any_element()
                                    } else {
                                        div().h(px(32.0)).w(px(360.0)).into_any_element()
                                    },
                                ),
                        )
                    })
                    .when(show_assemblyai_key, |d| {
                        d.child(
                            // Render AssemblyAI key input only when AssemblyAI backend is selected.
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(white().opacity(0.8))
                                        .w(px(140.0))
                                        .child("AssemblyAI API Key"),
                                )
                                .child(
                                    if let Some(input) = self.assemblyai_key_input.as_ref() {
                                        Input::new(input)
                                            .h(px(32.0))
                                            .w(px(360.0))
                                            .mask_toggle()
                                            .into_any_element()
                                    } else {
                                        div().h(px(32.0)).w(px(360.0)).into_any_element()
                                    },
                                ),
                        )
                    })
                    .child(
                        // Keep model selection fully hidden in cloud mode.
                        if is_local_engine {
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(white().opacity(0.9))
                                        .child(format!("Model: {}", selected_model_label))
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(white().opacity(0.8))
                                                .child("Model Selector"),
                                        )
                                        .child(
                                            if let Some(select) = self.model_select.as_ref() {
                                                Select::new(select)
                                                    .placeholder("Select ONNX model")
                                                    .menu_width(px(320.0))
                                                    .into_any_element()
                                            } else {
                                                div()
                                                    .h(px(28.0))
                                                    .rounded_sm()
                                                    .bg(white().opacity(0.06))
                                                    .text_color(white().opacity(0.6))
                                                    .px_2()
                                                    .child("No valid model packs found")
                                                    .into_any_element()
                                            },
                                        )
                                )
                                .into_any_element()
                        } else {
                            div().into_any_element()
                        },
                    )
                    .child(
                        // Keep download manager local-only because cloud providers do not use local ONNX packs.
                        if is_local_engine {
                            model_download_manager_section.into_any_element()
                        } else {
                            div().into_any_element()
                        },
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(white().opacity(0.9))
                            .child(format!("Language: {}", self.lang_code))
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(white().opacity(0.8))
                                    .child("Language Selector"),
                            )
                            .child(
                                if let Some(select) = self.language_select.as_ref() {
                                    Select::new(select)
                                        .placeholder("Select language")
                                        .menu_width(px(220.0))
                                        .into_any_element()
                                } else {
                                    div()
                                        .h(px(28.0))
                                        .rounded_sm()
                                        .bg(white().opacity(0.06))
                                        .text_color(white().opacity(0.6))
                                        .px_2()
                                        .child("No language options")
                                        .into_any_element()
                                },
                            )
                    )
                    .child(
                        // Keep subtitle tuning controls local-only; cloud services provide their own timing.
                        if is_local_engine {
                            div()
                                .mt_1()
                                .pt_2()
                                .px_2()
                                .pb_2()
                                .rounded_sm()
                                .border_1()
                                .border_color(rgba(0x34d39955))
                                .bg(rgba(0x052e2bcc))
                                .border_t_1()
                                .border_color(rgba(0x34d39955))
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(white().opacity(0.9))
                                        .child("Subtitle Tuning"),
                                )
                                .when_some(self.max_subtitle_duration_slider.as_ref(), |d, slider| {
                                    d.child(Self::tuning_slider_row(
                                        "1) Max Subtitle Duration (sec)",
                                        slider,
                                        format!("{:.1}", self.max_subtitle_duration_sec),
                                    ))
                                })
                                .when_some(self.max_subtitle_chars_slider.as_ref(), |d, slider| {
                                    d.child(Self::tuning_slider_row(
                                        "2) Max Subtitle Chars",
                                        slider,
                                        format!("{:.0}", self.max_subtitle_chars),
                                    ))
                                })
                                .when_some(self.timing_max_window_slider.as_ref(), |d, slider| {
                                    d.child(Self::tuning_slider_row(
                                        "3) Timing Max Window (sec)",
                                        slider,
                                        format!("{:.1}", self.timing_max_window_sec),
                                    ))
                                })
                                .when_some(self.vad_db_offset_slider.as_ref(), |d, slider| {
                                    d.child(Self::tuning_slider_row(
                                        "4) VAD dB Offset",
                                        slider,
                                        format!("{:.1}", self.vad_db_offset),
                                    ))
                                })
                                .when_some(self.vad_merge_gap_slider.as_ref(), |d, slider| {
                                    d.child(Self::tuning_slider_row(
                                        "5) VAD Merge Gap (sec)",
                                        slider,
                                        format!("{:.2}", self.vad_merge_gap_sec),
                                    ))
                                })
                                .into_any_element()
                        } else {
                            div().into_any_element()
                        },
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Self::run_btn("Pick WAV")
                                    .on_mouse_down(MouseButton::Left, cx.listener(move |_this, _, win, cx| {
                                        let global_for_pick = global_for_pick.clone();
                                        let rx = cx.prompt_for_paths(PathPromptOptions {
                                            files: true,
                                            directories: false,
                                            multiple: false,
                                            prompt: Some("Select WAV (16kHz mono)".into()),
                                        });
                                        cx.spawn_in(win, async move |view, window| {
                                            let Ok(result) = rx.await else { return; };
                                            let Some(path) = result.ok().flatten().and_then(|v| v.into_iter().next()) else { return; };
                                            let _ = view.update_in(window, |this, _window, cx| {
                                                // Reject non-WAV or malformed WAV files at file-pick time.
                                                if !Self::is_valid_wav_file(&path) {
                                                    this.status_text = "Invalid WAV file. Please select a real .wav (RIFF/WAVE).".to_string();
                                                    global_for_pick.update(cx, |gs, cx| {
                                                        gs.ui_notice = Some("Invalid WAV file selection.".to_string());
                                                        cx.notify();
                                                    });
                                                    cx.notify();
                                                    return;
                                                }
                                                this.input_path = Some(path.to_string_lossy().to_string());
                                                this.status_text = "WAV selected.".to_string();
                                                if this.output_srt_path.is_empty() || this.output_txt_path.is_empty() {
                                                    let (default_srt, default_txt) = Self::default_output_paths();
                                                    this.output_srt_path = default_srt;
                                                    this.output_txt_path = default_txt;
                                                }
                                                global_for_pick.update(cx, |gs, cx| {
                                                    gs.ui_notice = Some(format!("Selected WAV: {}", path.display()));
                                                    cx.notify();
                                                });
                                                cx.notify();
                                            });
                                        }).detach();
                                    }))
                            )
                            .child(
                                // Pick media (mp4/mp3/...) and convert to WAV before model inference.
                                Self::run_btn("Pick Media (trans to WAV)")
                                    .on_mouse_down(MouseButton::Left, cx.listener(move |_this, _, win, cx| {
                                        let global_for_media_pick = global_for_media_pick.clone();
                                        let ffmpeg_path = _this.global.read(cx).ffmpeg_path.clone();
                                        let rx = cx.prompt_for_paths(PathPromptOptions {
                                            files: true,
                                            directories: false,
                                            multiple: false,
                                            prompt: Some("Select audio/video media".into()),
                                        });
                                        cx.spawn_in(win, async move |view, window| {
                                            let Ok(result) = rx.await else { return; };
                                            let Some(path) = result.ok().flatten().and_then(|v| v.into_iter().next()) else { return; };
                                            let _ = view.update_in(window, |this, _window, cx| {
                                                // Enforce audio/video-only selection before running conversion.
                                                let Some(kind) = Self::detect_media_kind(&path, &ffmpeg_path) else {
                                                    this.status_text = "Invalid media file. Please select an audio/video file.".to_string();
                                                    global_for_media_pick.update(cx, |gs, cx| {
                                                        gs.ui_notice = Some("Only audio/video media files are supported.".to_string());
                                                        cx.notify();
                                                    });
                                                    cx.notify();
                                                    return;
                                                };
                                                this.input_path = Some(path.to_string_lossy().to_string());
                                                this.status_text = format!("{kind} selected. It will be converted to WAV automatically.");
                                                cx.notify();
                                            });
                                        }).detach();
                                    }))
                            )
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Self::run_btn(if self.import_after_generate {
                                    "Import to Timeline: ON"
                                } else {
                                    "Import to Timeline: OFF"
                                })
                                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                    this.import_after_generate = !this.import_after_generate;
                                    cx.notify();
                                }))
                            )
                            .child(
                                Self::run_btn("Pick Output Folder")
                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                        let current_srt_name = Self::file_name_or(&this.output_srt_path, "ai_transcript.srt");
                                        let current_txt_name = Self::file_name_or(&this.output_txt_path, "ai_transcript.txt");
                                        let rx = cx.prompt_for_paths(PathPromptOptions {
                                            files: false,
                                            directories: true,
                                            multiple: false,
                                            prompt: Some("Select output folder for SRT/TXT".into()),
                                        });
                                        cx.spawn(async move |view, cx| {
                                            let Ok(result) = rx.await else { return; };
                                            let Some(folder) = result.ok().flatten().and_then(|v| v.into_iter().next()) else { return; };
                                            let _ = view.update(cx, |this, cx| {
                                                this.output_srt_path = folder.join(&current_srt_name).to_string_lossy().to_string();
                                                this.output_txt_path = folder.join(&current_txt_name).to_string_lossy().to_string();
                                                this.status_text = "Output folder updated.".to_string();
                                                cx.notify();
                                            });
                                        }).detach();
                                    }))
                            )
                            .child(
                                Self::run_btn("Reset Output")
                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                        let (default_srt, default_txt) = Self::default_output_paths();
                                        this.output_srt_path = default_srt;
                                        this.output_txt_path = default_txt;
                                        this.status_text = "Output paths reset to default.".to_string();
                                        cx.notify();
                                    }))
                            )
                            .child({
                                let mut btn = Self::run_btn(if self.is_running {
                                    "Generating..."
                                } else {
                                    "Generate SRT"
                                });
                                if run_disabled {
                                    btn = btn
                                        .bg(white().opacity(0.03))
                                        .text_color(white().opacity(0.45))
                                        .cursor_default();
                                } else {
                                    btn = btn.on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _win, cx| {
                                        let input_path = this.input_path.clone().unwrap_or_default();
                                        let lang = this.lang_code.clone();
                                        let engine_id = this.selected_engine_id.clone();
                                        let openai_api_key = this.openai_api_key.clone();
                                        let gemini_api_key = this.gemini_api_key.clone();
                                        let assemblyai_api_key = this.assemblyai_api_key.clone();
                                        let import_after = this.import_after_generate;
                                        let global = this.global.clone();
                                        let ffmpeg_path = this.global.read(cx).ffmpeg_path.clone();
                                        let model = this.selected_model_option().cloned();
                                        if engine_id == "local_onnx" && model.is_none() {
                                            this.status_text = "No model selected.".to_string();
                                            cx.notify();
                                            return;
                                        }
                                        // Validate required key for selected cloud engine before starting async work.
                                        if matches!(
                                            engine_id.as_str(),
                                            "openai_whisper_1"
                                                | "openai_whisper_1_plus_4o_merge"
                                                | "gpt4o_transcribe"
                                                | "gpt4o_transcribe_diarize"
                                                | "gpt4o_mini_transcribe"
                                                | "gpt4o_mini_tts"
                                        ) && openai_api_key.trim().is_empty() {
                                            this.status_text = "OpenAI API key is required for the selected OpenAI engine.".to_string();
                                            cx.notify();
                                            return;
                                        }
                                        // Block TTS-only model before job start so users get immediate feedback in UI.
                                        if engine_id == "gpt4o_mini_tts" {
                                            this.status_text = "gpt-4o-mini-tts is TTS-only and cannot generate SRT from audio.".to_string();
                                            cx.notify();
                                            return;
                                        }
                                        if (engine_id == "gemini_25_pro" || engine_id == "gemini_25_flash")
                                            && gemini_api_key.trim().is_empty()
                                        {
                                            this.status_text = "Gemini API key is required for Gemini engines.".to_string();
                                            cx.notify();
                                            return;
                                        }
                                        if engine_id == "assemblyai" && assemblyai_api_key.trim().is_empty() {
                                            this.status_text = "AssemblyAI API key is required for AssemblyAI.".to_string();
                                            cx.notify();
                                            return;
                                        }
                                        let max_subtitle_duration_sec = this.max_subtitle_duration_sec;
                                        let max_subtitle_chars = this.max_subtitle_chars;
                                        let timing_max_window_sec = this.timing_max_window_sec;
                                        let vad_db_offset = this.vad_db_offset;
                                        let vad_merge_gap_sec = this.vad_merge_gap_sec;
                                        // Generate fresh timestamped output names on each successful run start to reduce overwrite risk.
                                        this.refresh_output_paths_for_new_generation();
                                        let out_srt_path = this.output_srt_path.clone();
                                        let out_txt_path = this.output_txt_path.clone();
                                        let runtime_hint = Self::runtime_hint_for_engine(&engine_id);

                                        this.is_running = true;
                                        this.status_text = format!("Generating subtitles... ({runtime_hint})");
                                        cx.notify();

                                        cx.spawn(async move |view, cx| {
                                            // Use dedicated clones for background work so UI-update paths keep ownership.
                                            let run_input_path = input_path.clone();
                                            let run_out_srt_path = out_srt_path.clone();
                                            let run_out_txt_path = out_txt_path.clone();
                                            let run_lang = lang.clone();
                                            let run_engine_id = engine_id.clone();
                                            let run_openai_api_key = openai_api_key.clone();
                                            let run_gemini_api_key = gemini_api_key.clone();
                                            let run_assemblyai_api_key = assemblyai_api_key.clone();
                                            let run_ffmpeg_path = ffmpeg_path.clone();
                                            let run_model = model.clone();
                                            let run_max_subtitle_duration_sec = max_subtitle_duration_sec;
                                            let run_max_subtitle_chars = max_subtitle_chars;
                                            let run_timing_max_window_sec = timing_max_window_sec;
                                            let run_vad_db_offset = vad_db_offset;
                                            let run_vad_merge_gap_sec = vad_merge_gap_sec;
                                            let result = cx
                                                .background_spawn(async move {
                                                    Self::run_ai_pipeline(
                                                        &run_input_path,
                                                        &run_out_srt_path,
                                                        &run_out_txt_path,
                                                        &run_lang,
                                                        &run_engine_id,
                                                        &run_ffmpeg_path,
                                                        run_model.as_ref(),
                                                        &run_openai_api_key,
                                                        &run_gemini_api_key,
                                                        &run_assemblyai_api_key,
                                                        run_max_subtitle_duration_sec,
                                                        run_max_subtitle_chars,
                                                        run_timing_max_window_sec,
                                                        run_vad_db_offset,
                                                        run_vad_merge_gap_sec,
                                                    )
                                                })
                                                .await;

                                            let _ = view.update(cx, |this, cx| {
                                                this.is_running = false;
                                                match result {
                                                    Ok(stdout) => {
                                                        if let Some(runtime_backend) =
                                                            Self::parse_runtime_backend_from_stdout(&stdout)
                                                        {
                                                            this.last_runtime_backend = runtime_backend;
                                                        } else {
                                                            this.last_runtime_backend = Self::runtime_hint_for_engine(&engine_id);
                                                        }
                                                        this.status_text = "SRT generated.".to_string();
                                                        if import_after {
                                                            match std::fs::read_to_string(&out_srt_path) {
                                                                Ok(srt_text) => {
                                                                    global.update(cx, |gs, cx| {
                                                                        match gs.import_srt(&srt_text) {
                                                                            Ok(count) => {
                                                                                gs.ui_notice = Some(format!("AI SRT imported: {count} cues."));
                                                                                gs.set_active_page(crate::core::global_state::AppPage::Editor);
                                                                            }
                                                                            Err(err) => {
                                                                                gs.ui_notice = Some(format!("SRT import failed: {err}"));
                                                                            }
                                                                        }
                                                                        cx.notify();
                                                                    });
                                                                }
                                                                Err(err) => {
                                                                    global.update(cx, |gs, cx| {
                                                                        gs.ui_notice = Some(format!("Read SRT failed: {err}"));
                                                                        cx.notify();
                                                                    });
                                                                }
                                                            }
                                                        } else {
                                                            global.update(cx, |gs, cx| {
                                                                gs.ui_notice = Some(format!(
                                                                    "Generated files: {} | {}",
                                                                    out_srt_path, out_txt_path
                                                                ));
                                                                cx.notify();
                                                            });
                                                        }
                                                        if !stdout.trim().is_empty() {
                                                            println!("[AI-SRT] {stdout}");
                                                        }
                                                    }
                                                    Err(err) => {
                                                        this.last_runtime_backend = Self::runtime_hint_for_engine(&engine_id);
                                                        this.status_text = "Generation failed.".to_string();
                                                        global.update(cx, |gs, cx| {
                                                            gs.ui_notice = Some(format!("AI SRT failed: {err}"));
                                                            cx.notify();
                                                        });
                                                    }
                                                }
                                                cx.notify();
                                            });
                                        }).detach();
                                    }));
                                }
                                btn
                            })
                    )
                    .child(
                        div()
                            .rounded_sm()
                            .border_1()
                            .border_color(rgba(0xf59e0b55))
                            .bg(rgba(0x42200688))
                            .px_2()
                            .py_1()
                            .text_sm()
                            .text_color(white().opacity(0.75))
                            .child(format!("Status: {}", self.status_text))
                    )
                    .child(
                        div()
                            .rounded_sm()
                            .border_1()
                            .border_color(rgba(0x60a5fa55))
                            .bg(rgba(0x17255488))
                            .px_2()
                            .py_1()
                            .text_sm()
                            .text_color(white().opacity(0.65))
                            .child(format!("Runtime backend: {}", self.last_runtime_backend))
                    )
            )
            )
    }
}
