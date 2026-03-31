// =========================================
// =========================================
// crates/ai-subtitle-engine/src/main.rs
mod cloud_api_connect;

use anyhow::{Result, anyhow};
use hound;
use ort::ep::ExecutionProvider as _;
use ort::{
    ep,
    session::{
        Session,
        builder::GraphOptimizationLevel,
        run_options::{OutputSelector, RunOptions},
    },
    value::Tensor,
};
use serde::Deserialize;
use std::sync::OnceLock;
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};
use tokenizers::Tokenizer;

use rustfft::{FftPlanner, num_complex::Complex32};

const MAX_PROMPT_TOKENS: usize = 64;

// Keep subtitle cues readable by limiting single-cue on-screen duration.
const DEFAULT_MAX_SUBTITLE_DURATION_SEC: f32 = 6.0;
// Keep each subtitle cue compact enough for two-line display in most players.
const DEFAULT_MAX_SUBTITLE_CHARS: usize = 42;
// VAD tuning for segment-level timestamp estimation.
const DEFAULT_VAD_DB_OFFSET: f32 = 8.0;
const VAD_MIN_SEG_SEC: f32 = 0.25;
const DEFAULT_VAD_MERGE_GAP_SEC: f32 = 0.30;
const VAD_PAD_SEC: f32 = 0.10;
// Keep timing windows short to reduce subtitle drift on continuous speech/singing.
const DEFAULT_TIMING_MAX_WINDOW_SEC: f32 = 15.0;
const TIMING_WINDOW_OVERLAP_SEC: f32 = 0.20;
// Whisper timestamp token resolution is 20ms per step.
const TIMESTAMP_RESOLUTION_SEC: f32 = 0.02;
// Disable prompt tail by default in timestamp mode to reduce repeated-line hallucinations.
const USE_PROMPT_TAIL_WITH_TIMESTAMPS: bool = false;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EngineMode {
    LocalOnnx,
    OpenAiWhisper1,
    OpenAiWhisper1Plus4oMerge,
    Gpt4oTranscribe,
    Gpt4oTranscribeDiarize,
    Gpt4oMiniTranscribe,
    Gpt4oMiniTts,
    Gemini25Pro,
    Gemini25Flash,
    AssemblyAi,
}

