# 05 — Repo Hygiene & Cleanup Plan

Pre-split cleanup for `/Users/master/projects/synapse`. Goal: shrink the working tree, archive dated noise, and pre-stage `products/` + `release/context-os/` so the physical split into **synapse-db** and **synapse-memory** lands on a clean base.

All commands assume CWD `/Users/master/projects/synapse`. Run on a dedicated branch/worktree, not on a dirty tree. Steps marked **REVIEW BEFORE RUN** touch history or delete bytes — eyeball the listing first.

This is a **prep pass**, not the split itself. Do hygiene first, then execute the crate-level split per docs 01–04.

---

## 0. Verified state (this session, 2026-05-29)

| Check | Command | Result |
|---|---|---|
| examples bloat | `du -sh examples/*` | `library_mode 992M · agent_memory 892M · code_search 886M · multimodal_rag 477M` (~3.2G) |
| example `target/` tracked? | `git ls-files examples/ \| grep target/` | **0 tracked** (build dirs are local-only) |
| example data tracked? | `git ls-files examples/ \| grep -iE '\.(db\|parquet\|bin\|safetensors\|gguf\|onnx\|csv)$'` | **0 tracked** |
| examples tracked total | `git ls-files examples/ \| wc -l` | 84 source files only (the 3.2G is untracked build/data) |
| `.gitignore` examples cover | `grep -nE 'examples\|target' .gitignore` | `target/` (line 1) + `bench/**/target/` (line 17) |
| `docs/archive/` exists | `ls -d docs/archive` | yes |
| tracked bench data | `git ls-files \| grep -iE '\.(json\|csv\|parquet\|bin)$'` | many small bench JSON/CSV/`.bin` under `bench/` + `docs/` (ground-truth fixtures, mostly fine) |

**Headline:** the 3.2G in `examples/` is **not in git** — it is local `target/` + downloaded datasets, already ignored. So the bloat is a working-tree/clone-size concern, not a history-rewrite concern. No mistracked large binaries were found.

---

## 1. `.gitignore` additions

Current `.gitignore` already covers `target/`, `bench/**/target/`, `*.db`, `*.parquet`. The global `target/` line already matches `examples/*/target/`, so those are safe. Add the following to harden against future drift (idempotent — append only if missing).

```gitignore
# --- examples: keep source, never commit built artifacts or downloaded datasets ---
examples/**/target/
examples/**/data/
examples/**/*.db
examples/**/*.parquet
examples/**/*.bin
examples/**/*.safetensors
examples/**/*.gguf
examples/**/*.onnx
examples/**/*.csv

# --- split scaffolding: vendored foundation submodule build output (synapse-memory repo) ---
vendor/synapse-db/**/target/

# --- legacy duplicate store (archived during split) ---
synapsestore/**/target/
```

Apply (review the diff before staging):

```bash
# REVIEW BEFORE RUN — appends ignore rules
cat >> .gitignore <<'EOF'

# examples: keep source, never commit built artifacts or downloaded datasets
examples/**/target/
examples/**/data/
examples/**/*.db
examples/**/*.parquet
examples/**/*.bin
examples/**/*.safetensors
examples/**/*.gguf
examples/**/*.onnx
examples/**/*.csv

# split scaffolding: vendored foundation submodule build output
vendor/synapse-db/**/target/

# legacy duplicate store (archived during split)
synapsestore/**/target/
EOF
git diff .gitignore
```

> Note: a blanket `examples/**/*.csv` could mask a *committed* fixture. Verified above that **0 data files are tracked under `examples/`**, so this is safe today. If a future example needs a committed fixture, add a `!examples/<name>/fixtures/foo.csv` negation rather than dropping the rule.

---

## 2. Root `.md` cleanup (17 files → KEEP / ARCHIVE)

Move dated bench/launch/status reports into `docs/archive/`. Keep only the evergreen project docs at root.

