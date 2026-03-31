// =========================================
// =========================================
// crates/media_gen_protocol/src/provider/google_genai/mod.rs

use crate::error::{ErrorCode, ProtocolError, Result};
use crate::gateway::GatewayContext;
use crate::model::ModelRouteKey;
use crate::output::{ProviderRawOutput, normalize_provider_output};
use crate::protocol::{
    AssetKind, GenerateRequest, GenerateResult, InputAssetKind, JobStatus, OutputAsset,
};
use crate::provider::{ProviderAdapter, ProviderPollResult, ProviderSubmitResult};
use async_trait::async_trait;
use base64::Engine as _;
use serde_json::{Map, Value, json};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

mod models;

const GOOGLE_GENAI_BASE_ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta";
const GOOGLE_GENAI_MODELS_ENDPOINT: &str =
    "https://generativelanguage.googleapis.com/v1beta/models";
const GOOGLE_DEFAULT_IMAGE_CONTENT_TYPE: &str = "image/png";
const GOOGLE_DEFAULT_VIDEO_CONTENT_TYPE: &str = "video/mp4";
const GOOGLE_VEO_MIN_SECONDS: u64 = 5;
const GOOGLE_VEO_MAX_SECONDS: u64 = 8;
const GOOGLE_VEO_HIGH_RES_REQUIRED_SECONDS: u64 = 8;

#[derive(Debug, Clone, Copy, Default)]
pub struct GoogleGenAiAdapter;

#[async_trait]
impl ProviderAdapter for GoogleGenAiAdapter {
    fn provider(&self) -> &'static str {
        "google"
    }

    async fn submit(
        &self,
        ctx: &GatewayContext,
        request: &GenerateRequest,
    ) -> Result<ProviderSubmitResult> {
        let route = request.route_key()?;
        if route.provider != self.provider() {
            return Err(ProtocolError::new(
                ErrorCode::InvalidArgument,
                format!(
                    "google adapter received non-google route key: {}",
                    request.model
                ),
            ));
        }
        let model = models::require_supported_model(&route)?;
        models::require_model_supports_asset_kind(&route, request.asset_kind)?;
        if request.asset_kind == AssetKind::Video {
            require_model_supports_asset_kind(&route, AssetKind::Video)?;
            return self.submit_video(ctx, request, &route, model).await;
        }
        require_model_supports_asset_kind(&route, AssetKind::Image)?;

        let api_key = resolve_api_key(ctx, request)?;
        let client = reqwest::Client::builder()
            .user_agent("media-gen-protocol/0.1")
            .build()
            .map_err(|err| {
                ProtocolError::new(
                    ErrorCode::ProviderUnavailable,
                    format!("failed to build reqwest client: {err}"),
                )
                .retriable(true)
                .with_provider(self.provider())
            })?;

        let response_payload =
            submit_google_image_generation(&client, &api_key, model, &route, request).await?;
        let hints = GoogleOutputHints::from_request(request);
        let mut outputs = Vec::new();
        let raw_outputs = extract_google_raw_outputs(&response_payload, &hints)?;
        for raw in raw_outputs {
            let normalized = normalize_provider_output(raw, ctx.output_uploader.as_ref()).await?;
            outputs.push(normalized);
        }

        Ok(ProviderSubmitResult {
            status: JobStatus::Succeeded,
            provider_job_id: None,
            result: Some(GenerateResult {
                outputs,
                usage: None,
            }),
            error: None,
        })
    }

    async fn poll(
        &self,
        ctx: &GatewayContext,
        model: &str,
        provider_job_id: &str,
    ) -> Result<ProviderPollResult> {
        let route = ModelRouteKey::parse(model)?;
        if route.provider != self.provider() {
            return Err(ProtocolError::new(
                ErrorCode::InvalidArgument,
                format!("google adapter poll received non-google route key: {model}"),
            ));
        }
        let model = models::require_supported_model(&route)?;
        if model.asset_kind == AssetKind::Image {
            return Ok(ProviderPollResult {
                status: JobStatus::Failed,
                result: None,
                error: Some(
                    ProtocolError::new(
                        ErrorCode::InvalidArgument,
                        "google image generation is sync in this adapter; no poll step",
                    )
                    .with_provider(self.provider()),
                ),
            });
        }

        self.poll_video(ctx, &route, provider_job_id).await
    }

    async fn cancel(&self, ctx: &GatewayContext, model: &str, provider_job_id: &str) -> Result<()> {
        let route = ModelRouteKey::parse(model)?;
        if route.provider != self.provider() {
            return Err(ProtocolError::new(
                ErrorCode::InvalidArgument,
                format!("google adapter cancel received non-google route key: {model}"),
            ));
        }
        let model = models::require_supported_model(&route)?;
        if model.asset_kind == AssetKind::Image {
            return Err(ProtocolError::new(
                ErrorCode::InvalidArgument,
                "google image generation is sync in this adapter; cancel not supported",
            )
            .with_provider(self.provider()));
        }
        self.cancel_video(ctx, &route, provider_job_id).await
    }
}

impl GoogleGenAiAdapter {
    async fn submit_video(
        &self,
        ctx: &GatewayContext,
        request: &GenerateRequest,
        route: &ModelRouteKey,
        model: models::GoogleGenAiModelSpec,
    ) -> Result<ProviderSubmitResult> {
        let api_key = resolve_api_key(ctx, request)?;
        let client = reqwest::Client::builder()
            .user_agent("media-gen-protocol/0.1")
            .build()
            .map_err(|err| {
                ProtocolError::new(
                    ErrorCode::ProviderUnavailable,
                    format!("failed to build reqwest client: {err}"),
                )
                .retriable(true)
                .with_provider(self.provider())
            })?;

        let response_payload =
            submit_google_video_generation(&client, &api_key, model, route, request).await?;
        let provider_job_id = response_payload
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(ToString::to_string)
            .ok_or_else(|| {
                ProtocolError::new(ErrorCode::JobFailed, "google video response missing `name`")
                    .with_provider(self.provider())
            })?;

        let status = parse_google_operation_status(&response_payload).unwrap_or(JobStatus::Queued);
        if status == JobStatus::Failed {
            let error = parse_google_operation_error(&response_payload).unwrap_or_else(|| {
                ProtocolError::new(
                    ErrorCode::JobFailed,
                    format!("google video job '{provider_job_id}' failed"),
                )
                .with_provider(self.provider())
            });
            return Ok(ProviderSubmitResult {
                status: JobStatus::Failed,
                provider_job_id: Some(provider_job_id),
                result: None,
                error: Some(error),
            });
        }

        if status == JobStatus::Canceled {
            let canceled_message = format!("google video job '{provider_job_id}' was canceled");
            return Ok(ProviderSubmitResult {
                status: JobStatus::Canceled,
                provider_job_id: Some(provider_job_id),
                result: None,
                error: Some(
                    ProtocolError::new(ErrorCode::JobFailed, canceled_message)
                        .with_provider(self.provider()),
                ),
            });
        }

        let submit_status = if status == JobStatus::Succeeded {
            JobStatus::Running
        } else {
            status
        };

        Ok(ProviderSubmitResult {
            status: submit_status,
            provider_job_id: Some(provider_job_id),
            result: None,
            error: None,
        })
    }

