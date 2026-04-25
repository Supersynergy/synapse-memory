//! MLX Metal embedder — Phase 5 implementation (IPC sidecar).
//!
//! Spawns a persistent Python sidecar (`scripts/synapse-mlx-embed.py`) that
//! loads BGE-small via `mlx-embeddings` and serves embed requests over
//! length-prefixed msgpack on stdin/stdout.
//!
//! # Why a sidecar?
//! Native `mlx-rs` Rust bindings are still nascent (no published BGE/BERT
//! reference impl as of 2026-04). A persistent sidecar gives us Metal-class
//! latency today while keeping the door open to swap in pure-Rust MLX once
//! the bindings mature — the [`TextEmbedder`] trait is the seam.
//!
//! # Performance (M4 Max, mlx-community/bge-small-en-v1.5-bf16)
//! - Single embed:  p50 ~4.3ms (parity with fastembed CPU at single-doc)
//! - Batch 32:      p50 ~7.1ms total → **0.22ms/doc**, ~30× CPU
//! - Batch 64:      p50 ~10.3ms total → **0.16ms/doc**, ~40× CPU
//!
//! See `bench/results/2026-04-25/mlx-embedder-impl.md` for full table.
//!
//! # Configuration
//! - `SYNAPSE_MLX_PYTHON` — python interpreter path (default: `python3`).
//! - `SYNAPSE_MLX_SCRIPT` — sidecar script path (default: repo `scripts/synapse-mlx-embed.py`).
//! - `SYNAPSE_MLX_MODEL`  — HF model id (default: `mlx-community/bge-small-en-v1.5-bf16`).
//!
//! # Status
//! Feature-flagged `embed-mlx`. **Not** wired into `pick_embedder()` defaults
//! pending a model-fidelity fix (current bf16 weights diverge ~9% cosine from
//! BAAI fp32; needs re-conversion or bf16-from-fp32 download).

#![cfg(all(target_os = "macos", target_arch = "aarch64", feature = "embed-mlx"))]

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use crate::embedder_trait::TextEmbedder;
use crate::error::{Error, Result};

const DEFAULT_SCRIPT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../scripts/synapse-mlx-embed.py",
);

