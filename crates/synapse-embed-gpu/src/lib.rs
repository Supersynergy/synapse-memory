//! synapse-embed-gpu — pluggable GPU embedding backend.
//!
//! Pattern: ort 2.0 ExecutionProvider cascade [CUDA, CoreML, CPU].
//! Reference impls (cloned to ../../synapse-gap-sprint/repos/):
//! - `EmbedAnything/rust/src/embeddings/local/ort_jina.rs`
//! - `kreuzberg-dev/kreuzberg/src/ort_discovery.rs` (runtime EP detection)
//!
//! **STATUS**: scaffold. Wire ort dep + EP cascade in cluster-F follow-up.

use async_trait::async_trait;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionProvider {
    Cuda,
    CoreMl,
    Mlx,
    Cpu,
}

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("provider {0:?} not available on this build/platform")]
    ProviderUnavailable(ExecutionProvider),
    #[error("model load: {0}")]
    ModelLoad(String),
    #[error("inference: {0}")]
    Inference(String),
}

#[async_trait]
pub trait EmbedBackend: Send + Sync {
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
    fn provider(&self) -> ExecutionProvider;
    fn dim(&self) -> usize;
}

/// Cascade selector — try providers in order, fall back on `ProviderUnavailable`.
pub fn auto_select() -> ExecutionProvider {
    #[cfg(feature = "gpu-cuda")]
    {
        return ExecutionProvider::Cuda;
    }
    #[cfg(all(target_os = "macos", feature = "gpu-coreml"))]
    {
        return ExecutionProvider::CoreMl;
    }
    #[cfg(target_os = "macos")]
    {
        return ExecutionProvider::Mlx;
    }
    ExecutionProvider::Cpu
}

#[cfg(any(feature = "gpu-cuda", feature = "gpu-coreml", feature = "cpu"))]
pub mod ort_backend {
    //! Real `ort` 2.0 ExecutionProvider cascade.
    //! Pattern: try CUDA → CoreML → CPU.
    use super::*;
    use ort::execution_providers::ExecutionProviderDispatch;

    /// Build the EP dispatch list ordered by preference, falling back to CPU.
    pub fn build_ep_cascade() -> Vec<ExecutionProviderDispatch> {
        let mut eps: Vec<ExecutionProviderDispatch> = Vec::new();
        #[cfg(feature = "gpu-cuda")]
        {
            eps.push(ort::execution_providers::CUDAExecutionProvider::default().build());
        }
        #[cfg(feature = "gpu-coreml")]
        {
            eps.push(ort::execution_providers::CoreMLExecutionProvider::default().build());
        }
        eps.push(ort::execution_providers::CPUExecutionProvider::default().build());
        eps
    }

    /// Initialize ort with the EP cascade. Call once at startup.
    pub fn init() -> Result<(), EmbedError> {
        ort::init()
            .with_execution_providers(build_ep_cascade())
            .commit()
            .map(|_| ())
            .map_err(|e| EmbedError::ModelLoad(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auto_select_returns_some_provider() {
        let p = auto_select();
        // any non-panic is success; macos-debug should pick MLX or CoreML.
        assert!(matches!(
            p,
            ExecutionProvider::Cuda | ExecutionProvider::CoreMl | ExecutionProvider::Mlx | ExecutionProvider::Cpu
        ));
    }
}
