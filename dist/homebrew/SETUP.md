# Distribution Setup — Brew / npm / PyPI

This document covers the one-time setup steps for each distribution channel.
Run these **after** the GitHub release is created and binaries are uploaded.

---

## 1. Homebrew Tap

### 1.1 Create the tap repo

```bash
# On GitHub: create public repo  Supersynergy/homebrew-synapse
# Then clone locally
git clone git@github.com:Supersynergy/homebrew-synapse.git ~/projects/homebrew-synapse
```

### 1.2 Copy the formula

```bash
cp /Users/master/projects/synapse/dist/homebrew/synx.rb \
   ~/projects/homebrew-synapse/Formula/synx.rb
```

### 1.3 Fill SHA256 placeholders

After `release.yml` uploads tarballs to the GitHub release, download each one and compute:

```bash
curl -L https://github.com/Supersynergy/synapse/releases/download/v1.0.1/synx-aarch64-apple-darwin.tar.gz \
  | shasum -a 256
# → paste as PLACEHOLDER_SHA256_AARCH64_APPLE_DARWIN

curl -L https://github.com/Supersynergy/synapse/releases/download/v1.0.1/synx-x86_64-apple-darwin.tar.gz \
  | shasum -a 256

curl -L https://github.com/Supersynergy/synapse/releases/download/v1.0.1/synx-x86_64-unknown-linux-gnu.tar.gz \
  | shasum -a 256
```

Edit `~/projects/homebrew-synapse/Formula/synx.rb` replacing each `PLACEHOLDER_SHA256_*`.

### 1.4 Commit and push

```bash
cd ~/projects/homebrew-synapse
git add Formula/synx.rb
git commit -m "feat: synx v1.0.1"
git push
```

### 1.5 Test locally

```bash
brew tap Supersynergy/synapse
brew install synx
synx --version   # should print 1.0.1
```

### 1.6 Future updates

- Bump `version` in `synx.rb`
- Update three `url` lines and three `sha256` lines
- Push to homebrew-synapse; users get it via `brew upgrade synx`

---

## 2. npm (`@supersynergy/synx`)

### 2.1 Pre-requisites

```bash
# npm account with publish rights to @supersynergy scope
# NPM_TOKEN set in env (or ~/.npmrc)
export NPM_TOKEN=npm_xxxx
```

### 2.2 Verify package

```bash
cd /Users/master/projects/synapse/dist/npm
cat package.json   # version should be 1.0.1-rc.1 (or strip -rc.1 for stable)
```

Ensure `postinstall.js` references the correct GitHub release download URL for the current version.

### 2.3 Publish

```bash
cd /Users/master/projects/synapse/dist/npm
npm publish --access public
# For release candidate (won't install by default):
npm publish --access public --tag rc
```

### 2.4 Test

```bash
npm install -g @supersynergy/synx@rc
synx --version
```

### 2.5 Stable promotion

When ready to promote RC → stable:

```bash
npm dist-tag add @supersynergy/synx@1.0.1-rc.1 latest
```

---

## 3. PyPI (`synapse-rs` via maturin)

### 3.1 Pre-requisites

```bash
# uv + maturin
uv pip install maturin -p ~/.venvs/maturin
export PYPI_TOKEN=pypi-xxxx
```

### 3.2 Build wheels

```bash
cd /Users/master/projects/synapse/crates/synapse-py
maturin build --release --features pyo3/extension-module
# Output: target/wheels/synapse_rs-1.0.1rc1-*.whl
```

For cross-platform wheels (CI-recommended, see `.github/workflows/release.yml`):

```bash
maturin publish -u __token__ -p $PYPI_TOKEN
# maturin handles sdist + manylinux wheel upload in one step
```

### 3.3 Test

```bash
pip install synapse-rs==1.0.1rc1 --pre
python -c "import synapse_py; print('ok')"
```

### 3.4 Stable promotion

Bump `version` in `crates/synapse-py/pyproject.toml` to `1.0.1`, rebuild, republish.

---

## 4. GitHub Release (prerequisite for all above)

1. Ensure `release.yml` workflow is triggered (push a tag `v1.0.1-rc.1` or run manually).
2. Workflow builds 3 targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`.
3. Tarballs are uploaded as release assets automatically.
4. Only after assets are live: run steps 1.3 (sha256), 2.3 (npm publish), 3.2 (maturin publish).

**User action**: `git tag v1.0.1-rc.1 && git push origin v1.0.1-rc.1`
