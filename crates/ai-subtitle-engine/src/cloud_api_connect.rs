// =========================================
// =========================================
// crates/ai-subtitle-engine/src/cloud_api_connect.rs
use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose};
use reqwest::blocking::{Client, multipart};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudProvider {
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

impl CloudProvider {
    // Keep provider ids aligned with UI/CLI ids so engine routing stays consistent.
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
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
                "Unsupported cloud provider '{other}'. Supported values: openai_whisper_1, openai_whisper_1_plus_4o_merge, gpt4o_transcribe, gpt4o_transcribe_diarize, gpt4o_mini_transcribe, gpt4o_mini_tts, gemini_25_pro, gemini_25_flash, assemblyai."
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
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

#[derive(Debug, Clone)]
pub struct CloudRunRequest {
    pub provider: CloudProvider,
    pub input_audio_path: PathBuf,
    pub output_srt_path: PathBuf,
    pub output_txt_path: PathBuf,
    pub language_code: Option<String>,
    pub max_subtitle_duration_sec: f32,
    pub max_subtitle_chars: usize,
}

#[derive(Debug, Clone)]
struct CloudSegment {
    start_sec: f32,
    end_sec: f32,
    text: String,
}

#[derive(Debug, Clone)]
struct CloudResult {
    text: String,
    segments: Vec<CloudSegment>,
    srt_override: Option<String>,
    analysis_json: Option<Value>,
}

pub fn run_cloud_transcription(request: &CloudRunRequest) -> Result<String> {
    if !request.input_audio_path.exists() {
        return Err(anyhow!(
            "Input audio not found: {}",
            request.input_audio_path.display()
        ));
    }
    if let Some(parent) = request.output_srt_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create SRT output directory '{}'",
                parent.display()
            )
        })?;
    }
    if let Some(parent) = request.output_txt_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create TXT output directory '{}'",
                parent.display()
            )
        })?;
    }

    let result = match request.provider {
        // Route OpenAI engines by model id while keeping one shared API implementation.
        CloudProvider::OpenAiWhisper1 => run_openai_transcribe(request, "whisper-1", false)?,
        CloudProvider::OpenAiWhisper1Plus4oMerge => run_openai_whisper1_plus_4o_merge(request)?,
        CloudProvider::Gpt4oTranscribe => {
            run_openai_transcribe(request, "gpt-4o-transcribe", false)?
        }
        CloudProvider::Gpt4oTranscribeDiarize => {
            run_openai_transcribe(request, "gpt-4o-transcribe-diarize", true)?
        }
        CloudProvider::Gpt4oMiniTranscribe => {
            run_openai_transcribe(request, "gpt-4o-mini-transcribe", false)?
        }
        CloudProvider::Gpt4oMiniTts => {
            return Err(anyhow!(
                "gpt-4o-mini-tts is a text-to-speech model and cannot generate subtitles from audio input."
            ));
        }
        CloudProvider::Gemini25Pro => run_gemini_transcribe(request, "gemini-2.5-pro")?,
        CloudProvider::Gemini25Flash => run_gemini_transcribe(request, "gemini-2.5-flash")?,
        CloudProvider::AssemblyAi => run_assemblyai_transcribe(request)?,
    };

    fs::write(&request.output_txt_path, result.text.as_bytes()).with_context(|| {
        format!(
            "Failed to write TXT output '{}'",
            request.output_txt_path.display()
        )
    })?;

    if let Some(raw_srt) = result.srt_override {
        fs::write(&request.output_srt_path, raw_srt.as_bytes()).with_context(|| {
            format!(
                "Failed to write SRT output '{}'",
                request.output_srt_path.display()
            )
        })?;
    } else {
        let cues = normalize_segments(
            result.segments,
            request.max_subtitle_duration_sec,
            request.max_subtitle_chars,
        );
        let srt = render_srt(&cues);
        fs::write(&request.output_srt_path, srt.as_bytes()).with_context(|| {
            format!(
                "Failed to write SRT output '{}'",
                request.output_srt_path.display()
            )
        })?;
    }

    let mut json_out_path: Option<PathBuf> = None;
    if let Some(analysis_json) = result.analysis_json {
        let out_json_path = request.output_srt_path.with_extension("json");
        if let Some(parent) = out_json_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create JSON output directory '{}'",
                    parent.display()
                )
            })?;
        }
        let bytes = serde_json::to_vec_pretty(&analysis_json)
            .context("Failed to serialize JSON output.")?;
        fs::write(&out_json_path, bytes).with_context(|| {
            format!("Failed to write JSON output '{}'", out_json_path.display())
        })?;
        json_out_path = Some(out_json_path);
    }

    let mut summary = format!(
        "Cloud transcription completed with {}.\nSRT: {}\nTXT: {}",
        request.provider.as_str(),
        request.output_srt_path.display(),
        request.output_txt_path.display()
    );
    if let Some(path) = json_out_path {
        summary.push_str(&format!("\nJSON: {}", path.display()));
    }
    Ok(summary)
}