impl EngineMode {
    // Parse engine ids from CLI/UI into one runtime enum so dispatch is explicit.
    fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "local_onnx" => Ok(Self::LocalOnnx),
            "openai_whisper_1" => Ok(Self::OpenAiWhisper1),
            "openai_whisper_1_plus_4o_merge" => Ok(Self::OpenAiWhisper1Plus4oMerge),
            "gpt4o_transcribe" => Ok(Self::Gpt4oTranscribe),
            "gpt4o_transcribe_diarize" => Ok(Self::Gpt4oTranscribeDiarize),
            "gpt4o_mini_transcribe" => Ok(Self::Gpt4oMiniTranscribe),
            "gpt4o_mini_tts" => Ok(Self::Gpt4oMiniTts),
            "gemini_25_pro" => Ok(Self::Gemini25Pro),
            "gemini_25_flash" => Ok(Self::Gemini25Flash),
            "assemblyai" => Ok(Self::AssemblyAi),
            other => Err(anyhow!(
                "Unsupported --engine '{other}'. Supported values: local_onnx, openai_whisper_1, openai_whisper_1_plus_4o_merge, gpt4o_transcribe, gpt4o_transcribe_diarize, gpt4o_mini_transcribe, gpt4o_mini_tts, gemini_25_pro, gemini_25_flash, assemblyai."
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::LocalOnnx => "local_onnx",
            Self::OpenAiWhisper1 => "openai_whisper_1",
            Self::OpenAiWhisper1Plus4oMerge => "openai_whisper_1_plus_4o_merge",
            Self::Gpt4oTranscribe => "gpt4o_transcribe",
            Self::Gpt4oTranscribeDiarize => "gpt4o_transcribe_diarize",
            Self::Gpt4oMiniTranscribe => "gpt4o_mini_transcribe",
            Self::Gpt4oMiniTts => "gpt4o_mini_tts",
            Self::Gemini25Pro => "gemini_25_pro",
            Self::Gemini25Flash => "gemini_25_flash",
            Self::AssemblyAi => "assemblyai",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeKind {
    WhisperSeq2SeqV1,
}

impl RuntimeKind {
    // Parse runtime kind from manifest and keep backward compatibility for older Whisper packs.
    fn parse(raw: Option<&str>) -> Result<Self> {
        let normalized = raw
            .unwrap_or("whisper_seq2seq_v1")
            .trim()
            .to_ascii_lowercase();
        match normalized.as_str() {
            "whisper_seq2seq_v1" | "whisper_seq2seq" | "whisper" => Ok(Self::WhisperSeq2SeqV1),
            other => Err(anyhow!(
                "Unsupported runtime_kind '{other}'. Currently supported: whisper_seq2seq_v1."
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::WhisperSeq2SeqV1 => "whisper_seq2seq_v1",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelArchitecture {
    WhisperSeq2Seq,
}

impl ModelArchitecture {
    // Normalize architecture aliases so contributor-provided manifests stay compatible.
    fn parse(raw: &str) -> Result<Self> {
        let normalized = raw.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "whisper" | "whisper_seq2seq" | "whisperforconditionalgeneration" => {
                Ok(ModelArchitecture::WhisperSeq2Seq)
            }
            other => Err(anyhow!(
                "Unsupported model architecture '{other}'. Currently supported: whisper_seq2seq."
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrontendKind {
    WhisperLogMel,
}

impl FrontendKind {
    // Normalize frontend aliases so manifests can use short or descriptive names.
    fn parse(raw: &str) -> Result<Self> {
        let normalized = raw.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "mel" | "log_mel" | "whisper_log_mel" => Ok(FrontendKind::WhisperLogMel),
            other => Err(anyhow!(
                "Unsupported frontend '{other}'. Currently supported: whisper_log_mel."
            )),
        }
    }
}

#[derive(Debug, Clone)]
struct ModelFrontendConfig {
    runtime_kind: RuntimeKind,
    architecture: ModelArchitecture,
    frontend: FrontendKind,
    sample_rate: usize,
    n_fft: usize,
    hop_length: usize,
    n_mels: usize,
    chunk_length_sec: usize,
    overlap_frames: usize,
    max_decode_steps: usize,
}

impl ModelFrontendConfig {
    // Derive expected frame count for each decode chunk from manifest-driven audio parameters.
    fn n_frames(&self) -> usize {
        self.chunk_length_sec
            .saturating_mul(self.sample_rate)
            .checked_div(self.hop_length.max(1))
            .unwrap_or(0)
    }

    // Validate frontend dimensions early so bad manifests fail fast with actionable errors.
    fn validate(&self) -> Result<()> {
        if self.sample_rate == 0 {
            return Err(anyhow!("sample_rate must be > 0"));
        }
        if self.n_fft < 8 {
            return Err(anyhow!("n_fft must be >= 8"));
        }
        if self.hop_length == 0 {
            return Err(anyhow!("hop_length must be > 0"));
        }
        if self.n_mels == 0 {
            return Err(anyhow!("n_mels must be > 0"));
        }
        if self.chunk_length_sec == 0 {
            return Err(anyhow!("chunk_length_sec must be > 0"));
        }
        if self.n_frames() == 0 {
            return Err(anyhow!(
                "Derived n_frames is 0. Check sample_rate / hop_length / chunk_length_sec."
            ));
        }
        if self.max_decode_steps == 0 {
            return Err(anyhow!("max_decode_steps must be > 0"));
        }
        // Keep runtime + architecture/frontend pairing strict to make pack behavior deterministic.
        if self.runtime_kind == RuntimeKind::WhisperSeq2SeqV1
            && (self.architecture != ModelArchitecture::WhisperSeq2Seq
                || self.frontend != FrontendKind::WhisperLogMel)
        {
            return Err(anyhow!(
                "runtime_kind=whisper_seq2seq_v1 requires architecture=whisper_seq2seq and frontend=whisper_log_mel."
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct SubtitleTuning {
    max_subtitle_duration_sec: f32,
    max_subtitle_chars: usize,
    timing_max_window_sec: f32,
    vad_db_offset: f32,
    vad_merge_gap_sec: f32,
}

impl Default for SubtitleTuning {
    fn default() -> Self {
        Self {
            max_subtitle_duration_sec: DEFAULT_MAX_SUBTITLE_DURATION_SEC,
            max_subtitle_chars: DEFAULT_MAX_SUBTITLE_CHARS,
            timing_max_window_sec: DEFAULT_TIMING_MAX_WINDOW_SEC,
            vad_db_offset: DEFAULT_VAD_DB_OFFSET,
            vad_merge_gap_sec: DEFAULT_VAD_MERGE_GAP_SEC,
        }
    }
}

static SUBTITLE_TUNING: OnceLock<SubtitleTuning> = OnceLock::new();
static MODEL_FRONTEND: OnceLock<ModelFrontendConfig> = OnceLock::new();

// Read active subtitle tuning chosen by CLI args (or defaults when not provided).
fn subtitle_tuning() -> &'static SubtitleTuning {
    SUBTITLE_TUNING.get_or_init(SubtitleTuning::default)
}

// Read active model frontend settings chosen by manifest/config at startup.
fn model_frontend() -> &'static ModelFrontendConfig {
    MODEL_FRONTEND
        .get()
        .expect("Model frontend not initialized before runtime usage")
}

#[derive(Debug, Clone)]
struct CliConfig {
    engine_mode: EngineMode,
    model_dir: Option<PathBuf>,
    encoder_path: PathBuf,
    decoder_path: PathBuf,
    tokenizer_path: PathBuf,
    wav_path: PathBuf,
    srt_out_path: PathBuf,
    txt_out_path: PathBuf,
    language_mode: LanguagePromptMode,
    tuning: SubtitleTuning,
    frontend: ModelFrontendConfig,
}

#[derive(Debug, Clone)]
enum LanguagePromptMode {
    AutoDetect,
    ForcedToken(String),
}

impl LanguagePromptMode {
    // Keep CLI and logs explicit so users can tell whether language is forced or auto-detected.
    fn display_label(&self) -> &str {
        match self {
            Self::AutoDetect => "auto",
            Self::ForcedToken(token) => token.as_str(),
        }
    }
}

#[derive(Debug, Clone)]
struct EmittedSubtitleState {
    end_sec: f32,
    norm_text: String,
}

// Allow optional metadata fields so contributors can document model origin/variant in manifest.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct ModelPackManifest {
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
    architecture: Option<String>,
    #[serde(default)]
    frontend: Option<String>,
    #[serde(default)]
    sample_rate: Option<usize>,
    #[serde(default)]
    n_fft: Option<usize>,
    #[serde(default)]
    hop_length: Option<usize>,
    #[serde(default)]
    n_mels: Option<usize>,
    #[serde(default)]
    chunk_length_sec: Option<usize>,
    #[serde(default)]
    overlap_frames: Option<usize>,
    #[serde(default)]
    max_decode_steps: Option<usize>,
    #[serde(default)]
    model_author: Option<String>,
    #[serde(default)]
    model_repo: Option<String>,
    #[serde(default)]
    precision: Option<String>,
    #[serde(default)]
    variant: Option<String>,
}

fn default_model_config_ref() -> String {
    "config.json".to_string()
}

fn default_preprocessor_config_ref() -> String {
    "preprocessor_config.json".to_string()
}

#[derive(Debug, Clone, Deserialize)]
struct ExternalModelConfig {
    #[serde(default)]
    architectures: Vec<String>,
    #[serde(default)]
    model_type: Option<String>,
    #[serde(default)]
    num_mel_bins: Option<usize>,
    #[serde(default)]
    max_source_positions: Option<usize>,
    #[serde(default)]
    max_target_positions: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
struct ExternalPreprocessorConfig {
    #[serde(default)]
    sampling_rate: Option<usize>,
    #[serde(default)]
    n_fft: Option<usize>,
    #[serde(default)]
    hop_length: Option<usize>,
    #[serde(default)]
    feature_size: Option<usize>,
    #[serde(default)]
    chunk_length: Option<f32>,
    #[serde(default)]
    nb_max_frames: Option<usize>,
}

fn usage() -> &'static str {
    "Usage: cargo run -p ai-subtitle-engine -- [OPTIONS]
Options:
  --wav <path>         Input wav file (mono, sample_rate must match manifest/config)
  --engine <id>        Transcription backend (local_onnx|openai_whisper_1|openai_whisper_1_plus_4o_merge|gpt4o_transcribe|gpt4o_transcribe_diarize|gpt4o_mini_transcribe|gpt4o_mini_tts|gemini_25_pro|gemini_25_flash|assemblyai)
  --model-dir <path>   Model pack folder containing manifest.json
  --encoder <path>     Encoder ONNX path
  --decoder <path>     Decoder ONNX path
  --tokenizer <path>   tokenizer.json path
  --out <path>         Output .srt path
  --txt-out <path>     Output transcript .txt path
  --lang <code|token|auto>  Language code/token or auto-detect (default: auto)
  --max-subtitle-duration-sec <f32>  Max duration per subtitle cue
  --max-subtitle-chars <usize>       Max chars per subtitle cue
  --timing-max-window-sec <f32>      Max VAD timing window length
  --vad-db-offset <f32>              VAD speech threshold offset in dB
  --vad-merge-gap-sec <f32>          VAD merge gap for near speech chunks
  -h, --help           Show this help"
}

// Read one model pack manifest so default model wiring is explicit and stable.
fn read_model_manifest(model_dir: &Path) -> Result<ModelPackManifest> {
    let manifest_path = model_dir.join("manifest.json");
    let manifest_text = fs::read_to_string(&manifest_path).map_err(|e| {
        anyhow!(
            "Failed to read model manifest '{}': {e}",
            manifest_path.display()
        )
    })?;
    let manifest: ModelPackManifest = serde_json::from_str(&manifest_text)
        .map_err(|e| anyhow!("Invalid model manifest '{}': {e}", manifest_path.display()))?;
    if manifest.encoder.trim().is_empty()
        || manifest.decoder.trim().is_empty()
        || manifest.tokenizer.trim().is_empty()
    {
        return Err(anyhow!(
            "Model manifest '{}' must define non-empty encoder/decoder/tokenizer.",
            manifest_path.display()
        ));
    }
    // Enforce model_config presence so every model pack has a stable frontend source.
    if manifest.model_config.trim().is_empty()
        || manifest.model_config.trim().eq_ignore_ascii_case("none")
    {
        return Err(anyhow!(
            "Model manifest '{}' must define model_config (e.g. config.json).",
            manifest_path.display()
        ));
    }
    Ok(manifest)
}

// Load required model config JSON referenced by manifest (`model_config`).
fn read_external_model_config(
    model_dir: &Path,
    manifest: &ModelPackManifest,
) -> Result<ExternalModelConfig> {
    let config_ref = manifest.model_config.trim();
    let config_path = model_dir.join(config_ref);
    let config_text = fs::read_to_string(&config_path).map_err(|e| {
        anyhow!(
            "Failed to read model_config '{}': {e}",
            config_path.display()
        )
    })?;
    let config: ExternalModelConfig = serde_json::from_str(&config_text)
        .map_err(|e| anyhow!("Invalid model_config JSON '{}': {e}", config_path.display()))?;
    Ok(config)
}

// Load optional preprocessor config to fill audio frontend values when available.
fn read_external_preprocessor_config(
    model_dir: &Path,
    manifest: &ModelPackManifest,
) -> Result<Option<ExternalPreprocessorConfig>> {
    let config_ref = manifest.preprocessor_config.trim();
    if config_ref.is_empty() || config_ref.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let config_path = model_dir.join(config_ref);
    if !config_path.exists() {
        return Ok(None);
    }
    let config_text = fs::read_to_string(&config_path).map_err(|e| {
        anyhow!(
            "Failed to read preprocessor_config '{}': {e}",
            config_path.display()
        )
    })?;
    let config: ExternalPreprocessorConfig = serde_json::from_str(&config_text).map_err(|e| {
        anyhow!(
            "Invalid preprocessor_config JSON '{}': {e}",
            config_path.display()
        )
    })?;
    Ok(Some(config))
}

// Load tokenizer with one compatibility fallback for ONNX-community packs that include unsupported fields.
fn load_tokenizer_with_compat(path: &Path) -> Result<Tokenizer> {
    match Tokenizer::from_file(path) {
        Ok(tokenizer) => Ok(tokenizer),
        Err(initial_err) => {
            let raw = fs::read(path)
                .map_err(|e| anyhow!("Failed to read tokenizer '{}': {e}", path.display()))?;
            let mut json: serde_json::Value = serde_json::from_slice(&raw).map_err(|_| {
                anyhow!(
                    "Failed to load tokenizer '{}': {initial_err}",
                    path.display()
                )
            })?;
            let mut fixes_applied: Vec<&'static str> = Vec::new();

            // Remove legacy field that older tokenizers crate cannot deserialize.
            let removed = json
                .get_mut("model")
                .and_then(|model| model.as_object_mut())
                .and_then(|model_obj| model_obj.remove("ignore_merges"))
                .is_some();
            if removed {
                fixes_applied.push("removed model.ignore_merges");
            }

            // Normalize merges format from [["a","b"], ...] into ["a b", ...] for crates expecting string merges.
            if let Some(merges) = json
                .get_mut("model")
                .and_then(|model| model.get_mut("merges"))
                .and_then(|merges| merges.as_array_mut())
            {
                let has_pair_format = merges.iter().any(|entry| entry.is_array());
                if has_pair_format {
                    let mut normalized = Vec::with_capacity(merges.len());
                    for entry in merges.iter() {
                        if let Some(pair) = entry.as_array() {
                            let left = pair.first().and_then(|v| v.as_str()).unwrap_or("");
                            let right = pair.get(1).and_then(|v| v.as_str()).unwrap_or("");
                            normalized.push(serde_json::Value::String(format!("{left} {right}")));
                        } else {
                            normalized.push(entry.clone());
                        }
                    }
                    *merges = normalized;
                    fixes_applied.push("normalized model.merges pair format");
                }
            }

            if fixes_applied.is_empty() {
                return Err(anyhow!(
                    "Failed to load tokenizer '{}': {initial_err}",
                    path.display()
                ));
            }

            println!(
                "Tokenizer compatibility fallback applied for {}: {}",
                path.display(),
                fixes_applied.join(", ")
            );

            let cleaned_bytes = serde_json::to_vec(&json)
                .map_err(|e| anyhow!("Failed to serialize cleaned tokenizer JSON: {e}"))?;
            Tokenizer::from_bytes(cleaned_bytes).map_err(|e| {
                anyhow!(
                    "Failed to load tokenizer '{}' after compatibility cleanup: {e}",
                    path.display()
                )
            })
        }
    }
}

// Infer model architecture from explicit manifest override, then config architecture/model_type.
fn resolve_architecture(
    manifest: &ModelPackManifest,
    external_model: &ExternalModelConfig,
) -> Result<ModelArchitecture> {
    if let Some(raw) = manifest.architecture.as_deref() {
        return ModelArchitecture::parse(raw);
    }
    if let Some(raw) = external_model.architectures.first() {
        return ModelArchitecture::parse(raw);
    }
    if let Some(raw) = external_model.model_type.as_deref() {
        return ModelArchitecture::parse(raw);
    }
    Err(anyhow!(
        "Cannot resolve model architecture. Provide manifest.architecture or config.json architectures/model_type."
    ))
}

// Infer frontend from explicit manifest override, then architecture/model_type.
fn resolve_frontend(
    manifest: &ModelPackManifest,
    architecture: ModelArchitecture,
    external_model: &ExternalModelConfig,
) -> Result<FrontendKind> {
    if let Some(raw) = manifest.frontend.as_deref() {
        return FrontendKind::parse(raw);
    }
    if let Some(raw) = external_model.model_type.as_deref() {
        if raw.trim().eq_ignore_ascii_case("whisper") {
            return Ok(FrontendKind::WhisperLogMel);
        }
    }
    match architecture {
        ModelArchitecture::WhisperSeq2Seq => Ok(FrontendKind::WhisperLogMel),
    }
}

// Convert preprocessor chunk length fields to integer seconds.
fn derive_chunk_length_from_preprocessor(
    pre: Option<&ExternalPreprocessorConfig>,
    sample_rate: usize,
    hop_length: usize,
) -> Option<usize> {
    let pre = pre?;
    if let Some(v) = pre.chunk_length {
        if v.is_finite() && v > 0.0 {
            return Some(v.round().max(1.0) as usize);
        }
    }
    if let Some(v) = pre.nb_max_frames {
        if v > 0 && sample_rate > 0 && hop_length > 0 {
            return Some(
                ((v as f32 * hop_length as f32) / sample_rate as f32)
                    .round()
                    .max(1.0) as usize,
            );
        }
    }
    None
}

// Convert model config source length to chunk seconds for Whisper-style encoders.
fn derive_chunk_length_from_model_config(
    external_model: &ExternalModelConfig,
    sample_rate: usize,
    hop_length: usize,
) -> Option<usize> {
    let max_source_positions = external_model.max_source_positions?;
    if max_source_positions == 0 || sample_rate == 0 || hop_length == 0 {
        return None;
    }
    let inferred_frames = max_source_positions.saturating_mul(2);
    Some(
        ((inferred_frames as f32 * hop_length as f32) / sample_rate as f32)
            .round()
            .max(1.0) as usize,
    )
}

// Convert model_config + preprocessor_config + manifest fields into runtime frontend settings.
// Precedence: model/preprocessor config < manifest explicit override.
fn frontend_from_manifest(
    model_dir: &Path,
    manifest: &ModelPackManifest,
) -> Result<ModelFrontendConfig> {
    let external_model = read_external_model_config(model_dir, manifest)?;
    let external_pre = read_external_preprocessor_config(model_dir, manifest)?;
    // Resolve runtime kind first so invalid model packs fail before runtime decode starts.
    let runtime_kind = RuntimeKind::parse(manifest.runtime_kind.as_deref())?;
    let architecture = resolve_architecture(manifest, &external_model)?;
    let frontend = resolve_frontend(manifest, architecture, &external_model)?;

    let sample_rate = manifest
        .sample_rate
        .or(external_pre.as_ref().and_then(|x| x.sampling_rate))
        .ok_or_else(|| anyhow!("Missing sample_rate in model pack config/manifest."))?;
    let n_fft = manifest
        .n_fft
        .or(external_pre.as_ref().and_then(|x| x.n_fft))
        .ok_or_else(|| anyhow!("Missing n_fft in preprocessor_config or manifest."))?;
    let hop_length = manifest
        .hop_length
        .or(external_pre.as_ref().and_then(|x| x.hop_length))
        .ok_or_else(|| anyhow!("Missing hop_length in preprocessor_config or manifest."))?;
    let n_mels = manifest
        .n_mels
        .or(external_pre.as_ref().and_then(|x| x.feature_size))
        .or(external_model.num_mel_bins)
        .ok_or_else(|| anyhow!("Missing n_mels/feature_size/num_mel_bins in model pack."))?;
    let chunk_length_sec = manifest
        .chunk_length_sec
        .or_else(|| {
            derive_chunk_length_from_preprocessor(external_pre.as_ref(), sample_rate, hop_length)
        })
        .or_else(|| derive_chunk_length_from_model_config(&external_model, sample_rate, hop_length))
        .ok_or_else(|| anyhow!("Missing chunk_length metadata; add manifest.chunk_length_sec."))?;
    let overlap_frames = manifest
        .overlap_frames
        .ok_or_else(|| anyhow!("Missing overlap_frames in manifest.json."))?;
    // Keep decoder step limit aligned with model config so runtime does not clip valid outputs.
    let max_decode_steps = manifest
        .max_decode_steps
        .or(external_model.max_target_positions)
        .ok_or_else(|| anyhow!("Missing max_decode_steps/max_target_positions in model pack."))?;

    let cfg = ModelFrontendConfig {
        runtime_kind,
        architecture,
        frontend,
        sample_rate,
        n_fft,
        hop_length,
        n_mels,
        chunk_length_sec,
        overlap_frames,
        max_decode_steps,
    };
    cfg.validate()?;
    Ok(cfg)
}

fn parse_cli() -> Result<CliConfig> {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // Default to the bundled model pack folder so model artifacts stay together.
    let default_model_dir = crate_dir
        .join("src")
        .join("model")
        .join("onnx")
        .join("whisper_large_v3_turbo");
    // Resolve default model files from manifest instead of ambiguous hardcoded fallbacks.
    let default_manifest = read_model_manifest(&default_model_dir)?;
    let default_frontend = frontend_from_manifest(&default_model_dir, &default_manifest)?;
    let default_encoder = default_model_dir.join(&default_manifest.encoder);
    let default_decoder = default_model_dir.join(&default_manifest.decoder);
    let default_tokenizer = default_model_dir.join(&default_manifest.tokenizer);
    let default_srt = crate_dir.join("transcript.srt");
    let default_txt = crate_dir.join("transcript.txt");

    let mut cfg = CliConfig {
        engine_mode: EngineMode::LocalOnnx,
        model_dir: Some(default_model_dir.clone()),
        encoder_path: default_encoder,
        decoder_path: default_decoder,
        tokenizer_path: default_tokenizer,
        wav_path: PathBuf::new(),
        srt_out_path: default_srt,
        txt_out_path: default_txt,
        language_mode: LanguagePromptMode::AutoDetect,
        tuning: SubtitleTuning::default(),
        frontend: default_frontend,
    };
    let mut explicit_encoder = false;
    let mut explicit_decoder = false;
    let mut explicit_tokenizer = false;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{}", usage());
                std::process::exit(0);
            }
            "--model-dir" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow!("--model-dir requires a path"))?;
                cfg.model_dir = Some(PathBuf::from(raw));
            }
            "--wav" => {
                cfg.wav_path = PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow!("--wav requires a path"))?,
                )
            }
            "--engine" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow!("--engine requires a value"))?;
                cfg.engine_mode = EngineMode::parse(&raw)?;
            }
            "--encoder" => {
                cfg.encoder_path = PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow!("--encoder requires a path"))?,
                );
                explicit_encoder = true;
            }
            "--decoder" => {
                cfg.decoder_path = PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow!("--decoder requires a path"))?,
                );
                explicit_decoder = true;
            }
            "--tokenizer" => {
                cfg.tokenizer_path = PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow!("--tokenizer requires a path"))?,
                );
                explicit_tokenizer = true;
            }
            "--out" => {
                cfg.srt_out_path = PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow!("--out requires a path"))?,
                )
            }
            "--txt-out" => {
                cfg.txt_out_path = PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow!("--txt-out requires a path"))?,
                )
            }
            "--lang" => {
                let lang = args
                    .next()
                    .ok_or_else(|| anyhow!("--lang requires a value"))?;
                let normalized = lang.trim();
                // Support explicit language forcing and auto mode without changing model pack files.
                cfg.language_mode = if normalized.eq_ignore_ascii_case("auto")
                    || normalized.eq_ignore_ascii_case("<|auto|>")
                {
                    LanguagePromptMode::AutoDetect
                } else if normalized.starts_with("<|") && normalized.ends_with("|>") {
                    LanguagePromptMode::ForcedToken(normalized.to_string())
                } else {
                    LanguagePromptMode::ForcedToken(format!("<|{}|>", normalized))
                };
            }
            "--max-subtitle-duration-sec" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow!("--max-subtitle-duration-sec requires a value"))?;
                let value: f32 = raw
                    .parse()
                    .map_err(|_| anyhow!("invalid f32 for --max-subtitle-duration-sec: {raw}"))?;
                // Keep duration bounds stable to prevent unusable zero/huge cue windows.
                cfg.tuning.max_subtitle_duration_sec = value.clamp(1.0, 20.0);
            }
            "--max-subtitle-chars" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow!("--max-subtitle-chars requires a value"))?;
                let value: usize = raw
                    .parse()
                    .map_err(|_| anyhow!("invalid usize for --max-subtitle-chars: {raw}"))?;
                // Bound chars to keep readability and avoid pathological long lines.
                cfg.tuning.max_subtitle_chars = value.clamp(8, 200);
            }
            "--timing-max-window-sec" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow!("--timing-max-window-sec requires a value"))?;
                let value: f32 = raw
                    .parse()
                    .map_err(|_| anyhow!("invalid f32 for --timing-max-window-sec: {raw}"))?;
                // Store requested value first; final clamp is done after manifest frontend is loaded.
                cfg.tuning.timing_max_window_sec = value;
            }
            "--vad-db-offset" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow!("--vad-db-offset requires a value"))?;
                let value: f32 = raw
                    .parse()
                    .map_err(|_| anyhow!("invalid f32 for --vad-db-offset: {raw}"))?;
                // Bound threshold offset so VAD remains numerically stable.
                cfg.tuning.vad_db_offset = value.clamp(0.0, 30.0);
            }
            "--vad-merge-gap-sec" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow!("--vad-merge-gap-sec requires a value"))?;
                let value: f32 = raw
                    .parse()
                    .map_err(|_| anyhow!("invalid f32 for --vad-merge-gap-sec: {raw}"))?;
                // Clamp merge gap to avoid over-joining distant speech chunks.
                cfg.tuning.vad_merge_gap_sec = value.clamp(0.0, 3.0);
            }
            unknown => return Err(anyhow!("Unknown argument: {unknown}\n\n{}", usage())),
        }
    }

    if cfg.engine_mode == EngineMode::LocalOnnx {
        if let Some(model_dir) = cfg.model_dir.clone() {
            // Re-load manifest from selected model directory so local frontend settings stay model-pack driven.
            let manifest = read_model_manifest(&model_dir)?;
            cfg.frontend = frontend_from_manifest(&model_dir, &manifest)?;
            if !explicit_encoder {
                cfg.encoder_path = model_dir.join(&manifest.encoder);
            }
            if !explicit_decoder {
                cfg.decoder_path = model_dir.join(&manifest.decoder);
            }
            if !explicit_tokenizer {
                cfg.tokenizer_path = model_dir.join(&manifest.tokenizer);
            }
        }
    }

    // Keep timing windows bounded even when running cloud engines.
    let timing_upper_bound = if cfg.engine_mode == EngineMode::LocalOnnx {
        cfg.frontend.chunk_length_sec as f32
    } else {
        30.0
    };
    cfg.tuning.timing_max_window_sec = cfg
        .tuning
        .timing_max_window_sec
        .clamp(2.0, timing_upper_bound);

    // Require WAV explicitly to avoid stale sample/test-file behavior.
    if cfg.wav_path.as_os_str().is_empty() {
        return Err(anyhow!("--wav is required"));
    }
    if cfg.engine_mode == EngineMode::LocalOnnx {
        if !cfg.encoder_path.exists() {
            return Err(anyhow!(
                "Encoder model not found: {}",
                cfg.encoder_path.display()
            ));
        }
        if !cfg.decoder_path.exists() {
            return Err(anyhow!(
                "Decoder model not found: {}",
                cfg.decoder_path.display()
            ));
        }
        if !cfg.tokenizer_path.exists() {
            return Err(anyhow!(
                "Tokenizer not found: {}",
                cfg.tokenizer_path.display()
            ));
        }
    }
    if !cfg.wav_path.exists() {
        return Err(anyhow!("Input wav not found: {}", cfg.wav_path.display()));
    }

    Ok(cfg)
}

