// =========================================
// =========================================
// crates/media_gen_protocol/src/provider/openai/mod.rs

use crate::error::{ErrorCode, ProtocolError, Result};
use crate::gateway::GatewayContext;
use crate::model::ModelRouteKey;
use crate::output::{ProviderRawOutput, normalize_provider_output};
use crate::protocol::{
    AssetKind, GenerateRequest, GenerateResult, InputAssetKind, JobStatus, OutputAsset,
};
use crate::provider::{ProviderAdapter, ProviderPollResult, ProviderSubmitResult};
use async_trait::async_trait;
use reqwest::multipart::{Form, Part};
use serde_json::{Map, Value};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

mod models;
use models::MODEL_GPT_IMAGE_1;

const OPENAI_IMAGES_ENDPOINT: &str = "https://api.openai.com/v1/images/generations";
const OPENAI_IMAGES_EDITS_ENDPOINT: &str = "https://api.openai.com/v1/images/edits";
const OPENAI_VIDEOS_ENDPOINT: &str = "https://api.openai.com/v1/videos";
const OPENAI_SORA_ALLOWED_SECONDS: &[u64] = &[4, 8, 12];

#[derive(Debug, Clone, Copy, Default)]
pub struct OpenAiAdapter;

#[async_trait]
impl ProviderAdapter for OpenAiAdapter {
    fn provider(&self) -> &'static str {
        "openai"
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
                    "openai adapter received non-openai route key: {}",
                    request.model
                ),
            ));
        }
        require_supported_model(&route)?;
        if request.asset_kind == AssetKind::Video {
            require_model_supports_asset_kind(&route, AssetKind::Video)?;
            return self.submit_video(ctx, request, &route).await;
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

        let has_image_inputs = has_edit_inputs(request)?;
        let response_payload = if has_image_inputs {
            require_model_supports_image_edit(&route)?;
            submit_openai_image_edit(&client, &api_key, &route, request).await?
        } else {
            submit_openai_image_generation(&client, &api_key, &route, request).await?
        };

        let hints = OpenAiOutputHints::from_request(request);
        let mut outputs = Vec::new();
        let data = response_payload
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ProtocolError::new(
                    ErrorCode::JobFailed,
                    "openai image response missing `data` array",
                )
                .with_provider(self.provider())
            })?;

        for item in data {
            let raw = extract_openai_raw_output(item, &hints)?;
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
                format!("openai adapter poll received non-openai route key: {model}"),
            ));
        }
        require_supported_model(&route)?;

        if models::is_image_model(route.model_name.as_str()) {
            return Ok(ProviderPollResult {
                status: JobStatus::Failed,
                result: None,
                error: Some(
                    ProtocolError::new(
                        ErrorCode::InvalidArgument,
                        "openai image generation is sync in this adapter; no poll step",
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
                format!("openai adapter cancel received non-openai route key: {model}"),
            ));
        }
        require_supported_model(&route)?;

        if models::is_image_model(route.model_name.as_str()) {
            return Err(ProtocolError::new(
                ErrorCode::InvalidArgument,
                "openai image generation is sync in this adapter; cancel not supported",
            )
            .with_provider(self.provider()));
        }

        self.cancel_video(ctx, &route, provider_job_id).await
    }
}

impl OpenAiAdapter {
    async fn submit_video(
        &self,
        ctx: &GatewayContext,
        request: &GenerateRequest,
        route: &ModelRouteKey,
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
            submit_openai_video_generation(&client, &api_key, route, request).await?;
        let provider_job_id = response_payload
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .map(ToString::to_string)
            .ok_or_else(|| {
                ProtocolError::new(ErrorCode::JobFailed, "openai video response missing `id`")
                    .with_provider(self.provider())
            })?;

        let status = parse_openai_video_status(&response_payload).unwrap_or(JobStatus::Queued);
        if status == JobStatus::Failed {
            let error = parse_openai_video_job_error(&response_payload).unwrap_or_else(|| {
                ProtocolError::new(
                    ErrorCode::JobFailed,
                    format!("openai video job '{provider_job_id}' failed"),
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
            let canceled_message = format!("openai video job '{provider_job_id}' was canceled");
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

        // OpenAI video outputs are finalized in the poll phase where we download content.
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

        let payload = retrieve_openai_video(&client, &api_key, provider_job_id).await?;
        let status = parse_openai_video_status(&payload).unwrap_or(JobStatus::Running);

        if matches!(status, JobStatus::Queued | JobStatus::Running) {
            return Ok(ProviderPollResult {
                status,
                result: None,
                error: None,
            });
        }

        if status == JobStatus::Failed {
            let error = parse_openai_video_job_error(&payload).unwrap_or_else(|| {
                ProtocolError::new(
                    ErrorCode::JobFailed,
                    format!("openai video job '{provider_job_id}' failed"),
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
                        format!("openai video job '{provider_job_id}' was canceled"),
                    )
                    .with_provider(self.provider()),
                ),
            });
        }

        let (bytes, content_type) =
            download_openai_video_content(&client, &api_key, provider_job_id).await?;
        let uploaded_url = ctx
            .output_uploader
            .upload_bytes(content_type.as_str(), &bytes)
            .await?;
        let (width, height) = parse_openai_video_dimensions(&payload);
        let duration_ms = parse_openai_video_duration_ms(&payload);

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

        let response = client
            .delete(format!("{OPENAI_VIDEOS_ENDPOINT}/{provider_job_id}"))
            .bearer_auth(api_key.trim())
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
                format!("failed reading openai video cancel response body: {err}"),
            )
            .retriable(true)
            .with_provider(self.provider())
        })?;
        Err(map_openai_http_error_with_operation(
            status,
            body.as_str(),
            "video cancel request",
        ))
    }
}

