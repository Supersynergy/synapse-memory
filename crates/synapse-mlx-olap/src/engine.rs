use crate::agg::AggOp;
use crate::batch::{Column, RecordBatch};
use anyhow::Result;
use std::collections::HashMap;

/// Backend in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// candle-core Metal GPU (M-Series).
    Metal,
    /// Pure-Rust CPU columnar fallback.
    Cpu,
}

/// MLX OLAP engine.
///
/// With `mlx-olap` feature and Metal device available → GPU path.
/// Otherwise → CPU columnar fallback (zero deps, always compiles).
pub struct MlxOlapEngine {
    pub backend: Backend,
    #[cfg(feature = "mlx-olap")]
    device: candle_core::Device,
}

impl MlxOlapEngine {
    /// Create engine. Tries Metal first, falls back to CPU.
    pub fn new() -> Result<Self> {
        #[cfg(feature = "mlx-olap")]
        {
            match candle_core::Device::new_metal(0) {
                Ok(device) => {
                    return Ok(Self { backend: Backend::Metal, device });
                }
                Err(e) => {
                    tracing::warn!("Metal init failed ({e}), falling back to CPU");
                }
            }
        }
        Ok(Self {
            backend: Backend::Cpu,
            #[cfg(feature = "mlx-olap")]
            device: candle_core::Device::Cpu,
        })
    }

    /// Force CPU backend (testing / non-Mac).
    pub fn cpu() -> Result<Self> {
        Ok(Self {
            backend: Backend::Cpu,
            #[cfg(feature = "mlx-olap")]
            device: candle_core::Device::Cpu,
        })
    }

    /// Execute aggregation over `value_col`.
    /// Optional `group_by` column name: if Some → grouped output, else scalar.
    pub fn execute_agg(
        &self,
        batch: &RecordBatch,
        agg: AggOp,
        value_col: &str,
        group_by: Option<&str>,
    ) -> Result<RecordBatch> {
        #[cfg(feature = "mlx-olap")]
        if self.backend == Backend::Metal {
            return self.execute_agg_metal(batch, agg, value_col, group_by);
        }
        self.execute_agg_cpu(batch, agg, value_col, group_by)
    }

    // ── Metal GPU path ────────────────────────────────────────────────────────

    #[cfg(feature = "mlx-olap")]
    fn execute_agg_metal(
        &self,
        batch: &RecordBatch,
        agg: AggOp,
        value_col: &str,
        group_by: Option<&str>,
    ) -> Result<RecordBatch> {
        use candle_core::Tensor;

        let values = self.col_to_f64(batch, value_col)?;
        let n = values.len();

        if group_by.is_none() {
            // Scalar agg — push full column to Metal tensor
            let t = Tensor::from_slice(&values, n, &self.device)?;
            let result = match agg {
                AggOp::Sum => t.sum_all()?.to_scalar::<f64>()?,
                AggOp::Avg => t.mean_all()?.to_scalar::<f64>()?,
                AggOp::Count => n as f64,
                AggOp::Min => {
                    // candle min_all not available → fallback scalar
                    values.iter().cloned().fold(f64::INFINITY, f64::min)
                }
                AggOp::Max => {
                    values.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
                }
            };
            return RecordBatch::new(
                vec![format!("{agg:?}({value_col})")],
                vec![Column::Float(vec![result])],
            );
        }

        // GROUP BY — segment per-group on CPU, agg each segment on Metal
        let keys = self.col_to_strs(batch, group_by.unwrap())?;
        let mut groups: HashMap<String, Vec<f64>> = HashMap::new();
        for (k, v) in keys.iter().zip(values.iter()) {
            groups.entry(k.clone()).or_default().push(*v);
        }

        let mut group_keys: Vec<String> = Vec::with_capacity(groups.len());
        let mut results: Vec<f64> = Vec::with_capacity(groups.len());
        let mut sorted_keys: Vec<String> = groups.keys().cloned().collect();
        sorted_keys.sort();

        for key in &sorted_keys {
            let vals = &groups[key];
            let m = vals.len();
            let t = Tensor::from_slice(vals.as_slice(), m, &self.device)?;
            let agg_val = match agg {
                AggOp::Sum => t.sum_all()?.to_scalar::<f64>()?,
                AggOp::Avg => t.mean_all()?.to_scalar::<f64>()?,
                AggOp::Count => m as f64,
                AggOp::Min => vals.iter().cloned().fold(f64::INFINITY, f64::min),
                AggOp::Max => vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            };
            group_keys.push(key.clone());
            results.push(agg_val);
        }

        RecordBatch::new(
            vec![group_by.unwrap().to_string(), format!("{agg:?}({value_col})")],
            vec![Column::Str(group_keys), Column::Float(results)],
        )
    }