#[derive(Debug, Deserialize)]
struct OpenAiSegment {
    start: f32,
    end: f32,
    text: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiTranscriptionResponse {
    #[serde(default)]
    segments: Vec<OpenAiSegment>,
}

fn run_openai_transcribe(
    request: &CloudRunRequest,
    model_name: &str,
    enable_diarize: bool,
) -> Result<CloudResult> {
    run_openai_transcribe_with_options(request, model_name, enable_diarize, true)
}

// Run one OpenAI transcription call with configurable response-format preference.
fn run_openai_transcribe_with_options(
    request: &CloudRunRequest,
    model_name: &str,
    enable_diarize: bool,
    prefer_native_srt: bool,
) -> Result<CloudResult> {
    let api_key = std::env::var("OPENAI_API_KEY")
        .context("OPENAI_API_KEY is not set. Set it before using OpenAI cloud transcription.")?;
    let bytes = fs::read(&request.input_audio_path).with_context(|| {
        format!(
            "Failed to read input audio '{}'",
            request.input_audio_path.display()
        )
    })?;

    let file_name = request
        .input_audio_path
        .file_name()
        .and_then(|x| x.to_str())
        .unwrap_or("audio.wav")
        .to_string();
    let mime = guess_audio_mime(&request.input_audio_path);

    let client = Client::builder()
        .timeout(Duration::from_secs(1800))
        .build()?;

    // Select response formats by model capability; 4o transcribe models do not support verbose_json/srt.
    let response_formats: Vec<&str> = if model_name.eq_ignore_ascii_case("whisper-1") {
        // Whisper-1 path must preserve the richest native payload for downstream analysis.
        // Always request verbose_json and render SRT locally from timestamped segments.
        let _ = prefer_native_srt;
        vec!["verbose_json"]
    } else if enable_diarize {
        vec!["diarized_json", "json", "text"]
    } else {
        vec!["json", "text"]
    };
    let mut last_format_error: Option<String> = None;

    for response_format in response_formats {
        let mut form = multipart::Form::new()
            .text("model", model_name.to_string())
            .text("response_format", response_format.to_string())
            .part(
                "file",
                multipart::Part::bytes(bytes.clone())
                    .file_name(file_name.clone())
                    .mime_str(mime)?,
            );

        // Whisper verbose_json supports segment + word timestamp granularities.
        if model_name.eq_ignore_ascii_case("whisper-1") && response_format == "verbose_json" {
            form = form
                .text("timestamp_granularities[]", "segment")
                .text("timestamp_granularities[]", "word");
        }

        if let Some(language) = normalized_language(request.language_code.as_deref()) {
            form = form.text("language", language.to_string());
        }

        let response = client
            .post("https://api.openai.com/v1/audio/transcriptions")
            .bearer_auth(&api_key)
            .multipart(form)
            .send()?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            let is_response_format_error = status.as_u16() == 400
                && body.contains("\"param\": \"response_format\"")
                && body.contains("\"code\": \"unsupported_value\"");
            if is_response_format_error {
                last_format_error = Some(format!(
                    "response_format '{response_format}' rejected: {body}"
                ));
                continue;
            }
            return Err(anyhow!(
                "OpenAI transcription request failed ({status}) with response_format '{response_format}': {body}"
            ));
        }

        // Preserve provider-native SRT output when requested.
        if response_format == "srt" {
            let raw_srt = response.text()?;
            return Ok(CloudResult {
                text: text_from_srt(&raw_srt),
                segments: Vec::new(),
                srt_override: Some(raw_srt),
                analysis_json: None,
            });
        }

        // Handle plain text response and synthesize coarse cues when model does not return timings.
        if response_format == "text" {
            let text = response.text()?.trim().to_string();
            if text.is_empty() {
                last_format_error =
                    Some("response_format 'text' returned empty transcript".to_string());
                continue;
            }
            let segments = synthesize_segments_from_text(&text);
            return Ok(CloudResult {
                text,
                segments,
                srt_override: None,
                analysis_json: None,
            });
        }

        // Parse JSON-like responses (json / verbose_json / diarized_json) in one tolerant path.
        let payload_json: Value = response.json()?;
        let analysis_json =
            if model_name.eq_ignore_ascii_case("whisper-1") && response_format == "verbose_json" {
                Some(extract_whisper_analysis_json(&payload_json, model_name))
            } else {
                None
            };
        let payload_text = payload_json
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        let mut segments = extract_json_segments(&payload_json);

        // Backward-compatible parse for verbose_json segment schema.
        if segments.is_empty() {
            if let Ok(legacy_payload) =
                serde_json::from_value::<OpenAiTranscriptionResponse>(payload_json.clone())
            {
                segments = legacy_payload
                    .segments
                    .into_iter()
                    .filter_map(|seg| {
                        let text = seg.text.trim().to_string();
                        if text.is_empty() || seg.end <= seg.start {
                            None
                        } else {
                            Some(CloudSegment {
                                start_sec: seg.start,
                                end_sec: seg.end,
                                text,
                            })
                        }
                    })
                    .collect::<Vec<_>>();
            }
        }

        // If provider gives no timings, synthesize coarse cues so SRT generation still works.
        if segments.is_empty() {
            if payload_text.is_empty() {
                last_format_error = Some(format!(
                    "response_format '{response_format}' returned no usable text/segments"
                ));
                continue;
            }
            segments = synthesize_segments_from_text(&payload_text);
        }

        return Ok(CloudResult {
            text: payload_text,
            segments,
            srt_override: None,
            analysis_json,
        });
    }

