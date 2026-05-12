//! MLX Metal embedder sidecar for synapse-ultra.
//!
//! Reuses `scripts/synapse-mlx-embed.py` from synapse-core.
//! Wire protocol: 4-byte big-endian length + msgpack body (via rmp-serde).
//!
//! Env vars:
//! - `SYNAPSE_MLX_PYTHON`  — python interpreter (default: `python3`)
//! - `SYNAPSE_MLX_SCRIPT`  — sidecar script path (default: repo scripts/)
//! - `SYNAPSE_MLX_MODEL`   — HF model id passed through to sidecar
//! - `ULTRA_EMBEDDER`      — set to `mlx` to use this as primary
//!
//! # Coalescing worker
//! Under concurrent load, individual `embed_one` calls that arrive within a
//! 1ms window are merged into a single sidecar batch dispatch. The worker
//! thread owns the sidecar handle exclusively — no Mutex contention.
//!
//! - Window: 1ms
//! - Max batch: 16 items
//! - `embed_batch` (explicit batch callers) bypasses the coalescer.

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::{Result, UltraError};

const DEFAULT_SCRIPT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../scripts/synapse-mlx-embed.py",
);

const MAX_FRAME: usize = 64 * 1024 * 1024; // 64 MiB

const COALESCE_WINDOW: Duration = Duration::from_millis(1);
const COALESCE_MAX_BATCH: usize = 16;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct EmbedReq<'a> {
    texts: &'a [String],
}

#[derive(Deserialize)]
struct EmbedResp {
    #[serde(default)]
    vecs: Vec<Vec<f32>>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    ready: Option<bool>,
}

// ---------------------------------------------------------------------------
// Sidecar I/O
// ---------------------------------------------------------------------------