#[derive(Debug, Clone)]
struct OpenAiOutputHints {
    size: Option<(u32, u32)>,
    output_format: Option<String>,
}

impl OpenAiOutputHints {
    fn from_request(request: &GenerateRequest) -> Self {
        let size_text = request
            .provider_options
            .get("size")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .unwrap_or_else(|| default_size_for_aspect_ratio(request));
        let size = parse_size_from_str(&size_text);
        let output_format = request
            .provider_options
            .get("output_format")
            .and_then(Value::as_str)
            .map(|v| v.to_ascii_lowercase());
        Self {
            size,
            output_format,
        }
    }
}

async fn submit_openai_image_generation(
    client: &reqwest::Client,
    api_key: &str,
    route: &ModelRouteKey,
    request: &GenerateRequest,
) -> Result<Value> {
    let payload = build_openai_images_payload(route, request);
    let response = client
        .post(OPENAI_IMAGES_ENDPOINT)
        .bearer_auth(api_key.trim())
        .json(&payload)
        .send()
        .await
        .map_err(map_reqwest_transport_error)?;
    parse_openai_json_response(response, "image request").await
}

async fn submit_openai_image_edit(
    client: &reqwest::Client,
    api_key: &str,
    route: &ModelRouteKey,
    request: &GenerateRequest,
) -> Result<Value> {
    let form = build_openai_image_edits_form(client, route, request).await?;
    let response = client
        .post(OPENAI_IMAGES_EDITS_ENDPOINT)
        .bearer_auth(api_key.trim())
        .multipart(form)
        .send()
        .await
        .map_err(map_reqwest_transport_error)?;
    parse_openai_json_response(response, "image edit request").await
}

async fn submit_openai_video_generation(
    client: &reqwest::Client,
    api_key: &str,
    route: &ModelRouteKey,
    request: &GenerateRequest,
) -> Result<Value> {
    let payload = build_openai_videos_payload(route, request)?;
    let response = client
        .post(OPENAI_VIDEOS_ENDPOINT)
        .bearer_auth(api_key.trim())
        .json(&payload)
        .send()
        .await
        .map_err(map_reqwest_transport_error)?;
    parse_openai_json_response(response, "video request").await
}

async fn retrieve_openai_video(
    client: &reqwest::Client,
    api_key: &str,
    provider_job_id: &str,
) -> Result<Value> {
    let response = client
        .get(format!("{OPENAI_VIDEOS_ENDPOINT}/{provider_job_id}"))
        .bearer_auth(api_key.trim())
        .send()
        .await
        .map_err(map_reqwest_transport_error)?;
    parse_openai_json_response(response, "video status request").await
}

async fn download_openai_video_content(
    client: &reqwest::Client,
    api_key: &str,
    provider_job_id: &str,
) -> Result<(Vec<u8>, String)> {
    let response = client
        .get(format!(
            "{OPENAI_VIDEOS_ENDPOINT}/{provider_job_id}/content"
        ))
        .bearer_auth(api_key.trim())
        .send()
        .await
        .map_err(map_reqwest_transport_error)?;
    parse_openai_binary_response(response, "video content request").await
}

async fn parse_openai_json_response(response: reqwest::Response, operation: &str) -> Result<Value> {
    let status = response.status();
    let body_text = response.text().await.map_err(|err| {
        ProtocolError::new(
            ErrorCode::ProviderUnavailable,
            format!("failed reading openai {operation} response body: {err}"),
        )
        .retriable(true)
        .with_provider("openai")
    })?;

    if !status.is_success() {
        return Err(map_openai_http_error_with_operation(
            status.as_u16(),
            &body_text,
            operation,
        ));
    }

    serde_json::from_str::<Value>(&body_text).map_err(|err| {
        ProtocolError::new(
            ErrorCode::ProviderUnavailable,
            format!("failed to parse openai {operation} response json: {err}"),
        )
        .retriable(true)
        .with_provider("openai")
    })
}

