# Archive manifest

Every release asset contains exactly one top-level folder:

```text
synapse-agent-memory-<rust-target>/
├── synx[.exe]
├── README.md
├── THIRD-PARTY-LICENSES.html
├── NOTICE
├── ATTRIBUTIONS.md
├── LICENSES/
│   ├── FSL-1.1-ALv2.txt
│   └── MIT.txt
└── BUILD-INFO.json
```

Each archive has a sibling `<archive>.sha256` file. The installer fails closed when
the sidecar is absent or mismatched.

Never include:

- `brain.db`, WAL/SHM files, `.synx` backups, corpus data, embeddings, model caches
- signing keys, verifying keys from a maintainer machine, tokens, environment files
- Codex/Claude transcripts, checkpoints, hooks history, file-history, telemetry
- `target/`, source workspace, benchmark datasets, graph outputs, unrelated generated reports
- Python/Node wrappers masquerading as `synx`
- daemon, MCP, market, database proxy, multimodal, cluster, or experimental binaries

The source repository remains the source distribution. End users receive the
small binary archive; source builders use the exact locked tag.