#[derive(Debug, Clone)]
struct ExecutionProviderPlan {
    dispatches: Vec<ep::ExecutionProviderDispatch>,
    active_hint: String,
    chain_label: String,
}

// Build a platform-aware EP chain: try GPU-friendly providers first, always keep CPU as fallback.
fn build_local_execution_provider_plan() -> ExecutionProviderPlan {
    let mut dispatches: Vec<ep::ExecutionProviderDispatch> = Vec::new();
    let mut active_hint = "CPU".to_string();

    #[cfg(target_os = "macos")]
    {
        let coreml = ep::CoreML::default();
        if coreml.supported_by_platform() && coreml.is_available().unwrap_or(false) {
            active_hint = "CoreML".to_string();
            dispatches.push(coreml.build());
        }
    }

    #[cfg(target_os = "linux")]
    {
        let cuda = ep::CUDA::default();
        if cuda.supported_by_platform() && cuda.is_available().unwrap_or(false) {
            active_hint = "CUDA".to_string();
            dispatches.push(cuda.build());
        }
    }

    #[cfg(target_os = "windows")]
    {
        // Prefer CUDA on NVIDIA machines; keep DirectML as fallback for broader GPU coverage.
        let mut has_gpu_ep = false;
        let cuda = ep::CUDA::default();
        if cuda.supported_by_platform() && cuda.is_available().unwrap_or(false) {
            active_hint = "CUDA".to_string();
            has_gpu_ep = true;
            dispatches.push(cuda.build());
        }
        let directml = ep::DirectML::default();
        if directml.supported_by_platform() && directml.is_available().unwrap_or(false) {
            if !has_gpu_ep {
                active_hint = "DirectML".to_string();
            }
            dispatches.push(directml.build());
        }
    }

    // Always keep CPU at the end so local inference remains available even if GPU EP registration fails.
    dispatches.push(ep::CPU::default().build());
    let chain_label = if dispatches.len() == 1 {
        "CPU".to_string()
    } else {
        format!("{active_hint} -> CPU")
    };

    ExecutionProviderPlan {
        dispatches,
        active_hint,
        chain_label,
    }
}

