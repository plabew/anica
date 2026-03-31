// =========================================
// =========================================
// src/core/srt.rs
use std::time::Duration;

use thiserror::Error;

#[derive(Debug, Clone)]
pub struct SrtCue {
    pub start: Duration,
    pub end: Duration,
    pub text: String,
}

/// Errors that can occur when parsing an SRT subtitle file.
#[derive(Debug, Error)]
pub enum SrtError {
    #[error("no subtitle cues found in input")]
    NoCuesFound,
}

pub fn parse_srt(input: &str) -> Result<Vec<SrtCue>, SrtError> {
    let mut cues = Vec::new();
    let lines: Vec<&str> = input.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        while i < lines.len() && lines[i].trim().is_empty() {
            i += 1;
        }
        if i >= lines.len() {
            break;
        }

        let mut line = lines[i].trim();
        if let Some(stripped) = line.strip_prefix('\u{feff}') {
            line = stripped.trim();
        }

        let mut time_line = line;
        if !time_line.contains("-->") {
            i += 1;
            if i >= lines.len() {
                break;
            }
            time_line = lines[i].trim();
        }

        let Some((start, end)) = parse_time_range(time_line) else {
            i += 1;
            continue;
        };
        i += 1;

        let mut text_lines = Vec::new();
        while i < lines.len() && !lines[i].trim().is_empty() {
            text_lines.push(lines[i].trim_end_matches('\r').to_string());
            i += 1;
        }

        let text = text_lines.join("\n").trim_end().to_string();
        if text.is_empty() || end <= start {
            continue;
        }

        cues.push(SrtCue { start, end, text });
    }

    if cues.is_empty() {
        Err(SrtError::NoCuesFound)
    } else {
        Ok(cues)
    }
}

fn parse_time_range(line: &str) -> Option<(Duration, Duration)> {
    let mut parts = line.split("-->");
    let start_raw = parts.next()?.trim();
    let end_raw = parts.next()?.trim();

    let start_str = start_raw.split_whitespace().next().unwrap_or(start_raw);
    let end_str = end_raw.split_whitespace().next().unwrap_or(end_raw);

    let start = parse_timestamp(start_str)?;
    let end = parse_timestamp(end_str)?;
    Some((start, end))
}

fn parse_timestamp(input: &str) -> Option<Duration> {
    let trimmed = input.trim();
    let (hms, ms) = if let Some((left, right)) = trimmed.split_once(',') {
        (left, right)
    } else if let Some((left, right)) = trimmed.split_once('.') {
        (left, right)
    } else {
        (trimmed, "0")
    };

    let mut parts = hms.split(':');
    let hours: u64 = parts.next()?.parse().ok()?;
    let minutes: u64 = parts.next()?.parse().ok()?;
    let seconds: u64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }

    let ms_digits: String = ms.chars().take_while(|c| c.is_ascii_digit()).collect();
    let ms_value = if ms_digits.is_empty() {
        0
    } else {
        let raw: u64 = ms_digits.parse().ok()?;
        match ms_digits.len() {
            1 => raw * 100,
            2 => raw * 10,
            _ => ms_digits[..3].parse().ok()?,
        }
    };

    let total_ms = hours
        .saturating_mul(3_600_000)
        .saturating_add(minutes.saturating_mul(60_000))
        .saturating_add(seconds.saturating_mul(1_000))
        .saturating_add(ms_value);
    Some(Duration::from_millis(total_ms))
}
