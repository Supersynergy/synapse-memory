# 04 — CI/CD Workflows for the Split Repos

This document defines the continuous-integration / release design for the two
product repos produced by the Synapse split, plus a ready-to-drop set of
GitHub Actions workflow files.

- **`synapse-db`** — the engine product. Holds the shared foundation crates
  (`kernel`, `core`, `engine`, `ann`, `fts`, `graph`, `quant`, `spann`, `obs`)
  plus all DB / SQL-wire / distributed / server surfaces. Self-contained: no
  internal dependency on any memory crate.
- **`synapse-memory`** — the agent-memory product. Depends on the `synapse-db`
  foundation crates, consumed via a **git submodule** at `vendor/synapse-db`
  with Cargo **path** deps (local-first; no registry required).
- **`synapse-market`** — vertical spinoff (own repo). The `synapse-market*`
  CI workflows move there; `synapse-mcp -> synapse-market` dependency is **cut**.

Toolchain ground truth (from the monorepo `Cargo.toml`): **edition 2024**,
**MSRV 1.95**, workspace build with `cargo nextest` + `clippy -D warnings`.

---

## 1. Design principles (current best practice, May 2026)

| Principle | Decision |
|-----------|----------|
| Toolchain pin | `dtolnay/rust-toolchain@stable` (+ explicit `1.95` MSRV job). No `actions-rs/*` (unmaintained). |
| Caching | `Swatinem/rust-cache@v2` keyed per-job; saves only on the default branch to keep PR caches read-only and clean. |
| Tool install | `taiki-e/install-action@v2` for `nextest`, `cargo-deny`, `cargo-audit` (prebuilt binaries, no `cargo install` compile cost). |
| Lint gate | `cargo clippy --all-targets --all-features -- -D warnings` + `cargo fmt --all --check`. |
| Test runner | `cargo nextest run --all-features` (faster, better isolation than `cargo test`). |
| Concurrency | `concurrency:` group per ref with `cancel-in-progress` to kill superseded PR runs. |
| Permissions | Least-privilege top-level `permissions: contents: read`; release job elevates to `contents: write` only where needed. |
| Triggers | CI on `push` (main) + `pull_request`; security weekly `schedule` + PR; release on `v*` tags. |
| Submodule (memory only) | `actions/checkout@v4` with `submodules: recursive` so `vendor/synapse-db` foundation crates resolve via path deps. |
| Heavy artifacts | `target/` and `examples/**/target`, `*.db`, `*.parquet` stay gitignored; CI never commits build output. |

---

## 2. Existing workflow disposition (11 files)

| Existing workflow | Action | Lands in | Rationale |
|-------------------|--------|----------|-----------|
| `ci.yml` | **Merge** → new `ci.yml` | both | Consolidate the three overlapping CI files into one canonical pipeline. |
| `rust-ci.yml` | **Merge** (drop file) | both | Overlaps `ci.yml`; fold its steps in, delete the file. |
| `quality.yml` | **Merge** (drop file) | both | fmt/clippy belong in the single `ci.yml` quality gate. |
| `security.yml` | **Keep** (refresh) | both | Becomes the canonical `cargo-deny` + `cargo-audit` workflow below. |
| `release.yml` | **Keep** (refresh) | both | Becomes the matrix tag-release workflow below. |
| `docs.yml` | **Keep** (lightly adapt) | both | `cargo doc` build; scope each repo to its own crates. |
| `bench.yml` | **Keep, manual** | both | Convert to `workflow_dispatch` + on-demand; not on every PR. |
| `bench-nightly.yml` | **Keep, scheduled** | both | Nightly perf trend; split by repo (db engine vs memory recall benches). |
| `linux-bench.yml` | **Merge into bench-nightly** | both | Fold Linux runner into the nightly bench matrix; delete the file. |
| `synapse-market-ci.yml` | **Move** | `synapse-market` | Vertical spinoff owns its CI. |
| `synapse-market.yml` | **Move** | `synapse-market` | Vertical spinoff owns its release/publish. |

Net result per engine/memory repo: **`ci.yml`, `security.yml`, `release.yml`,
`docs.yml`, `bench-nightly.yml`** (5 files), down from the 8 non-market files,
with `rust-ci.yml`/`quality.yml`/`linux-bench.yml` folded in and deleted.

---

## 3. `synapse-db` — workflows

### 3.1 `.github/workflows/ci.yml`

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

permissions:
  contents: read

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  quality:
    name: fmt + clippy
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - name: rustfmt
        run: cargo fmt --all --check
      - name: clippy
        run: cargo clippy --all-targets --all-features --workspace -- -D warnings

  check:
    name: check + test
    needs: quality
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - uses: taiki-e/install-action@v2
        with:
          tool: nextest
      - name: cargo check
        run: cargo check --workspace --all-targets --all-features
      - name: cargo nextest
        run: cargo nextest run --workspace --all-features --no-tests=pass

  msrv:
    name: MSRV 1.95
    needs: quality
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.95
      - uses: Swatinem/rust-cache@v2
      - name: check on MSRV
        run: cargo check --workspace --all-features
