pub mod block_max;
pub mod encoder;
pub mod index;

pub use block_max::{Block, BlockMaxIndex, DocId, Posting, ScoredDoc};
pub use encoder::{SparseVec, SpladeEncoder};
pub use index::{SearchHit, SpladeIndex};

#[cfg(feature = "splade-onnx")]
pub use encoder::OnnxSpladeEncoder;