fn main() -> Result<()> {
    let cfg = parse_cli()?;
    // Persist CLI-selected subtitle tuning so helper functions use one consistent config.
    let _ = SUBTITLE_TUNING.set(cfg.tuning.clone());
    let engine_id = cfg.engine_mode.as_str();
    let lang_code = match &cfg.language_mode {
        LanguagePromptMode::AutoDetect => None,
        LanguagePromptMode::ForcedToken(token) => {
            let code = token.trim().trim_start_matches("<|").trim_end_matches("|>");
            if code.is_empty() {
                None
            } else {
                Some(code.to_string())
            }
        }
    };

    // Run cloud backends before local ONNX initialization to avoid loading model files unnecessarily.
    if cfg.engine_mode != EngineMode::LocalOnnx {
        let provider = cloud_api_connect::CloudProvider::parse(engine_id)?;
        let request = cloud_api_connect::CloudRunRequest {
            provider,
            input_audio_path: cfg.wav_path.clone(),
            output_srt_path: cfg.srt_out_path.clone(),
            output_txt_path: cfg.txt_out_path.clone(),
            language_code: lang_code,
            max_subtitle_duration_sec: cfg.tuning.max_subtitle_duration_sec,
            max_subtitle_chars: cfg.tuning.max_subtitle_chars,
        };
        let summary = cloud_api_connect::run_cloud_transcription(&request)?;
        println!("{summary}");
        return Ok(());
    }

    // Persist manifest-selected frontend settings so helper functions stay model-pack driven.
    let _ = MODEL_FRONTEND.set(cfg.frontend.clone());
    // Keep runtime behavior explicit until non-Whisper runtime branches are implemented.
    if model_frontend().runtime_kind != RuntimeKind::WhisperSeq2SeqV1 {
        return Err(anyhow!(
            "Unsupported runtime_kind for current binary. Expected whisper_seq2seq_v1."
        ));
    }
    println!(
        "Running with:\n  engine: {}\n  wav: {}\n  model_dir: {}\n  encoder: {}\n  decoder: {}\n  tokenizer: {}\n  out: {}\n  txt: {}\n  lang: {}\n  runtime_kind: {}\n  frontend: whisper_log_mel\n  sample_rate: {}\n  n_fft: {}\n  hop_length: {}\n  n_mels: {}\n  chunk_length_sec: {}\n  n_frames: {}\n  overlap_frames: {}\n  max_decode_steps: {}\n  max_subtitle_duration_sec: {:.2}\n  max_subtitle_chars: {}\n  timing_max_window_sec: {:.2}\n  vad_db_offset: {:.2}\n  vad_merge_gap_sec: {:.2}",
        engine_id,
        cfg.wav_path.display(),
        cfg.model_dir
            .as_ref()
            .map(|x| x.display().to_string())
            .unwrap_or_else(|| "<none>".to_string()),
        cfg.encoder_path.display(),
        cfg.decoder_path.display(),
        cfg.tokenizer_path.display(),
        cfg.srt_out_path.display(),
        cfg.txt_out_path.display(),
        cfg.language_mode.display_label(),
        model_frontend().runtime_kind.as_str(),
        model_frontend().sample_rate,
        model_frontend().n_fft,
        model_frontend().hop_length,
        model_frontend().n_mels,
        model_frontend().chunk_length_sec,
        model_frontend().n_frames(),
        model_frontend().overlap_frames,
        model_frontend().max_decode_steps,
        cfg.tuning.max_subtitle_duration_sec,
        cfg.tuning.max_subtitle_chars,
        cfg.tuning.timing_max_window_sec,
        cfg.tuning.vad_db_offset,
        cfg.tuning.vad_merge_gap_sec,
    );
    // Emit machine-readable EP lines so parent UI can display the latest runtime backend.
    let ep_plan = build_local_execution_provider_plan();
    println!("[EP_ACTIVE] {}", ep_plan.active_hint);
    println!("[EP_CHAIN] {}", ep_plan.chain_label);

    // ===== ORT init =====
    let ok = ort::init()
        .with_name("whisper-ort")
        .with_execution_providers(ep_plan.dispatches.as_slice())
        .commit();
    if !ok {
        return Err(anyhow!("ort::init().commit() returned false"));
    }

    // ===== Load sessions =====
    let mut enc = Session::builder()?
        // Keep graph optimization conservative to avoid ONNXRuntime fusion crashes on some Whisper exports.
        .with_optimization_level(GraphOptimizationLevel::Level1)?
        .with_execution_providers(ep_plan.dispatches.as_slice())?
        .commit_from_file(&cfg.encoder_path)?;

    let mut dec = Session::builder()?
        // Use the same safe optimization level for decoder to keep model initialization stable.
        .with_optimization_level(GraphOptimizationLevel::Level1)?
        .with_execution_providers(ep_plan.dispatches.as_slice())?
        .commit_from_file(&cfg.decoder_path)?;

    println!("✅ Loaded ONNX encoder + decoder (no-cache).");
    print_io("ENC", &enc);
    print_io("DEC", &dec);

    // ===== Tokenizer =====
    let tokenizer = load_tokenizer_with_compat(&cfg.tokenizer_path)?;

    // ===== Audio -> Mel =====
    let samples = load_wav_mono(&cfg.wav_path)?;
    println!("Loaded wav samples: {}", samples.len());

    let (mel_all, total_frames) = build_model_features(&samples)?;
    println!("Mel frames: {}", total_frames);

    // ===== Decode tokens =====
    let start_token = tokenizer
        .token_to_id("<|startoftranscript|>")
        .ok_or_else(|| anyhow!("token not found: <|startoftranscript|>"))?
        as u32;

    let trans_token = tokenizer
        .token_to_id("<|transcribe|>")
        .ok_or_else(|| anyhow!("token not found: <|transcribe|>"))? as u32;
    // Keep language token optional so auto mode can let Whisper decide language from audio.
    let forced_lang_token = match &cfg.language_mode {
        LanguagePromptMode::AutoDetect => None,
        LanguagePromptMode::ForcedToken(token) => Some(
            tokenizer
                .token_to_id(token)
                .ok_or_else(|| anyhow!("token not found: {}", token))? as u32,
        ),
    };
    // Build one immutable prompt prefix used by normal and retry decode loops.
    let mut prompt_prefix_tokens = vec![start_token];
    if let Some(token) = forced_lang_token {
        prompt_prefix_tokens.push(token);
    }
    prompt_prefix_tokens.push(trans_token);
    let prompt_prefix_len = prompt_prefix_tokens.len();
    // Keep total decoder input length within model max_target_positions by reserving prefix slots.
    let max_generation_steps = model_frontend()
        .max_decode_steps
        .saturating_sub(prompt_prefix_len)
        .max(1);

    let eot = tokenizer.token_to_id("<|endoftext|>").unwrap_or(50257) as u32;
    // Discover timestamp token range from tokenizer; fallback to Whisper defaults when missing.
    let ts_begin_token = tokenizer.token_to_id("<|0.00|>").unwrap_or(50364) as u32;
    let ts_end_token = tokenizer
        .token_to_id("<|30.00|>")
        .unwrap_or(ts_begin_token + 1500) as u32;

    let mut prompt_tail: Vec<u32> = Vec::new();

    // Decoder input/output names are typically input_ids / encoder_hidden_states / logits.
    let dec_in_ids = dec.inputs()[0].name().to_string();
    let dec_in_xa = dec.inputs()[1].name().to_string();
    let dec_out_logits = dec.outputs()[0].name().to_string();
    // Support both 2-input decoders and merged decoders that also require cache-related inputs.
    let dec_has_cache_inputs = dec.inputs().iter().any(|x| x.name() == "use_cache_branch");
    let dec_past_input_names: Vec<String> = dec
        .inputs()
        .iter()
        .map(|x| x.name().to_string())
        .filter(|name| name.starts_with("past_key_values."))
        .collect();
    let dec_run_opts =
        RunOptions::new()?.with_outputs(OutputSelector::no_default().with(dec_out_logits.clone()));

    let mut transcript = String::new();
    let mut srt = String::new();
    print!("TRANSCRIPT: ");
    std::io::stdout().flush()?;

    // Build decode windows from VAD so timestamps follow speech boundaries instead of fixed chunks.
    let decode_windows = build_vad_decode_windows(&samples, total_frames);
    println!("VAD decode windows: {}", decode_windows.len());

    // ===== Chunked encode+decode =====
    let mut last_chunk_text = String::new();
    let mut last_emitted_subtitle: Option<EmittedSubtitleState> = None;
    for (chunk_idx, (chunk_start, chunk_end_frame)) in decode_windows.into_iter().enumerate() {
        let mel_chunk = slice_mel_chunk(
            &mel_all,
            total_frames,
            model_frontend().n_mels,
            chunk_start,
            model_frontend().n_frames(),
        );

        // Build encoder input shape from manifest-driven frontend dimensions.
        let mel_t: Tensor<f32> = Tensor::from_array((
            vec![
                1i64,
                model_frontend().n_mels as i64,
                model_frontend().n_frames() as i64,
            ],
            mel_chunk,
        ))?;

        // ===== Encoder forward =====
        let enc_in_name = enc.inputs()[0].name().to_string(); // input_features
        let enc_out_name = enc.outputs()[0].name().to_string(); // last_hidden_state

        let enc_outs = enc.run(ort::inputs![enc_in_name.as_str() => mel_t])?;
        let (xa_shape, xa_data) = enc_outs[enc_out_name.as_str()].try_extract_tensor::<f32>()?;

        let xa_dims: Vec<i64> = xa_shape.to_vec();
        println!(
            "\nEncoder out dims: {:?} (chunk @ frame {})",
            xa_dims, chunk_start
        );

        let (t, d) = match xa_dims.as_slice() {
            [1, t, d] => (*t as usize, *d as usize),
            [t, d] => (*t as usize, *d as usize),
            other => return Err(anyhow!("Unexpected encoder output shape: {:?}", other)),
        };

        // Decoder input is usually shaped as [1, T, D].
        let xa_tsr: Tensor<f32> =
            Tensor::from_array((vec![1i64, t as i64, d as i64], xa_data.to_vec()))?;
        // Build static merged-decoder placeholders once per chunk.
        let mut dec_static_past_inputs: Vec<(String, Tensor<f32>)> = Vec::new();
        let mut dec_use_cache_false: Option<Tensor<bool>> = None;
        if dec_has_cache_inputs {
            // Derive attention layout from hidden size using Whisper's common head dimension.
            let head_dim = if d % 64 == 0 { 64 } else { d.max(1) };
            let num_heads = (d / head_dim).max(1);
            let past_shape = vec![1i64, num_heads as i64, 1i64, head_dim as i64];
            let past_values = vec![0.0f32; num_heads * head_dim];
            for input_name in &dec_past_input_names {
                let tensor = Tensor::from_array((past_shape.clone(), past_values.clone()))?;
                dec_static_past_inputs.push((input_name.clone(), tensor));
            }
            dec_use_cache_false = Some(Tensor::from_array((vec![1i64], vec![false]))?);
        }

        let mut tokens: Vec<u32> = prompt_prefix_tokens.clone();
        if USE_PROMPT_TAIL_WITH_TIMESTAMPS && !prompt_tail.is_empty() {
            tokens.extend_from_slice(&prompt_tail);
        }

        let mut chunk_text = String::new();
        let mut generated_tokens: Vec<u32> = Vec::new();
        // Use model-driven decode limit while accounting for prompt prefix token count.
        for step in 0..max_generation_steps {
            // No-cache decoder path requires the full token sequence on every step.
            let ids_i64: Vec<i64> = tokens.iter().map(|&x| x as i64).collect();
            let ids_t: Tensor<i64> =
                Tensor::from_array((vec![1i64, ids_i64.len() as i64], ids_i64))?;

            let mut dec_inputs = ort::inputs![
                dec_in_ids.as_str() => ids_t,
                dec_in_xa.as_str()  => &xa_tsr,
            ];
            if let Some(use_cache_false) = &dec_use_cache_false {
                dec_inputs.push(("use_cache_branch".into(), use_cache_false.into()));
            }
            for (name, tensor) in &dec_static_past_inputs {
                dec_inputs.push((name.as_str().into(), tensor.into()));
            }
            // Add chunk/step/token diagnostics so ONNX node failures can be reproduced quickly.
            let outs = dec.run_with_options(dec_inputs, &dec_run_opts).map_err(|e| {
                anyhow!(
                    "Decoder run failed at chunk={chunk_idx}, step={step}, tokens_len={}, prompt_prefix_len={}, max_generation_steps={}, lang_mode={}: {e}",
                    tokens.len(),
                    prompt_prefix_len,
                    max_generation_steps,
                    cfg.language_mode.display_label(),
                )
            })?;

            let (log_shape, log_data) =
                outs[dec_out_logits.as_str()].try_extract_tensor::<f32>()?;
            let log_dims: Vec<i64> = log_shape.to_vec();

            // Logits are typically shaped as [1, seq, vocab].
            let (seq_len, vocab, base) = match log_dims.as_slice() {
                [1, s, v] => (
                    *s as usize,
                    *v as usize,
                    ((*s as usize) - 1) * (*v as usize),
                ),
                [s, v] => (
                    *s as usize,
                    *v as usize,
                    ((*s as usize) - 1) * (*v as usize),
                ),
                other => return Err(anyhow!("Unexpected logits shape: {:?}", other)),
            };

            if seq_len == 0 || vocab == 0 {
                return Err(anyhow!("Bad logits shape: {:?}", log_dims));
            }

            let last_row = &log_data[base..base + vocab];
            let mut best_i = 0usize;
            let mut best_v = f32::NEG_INFINITY;
            for (i, &v) in last_row.iter().enumerate() {
                if v > best_v {
                    best_v = v;
                    best_i = i;
                }
            }
            let next = best_i as u32;

            if next == eot {
                break;
            }

            tokens.push(next);
            generated_tokens.push(next);
            if !is_timestamp_token(next, ts_begin_token, ts_end_token) {
                let decoded = tokenizer
                    .decode(&[next], true)
                    .map_err(|e| anyhow!("{e}"))?;
                chunk_text.push_str(&decoded);
                print!("{decoded}");
                std::io::stdout().flush()?;
            }
        }

        if chunk_text.trim().is_empty() && !prompt_tail.is_empty() {
            // If prompt tail causes immediate EOT, retry without it for this chunk.
            let mut retry_tokens: Vec<u32> = prompt_prefix_tokens.clone();
            let mut retry_text = String::new();
            let mut retry_generated: Vec<u32> = Vec::new();
            // Use model-driven decode limit while accounting for prompt prefix token count.
            for step in 0..max_generation_steps {
                let ids_i64: Vec<i64> = retry_tokens.iter().map(|&x| x as i64).collect();
                let ids_t: Tensor<i64> =
                    Tensor::from_array((vec![1i64, ids_i64.len() as i64], ids_i64))?;
                let mut dec_inputs = ort::inputs![
                    dec_in_ids.as_str() => ids_t,
                    dec_in_xa.as_str()  => &xa_tsr,
                ];
                if let Some(use_cache_false) = &dec_use_cache_false {
                    dec_inputs.push(("use_cache_branch".into(), use_cache_false.into()));
                }
                for (name, tensor) in &dec_static_past_inputs {
                    dec_inputs.push((name.as_str().into(), tensor.into()));
                }
                // Mirror diagnostics for retry path to capture failures after prompt-tail fallback.
                let outs = dec.run_with_options(dec_inputs, &dec_run_opts).map_err(|e| {
                    anyhow!(
                        "Decoder retry failed at chunk={chunk_idx}, step={step}, tokens_len={}, prompt_prefix_len={}, max_generation_steps={}, lang_mode={}: {e}",
                        retry_tokens.len(),
                        prompt_prefix_len,
                        max_generation_steps,
                        cfg.language_mode.display_label(),
                    )
                })?;
                let (log_shape, log_data) =
                    outs[dec_out_logits.as_str()].try_extract_tensor::<f32>()?;
                let log_dims: Vec<i64> = log_shape.to_vec();
                let (seq_len, vocab, base) = match log_dims.as_slice() {
                    [1, s, v] => (
                        *s as usize,
                        *v as usize,
                        ((*s as usize) - 1) * (*v as usize),
                    ),
                    [s, v] => (
                        *s as usize,
                        *v as usize,
                        ((*s as usize) - 1) * (*v as usize),
                    ),
                    other => return Err(anyhow!("Unexpected logits shape: {:?}", other)),
                };
                if seq_len == 0 || vocab == 0 {
                    return Err(anyhow!("Bad logits shape: {:?}", log_dims));
                }
                let last_row = &log_data[base..base + vocab];
                let mut best_i = 0usize;
                let mut best_v = f32::NEG_INFINITY;
                for (i, &v) in last_row.iter().enumerate() {
                    if v > best_v {
                        best_v = v;
                        best_i = i;
                    }
                }
                let next = best_i as u32;
                if next == eot {
                    break;
                }
                retry_tokens.push(next);
                retry_generated.push(next);
                if !is_timestamp_token(next, ts_begin_token, ts_end_token) {
                    let decoded = tokenizer
                        .decode(&[next], true)
                        .map_err(|e| anyhow!("{e}"))?;
                    retry_text.push_str(&decoded);
                    print!("{decoded}");
                    std::io::stdout().flush()?;
                }
            }
            chunk_text = retry_text;
            generated_tokens = retry_generated;
        }

        // Remove overlap-induced duplicate prefix to avoid very long repeated subtitle cues.
        if !last_chunk_text.trim().is_empty() && !chunk_text.trim().is_empty() {
            chunk_text = trim_repeated_chunk_prefix(&last_chunk_text, &chunk_text);
        }
        if !chunk_text.trim().is_empty() {
            last_chunk_text = chunk_text.clone();
        }

        let start_sec = (chunk_start * model_frontend().hop_length) as f32
            / model_frontend().sample_rate as f32;
        let end_frame = chunk_end_frame.min(total_frames);
        let end_sec =
            (end_frame * model_frontend().hop_length) as f32 / model_frontend().sample_rate as f32;
        println!("\n[chunk {chunk_idx}] decoded chars: {}", chunk_text.len());
        let chunk_duration = (end_sec - start_sec).max(0.0);
        let ts_segments = extract_timestamp_segments(
            &generated_tokens,
            &tokenizer,
            ts_begin_token,
            ts_end_token,
            chunk_duration,
        );
        if !ts_segments.is_empty() {
            // Use model timestamp tokens first for higher timing fidelity.
            for seg in ts_segments {
                let abs_start = (start_sec + seg.start_sec).clamp(start_sec, end_sec);
                let abs_end = (start_sec + seg.end_sec).clamp(abs_start, end_sec);
                if abs_end <= abs_start || seg.text.trim().is_empty() {
                    continue;
                }
                // Apply readability split on timestamped segments to avoid overly long one-line cues.
                let base_seg = TimestampedSegment {
                    start_sec: abs_start,
                    end_sec: abs_end,
                    text: seg.text,
                };
                let refined = split_timestamp_segment_for_readability(&base_seg);
                for piece in refined {
                    // Enforce global non-overlap so imported SRT stays on a single subtitle lane.
                    let mut piece_start = piece.start_sec;
                    let mut piece_end = piece.end_sec;
                    if let Some(prev) = &last_emitted_subtitle {
                        if piece_start < prev.end_sec {
                            // Preserve cue duration when resolving overlap; avoid compressing cues into dense bursts.
                            let target_start = prev.end_sec + TIMESTAMP_RESOLUTION_SEC;
                            let shift = target_start - piece_start;
                            piece_start += shift;
                            piece_end += shift;
                        }
                    }
                    if piece_end <= piece_start {
                        continue;
                    }

                    let norm_text = normalize_subtitle_text_for_match(&piece.text);
                    if let Some(prev) = &last_emitted_subtitle {
                        if is_near_duplicate_subtitle(prev, piece_start, &norm_text) {
                            // Skip repeated cue emitted by overlap/timestamp jitter.
                            continue;
                        }
                    }
                    let idx = srt_count(&srt) + 1;
                    let start_ts = format_srt_ts(piece_start);
                    let end_ts = format_srt_ts(piece_end);
                    srt.push_str(&format!(
                        "{idx}\n{start_ts} --> {end_ts}\n{}\n\n",
                        piece.text
                    ));
                    transcript.push_str(piece.text.trim());
                    transcript.push('\n');
                    last_emitted_subtitle = Some(EmittedSubtitleState {
                        end_sec: piece_end,
                        norm_text,
                    });
                }
            }
        } else if !chunk_text.trim().is_empty() {
            // Split long decoded text into readable subtitle-sized chunks.
            let mut segments = split_sentences(&chunk_text);
            let min_segments_by_time =
                ((chunk_duration / subtitle_tuning().max_subtitle_duration_sec).ceil() as usize)
                    .max(1);
            if segments.len() < min_segments_by_time {
                segments = rebalance_segments_for_duration(&chunk_text, min_segments_by_time);
            }
            // Hard-limit estimated cue duration so no single cue stretches too long on screen.
            segments = enforce_segment_duration_limit(segments, chunk_duration);
            let total_chars: usize = segments.iter().map(|s| s.len().max(1)).sum();
            let mut seg_start = start_sec;
            for seg in segments {
                let seg_chars = seg.len().max(1);
                let frac = seg_chars as f32 / total_chars as f32;
                let seg_dur = (end_sec - start_sec) * frac;
                let seg_end = (seg_start + seg_dur).min(end_sec);
                let mut out_start = seg_start;
                let mut out_end = seg_end;
                let idx = srt_count(&srt) + 1;
                let seg_text = seg.trim().to_string();
                let norm_text = normalize_subtitle_text_for_match(&seg_text);
                if let Some(prev) = &last_emitted_subtitle {
                    if out_start < prev.end_sec {
                        // Preserve cue duration when resolving overlap; avoid compressing cues into dense bursts.
                        let target_start = prev.end_sec + TIMESTAMP_RESOLUTION_SEC;
                        let shift = target_start - out_start;
                        out_start += shift;
                        out_end += shift;
                    }
                    if out_end <= out_start {
                        seg_start = seg_end;
                        continue;
                    }
                    if is_near_duplicate_subtitle(prev, out_start, &norm_text) {
                        seg_start = seg_end;
                        continue;
                    }
                }
                let start_ts = format_srt_ts(out_start);
                let end_ts = format_srt_ts(out_end.max(out_start));
                srt.push_str(&format!("{idx}\n{start_ts} --> {end_ts}\n{seg_text}\n\n"));
                transcript.push_str(&seg_text);
                transcript.push('\n');
                last_emitted_subtitle = Some(EmittedSubtitleState {
                    end_sec: out_end,
                    norm_text,
                });
                seg_start = seg_end;
            }
        }

        if USE_PROMPT_TAIL_WITH_TIMESTAMPS && tokens.len() > prompt_prefix_len {
            // Keep only text tokens in prompt tail to avoid carrying stale timestamp tokens.
            let mut tail: Vec<u32> = tokens[prompt_prefix_len..]
                .iter()
                .copied()
                .filter(|t| !is_timestamp_token(*t, ts_begin_token, ts_end_token))
                .collect();
            if tail.len() > MAX_PROMPT_TOKENS {
                let start = tail.len().saturating_sub(MAX_PROMPT_TOKENS);
                tail = tail[start..].to_vec();
            }
            prompt_tail = tail;
        } else {
            prompt_tail.clear();
        }
    }

    println!("\n✅ Done.");
    if let Some(parent) = cfg.srt_out_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(parent) = cfg.txt_out_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&cfg.srt_out_path, &srt)?;
    fs::write(&cfg.txt_out_path, &transcript)?;
    println!("Saved SRT: {}", cfg.srt_out_path.display());
    println!("Saved TXT: {}", cfg.txt_out_path.display());
    Ok(())
}

