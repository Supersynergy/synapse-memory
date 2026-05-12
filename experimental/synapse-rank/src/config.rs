use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankConfig {
    pub num_trees: u32,
    pub learning_rate: f64,
    pub max_depth: i32,
    pub num_leaves: i32,
    pub eval_metric: String,
    pub objective: String,
    pub label_gain: Vec<f64>,
    pub model_path: String,
    pub libsvm_path: String,
}

impl Default for RankConfig {
    fn default() -> Self {
        Self {
            num_trees: 300,
            learning_rate: 0.05,
            max_depth: 8,
            num_leaves: 31,
            eval_metric: "ndcg@10".into(),
            objective: "lambdarank".into(),
            label_gain: vec![0.0, 1.0],
            model_path: "model.lgbm".into(),
            libsvm_path: "train.libsvm".into(),
        }
    }
}
