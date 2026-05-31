use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{ClipboardEntry, ClipboardItem, ImageFormat as GpuiImageFormat};

use crate::core::global_state::AiChatImageAttachment;

static CHAT_IMAGE_COUNTER: AtomicU64 = AtomicU64::new(1);

fn image_extension(format: GpuiImageFormat) -> &'static str {
    match format {
        GpuiImageFormat::Png => "png",
        GpuiImageFormat::Jpeg => "jpg",
        GpuiImageFormat::Webp => "webp",
        GpuiImageFormat::Gif => "gif",
        GpuiImageFormat::Svg => "svg",
        GpuiImageFormat::Bmp => "bmp",
        GpuiImageFormat::Tiff => "tiff",
    }
}

fn detect_image_dimensions(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    image::load_from_memory(bytes)
        .map(|img| (Some(img.width()), Some(img.height())))
        .unwrap_or((None, None))
}

fn save_clipboard_image(image: &gpui::Image) -> Result<AiChatImageAttachment, String> {
    let seq = CHAT_IMAGE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("System clock error: {e}"))?
        .as_millis();
    let ext = image_extension(image.format);
    let dir = std::env::temp_dir().join("anica_ai_chat_images");
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create chat image dir: {e}"))?;
    let path = dir.join(format!("chat_image_{millis}_{seq}.{ext}"));
    fs::write(&path, &image.bytes).map_err(|e| format!("Failed to save chat image: {e}"))?;
    let (width, height) = detect_image_dimensions(&image.bytes);

    Ok(AiChatImageAttachment {
        id: format!("chat_image_{millis}_{seq}"),
        path,
        mime_type: image.format.mime_type().to_string(),
        width,
        height,
        byte_len: image.bytes.len(),
    })
}

pub fn image_path_attachment(path: PathBuf) -> Result<AiChatImageAttachment, String> {
    let metadata = fs::metadata(&path).map_err(|e| format!("Failed to read image path: {e}"))?;
    let bytes = fs::read(&path).unwrap_or_default();
    let (width, height) = detect_image_dimensions(&bytes);
    let mime_type = match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        _ => "application/octet-stream",
    }
    .to_string();

    Ok(AiChatImageAttachment {
        id: path
            .file_stem()
            .and_then(|name| name.to_str())
            .map(ToString::to_string)
            .unwrap_or_else(|| "image_path".to_string()),
        path,
        mime_type,
        width,
        height,
        byte_len: metadata.len() as usize,
    })
}

pub fn save_clipboard_images(item: &ClipboardItem) -> Result<Vec<AiChatImageAttachment>, String> {
    let mut attachments = Vec::new();
    for entry in item.entries() {
        if let ClipboardEntry::Image(image) = entry {
            attachments.push(save_clipboard_image(image)?);
        }
    }
    Ok(attachments)
}

pub fn attachment_display_name(att: &AiChatImageAttachment) -> String {
    att.path
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| att.id.clone())
}

pub fn attachment_size_label(att: &AiChatImageAttachment) -> String {
    match (att.width, att.height) {
        (Some(w), Some(h)) => format!("{w}x{h}"),
        _ => format!("{:.1} KB", att.byte_len as f64 / 1024.0),
    }
}

pub fn append_image_references(prompt: &str, attachments: &[AiChatImageAttachment]) -> String {
    if attachments.is_empty() {
        return prompt.to_string();
    }

    let mut out = prompt.trim().to_string();
    if out.is_empty() {
        out.push_str("Please review the attached image(s).");
    }
    out.push_str("\n\nAttached image files:\n");
    for (idx, att) in attachments.iter().enumerate() {
        let size = attachment_size_label(att);
        out.push_str(&format!(
            "{}. {} ({}, {})\n",
            idx + 1,
            att.path.display(),
            att.mime_type,
            size
        ));
    }
    out.push_str(
        "\nUse these local image paths as visual references if your runtime can inspect image files.",
    );
    out
}

pub fn clipboard_text_path_attachment(item: &ClipboardItem) -> Option<PathBuf> {
    let text = item.text()?;
    let trimmed = text.trim().trim_matches('"');
    let path = PathBuf::from(trimmed);
    path.exists().then_some(path)
}
