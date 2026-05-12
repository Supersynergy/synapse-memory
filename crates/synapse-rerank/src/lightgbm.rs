// Requires LightGBM C library: brew install lightgbm
// Build with: LIGHTGBM_LIB_DIR=/opt/homebrew/lib cargo check -p synapse-rerank --features lightgbm

use crate::Reranker;
use anyhow::Result;
use lightgbm3::Booster;
use std::path::Path;
use std::sync::Mutex;
use synapse_core::Hit;

pub struct LightGbmReranker {
    booster: Mutex<Booster>,
}

// SAFETY: Booster wraps a C pointer; we serialize access via Mutex.
unsafe impl Send for LightGbmReranker {}
unsafe impl Sync for LightGbmReranker {}

impl LightGbmReranker {
    pub fn load(path: &Path) -> Result<Self> {
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-UTF8 path"))?;
        let booster = Booster::from_file(path_str)?;
        Ok(Self {
            booster: Mutex::new(booster),
        })
    }
}

fn extract_features(query: &str, hit: &Hit) -> [f32; 6] {
    let vec_score = hit.score as f32;
    // FTS score not stored separately in Hit; approximate from score.
    let fts_score = hit.score as f32;
    // No timestamp in Hit; default to neutral 365 days.
    let recency_days: f32 = 365.0;
    let title_exact_match = hit
        .title
        .as_deref()
        .map(|t| {
            if t.to_lowercase().contains(&query.to_lowercase()) {
                1.0_f32
            } else {
                0.0
            }
        })
        .unwrap_or(0.0);
    let path_depth = hit
        .uri
        .as_deref()
        .map(|u| u.matches('/').count() as f32)
        .unwrap_or(3.0);
    let doc_len_log = (hit.text.len() as f32 + 1.0).ln();

    [
        vec_score,
        fts_score,
        recency_days,
        title_exact_match,
        path_depth,
        doc_len_log,
    ]
}

impl Reranker for LightGbmReranker {
    fn rerank(&self, query: &str, mut candidates: Vec<Hit>, top_k: usize) -> Result<Vec<Hit>> {
        if candidates.is_empty() {
            return Ok(candidates);
        }

        let n_features = 6_i32;
        let mut flat: Vec<f32> = Vec::with_capacity(candidates.len() * 6);
        for hit in &candidates {
            flat.extend_from_slice(&extract_features(query, hit));
        }

        let scores = self.booster.lock().unwrap().predict(
            flat.as_slice(),
            n_features,
            true, // is_row_major
        )?;

        for (hit, &lgb_score) in candidates.iter_mut().zip(scores.iter()) {
            hit.score = crate::blend(lgb_score as f64, hit.score);
        }

        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates.truncate(top_k);
        Ok(candidates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(id: i64, score: f64, text: &str) -> Hit {
        Hit {
            id,
            uri: None,
            title: None,
            text: text.into(),
            score,
        }
    }

    #[test]
    fn lgbm_load_and_score() {
        let model_path = std::path::PathBuf::from(
            std::env::var("HOME").unwrap_or_default() + "/.synapse/rerank.lgb",
        );
        if !model_path.exists() {
            eprintln!("Skipping: model not found at {:?}", model_path);
            return;
        }
        let reranker = LightGbmReranker::load(&model_path).expect("load model");
        let candidates = vec![
            h(1, 0.9, "synapse search engine fast retrieval"),
            h(2, 0.7, "lightgbm gradient boosting"),
            h(3, 0.5, "random unrelated text about cooking"),
            h(4, 0.6, "rust async tokio runtime"),
            h(5, 0.4, "vector similarity cosine distance"),
        ];
        let out = reranker.rerank("search engine", candidates, 3).unwrap();
        assert_eq!(out.len(), 3);
        // scores should be descending
        assert!(out[0].score >= out[1].score);
        assert!(out[1].score >= out[2].score);
    }
}
