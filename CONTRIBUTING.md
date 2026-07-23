# Contributing to Synapse Agent Memory

Synapse Agent Memory is the release product. The wider workspace also contains engine
research. Keep pull requests inside one clearly named surface.

## Setup

Rust 1.95 is pinned by `rust-toolchain.toml`.

```sh
git clone https://github.com/Supersynergy/synapse-agent-memory
cd synapse
cargo test --locked -p synapse-cli --no-default-features
```

Portable release smoke:

```sh
cargo build --locked --profile release-hardened \
  -p synapse-cli --no-default-features
SYNX_BIN=target/release-hardened/synx \
  release/synapse-agent-memory/verify.sh
```

That verifier requires exact local versions of `cargo-audit 0.22.2`,
`cargo-deny 0.19.9`, and `cargo-about 0.9.0` with its `cli` feature.

## Pull-request contract

- State whether the change affects portable memory, an optional adapter, or the
  engine lab.
- Add a test that fails before the fix and passes after it.
- Keep new heavy or platform-specific dependencies behind a feature.
- Do not add a package published less than 14 days ago.
- Include dependency purpose, license, maintenance, and portable-closure impact.
- Never commit a brain database, checkpoint journal, transcript, keys, model
  cache, benchmark corpus, generated graph, or local absolute path.
- Document a breaking format or CLI change in `CHANGELOG.md` and an ADR.

Rust: use `synapse_core::Error` inside core libraries; `anyhow` is acceptable in
binaries. Format with rustfmt and deny Clippy warnings on the changed package.

## License of contributions

Files inherit the license declared by their crate or directory. In particular,
`synapse-core` uses FSL-1.1-ALv2; the portable CLI, graph, and learning utilities
use MIT; `synapse-engine` has separate proprietary terms. By submitting a
contribution, you agree to license it under the existing terms of its destination.

## Release process

Maintainers only:

1. Resolve the intentional diff and run the portable verifier from a clean tree.
2. Match workspace, CLI, `release/synapse-agent-memory/VERSION`, installer constants,
   release notes, and tag versions.
3. Approve the recorded FSL/MIT boundary with repository variable
   `SYNAPSE_RELEASE_LICENSE_APPROVED=true`.
4. Push `synapse-agent-memory-v<version>`; do not create another public tag family.
5. Require all six native jobs and checksum validation before publishing.

Exact gates: [release/synapse-agent-memory/RELEASE-GATES.md](release/synapse-agent-memory/RELEASE-GATES.md).

## Code of Conduct

See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