async fn parse_openai_binary_response(
    response: reqwest::Response,
    operation: &str,
) -> Result<(Vec<u8>, String)> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(map_openai_http_error_with_operation(
            status.as_u16(),
            body.as_str(),
            operation,
        ));
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(ToString::to_string)
        .unwrap_or_else(|| "video/mp4".to_string());

    let bytes = response.bytes().await.map_err(|err| {
        ProtocolError::new(
            ErrorCode::ProviderUnavailable,
            format!("failed reading openai {operation} bytes: {err}"),
        )
        .retriable(true)
        .with_provider("openai")
    })?;

    Ok((bytes.to_vec(), content_type))
}

async fn build_openai_image_edits_form(
    client: &reqwest::Client,
    route: &ModelRouteKey,
    request: &GenerateRequest,
) -> Result<Form> {
    let mut provider_options = request.provider_options.clone();
    let requested_size = provider_options
        .remove("size")
        .and_then(|v| v.as_str().map(ToString::to_string));
    if is_gpt_image_model(route) {
        // GPT image edits always return base64; response_format is dall-e-2 only.
        provider_options.remove("response_format");
    }
    let mut form = Form::new()
        .text("model", route.model_name.clone())
        .text("prompt", request.prompt.clone());

    if let Some(negative) = request.negative_prompt.as_ref()
        && !negative.trim().is_empty()
    {
        form = form.text("negative_prompt", negative.clone());
    }

    let fallback_size = default_size_for_aspect_ratio(request);
    let normalized_size = normalize_size_for_model(
        route,
        requested_size.as_deref().unwrap_or(fallback_size.as_str()),
    );
    form = form.text("size", normalized_size);
    form = append_provider_options_to_form(form, &provider_options);

    let mut image_index = 0usize;
    let mut has_mask = false;

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
                let field_name = openai_image_edit_image_field_name(route, image_index);
                form = form.part(field_name.to_string(), to_part(downloaded));
            }
            InputAssetKind::Mask => {
                if has_mask {
                    return Err(ProtocolError::new(
                        ErrorCode::InvalidArgument,
                        "openai image edits currently supports only one mask input",
                    )
                    .with_provider("openai"));
                }
                let downloaded = fetch_remote_input(client, &input.url, "input-mask.png").await?;
                form = form.part("mask", to_part(downloaded));
                has_mask = true;
            }
            InputAssetKind::Video | InputAssetKind::Audio => {
                return Err(ProtocolError::new(
                    ErrorCode::InvalidArgument,
                    format!(
                        "unsupported input kind for openai image edits: {:?}",
                        input.kind
                    ),
                )
                .with_provider("openai"));
            }
        }
    }

    if image_index == 0 {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            "multipart image edit requires at least one image input",
        )
        .with_provider("openai"));
    }

    Ok(form)
}

fn openai_image_edit_image_field_name(route: &ModelRouteKey, image_index: usize) -> &'static str {
    // GPT-IMAGE edit API expects image[] fields (even when only one input image).
    // Using legacy "image" field can route request to older validation path (dall-e-2 only).
    if route.model_name.as_str() == MODEL_GPT_IMAGE_1 {
        "image[]"
    } else if image_index <= 1 {
        "image"
    } else {
        "image[]"
    }
}

fn append_provider_options_to_form(mut form: Form, options: &Map<String, Value>) -> Form {
    for (k, v) in options {
        if matches!(k.as_str(), "model" | "prompt" | "negative_prompt" | "size") {
            continue;
        }
        match v {
            Value::String(s) => {
                form = form.text(k.clone(), s.clone());
            }
            Value::Number(n) => {
                form = form.text(k.clone(), n.to_string());
            }
            Value::Bool(b) => {
                form = form.text(k.clone(), b.to_string());
            }
            Value::Array(arr) => {
                for item in arr {
                    if let Some(text) = scalar_value_to_string(item) {
                        form = form.text(k.clone(), text);
                    }
                }
            }
            Value::Object(_) | Value::Null => {}
        }
    }
    form
}

fn scalar_value_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Array(_) | Value::Object(_) | Value::Null => None,
    }
}

#[derive(Debug, Clone)]
struct DownloadedInput {
    bytes: Vec<u8>,
    content_type: Option<String>,
    file_name: String,
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
        .with_provider("openai")
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
        .and_then(|v| v.to_str().ok())
        .map(ToString::to_string);

