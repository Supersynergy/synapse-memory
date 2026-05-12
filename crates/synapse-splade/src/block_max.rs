//! Block-Max Pruning (BMP) index for SPLADE sparse retrieval.
//!
//! Posting lists split into fixed-size blocks (default 128).
//! Each block caches its max per-doc weight. At query time,
//! blocks are skipped when their upper-bound score can't beat heap.min().
//!
//! Score(q,d) = Σ_{t ∈ q∩d} q_weight(t) * d_weight(t)

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use crate::SparseVec;

pub type DocId = u64;

/// One fixed-size block of postings for a single term.
#[derive(Clone)]
pub struct Block {
    /// Maximum document weight in this block (for pruning upper bounds).
    pub max_weight: f32,
    /// (doc_id, doc_weight) pairs — sorted by doc_id for cache locality.
    pub postings: Vec<(DocId, f32)>,
}

impl Block {
    fn from_postings(mut postings: Vec<(DocId, f32)>) -> Self {
        postings.sort_unstable_by_key(|p| p.0);
        let max_weight = postings.iter().map(|p| p.1).fold(0.0_f32, f32::max);
        Self {
            max_weight,
            postings,
        }
    }
}

/// In-memory Block-Max inverted index.
pub struct BlockMaxIndex {
    /// term_id → list of blocks
    terms: HashMap<u32, Vec<Block>>,
    /// Accumulator for pending postings before they fill a block
    pending: HashMap<u32, Vec<(DocId, f32)>>,
    pub block_size: usize,
    doc_count: u64,
}

impl Default for BlockMaxIndex {
    fn default() -> Self {
        Self::new(128)
    }
}

impl BlockMaxIndex {
    pub fn new(block_size: usize) -> Self {
        assert!(block_size > 0);
        Self {
            terms: HashMap::new(),
            pending: HashMap::new(),
            block_size,
            doc_count: 0,
        }
    }

    /// Add a document's sparse vector. Appends to term posting lists;
    /// seals a block when it reaches `block_size`.
    pub fn add_doc(&mut self, doc_id: DocId, sparse: &SparseVec) {
        self.doc_count += 1;
        for (&term_id, &weight) in sparse {
            let buf = self.pending.entry(term_id).or_default();
            buf.push((doc_id, weight));
            if buf.len() >= self.block_size {
                let full = std::mem::take(buf);
                self.terms
                    .entry(term_id)
                    .or_default()
                    .push(Block::from_postings(full));
            }
        }
    }

    /// Flush all pending (partial) postings into blocks.
    /// Call before search for up-to-date results.
    pub fn flush(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        for (term_id, buf) in pending {
            if !buf.is_empty() {
                self.terms
                    .entry(term_id)
                    .or_default()
                    .push(Block::from_postings(buf));
            }
        }
    }

    /// BMP top-k search.
    ///
    /// Algorithm:
    /// 1. For each query term, compute `term_max = q_weight * max(block.max_weight)` across all blocks.
    /// 2. Sort query terms descending by term_max.
    /// 3. Maintain a min-heap of size k (doc_id, score).
    /// 4. For each query term × block: compute upper-bound = accumulated_so_far + q_weight * block.max_weight + remaining_term_max_sum.
    ///    If upper-bound ≤ heap.min() → skip block entirely.
    /// 5. Otherwise accumulate per-doc scores.
    pub fn search_topk(&self, query: &SparseVec, k: usize) -> Vec<(DocId, f32)> {
        if query.is_empty() || k == 0 {
            return vec![];
        }

        // Build sorted query terms: (term_id, q_weight, term_upper_bound)
        // term_upper_bound = q_weight * max block max_weight for this term
        let mut query_terms: Vec<(u32, f32, f32)> = query
            .iter()
            .filter_map(|(&tid, &qw)| {
                let blocks = self.terms.get(&tid)?;
                let term_max = blocks.iter().map(|b| b.max_weight).fold(0.0_f32, f32::max);
                Some((tid, qw, qw * term_max))
            })
            .collect();

        // Sort by term_upper_bound descending (process most impactful terms first)
        query_terms
            .sort_unstable_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

        // Prefix sum of remaining term upper bounds (for skip estimation)
        let n = query_terms.len();
        let mut suffix_max: Vec<f32> = vec![0.0; n + 1];
        for i in (0..n).rev() {
            suffix_max[i] = suffix_max[i + 1] + query_terms[i].2;
        }

        // Score accumulator
        let mut scores: HashMap<DocId, f32> = HashMap::new();

        // Min-heap: Reverse(score_bits, doc_id) — we use ordered_float trick with bits
        // Use BinaryHeap<Reverse<(u32, DocId)>> where u32 = f32::to_bits
        let mut heap: BinaryHeap<Reverse<(u32, DocId)>> = BinaryHeap::new();
        let heap_min = |heap: &BinaryHeap<Reverse<(u32, DocId)>>| -> f32 {
            heap.peek()
                .map(|Reverse((bits, _))| f32::from_bits(*bits))
                .unwrap_or(0.0)
        };

        for (i, &(tid, qw, _)) in query_terms.iter().enumerate() {
            let blocks = match self.terms.get(&tid) {
                Some(b) => b,
                None => continue,
            };

            let remaining = suffix_max[i + 1]; // upper bound from future terms

            for block in blocks {
                // Upper bound for docs in this block, given what we know so far
                // We don't track per-doc accumulated scores pre-block, so we use
                // optimistic: any doc could have full score from all remaining terms
                let block_upper = qw * block.max_weight + remaining;

                if heap.len() >= k && block_upper <= heap_min(&heap) {
                    continue; // skip entire block
                }

                for &(doc_id, dw) in &block.postings {
                    let entry = scores.entry(doc_id).or_insert(0.0);
                    *entry += qw * dw;

                    // Update heap
                    let s = *entry;
                    if heap.len() < k {
                        // Re-insert (heap doesn't support update; we over-insert and dedupe at end)
                        heap.push(Reverse((s.to_bits(), doc_id)));
                    } else if s > heap_min(&heap) {
                        heap.push(Reverse((s.to_bits(), doc_id)));
                        // Trim excess — lazy; keep bounded
                        while heap.len() > k * 4 {
                            heap.pop();
                        }
                    }
                }
            }
        }

        // Final top-k from accumulator (heap may have stale entries)
        let mut ranked: Vec<(DocId, f32)> = scores.into_iter().collect();
        ranked.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(k);
        ranked
    }

