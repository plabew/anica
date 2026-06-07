use serde::{Deserialize, Serialize};

fn default_wrap() -> String {
    "normal".to_string()
}

fn default_overflow() -> String {
    "clip".to_string()
}

fn default_align() -> String {
    "left".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextWrapMode {
    None,
    Normal,
    Balance,
}

impl TextWrapMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "none" => Some(Self::None),
            "normal" => Some(Self::Normal),
            "balance" => Some(Self::Balance),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOverflowMode {
    Clip,
    Fit,
    Ellipsis,
}

impl TextOverflowMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "clip" => Some(Self::Clip),
            "fit" => Some(Self::Fit),
            "ellipsis" => Some(Self::Ellipsis),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlignMode {
    Left,
    Center,
    Right,
}

impl TextAlignMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "left" => Some(Self::Left),
            "center" => Some(Self::Center),
            "right" => Some(Self::Right),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextLayoutNode {
    #[serde(default = "default_wrap")]
    pub wrap: String,
    #[serde(default = "default_overflow")]
    pub overflow: String,
    #[serde(default)]
    pub safe_area: Option<String>,
    #[serde(default)]
    pub max_lines: Option<String>,
    #[serde(default)]
    pub align: Option<String>,
}

impl Default for TextLayoutNode {
    fn default() -> Self {
        Self {
            wrap: default_wrap(),
            overflow: default_overflow(),
            safe_area: None,
            max_lines: None,
            align: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTextLayoutSpec {
    pub wrap: TextWrapMode,
    pub overflow: TextOverflowMode,
    pub align: TextAlignMode,
    pub safe_area: Option<[f32; 4]>,
}

impl ResolvedTextLayoutSpec {
    pub fn from_parts(
        base_align: Option<&str>,
        layout: Option<&TextLayoutNode>,
    ) -> Result<Self, String> {
        let wrap_raw = layout
            .map(|layout| layout.wrap.as_str())
            .unwrap_or("normal");
        let overflow_raw = layout
            .map(|layout| layout.overflow.as_str())
            .unwrap_or("clip");
        let align_raw = layout
            .and_then(|layout| layout.align.as_deref())
            .or(base_align)
            .unwrap_or(default_align().as_str())
            .to_string();

        let wrap = TextWrapMode::parse(wrap_raw)
            .ok_or_else(|| format!("invalid TextLayout.wrap '{wrap_raw}'"))?;
        let overflow = TextOverflowMode::parse(overflow_raw)
            .ok_or_else(|| format!("invalid TextLayout.overflow '{overflow_raw}'"))?;
        let align = TextAlignMode::parse(&align_raw)
            .ok_or_else(|| format!("invalid Text align '{align_raw}'"))?;
        let safe_area = layout
            .and_then(|layout| layout.safe_area.as_deref())
            .map(parse_safe_area)
            .transpose()?;

        Ok(Self {
            wrap,
            overflow,
            align,
            safe_area,
        })
    }
}

pub fn parse_safe_area(raw: &str) -> Result<[f32; 4], String> {
    let mut values = [0.0_f32; 4];
    let mut count = 0usize;
    for part in raw.split(',') {
        if count >= 4 {
            return Err(format!(
                "safeArea expects 4 comma-separated numbers, got '{raw}'"
            ));
        }
        values[count] = part
            .trim()
            .parse::<f32>()
            .map_err(|_| format!("invalid safeArea number '{}'", part.trim()))?;
        count += 1;
    }
    if count != 4 {
        return Err(format!(
            "safeArea expects 4 comma-separated numbers, got '{raw}'"
        ));
    }
    Ok(values)
}