    async fn poll_video(
        &self,
        ctx: &GatewayContext,
        route: &ModelRouteKey,
        provider_job_id: &str,
    ) -> Result<ProviderPollResult> {
        let api_key = resolve_api_key_for_route(ctx, route)?;
        let client = reqwest::Client::builder()
            .user_agent("media-gen-protocol/0.1")
            .build()
            .map_err(|err| {
                ProtocolError::new(
                    ErrorCode::ProviderUnavailable,
                    format!("failed to build reqwest client: {err}"),
                )
                .retriable(true)
                .with_provider(self.provider())
            })?;

        let payload = retrieve_google_operation(&client, &api_key, provider_job_id).await?;
        let status = parse_google_operation_status(&payload).unwrap_or(JobStatus::Running);
        if matches!(status, JobStatus::Queued | JobStatus::Running) {
            return Ok(ProviderPollResult {
                status,
                result: None,
                error: None,
            });
        }

        if status == JobStatus::Failed {
            let error = parse_google_operation_error(&payload).unwrap_or_else(|| {
                ProtocolError::new(
                    ErrorCode::JobFailed,
                    format!("google video job '{provider_job_id}' failed"),
                )
                .with_provider(self.provider())
            });
            return Ok(ProviderPollResult {
                status: JobStatus::Failed,
                result: None,
                error: Some(error),
            });
        }

        if status == JobStatus::Canceled {
            return Ok(ProviderPollResult {
                status: JobStatus::Canceled,
                result: None,
                error: Some(
                    ProtocolError::new(
                        ErrorCode::JobFailed,
                        format!("google video job '{provider_job_id}' was canceled"),
                    )
                    .with_provider(self.provider()),
                ),
            });
        }

        let Some(video_uri) = extract_google_video_uri(&payload) else {
            return Ok(ProviderPollResult {
                status: JobStatus::Failed,
                result: None,
                error: Some(
                    ProtocolError::new(
                        ErrorCode::JobFailed,
                        format!(
                            "google video job '{provider_job_id}' succeeded but response is missing video URI"
                        ),
                    )
                    .with_provider(self.provider())
                    .with_details(payload),
                ),
            });
        };

        let (bytes, content_type) =
            download_google_video_content(&client, &api_key, &video_uri).await?;
        let uploaded_url = ctx
            .output_uploader
            .upload_bytes(content_type.as_str(), &bytes)
            .await?;
        let (width, height) = parse_google_video_dimensions(&payload);
        let duration_ms = parse_google_video_duration_ms(&payload);

        Ok(ProviderPollResult {
            status: JobStatus::Succeeded,
            result: Some(GenerateResult {
                outputs: vec![OutputAsset {
                    url: uploaded_url,
                    content_type,
                    width,
                    height,
                    duration_ms,
                    sha256: None,
                    bytes: Some(bytes.len() as u64),
                    expires_at_ms: None,
                }],
                usage: None,
            }),
            error: None,
        })
    }

    async fn cancel_video(
        &self,
        ctx: &GatewayContext,
        route: &ModelRouteKey,
        provider_job_id: &str,
    ) -> Result<()> {
        let api_key = resolve_api_key_for_route(ctx, route)?;
        let client = reqwest::Client::builder()
            .user_agent("media-gen-protocol/0.1")
            .build()
            .map_err(|err| {
                ProtocolError::new(
                    ErrorCode::ProviderUnavailable,
                    format!("failed to build reqwest client: {err}"),
                )
                .retriable(true)
                .with_provider(self.provider())
            })?;

        cancel_google_operation(&client, &api_key, provider_job_id).await
    }
}

#[derive(Debug, Clone)]
struct GoogleOutputHints {
    size: Option<(u32, u32)>,
}

impl GoogleOutputHints {
    fn from_request(request: &GenerateRequest) -> Self {
        let size_text = request
            .provider_options
            .get("size")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .unwrap_or_default();
        Self {
            size: parse_size_from_str(size_text.as_str()),
        }
    }
}

fn require_model_supports_asset_kind(route: &ModelRouteKey, kind: AssetKind) -> Result<()> {
    models::require_model_supports_asset_kind(route, kind).map(|_| ())
}

async fn submit_google_image_generation(
    client: &reqwest::Client,
    api_key: &str,
    model: models::GoogleGenAiModelSpec,
    route: &ModelRouteKey,
    request: &GenerateRequest,
) -> Result<Value> {
    let payload = build_google_generate_content_payload(client, route, request).await?;
    let response = client
        .post(format!(
            "{GOOGLE_GENAI_MODELS_ENDPOINT}/{}:generateContent",
            model.api_model_name
        ))
        .header("x-goog-api-key", api_key.trim())
        .json(&payload)
        .send()
        .await
        .map_err(map_reqwest_transport_error)?;
    parse_google_json_response(response, "image request").await
}

async fn submit_google_video_generation(
    client: &reqwest::Client,
    api_key: &str,
    model: models::GoogleGenAiModelSpec,
    route: &ModelRouteKey,
    request: &GenerateRequest,
) -> Result<Value> {
    let payload = build_google_video_predict_payload(route, request)?;
    let response = client
        .post(format!(
            "{GOOGLE_GENAI_MODELS_ENDPOINT}/{}:predictLongRunning",
            model.api_model_name
        ))
        .header("x-goog-api-key", api_key.trim())
        .json(&payload)
        .send()
        .await
        .map_err(map_reqwest_transport_error)?;
    parse_google_json_response(response, "video request").await
}

async fn retrieve_google_operation(
    client: &reqwest::Client,
    api_key: &str,
    provider_job_id: &str,
) -> Result<Value> {
    let operation_url = google_operation_url(provider_job_id);
    let response = client
        .get(operation_url)
        .header("x-goog-api-key", api_key.trim())
        .send()
        .await
        .map_err(map_reqwest_transport_error)?;
    parse_google_json_response(response, "video status request").await
}

async fn cancel_google_operation(
    client: &reqwest::Client,
    api_key: &str,
    provider_job_id: &str,
) -> Result<()> {
    let operation_url = format!("{}:cancel", google_operation_url(provider_job_id));
    let response = client
        .post(operation_url)
        .header("x-goog-api-key", api_key.trim())
        .json(&json!({}))
        .send()
        .await
        .map_err(map_reqwest_transport_error)?;

    if response.status().is_success() {
        return Ok(());
    }

    let status = response.status().as_u16();
    let body = response.text().await.map_err(|err| {
        ProtocolError::new(
            ErrorCode::ProviderUnavailable,
            format!("failed reading google video cancel response body: {err}"),
        )
        .retriable(true)
        .with_provider("google")
    })?;
    Err(map_google_http_error_with_operation(
        status,
        body.as_str(),
        "video cancel request",
    ))
}

async fn download_google_video_content(
    client: &reqwest::Client,
    api_key: &str,
    uri: &str,
) -> Result<(Vec<u8>, String)> {
    let response = client
        .get(uri)
        .header("x-goog-api-key", api_key.trim())
        .send()
        .await
        .map_err(map_reqwest_transport_error)?;
    parse_google_binary_response(response, "video content request").await
}

fn google_operation_url(provider_job_id: &str) -> String {
    format!(
        "{GOOGLE_GENAI_BASE_ENDPOINT}/{}",
        normalize_google_operation_name(provider_job_id)
    )
}