    Err(anyhow!(
        "OpenAI transcription request failed: provider did not return usable transcript payload. Last format error: {}",
        last_format_error.unwrap_or_else(|| "unknown format error".to_string())
    ))
}

// Merge mode: use whisper-1 timestamps as anchors, replace cue text with 4o transcript text.
fn run_openai_whisper1_plus_4o_merge(request: &CloudRunRequest) -> Result<CloudResult> {
    // 1) Build stable timing anchors from whisper-1 verbose_json segments.
    let whisper_anchor = run_openai_transcribe_with_options(request, "whisper-1", false, false)?;
    if whisper_anchor.segments.is_empty() {
        return Err(anyhow!(
            "Whisper-1 did not return timestamp segments for merge mode."
        ));
    }

    // 2) Get stronger wording/text from GPT-4o Transcribe.
    let four_o_text =
        run_openai_transcribe_with_options(request, "gpt-4o-transcribe", false, false)?;
    // 3) Ask GPT-4o to fill each fixed whisper timing slot with best-matching text.
    let merged = align_text_on_anchor_with_openai_gpt4o(
        &request.language_code,
        &whisper_anchor.segments,
        &four_o_text.text,
    )
    // Keep a deterministic local fallback in case the alignment call fails.
    .unwrap_or_else(|_| {
        merge_transcript_text_on_anchor(&whisper_anchor.segments, &four_o_text.text)
    });
    let merged_text = merged
        .iter()
        .map(|x| x.text.trim())
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    Ok(CloudResult {
        text: if merged_text.is_empty() {
            four_o_text.text
        } else {
            merged_text
        },
        segments: merged,
        srt_override: None,
        analysis_json: whisper_anchor.analysis_json,
    })
}

// Align full transcript text onto existing timestamp anchors with language-aware unit splitting.
fn merge_transcript_text_on_anchor(
    anchor: &[CloudSegment],
    source_text: &str,
) -> Vec<CloudSegment> {
    if anchor.is_empty() {
        return Vec::new();
    }

    let cleaned_source = source_text.trim().replace('\n', " ");
    if cleaned_source.trim().is_empty() {
        return anchor.to_vec();
    }

    let cjk_mode = is_cjk_dominant(&cleaned_source);
    let source_units = tokenize_for_alignment(&cleaned_source, cjk_mode);
    if source_units.is_empty() {
        return anchor.to_vec();
    }

    let mut weights = anchor
        .iter()
        .map(|seg| count_alignment_units(&seg.text, cjk_mode).max(1))
        .collect::<Vec<_>>();
    if weights.iter().all(|x| *x == 0) {
        weights.fill(1);
    }

    let total_units = source_units.len();
    let total_weight = weights.iter().sum::<usize>().max(1);
    let mut target_counts = weights
        .iter()
        .map(|w| ((total_units as f64 * (*w as f64 / total_weight as f64)).floor() as usize).max(1))
        .collect::<Vec<_>>();

    let mut assigned = target_counts.iter().sum::<usize>();
    while assigned > total_units {
        if let Some(idx) = target_counts
            .iter()
            .enumerate()
            .filter(|(_, v)| **v > 1)
            .max_by_key(|(_, v)| **v)
            .map(|(idx, _)| idx)
        {
            target_counts[idx] -= 1;
            assigned -= 1;
        } else {
            break;
        }
    }
    let mut cursor = 0usize;
    while assigned < total_units {
        let idx = cursor % target_counts.len();
        target_counts[idx] += 1;
        assigned += 1;
        cursor += 1;
    }

    let mut output = Vec::with_capacity(anchor.len());
    let mut read_cursor = 0usize;
    for (idx, anchor_seg) in anchor.iter().enumerate() {
        let desired = target_counts[idx];
        let end = if idx + 1 == anchor.len() {
            total_units
        } else {
            (read_cursor + desired).min(total_units)
        };
        let slice = &source_units[read_cursor..end];
        let aligned_text = if cjk_mode {
            slice.join("")
        } else {
            slice.join(" ")
        };
        output.push(CloudSegment {
            start_sec: anchor_seg.start_sec,
            end_sec: anchor_seg.end_sec,
            text: if aligned_text.trim().is_empty() {
                anchor_seg.text.clone()
            } else {
                aligned_text
            },
        });
        read_cursor = end;
    }

    if read_cursor < total_units {
        let tail = if cjk_mode {
            source_units[read_cursor..].join("")
        } else {
            source_units[read_cursor..].join(" ")
        };
        if let Some(last) = output.last_mut() {
            if !tail.trim().is_empty() {
                if cjk_mode {
                    last.text.push_str(&tail);
                } else {
                    if !last.text.trim().is_empty() {
                        last.text.push(' ');
                    }
                    last.text.push_str(&tail);
                }
            }
        }
    }

    output
}

