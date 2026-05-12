//! LLM-judge for LongMemEval. Spawns a persistent Python process
//! (bench/longmemeval/judge_server.py) that loads the MLX model once
//! and accepts JSON-line requests over stdin/stdout. ~5-7s startup,
//! then ~50-300ms per judgment instead of full reload each call.
//!
//! Protocol parity: substring R@5 caps at ~0.64 on LME-S-50 because
//! 18/50 gold answers are paraphrases that never appear literally
//! in the conversation. LLM-judge mirrors the LongMemEval paper.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct Judge {
    pub model: String,
    pub timeout_ms: u64,
    pub max_passage_chars: usize,
    inner: Mutex<Option<Inner>>,
    available: bool,
}

struct Inner {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Judge {
    pub fn new(model: String, timeout_ms: u64) -> Self {
        let python = std::env::var("LME_PYTHON")
            .unwrap_or_else(|_| "/Users/master/.venvs/agents/bin/python".to_string());
        let server = std::env::var("LME_JUDGE_SERVER").unwrap_or_else(|_| {
            let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            p.push("judge_server.py");
            p.to_string_lossy().into_owned()
        });
        if !std::path::Path::new(&python).exists() {
            eprintln!("[judge] python not found at {} — disabled", python);
            return Self {
                model,
                timeout_ms,
                max_passage_chars: 600,
                inner: Mutex::new(None),
                available: false,
            };
        }
        if !std::path::Path::new(&server).exists() {
            eprintln!("[judge] server script not found at {} — disabled", server);
            return Self {
                model,
                timeout_ms,
                max_passage_chars: 600,
                inner: Mutex::new(None),
                available: false,
            };
        }
        eprintln!("[judge] launching server: {} {}", python, server);
        let mut child = match Command::new(&python)
            .arg(&server)
            .env("LME_JUDGE_MODEL", &model)
            .env("HF_HUB_OFFLINE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[judge] spawn failed: {}", e);
                return Self {
                    model,
                    timeout_ms,
                    max_passage_chars: 600,
                    inner: Mutex::new(None),
                    available: false,
                };
            }
        };
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        let mut inner = Inner {
            child,
            stdin,
            stdout,
        };
        // Wait for ready handshake (model load can take 5-15s cold).
        let mut line = String::new();
        let read_deadline = Instant::now() + Duration::from_secs(60);
        let ready = loop {
            line.clear();
            match inner.stdout.read_line(&mut line) {
                Ok(0) => break false,
                Ok(_) => {
                    if line.contains("\"ready\"") {
                        break true;
                    }
                }
                Err(_) => break false,
            }
            if Instant::now() > read_deadline {
                break false;
            }
        };
        if !ready {
            eprintln!("[judge] server failed to signal ready");
            let _ = inner.child.kill();
            return Self {
                model,
                timeout_ms,
                max_passage_chars: 600,
                inner: Mutex::new(None),
                available: false,
            };
        }
        eprintln!("[judge] ready");
        Self {
            model,
            timeout_ms,
            max_passage_chars: 600,
            inner: Mutex::new(Some(inner)),
            available: true,
        }
    }

    pub fn is_available(&self) -> bool {
        self.available
    }

    pub fn judge(&self, question: &str, gold: &str, passages: &[&str]) -> Option<bool> {
        if !self.available {
            return None;
        }
        // Send full passages — server does smart density-window truncation
        // around question/answer keywords (LME_JUDGE_PASS_CHARS, default 3000).
        // Hard cap at 16K chars per passage as a safety belt.
        let trimmed: Vec<String> = passages
            .iter()
            .map(|p| p.chars().take(16_000).collect::<String>())
            .collect();
        let req = json!({
            "q": question,
            "a": gold,
            "passages": trimmed,
        });
        let payload = req.to_string() + "\n";
        let mut guard = self.inner.lock().ok()?;
        let inner = guard.as_mut()?;
        if inner.stdin.write_all(payload.as_bytes()).is_err() {
            return None;
        }
        if inner.stdin.flush().is_err() {
            return None;
        }
        let mut line = String::new();
        // Blocking read with no per-call timeout — server is in-process and
        // each call after warmup is well under 1s. If the server hangs, the
        // outer bench has its own wallclock — we'd see the hang in `ms`.
        if inner.stdout.read_line(&mut line).is_err() {
            return None;
        }
        let v: Value = serde_json::from_str(line.trim()).ok()?;
        if let Some(c) = v.get("correct").and_then(|x| x.as_bool()) {
            return Some(c);
        }
        None
    }
}

impl Drop for Judge {
    fn drop(&mut self) {
        if let Ok(mut g) = self.inner.lock() {
            if let Some(mut inner) = g.take() {
                let _ = inner.child.kill();
                let _ = inner.child.wait();
            }
        }
    }
}
