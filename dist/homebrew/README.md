# homebrew-synapse

Homebrew tap for `synx` — the Synapse CLI.

## Install

```bash
brew tap supersynergy/synapse
brew install synx
```

## Upgrade

```bash
brew upgrade synx
```

## Tap repo setup (one-time, maintainer)

1. Create GitHub repo named `homebrew-synapse` under `Supersynergy` org
2. Copy `synx.rb` into the repo root
3. Fill in real sha256 checksums after building release tarballs:
   ```bash
   shasum -a 256 synx-aarch64-apple-darwin.tar.gz
   shasum -a 256 synx-x86_64-apple-darwin.tar.gz
   shasum -a 256 synx-x86_64-unknown-linux-gnu.tar.gz
   ```
4. Push — Homebrew auto-discovers formulas in repo root

## Tarball contents expected

```
synx                    # the binary
completions/synx.bash   # optional
completions/_synx       # optional
completions/synx.fish   # optional
```