    let bytes = response.bytes().await.map_err(|err| {
        ProtocolError::new(
            ErrorCode::ProviderUnavailable,
            format!("failed reading downloaded input bytes: {err}"),
        )
        .retriable(true)
        .with_provider("openai")
    })?;

    Ok(DownloadedInput {
        bytes: bytes.to_vec(),
        content_type,
        file_name: file_name_from_url(source_url).unwrap_or_else(|| fallback_file_name.to_string()),
    })
}

fn try_read_local_input(source: &str, fallback_file_name: &str) -> Result<Option<DownloadedInput>> {
    let source = source.trim();
    if source.is_empty() {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            "input asset URL/path cannot be empty",
        )
        .with_provider("openai"));
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
        .with_provider("openai")
    })?;

    let file_name = local_path
        .file_name()
        .and_then(|v| v.to_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| fallback_file_name.to_string());
    let content_type = infer_content_type_from_path(&local_path).map(ToString::to_string);

    Ok(Some(DownloadedInput {
        bytes,
        content_type,
        file_name,
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
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2]))
        {
            out.push((h << 4) | l);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_nibble(ch: u8) -> Option<u8> {
    match ch {
        b'0'..=b'9' => Some(ch - b'0'),
        b'a'..=b'f' => Some(ch - b'a' + 10),
        b'A'..=b'F' => Some(ch - b'A' + 10),
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

fn to_part(input: DownloadedInput) -> Part {
    let DownloadedInput {
        bytes,
        content_type,
        file_name,
    } = input;
    if let Some(content_type) = content_type.as_deref() {
        let mime_type = content_type
            .split(';')
            .next()
            .map(str::trim)
            .unwrap_or(content_type);
        if let Ok(with_mime) = Part::bytes(bytes.clone())
            .file_name(file_name.clone())
            .mime_str(mime_type)
        {
            return with_mime;
        }
    }
    Part::bytes(bytes).file_name(file_name)
}

fn file_name_from_url(url: &str) -> Option<String> {
    let no_query = url.split('?').next()?;
    let name = no_query.rsplit('/').next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn map_reqwest_transport_error(err: reqwest::Error) -> ProtocolError {
    let code = if err.is_timeout() {
        ErrorCode::Timeout
    } else {
        ErrorCode::ProviderUnavailable
    };
    ProtocolError::new(code, format!("openai request transport failed: {err}"))
        .retriable(true)
        .with_provider("openai")
}

fn has_edit_inputs(request: &GenerateRequest) -> Result<bool> {
    if request.inputs.is_empty() {
        return Ok(false);
    }
    let mut has_edit_inputs = false;
    for input in &request.inputs {
        match input.kind {
            InputAssetKind::Image | InputAssetKind::Mask => {
                has_edit_inputs = true;
            }
            InputAssetKind::Video | InputAssetKind::Audio => {
                return Err(ProtocolError::new(
                    ErrorCode::InvalidArgument,
                    format!(
                        "unsupported input kind for openai image request: {:?}",
                        input.kind
                    ),
                )
                .with_provider("openai"));
            }
        }
    }
    Ok(has_edit_inputs)
}

fn require_supported_model(route: &ModelRouteKey) -> Result<()> {
    models::require_supported_model(route).map(|_| ())
}

fn require_model_supports_asset_kind(route: &ModelRouteKey, kind: AssetKind) -> Result<()> {
    models::require_model_supports_asset_kind(route, kind).map(|_| ())
}

fn require_model_supports_image_edit(route: &ModelRouteKey) -> Result<()> {
    models::require_model_supports_image_edit(route)
}

fn is_gpt_image_model(route: &ModelRouteKey) -> bool {
    models::is_image_model(route.model_name.as_str())
}

fn normalize_size_for_model(route: &ModelRouteKey, requested: &str) -> String {
    if is_gpt_image_model(route) {
        return normalize_gpt_image_size(requested);
    }
    normalize_dalle2_size(requested)
}

fn normalize_gpt_image_size(requested: &str) -> String {
    let normalized = requested.trim().to_ascii_lowercase();
    if matches!(
        normalized.as_str(),
        "auto" | "1024x1024" | "1536x1024" | "1024x1536"
    ) {
        return normalized;
    }
    if let Some((w, h)) = parse_size_from_str(&normalized) {
        let ratio = w as f32 / h.max(1) as f32;
        if ratio > 1.1 {
            return "1536x1024".to_string();
        }
        if ratio < 0.9 {
            return "1024x1536".to_string();
        }
    }
    "1024x1024".to_string()
}

fn normalize_dalle2_size(requested: &str) -> String {
    match requested.trim() {
        "256x256" | "512x512" | "1024x1024" => requested.trim().to_string(),
        _ => "1024x1024".to_string(),
    }
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
        .unwrap_or("openai");

    let api_key = ctx
        .key_resolver
        .resolve_api_key(key_slot)
        .or_else(|| ctx.key_resolver.resolve_api_key("openai"))
        .filter(|k| !k.trim().is_empty());

    api_key.ok_or_else(|| {
        ProtocolError::new(
            ErrorCode::AuthFailed,
            format!(
                "api key not found for slot '{key_slot}' (provider='openai', model='{}')",
                route.model_name
            ),
        )
        .with_provider("openai")
    })
}

fn build_openai_images_payload(route: &ModelRouteKey, request: &GenerateRequest) -> Value {
    let mut map = request.provider_options.clone();
    let requested_size = map
        .remove("size")
        .and_then(|v| v.as_str().map(ToString::to_string));
    if is_gpt_image_model(route) {
        // GPT image models do not support response_format on images API.
        map.remove("response_format");
    }
    map.insert("model".to_string(), Value::String(route.model_name.clone()));
    map.insert("prompt".to_string(), Value::String(request.prompt.clone()));
    if let Some(negative) = request.negative_prompt.as_ref()
        && !negative.trim().is_empty()
    {
        map.insert(
            "negative_prompt".to_string(),
            Value::String(negative.clone()),
        );
    }
    let fallback_size = default_size_for_aspect_ratio(request);
    let normalized_size = normalize_size_for_model(
        route,
        requested_size.as_deref().unwrap_or(fallback_size.as_str()),
    );
    map.insert("size".to_string(), Value::String(normalized_size));
    Value::Object(map)
}

fn build_openai_videos_payload(route: &ModelRouteKey, request: &GenerateRequest) -> Result<Value> {
    if !request.inputs.is_empty() {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            format!(
                "openai model '{}' video inputs are not implemented in this adapter yet; submit prompt-only video request or remove inputs",
                route.model_name
            ),
        )
        .with_provider("openai"));
    }

    let mut map = request.provider_options.clone();
    if request.duration_sec.is_some() && map.contains_key("seconds") {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            "both duration_sec and provider_options.seconds were provided; keep only one source for video seconds",
        )
        .with_provider("openai"));
    }

    map.insert("model".to_string(), Value::String(route.model_name.clone()));
    map.insert("prompt".to_string(), Value::String(request.prompt.clone()));
    if let Some(seconds) = resolve_openai_video_seconds(route, request)? {
        // OpenAI Sora currently expects a string enum for seconds ("4" | "8" | "12").
        map.insert("seconds".to_string(), Value::String(seconds.to_string()));
    }

    Ok(Value::Object(map))
}