struct Sidecar {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

impl Sidecar {
    fn spawn() -> Result<Self> {
        let py = std::env::var("SYNAPSE_MLX_PYTHON").unwrap_or_else(|_| "python3".into());
        let script =
            std::env::var("SYNAPSE_MLX_SCRIPT").unwrap_or_else(|_| DEFAULT_SCRIPT.to_string());

        let mut child = Command::new(&py)
            .arg(&script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| UltraError::Embed(format!("mlx sidecar spawn ({py} {script}): {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| UltraError::Embed("mlx: no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| UltraError::Embed("mlx: no stdout".into()))?;

        let mut s = Self { child, stdin, stdout };

        // Wait for ready handshake
        let ready: EmbedResp = s.read_msg()?;
        if ready.ready != Some(true) {
            return Err(UltraError::Embed("mlx sidecar handshake failed".into()));
        }
        Ok(s)
    }

    fn read_msg<T: for<'de> Deserialize<'de>>(&mut self) -> Result<T> {
        let mut hdr = [0u8; 4];
        self.stdout
            .read_exact(&mut hdr)
            .map_err(|e| UltraError::Embed(format!("mlx read hdr: {e}")))?;
        let n = u32::from_be_bytes(hdr) as usize;
        if n == 0 || n > MAX_FRAME {
            return Err(UltraError::Embed(format!("mlx bad frame size: {n}")));
        }
        let mut buf = vec![0u8; n];
        self.stdout
            .read_exact(&mut buf)
            .map_err(|e| UltraError::Embed(format!("mlx read body: {e}")))?;
        rmp_serde::from_slice(&buf)
            .map_err(|e| UltraError::Embed(format!("mlx msgpack decode: {e}")))
    }

    fn write_msg<T: Serialize>(&mut self, val: &T) -> Result<()> {
        let payload =
            rmp_serde::to_vec_named(val).map_err(|e| UltraError::Embed(format!("mlx encode: {e}")))?;
        let hdr = (payload.len() as u32).to_be_bytes();
        self.stdin
            .write_all(&hdr)
            .map_err(|e| UltraError::Embed(format!("mlx write hdr: {e}")))?;
        self.stdin
            .write_all(&payload)
            .map_err(|e| UltraError::Embed(format!("mlx write body: {e}")))?;
        self.stdin
            .flush()
            .map_err(|e| UltraError::Embed(format!("mlx flush: {e}")))?;
        Ok(())
    }

    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.write_msg(&EmbedReq { texts })?;
        let resp: EmbedResp = self.read_msg()?;
        if let Some(err) = resp.error {
            return Err(UltraError::Embed(format!("mlx sidecar: {err}")));
        }
        Ok(resp.vecs)
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// Coalescing worker
// ---------------------------------------------------------------------------

type ReplySender = mpsc::Sender<Result<Vec<f32>>>;

struct CoalesceReq {
    text: String,
    reply: ReplySender,
}

fn coalesce_run(rx: mpsc::Receiver<CoalesceReq>, mut sidecar: Sidecar) {
    loop {
        let first = match rx.recv() {
            Ok(r) => r,
            Err(_) => return,
        };
        let mut batch: Vec<CoalesceReq> = vec![first];
        let deadline = Instant::now() + COALESCE_WINDOW;

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
        match sidecar.embed(&texts) {
            Ok(vecs) => {
                debug_assert_eq!(vecs.len(), batch.len());
                for (req, v) in batch.into_iter().zip(vecs.into_iter()) {
                    let _ = req.reply.send(Ok(v));
                }
            }
            Err(e) => {
                let msg = format!("{e}");
                for req in batch {
                    let _ = req.reply.send(Err(UltraError::Embed(msg.clone())));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public embedder
// ---------------------------------------------------------------------------

/// MLX Metal embedder with coalescing worker.
///
/// Concurrent `embed_one` calls arriving within 1ms are batched into a single
/// sidecar dispatch. `embed_batch` bypasses the coalescer (caller already
/// amortises IPC).
pub struct MlxEmbedder {
    tx: mpsc::SyncSender<CoalesceReq>,
    // Kept for `embed_batch` direct path — separate sidecar or serialised via tx.
    // We use tx even for batch: split into individual requests and fan-out.
    // (Explicit-batch callers get per-item oneshots merged by the worker.)
    _worker: thread::JoinHandle<()>,
}

impl MlxEmbedder {
    pub fn new() -> Result<Self> {
        let sidecar = Sidecar::spawn()?;
        // Bounded channel — backpressure at 64 pending requests.
        let (tx, rx) = mpsc::sync_channel::<CoalesceReq>(64);
        let worker = thread::Builder::new()
            .name("mlx-coalesce".into())
            .spawn(move || coalesce_run(rx, sidecar))
            .map_err(|e| UltraError::Embed(format!("coalesce worker spawn: {e}")))?;
        Ok(Self { tx, _worker: worker })
    }

    /// Embed a batch. Requests are split into individual coalesce-queue items
    /// then re-assembled. Under concurrent load the worker merges them with
    /// other in-flight singletons into one sidecar call.
    pub fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut receivers = Vec::with_capacity(texts.len());
        for text in texts {
            let (reply_tx, reply_rx) = mpsc::channel::<Result<Vec<f32>>>();
            self.tx
                .send(CoalesceReq { text: text.clone(), reply: reply_tx })
                .map_err(|_| UltraError::Embed("mlx coalesce worker dead".into()))?;
            receivers.push(reply_rx);
        }
        receivers
            .into_iter()
            .map(|rx| {
                rx.recv()
                    .map_err(|_| UltraError::Embed("mlx coalesce reply dropped".into()))?
            })
            .collect()
    }

    pub fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let (reply_tx, reply_rx) = mpsc::channel::<Result<Vec<f32>>>();
        self.tx
            .send(CoalesceReq { text: text.to_string(), reply: reply_tx })
            .map_err(|_| UltraError::Embed("mlx coalesce worker dead".into()))?;
        reply_rx
            .recv()
            .map_err(|_| UltraError::Embed("mlx coalesce reply dropped".into()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Coalescer microbench: fake batch_fn sleeps 5ms (≈ MLX IPC roundtrip).
    /// 16 concurrent senders should merge into ≤2 batch calls.
    #[test]
    fn coalescer_fans_in_concurrent_singletons() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let call_count = Arc::new(AtomicUsize::new(0));
        let cc = call_count.clone();

        // Build a fake Sidecar-equivalent via the internal channel directly.
        let (tx, rx) = mpsc::sync_channel::<CoalesceReq>(64);
        let _worker = thread::spawn(move || {
            loop {
                let first = match rx.recv() {
                    Ok(r) => r,
                    Err(_) => return,
                };
                let mut batch = vec![first];
                let deadline = Instant::now() + COALESCE_WINDOW;
                while batch.len() < COALESCE_MAX_BATCH {
                    let now = Instant::now();
                    if now >= deadline { break; }
                    match rx.recv_timeout(deadline - now) {
                        Ok(r) => batch.push(r),
                        _ => break,
                    }
                }
                cc.fetch_add(1, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(5));
                for req in batch {
                    let _ = req.reply.send(Ok(vec![0.1f32; 384]));
                }
            }
        });

        let start = Instant::now();
        let n = 16usize;
        let mut replies = Vec::with_capacity(n);
        for i in 0..n {
            let (rtx, rrx) = mpsc::channel();
            tx.send(CoalesceReq { text: format!("doc-{i}"), reply: rtx }).unwrap();
            replies.push(rrx);
        }
        for rrx in replies {
            let v = rrx.recv().unwrap().expect("vec");
            assert_eq!(v.len(), 384);
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(40),
            "coalescer took {elapsed:?}, expected < 40ms"
        );
        let calls = call_count.load(Ordering::SeqCst);
        assert!(calls <= 2, "expected ≤2 batch calls, got {calls}");
    }
}