// Use GPT-4o to align transcript text to fixed whisper anchor windows without changing timing.
fn align_text_on_anchor_with_openai_gpt4o(
    language_code: &Option<String>,
    anchor: &[CloudSegment],
    source_text: &str,
) -> Result<Vec<CloudSegment>> {
    if anchor.is_empty() {
        return Ok(Vec::new());
    }
    if source_text.trim().is_empty() {
        return Ok(anchor.to_vec());
    }
    if anchor.len() > 400 {
        return Err(anyhow!(
            "Too many anchor segments ({}) for one GPT-4o alignment request.",
            anchor.len()
        ));
    }

    let api_key = std::env::var("OPENAI_API_KEY")
        .context("OPENAI_API_KEY is not set for GPT-4o alignment.")?;
    let language_hint = language_code.as_deref().unwrap_or("auto");

    // Send compact anchor metadata so GPT-4o can map better text onto immutable timing slots.
    let anchor_windows = anchor
        .iter()
        .enumerate()
        .map(|(idx, seg)| {
            json!({
                "index": idx,
                "start_sec": seg.start_sec,
                "end_sec": seg.end_sec,
                "anchor_text_hint": seg.text
            })
        })
        .collect::<Vec<_>>();
    let user_payload = json!({
        "language_hint": language_hint,
        "full_transcript_text": source_text,
        "anchor_windows": anchor_windows,
        "rules": [
            "Do not change timing windows.",
            "Return exactly one text line per anchor window.",
            "Keep line order equal to anchor_windows order.",
            "Do not add numbering, timestamps, or markdown.",
            "Output strict JSON object only: {\"lines\": [\"...\", \"...\"]}."
        ]
    });
    let payload = json!({
        "model": "gpt-4o",
        "temperature": 0.0,
        "messages": [
            {"role":"system","content":"You align transcript text to fixed subtitle time windows. Return JSON only."},
            {"role":"user","content": user_payload.to_string()}
        ]
    });

    let client = Client::builder()
        .timeout(Duration::from_secs(240))
        .build()?;
    let response = client
        .post("https://api.openai.com/v1/chat/completions")
        .bearer_auth(api_key)
        .json(&payload)
        .send()?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(anyhow!(
            "OpenAI GPT-4o alignment request failed ({status}): {body}"
        ));
    }

    let value: Value = response.json()?;
    let content = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|x| x.first())
        .and_then(|x| x.get("message"))
        .and_then(|x| x.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if content.is_empty() {
        return Err(anyhow!("GPT-4o alignment returned empty content."));
    }

    let cleaned = strip_markdown_fence(&content);
    let parsed_json = serde_json::from_str::<Value>(&cleaned)
        .or_else(|_| serde_json::from_str::<Value>(&content))?;
    let lines = parsed_json
        .get("lines")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|x| x.as_str().unwrap_or_default().trim().to_string())
                .collect::<Vec<_>>()
        })
        .ok_or_else(|| anyhow!("GPT-4o alignment JSON does not contain 'lines' array."))?;
    if lines.len() != anchor.len() {
        return Err(anyhow!(
            "GPT-4o alignment line count mismatch: expected {}, got {}.",
            anchor.len(),
            lines.len()
        ));
    }

    Ok(anchor
        .iter()
        .zip(lines)
        .map(|(seg, line)| CloudSegment {
            start_sec: seg.start_sec,
            end_sec: seg.end_sec,
            text: if line.is_empty() {
                seg.text.clone()
            } else {
                line
            },
        })
        .collect())
}

