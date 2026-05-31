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
use std::sync::mpsc;
use std::time::Duration;

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
        let py = std::env::var("SYNAPSE_MLX_PYTHON").unwrap_or_else(|_| "python3".to_string());
        let script =
            std::env::var("SYNAPSE_MLX_SCRIPT").unwrap_or_else(|_| DEFAULT_SCRIPT.to_string());

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

        let timeout_ms = std::env::var("SYNAPSE_MLX_READY_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(15_000);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut stdout = stdout;
            let ready = Self::read_msg_from(&mut stdout);
            let _ = tx.send((stdout, ready));
        });

        let (stdout, ready) = match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
            Ok(v) => v,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Other(format!(
                    "mlx sidecar ready timeout after {timeout_ms}ms"
                )));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Other(format!("mlx sidecar ready channel: {e}")));
            }
        };

        let ready = ready?;
        let ready_ok = match &ready {
            rmpv::Value::Map(m) => m
                .iter()
                .any(|(k, v)| k.as_str() == Some("ready") && v.as_bool() == Some(true)),
            _ => false,
        };
        if !ready_ok {
            return Err(Error::Other(format!(
                "mlx sidecar handshake failed: {ready:?}"
            )));
        }
        Ok(Self {
            child,
            stdin,
            stdout,
        })
    }

    fn read_msg(&mut self) -> Result<rmpv::Value> {
        Self::read_msg_from(&mut self.stdout)
    }

    fn read_msg_from(stdout: &mut ChildStdout) -> Result<rmpv::Value> {
        // CRIT-2: bound msgpack frame size — untrusted u32 length prefix
        // could otherwise force a 4 GiB allocation (DoS via crafted sidecar
        // response or compromised script). 256 MiB is far above any
        // legitimate batch (BGE-small @ batch=1024 ≈ 1.5 MiB).
        const MAX_FRAME: usize = 256 * 1024 * 1024;

        let mut hdr = [0u8; 4];
        stdout
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
        stdout
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
                                let f = n
                                    .as_f64()
                                    .ok_or_else(|| Error::Other("mlx vec elem not float".into()))?;
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

// ---------------------------------------------------------------------------
// 1ms coalesce window
// ---------------------------------------------------------------------------
//
// Rationale: real bench (50 Vec, 50 Hybrid) showed singleton p50 +5ms (Vec)
// and +35ms (Hybrid) vs fastembed CPU because each query pays one round-trip
// of msgpack-framed sidecar IPC. The MLX *batched* path is dramatically
// faster (~0.2ms/doc at batch=32). Solution: coalesce concurrent
// `embed_one` calls within a 1ms window into a single sidecar batch.
//
// Design:
//   * A worker thread owns the sidecar handle (no Mutex contention).
//   * Each `embed_one` call sends `(text, oneshot::Sender<Vec<f32>>)` over
//     a std::sync::mpsc and blocks on the reply.
//   * Worker drains the queue with a 1ms timeout; flushes when either
//     1ms elapses since the first queued item OR queue size hits 32.
//   * Pre-existing `embed_batch` path bypasses the coalescer (explicit
//     batch callers already pay one IPC for many docs).

use std::thread;
use std::time::Instant;

const COALESCE_WINDOW: Duration = Duration::from_millis(1);
const COALESCE_MAX_BATCH: usize = 32;

type ReplySender = std::sync::mpsc::Sender<Result<Vec<f32>>>;
type BatchFn = dyn Fn(&[String]) -> Result<Vec<Vec<f32>>> + Send + Sync;

struct CoalesceReq {
    text: String,
    reply: ReplySender,
}

/// Coalescing wrapper around [`MlxMetalEmbedder`]. Drop-in for
/// `Arc<dyn TextEmbedder>` consumers; transparently merges concurrent
/// singleton calls into batched sidecar invocations.
pub struct CoalescingMlxEmbedder {
    name: String,
    dim: usize,
    inner: std::sync::Arc<MlxMetalEmbedder>,
    tx: mpsc::Sender<CoalesceReq>,
    _worker: std::sync::Arc<thread::JoinHandle<()>>,
}

impl CoalescingMlxEmbedder {
    pub fn new() -> Result<Self> {
        let inner = std::sync::Arc::new(MlxMetalEmbedder::new()?);
        let name = format!("{}+coalesce1ms", inner.name);
        let dim = inner.dim;

        let (tx, rx) = mpsc::channel::<CoalesceReq>();
        let worker_inner = inner.clone();
        let batch_fn: std::sync::Arc<BatchFn> =
            std::sync::Arc::new(move |texts: &[String]| worker_inner.embed_batch(texts));
        let worker = thread::Builder::new()
            .name("mlx-coalesce".into())
            .spawn(move || coalesce_run(rx, batch_fn))
            .map_err(|e| Error::Other(format!("coalesce worker spawn: {e}")))?;

        Ok(Self {
            name,
            dim,
            inner,
            tx,
            _worker: std::sync::Arc::new(worker),
        })
    }
}

fn coalesce_run(rx: mpsc::Receiver<CoalesceReq>, batch_fn: std::sync::Arc<BatchFn>) {
    loop {
        // Block until first request arrives.
        let first = match rx.recv() {
            Ok(r) => r,
            Err(_) => return, // channel closed
        };
        let mut batch: Vec<CoalesceReq> = vec![first];
        let deadline = Instant::now() + COALESCE_WINDOW;

        // Drain additional requests within the 1ms window or up to the cap.
        while batch.len() < COALESCE_MAX_BATCH {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match rx.recv_timeout(deadline - now) {
                Ok(r) => batch.push(r),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        let texts: Vec<String> = batch.iter().map(|r| r.text.clone()).collect();
        match batch_fn(&texts) {
            Ok(vecs) => {
                debug_assert_eq!(vecs.len(), batch.len());
                for (req, v) in batch.into_iter().zip(vecs) {
                    let _ = req.reply.send(Ok(v));
                }
            }
            Err(e) => {
                let msg = format!("{e}");
                for req in batch {
                    let _ = req.reply.send(Err(Error::Other(msg.clone())));
                }
            }
        }
    }
}

impl TextEmbedder for CoalescingMlxEmbedder {
    fn name(&self) -> &str {
        &self.name
    }

    fn dim(&self) -> usize {
        self.dim
    }

    /// Batch path: bypass the coalescer entirely. The caller already
    /// amortised IPC cost themselves.
    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.inner.embed_batch(texts)
    }

    /// Single path: enqueue + block on oneshot reply. Concurrent callers
    /// merge into a single sidecar batch within `COALESCE_WINDOW`.
    fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let (reply_tx, reply_rx) = mpsc::channel::<Result<Vec<f32>>>();
        self.tx
            .send(CoalesceReq {
                text: text.to_string(),
                reply: reply_tx,
            })
            .map_err(|_| Error::Other("mlx coalesce worker dead".into()))?;
        reply_rx
            .recv()
            .map_err(|_| Error::Other("mlx coalesce reply dropped".into()))?
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

    /// Coalescer microbench: a fake batch_fn sleeps 5ms (≈ MLX IPC roundtrip)
    /// but returns vectors for the entire batch. Drive 16 concurrent
    /// `embed_one`-style senders. Total wall-time should be ~5ms (one batch
    /// pays IPC) — *not* 16×5=80ms (one IPC per query, the pre-coalescer
    /// behaviour). This proves fan-in works.
    #[test]
    fn coalescer_fans_in_concurrent_singletons() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let batch_calls = Arc::new(AtomicUsize::new(0));
        let bc = batch_calls.clone();
        let fake: Arc<BatchFn> = Arc::new(move |texts: &[String]| {
            bc.fetch_add(1, Ordering::SeqCst);
            // Simulate the ~5ms msgpack-IPC roundtrip seen in real bench.
            std::thread::sleep(Duration::from_millis(5));
            Ok(texts.iter().map(|_| vec![0.1f32; 384]).collect())
        });

        let (tx, rx) = mpsc::channel::<CoalesceReq>();
        let _w = thread::spawn(move || coalesce_run(rx, fake));

        let start = Instant::now();
        let n = 16;
        let mut replies = Vec::with_capacity(n);
        for i in 0..n {
            let (rtx, rrx) = mpsc::channel();
            tx.send(CoalesceReq {
                text: format!("doc-{i}"),
                reply: rtx,
            })
            .unwrap();
            replies.push(rrx);
        }
        // Collect all replies.
        for rrx in replies {
            let v = rrx.recv().unwrap().expect("vec");
            assert_eq!(v.len(), 384);
        }
        let elapsed = start.elapsed();

        // With coalescing: 1 batch call covers all 16 → ~5ms + overhead.
        // Without: 16 batch calls → ~80ms. Assert we're well under 40ms.
        assert!(
            elapsed < Duration::from_millis(40),
            "coalescer took {:?}, expected < 40ms (fan-in failed?)",
            elapsed
        );
        // 16 concurrent sends within 1ms window should batch into 1 call.
        // Allow up to 2 in case scheduling jitter splits them.
        let calls = batch_calls.load(Ordering::SeqCst);
        assert!(
            calls <= 2,
            "expected ≤2 batch calls, got {calls} (poor fan-in)"
        );
    }

    /// Verify the deadline mechanism: a single in-flight request flushes
    /// after ~1ms even if no further requests arrive.
    #[test]
    fn coalescer_flushes_lone_request_after_window() {
        use std::sync::Arc;
        let fake: Arc<BatchFn> =
            Arc::new(|texts: &[String]| Ok(texts.iter().map(|_| vec![0.0f32; 384]).collect()));
        let (tx, rx) = mpsc::channel::<CoalesceReq>();
        let _w = thread::spawn(move || coalesce_run(rx, fake));

        let start = Instant::now();
        let (rtx, rrx) = mpsc::channel();
        tx.send(CoalesceReq {
            text: "alone".to_string(),
            reply: rtx,
        })
        .unwrap();
        let v = rrx.recv().unwrap().expect("vec");
        assert_eq!(v.len(), 384);
        // Single request should be flushed within ~1ms + jitter.
        assert!(
            start.elapsed() < Duration::from_millis(20),
            "lone request took {:?}",
            start.elapsed()
        );
    }
}