fn resolve_openai_video_seconds(
    route: &ModelRouteKey,
    request: &GenerateRequest,
) -> Result<Option<u64>> {
    let raw_seconds = if let Some(seconds) = request.provider_options.get("seconds") {
        parse_openai_video_seconds_value(seconds)?
    } else if let Some(duration_sec) = request.duration_sec {
        Some(duration_sec)
    } else {
        None
    };

    let Some(raw_seconds) = raw_seconds else {
        return Ok(None);
    };
    if !raw_seconds.is_finite() || raw_seconds <= 0.0 {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            format!(
                "requested video seconds='{raw_seconds}' for model '{}' is invalid; expected a positive value and one of: 4, 8, 12 (e.g. 4)",
                route.model_name
            ),
        )
        .with_provider("openai"));
    }

    let rounded = raw_seconds.round();
    if (raw_seconds - rounded).abs() > 0.0001 {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            format!(
                "requested video seconds='{raw_seconds}' for model '{}' is unsupported; OpenAI Sora accepts one of: 4, 8, 12 (try 4)",
                route.model_name
            ),
        )
        .with_provider("openai"));
    }
    let seconds = rounded as i64;
    if seconds <= 0 {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            format!(
                "requested video seconds='{raw_seconds}' for model '{}' is invalid; expected one of: 4, 8, 12 (e.g. 4)",
                route.model_name
            ),
        )
        .with_provider("openai"));
    }

    let seconds = seconds as u64;
    if !OPENAI_SORA_ALLOWED_SECONDS.contains(&seconds) {
        return Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            format!(
                "requested video seconds='{seconds}' for model '{}' is unsupported; OpenAI Sora accepts one of: 4, 8, 12 (try 4)",
                route.model_name
            ),
        )
        .with_provider("openai"));
    }

    Ok(Some(seconds))
}

fn parse_openai_video_seconds_value(value: &Value) -> Result<Option<f64>> {
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
                        "provider_options.seconds='{trimmed}' is not numeric; use one of: 4, 8, 12"
                    ),
                )
                .with_provider("openai")
            })?;
            Ok(Some(parsed))
        }
        Value::Bool(_) | Value::Array(_) | Value::Object(_) => Err(ProtocolError::new(
            ErrorCode::InvalidArgument,
            "provider_options.seconds must be a number/string; use one of: 4, 8, 12",
        )
        .with_provider("openai")),
    }
}