fn normalize_google_operation_name(provider_job_id: &str) -> String {
    let mut normalized = provider_job_id.trim().trim_start_matches('/').to_string();
    if let Some(rest) = normalized.strip_prefix(GOOGLE_GENAI_BASE_ENDPOINT) {
        normalized = rest.trim_start_matches('/').to_string();
    }
    if let Some(rest) = normalized.strip_prefix("v1beta/") {
        normalized = rest.to_string();
    }
    normalized
}

async fn parse_google_json_response(response: reqwest::Response, operation: &str) -> Result<Value> {
    let status = response.status();
    let body_text = response.text().await.map_err(|err| {
        ProtocolError::new(
            ErrorCode::ProviderUnavailable,
            format!("failed reading google {operation} response body: {err}"),
        )
        .retriable(true)
        .with_provider("google")
    })?;

    if !status.is_success() {
        return Err(map_google_http_error_with_operation(
            status.as_u16(),
            body_text.as_str(),
            operation,
        ));
    }

    serde_json::from_str::<Value>(&body_text).map_err(|err| {
        ProtocolError::new(
            ErrorCode::ProviderUnavailable,
            format!("failed to parse google {operation} response json: {err}"),
        )
        .retriable(true)
        .with_provider("google")
    })
}

async fn parse_google_binary_response(
    response: reqwest::Response,
    operation: &str,
) -> Result<(Vec<u8>, String)> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(map_google_http_error_with_operation(
            status.as_u16(),
            body.as_str(),
            operation,
        ));
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
        .unwrap_or_else(|| GOOGLE_DEFAULT_VIDEO_CONTENT_TYPE.to_string());
    let bytes = response.bytes().await.map_err(|err| {
        ProtocolError::new(
            ErrorCode::ProviderUnavailable,
            format!("failed reading google {operation} bytes: {err}"),
        )
        .retriable(true)
        .with_provider("google")
    })?;

    Ok((bytes.to_vec(), content_type))
}

async fn build_google_generate_content_payload(
    client: &reqwest::Client,
    route: &ModelRouteKey,
    request: &GenerateRequest,
) -> Result<Value> {
    let mut root = request.provider_options.clone();
    let requested_size = root
        .remove("size")
        .and_then(|v| v.as_str().map(ToString::to_string));
    let requested_aspect_ratio = root
        .remove("aspect_ratio")
        .or_else(|| root.remove("aspectRatio"))
        .and_then(|v| v.as_str().map(ToString::to_string));
    let requested_image_size = root
        .remove("image_size")
        .or_else(|| root.remove("imageSize"))
        .and_then(|v| v.as_str().map(ToString::to_string));

    let mut generation_config = take_object_entry(&mut root, "generationConfig")
        .or_else(|| take_object_entry(&mut root, "generation_config"))
        .unwrap_or_default();
    let mut image_config =
        take_object_entry(&mut generation_config, "imageConfig").unwrap_or_default();

    let normalized_aspect_ratio = requested_aspect_ratio
        .as_deref()
        .and_then(normalize_google_aspect_ratio)
        .or_else(|| {
            requested_size
                .as_deref()
                .and_then(aspect_ratio_from_size_text)
                .and_then(|ratio| normalize_google_aspect_ratio(ratio.as_str()))
        })
        .or_else(|| {
            request
                .aspect_ratio
                .as_deref()
                .and_then(normalize_google_aspect_ratio)
        });

    if let Some(aspect_ratio) = normalized_aspect_ratio {
        image_config.insert("aspectRatio".to_string(), Value::String(aspect_ratio));
    }

    if let Some(image_size) = requested_image_size
        && !image_size.trim().is_empty()
    {
        image_config.insert(
            "imageSize".to_string(),
            Value::String(image_size.trim().to_string()),
        );
    }

    if !image_config.is_empty() {
        generation_config.insert("imageConfig".to_string(), Value::Object(image_config));
    }
    if !generation_config.contains_key("responseModalities")
        && !generation_config.contains_key("response_modalities")
    {
        // Force image-first output so callers receive binary data in `inline_data`.
        generation_config.insert("responseModalities".to_string(), json!(["IMAGE"]));
    }
    if !generation_config.is_empty() {
        root.insert(
            "generationConfig".to_string(),
            Value::Object(generation_config),
        );
    }

    let parts = build_google_content_parts(client, route, request).await?;
    root.insert(
        "contents".to_string(),
        Value::Array(vec![json!({ "parts": parts })]),
    );

    Ok(Value::Object(root))
}

fn build_google_video_predict_payload(
    route: &ModelRouteKey,
    request: &GenerateRequest,
) -> Result<Value> {
    if !request.inputs.is_empty() {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            format!(
                "google model '{}' video inputs are not implemented in this adapter yet; submit prompt-only video request or remove inputs",
                route.model_name
            ),
        )
        .with_provider("google"));
    }

    let mut root = request.provider_options.clone();
    let mut parameters = take_object_entry(&mut root, "parameters")
        .or_else(|| take_object_entry(&mut root, "Parameters"))
        .unwrap_or_default();

    let root_duration = root
        .remove("durationSeconds")
        .or_else(|| root.remove("duration_seconds"));
    let param_duration = parameters
        .remove("durationSeconds")
        .or_else(|| parameters.remove("duration_seconds"));
    if request.duration_sec.is_some() && (root_duration.is_some() || param_duration.is_some()) {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            "both duration_sec and provider_options.durationSeconds were provided; keep only one source for video duration",
        )
        .with_provider("google"));
    }
    if root_duration.is_some() && param_duration.is_some() {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            "durationSeconds was provided in both provider_options and provider_options.parameters; keep only one source",
        )
        .with_provider("google"));
    }
    let duration_seconds = resolve_google_video_duration_seconds(
        route,
        request.duration_sec,
        root_duration.as_ref().or(param_duration.as_ref()),
    )?;
    if let Some(duration_seconds) = duration_seconds {
        parameters.insert(
            "durationSeconds".to_string(),
            Value::Number(duration_seconds.into()),
        );
    }

    let root_resolution = root.remove("resolution");
    let param_resolution = parameters.remove("resolution");
    if root_resolution.is_some() && param_resolution.is_some() {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            "resolution was provided in both provider_options and provider_options.parameters; keep only one source",
        )
        .with_provider("google"));
    }
    let explicit_resolution = parse_google_video_resolution_value(
        route,
        root_resolution.as_ref().or(param_resolution.as_ref()),
        "resolution",
    )?;

    let root_size = root.remove("size");
    let param_size = parameters.remove("size");
    if root_size.is_some() && param_size.is_some() {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            "size was provided in both provider_options and provider_options.parameters; keep only one source",
        )
        .with_provider("google"));
    }
    let requested_size =
        parse_google_video_size_value(route, root_size.as_ref().or(param_size.as_ref()), "size")?;

    let root_width = root.remove("width");
    let root_height = root.remove("height");
    let param_width = parameters.remove("width");
    let param_height = parameters.remove("height");
    let has_root_dimensions = root_width.is_some() || root_height.is_some();
    let has_param_dimensions = param_width.is_some() || param_height.is_some();
    if has_root_dimensions && has_param_dimensions {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            "width/height was provided in both provider_options and provider_options.parameters; keep only one source",
        )
        .with_provider("google"));
    }
    let requested_dimensions = if has_root_dimensions {
        parse_google_video_dimensions_pair(
            route,
            root_width.as_ref(),
            root_height.as_ref(),
            "provider_options.width/height",
        )?
    } else if has_param_dimensions {
        parse_google_video_dimensions_pair(
            route,
            param_width.as_ref(),
            param_height.as_ref(),
            "provider_options.parameters.width/height",
        )?
    } else {
        None
    };

    let resolution = explicit_resolution
        .or_else(|| requested_size.map(|(w, h)| resolution_from_dimensions(w, h)))
        .or_else(|| requested_dimensions.map(|(w, h)| resolution_from_dimensions(w, h)))
        .unwrap_or_else(|| "720p".to_string());
    validate_google_video_resolution_for_duration(route, resolution.as_str(), duration_seconds)?;
    parameters.insert("resolution".to_string(), Value::String(resolution));

    root.insert(
        "instances".to_string(),
        Value::Array(vec![json!({
            "prompt": compose_google_prompt(request.prompt.as_str(), request.negative_prompt.as_deref())
        })]),
    );
    if !parameters.is_empty() {
        root.insert("parameters".to_string(), Value::Object(parameters));
    }

    Ok(Value::Object(root))
}

