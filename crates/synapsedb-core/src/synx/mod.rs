//! `.synx` file format v2 — next-generation agent memory container.
//!
//! v0.2 surface:
//!   - header/footer binary layout
//!   - append-only content-addressed chunks
//!   - JSON-archived manifest (rkyv migration TBD)
//!   - buffered writer + reader roundtrip
//!   - `mmap` feature: zero-copy reader (Phase 3 Track b)
//!   - `fts-tantivy` feature: Tantivy BM25 index
//!   - `vec-hnsw` feature: HNSW kNN + scalar-quant codebook
//!   - migrate path from v1 SQLite
//!   - temporal KG edges + memory scopes (mem0 / Graphiti parity)
//!
//! See `docs/SYNX-FORMAT-V2.md` for the specification.

pub mod chunk;
pub mod fts;
pub mod header;
pub mod kg;
pub mod manifest;
pub mod migrate;
pub mod mmap;
pub mod reader;
pub mod sign;
pub mod vec_index;
pub mod writer;

pub use chunk::{Chunk, ChunkKind, Codec};
pub use fts::FtsIndex;
pub use header::{SynxFooter, SynxHeader, FOOTER_MAGIC, MAGIC, VERSION};
pub use kg::{Edge, EdgeKind, EdgeSet, Scope};
pub use manifest::{ChunkRef, Manifest, SchemaVersion};
pub use mmap::MmapReader;
pub use reader::SynxReader;
pub use vec_index::{HnswIndex, ScalarCodebook};
pub use writer::SynxWriter;