fn default_size_for_aspect_ratio(request: &GenerateRequest) -> String {
    match request.aspect_ratio.as_deref() {
        Some("16:9") => "1536x1024".to_string(),
        Some("9:16") => "1024x1536".to_string(),
        _ => "1024x1024".to_string(),
    }
}

fn map_openai_http_error_with_operation(
    status: u16,
    body_text: &str,
    operation: &str,
) -> ProtocolError {
    let parsed = serde_json::from_str::<Value>(body_text).ok();
    let (provider_code, provider_type, provider_message) = parsed
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(Value::as_object)
        .map(|err| {
            let code = err
                .get("code")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let typ = err
                .get("type")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            (code, typ, msg)
        })
        .unwrap_or((None, None, None));

    let code_text = provider_code.clone().unwrap_or_default();
    let type_text = provider_type.clone().unwrap_or_default();
    let is_content_policy = code_text.contains("content_policy")
        || type_text.contains("content_policy")
        || body_text.contains("content_policy");
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

    let mut msg = String::new();
    let _ = write!(&mut msg, "openai {operation} failed ({status})");
    if let Some(pmsg) = provider_message {
        let _ = write!(&mut msg, ": {pmsg}");
    } else if !body_text.trim().is_empty() {
        let _ = write!(&mut msg, ": {}", body_text.trim());
    }
    if operation.contains("image") && body_text.contains("Value must be 'dall-e-2'") {
        let _ = write!(
            &mut msg,
            " (OpenAI accepted only dall-e-2 for this edit call; verify org/project access for GPT image edits)"
        );
    }

    let mut err = ProtocolError::new(error_code, msg)
        .with_provider("openai")
        .with_http_status(status);
    if let Some(pc) = provider_code {
        err = err.with_provider_code(pc);
    }
    if status == 429 || (500..=599).contains(&status) {
        err = err.retriable(true);
    }
    if let Some(parsed) = parsed {
        err = err.with_details(parsed);
    }
    err
}

fn parse_openai_video_status(payload: &Value) -> Option<JobStatus> {
    let status = payload.get("status").and_then(Value::as_str)?.trim();
    if status.is_empty() {
        return None;
    }
    let lowered = status.to_ascii_lowercase();
    match lowered.as_str() {
        "queued" | "pending" => Some(JobStatus::Queued),
        "running" | "in_progress" | "processing" => Some(JobStatus::Running),
        "succeeded" | "completed" | "complete" => Some(JobStatus::Succeeded),
        "failed" | "error" => Some(JobStatus::Failed),
        "canceled" | "cancelled" => Some(JobStatus::Canceled),
        _ => Some(JobStatus::Running),
    }
}

fn parse_openai_video_job_error(payload: &Value) -> Option<ProtocolError> {
    let error_obj = payload.get("error")?.as_object()?;
    let provider_message = error_obj
        .get("message")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| "openai video job failed".to_string());
    let provider_code = error_obj
        .get("code")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let provider_type = error_obj
        .get("type")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    let provider_code_text = provider_code
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let provider_type_text = provider_type
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let message_text = provider_message.to_ascii_lowercase();
    let error_code = if provider_code_text.contains("content_policy")
        || provider_type_text.contains("content_policy")
        || message_text.contains("content policy")
    {
        ErrorCode::ContentPolicyViolation
    } else {
        ErrorCode::JobFailed
    };

    let mut err = ProtocolError::new(error_code, provider_message).with_provider("openai");
    if let Some(provider_code) = provider_code {
        err = err.with_provider_code(provider_code);
    }
    Some(err)
}

fn parse_openai_video_dimensions(payload: &Value) -> (Option<u32>, Option<u32>) {
    let width = payload
        .get("width")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok());
    let height = payload
        .get("height")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok());
    if width.is_some() && height.is_some() {
        return (width, height);
    }

    if let Some(size_text) = payload.get("size").and_then(Value::as_str)
        && let Some((w, h)) = parse_size_from_str(size_text)
    {
        return (Some(w), Some(h));
    }

    (width, height)
}

fn parse_openai_video_duration_ms(payload: &Value) -> Option<u64> {
    let seconds = payload
        .get("seconds")
        .and_then(Value::as_f64)
        .or_else(|| payload.get("duration").and_then(Value::as_f64))?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    Some((seconds * 1000.0).round() as u64)
}

