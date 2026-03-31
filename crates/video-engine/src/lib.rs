// lib.rs

mod error;
mod video;

pub use error::Error;
pub use video::{Position, Video, VideoOptions};

// Re-export commonly used types
pub use gstreamer as gst;
pub use url::Url;