// Use CJK-sensitive alignment so Chinese/Japanese text is aligned by characters rather than words.
fn is_cjk_dominant(text: &str) -> bool {
    let mut cjk_count = 0usize;
    let mut visible_count = 0usize;
    for ch in text.chars() {
        if ch.is_whitespace() {
            continue;
        }
        visible_count += 1;
        if ('\u{4E00}'..='\u{9FFF}').contains(&ch)
            || ('\u{3400}'..='\u{4DBF}').contains(&ch)
            || ('\u{3040}'..='\u{30FF}').contains(&ch)
            || ('\u{AC00}'..='\u{D7AF}').contains(&ch)
        {
            cjk_count += 1;
        }
    }
    visible_count > 0 && (cjk_count as f32 / visible_count as f32) >= 0.30
}

fn tokenize_for_alignment(text: &str, cjk_mode: bool) -> Vec<String> {
    if cjk_mode {
        text.chars()
            .filter(|ch| !ch.is_whitespace())
            .map(|ch| ch.to_string())
            .collect()
    } else {
        text.split_whitespace().map(|x| x.to_string()).collect()
    }
}

fn count_alignment_units(text: &str, cjk_mode: bool) -> usize {
    tokenize_for_alignment(text, cjk_mode).len()
}

// Build coarse segments from plain text when cloud model omits timestamps.
fn synthesize_segments_from_text(text: &str) -> Vec<CloudSegment> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut cursor_sec = 0.0f32;
    for chunk in trimmed
        .split_terminator(['.', '!', '?', '\n'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let est_duration = (chunk.chars().count() as f32 / 14.0).clamp(1.0, 5.0);
        let next_sec = cursor_sec + est_duration;
        out.push(CloudSegment {
            start_sec: cursor_sec,
            end_sec: next_sec,
            text: chunk.to_string(),
        });
        cursor_sec = next_sec;
    }

    if out.is_empty() {
        out.push(CloudSegment {
            start_sec: 0.0,
            end_sec: 2.0,
            text: trimmed.to_string(),
        });
    }
    out
}

// Extract plain transcript text from SRT output for TXT export.
fn text_from_srt(raw_srt: &str) -> String {
    raw_srt
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.chars().all(|c| c.is_ascii_digit()))
        .filter(|line| !line.contains("-->"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn run_gemini_transcribe(request: &CloudRunRequest, model_name: &str) -> Result<CloudResult> {
    let api_key = std::env::var("GEMINI_API_KEY")
        .context("GEMINI_API_KEY is not set. Set it before using Gemini backends.")?;
    let bytes = fs::read(&request.input_audio_path).with_context(|| {
        format!(
            "Failed to read input audio '{}'",
            request.input_audio_path.display()
        )
    })?;
    let mime = guess_audio_mime(&request.input_audio_path);
    let language_hint = normalized_language(request.language_code.as_deref())
        .map(str::to_string)
        .unwrap_or_else(|| "auto".to_string());

    // Ask Gemini to emit strict JSON so post-processing can stay deterministic.
    let prompt = format!(
        "Transcribe this audio. Return strict JSON only with schema: \
{{\"text\":\"full transcript\",\"segments\":[{{\"start_sec\":0.0,\"end_sec\":1.0,\"text\":\"...\"}}]}}. \
Use language hint: {language_hint}. Keep segments short and chronological."
    );
    let payload = json!({
        "generationConfig": {
            "responseMimeType": "application/json"
        },
        "contents": [{
            "role": "user",
            "parts": [
                { "text": prompt },
                {
                    "inlineData": {
                        "mimeType": mime,
                        "data": general_purpose::STANDARD.encode(bytes),
                    }
                }
            ]
        }]
    });

    let client = Client::builder()
        .timeout(Duration::from_secs(1800))
        .build()?;
    let response = client
        .post(format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{model_name}:generateContent?key={api_key}"
        ))
        .json(&payload)
        .send()?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(anyhow!("Gemini request failed ({status}): {body}"));
    }
    let value: Value = response.json()?;
    let raw = extract_gemini_text_payload(&value)
        .ok_or_else(|| anyhow!("Gemini response does not contain text payload."))?;
    let parsed_json = serde_json::from_str::<Value>(&strip_markdown_fence(&raw))
        .unwrap_or_else(|_| json!({ "text": raw, "segments": [] }));

    let text = parsed_json
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let segments = extract_json_segments(&parsed_json);

    Ok(CloudResult {
        text,
        segments,
        srt_override: None,
        analysis_json: None,
    })
}