    pub fn doc_count(&self) -> u64 {
        self.doc_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SpladeEncoder;

    fn build_index(n_docs: usize) -> BlockMaxIndex {
        let enc = SpladeEncoder::default();
        let mut idx = BlockMaxIndex::default();
        for i in 0..n_docs {
            let text = format!(
                "document number {} with tokens splade neural sparse retrieval",
                i
            );
            let sv = enc.encode(&text).unwrap();
            idx.add_doc(i as DocId, &sv);
        }
        idx.flush();
        idx
    }

    #[test]
    fn rank_equivalence_bmp_vs_naive() {
        use crate::index::SpladeIndex;
        let enc = SpladeEncoder::default();
        let mut naive = SpladeIndex::open(":memory:").unwrap();
        let mut bmp = BlockMaxIndex::default();

        let docs: Vec<(DocId, &str)> = vec![
            (0, "splade neural sparse retrieval model"),
            (1, "dense retrieval bi-encoder sentence"),
            (2, "inverted index posting list BM25"),
            (3, "transformer masked language model BERT"),
            (4, "splade expansion vocabulary terms"),
            (5, "colbert late interaction multi-vector"),
            (6, "sparse representation regularisation"),
            (7, "neural ranking passage reranking"),
            (8, "query expansion pseudo relevance feedback"),
            (9, "MTEB benchmark retrieval recall"),
        ];

        for &(id, text) in &docs {
            let sv = enc.encode(text).unwrap();
            naive.add_doc(id, &sv).unwrap();
            bmp.add_doc(id, &sv);
        }
        bmp.flush();

        let query = enc.encode("splade neural sparse retrieval model").unwrap();
        let k = 5;

        let naive_res = naive.search(&query, k).unwrap();
        let bmp_res = bmp.search_topk(&query, k);

        // Rank equivalence: same doc_ids in same order
        let naive_ids: Vec<DocId> = naive_res.iter().map(|(id, _)| *id).collect();
        let bmp_ids: Vec<DocId> = bmp_res.iter().map(|(id, _)| *id).collect();

        assert_eq!(
            naive_ids, bmp_ids,
            "BMP rank mismatch vs naive\nnaive={naive_ids:?}\nbmp={bmp_ids:?}"
        );
    }

    #[test]
    fn smoke_basic() {
        let idx = build_index(50);
        let enc = SpladeEncoder::default();
        let q = enc.encode("splade neural sparse retrieval").unwrap();
        let res = idx.search_topk(&q, 10);
        assert!(!res.is_empty());
        // scores descending
        for w in res.windows(2) {
            assert!(w[0].1 >= w[1].1);
        }
    }

    #[test]
    fn empty_query() {
        let idx = build_index(10);
        assert!(idx.search_topk(&HashMap::new(), 10).is_empty());
    }

    #[test]
    fn self_match_top() {
        let enc = SpladeEncoder::default();
        let mut idx = BlockMaxIndex::default();
        let sv0 = enc.encode("splade neural sparse unique_token_xyz").unwrap();
        for i in 1..20u64 {
            let sv = enc
                .encode(&format!("unrelated document number {}", i))
                .unwrap();
            idx.add_doc(i, &sv);
        }
        idx.add_doc(0, &sv0);
        idx.flush();
        let res = idx.search_topk(&sv0, 5);
        assert_eq!(res[0].0, 0, "self-match must be top result");
    }
}
