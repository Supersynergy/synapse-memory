# Synapse Fuzz Targets

Cargo-fuzz infrastructure for `.synx` format hardening.

## Requirements

- Rust nightly: `rustup install nightly`
- cargo-fuzz: `cargo install cargo-fuzz`

## Targets

### `synx_deserialize`

Feeds random bytes into the `.synx` header / footer / chunk parsers.
Verifies no panic occurs on arbitrary input.

**Seed corpus**: `corpus/synx_deserialize/` — 10 valid `.synx` files
generated from `synapse-core`'s writer (run `gen_fuzz_corpus` test to regenerate).

## Run

```bash
# From workspace root
cargo +nightly fuzz run synx_deserialize --fuzz-dir fuzz

# With corpus
cargo +nightly fuzz run synx_deserialize --fuzz-dir fuzz fuzz/corpus/synx_deserialize

# Minimize a crash
cargo +nightly fuzz tmin synx_deserialize --fuzz-dir fuzz <artifact>

# Coverage report
cargo +nightly fuzz coverage synx_deserialize --fuzz-dir fuzz
```

## Regenerate seed corpus

```bash
cargo test --package synapse-core --test gen_fuzz_corpus generate_fuzz_corpus_seeds
```