#[derive(Debug, Deserialize)]
struct AssemblyUploadResponse {
    upload_url: String,
}

#[derive(Debug, Deserialize)]
struct AssemblyTranscriptCreateResponse {
    id: String,
}

#[derive(Debug, Deserialize)]
struct AssemblyTranscriptStatusResponse {
    status: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    error: Option<String>,
}

fn run_assemblyai_transcribe(request: &CloudRunRequest) -> Result<CloudResult> {
    let api_key = std::env::var("ASSEMBLYAI_API_KEY")
        .context("ASSEMBLYAI_API_KEY is not set. Set it before using assemblyai.")?;
    let bytes = fs::read(&request.input_audio_path).with_context(|| {
        format!(
            "Failed to read input audio '{}'",
            request.input_audio_path.display()
        )
    })?;
    let client = Client::builder()
        .timeout(Duration::from_secs(1800))
        .build()?;

    // Upload raw audio bytes to AssemblyAI and receive a temporary upload URL.
    let upload = client
        .post("https://api.assemblyai.com/v2/upload")
        .header("authorization", &api_key)
        .body(bytes)
        .send()?;
    if !upload.status().is_success() {
        let status = upload.status();
        let body = upload.text().unwrap_or_default();
        return Err(anyhow!("AssemblyAI upload failed ({status}): {body}"));
    }
    let upload_json: AssemblyUploadResponse = upload.json()?;

    let mut create_payload = json!({
        "audio_url": upload_json.upload_url,
        "speech_model": "best",
    });
    if let Some(language) = normalized_language(request.language_code.as_deref()) {
        create_payload["language_code"] = Value::String(language.to_string());
    }

    // Start transcript job and then poll until completion.
    let create = client
        .post("https://api.assemblyai.com/v2/transcript")
        .header("authorization", &api_key)
        .json(&create_payload)
        .send()?;
    if !create.status().is_success() {
        let status = create.status();
        let body = create.text().unwrap_or_default();
        return Err(anyhow!(
            "AssemblyAI transcript create failed ({status}): {body}"
        ));
    }
    let create_json: AssemblyTranscriptCreateResponse = create.json()?;
    let transcript_id = create_json.id;

    let mut attempts = 0usize;
    let status_json = loop {
        attempts += 1;
        let status_response = client
            .get(format!(
                "https://api.assemblyai.com/v2/transcript/{transcript_id}"
            ))
            .header("authorization", &api_key)
            .send()?;
        if !status_response.status().is_success() {
            let status = status_response.status();
            let body = status_response.text().unwrap_or_default();
            return Err(anyhow!(
                "AssemblyAI transcript status failed ({status}): {body}"
            ));
        }
        let status_json: AssemblyTranscriptStatusResponse = status_response.json()?;
        if status_json.status == "completed" {
            break status_json;
        }
        if status_json.status == "error" {
            let reason = status_json
                .error
                .unwrap_or_else(|| "unknown AssemblyAI error".to_string());
            return Err(anyhow!("AssemblyAI transcript failed: {reason}"));
        }
        if attempts > 900 {
            return Err(anyhow!("AssemblyAI transcript polling timed out."));
        }
        thread::sleep(Duration::from_secs(2));
    };

    // Fetch official SRT output from AssemblyAI to preserve provider-side timing details.
    let srt_response = client
        .get(format!(
            "https://api.assemblyai.com/v2/transcript/{transcript_id}/srt"
        ))
        .header("authorization", &api_key)
        .send()?;
    if !srt_response.status().is_success() {
        let status = srt_response.status();
        let body = srt_response.text().unwrap_or_default();
        return Err(anyhow!("AssemblyAI SRT fetch failed ({status}): {body}"));
    }
    let srt_text = srt_response.text()?;

    Ok(CloudResult {
        text: status_json.text,
        segments: Vec::new(),
        srt_override: Some(srt_text),
        analysis_json: None,
    })
}

fn value_to_f32(value: Option<&Value>) -> Option<f32> {
    let value = value?;
    if let Some(v) = value.as_f64() {
        return Some(v as f32);
    }
    value.as_str()?.parse::<f32>().ok()
}

fn parse_ts_pair(item: &Value) -> Option<(f32, f32)> {
    let start = value_to_f32(
        item.get("start_sec")
            .or_else(|| item.get("start"))
            .or_else(|| item.get("start_time")),
    )
    .or_else(|| value_to_f32(item.get("start_ms")).map(|x| x / 1000.0))?;
    let end = value_to_f32(
        item.get("end_sec")
            .or_else(|| item.get("end"))
            .or_else(|| item.get("end_time")),
    )
    .or_else(|| value_to_f32(item.get("end_ms")).map(|x| x / 1000.0))?;
    if end <= start {
        None
    } else {
        Some((start, end))
    }
}

