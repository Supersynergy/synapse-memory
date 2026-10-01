# Synapse Memory

![Synapse Memory: come back tomorrow, the why is still here](docs/assets/social-preview.png)

[![License: FSL-1.1-ALv2 + MIT](https://img.shields.io/badge/license-FSL--1.1--ALv2%20%2B%20MIT-blue)](#license)
[![Platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey)](#install)

Long-term memory for coding agents such as Claude Code, Codex and Cursor.
It keeps decisions, fixes and facts in one SQLite file on your disk and gives
your agent a short brief with sources before it starts a task.

Every new agent session starts empty. The bug you fixed last week, the reason
you picked Postgres, the deploy rule your team agreed on: the agent has to ask
again. Synapse Memory stores these as dated notes and hands back the ones that
matter for the task at hand.

Everything stays on your machine. You need no account, API key, Docker or
background service.

## Install

macOS and Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/Supersynergy/synapse-memory/main/release/synapse-agent-memory/install.sh | sh
```

Windows (PowerShell):

```powershell
irm https://raw.githubusercontent.com/Supersynergy/synapse-memory/main/release/synapse-agent-memory/install.ps1 | iex
```

The installer downloads one binary, `synx`, for your system. It verifies the
SHA-256 checksum before it unpacks anything and creates your memory file at
`~/.synapse/brain.db`. An existing memory file is kept as it is.

Check that it worked:

```sh
synx --version
```

If your shell cannot find `synx`, add the install folder to your `PATH`:
`~/.local/bin` on macOS and Linux, `%LOCALAPPDATA%\Synapse\bin` on Windows.
On macOS and Linux the installer prints the exact line for you.

## First steps

Store something you want to keep:

```sh
synx remember --kind bugfix --title "Login loop" \
  "Login loop on Safari came from SameSite=Strict on the session cookie. Fixed with SameSite=Lax."
```

Ask for it later, from any folder and any session:

```sh
synx context "safari login loop"
```

The answer is a short Markdown brief that an agent can read directly:

```text
# Synapse Agent Memory Context Pack

context_id: 0cba73da100ec9d2
query: safari login loop
route: lexical
budget: 239/2400 chars
...

### [2] Login loop score=0.5970
kind: bugfix priority: normal
captured_at: 2026-10-01T00:28:01Z occurred_at: unspecified
Login loop on Safari came from SameSite=Strict on the session cookie. Fixed with SameSite=Lax.
...
```

Each entry carries its id, type and date, so the agent can cite it and you can
check it. The brief stays within a character budget (2,400 by default), so a
large memory never floods the prompt.

When a fact changes, save the new version and name the id of the old one (the
number in brackets in a context brief). The history stays in the file:

```sh
synx remember --kind fact --supersedes 3 "Staging runs Postgres 18 since 2026-09-20."
```

From then on, context briefs show the new fact and skip the old one.

## Connect your agent

Paste this into the `AGENTS.md` or `CLAUDE.md` of your project. Codex and Cursor
read `AGENTS.md`, Claude Code reads `CLAUDE.md`.

```markdown
## Memory
- Before a non-trivial task, run `synx context "<task>" --mode coding` and read the result.
- After a decision, a fix or a lesson, save it:
  `synx remember --kind decision|bugfix|fact "<what happened and why>"`.
- If a memory helped, run the `synx feedback` command printed in the brief.
```

At the start of a session, `synx prime .` briefs the agent on the repository in
front of it. The brief lists Git state, key documents, test commands and recent
memories.

Codex users can add crash-safe resume. After an interrupted session, the next
one starts with a short checkpoint that names the working directory, the last
tool and the changed files. It never stores the conversation, file contents or
tool output.
Setup: [integrations/codex](integrations/codex/README.md).

## Everyday commands

| You want to | Command |
|---|---|
| Save a decision, fix or fact | `synx remember --kind decision "..."` |
| Replace an outdated memory | `synx remember --supersedes <id> "..."` |
| Get context for a task | `synx context "<task>" --mode coding` |
| Brief a new session on a repo | `synx prime .` |
| Rate a context brief | `synx feedback context:<id> <doc_id> --gate pass` |
| Check health, repair the search index | `synx doctor --fix` |
| Back up your memory | `synx backup brain-backup.synx` |
| Restore a backup | `synx db-restore brain-backup.synx` |

`--kind` accepts `decision`, `fact`, `preference`, `bugfix`, `benchmark`,
`command`, `session`, `adr`, `research` and `note`. Run `synx <command> --help`
for every option.

## Where your memory lives

`synx` uses `~/.synapse/brain.db`. If a folder contains its own
`.synapse/brain.db`, commands run inside that folder use the project file
instead. Pass `-f <file>` to choose a file yourself. On Windows, pass
`-f $env:USERPROFILE\.synapse\brain.db` explicitly.

The file is plain SQLite. You can copy it, back it up with any tool and open it
with `sqlite3`. Synapse Memory stores only what you or your agent save with
`remember`. It never records transcripts.

To uninstall on macOS or Linux, delete `~/.local/bin/synx` and, after an
upgrade, `~/.local/bin/synx.previous`. Your memory file stays until you delete
it yourself.

## What the download includes

The installer ships the portable release, version `1.1.0-rc.3`. It includes
the `synx` command line with keyword search, cited context briefs, typed and
dated memories, supersession, feedback, health checks, backup, restore, merge
and Ed25519 signatures. It runs on macOS, Linux and Windows, each on x86-64
and ARM64.

Search in the download is keyword-based. Vector search, the `synapsed` daemon
and the `synapse-mcp` MCP server are part of this repository and need a build
from source. MCP setup for Claude Code and Cursor:
[crates/synapse-mcp](crates/synapse-mcp/README.md).

Exact capability list: [FEATURES.md](release/synapse-agent-memory/FEATURES.md).
How each release is verified: [PROOF.md](release/synapse-agent-memory/PROOF.md).

## Build from source

You need a Rust toolchain ([rustup](https://rustup.rs)).

```sh
git clone https://github.com/Supersynergy/synapse-memory.git
cd synapse-memory
cargo build --release --locked -p synapse-cli
./target/release/synx --version
```

The first build takes a few minutes. A source build of `main` reports its
workspace version (`2.1.0`), which differs from the release numbering.

Layer map: [ARCHITECTURE.md](ARCHITECTURE.md). Workflow and checks:
[CONTRIBUTING.md](CONTRIBUTING.md).

## License

- `synapse-core` uses [FSL-1.1-ALv2](LICENSES/FSL-1.1-ALv2.txt). You may use,
  change and share it for any purpose except a competing commercial product.
  Two years after each release, that release becomes Apache-2.0.
- The command line and most other crates use [MIT](LICENSES/MIT.txt).
  `synapse-kernel` and `synapse-embed-gpu` use MIT or Apache-2.0.
- `synapse-engine` is proprietary and is not part of the download. See
  [LICENSE-ENGINE.md](LICENSE-ENGINE.md).

Every release archive contains both license texts and a list of all
third-party dependencies with their licenses.

## Help and contributing

- Bugs and questions: [GitHub Issues](https://github.com/Supersynergy/synapse-memory/issues)
- Security reports: [SECURITY.md](SECURITY.md)
- Contributing: [CONTRIBUTING.md](CONTRIBUTING.md)
- Changes per version: [CHANGELOG.md](CHANGELOG.md)