| File | Action | Reason |
|---|---|---|
| `ARCHITECTURE.md` | **KEEP** | Describes whole monorepo. Split note: this becomes the **synapse-db** arch doc; trim memory crates out post-split (see doc 00 on the SPEC vs ARCHITECTURE contradiction). |
| `SPEC.md` | **KEEP** | The 15-crate "Context-OS" spec = **synapse-memory** product scope. Moves with the memory repo at split time. |
| `README.md` | **KEEP** | Front door (rewrite per-repo at split). |
| `CHANGELOG.md` | **KEEP** | Release history. |
| `CONTRIBUTING.md` | **KEEP** | Contributor guide. |
| `LICENSE-CORE.md` | **KEEP** | Dual-license (foundation). |
| `LICENSE-ENGINE.md` | **KEEP** | Dual-license (engine/db). |
| `BENCH_2026-05-10.md` | ARCHIVE | Dated bench snapshot. |
| `PROD_READY_2026-05-10.md` | ARCHIVE | Dated status (filename ≠ truth). |
| `PUBLISH_STATUS_2026-05-13.md` | ARCHIVE | Dated publish status. |
| `CORRECTIVE-ACTION-PLAN-2026-04-25.md` | ARCHIVE | Dated plan, superseded. |
| `LAUNCH_CHECKLIST_v1.0.1.md` | ARCHIVE | Past-release checklist. |
| `RELEASE_NOTES_v1.0.1-rc.md` | ARCHIVE | RC notes → fold into `CHANGELOG.md`. |
| `RELEASE_NOTES_v1.0.1-rc.1.md` | ARCHIVE | RC notes → fold into `CHANGELOG.md`. |
| `HN_LAUNCH.md` | ARCHIVE | One-time launch copy. |
| `KNOWN-ISSUES.md` | ARCHIVE | Dated audit log; live issues belong in the tracker. Re-create a fresh per-repo `KNOWN-ISSUES.md` if needed. |
| `tuning.md` | ARCHIVE | Superseded by `docs/ANN-TUNING-SOTA-2026.json` + bench dirs. |

Commands (history-preserving moves):

```bash
mkdir -p docs/archive/root-2026
git mv BENCH_2026-05-10.md                  docs/archive/root-2026/
git mv PROD_READY_2026-05-10.md             docs/archive/root-2026/
git mv PUBLISH_STATUS_2026-05-13.md         docs/archive/root-2026/
git mv CORRECTIVE-ACTION-PLAN-2026-04-25.md docs/archive/root-2026/
git mv LAUNCH_CHECKLIST_v1.0.1.md           docs/archive/root-2026/
git mv RELEASE_NOTES_v1.0.1-rc.md           docs/archive/root-2026/
git mv RELEASE_NOTES_v1.0.1-rc.1.md         docs/archive/root-2026/
git mv HN_LAUNCH.md                         docs/archive/root-2026/
git mv KNOWN-ISSUES.md                      docs/archive/root-2026/
git mv tuning.md                            docs/archive/root-2026/
git status --short
```

After this, root holds 7 evergreen `.md` files + the two `LICENSE-*` files. Verify:

```bash
ls *.md   # expect: ARCHITECTURE CHANGELOG CONTRIBUTING LICENSE-CORE LICENSE-ENGINE README SPEC
```

---

## 3. `docs/` cleanup (~74 md + 5 dated dirs → archive dated bench/plan)

`docs/` holds 74 `.md` and dated bench dirs. Archive everything that is a **dated bench, plan, roadmap, or status snapshot**; keep only living reference docs (the `split/` set, current spec-vs-reality, tuning config). `docs/archive/` already exists.

### 3a. Dated bench directories → archive

