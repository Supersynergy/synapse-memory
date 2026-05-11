# Synapse Optimization Roadmap — Next Sprint (2026-05-11)

Generated post omega-pipe self-eval (R@5 baseline 0.60 verified, security clean, distribution-assets ready).

## Verified-State (2026-05-11)

| Metric | Value | Source |
|--------|-------|--------|
| Latency vs LanceDB @ 20k | 534× faster | live bench |
| Latency vs Qdrant @ 20k | 892× faster | live bench |
| R@5 BGE-small + JINA-rerank | 0.60 (N=30 stable) | longmemeval bench |
| cargo build | 0 errors, 6 warns | this session |
| cargo deny check advisories | OK (4 docs ignores) | this session |
| gitleaks (real findings) | 0 | leaks2.json filtered |
| Workspace gating | edge+cms-bench excluded | Cargo.toml |

## Top-10 ROI Optimizations (sorted, no API key required)

| # | Action | Expected Δ | Effort | Risk |
|---|--------|-----------:|-------:|-----:|
| 1 | Re-bench R@5 with `--ppr --hyde-threshold 5 --pre-extract` | +0.10–0.15 | 10min | 0 |
| 2 | `SYNAPSE_EMBED_MODEL=arctic-m` re-bench (768-dim, fresh corpus) | +0.06–0.10 | 1h (model dl) | low |
| 3 | `SYNAPSE_EMBED_MODEL=mxbai-large` re-bench (1024-dim) | +0.08–0.12 | 1h | medium (size) |
| 4 | Train LightGBM LambdaMART on `synapse-learn` click-log | +0.06–0.09 | 1d | medium |
| 5 | Add `cargo fix` 6 remaining warns (pub-but-unused → `#[allow]` or remove) | clean | 5min | 0 |
| 6 | omega-pipe MCP-server mode → expose to other Claude sessions | qual win | 4h | low |
| 7 | omega cron-warm-loop nightly → top arms → MEMORY.md auto-promote | qual win | 1h | 0 |
| 8 | Auto-merge dedupe (MinHash + cosine) → corpus quality | latent | 3h | low |
| 9 | Conformal-prediction recall-guarantee (enterprise sell) | feature | 1w | medium |
| 10 | GPU/Metal kernel for 1B+ corpus | scale | 1w | high |

## omega-pipe Self-Improvements (3 ship-this-session)

### 1. Real reward-loop wire
Currently `--verify` calls `verification-loop` binary; reward = 0/1.
Better: pipe LongMemEval R@5 delta-vs-baseline as continuous reward into bandit.

```python
# In omega.py update_bandit():
def reward_from_recall(arm_output: str) -> float:
    # parse "Recall@5 : 0.600" → 0.60
    ...
```

### 2. MCP-server expose
Add `omega-pipe mcp-serve` subcommand → other Claude sessions trigger via tool-call:

```bash
omega mcp-serve --port 9477
# Other session: tool-call omega.run(intent="...")
```

### 3. Cron weekly arm-promote
launchd job: every Sun 20:00 → `omega bandit-stats` → top-arm-per-stage → append to `~/.claude/projects/-Users-master/memory/MEMORY.md` under "Top-Verified Skills".

## Hebel-Lens Move (after Top-10 ship)

| Hebel | Now | After Top-3 (R@5 push) | After Top-10 |
|-------|----:|-----------------------:|-------------:|
| moat | 10 | 10 | 10 |
| 10x | 10 | 10 (R@5 ≥ 0.80) | 10 (+ scale 1B+) |
| compounding | 9 | 10 (more R@5 → more usage) | 10 |
| flywheel | 8 | 9 | 10 (cron + MCP loop) |
| automation | 9 | 10 | 10 |

## Live Test Result — Stack-3-Lever (2026-05-11 02:13)

```
~/projects/synapse/target/release/longmemeval --embed --limit 10 \
  --rerank-top 30 --hyde-threshold 5 --ppr --pre-extract
→ Recall@5 : 0.600  (UNCHANGED vs baseline 0.600)
```

### Why zero-delta (root cause)

| Lever | Status | Reason |
|-------|--------|--------|
| `--hyde-threshold 5` | inactive | "HyDE fires if hits<5", avg=47 docs/Q → never |
| `--ppr` | inactive | needs graph-data populated; corpus is conversation-flat |
| `--pre-extract` | trivial | RuleExtractor (no-LLM) = string-pattern only |

**Real bottleneck = embedder ceiling** (BGE-small MTEB 53.0 → R@5 ≈ 0.60 plateau).

### Revised top-3 (post-discovery)

1. **`SYNAPSE_EMBED_MODEL=arctic-m`** (768-dim, MTEB 62.5) — ONLY way past 0.60 plateau
2. **`SYNAPSE_EMBED_MODEL=mxbai-large`** (1024-dim, MTEB 64.7) — even higher ceiling
3. **`--hyde-threshold 50`** force-trigger HyDE on every Q (needs MLX/MiniMax key)

Skip PPR/pre-extract until graph-population pipeline added.

## Sequence (ship-this-session, ROI-sorted, post-discovery)

1. **arctic-m re-bench** — only realistic 0.60→0.70+ path on this dataset
2. **#5** cargo fix 6 warns (5min, free win)
3. **#7** omega cron weekly arm-promote launchd (1h, automation moat)

#4, #6, #8-10 = next session.

## Reproduce (from any clone)

```bash
git clone <repo>; cd synapse
cargo build --release -p longmemeval --features rerank
~/projects/synapse/target/release/longmemeval --embed --limit 30 --rerank-top 20         # baseline 0.60
~/projects/synapse/target/release/longmemeval --embed --limit 30 --rerank-top 30 \
    --hyde-threshold 5 --ppr --pre-extract                                                 # +stack
SYNAPSE_EMBED_MODEL=arctic-m \
~/projects/synapse/target/release/longmemeval --embed --limit 30 --rerank-top 30          # +arctic
```
