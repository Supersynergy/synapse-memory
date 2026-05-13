# Launch Checklist — Synapse v1.0.1-rc.1

## Pre-tag (local, blocking)

- [ ] `cargo nextest run --workspace` — all green
- [ ] `cargo deny check` — no denied deps
- [ ] `cargo clippy --workspace -- -D warnings` — 0 warnings
- [ ] `CHANGELOG.md` updated with `[1.0.1-rc.1]` section + date
- [ ] `README.md` verified-numbers match `RELEASE_NOTES_v1.0.1-rc.1.md`
- [ ] `synx --version` returns `synapse 1.0.1-rc.1`
- [ ] `dist/npm/package.json` version = `1.0.1-rc.1`
- [ ] `crates/synapse-py/pyproject.toml` version = `1.0.1rc1`
- [ ] `dist/homebrew/synx.rb` version = `1.0.1-rc.1`, URLs point to rc.1 tag

## Tag + Push (user-action, triggers CI)

- [ ] `git tag v1.0.1-rc.1`
- [ ] `git push origin v1.0.1-rc.1`
- [ ] CI green: `cargo build --release --workspace` on Linux x86_64
- [ ] CI uploads: `synx-aarch64-apple-darwin.tar.gz`, `synx-x86_64-apple-darwin.tar.gz`, `synx-x86_64-unknown-linux-gnu.tar.gz`

## Post-CI binary updates

- [ ] Update `dist/homebrew/synx.rb` SHA256 for all 3 tarballs (run: `sha256sum synx-*.tar.gz`)
- [ ] Commit updated formula to `homebrew-synapse` tap repo
- [ ] Push tap: `git push origin main` in `homebrew-synapse`

## npm

- [ ] `export NPM_TOKEN=<token>`
- [ ] `cd dist/npm && npm publish --access public --tag rc`
- [ ] Verify: `npm info @supersynergy/synx dist-tags`

## PyPI

- [ ] `RUSTFLAGS="-C link-arg=-undefined -C link-arg=dynamic_lookup" maturin publish -m crates/synapse-py/Cargo.toml --token $PYPI_TOKEN`
- [ ] Verify: `pip install synapse-rs==1.0.1rc1`

## GitHub Release

- [ ] `gh release create v1.0.1-rc.1 --prerelease --title "Synapse v1.0.1-rc.1" --notes-file RELEASE_NOTES_v1.0.1-rc.1.md`
- [ ] Upload binaries to release if CI didn't attach them

## Announce

- [ ] Discord `#releases` — paste highlights + install snippet
- [ ] X / Twitter — killer numbers thread (71× SimSIMD, 56× vs Qdrant insert, 8ms hybrid)
- [ ] HN Show HN — include honest caveats (real bench doc link)
- [ ] Reddit r/rust, r/MachineLearning

---

## Known-Blockers Before Stable (1.0.1 final)

- SHA256 placeholders in brew formula → fill after CI
- `synapse-extract` Linux link-order bug → fix before stable
- Python wheel not yet on PyPI → maturin publish step above
- HNSW build time 197–672s for 1M (parallel insert TODO)
