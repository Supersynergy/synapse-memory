# WORK_LOG - arctic-m default embedder

Status note 2026-05-25: the current release-verifiable baseline is the
no-download run in `docs/LONGMEMEVAL_RESULTS_2026-05-25.md`:
`cargo run -p longmemeval --no-default-features -- --rerank-top 0` with
Recall@5 = 0.640 on 50 questions. The arctic-m + rerank path below is kept as
implementation history and must be re-run before being used as a release claim.

## Ziel
arctic-m als default embedder wenn `embed-768` + `rerank` features beide aktiv.

## Implementierung

**File**: `bench/longmemeval/src/main.rs`

Vor dem `embedder` block eingefügt:
```rust
#[cfg(all(feature = "embed-768", feature = "rerank"))]
if args.embed && std::env::var("SYNAPSE_EMBED_MODEL").is_err() {
    std::env::set_var("SYNAPSE_EMBED_MODEL", "arctic-m");
}
```

- Falls `SYNAPSE_EMBED_MODEL` gesetzt: override respektiert (kein `set_var`)
- Nur aktiv wenn beide features compiliert (compile-time guard `#[cfg]`)
- `select_model()` in `embed.rs` kennt `"arctic-m"` bereits → mapped auf `EmbeddingModel::SnowflakeArcticEmbedM`

## Model Name (fastembed)
fastembed-rs enum: `EmbeddingModel::SnowflakeArcticEmbedM`  
Key string: `"arctic-m"` (via `select_model()` in `crates/synapse-core/src/embed.rs`)  
HF Name: `Snowflake/snowflake-arctic-embed-m` (768-dim)

## Bench Ergebnisse (confirmed, pre-implementation)
| Config | R@5 |
|--------|-----|
| BGE-small baseline | ~0.60 |
| arctic-m + rerank | **0.64** (+4pp) |

Smoke-bench command:
```bash
cargo run -p longmemeval --features "embed-768,rerank" -- \
  --embed --rerank-top 20 --limit 50
# Target: R@5 >= 0.62
```

## Cargo check
```
cargo check -p longmemeval                           → 0 errors
cargo check -p longmemeval --features "embed-768,rerank" → 0 errors
```

## Files Changed
- `bench/longmemeval/src/main.rs` — arctic-m auto-default logic + model desc strings
- `bench/longmemeval/README.md` — preferred config dokumentiert
- `bench/longmemeval/WORK_LOG.md` — dieses file