// =========================
// WAV helpers
// =========================
fn load_wav_mono(path: &Path) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();

    if spec.channels != 1 || spec.sample_rate as usize != model_frontend().sample_rate {
        return Err(anyhow!(
            "WAV must be {}Hz mono. Got {:?}.",
            model_frontend().sample_rate,
            spec
        ));
    }

    let samples: Vec<f32> = reader
        .samples::<i16>()
        .filter_map(|s| s.ok())
        .map(|s| s as f32 / 32768.0)
        .collect();

    Ok(samples)
}

// Build model input features using manifest-selected frontend implementation.
fn build_model_features(samples: &[f32]) -> Result<(Vec<f32>, usize)> {
    match model_frontend().frontend {
        FrontendKind::WhisperLogMel => pcm_to_log_mel_all_frames(samples),
    }
}

// =========================
// Mel helpers (frontend parameters come from model manifest at runtime)
// =========================
fn pcm_to_log_mel_all_frames(samples: &[f32]) -> Result<(Vec<f32>, usize)> {
    let n_frames = (samples.len() + model_frontend().hop_length - 1) / model_frontend().hop_length;
    let mel = pcm_to_log_mel_fixed_frames(samples, n_frames)?;
    Ok((mel, n_frames))
}

fn pcm_to_log_mel_fixed_frames(samples: &[f32], n_frames: usize) -> Result<Vec<f32>> {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(model_frontend().n_fft);

    let mut mel_output = vec![0.0f32; model_frontend().n_mels * n_frames];

    let window: Vec<f32> = (0..model_frontend().n_fft)
        .map(|i| {
            0.5 * (1.0
                - (2.0 * std::f32::consts::PI * i as f32 / model_frontend().n_fft as f32).cos())
        })
        .collect();

    let filters = create_mel_filters(
        model_frontend().sample_rate,
        model_frontend().n_fft,
        model_frontend().n_mels,
    );

    for i in 0..n_frames {
        let start = i * model_frontend().hop_length;

        // Pad tail frames with zeros when input samples are shorter than one FFT window.
        let mut buffer: Vec<Complex32> = (0..model_frontend().n_fft)
            .map(|j| {
                let idx = start + j;
                let s = if idx < samples.len() {
                    samples[idx]
                } else {
                    0.0
                };
                Complex32::new(s * window[j], 0.0)
            })
            .collect();

        fft.process(&mut buffer);

        let power_spec: Vec<f32> = buffer[0..model_frontend().n_fft / 2 + 1]
            .iter()
            .map(|c| c.norm_sqr())
            .collect();

        for m in 0..model_frontend().n_mels {
            let mut sum = 0.0f32;
            for (k, &w) in filters[m].iter().enumerate() {
                sum += power_spec[k] * w;
            }
            mel_output[m * n_frames + i] = (sum.max(1e-10)).log10();
        }
    }

    // dynamic range + scaling
    let max_val = mel_output.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
    let target = max_val - 8.0;
    for x in mel_output.iter_mut() {
        *x = ((*x).max(target) + 4.0) / 4.0;
    }

    Ok(mel_output)
}

