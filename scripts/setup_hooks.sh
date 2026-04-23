#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
HOOKS_DIR="$REPO_ROOT/.git/hooks"

cat > "$HOOKS_DIR/pre-push" << 'HOOK'
#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

echo "==> pre-push: cargo shear"
cargo shear --fix 2>&1 || true

echo "==> pre-push: cargo fmt --check"
cargo fmt --check

echo "==> pre-push: clippy"
cargo clippy --workspace -- -D warnings

echo "==> pre-push: nextest (fast subset)"
cargo nextest run --workspace --fail-fast

echo "==> pre-push: suspicious patterns"
PATTERN='unwrap()\|panic!\|todo!()\|unimplemented!()'
HITS=$(grep -rn "$PATTERN" crates/*/src/ \
  --include='*.rs' \
  --exclude='*test*' \
  --exclude='*spec*' \
  2>/dev/null || true)
if [ -n "$HITS" ]; then
  echo "WARNING: suspicious patterns found (non-test files):"
  echo "$HITS"
fi

echo "==> pre-push: all checks passed"
HOOK

chmod +x "$HOOKS_DIR/pre-push"
echo "Installed pre-push hook at $HOOKS_DIR/pre-push"
