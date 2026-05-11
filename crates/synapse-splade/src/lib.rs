pub mod encoder;
pub mod index;
pub mod block_max;

pub use encoder::{SpladeEncoder, SparseVec};
pub use index::SpladeIndex;
pub use block_max::{BlockMaxIndex, Block, DocId};

#[cfg(feature = "splade-onnx")]
pub use encoder::OnnxSpladeEncoder;