fn extract_whisper_words(payload: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    let mut push_word = |item: &Value, segment_index: Option<usize>| {
        let word = item
            .get("word")
            .or_else(|| item.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if word.is_empty() {
            return;
        }
        let Some((start_sec, end_sec)) = parse_ts_pair(item) else {
            return;
        };
        let key = format!("{segment_index:?}:{start_sec:.3}:{end_sec:.3}:{word}");
        if !seen.insert(key) {
            return;
        }
        out.push(json!({
            "start_sec": start_sec,
            "end_sec": end_sec,
            "word": word,
            "probability": value_to_f32(item.get("probability")),
            "segment_index": segment_index,
        }));
    };

    if let Some(words) = payload.get("words").and_then(Value::as_array) {
        for item in words {
            push_word(item, None);
        }
    }

    if let Some(segments) = payload.get("segments").and_then(Value::as_array) {
        for (segment_index, seg) in segments.iter().enumerate() {
            if let Some(words) = seg.get("words").and_then(Value::as_array) {
                for item in words {
                    push_word(item, Some(segment_index));
                }
            }
        }
    }

    out
}

fn extract_whisper_analysis_json(payload: &Value, model_name: &str) -> Value {
    let segments = payload
        .get("segments")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    let text = item
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    let (start_sec, end_sec) = parse_ts_pair(item)?;
                    Some(json!({
                        "index": index,
                        "id": item.get("id").and_then(Value::as_i64),
                        "seek": item.get("seek").and_then(Value::as_i64),
                        "start_sec": start_sec,
                        "end_sec": end_sec,
                        "text": text,
                        "avg_logprob": value_to_f32(item.get("avg_logprob")),
                        "no_speech_prob": value_to_f32(item.get("no_speech_prob")),
                        "silence_probability": value_to_f32(item.get("no_speech_prob")),
                        "temperature": value_to_f32(item.get("temperature")),
                        "compression_ratio": value_to_f32(item.get("compression_ratio")),
                        "tokens": item.get("tokens").cloned(),
                    }))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let normalized = json!({
        "provider": "openai",
        "model": model_name,
        "text": payload.get("text").and_then(Value::as_str).unwrap_or_default(),
        "language": payload.get("language").and_then(Value::as_str),
        "duration_sec": value_to_f32(payload.get("duration")),
        "segments": segments,
        "words": extract_whisper_words(payload),
    });

    // Keep existing normalized shape for compatibility and also store full provider-native payload.
    // This guarantees downstream AI analysis can access every field returned by whisper-1.
    json!({
        "provider": "openai",
        "model": model_name,
        "response_format": "verbose_json",
        "native": payload,
        "normalized": normalized,
        "text": normalized.get("text").cloned().unwrap_or(Value::String(String::new())),
        "language": normalized.get("language").cloned().unwrap_or(Value::Null),
        "duration_sec": normalized.get("duration_sec").cloned().unwrap_or(Value::Null),
        "segments": normalized.get("segments").cloned().unwrap_or(Value::Array(Vec::new())),
        "words": normalized.get("words").cloned().unwrap_or(Value::Array(Vec::new())),
    })
}

fn normalized_language(raw: Option<&str>) -> Option<&str> {
    let value = raw?.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("auto") {
        None
    } else {
        Some(value)
    }
}

fn guess_audio_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|x| x.to_str())
        .map(|x| x.to_ascii_lowercase())
        .as_deref()
    {
        Some("wav") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        Some("m4a") => "audio/mp4",
        Some("aac") => "audio/aac",
        Some("flac") => "audio/flac",
        Some("ogg") => "audio/ogg",
        _ => "application/octet-stream",
    }
}

fn extract_gemini_text_payload(value: &Value) -> Option<String> {
    value
        .get("candidates")?
        .as_array()?
        .first()?
        .get("content")?
        .get("parts")?
        .as_array()?
        .iter()
        .find_map(|part| part.get("text").and_then(Value::as_str))
        .map(|x| x.to_string())
}

fn strip_markdown_fence(raw: &str) -> String {
    let mut text = raw.trim();
    if text.starts_with("```") {
        text = text.trim_start_matches('`').trim();
        if let Some(idx) = text.find('\n') {
            text = &text[idx + 1..];
        }
        if let Some(end) = text.rfind("```") {
            text = &text[..end];
        }
    }
    text.trim().to_string()
}