fn extract_openai_raw_output(item: &Value, hints: &OpenAiOutputHints) -> Result<ProviderRawOutput> {
    let obj = item.as_object().ok_or_else(|| {
        ProtocolError::new(
            ErrorCode::JobFailed,
            "openai data entry is not an object in response",
        )
        .with_provider("openai")
    })?;

    let content_type = infer_content_type(hints, obj);
    let url = obj
        .get("url")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let base64_data = obj
        .get("b64_json")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    if url.is_none() && base64_data.is_none() {
        return Err(ProtocolError::new(
            ErrorCode::JobFailed,
            "openai image response entry has neither `url` nor `b64_json`",
        )
        .with_provider("openai"));
    }

    Ok(ProviderRawOutput {
        content_type,
        url,
        base64_data,
        width: hints.size.map(|v| v.0),
        height: hints.size.map(|v| v.1),
        duration_ms: None,
    })
}

fn infer_content_type(hints: &OpenAiOutputHints, obj: &Map<String, Value>) -> String {
    if let Some(v) = obj.get("mime_type").and_then(Value::as_str) {
        return v.to_string();
    }
    let fmt = hints.output_format.as_deref().unwrap_or("png");
    match fmt {
        "jpg" | "jpeg" => "image/jpeg".to_string(),
        "webp" => "image/webp".to_string(),
        _ => "image/png".to_string(),
    }
}