    // ── CPU fallback ──────────────────────────────────────────────────────────

    fn execute_agg_cpu(
        &self,
        batch: &RecordBatch,
        agg: AggOp,
        value_col: &str,
        group_by: Option<&str>,
    ) -> Result<RecordBatch> {
        let values = self.col_to_f64(batch, value_col)?;

        if group_by.is_none() {
            let result = scalar_agg(agg, &values);
            return RecordBatch::new(
                vec![format!("{agg:?}({value_col})")],
                vec![Column::Float(vec![result])],
            );
        }

        let keys = self.col_to_strs(batch, group_by.unwrap())?;
        let mut groups: HashMap<String, Vec<f64>> = HashMap::new();
        for (k, v) in keys.iter().zip(values.iter()) {
            groups.entry(k.clone()).or_default().push(*v);
        }

        let mut sorted_keys: Vec<String> = groups.keys().cloned().collect();
        sorted_keys.sort();

        let mut group_keys: Vec<String> = Vec::with_capacity(sorted_keys.len());
        let mut results: Vec<f64> = Vec::with_capacity(sorted_keys.len());
        for key in &sorted_keys {
            group_keys.push(key.clone());
            results.push(scalar_agg(agg, &groups[key]));
        }

        RecordBatch::new(
            vec![group_by.unwrap().to_string(), format!("{agg:?}({value_col})")],
            vec![Column::Str(group_keys), Column::Float(results)],
        )
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn col_to_f64(&self, batch: &RecordBatch, name: &str) -> Result<Vec<f64>> {
        let col = batch
            .column_by_name(name)
            .ok_or_else(|| anyhow::anyhow!("column not found: {name}"))?;
        match col {
            Column::Float(v) => Ok(v.clone()),
            Column::Int(v) => Ok(v.iter().map(|&x| x as f64).collect()),
            Column::Str(_) => anyhow::bail!("cannot aggregate string column: {name}"),
        }
    }

    fn col_to_strs(&self, batch: &RecordBatch, name: &str) -> Result<Vec<String>> {
        let col = batch
            .column_by_name(name)
            .ok_or_else(|| anyhow::anyhow!("column not found: {name}"))?;
        match col {
            Column::Str(v) => Ok(v.clone()),
            Column::Int(v) => Ok(v.iter().map(|x| x.to_string()).collect()),
            Column::Float(v) => Ok(v.iter().map(|x| x.to_string()).collect()),
        }
    }
}

impl Default for MlxOlapEngine {
    fn default() -> Self {
        Self::new().expect("engine init")
    }
}

fn scalar_agg(op: AggOp, vals: &[f64]) -> f64 {
    match op {
        AggOp::Count => vals.len() as f64,
        AggOp::Sum => vals.iter().sum(),
        AggOp::Avg => {
            if vals.is_empty() {
                0.0
            } else {
                vals.iter().sum::<f64>() / vals.len() as f64
            }
        }
        AggOp::Min => vals.iter().cloned().fold(f64::INFINITY, f64::min),
        AggOp::Max => vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
    }
}