```bash
mkdir -p docs/archive/bench
# REVIEW BEFORE RUN — moves dated bench result dirs (contain tracked JSON/CSV fixtures)
git mv docs/bench_1on1_2026-04-23  docs/archive/bench/
git mv docs/bench_2026-04-23       docs/archive/bench/
git mv docs/bench_2026-04-24       docs/archive/bench/
git mv docs/bench_scale_2026-04-23 docs/archive/bench/
git mv docs/bench_realworld        docs/archive/bench/   # review: confirm not referenced by live CI
```

### 3b. Dated `*.md` reports → archive

Pattern: any doc whose name carries a date stamp or is a one-shot plan/roadmap/audit/status. Living docs (`split/*`, current SPEC, `INTEGRATION_PLAN.md` if still active) stay.

```bash
mkdir -p docs/archive/reports-2026
# REVIEW BEFORE RUN — batch-archive dated reports. Inspect the glob first:
ls docs/*_2026-*.md docs/*-2026-*.md 2>/dev/null
```

Concrete moves (the dated/one-shot set found this session):

```bash
git mv docs/BILLION_SCALE_ROADMAP_2026-05-06.md            docs/archive/reports-2026/
git mv docs/CONTEXT_OS_PLAN_2026-05-18.md                  docs/archive/reports-2026/
git mv docs/CONTEXT_OS_RELEASE_COMPLETION_AUDIT_2026-05-25.md docs/archive/reports-2026/
git mv docs/DEEP_ANALYSIS_2026-05-07.md                    docs/archive/reports-2026/
git mv docs/EDGE_STACK_SETUP_2026-05-16.md                 docs/archive/reports-2026/
git mv docs/FINAL_BENCH_SCREENSHOT_2026-05-06.md           docs/archive/reports-2026/
git mv docs/HNSW_SWEEP_2026-05-06.md                       docs/archive/reports-2026/
git mv docs/KRASS-REBASE-PLAN-2026-04-26.md                docs/archive/reports-2026/
git mv docs/LONGMEMEVAL_RESULTS_2026-05-25.md              docs/archive/reports-2026/
git mv docs/MASTERPLAN_v3_2026-05-07.md                    docs/archive/reports-2026/
git mv docs/MATRIX_20x20_BENCH_2026-05-07.md               docs/archive/reports-2026/
git mv docs/MAX_EDGE_STACK_2026-05-14.md                   docs/archive/reports-2026/
git mv docs/MEM0-LETTA-VS-SYNAPSE-2026-04-26.md            docs/archive/reports-2026/
git mv docs/MINING-SOTA-2026-04-29.md                      docs/archive/reports-2026/
git mv docs/MINING-WORLDBEST-2026-05-05.md                 docs/archive/reports-2026/
git mv docs/MOMENTUM-2026-04-26.md                         docs/archive/reports-2026/
git mv docs/MULTI_AGENT_TEAMS_2026_04_27.md                docs/archive/reports-2026/
git mv docs/NICHE-DOMINANCE-2026-04-26.md                  docs/archive/reports-2026/
git mv docs/REBASE-INTEL-2026-04-26.md                     docs/archive/reports-2026/
git mv docs/RUST-DB-50-BREAKTHROUGHS-2026-04-26.md         docs/archive/reports-2026/
git mv docs/SESSION_VERDICT_2026-04-23.md                  docs/archive/reports-2026/
git mv docs/SOTA-ROADMAP-2026-04-29.md                     docs/archive/reports-2026/
git mv docs/SPEC-VS-REALITY-2026-05-04.md                  docs/archive/reports-2026/
git mv docs/SQLITE_TOP5_DEEP_BENCH_2026-05-06.md           docs/archive/reports-2026/
git mv docs/STANDARD_BENCH_TOOLS_2026-04-23.md             docs/archive/reports-2026/
git mv docs/STOLEN_PATTERNS_2026_04_27.md                  docs/archive/reports-2026/
git mv docs/SUPERDB_ROADMAP_2026_04_27.md                  docs/archive/reports-2026/
git mv docs/SYNAPSE_BEST_IN_CLASS_2026-04-23.md            docs/archive/reports-2026/
git mv docs/SYNAPSE_SPEC_v1_2026-04-23.md                  docs/archive/reports-2026/
git mv docs/SYNAPSE_VERIFICATION_2026-04-23.md             docs/archive/reports-2026/
git mv docs/SYNAPSE-X-ARCH-2026-05-03.md                   docs/archive/reports-2026/
git mv docs/TASK_1_METAL_SHADER_PLAN.md                    docs/archive/reports-2026/
git mv docs/TOP100_VEC_DB_LANDSCAPE_2026-05.md             docs/archive/reports-2026/
git mv docs/TOP100_VEC_DB_LIVE_2026-05-06.md               docs/archive/reports-2026/
git mv docs/TOP90_VEC_DB_LIVE_2026-05-06.md                docs/archive/reports-2026/
git mv docs/TRUTH-2026-05-10.md                            docs/archive/reports-2026/
git mv docs/TURSO-ANALYSIS-2026-04-26.md                   docs/archive/reports-2026/
git mv docs/ULTIMATE_MEMORY_GAP_ANALYSIS_2026_04_26.md     docs/archive/reports-2026/
git mv docs/WORDPRESS_STATUS_2026-04-23.md                 docs/archive/reports-2026/
git mv docs/WP_3WAY_BENCHMARK_2026-04-23.md                docs/archive/reports-2026/
git mv docs/WP_PERF_PATCHES_2026-04-23.md                  docs/archive/reports-2026/
git mv docs/WP_PERF_RESEARCH_2026-04-23.md                 docs/archive/reports-2026/
```

