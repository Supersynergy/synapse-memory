#!/bin/bash
# Install synapse self-learning git hooks
REPO_ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
HOOK="$REPO_ROOT/.git/hooks/post-commit"

cat > "$HOOK" << 'EOF'
#!/bin/bash
COMMIT_MSG=$(git log -1 --pretty=%B)
COMMIT_HASH=$(git rev-parse HEAD)
FILES=$(git diff-tree --no-commit-id --name-only -r HEAD | head -10 | tr '\n' ' ')

if command -v synx &>/dev/null; then
  synx put "commit:$COMMIT_HASH | files:$FILES | msg:$COMMIT_MSG" --tag synapse-dev 2>/dev/null || true
else
  ENTRY="{\"ts\":\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\",\"commit\":\"$COMMIT_HASH\",\"files\":\"$FILES\",\"msg\":$(echo "$COMMIT_MSG" | python3 -c 'import sys,json;print(json.dumps(sys.stdin.read().strip()))')}"
  echo "$ENTRY" >> ~/.synapse/dev-log.jsonl
fi
EOF

chmod +x "$HOOK"
echo "post-commit hook installed at $HOOK"
