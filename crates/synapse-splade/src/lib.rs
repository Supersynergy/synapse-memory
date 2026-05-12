pub mod block_max;
pub mod encoder;
pub mod index;

pub use block_max::{Block, BlockMaxIndex, DocId};
pub use encoder::{SparseVec, SpladeEncoder};
pub use index::SpladeIndex;

#[cfg(feature = "splade-onnx")]
pub use encoder::OnnxSpladeEncoder;