fn parse_size_from_str(size: &str) -> Option<(u32, u32)> {
    let (w, h) = size.split_once('x')?;
    let w = w.trim().parse::<u32>().ok()?;
    let h = h.trim().parse::<u32>().ok()?;
    Some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{AssetKind, InputAsset};
    use serde_json::json;

    #[test]
    fn supports_configured_openai_models() {
        for m in [
            models::MODEL_GPT_IMAGE_1,
            models::MODEL_GPT_IMAGE_1_MINI,
            models::MODEL_GPT_IMAGE_1_5,
            models::MODEL_SORA_2,
            models::MODEL_SORA_2_PRO,
        ] {
            let route = ModelRouteKey::parse(&format!("openai/{m}")).expect("parse route");
            require_supported_model(&route).expect("model should be supported");
        }
    }

    #[test]
    fn sora_models_require_video_asset_kind() {
        let sora = ModelRouteKey::parse("openai/sora-2").expect("parse route");
        assert!(require_model_supports_asset_kind(&sora, AssetKind::Video).is_ok());
        assert!(require_model_supports_asset_kind(&sora, AssetKind::Image).is_err());

        let sora_pro = ModelRouteKey::parse("openai/sora-2-pro").expect("parse route");
        assert!(require_model_supports_asset_kind(&sora_pro, AssetKind::Video).is_ok());
        assert!(require_model_supports_asset_kind(&sora_pro, AssetKind::Image).is_err());
    }

    #[test]
    fn edit_model_requires_gpt_image_1() {
        let mini = ModelRouteKey::parse("openai/gpt-image-1-mini").expect("parse route");
        assert!(require_model_supports_image_edit(&mini).is_err());

        let one_five = ModelRouteKey::parse("openai/gpt-image-1.5").expect("parse route");
        assert!(require_model_supports_image_edit(&one_five).is_err());

        let one = ModelRouteKey::parse("openai/gpt-image-1").expect("parse route");
        assert!(require_model_supports_image_edit(&one).is_ok());
    }

    #[test]
    fn payload_includes_negative_prompt_when_present() {
        let req = GenerateRequest {
            model: "openai/gpt-image-1".to_string(),
            asset_kind: AssetKind::Image,
            prompt: "hero".to_string(),
            negative_prompt: Some("blurry".to_string()),
            inputs: Vec::new(),
            duration_sec: None,
            aspect_ratio: Some("16:9".to_string()),
            provider_options: Map::new(),
            callback_url: None,
            idempotency_key: None,
            metadata: Map::new(),
        };
        let route = req.route_key().expect("parse route");
        let payload = build_openai_images_payload(&route, &req);
        assert_eq!(
            payload.get("negative_prompt").and_then(Value::as_str),
            Some("blurry")
        );
        assert_eq!(
            payload.get("size").and_then(Value::as_str),
            Some("1536x1024")
        );
    }

    #[test]
    fn parse_size_from_str_works() {
        assert_eq!(parse_size_from_str("1024x1536"), Some((1024, 1536)));
    }

    #[test]
    fn has_edit_inputs_detects_image_input() {
        let req = GenerateRequest {
            model: "openai/gpt-image-1".to_string(),
            asset_kind: AssetKind::Image,
            prompt: "hero".to_string(),
            negative_prompt: None,
            inputs: vec![InputAsset {
                kind: InputAssetKind::Image,
                url: "https://example.com/input.png".to_string(),
                role: None,
            }],
            duration_sec: None,
            aspect_ratio: None,
            provider_options: Map::new(),
            callback_url: None,
            idempotency_key: None,
            metadata: Map::new(),
        };
        assert!(has_edit_inputs(&req).expect("validate inputs"));
    }

    #[test]
    fn gpt_image_1_edits_use_image_array_field_name() {
        let route = ModelRouteKey::parse("openai/gpt-image-1").expect("parse route");
        assert_eq!(openai_image_edit_image_field_name(&route, 1), "image[]");
        assert_eq!(openai_image_edit_image_field_name(&route, 2), "image[]");
    }

    #[test]
    fn normalize_gpt_image_size_from_arbitrary_canvas() {
        let route = ModelRouteKey::parse("openai/gpt-image-1").expect("parse route");
        assert_eq!(normalize_size_for_model(&route, "1920x1080"), "1536x1024");
        assert_eq!(normalize_size_for_model(&route, "1080x1920"), "1024x1536");
        assert_eq!(normalize_size_for_model(&route, "1000x1000"), "1024x1024");
    }

    #[test]
    fn video_payload_rejects_inputs_until_supported() {
        let route = ModelRouteKey::parse("openai/sora-2").expect("parse route");
        let req = GenerateRequest {
            model: "openai/sora-2".to_string(),
            asset_kind: AssetKind::Video,
            prompt: "hero".to_string(),
            negative_prompt: None,
            inputs: vec![InputAsset {
                kind: InputAssetKind::Image,
                url: "file:///tmp/input.png".to_string(),
                role: None,
            }],
            duration_sec: Some(2.0),
            aspect_ratio: None,
            provider_options: Map::new(),
            callback_url: None,
            idempotency_key: None,
            metadata: Map::new(),
        };

        assert!(build_openai_videos_payload(&route, &req).is_err());
    }

    #[test]
    fn video_payload_rejects_conflicting_seconds_sources() {
        let route = ModelRouteKey::parse("openai/sora-2").expect("parse route");
        let mut provider_options = Map::new();
        provider_options.insert("seconds".to_string(), json!(4));
        let req = GenerateRequest {
            model: "openai/sora-2".to_string(),
            asset_kind: AssetKind::Video,
            prompt: "hero".to_string(),
            negative_prompt: None,
            inputs: Vec::new(),
            duration_sec: Some(2.0),
            aspect_ratio: None,
            provider_options,
            callback_url: None,
            idempotency_key: None,
            metadata: Map::new(),
        };

        assert!(build_openai_videos_payload(&route, &req).is_err());
    }

    #[test]
    fn video_payload_rejects_unsupported_seconds() {
        let route = ModelRouteKey::parse("openai/sora-2").expect("parse route");
        let req = GenerateRequest {
            model: "openai/sora-2".to_string(),
            asset_kind: AssetKind::Video,
            prompt: "hero".to_string(),
            negative_prompt: None,
            inputs: Vec::new(),
            duration_sec: Some(2.0),
            aspect_ratio: None,
            provider_options: Map::new(),
            callback_url: None,
            idempotency_key: None,
            metadata: Map::new(),
        };

        let err = build_openai_videos_payload(&route, &req).expect_err("must reject 2 seconds");
        assert_eq!(err.code, ErrorCode::InvalidArgument);
        assert!(err.message.contains("one of: 4, 8, 12"));
    }

    #[test]
    fn video_payload_accepts_supported_seconds() {
        let route = ModelRouteKey::parse("openai/sora-2-pro").expect("parse route");
        let req = GenerateRequest {
            model: "openai/sora-2-pro".to_string(),
            asset_kind: AssetKind::Video,
            prompt: "hero".to_string(),
            negative_prompt: None,
            inputs: Vec::new(),
            duration_sec: Some(4.0),
            aspect_ratio: None,
            provider_options: Map::new(),
            callback_url: None,
            idempotency_key: None,
            metadata: Map::new(),
        };

        let payload = build_openai_videos_payload(&route, &req).expect("payload should build");
        assert_eq!(payload.get("seconds").and_then(Value::as_str), Some("4"));
    }

    #[test]
    fn parse_openai_video_status_maps_known_values() {
        assert_eq!(
            parse_openai_video_status(&json!({ "status": "queued" })),
            Some(JobStatus::Queued)
        );
        assert_eq!(
            parse_openai_video_status(&json!({ "status": "in_progress" })),
            Some(JobStatus::Running)
        );
        assert_eq!(
            parse_openai_video_status(&json!({ "status": "completed" })),
            Some(JobStatus::Succeeded)
        );
        assert_eq!(
            parse_openai_video_status(&json!({ "status": "failed" })),
            Some(JobStatus::Failed)
        );
        assert_eq!(
            parse_openai_video_status(&json!({ "status": "canceled" })),
            Some(JobStatus::Canceled)
        );
    }
}
