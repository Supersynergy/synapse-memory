# Three-Session Bugfix Demo

This demo proves the saleable workflow:

1. Session 1 stores a project decision.
2. Session 2 stores a failing-test observation.
3. Session 3 recalls both facts through scoped Synapse memory and applies the fix.

Run:

```bash
make demo-agent-memory
```

The demo uses an isolated temporary daemon and database, so it does not touch
the user's real Synapse brain.
