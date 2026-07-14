# Security policy

## Supported surface

Security fixes target the latest Synapse Memory release and `main`. Engine-lab
crates are not part of the portable security promise unless a release explicitly
names them.

| Surface | Status |
|---|---|
| Portable `synx --no-default-features` | Supported release candidate |
| `integrations/codex` checkpoint adapter | Supported optional adapter |
| MCP, daemon, semantic, cluster, market, database-proxy crates | Separate gates; not in portable artifact |

## Report privately

Email `security@supersynergy.de` with:

- affected version and target;
- minimal reproduction;
- impact and attacker prerequisites;
- suggested fix, if known.

Do not open a public issue for a live vulnerability. Do not attach real memory,
transcripts, credentials, keys, or personal data. We aim to acknowledge a complete
report within 72 hours; remediation timing depends on severity and reproducibility.

## Portable threat boundary

- Memory is local plaintext SQLite by default. Rely on OS account isolation and
  full-disk encryption for at-rest confidentiality.
- The portable binary has no network client, daemon listener, model downloader,
  PDF parser, or proprietary engine in its resolved dependency graph.
- Imports, snapshots, paths, and hook events are untrusted input. Integrity checks
  do not turn untrusted content into trusted instructions.
- Release installers require SHA-256 sidecars, reject path traversal and script
  wrappers, preserve the previous binary, and never purge memory by default.
- Codex recovery journals store compact execution metadata, not prompt text,
  command arguments, file contents, or tool-output bodies.
- Context excludes explicit Telepathy/status notifications, stale/archived rows,
  and superseded memory without deleting the underlying evidence.
- Context feedback is accepted only for document ids emitted in that pack.
- `doctor --fix` refuses work when canonical SQLite is unhealthy. Before changing
  FTS it creates a mode-0600 brainpack, restores it, and runs `quick_check`;
  canonical documents and vectors are not rewritten.

Exact audit and package gates:
[release/synapse-memory/RELEASE-GATES.md](release/synapse-memory/RELEASE-GATES.md).
Optional daemon, MCP, semantic, and hosted channels require their own threat model
and release gate before they are supported.