fn create_mel_filters(sr: usize, n_fft: usize, n_mels: usize) -> Vec<Vec<f32>> {
    let n_freqs = n_fft / 2 + 1;

    let fft_freqs: Vec<f32> = (0..n_freqs)
        .map(|i| i as f32 * sr as f32 / n_fft as f32)
        .collect();

    let mel_min = 0.0f32;
    let mel_max = 2595.0 * (1.0 + (sr as f32 / 2.0) / 700.0).log10();

    let mel_points: Vec<f32> = (0..n_mels + 2)
        .map(|i| mel_min + (mel_max - mel_min) * i as f32 / (n_mels + 1) as f32)
        .collect();

    let hz_points: Vec<f32> = mel_points
        .iter()
        .map(|&m| 700.0 * (10f32.powf(m / 2595.0) - 1.0))
        .collect();

    let mut filters = vec![vec![0.0f32; n_freqs]; n_mels];

    for m in 0..n_mels {
        for (k, &f) in fft_freqs.iter().enumerate() {
            if f >= hz_points[m] && f <= hz_points[m + 1] {
                filters[m][k] = (f - hz_points[m]) / (hz_points[m + 1] - hz_points[m]);
            } else if f > hz_points[m + 1] && f <= hz_points[m + 2] {
                filters[m][k] = (hz_points[m + 2] - f) / (hz_points[m + 2] - hz_points[m + 1]);
            }
        }
    }
    filters
}