```

### 3.2 `.github/workflows/security.yml`

```yaml
name: Security

on:
  pull_request:
  schedule:
    - cron: "0 6 * * 1"   # Mondays 06:00 UTC
  workflow_dispatch:

permissions:
  contents: read

concurrency:
  group: security-${{ github.ref }}
  cancel-in-progress: true

jobs:
  deny:
    name: cargo-deny
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: taiki-e/install-action@v2
        with:
          tool: cargo-deny
      - name: cargo deny check
        run: cargo deny check

  audit:
    name: cargo-audit
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: taiki-e/install-action@v2
        with:
          tool: cargo-audit
      - name: cargo audit
        run: cargo audit --deny warnings
```

### 3.3 `.github/workflows/release.yml`

```yaml
name: Release

on:
  push:
    tags: ["v*"]

permissions:
  contents: write   # needed to create the release + upload assets

concurrency:
  group: release-${{ github.ref }}
  cancel-in-progress: false

env:
  CARGO_TERM_COLOR: always
  # Primary shippable binary for the engine product.
  BIN_NAME: synapsed

jobs:
  build:
    name: build ${{ matrix.target }}
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
          - os: ubuntu-latest
            target: aarch64-unknown-linux-gnu
          - os: macos-latest
            target: x86_64-apple-darwin
          - os: macos-latest
            target: aarch64-apple-darwin
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - uses: Swatinem/rust-cache@v2
      - name: install cross linker (linux aarch64)
        if: matrix.target == 'aarch64-unknown-linux-gnu'
        run: |
          sudo apt-get update
          sudo apt-get install -y gcc-aarch64-linux-gnu
          mkdir -p .cargo
          printf '[target.aarch64-unknown-linux-gnu]\nlinker = "aarch64-linux-gnu-gcc"\n' >> .cargo/config.toml
      - name: build release
        run: cargo build --release --locked --bin "$BIN_NAME" --target ${{ matrix.target }}
      - name: package
        shell: bash
        run: |
          set -euo pipefail
          STAGE="${BIN_NAME}-${{ github.ref_name }}-${{ matrix.target }}"
          mkdir -p "dist/$STAGE"
          cp "target/${{ matrix.target }}/release/${BIN_NAME}" "dist/$STAGE/"
          cp README.md LICENSE* "dist/$STAGE/" 2>/dev/null || true
          tar -C dist -czf "dist/$STAGE.tar.gz" "$STAGE"
          ( cd dist && shasum -a 256 "$STAGE.tar.gz" > "$STAGE.tar.gz.sha256" )
      - uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.target }}
          path: dist/*.tar.gz*

  publish:
    name: publish release
    needs: build
    runs-on: ubuntu-latest
    permissions:
      contents: write
    steps:
      - uses: actions/download-artifact@v4
        with:
          path: dist
          merge-multiple: true
      - name: create release
        uses: softprops/action-gh-release@v2
        with:
          files: dist/*
          generate_release_notes: true
          fail_on_unmatched_files: true
```

---

## 4. `synapse-memory` — workflows

Difference from `synapse-db`: **every checkout uses `submodules: recursive`**
so the `vendor/synapse-db` foundation crates (referenced by Cargo path deps /
`[patch]`) resolve. Everything else mirrors the engine repo.

### 4.1 `.github/workflows/ci.yml`

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

permissions:
  contents: read

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  quality:
    name: fmt + clippy
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          submodules: recursive       # pulls vendor/synapse-db foundation crates
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - name: rustfmt
        run: cargo fmt --all --check
      - name: clippy
        run: cargo clippy --all-targets --all-features --workspace -- -D warnings

  check:
    name: check + test
    needs: quality
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
        with:
          submodules: recursive
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - uses: taiki-e/install-action@v2
        with:
          tool: nextest
      - name: cargo check
        run: cargo check --workspace --all-targets --all-features
      - name: cargo nextest
        run: cargo nextest run --workspace --all-features --no-tests=pass

  msrv:
    name: MSRV 1.95
    needs: quality
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          submodules: recursive
      - uses: dtolnay/rust-toolchain@1.95
      - uses: Swatinem/rust-cache@v2
      - name: check on MSRV
        run: cargo check --workspace --all-features
```

### 4.2 `.github/workflows/security.yml`

```yaml
name: Security

on:
  pull_request:
  schedule:
    - cron: "0 6 * * 1"   # Mondays 06:00 UTC
  workflow_dispatch:

permissions:
  contents: read

concurrency:
  group: security-${{ github.ref }}
  cancel-in-progress: true

jobs:
  deny:
    name: cargo-deny
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          submodules: recursive
      - uses: dtolnay/rust-toolchain@stable
      - uses: taiki-e/install-action@v2
        with:
          tool: cargo-deny
      - name: cargo deny check
        run: cargo deny check

  audit:
    name: cargo-audit
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          submodules: recursive
      - uses: dtolnay/rust-toolchain@stable
      - uses: taiki-e/install-action@v2
        with:
          tool: cargo-audit
      - name: cargo audit
        run: cargo audit --deny warnings
```

### 4.3 `.github/workflows/release.yml`

```yaml
name: Release

on:
  push:
    tags: ["v*"]

permissions:
  contents: write

concurrency:
  group: release-${{ github.ref }}
  cancel-in-progress: false

env:
  CARGO_TERM_COLOR: always
  # Primary shippable binary for the memory product (CLI + daemon).
  BIN_NAME: synapse-cli

jobs:
  build:
    name: build ${{ matrix.target }}
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
          - os: ubuntu-latest
            target: aarch64-unknown-linux-gnu
          - os: macos-latest
            target: x86_64-apple-darwin
          - os: macos-latest
            target: aarch64-apple-darwin
    steps:
      - uses: actions/checkout@v4
        with:
          submodules: recursive       # foundation crates needed to build
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - uses: Swatinem/rust-cache@v2
      - name: install cross linker (linux aarch64)
        if: matrix.target == 'aarch64-unknown-linux-gnu'
        run: |
          sudo apt-get update
          sudo apt-get install -y gcc-aarch64-linux-gnu
          mkdir -p .cargo
          printf '[target.aarch64-unknown-linux-gnu]\nlinker = "aarch64-linux-gnu-gcc"\n' >> .cargo/config.toml
      - name: build release
        run: cargo build --release --locked --bin "$BIN_NAME" --target ${{ matrix.target }}
      - name: package
        shell: bash
        run: |
          set -euo pipefail
          STAGE="${BIN_NAME}-${{ github.ref_name }}-${{ matrix.target }}"
          mkdir -p "dist/$STAGE"
          cp "target/${{ matrix.target }}/release/${BIN_NAME}" "dist/$STAGE/"
          cp README.md LICENSE* "dist/$STAGE/" 2>/dev/null || true
          tar -C dist -czf "dist/$STAGE.tar.gz" "$STAGE"
          ( cd dist && shasum -a 256 "$STAGE.tar.gz" > "$STAGE.tar.gz.sha256" )
      - uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.target }}
          path: dist/*.tar.gz*

  publish:
    name: publish release
    needs: build
    runs-on: ubuntu-latest
    permissions:
      contents: write
    steps:
      - uses: actions/download-artifact@v4
        with:
          path: dist
          merge-multiple: true
      - name: create release
        uses: softprops/action-gh-release@v2
        with:
          files: dist/*
          generate_release_notes: true
          fail_on_unmatched_files: true
```

> Submodule note: when bumping the pinned `synapse-db` commit, the
> memory-repo author updates `vendor/synapse-db` and commits the new submodule
> SHA. CI builds against exactly that pin — reproducible, registry-free.
> Option B (publish foundation crates to a private registry and depend by
> version) removes the submodule but adds a publish step; the submodule path
> is the local-first default.

---

## 5. `justfile` verbs to standardize (both repos)

Both repos expose the same stable verbs so agents and CI call identical
commands locally and in Actions.

| Verb | Command | Used by |
|------|---------|---------|
| `setup` | `rustup show && cargo fetch` (memory: `git submodule update --init --recursive` first) | dev bootstrap |
| `fmt` | `cargo fmt --all` | dev |
| `lint` | `cargo clippy --all-targets --all-features --workspace -- -D warnings` | `ci.yml` quality |
| `check` | `cargo check --workspace --all-targets --all-features` | `ci.yml` check |
| `test` | `cargo nextest run --workspace --all-features --no-tests=pass` | `ci.yml` check |
| `build` | `cargo build --release --locked` | `release.yml` |
| `ci` | `just fmt --check && just lint && just check && just test` | local pre-push mirror of CI |
| `release` | `cargo build --release --locked --bin <BIN>` (db: `synapsed`, memory: `synapse-cli`) | `release.yml` |

```just
# justfile (shared shape; memory repo adds the submodule line to `setup`)
set shell := ["bash", "-uc"]

setup:
    cargo fetch

fmt:
    cargo fmt --all

lint:
    cargo clippy --all-targets --all-features --workspace -- -D warnings

check:
    cargo check --workspace --all-targets --all-features

test:
    cargo nextest run --workspace --all-features --no-tests=pass

build:
    cargo build --release --locked

ci: fmt lint check test

release BIN:
    cargo build --release --locked --bin {{BIN}}
```

For `synapse-memory`, prepend the submodule sync to `setup`:

```just
setup:
    git submodule update --init --recursive
    cargo fetch
```

---

## 6. Verification checklist before adopting

- [ ] `synapse-db` workspace builds standalone (no memory crate in tree).
- [ ] `synapse-memory` workspace builds only after `git submodule update --init --recursive`.
- [ ] `synapse-mcp -> synapse-market` dependency removed (else memory CI fails resolving market).
- [ ] `cargo deny` config (`deny.toml`) present in each repo (advisories + licenses + bans).
- [ ] `Cargo.lock` committed in both repos so `--locked` release builds are reproducible.
- [ ] `synapse-market*` workflows deleted from db/memory and present only in `synapse-market`.
- [ ] `.gitignore` carries `target/`, `examples/**/target`, `*.db`, `*.parquet` (no built artifacts committed).
