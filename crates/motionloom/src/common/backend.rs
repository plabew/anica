// =========================================
// =========================================
// crates/motionloom/src/backend.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum OutputFormat {
    Nv12,
    Bgra8,
}

impl OutputFormat {
    pub const fn label(self) -> &'static str {
        match self {
            OutputFormat::Nv12 => "NV12",
            OutputFormat::Bgra8 => "BGRA",
        }
    }
}
