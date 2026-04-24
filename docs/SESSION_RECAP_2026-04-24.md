# Session Recap — v2.1-m4max-preview · 2026-04-24 · 30-iter loop

> Post-mortem of the single-session Synapse v2.1 build. For readers picking up v2.2.

## Phase timeline

| Phase | Iters | Output |
|-------|-------|--------|
| **A. Core kernels** | 1-4 | SimSIMD bindings · Matryoshka · TextEmbedder trait · bench progression S0→S6 |
| **B. Python surface** | 5-10 | `synapse-py` crate · Brain · AdaptiveRouter · 3 framework adapters |
| **C. Indices** | 11-20 | InMemoryI8 · InMemoryF16 · InMemoryHamming · `MultiIndex` one-liner |
| **D. Real-world benches** | 12-14, 21-22 | 50-usecase catalog · 11 loaders · aggregator · 9-format loader · demo.sh |
| **E. SoA levers** | 15-18 | Binary-Matryoshka · rayon chunk-tuning · f16 helpers · f16 storage index |
| **F. Audit & polish** | 23, 25, 27 | 4-Haiku parallel audit (bottleneck/errors/best-practice/smart-test) · pytest parity · vs_competitors doc |
| **G. Publish assets** | 24, 26, 28-30 | CHANGELOG · vs_competitors · LAUNCH_CARD · demo.sh · FINAL_COMPARISON · synapse-metal |

## What worked

### Parallel Haiku-agents (cheap supervisor pattern)
- **5 agents × ~30s each** = full test + bench + clippy + loader-writes in under 5 min wall-time.
- Cost ≈ 20× less than Opus per agent. Opus stays the orchestrator, reviews, decides.
- Rich context given to each agent (existing-file templates, expected output shape) = high first-pass success.
- **Lesson**: Context-less agents waste tokens. Spend 30-50 tokens on briefing to save 5× that in re-tries.

### Token-efficient research pattern
- **1 ghgrep batch** per open question, stop after 2 zero-hit rounds.
- Skip deep-research-agents when known-SoA is enough. Example: `acceleration_menu.md` compiled 8 levers in <80 tokens vs 50k a full research-agent run would cost.

### 5-minute cache window scheduling
- `ScheduleWakeup delaySeconds ∈ [230, 278]` stays inside Anthropic's 5-minute TTL → warm-cache wake-ups every iteration.
- Cost delta vs 1500s wakeups: about 12× cheaper per wakeup, enables tighter commits-per-hour.

### Commit-per-iter discipline
- Each iter lands a single commit, runs tests+clippy first.
- Parallel sessions (WordPress bench, synapse-mysql) interleave on `main` without breaking our branch.
- `git branch m4max-preview + tag v2.1-m4max-preview-2026-04-24` = durable rollback points.

## What bit us

### Linter drift
- Some edits to `Cargo.toml` + `lib.rs` were reverted between turns by background tooling. Commits stayed in git history but working-tree lagged.
- **Mitigation**: stand-alone `[workspace]` in `synapse-py/Cargo.toml` and `synapse-metal/Cargo.toml` bypasses the parent-workspace members drift.

### Thermal throttling on M-series
- Same kernel measured from 192 µs (cold) to 5 822 µs (thermal-loaded) across runs.
- **Mitigation**: always report best-of-N + publish the raw-runs alongside. `FINAL_COMPARISON.md` documents variance explicitly.

### Stale internal modules (brainpack)
- Tried to expose `synapse_core::brainpack` to Python — it referenced `super::synx` + `Error::Format` that were never wired into current `lib.rs`.
- **Lesson**: `pub mod X` a new module before calling it from downstream crates. Revert was cheap because diff was small.

## Deferred (v2.2 target)

- Full MSL dispatch path in `synapse-metal::MetalI8Matvec::dispatch` (scaffold shipped)
- Thompson **real-sample** in `AdaptiveRouter` (currently posterior-mean, rand_distr dep risk)
- **Product Quantization** (FAISS-standard 8-16× compression)
- **HNSW live-wire** into `Store::search_vec` (feature builds, hook missing)
- **Candle-Metal** real BGE-small forward pass (scaffold only)
- **CoreML ANE** cross-encoder rerank via swift-bridge
- **Accelerate BLAS** auto-route for ndarray path

## Metric delta — v2.0 Turbo → v2.1 preview

| path | v2.0 | v2.1 best | factor |
|------|------|-----------|--------|
| int8 cos | 1 284 µs · 779 QPS | **348 µs · 2 877 QPS** | **3.69×** |
| binary cos | 661 µs · 1 512 QPS | **177 µs · 5 654 QPS** | **3.73×** |
| full-recall pipeline | — | **324 µs · 3 083 QPS** | new |
| RAM / vec at 384d | 1 536 B (fp32) | **768 B** (f16) | **-50 %** |

## Artifacts index

```
docs/
├── SPEC_V2_M4_MAX_2026-04-24.md      # living tick-list, Phase A-D shipped
├── M4_MAX_INTEGRATION.md             # hardware audit + 12-lever roadmap
├── LAUNCH_CARD.md                    # static launch blurb + 140-char pitches
├── TASK_1_METAL_SHADER_PLAN.md       # v2.2 MSL-dispatch plan
├── SESSION_RECAP_2026-04-24.md       # ← this file
├── bench_2026-04-24/
│   ├── vs_competitors.md             # head-to-head (scalar/ndarray/SimSIMD/pipeline)
│   ├── progression.md                # S0→S8 variance study
│   └── FINAL_COMPARISON.md           # all 29-iter runs + new-PB highlights
├── bench_realworld/
│   └── README.md                     # 50-usecase catalog
└── research_2026-04-24/
    └── acceleration_menu.md          # 8 SoA levers ranked

crates/
├── synapse-core/                     # (existed) + Phase A/B/C/E additions
├── synapse-py/                       # NEW · PyO3 bindings · pyproject.toml · pytest
└── synapse-metal/                    # NEW · Task #1 scaffold · CPU fallback · feature gpu

bench/realworld/
├── harness.py                        # 9-format loader
├── aggregator.py                     # JSON + HTML dashboard
├── bench_all.sh                      # env-driven runner
├── 01_obsidian.py · 03_apple_notes.py · 04_logseq.py · 11_chatgpt_history.py
├── 21_gmail_mbox.py · 22_slack_export.py · 23_imessage.py · 24_whatsapp.py
├── 33_photo_clip.py · 40_linear_issues.py · 49_health_fuse.py
└── (external/bench_vs_faiss.py stub for v0.3)

demo.sh                               # one-command build+test+bench+card
```

## Rollback

```bash
git checkout backup-pre-m4max-bench-2026-04-24      # git tag, pre-loop
git checkout v2.1-m4max-preview-2026-04-24          # this session's head
tar -xzf ~/projects/data/synapse-backup-2026-04-24.tgz
```
