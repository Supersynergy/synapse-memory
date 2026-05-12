use crate::config::RankConfig;
use anyhow::Result;
use std::process::Command;

/// Train via python subprocess: `lightgbm train config.json`
/// Generates `model.lgbm` at cfg.model_path.
pub fn train_via_python(cfg: &RankConfig) -> Result<()> {
    // Write a minimal LightGBM config file
    let conf_path = format!("{}.train_conf.txt", cfg.model_path);
    let conf_content = format!(
        "task = train\n\
         objective = {objective}\n\
         metric = {metric}\n\
         num_leaves = {num_leaves}\n\
         max_depth = {max_depth}\n\
         learning_rate = {lr}\n\
         num_iterations = {trees}\n\
         data = {data}\n\
         output_model = {model}\n\
         label_gain = {label_gain}\n\
         verbosity = 1\n",
        objective = cfg.objective,
        metric = cfg.eval_metric,
        num_leaves = cfg.num_leaves,
        max_depth = cfg.max_depth,
        lr = cfg.learning_rate,
        trees = cfg.num_trees,
        data = cfg.libsvm_path,
        model = cfg.model_path,
        label_gain = cfg
            .label_gain
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(","),
    );
    std::fs::write(&conf_path, conf_content)?;

    let status = Command::new("lightgbm").arg(&conf_path).status();
    match status {
        Ok(s) if s.success() => {
            tracing::info!("LightGBM training done → {}", cfg.model_path);
            Ok(())
        }
        Ok(s) => anyhow::bail!("lightgbm exited with {}", s),
        Err(e) => anyhow::bail!(
            "lightgbm binary not found ({e}). Install: pip install lightgbm && \
             ln -s $(python -c 'import lightgbm,os;print(os.path.dirname(lightgbm.__file__))') \
             or brew install lightgbm"
        ),
    }
}

/// Native train path (feature-gated).
#[cfg(feature = "lightgbm-native")]
pub fn train_native(_cfg: &RankConfig) -> Result<()> {
    // lightgbm-rs 0.4 does not expose a high-level train API yet.
    // Wire via Dataset + train_with_params when upstream adds it.
    anyhow::bail!("native train not yet wired; use train_via_python")
}