fn resolve_google_video_duration_seconds(
    route: &ModelRouteKey,
    duration_sec: Option<f64>,
    provider_duration: Option<&Value>,
) -> Result<Option<u64>> {
    let raw_seconds = if let Some(provider_duration) = provider_duration {
        parse_google_video_duration_value(provider_duration)?
    } else {
        duration_sec
    };
    let Some(raw_seconds) = raw_seconds else {
        return Ok(None);
    };

    if !raw_seconds.is_finite() || raw_seconds <= 0.0 {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            format!(
                "requested video duration='{raw_seconds}' for model '{}' is invalid; expected an integer between {} and {} seconds",
                route.model_name, GOOGLE_VEO_MIN_SECONDS, GOOGLE_VEO_MAX_SECONDS
            ),
        )
        .with_provider("google"));
    }

    let rounded = raw_seconds.round();
    if (raw_seconds - rounded).abs() > 0.0001 {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            format!(
                "requested video duration='{raw_seconds}' for model '{}' is unsupported; Gemini Veo accepts whole seconds between {} and {}",
                route.model_name, GOOGLE_VEO_MIN_SECONDS, GOOGLE_VEO_MAX_SECONDS
            ),
        )
        .with_provider("google"));
    }

    let seconds = rounded as i64;
    if seconds < GOOGLE_VEO_MIN_SECONDS as i64 || seconds > GOOGLE_VEO_MAX_SECONDS as i64 {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            format!(
                "requested video duration='{seconds}' for model '{}' is unsupported; Gemini Veo accepts {}..={} seconds",
                route.model_name, GOOGLE_VEO_MIN_SECONDS, GOOGLE_VEO_MAX_SECONDS
            ),
        )
        .with_provider("google"));
    }
    Ok(Some(seconds as u64))
}

fn parse_google_video_duration_value(value: &Value) -> Result<Option<f64>> {
    match value {
        Value::Null => Ok(None),
        Value::Number(number) => Ok(number.as_f64()),
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            let parsed = trimmed.parse::<f64>().map_err(|_| {
                ProtocolError::new(
                    ErrorCode::InvalidArgument,
                    format!(
                        "provider_options.durationSeconds='{trimmed}' is not numeric; use an integer between {} and {}",
                        GOOGLE_VEO_MIN_SECONDS, GOOGLE_VEO_MAX_SECONDS
                    ),
                )
                .with_provider("google")
            })?;
            Ok(Some(parsed))
        }
        Value::Bool(_) | Value::Array(_) | Value::Object(_) => Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            format!(
                "provider_options.durationSeconds must be a number/string; use an integer between {} and {}",
                GOOGLE_VEO_MIN_SECONDS, GOOGLE_VEO_MAX_SECONDS
            ),
        )
        .with_provider("google")),
    }
}

fn parse_google_video_resolution_value(
    route: &ModelRouteKey,
    value: Option<&Value>,
    field_name: &str,
) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        Value::Null => Ok(None),
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            let normalized = trimmed.to_ascii_lowercase();
            if matches!(normalized.as_str(), "720p" | "1080p" | "4k") {
                let canonical = if normalized == "4k" {
                    "4K".to_string()
                } else {
                    normalized
                };
                return Ok(Some(canonical));
            }
            if let Some((width, height)) = parse_size_from_str(normalized.as_str()) {
                return Ok(Some(resolution_from_dimensions(width, height)));
            }
            Err(ProtocolError::new(
                ErrorCode::InvalidArgument,
                format!(
                    "provider_options.{field_name}='{trimmed}' for model '{}' is invalid; use one of: 720p, 1080p, 4K",
                    route.model_name
                ),
            )
            .with_provider("google"))
        }
        Value::Bool(_) | Value::Array(_) | Value::Object(_) | Value::Number(_) => {
            Err(ProtocolError::new(
                ErrorCode::InvalidArgument,
                format!(
                    "provider_options.{field_name} must be a string; use one of: 720p, 1080p, 4K",
                ),
            )
            .with_provider("google"))
        }
    }
}

fn parse_google_video_size_value(
    route: &ModelRouteKey,
    value: Option<&Value>,
    field_name: &str,
) -> Result<Option<(u32, u32)>> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        Value::Null => Ok(None),
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            parse_size_from_str(trimmed).ok_or_else(|| {
                ProtocolError::new(
                    ErrorCode::InvalidArgument,
                    format!(
                        "provider_options.{field_name}='{trimmed}' for model '{}' is invalid; expected format WIDTHxHEIGHT (for example 1280x720)",
                        route.model_name
                    ),
                )
                .with_provider("google")
            }).map(Some)
        }
        Value::Bool(_) | Value::Array(_) | Value::Object(_) | Value::Number(_) => Err(
            ProtocolError::new(
                ErrorCode::InvalidArgument,
                format!(
                    "provider_options.{field_name} must be a string in WIDTHxHEIGHT format (for example 1280x720)",
                ),
            )
            .with_provider("google"),
        ),
    }
}

fn parse_google_video_dimensions_pair(
    route: &ModelRouteKey,
    width: Option<&Value>,
    height: Option<&Value>,
    field_name: &str,
) -> Result<Option<(u32, u32)>> {
    if width.is_none() && height.is_none() {
        return Ok(None);
    }
    let Some(width) = width else {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            format!(
                "{field_name} for model '{}' is incomplete; width and height must both be set",
                route.model_name
            ),
        )
        .with_provider("google"));
    };
    let Some(height) = height else {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            format!(
                "{field_name} for model '{}' is incomplete; width and height must both be set",
                route.model_name
            ),
        )
        .with_provider("google"));
    };

    let width = parse_google_video_dimension_value(width, "width")?;
    let height = parse_google_video_dimension_value(height, "height")?;
    Ok(Some((width, height)))
}

fn parse_google_video_dimension_value(value: &Value, name: &str) -> Result<u32> {
    let as_u64 = match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse::<u64>().ok(),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => None,
    };
    let parsed = as_u64
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0);
    parsed.ok_or_else(|| {
        ProtocolError::new(
            ErrorCode::InvalidArgument,
            format!("video {name} must be a positive integer"),
        )
        .with_provider("google")
    })
}