fn extract_json_segments(value: &Value) -> Vec<CloudSegment> {
    let mut out = Vec::new();
    let Some(items) = value.get("segments").and_then(Value::as_array) else {
        return out;
    };
    for item in items {
        let text = item
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if text.is_empty() {
            continue;
        }

        let start = value_to_f32(
            item.get("start_sec")
                .or_else(|| item.get("start"))
                .or_else(|| item.get("start_time")),
        )
        .or_else(|| value_to_f32(item.get("start_ms")).map(|x| x / 1000.0))
        .unwrap_or(0.0);
        let end = value_to_f32(
            item.get("end_sec")
                .or_else(|| item.get("end"))
                .or_else(|| item.get("end_time")),
        )
        .or_else(|| value_to_f32(item.get("end_ms")).map(|x| x / 1000.0))
        .unwrap_or(start + 1.0);
        if end <= start {
            continue;
        }
        out.push(CloudSegment {
            start_sec: start,
            end_sec: end,
            text,
        });
    }
    out
}

fn normalize_segments(
    mut segments: Vec<CloudSegment>,
    max_duration_sec: f32,
    max_chars: usize,
) -> Vec<CloudSegment> {
    // Keep provider output in chronological order before applying local readability limits.
    segments.sort_by(|a, b| {
        a.start_sec
            .partial_cmp(&b.start_sec)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut normalized = Vec::new();
    let mut previous_end = 0.0f32;
    for seg in segments {
        let start = seg.start_sec.max(previous_end);
        let end = seg.end_sec.max(start + 0.05);
        let pieces = split_segment_for_limits(start, end, &seg.text, max_duration_sec, max_chars);
        for piece in pieces {
            previous_end = piece.end_sec;
            normalized.push(piece);
        }
    }
    normalized
}

fn split_segment_for_limits(
    start_sec: f32,
    end_sec: f32,
    text: &str,
    max_duration_sec: f32,
    max_chars: usize,
) -> Vec<CloudSegment> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }

    // Split by chars first, then enforce a duration cap by increasing piece count if needed.
    let mut pieces = Vec::new();
    let mut current = String::new();
    for word in words {
        let candidate = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        if candidate.chars().count() > max_chars.max(8) && !current.is_empty() {
            pieces.push(current);
            current = word.to_string();
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        pieces.push(current);
    }

    let duration = (end_sec - start_sec).max(0.05);
    let min_piece_count = (duration / max_duration_sec.max(0.5)).ceil() as usize;
    let target_piece_count = min_piece_count.max(pieces.len()).max(1);
    if pieces.len() < target_piece_count {
        pieces = rebalance_text_chunks(text, target_piece_count, max_chars.max(8));
    }

    let step = duration / pieces.len().max(1) as f32;
    pieces
        .into_iter()
        .enumerate()
        .map(|(idx, piece)| {
            let start = start_sec + step * idx as f32;
            let end = if idx + 1 == target_piece_count {
                end_sec
            } else {
                (start + step).min(end_sec)
            };
            CloudSegment {
                start_sec: start,
                end_sec: end.max(start + 0.05),
                text: piece,
            }
        })
        .collect()
}

fn rebalance_text_chunks(text: &str, parts: usize, max_chars: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() || parts == 0 {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut cursor = 0usize;
    for idx in 0..parts {
        let remaining_parts = parts - idx;
        let remaining_words = words.len().saturating_sub(cursor);
        let take = (remaining_words as f32 / remaining_parts as f32).ceil() as usize;
        let end = (cursor + take).min(words.len());
        let mut chunk = words[cursor..end].join(" ");
        if chunk.chars().count() > max_chars {
            chunk = chunk.chars().take(max_chars).collect::<String>();
        }
        if !chunk.trim().is_empty() {
            chunks.push(chunk);
        }
        cursor = end;
        if cursor >= words.len() {
            break;
        }
    }
    chunks
}

fn render_srt(segments: &[CloudSegment]) -> String {
    let mut out = String::new();
    for (idx, seg) in segments.iter().enumerate() {
        let line = seg.text.trim();
        if line.is_empty() {
            continue;
        }
        out.push_str(&(idx + 1).to_string());
        out.push('\n');
        out.push_str(&format!(
            "{} --> {}\n",
            format_srt_ts(seg.start_sec),
            format_srt_ts(seg.end_sec)
        ));
        out.push_str(line);
        out.push_str("\n\n");
    }
    out
}

fn format_srt_ts(seconds: f32) -> String {
    let safe = seconds.max(0.0);
    let total_ms = (safe * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let total_s = total_ms / 1000;
    let s = total_s % 60;
    let total_m = total_s / 60;
    let m = total_m % 60;
    let h = total_m / 60;
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
}
