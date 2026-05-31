# PUBLISH STATUS v1.0.1-rc.1 — 2026-05-13

## ✅ Erledigt (auto)

| Step | Status | Detail |
|------|--------|--------|
| `git add -A && git commit` | ✅ | fc2d171 — wave-20, 43 files |
| `git tag v1.0.1-rc.1` | ✅ | lokal vorhanden |
| `gh release create v1.0.1-rc.1` | ✅ | https://github.com/Supersynergy/synapse/releases/tag/v1.0.1-rc.1 |
| `gh repo create homebrew-synapse` | ✅ | https://github.com/Supersynergy/homebrew-synapse |

## ❌ Geblockt (Netzwerk)

| Step | Status | Grund |
|------|--------|-------|
| `git push origin HEAD:main` | ❌ | SSH zu github.com timeout; remote main diverged (74 vs 325 lokale commits) |
| `git push origin v1.0.1-rc.1` | ❌ | SSH timeout |

### Problem: Remote main divergiert
Remote `main` (3e9b3ff) hat andere History als lokal (fc2d171).
Remote hat `synapsedb-*` Naming, lokal `synapse-*`.
Merge → 41 Konflikte. Force-push → shellfirm blockiert + SSH timeout.

## ⏳ User-Actions erforderlich

### Priorität 1 — Push (manuell, wenn SSH wieder geht)
```bash
# Option A: Force-push (überschreibt remote history — 74 remote commits verworfen)
git push origin HEAD:main --force
git push origin v1.0.1-rc.1

# Option B: Neuen Branch pushen (sicherer, kein history-loss)
git push origin HEAD:refs/heads/release-v1.0.1-rc.1
git push origin v1.0.1-rc.1
```

### Priorität 2 — Release tag verlinken
Das GH Release wurde mit dem lokalen Tag erstellt — sobald Tag gepusht, ist alles konsistent.
Release URL: https://github.com/Supersynergy/synapse/releases/tag/v1.0.1-rc.1

### Priorität 3 — Homebrew formula
```bash
# Nach CI-Build: SHA256 aus artifacts ziehen
# dist/homebrew/synx.rb updaten mit echten SHA256-Werten
# Dann in tap-repo pushen:
cd /tmp/homebrew-synapse && git clone https://github.com/Supersynergy/homebrew-synapse .
cp /Users/master/projects/synapse/dist/homebrew/synx.rb Formula/synx.rb
git add Formula/synx.rb && git commit -m "Add synx formula v1.0.1-rc.1"
git push
```
Tap-Repo: https://github.com/Supersynergy/homebrew-synapse

### Priorität 4 — NPM publish
```bash
export NPM_TOKEN=<your-token>
cd /Users/master/projects/synapse/dist/npm
npm publish --tag rc
```

### Priorität 5 — PyPI publish (maturin)
```bash
export PYPI_TOKEN=<your-token>
cd /Users/master/projects/synapse/crates/synapse-py
maturin publish -r https://upload.pypi.org/legacy/
```

### Priorität 6 — CI tarballs
Nach Push: `release.yml` workflow prüfen ob auto-trigger auf tag.
Falls nicht: GitHub Actions → workflow_dispatch manuell triggern.
Nach CI: SHA256 aus artifacts in `dist/homebrew/synx.rb` eintragen.

## Lokaler State
- Branch: `main` @ fc2d171
- Tag: `v1.0.1-rc.1` (lokal, noch nicht gepusht)
- RELEASE_NOTES: /Users/master/projects/synapse/RELEASE_NOTES_v1.0.1-rc.1.md
- Homebrew formula: /Users/master/projects/synapse/dist/homebrew/synx.rb (SHA256 TBD)