fn validate_google_video_resolution_for_duration(
    route: &ModelRouteKey,
    resolution: &str,
    duration_seconds: Option<u64>,
) -> Result<()> {
    if !matches!(resolution, "720p" | "1080p" | "4K") {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            format!(
                "unsupported resolution='{resolution}' for model '{}'; use one of: 720p, 1080p, 4K",
                route.model_name
            ),
        )
        .with_provider("google"));
    }

    if matches!(resolution, "1080p" | "4K")
        && duration_seconds != Some(GOOGLE_VEO_HIGH_RES_REQUIRED_SECONDS)
    {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            format!(
                "resolution='{resolution}' for model '{}' requires duration={} seconds",
                route.model_name, GOOGLE_VEO_HIGH_RES_REQUIRED_SECONDS
            ),
        )
        .with_provider("google"));
    }

    Ok(())
}

fn resolution_from_dimensions(width: u32, height: u32) -> String {
    let (long_side, short_side) = if width >= height {
        (width, height)
    } else {
        (height, width)
    };

    if long_side <= 1280 && short_side <= 720 {
        "720p".to_string()
    } else if long_side <= 1920 && short_side <= 1080 {
        "1080p".to_string()
    } else {
        "4K".to_string()
    }
}

async fn build_google_content_parts(
    client: &reqwest::Client,
    route: &ModelRouteKey,
    request: &GenerateRequest,
) -> Result<Vec<Value>> {
    let mut parts = Vec::new();
    let mut image_index = 0usize;

    for input in &request.inputs {
        match input.kind {
            InputAssetKind::Image => {
                image_index += 1;
                let downloaded = fetch_remote_input(
                    client,
                    &input.url,
                    &format!("input-image-{image_index}.png"),
                )
                .await?;
                let mime_type = normalize_mime_type(
                    downloaded
                        .content_type
                        .as_deref()
                        .unwrap_or(GOOGLE_DEFAULT_IMAGE_CONTENT_TYPE),
                )
                .unwrap_or(GOOGLE_DEFAULT_IMAGE_CONTENT_TYPE)
                .to_string();
                let encoded = base64::engine::general_purpose::STANDARD.encode(downloaded.bytes);
                parts.push(json!({
                    "inline_data": {
                        "mime_type": mime_type,
                        "data": encoded
                    }
                }));
            }
            InputAssetKind::Mask => {
                return Err(ProtocolError::new(
                    ErrorCode::InvalidArgument,
                    format!(
                        "google model '{}' does not support explicit mask inputs; remove mask input or use 'openai/gpt-image-1' for mask-based edits",
                        route.model_name
                    ),
                )
                .with_provider("google"));
            }
            InputAssetKind::Video | InputAssetKind::Audio => {
                return Err(ProtocolError::new(
                    ErrorCode::InvalidArgument,
                    format!(
                        "unsupported input kind for google image request: {:?}",
                        input.kind
                    ),
                )
                .with_provider("google"));
            }
        }
    }

    let prompt_text =
        compose_google_prompt(request.prompt.as_str(), request.negative_prompt.as_deref());
    parts.push(json!({ "text": prompt_text }));
    Ok(parts)
}

fn compose_google_prompt(prompt: &str, negative_prompt: Option<&str>) -> String {
    if let Some(negative_prompt) = negative_prompt
        && !negative_prompt.trim().is_empty()
    {
        // Keep negative prompts explicit even though Gemini has no dedicated `negative_prompt` field.
        return format!(
            "{prompt}\n\nAvoid the following in the generated image: {}",
            negative_prompt.trim()
        );
    }
    prompt.to_string()
}

fn take_object_entry(map: &mut Map<String, Value>, key: &str) -> Option<Map<String, Value>> {
    map.remove(key).and_then(|value| match value {
        Value::Object(object) => Some(object),
        Value::Array(_) | Value::Bool(_) | Value::Null | Value::Number(_) | Value::String(_) => {
            None
        }
    })
}

fn resolve_api_key(ctx: &GatewayContext, request: &GenerateRequest) -> Result<String> {
    let route = request.route_key()?;
    resolve_api_key_for_route(ctx, &route)
}

fn resolve_api_key_for_route(ctx: &GatewayContext, route: &ModelRouteKey) -> Result<String> {
    let route_key = format!("{}/{}", route.provider, route.model_name);
    let key_slot = ctx
        .model_registry
        .get(route_key.as_str())
        .and_then(|spec| spec.api_key_slot.as_deref())
        .unwrap_or("google");

    let api_key = ctx
        .key_resolver
        .resolve_api_key(key_slot)
        .filter(|key| !key.trim().is_empty());

    api_key.ok_or_else(|| {
        ProtocolError::new(
            ErrorCode::AuthFailed,
            format!(
                "api key not found for slot '{key_slot}' (provider='google', model='{}')",
                route.model_name
            ),
        )
        .with_provider("google")
    })
}

#[derive(Debug, Clone)]
struct DownloadedInput {
    bytes: Vec<u8>,
    content_type: Option<String>,
}

async fn fetch_remote_input(
    client: &reqwest::Client,
    source_url: &str,
    fallback_file_name: &str,
) -> Result<DownloadedInput> {
    if let Some(local) = try_read_local_input(source_url, fallback_file_name)? {
        return Ok(local);
    }

    let response = client
        .get(source_url)
        .send()
        .await
        .map_err(map_reqwest_transport_error)?;
    let status = response.status();

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let mut err = ProtocolError::new(
            if status.is_client_error() {
                ErrorCode::InvalidArgument
            } else {
                ErrorCode::ProviderUnavailable
            },
            format!("failed downloading input asset ({status})"),
        )
        .with_provider("google")
        .with_http_status(status.as_u16());
        if status.as_u16() == 429 || status.is_server_error() {
            err = err.retriable(true);
        }
        if !body.trim().is_empty() {
            err = err.with_details(Value::String(body));
        }
        return Err(err);
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let bytes = response.bytes().await.map_err(|err| {
        ProtocolError::new(
            ErrorCode::ProviderUnavailable,
            format!("failed reading downloaded input bytes: {err}"),
        )
        .retriable(true)
        .with_provider("google")
    })?;

    Ok(DownloadedInput {
        bytes: bytes.to_vec(),
        content_type,
    })
}

fn try_read_local_input(
    source: &str,
    _fallback_file_name: &str,
) -> Result<Option<DownloadedInput>> {
    let source = source.trim();
    if source.is_empty() {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            "input asset URL/path cannot be empty",
        )
        .with_provider("google"));
    }
    if source.starts_with("http://") || source.starts_with("https://") {
        return Ok(None);
    }

    let local_path = local_path_from_source(source);
    let bytes = fs::read(&local_path).map_err(|err| {
        ProtocolError::new(
            ErrorCode::InvalidArgument,
            format!(
                "failed reading local input asset '{}': {err}",
                local_path.display()
            ),
        )
        .with_provider("google")
    })?;
    let content_type = infer_content_type_from_path(&local_path).map(ToString::to_string);

    Ok(Some(DownloadedInput {
        bytes,
        content_type,
    }))
}

