pub mod encoder;
pub mod index;

pub use encoder::{SpladeEncoder, SparseVec};
pub use index::SpladeIndex;

#[cfg(feature = "splade-onnx")]
pub use encoder::OnnxSpladeEncoder;
