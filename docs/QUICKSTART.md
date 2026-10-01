# Quickstart: Synapse Memory

Goal: from zero to a working agent memory in under 10 minutes. Install `synx`,
create a brain, store and recall a memory, then wire it into Claude Desktop,
Cursor, or Windsurf over MCP.

Layout of the pieces:

- `synx`: the CLI. Works standalone on one SQLite file, no daemon needed.
- `synapsed`: optional daemon. Serves the brain over a unix socket on
  macOS/Linux (default `/tmp/synapse.sock`) and over loopback TCP
  `127.0.0.1:9477` on Windows. Required for MCP.
- `synapse-mcp`: stdio MCP bridge between your agent client and `synapsed`.

Auth: `synapsed` requires a shared token for every op except Ping/Stats/Auth.
It auto-generates `auth.token` (mode 0600) next to the brain DB on first start,
and `synapse-mcp` reads it automatically (`SYNAPSE_API_KEY` or
`SYNAPSE_AUTH_TOKEN_FILE` override it).

---

## macOS / Linux

### 1. Install `synx`

Prebuilt release (installs `synx`, verifies the SHA-256 sidecar, creates
`~/.synapse/brain.db`):

```bash
curl -fsSL https://raw.githubusercontent.com/Supersynergy/synapse-memory/main/release/synapse-agent-memory/install.sh | sh
```

Or build from source (needed for the daemon and MCP bridge):

```bash
git clone https://github.com/Supersynergy/synapse-memory.git
cd synapse-memory
cargo build --release -p synapse-cli -p synapsed -p synapse-mcp
# binaries: target/release/{synx,synapsed,synapse-mcp}
```

### 2. Init + smoke test

```bash
synx -f ~/.synapse/brain.db init                 # creates the brain
synx -f ~/.synapse/brain.db put --title "first" --text "Synapse smoke test"
synx -f ~/.synapse/brain.db find "smoke"         # FTS5 lexical search
synx -f ~/.synapse/brain.db hybrid "smoke test"  # FTS5 + vector + RRF
synx -f ~/.synapse/brain.db context "smoke test" # bounded cited context pack
synx -f ~/.synapse/brain.db doctor               # health check
```

Note: the first `put`/`hybrid`/`vec`/`context`/`remember` on a default build
downloads the BGE-small embedding model (fastembed, ~100 MB) once. `find` and
`put --no-embed` work without it.

### 3. Start the daemon

```bash
synapsed -f ~/.synapse/brain.db
# listens on /tmp/synapse.sock, writes ~/.synapse/auth.token
```

### 4. Wire MCP into your client

All three clients take a `mcpServers` entry. Use the absolute path to the
binary if it is not on the client's PATH.

```json
{
  "mcpServers": {
    "synapse": {
      "command": "synapse-mcp",
      "args": ["--sock", "/tmp/synapse.sock"]
    }
  }
}
```

Config file locations:

- Claude Desktop: `~/Library/Application Support/Claude/claude_desktop_config.json`
- Cursor: `~/.cursor/mcp.json` (global) or `.cursor/mcp.json` in the project
- Windsurf: `~/.codeium/windsurf/mcp_config.json`

Restart the client, then ask it to call `context_pack` or `memory_search`.
The most useful tools: `context_pack` (budgeted verbatim context),
`context_remember` / `memory_save` (write), `memory_search` (hybrid recall),
`memory_recent`, `why` (decision chain).

### 5. Optional: autostart the daemon

- macOS (launchd): create `~/Library/LaunchAgents/de.supersynergy.synapsed.plist`
  running `synapsed -f ~/.synapse/brain.db` with `RunAtLoad`, then
  `launchctl load ~/Library/LaunchAgents/de.supersynergy.synapsed.plist`.
- Linux (systemd user): unit at `~/.config/systemd/user/synapsed.service` with
  `ExecStart=%h/.local/bin/synapsed -f %h/.synapse/brain.db`, then
  `systemctl --user enable --now synapsed`.

---

## Windows (PowerShell)

### 1. Install `synx`

```powershell
irm https://raw.githubusercontent.com/Supersynergy/synapse-memory/main/release/synapse-agent-memory/install.ps1 | iex
```

Installs `synx.exe` to `%LOCALAPPDATA%\Synapse\bin` (added to your user `Path`)
and initializes `%USERPROFILE%\.synapse\brain.db`. Windows on ARM64 is not
supported yet — the vendored ANN stack (`usearch`/`numkong`) cannot compile
under MSVC arm64; use x64 Windows, WSL2, or another OS.

Or build from source (needed for the daemon and MCP bridge):

```powershell
git clone https://github.com/Supersynergy/synapse-memory.git
cd synapse-memory
cargo build --release -p synapse-cli -p synapsed -p synapse-mcp
# binaries: target\release\{synx.exe,synapsed.exe,synapse-mcp.exe}
```

### 2. Init + smoke test

```powershell
synx -f "$env:USERPROFILE\.synapse\brain.db" init
synx -f "$env:USERPROFILE\.synapse\brain.db" put --title "first" --text "Synapse smoke test"
synx -f "$env:USERPROFILE\.synapse\brain.db" find "smoke"
synx -f "$env:USERPROFILE\.synapse\brain.db" hybrid "smoke test"
synx -f "$env:USERPROFILE\.synapse\brain.db" context "smoke test"
synx -f "$env:USERPROFILE\.synapse\brain.db" doctor
```

### 3. Start the daemon

```powershell
synapsed -f "$env:USERPROFILE\.synapse\brain.db"
# listens on TCP 127.0.0.1:9477 (loopback only), writes %USERPROFILE%\.synapse\auth.token
```

The wire protocol and token auth are identical to the unix build; only the
transport differs. `synapse-mcp` picks the matching transport for the OS.

### 4. Wire MCP into your client

```json
{
  "mcpServers": {
    "synapse": {
      "command": "synapse-mcp.exe"
    }
  }
}
```

Config file locations:

- Claude Desktop: `%APPDATA%\Claude\claude_desktop_config.json`
- Cursor: `%USERPROFILE%\.cursor\mcp.json` or `.cursor\mcp.json` in the project
- Windsurf: `%USERPROFILE%\.codeium\windsurf\mcp_config.json`

### 5. Optional: autostart the daemon

```powershell
schtasks /create /tn "Synapse" /sc onlogon /tr "\"C:\path\to\synapsed.exe\" -f \"%USERPROFILE%\.synapse\brain.db\""
```

---

## Where things live

| What | macOS/Linux | Windows |
|------|-------------|---------|
| Brain DB | `~/.synapse/brain.db` (or `-f` path) | `%USERPROFILE%\.synapse\brain.db` |
| Auth token | `~/.synapse/auth.token` | `%USERPROFILE%\.synapse\auth.token` |
| Embedding cache | `.emb-cache` (synx) / `.brain.db.emb-cache` (synapsed), beside the DB | same, beside the DB |
| Daemon endpoint | `/tmp/synapse.sock` | `127.0.0.1:9477` |

One local SQLite file holds everything; nothing leaves the machine. Back it up
with `synx -f <db> backup <out>.synx` or `synapse-ultra backup --db <db>`.

Next: [README.md](../README.md) · [SPEC.md](../SPEC.md) ·
[KNOWN-ISSUES.md](../KNOWN-ISSUES.md)