fn local_path_from_source(source: &str) -> PathBuf {
    if let Some(rest) = source.strip_prefix("file://") {
        let decoded = percent_decode(rest);
        if cfg!(windows) && decoded.starts_with('/') && decoded.chars().nth(2) == Some(':') {
            return PathBuf::from(decoded[1..].to_string());
        }
        return PathBuf::from(decoded);
    }
    PathBuf::from(source)
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) =
                (hex_nibble(bytes[index + 1]), hex_nibble(bytes[index + 2]))
        {
            output.push((high << 4) | low);
            index += 3;
            continue;
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(output.as_slice()).into_owned()
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn infer_content_type_from_path(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "bmp" => Some("image/bmp"),
        "tif" | "tiff" => Some("image/tiff"),
        _ => None,
    }
}

fn normalize_mime_type(raw: &str) -> Option<&str> {
    raw.split(';')
        .next()
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn map_reqwest_transport_error(err: reqwest::Error) -> ProtocolError {
    let code = if err.is_timeout() {
        ErrorCode::Timeout
    } else {
        ErrorCode::ProviderUnavailable
    };
    ProtocolError::new(code, format!("google request transport failed: {err}"))
        .retriable(true)
        .with_provider("google")
}

fn map_google_http_error_with_operation(
    status: u16,
    body_text: &str,
    operation: &str,
) -> ProtocolError {
    let parsed = serde_json::from_str::<Value>(body_text).ok();
    let error_obj = parsed
        .as_ref()
        .and_then(|value| value.get("error"))
        .and_then(Value::as_object);

    let provider_status = error_obj
        .and_then(|obj| obj.get("status"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let provider_message = error_obj
        .and_then(|obj| obj.get("message"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let provider_code = provider_status.clone().or_else(|| {
        error_obj
            .and_then(|obj| obj.get("code"))
            .and_then(Value::as_i64)
            .map(|code| code.to_string())
    });

    let lower_message = provider_message
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let lower_status = provider_status
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let is_content_policy = lower_status.contains("safety")
        || lower_status.contains("blocked")
        || lower_message.contains("safety")
        || lower_message.contains("policy")
        || lower_message.contains("blocked");

    let error_code = if status == 401 || status == 403 {
        ErrorCode::AuthFailed
    } else if status == 429 {
        ErrorCode::RateLimited
    } else if is_content_policy {
        ErrorCode::ContentPolicyViolation
    } else if (500..=599).contains(&status) {
        ErrorCode::ProviderUnavailable
    } else {
        ErrorCode::InvalidArgument
    };

    let mut message = String::new();
    let _ = write!(&mut message, "google {operation} failed ({status})");
    if let Some(provider_message) = provider_message.as_deref() {
        let _ = write!(&mut message, ": {provider_message}");
    } else if !body_text.trim().is_empty() {
        let _ = write!(&mut message, ": {}", body_text.trim());
    }

    let mut err = ProtocolError::new(error_code, message)
        .with_provider("google")
        .with_http_status(status);
    if let Some(provider_code) = provider_code {
        err = err.with_provider_code(provider_code);
    }
    if status == 429 || (500..=599).contains(&status) {
        err = err.retriable(true);
    }
    if let Some(parsed) = parsed {
        err = err.with_details(parsed);
    }
    err
}

fn parse_google_operation_status(payload: &Value) -> Option<JobStatus> {
    if payload.get("error").is_some() {
        if is_google_operation_canceled(payload) {
            return Some(JobStatus::Canceled);
        }
        return Some(JobStatus::Failed);
    }

    if let Some(done) = payload.get("done").and_then(Value::as_bool) {
        if done {
            return Some(JobStatus::Succeeded);
        }
    }

    let metadata_state = first_non_empty_string_by_pointers(
        payload,
        &[
            "/metadata/state",
            "/metadata/status",
            "/metadata/operationState",
            "/metadata/operation_state",
        ],
    );
    if let Some(state) = metadata_state {
        return Some(map_google_operation_state(state));
    }

    if payload.get("name").is_some() {
        return Some(JobStatus::Running);
    }
    None
}

fn map_google_operation_state(state: &str) -> JobStatus {
    let lowered = state.trim().to_ascii_lowercase();
    match lowered.as_str() {
        "queued" | "queueing" | "pending" | "not_started" => JobStatus::Queued,
        "running" | "processing" | "in_progress" | "active" => JobStatus::Running,
        "succeeded" | "completed" | "complete" | "done" | "finished" => JobStatus::Succeeded,
        "canceled" | "cancelled" | "aborted" => JobStatus::Canceled,
        "failed" | "error" => JobStatus::Failed,
        _ => JobStatus::Running,
    }
}

fn parse_google_operation_error(payload: &Value) -> Option<ProtocolError> {
    let error = payload.get("error")?.as_object()?;
    let provider_message = error
        .get("message")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| "google operation failed".to_string());
    let provider_status = error
        .get("status")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let provider_code = provider_status.clone().or_else(|| {
        error
            .get("code")
            .and_then(Value::as_i64)
            .map(|value| value.to_string())
    });

    let status_text = provider_status
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let message_text = provider_message.to_ascii_lowercase();
    let error_code = if status_text.contains("safety")
        || status_text.contains("blocked")
        || message_text.contains("safety")
        || message_text.contains("policy")
    {
        ErrorCode::ContentPolicyViolation
    } else {
        ErrorCode::JobFailed
    };

    let mut err = ProtocolError::new(error_code, provider_message).with_provider("google");
    if let Some(provider_code) = provider_code {
        err = err.with_provider_code(provider_code);
    }
    Some(err)
}

fn is_google_operation_canceled(payload: &Value) -> bool {
    let status =
        first_non_empty_string_by_pointers(payload, &["/error/status"]).unwrap_or_default();
    let message =
        first_non_empty_string_by_pointers(payload, &["/error/message"]).unwrap_or_default();
    let status = status.to_ascii_lowercase();
    let message = message.to_ascii_lowercase();
    status.contains("cancel") || status.contains("aborted") || message.contains("cancel")
}

fn extract_google_video_uri(payload: &Value) -> Option<String> {
    first_non_empty_string_by_pointers(
        payload,
        &[
            "/response/generateVideoResponse/generatedSamples/0/video/uri",
            "/response/generate_video_response/generated_samples/0/video/uri",
            "/response/generatedSamples/0/video/uri",
            "/response/generated_samples/0/video/uri",
            "/response/video/uri",
            "/video/uri",
        ],
    )
    .map(ToString::to_string)
}

fn parse_google_video_dimensions(payload: &Value) -> (Option<u32>, Option<u32>) {
    let width = first_u64_by_pointers(
        payload,
        &[
            "/response/generateVideoResponse/generatedSamples/0/video/width",
            "/response/generate_video_response/generated_samples/0/video/width",
            "/response/video/width",
            "/response/width",
            "/width",
        ],
    )
    .and_then(|value| u32::try_from(value).ok());
    let height = first_u64_by_pointers(
        payload,
        &[
            "/response/generateVideoResponse/generatedSamples/0/video/height",
            "/response/generate_video_response/generated_samples/0/video/height",
            "/response/video/height",
            "/response/height",
            "/height",
        ],
    )
    .and_then(|value| u32::try_from(value).ok());
    if width.is_some() && height.is_some() {
        return (width, height);
    }

    if let Some(resolution) = first_non_empty_string_by_pointers(
        payload,
        &[
            "/response/generateVideoResponse/generatedSamples/0/video/resolution",
            "/response/generate_video_response/generated_samples/0/video/resolution",
            "/response/video/resolution",
            "/response/resolution",
            "/metadata/resolution",
            "/resolution",
        ],
    ) {
        let normalized = resolution.to_ascii_lowercase();
        match normalized.as_str() {
            "720p" => return (Some(1280), Some(720)),
            "1080p" => return (Some(1920), Some(1080)),
            "4k" => return (Some(3840), Some(2160)),
            _ => {
                if let Some((width, height)) = parse_size_from_str(normalized.as_str()) {
                    return (Some(width), Some(height));
                }
            }
        }
    }

    (width, height)
}

fn parse_google_video_duration_ms(payload: &Value) -> Option<u64> {
    let seconds = first_f64_by_pointers(
        payload,
        &[
            "/response/generateVideoResponse/generatedSamples/0/video/durationSeconds",
            "/response/generateVideoResponse/generatedSamples/0/video/duration_seconds",
            "/response/generate_video_response/generated_samples/0/video/duration_seconds",
            "/response/durationSeconds",
            "/response/duration_seconds",
            "/metadata/durationSeconds",
            "/metadata/duration_seconds",
            "/durationSeconds",
            "/duration_seconds",
        ],
    )?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    Some((seconds * 1000.0).round() as u64)
}

fn first_non_empty_string_by_pointers<'a>(
    payload: &'a Value,
    pointers: &[&str],
) -> Option<&'a str> {
    for pointer in pointers {
        let text = payload
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty());
        if text.is_some() {
            return text;
        }
    }
    None
}

fn first_u64_by_pointers(payload: &Value, pointers: &[&str]) -> Option<u64> {
    for pointer in pointers {
        if let Some(value) = payload.pointer(pointer).and_then(Value::as_u64) {
            return Some(value);
        }
    }
    None
}

fn first_f64_by_pointers(payload: &Value, pointers: &[&str]) -> Option<f64> {
    for pointer in pointers {
        if let Some(value) = payload.pointer(pointer).and_then(Value::as_f64) {
            return Some(value);
        }
        if let Some(value) = payload.pointer(pointer).and_then(Value::as_str)
            && let Ok(parsed) = value.trim().parse::<f64>()
        {
            return Some(parsed);
        }
    }
    None
}

fn extract_google_raw_outputs(
    payload: &Value,
    hints: &GoogleOutputHints,
) -> Result<Vec<ProviderRawOutput>> {
    if let Some(blocking_error) = parse_google_blocking_error(payload) {
        return Err(blocking_error);
    }

    let candidates = payload
        .get("candidates")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ProtocolError::new(
                ErrorCode::JobFailed,
                "google image response missing `candidates` array",
            )
            .with_provider("google")
        })?;

    let mut outputs = Vec::new();
    for candidate in candidates {
        let parts = candidate
            .get("content")
            .and_then(|content| content.get("parts"))
            .and_then(Value::as_array)
            .unwrap_or(&Vec::new())
            .clone();

        for part in parts {
            if part.get("thought").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            let inline_data = part
                .get("inline_data")
                .or_else(|| part.get("inlineData"))
                .and_then(Value::as_object);
            let Some(inline_data) = inline_data else {
                continue;
            };

            let encoded = inline_data
                .get("data")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .ok_or_else(|| {
                    ProtocolError::new(
                        ErrorCode::JobFailed,
                        "google inline_data entry is missing non-empty `data`",
                    )
                    .with_provider("google")
                })?;
            let content_type = inline_data
                .get("mime_type")
                .or_else(|| inline_data.get("mimeType"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .unwrap_or(GOOGLE_DEFAULT_IMAGE_CONTENT_TYPE)
                .to_string();

            outputs.push(ProviderRawOutput {
                content_type,
                url: None,
                base64_data: Some(encoded.to_string()),
                width: hints.size.map(|size| size.0),
                height: hints.size.map(|size| size.1),
                duration_ms: None,
            });
        }
    }

    if !outputs.is_empty() {
        return Ok(outputs);
    }

    let provider_message = first_google_response_text(payload).unwrap_or_else(|| {
        "google image response contained no inline_data image output".to_string()
    });
    Err(ProtocolError::new(ErrorCode::JobFailed, provider_message)
        .with_provider("google")
        .with_details(payload.clone()))
}

fn parse_google_blocking_error(payload: &Value) -> Option<ProtocolError> {
    let prompt_feedback = payload
        .get("promptFeedback")
        .or_else(|| payload.get("prompt_feedback"))?;
    let block_reason = prompt_feedback
        .get("blockReason")
        .or_else(|| prompt_feedback.get("block_reason"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())?;

    let message =
        format!("google image request blocked by safety policy (block_reason='{block_reason}')");
    Some(
        ProtocolError::new(ErrorCode::ContentPolicyViolation, message)
            .with_provider("google")
            .with_provider_code(block_reason.to_string()),
    )
}

fn first_google_response_text(payload: &Value) -> Option<String> {
    payload
        .get("candidates")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|candidate| {
            candidate
                .get("content")
                .and_then(|content| content.get("parts"))
                .and_then(Value::as_array)
                .and_then(|parts| {
                    parts.iter().find_map(|part| {
                        part.get("text")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|text| !text.is_empty())
                            .map(ToString::to_string)
                    })
                })
        })
}

