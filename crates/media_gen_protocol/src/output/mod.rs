pub mod normalizer;
pub mod uploader;

pub use normalizer::{ProviderRawOutput, normalize_provider_output};
pub use uploader::{NoopOutputUploader, OutputUploader};