/// Persistent MLX sidecar handle.
struct Sidecar {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

impl Sidecar {
    fn spawn() -> Result<Self> {
        let py = std::env::var("SYNAPSE_MLX_PYTHON")
            .unwrap_or_else(|_| "python3".to_string());
        let script = std::env::var("SYNAPSE_MLX_SCRIPT")
            .unwrap_or_else(|_| DEFAULT_SCRIPT.to_string());

        let mut child = Command::new(&py)
            .arg(&script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| Error::Other(format!("mlx sidecar spawn ({py} {script}): {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Other("mlx sidecar: no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Other("mlx sidecar: no stdout".into()))?;

        let mut s = Self { child, stdin, stdout };

        let ready = s.read_msg()?;
        let ready_ok = match &ready {
            rmpv::Value::Map(m) => m.iter().any(|(k, v)| {
                k.as_str() == Some("ready") && v.as_bool() == Some(true)
            }),
            _ => false,
        };
        if !ready_ok {
            return Err(Error::Other(format!(
                "mlx sidecar handshake failed: {ready:?}"
            )));
        }
        Ok(s)
    }

    fn read_msg(&mut self) -> Result<rmpv::Value> {
        // CRIT-2: bound msgpack frame size — untrusted u32 length prefix
        // could otherwise force a 4 GiB allocation (DoS via crafted sidecar
        // response or compromised script). 256 MiB is far above any
        // legitimate batch (BGE-small @ batch=1024 ≈ 1.5 MiB).
        const MAX_FRAME: usize = 256 * 1024 * 1024;

        let mut hdr = [0u8; 4];
        self.stdout
            .read_exact(&mut hdr)
            .map_err(|e| Error::Other(format!("mlx read hdr: {e}")))?;
        let n = u32::from_be_bytes(hdr) as usize;
        if n == 0 {
            return Err(Error::Other("mlx empty frame (n=0)".into()));
        }
        if n > MAX_FRAME {
            return Err(Error::Other(format!(
                "oversized embed frame: {n} bytes (max {MAX_FRAME})"
            )));
        }
        let mut buf = vec![0u8; n];
        self.stdout
            .read_exact(&mut buf)
            .map_err(|e| Error::Other(format!("mlx read body: {e}")))?;
        rmpv::decode::read_value(&mut &buf[..])
            .map_err(|e| Error::Other(format!("mlx msgpack decode: {e}")))
    }

    fn write_msg(&mut self, texts: &[String]) -> Result<()> {
        let mut payload = Vec::new();
        let val = rmpv::Value::Map(vec![(
            rmpv::Value::String("texts".into()),
            rmpv::Value::Array(
                texts
                    .iter()
                    .map(|t| rmpv::Value::String(t.clone().into()))
                    .collect(),
            ),
        )]);
        rmpv::encode::write_value(&mut payload, &val)
            .map_err(|e| Error::Other(format!("mlx msgpack encode: {e}")))?;
        let n = (payload.len() as u32).to_be_bytes();
        self.stdin
            .write_all(&n)
            .map_err(|e| Error::Other(format!("mlx write hdr: {e}")))?;
        self.stdin
            .write_all(&payload)
            .map_err(|e| Error::Other(format!("mlx write body: {e}")))?;
        self.stdin
            .flush()
            .map_err(|e| Error::Other(format!("mlx flush: {e}")))?;
        Ok(())
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// MLX Metal embedding backend (BGE-small via persistent Python sidecar).
pub struct MlxMetalEmbedder {
    sidecar: Mutex<Sidecar>,
    name: String,
    dim: usize,
}

impl MlxMetalEmbedder {
    /// Spawn the sidecar and wait for the ready handshake.
    ///
    /// # Errors
    /// Returns `Error::Other` if Python is missing, the script path is wrong,
    /// or the model fails to load.
    pub fn new() -> Result<Self> {
        let sidecar = Sidecar::spawn()?;
        Ok(Self {
            sidecar: Mutex::new(sidecar),
            name: "mlx-metal:bge-small-en-v1.5-bf16".to_string(),
            dim: 384,
        })
    }

    /// Model name as reported to the trait.
    pub fn backend_name(&self) -> &str {
        &self.name
    }

    /// Output dimensionality (BGE-small = 384).
    pub fn backend_dim(&self) -> usize {
        self.dim
    }
}

impl TextEmbedder for MlxMetalEmbedder {
    fn name(&self) -> &str {
        &self.name
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut guard = self
            .sidecar
            .lock()
            .map_err(|_| Error::Other("mlx sidecar mutex poisoned".into()))?;
        guard.write_msg(texts)?;
        let resp = guard.read_msg()?;

        let map = match resp {
            rmpv::Value::Map(m) => m,
            other => return Err(Error::Other(format!("mlx unexpected resp: {other:?}"))),
        };

        for (k, v) in &map {
            if let Some(s) = k.as_str() {
                match s {
                    "error" => {
                        return Err(Error::Other(format!(
                            "mlx sidecar error: {}",
                            v.as_str().unwrap_or("<non-string>")
                        )));
                    }
                    "vecs" => {
                        let arr = match v {
                            rmpv::Value::Array(a) => a,
                            _ => return Err(Error::Other("mlx vecs not array".into())),
                        };
                        let mut out: Vec<Vec<f32>> = Vec::with_capacity(arr.len());
                        for row in arr {
                            let row_arr = match row {
                                rmpv::Value::Array(a) => a,
                                _ => return Err(Error::Other("mlx row not array".into())),
                            };
                            let mut v: Vec<f32> = Vec::with_capacity(row_arr.len());
                            for n in row_arr {
                                let f = n.as_f64().ok_or_else(|| {
                                    Error::Other("mlx vec elem not float".into())
                                })?;
                                v.push(f as f32);
                            }
                            out.push(v);
                        }
                        return Ok(out);
                    }
                    _ => {}
                }
            }
        }
        Err(Error::Other("mlx response missing vecs/error".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test — only runs when the sidecar is reachable. Skips otherwise.
    #[test]
    #[ignore = "requires Python+mlx-embeddings sidecar; run with --ignored"]
    fn sidecar_roundtrip() {
        let e = MlxMetalEmbedder::new().expect("spawn sidecar");
        let v = e
            .embed_batch(&["hello".to_string(), "world".to_string()])
            .expect("embed");
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].len(), 384);
    }
}