fn aspect_ratio_from_size_text(size: &str) -> Option<String> {
    let (width, height) = parse_size_from_str(size)?;
    simplify_ratio(width, height)
}

fn normalize_google_aspect_ratio(raw: &str) -> Option<String> {
    let normalized = raw.trim();
    if normalized.is_empty() {
        return None;
    }
    if let Some((width, height)) = parse_ratio_from_str(normalized) {
        return simplify_ratio(width, height);
    }
    parse_size_from_str(normalized).and_then(|(width, height)| simplify_ratio(width, height))
}

fn parse_ratio_from_str(raw: &str) -> Option<(u32, u32)> {
    let separator = if raw.contains(':') { ':' } else { '/' };
    let (width, height) = raw.split_once(separator)?;
    let width = width.trim().parse::<u32>().ok()?;
    let height = height.trim().parse::<u32>().ok()?;
    Some((width, height))
}

fn simplify_ratio(width: u32, height: u32) -> Option<String> {
    if width == 0 || height == 0 {
        return None;
    }
    let divisor = gcd(width, height).max(1);
    Some(format!("{}:{}", width / divisor, height / divisor))
}

fn gcd(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let next = left % right;
        left = right;
        right = next;
    }
    left
}

fn parse_size_from_str(size: &str) -> Option<(u32, u32)> {
    let (width, height) = size.split_once('x')?;
    let width = width.trim().parse::<u32>().ok()?;
    let height = height.trim().parse::<u32>().ok()?;
    Some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{AssetKind, GenerateRequest};
    use serde_json::json;

    #[test]
    fn supports_configured_google_image_models() {
        for model_name in models::supported_model_names() {
            let route = ModelRouteKey::parse(&format!("google/{model_name}")).expect("parse route");
            models::require_supported_model(&route).expect("model should be supported");
        }
    }

    #[test]
    fn google_image_models_reject_video_asset_kind() {
        let route = ModelRouteKey::parse(&format!(
            "google/{}",
            models::MODEL_GEMINI_3_PRO_IMAGE_PREVIEW
        ))
        .expect("parse route");
        let err = models::require_model_supports_asset_kind(&route, AssetKind::Video)
            .expect_err("video must be rejected for image-only model");
        assert_eq!(err.code, ErrorCode::InvalidArgument);
        assert!(err.message.contains("asset_kind='video'"));
        assert!(err.message.contains(models::MODEL_VEO_3_1));
    }

    #[test]
    fn unknown_google_model_lists_valid_alternatives() {
        let route = ModelRouteKey::parse("google/gemini-unknown-image-model").expect("parse route");
        let err = models::require_supported_model(&route).expect_err("must reject unknown model");
        assert_eq!(err.code, ErrorCode::InvalidArgument);
        assert!(err.message.contains("gemini-unknown-image-model"));
        assert!(err.message.contains(models::MODEL_GEMINI_2_5_FLASH_IMAGE));
    }

    #[test]
    fn parse_size_to_aspect_ratio() {
        assert_eq!(
            aspect_ratio_from_size_text("1920x1080").as_deref(),
            Some("16:9")
        );
        assert_eq!(
            aspect_ratio_from_size_text("1080x1920").as_deref(),
            Some("9:16")
        );
    }

    #[test]
    fn extract_inline_data_outputs() {
        let payload = json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "inline_data": {
                            "mime_type": "image/png",
                            "data": "aGVsbG8="
                        }
                    }]
                }
            }]
        });
        let hints = GoogleOutputHints {
            size: Some((1024, 1024)),
        };
        let outputs = extract_google_raw_outputs(&payload, &hints).expect("must parse outputs");
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].content_type, "image/png");
        assert_eq!(outputs[0].width, Some(1024));
        assert_eq!(outputs[0].height, Some(1024));
    }

    #[test]
    fn parse_prompt_feedback_blocking_error() {
        let payload = json!({
            "prompt_feedback": {
                "block_reason": "SAFETY"
            },
            "candidates": []
        });
        let err = extract_google_raw_outputs(&payload, &GoogleOutputHints { size: None })
            .expect_err("must map blocked output");
        assert_eq!(err.code, ErrorCode::ContentPolicyViolation);
        assert_eq!(err.provider.as_deref(), Some("google"));
    }

    #[test]
    fn veo_model_accepts_video_and_rejects_image_asset_kind() {
        let route = ModelRouteKey::parse("google/veo_3_1").expect("parse route");
        assert!(models::require_model_supports_asset_kind(&route, AssetKind::Video).is_ok());
        assert!(models::require_model_supports_asset_kind(&route, AssetKind::Image).is_err());
    }

    #[test]
    fn veo_alias_model_name_is_supported() {
        let route = ModelRouteKey::parse("google/veo-3.1-generate-preview").expect("parse route");
        let spec =
            models::require_supported_model(&route).expect("alias model should be supported");
        assert_eq!(spec.name, models::MODEL_VEO_3_1);
        assert_eq!(spec.api_model_name, "veo-3.1-generate-preview");
        assert_eq!(spec.asset_kind, AssetKind::Video);
    }

    #[test]
    fn video_payload_maps_size_to_resolution() {
        let route = ModelRouteKey::parse("google/veo_3_1").expect("parse route");
        let mut provider_options = Map::new();
        provider_options.insert("size".to_string(), Value::String("1920x1080".to_string()));
        let req = video_request(Some(8.0), provider_options);
        let payload =
            build_google_video_predict_payload(&route, &req).expect("payload should build");
        assert_eq!(
            payload
                .pointer("/parameters/resolution")
                .and_then(Value::as_str),
            Some("1080p")
        );
        assert_eq!(
            payload
                .pointer("/parameters/durationSeconds")
                .and_then(Value::as_u64),
            Some(8)
        );
    }

    #[test]
    fn video_payload_rejects_high_res_without_eight_seconds() {
        let route = ModelRouteKey::parse("google/veo_3_1").expect("parse route");
        let mut provider_options = Map::new();
        provider_options.insert("size".to_string(), Value::String("1920x1080".to_string()));
        let req = video_request(Some(6.0), provider_options);
        let err = build_google_video_predict_payload(&route, &req)
            .expect_err("high resolution must require 8 seconds");
        assert_eq!(err.code, ErrorCode::InvalidArgument);
        assert!(err.message.contains("requires duration=8 seconds"));
    }

    #[test]
    fn video_payload_maps_large_size_to_uppercase_4k_resolution() {
        let route = ModelRouteKey::parse("google/veo_3_1").expect("parse route");
        let mut provider_options = Map::new();
        provider_options.insert("size".to_string(), Value::String("3840x2160".to_string()));
        let req = video_request(Some(8.0), provider_options);
        let payload =
            build_google_video_predict_payload(&route, &req).expect("payload should build");
        assert_eq!(
            payload
                .pointer("/parameters/resolution")
                .and_then(Value::as_str),
            Some("4K")
        );
    }

    #[test]
    fn video_payload_normalizes_lowercase_4k_to_uppercase_4k_resolution() {
        let route = ModelRouteKey::parse("google/veo_3_1").expect("parse route");
        let mut provider_options = Map::new();
        provider_options.insert("resolution".to_string(), Value::String("4k".to_string()));
        let req = video_request(Some(8.0), provider_options);
        let payload =
            build_google_video_predict_payload(&route, &req).expect("payload should build");
        assert_eq!(
            payload
                .pointer("/parameters/resolution")
                .and_then(Value::as_str),
            Some("4K")
        );
    }

    #[test]
    fn video_payload_rejects_out_of_range_duration() {
        let route = ModelRouteKey::parse("google/veo_3_1").expect("parse route");
        let mut provider_options = Map::new();
        provider_options.insert("resolution".to_string(), Value::String("720p".to_string()));
        let req = video_request(Some(4.0), provider_options);
        let err =
            build_google_video_predict_payload(&route, &req).expect_err("must reject duration < 5");
        assert_eq!(err.code, ErrorCode::InvalidArgument);
        assert!(err.message.contains("accepts 5..=8 seconds"));
    }

    #[test]
    fn parse_operation_status_done_and_error() {
        assert_eq!(
            parse_google_operation_status(&json!({ "name": "operations/abc", "done": false })),
            Some(JobStatus::Running)
        );
        assert_eq!(
            parse_google_operation_status(&json!({
                "name": "operations/abc",
                "done": true,
                "error": { "status": "FAILED_PRECONDITION", "message": "blocked" }
            })),
            Some(JobStatus::Failed)
        );
        assert_eq!(
            parse_google_operation_status(&json!({
                "name": "operations/abc",
                "done": true,
                "error": { "status": "CANCELLED", "message": "cancelled by user" }
            })),
            Some(JobStatus::Canceled)
        );
    }

    fn video_request(
        duration_sec: Option<f64>,
        provider_options: Map<String, Value>,
    ) -> GenerateRequest {
        GenerateRequest {
            model: "google/veo_3_1".to_string(),
            asset_kind: AssetKind::Video,
            prompt: "A calm lake at sunrise".to_string(),
            negative_prompt: None,
            inputs: Vec::new(),
            duration_sec,
            aspect_ratio: None,
            provider_options,
            callback_url: None,
            idempotency_key: None,
            metadata: Map::new(),
        }
    }
}
