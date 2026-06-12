// lib.rs

mod error;
mod video;

pub use error::Error;
pub use video::{Position, Video, VideoOptions};

pub use url::Url;