fn slice_mel_chunk(
    mel: &[f32],
    total_frames: usize,
    n_mels: usize,
    start: usize,
    len: usize,
) -> Vec<f32> {
    let mut out = vec![0.0f32; n_mels * len];
    for m in 0..n_mels {
        let src_base = m * total_frames;
        let dst_base = m * len;
        for i in 0..len {
            let src_i = start + i;
            if src_i < total_frames {
                out[dst_base + i] = mel[src_base + src_i];
            }
        }
    }
    out
}

fn format_srt_ts(secs: f32) -> String {
    let total_ms = (secs * 1000.0).max(0.0).round() as u64;
    let ms = total_ms % 1000;
    let total_s = total_ms / 1000;
    let s = total_s % 60;
    let total_m = total_s / 60;
    let m = total_m % 60;
    let h = total_m / 60;
    format!("{:02}:{:02}:{:02},{:03}", h, m, s, ms)
}

fn split_sentences(text: &str) -> Vec<String> {
    let max_subtitle_chars = subtitle_tuning().max_subtitle_chars;
    // First pass: split by terminal punctuation to keep natural sentence boundaries.
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        cur.push(ch);
        if ch == '.' || ch == '!' || ch == '?' || ch == '。' || ch == '！' || ch == '？' {
            let trimmed = cur.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
            cur.clear();
        }
    }
    let trimmed = cur.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }

    // Second pass: split oversized segments by soft punctuation and length limits.
    let mut normalized = Vec::new();
    for seg in out {
        for soft in split_with_soft_breaks(&seg, max_subtitle_chars) {
            normalized.extend(wrap_for_subtitle(&soft, max_subtitle_chars));
        }
    }
    normalized
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

// Split oversized text at commas/semicolons before doing hard wrapping.
fn split_with_soft_breaks(text: &str, max_chars: usize) -> Vec<String> {
    if text.chars().count() <= max_chars {
        return vec![text.trim().to_string()];
    }

    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut cur_chars = 0usize;
    for ch in text.chars() {
        cur.push(ch);
        cur_chars += 1;

        let is_soft_break = matches!(ch, ',' | '，' | ';' | '；' | ':' | '：');
        if is_soft_break && cur_chars >= max_chars / 2 {
            let t = cur.trim();
            if !t.is_empty() {
                parts.push(t.to_string());
            }
            cur.clear();
            cur_chars = 0;
        } else if cur_chars >= max_chars * 2 {
            let t = cur.trim();
            if !t.is_empty() {
                parts.push(t.to_string());
            }
            cur.clear();
            cur_chars = 0;
        }
    }
    let t = cur.trim();
    if !t.is_empty() {
        parts.push(t.to_string());
    }

    if parts.is_empty() {
        vec![text.trim().to_string()]
    } else {
        parts
    }
}

