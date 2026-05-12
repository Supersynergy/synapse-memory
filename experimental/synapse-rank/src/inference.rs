#[derive(Debug, Clone)]
pub struct Features {
    pub bm25_score: f32,
    pub vec_score: f32,
    pub rank: f32,
    pub score: f32,
}

impl Features {
    pub fn as_slice(&self) -> [f32; 4] {
        [self.bm25_score, self.vec_score, self.rank, self.score]
    }
}

/// Rerank candidates. Requires `lightgbm-native` feature + LightGBM shared lib.
/// Returns scores in same order as input.
#[cfg(feature = "lightgbm-native")]
pub fn rerank(features: &[Features], model_path: &str) -> anyhow::Result<Vec<f32>> {
    use lightgbm::{Booster};
    let booster = Booster::from_file(model_path)?;
    let flat: Vec<f64> = features
        .iter()
        .flat_map(|f| f.as_slice().map(|v| v as f64))
        .collect();
    let n = features.len();
    let preds = booster.predict(flat.as_slice(), n as i64, 4)?;
    Ok(preds.iter().map(|&v| v as f32).collect())
}

/// Fallback when native LightGBM not compiled in.
#[cfg(not(feature = "lightgbm-native"))]
pub fn rerank(_features: &[Features], _model_path: &str) -> anyhow::Result<Vec<f32>> {
    anyhow::bail!(
        "rerank requires `lightgbm-native` feature or call synapse-rank-train then load scores manually"
    )
}