### 3c. KEEP in `docs/` (living)

- `docs/split/*` — this plan set (00–05). Authoritative.
- `docs/ANN-TUNING-SOTA-2026.json` — live tuning config (referenced by code/bench).
- `docs/INTEGRATION_PLAN.md`, `docs/RELEASE-PLAN.md` — **review**: keep only if still the active plan; otherwise archive. Decide during the split, not blindly.
- `docs/archive/**` — destination, untouched.

> The 32 tracked JSON/CSV/bin under `docs/` are mostly inside the dated bench dirs above and travel with them. Re-run `git ls-files docs/ | grep -iE '\.(json|csv|parquet|bin)$'` after 3a to confirm none are orphaned at `docs/` root.

---

## 4. `products/`, `release/context-os/`, `synapsestore/` → split mapping

| Dir | Contents (verified) | Disposition |
|---|---|---|
| `release/context-os/` | `install.sh · package.sh · service.sh · verify.sh · MANIFEST.md · CHECKLIST.md · README.md · RELEASE_NOTES.md · VERIFICATION.md · sample/` | **REUSE as the synapse-memory release dir.** This is the clean Context-OS release slice. At split, move into the memory repo at `release/` (or `apps/synapse-memory/release/`). Its `MANIFEST.md` defines the memory product's shippable surface — reconcile against the DECIDED memory bucket (space, extract, rerank, temporal, learn, synapsed, cli, mcp, py, js, metal, embed-gpu, colbert, splade, fusion, multimodal, media). |
| `products/` | `agentdb-local · benchmark-kit · claude-code-memory · enterprise-onprem · freshness-router · dist · README.md` | **Prior half-done split — reuse naming/ideas, archive the scaffold.** The crate-level bucket assignment (docs 01–04) supersedes these folder-products. Keep `claude-code-memory`/`agentdb-local`/`freshness-router` as *naming references* for the memory product's marketing/release surface; archive the half-built trees so they don't compete with the real split. `benchmark-kit` ideas → the shared `bench/` harness (lives with synapse-db). |
| `synapsestore/` | `bench · crates · docs` (legacy duplicate of `wal`/`seg`/`ultra`) | **ARCHIVE — legacy duplicate crates.** These are stale copies of the CUT scaffold crates (`synapse-wal` STUB, `synapse-seg` STUB, `synapse-ultra` = duplicate `synapsestore` daemon). Not part of either product. Move out of the workspace root so neither repo inherits dead crates. |