// Wrap a sentence into subtitle-friendly chunks with max character limits.
fn wrap_for_subtitle(text: &str, max_chars: usize) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.chars().count() <= max_chars {
        return vec![trimmed.to_string()];
    }

    // Prefer word wrapping for Latin text.
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if !words.is_empty() && words.len() > 1 {
        let mut out = Vec::new();
        let mut line = String::new();
        for w in words {
            let candidate = if line.is_empty() {
                w.to_string()
            } else {
                format!("{line} {w}")
            };
            if candidate.chars().count() <= max_chars {
                line = candidate;
            } else {
                if !line.is_empty() {
                    out.push(line);
                }
                line = w.to_string();
            }
        }
        if !line.trim().is_empty() {
            out.push(line);
        }
        return out;
    }

    // Fallback for text without spaces (or single very long token): hard character split.
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_chars = 0usize;
    for ch in trimmed.chars() {
        cur.push(ch);
        cur_chars += 1;
        if cur_chars >= max_chars {
            out.push(cur.trim().to_string());
            cur.clear();
            cur_chars = 0;
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

// Ensure we have enough cue pieces to avoid one cue occupying an overly long time span.
fn rebalance_segments_for_duration(text: &str, target_segments: usize) -> Vec<String> {
    let clean = text.trim();
    if clean.is_empty() || target_segments <= 1 {
        return split_sentences(clean);
    }

    let words: Vec<&str> = clean.split_whitespace().collect();
    if words.len() >= target_segments && words.len() > 1 {
        let mut out = Vec::new();
        let per = (words.len() + target_segments - 1) / target_segments;
        let mut i = 0usize;
        while i < words.len() {
            let end = (i + per).min(words.len());
            let chunk = words[i..end].join(" ");
            if !chunk.trim().is_empty() {
                out.push(chunk);
            }
            i = end;
        }
        return out;
    }

    let total_chars = clean.chars().count().max(1);
    let max_chars = ((total_chars + target_segments - 1) / target_segments).max(1);
    wrap_for_subtitle(clean, max_chars)
}

// Split segments further when their estimated on-screen time exceeds the configured cue duration.
fn enforce_segment_duration_limit(segments: Vec<String>, total_duration_sec: f32) -> Vec<String> {
    if segments.is_empty() || total_duration_sec <= 0.0 {
        return segments;
    }

    let total_chars: usize = segments.iter().map(|s| s.chars().count().max(1)).sum();
    if total_chars == 0 {
        return segments;
    }

    let mut out = Vec::new();
    let max_subtitle_duration_sec = subtitle_tuning().max_subtitle_duration_sec;
    let max_subtitle_chars = subtitle_tuning().max_subtitle_chars;
    for seg in segments {
        let seg_chars = seg.chars().count().max(1);
        let est_dur = total_duration_sec * (seg_chars as f32 / total_chars as f32);
        let needed = (est_dur / max_subtitle_duration_sec).ceil().max(1.0) as usize;
        if needed <= 1 {
            out.push(seg);
            continue;
        }

        let target_chars = ((seg_chars + needed - 1) / needed)
            .max(1)
            .min(max_subtitle_chars);
        let mut pieces = wrap_for_subtitle(&seg, target_chars);
        if pieces.len() < needed {
            pieces = rebalance_segments_for_duration(&seg, needed);
        }
        out.extend(pieces.into_iter().filter(|p| !p.trim().is_empty()));
    }

    out
}

// Trim duplicate word-prefix from current chunk when overlap/prompt makes text repeat.
fn trim_repeated_chunk_prefix(previous: &str, current: &str) -> String {
    let prev_words: Vec<&str> = previous.split_whitespace().collect();
    let cur_words: Vec<&str> = current.split_whitespace().collect();
    if prev_words.len() < 6 || cur_words.len() < 6 {
        return current.trim().to_string();
    }

    let max_overlap = prev_words.len().min(cur_words.len()).min(48);
    let mut best = 0usize;
    for k in (6..=max_overlap).rev() {
        let prev_slice = &prev_words[prev_words.len() - k..];
        let cur_slice = &cur_words[..k];
        if prev_slice
            .iter()
            .zip(cur_slice.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            best = k;
            break;
        }
    }

    if best == 0 {
        current.trim().to_string()
    } else {
        cur_words[best..].join(" ").trim().to_string()
    }
}

fn srt_count(srt: &str) -> usize {
    srt.lines()
        .filter(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_digit()))
        .count()
}

#[derive(Debug, Clone)]
struct TimestampedSegment {
    start_sec: f32,
    end_sec: f32,
    text: String,
}

// Parse Whisper timestamp tokens into timed text segments within one decode window.
fn extract_timestamp_segments(
    generated_tokens: &[u32],
    tokenizer: &Tokenizer,
    ts_begin_token: u32,
    ts_end_token: u32,
    window_duration_sec: f32,
) -> Vec<TimestampedSegment> {
    if generated_tokens.is_empty() || window_duration_sec <= 0.0 {
        return Vec::new();
    }

    let mut segments = Vec::new();
    let mut current_start: Option<f32> = None;
    let mut text_tokens: Vec<u32> = Vec::new();
    let mut last_ts_sec = 0.0f32;

    for &tok in generated_tokens {
        if is_timestamp_token(tok, ts_begin_token, ts_end_token) {
            let rel_sec = ((tok.saturating_sub(ts_begin_token)) as f32 * TIMESTAMP_RESOLUTION_SEC)
                .clamp(0.0, window_duration_sec);
            if current_start.is_none() {
                current_start = Some(rel_sec);
                last_ts_sec = rel_sec;
                continue;
            }

            if text_tokens.is_empty() {
                current_start = Some(rel_sec);
                last_ts_sec = rel_sec;
                continue;
            }

            let start = current_start.unwrap_or(0.0).clamp(0.0, window_duration_sec);
            // Guard clamp bounds to avoid panic when start is already at the window end.
            let min_end = (start + TIMESTAMP_RESOLUTION_SEC).min(window_duration_sec);
            let mut end = rel_sec.clamp(min_end, window_duration_sec);
            if end <= start {
                end = (start + TIMESTAMP_RESOLUTION_SEC).min(window_duration_sec);
            }
            let text = decode_tokens_text(tokenizer, &text_tokens);
            if !text.trim().is_empty() && end > start {
                segments.push(TimestampedSegment {
                    start_sec: start,
                    end_sec: end,
                    text,
                });
            }
            text_tokens.clear();
            current_start = Some(rel_sec);
            last_ts_sec = rel_sec;
        } else {
            text_tokens.push(tok);
        }
    }

    if !text_tokens.is_empty() {
        let start = current_start
            .unwrap_or(last_ts_sec)
            .clamp(0.0, window_duration_sec);
        let text = decode_tokens_text(tokenizer, &text_tokens);
        // Guard clamp bounds to avoid panic when start is already at the window end.
        let min_end = (start + TIMESTAMP_RESOLUTION_SEC).min(window_duration_sec);
        let mut end = (start + estimate_text_duration_sec(text.chars().count()))
            .clamp(min_end, window_duration_sec);
        if end <= start {
            end = (start + TIMESTAMP_RESOLUTION_SEC).min(window_duration_sec);
        }
        if !text.trim().is_empty() && end > start {
            segments.push(TimestampedSegment {
                start_sec: start,
                end_sec: end,
                text,
            });
        }
    }

    // Ensure monotonic non-overlapping segments after clamping.
    // Preserve each cue duration when resolving overlap so timestamps do not collapse into dense clusters.
    let mut fixed = Vec::new();
    let mut cursor = 0.0f32;
    for seg in segments {
        let source_duration = (seg.end_sec - seg.start_sec).max(TIMESTAMP_RESOLUTION_SEC);
        let start = seg.start_sec.max(cursor).min(window_duration_sec);
        let end = (start + source_duration).min(window_duration_sec);
        if end <= start || seg.text.trim().is_empty() {
            continue;
        }
        fixed.push(TimestampedSegment {
            start_sec: start,
            end_sec: end,
            text: seg.text,
        });
        cursor = end;
    }
    fixed
}

// Decode token id list to text while removing empty/pure-special artifacts.
fn decode_tokens_text(tokenizer: &Tokenizer, tokens: &[u32]) -> String {
    if tokens.is_empty() {
        return String::new();
    }
    tokenizer
        .decode(tokens, true)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

// Estimate duration for fallback text-only segment completion.
fn estimate_text_duration_sec(chars: usize) -> f32 {
    let cps = 14.0f32;
    ((chars as f32) / cps).clamp(0.20, 2.50)
}

// Check whether token id belongs to Whisper timestamp token range.
fn is_timestamp_token(token: u32, ts_begin_token: u32, ts_end_token: u32) -> bool {
    token >= ts_begin_token && token <= ts_end_token
}

// Normalize subtitle text so duplicate checks are robust across punctuation/case differences.
fn normalize_subtitle_text_for_match(text: &str) -> String {
    let lower = text.to_lowercase();
    let mut out = String::new();
    let mut last_space = false;
    for ch in lower.chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
            last_space = false;
        } else if ch.is_whitespace() && !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim().to_string()
}

// Treat almost-adjacent same text as a duplicate produced by overlap windows.
fn is_near_duplicate_subtitle(
    prev: &EmittedSubtitleState,
    start_sec: f32,
    norm_text: &str,
) -> bool {
    if norm_text.is_empty() || prev.norm_text.is_empty() {
        return false;
    }
    prev.norm_text == norm_text && start_sec <= (prev.end_sec + 0.45)
}

// Split one timestamped segment into smaller readable cues while preserving total timing span.
fn split_timestamp_segment_for_readability(seg: &TimestampedSegment) -> Vec<TimestampedSegment> {
    let start = seg.start_sec;
    let end = seg.end_sec;
    if end <= start || seg.text.trim().is_empty() {
        return Vec::new();
    }

    let total_duration = (end - start).max(TIMESTAMP_RESOLUTION_SEC);
    let max_subtitle_duration_sec = subtitle_tuning().max_subtitle_duration_sec;
    let mut pieces = split_sentences(&seg.text);
    let min_segments_by_time =
        ((total_duration / max_subtitle_duration_sec).ceil() as usize).max(1);
    if pieces.len() < min_segments_by_time {
        pieces = rebalance_segments_for_duration(&seg.text, min_segments_by_time);
    }
    pieces = enforce_segment_duration_limit(pieces, total_duration);
    if pieces.is_empty() {
        return vec![TimestampedSegment {
            start_sec: start,
            end_sec: end,
            text: seg.text.trim().to_string(),
        }];
    }

    let total_chars: usize = pieces.iter().map(|s| s.chars().count().max(1)).sum();
    if total_chars == 0 {
        return vec![TimestampedSegment {
            start_sec: start,
            end_sec: end,
            text: seg.text.trim().to_string(),
        }];
    }

    let mut out = Vec::new();
    let mut cursor = start;
    let piece_count = pieces.len();
    for (i, text) in pieces.into_iter().enumerate() {
        let clean = text.trim().to_string();
        if clean.is_empty() {
            continue;
        }
        let seg_chars = clean.chars().count().max(1);
        let frac = seg_chars as f32 / total_chars as f32;
        let mut piece_end = if i + 1 >= piece_count {
            end
        } else {
            (cursor + total_duration * frac).min(end)
        };
        if piece_end <= cursor {
            piece_end = (cursor + TIMESTAMP_RESOLUTION_SEC).min(end);
        }
        if piece_end <= cursor {
            continue;
        }
        out.push(TimestampedSegment {
            start_sec: cursor,
            end_sec: piece_end,
            text: clean,
        });
        cursor = piece_end;
    }

    if let Some(last) = out.last_mut() {
        if last.end_sec < end {
            last.end_sec = end;
        }
    }

    out
}

// Build decode windows from speech segments so subtitle timing follows voice activity.
fn build_vad_decode_windows(samples: &[f32], total_frames: usize) -> Vec<(usize, usize)> {
    let speech_segments = detect_speech_segments(samples);
    let mut windows = Vec::new();
    let max_window_frames = ((subtitle_tuning().timing_max_window_sec
        * model_frontend().sample_rate as f32)
        / model_frontend().hop_length as f32)
        .round()
        .max(32.0) as usize;
    let max_window_frames = max_window_frames.min(model_frontend().n_frames()).max(32);
    let overlap_frames = ((TIMING_WINDOW_OVERLAP_SEC * model_frontend().sample_rate as f32)
        / model_frontend().hop_length as f32)
        .round()
        .max(0.0) as usize;
    let step = max_window_frames.saturating_sub(overlap_frames).max(1);

    if speech_segments.is_empty() {
        return build_full_coverage_windows(total_frames);
    }

    for (seg_start_sample, seg_end_sample) in speech_segments {
        let mut seg_start_frame = seg_start_sample / model_frontend().hop_length;
        let seg_end_frame = ((seg_end_sample + model_frontend().hop_length - 1)
            / model_frontend().hop_length)
            .min(total_frames);
        if seg_end_frame <= seg_start_frame {
            continue;
        }

        while seg_start_frame < seg_end_frame {
            // Force smaller decode windows for better local timestamp approximation.
            let chunk_end = (seg_start_frame + max_window_frames).min(seg_end_frame);
            windows.push((seg_start_frame, chunk_end));
            if chunk_end >= seg_end_frame {
                break;
            }
            seg_start_frame += step;
        }
    }

    if windows.is_empty() {
        return build_full_coverage_windows(total_frames);
    }
    windows
}

// Build full-coverage windows as a robust fallback when VAD is unavailable/empty.
fn build_full_coverage_windows(total_frames: usize) -> Vec<(usize, usize)> {
    let step = model_frontend()
        .n_frames()
        .saturating_sub(model_frontend().overlap_frames)
        .max(1);
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < total_frames {
        let end = (start + model_frontend().n_frames()).min(total_frames);
        out.push((start, end));
        if end >= total_frames {
            break;
        }
        start += step;
    }
    out
}

// Detect coarse speech regions using frame energy, then pad+merge to stabilize boundaries.
fn detect_speech_segments(samples: &[f32]) -> Vec<(usize, usize)> {
    if samples.is_empty() {
        return Vec::new();
    }

    let frame_size = model_frontend().n_fft;
    let hop = model_frontend().hop_length;
    if samples.len() < frame_size {
        return vec![(0, samples.len())];
    }

    let mut db_frames = Vec::new();
    let mut frame_start = 0usize;
    while frame_start + frame_size <= samples.len() {
        let mut sum = 0.0f32;
        for &s in &samples[frame_start..frame_start + frame_size] {
            sum += s * s;
        }
        let rms = (sum / frame_size as f32).sqrt();
        let db = 20.0 * (rms + 1e-8).log10();
        db_frames.push(db);
        frame_start += hop;
    }

    if db_frames.is_empty() {
        return Vec::new();
    }

    let mut sorted = db_frames.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let noise_idx = ((sorted.len() as f32) * 0.2) as usize;
    let noise_floor = sorted[noise_idx.min(sorted.len() - 1)];
    let speech_threshold = (noise_floor + subtitle_tuning().vad_db_offset).clamp(-55.0, -22.0);

    let speech_flags: Vec<bool> = db_frames.iter().map(|&db| db >= speech_threshold).collect();
    let min_speech_frames = ((VAD_MIN_SEG_SEC * model_frontend().sample_rate as f32) / hop as f32)
        .round()
        .max(1.0) as usize;
    let merge_gap_frames =
        ((subtitle_tuning().vad_merge_gap_sec * model_frontend().sample_rate as f32) / hop as f32)
            .round()
            .max(1.0) as usize;

    let mut segments_frames = Vec::new();
    let mut i = 0usize;
    while i < speech_flags.len() {
        while i < speech_flags.len() && !speech_flags[i] {
            i += 1;
        }
        if i >= speech_flags.len() {
            break;
        }

        let start = i;
        let mut end = i;
        while i < speech_flags.len() {
            while i < speech_flags.len() && speech_flags[i] {
                i += 1;
            }
            end = i;

            let gap_start = i;
            while i < speech_flags.len() && !speech_flags[i] {
                i += 1;
            }
            let gap_len = i.saturating_sub(gap_start);
            let resumes = i < speech_flags.len() && speech_flags[i];
            if !resumes || gap_len > merge_gap_frames {
                break;
            }
        }

        if end.saturating_sub(start) >= min_speech_frames {
            segments_frames.push((start, end));
        }
    }

    let pad_samples = (VAD_PAD_SEC * model_frontend().sample_rate as f32)
        .round()
        .max(0.0) as usize;
    let mut segments_samples = Vec::new();
    for (start_f, end_f) in segments_frames {
        let start_sample = start_f.saturating_mul(hop).saturating_sub(pad_samples);
        let end_sample = (end_f.saturating_mul(hop) + frame_size + pad_samples).min(samples.len());
        if end_sample > start_sample {
            segments_samples.push((start_sample, end_sample));
        }
    }

    // Merge any overlaps created by padding and tiny gaps.
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in segments_samples {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 {
                last.1 = last.1.max(e);
            } else {
                merged.push((s, e));
            }
        } else {
            merged.push((s, e));
        }
    }

    merged
}

// =========================
// Debug helpers
// =========================
fn print_io(tag: &str, s: &Session) {
    println!("\n[{tag}] INPUTS:");
    for (i, inp) in s.inputs().iter().enumerate() {
        println!("  [{i}] {}", inp.name());
    }
    println!("[{tag}] OUTPUTS:");
    for (i, out) in s.outputs().iter().enumerate() {
        println!("  [{i}] {}", out.name());
    }
}