Commands:

```bash
# 4a — stage release/context-os as the memory release dir (do at split, in the memory worktree).
#       For now just confirm it's intact:
ls release/context-os/

# 4b — archive synapsestore legacy duplicate (REVIEW BEFORE RUN — confirm no workspace member points here)
grep -rn "synapsestore" Cargo.toml crates/*/Cargo.toml 2>/dev/null | head   # expect: no path-dep into synapsestore/
mkdir -p archive
git mv synapsestore archive/synapsestore-legacy-2026

# 4c — archive the products/ half-split scaffold, keep README + dist as naming reference
#       (REVIEW BEFORE RUN — confirm products/* are not referenced by CI or Cargo workspace)
grep -rn "products/" Cargo.toml .github/workflows/*.yml 2>/dev/null | head
mkdir -p archive/products-halfsplit-2026
git mv products/agentdb-local      archive/products-halfsplit-2026/
git mv products/benchmark-kit      archive/products-halfsplit-2026/
git mv products/claude-code-memory archive/products-halfsplit-2026/
git mv products/enterprise-onprem  archive/products-halfsplit-2026/
git mv products/freshness-router   archive/products-halfsplit-2026/
# keep products/README.md + products/dist/ as the naming/marketing reference until memory repo is stood up
```

> `archive/` is a git-tracked sibling at repo root (matches the workspace `archive/` convention in projects/CLAUDE.md). It is **not** ignored, so history is preserved. If you'd rather drop bytes entirely, `git rm -r` instead — **REVIEW BEFORE RUN**, irreversible without history surgery.

---

## 5. `examples/` bloat fix (~3.2G)

The 3.2G is **local-only** (build `target/` + downloaded datasets); **0 bytes tracked** beyond 84 source files. So the fix is local-tree reclamation + ignore-hardening, **no history rewrite needed**.

```bash
# 5a — reclaim local disk: nuke per-example build dirs (REVIEW BEFORE RUN — local artifacts, rebuildable)
du -sh examples/*/target 2>/dev/null
rm -rf examples/*/target           # ~2-3G back; cargo rebuilds on demand

# 5b — confirm no example data ever sneaks into git (should print nothing)
git ls-files examples/ | grep -iE '\.(db|parquet|bin|safetensors|gguf|onnx|csv)$'

# 5c — ignore rules from §1 already cover examples/**/{target,data,*.db,...}.
#       Verify a fresh status is clean after rebuilds:
git status --short examples/
```

If a future audit ever finds large blobs **already in history** (none today), that is a separate `git filter-repo` / BFG operation — out of scope for this prep pass. Current state needs none.

---

## Post-cleanup verification (run before declaring hygiene done)

```bash
# 1. tree builds (foundation + both buckets still compile from the shared workspace)
cargo check --workspace 2>&1 | tail -5

# 2. root is lean
ls *.md                              # 7 evergreen + LICENSE-* only

# 3. docs root has no stray dated reports
ls docs/*_2026-*.md docs/*-2026-*.md 2>/dev/null   # expect: no matches

# 4. no workspace member dangles into archived dirs
grep -rn "synapsestore\|products/agentdb\|products/freshness" Cargo.toml crates/*/Cargo.toml 2>/dev/null   # expect: empty

# 5. example artifacts gone, source intact
du -sh examples 2>/dev/null          # should drop from ~3.2G toward MBs
git ls-files examples/ | wc -l       # still 84

# 6. nothing destructive staged unintentionally
git status --short | grep -E '^D' | head
```

Only after all six pass: proceed to the physical crate split (docs 01–04) in the git worktree. Hygiene moves and the split should be **separate commits** so the diff stays reviewable.
