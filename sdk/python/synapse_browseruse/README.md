# synapse-browser-use

browser-use memory plugin — persists browse history to [synapse](https://github.com/Supersynergy/synapse).

Interface spec: https://github.com/browser-use/browser-use (84k★)

## Install

```bash
pip install synapse-browser-use
```

## Usage

```python
from browser_use import Agent
from synapse_browseruse import SynapseMemoryPlugin

plugin = SynapseMemoryPlugin(sock_path="/tmp/synapse.sock")

# Wire into browser-use agent
agent = Agent(
    task="Research AI frameworks",
    on_step_end=plugin.on_step_end,
)
await agent.run()

# Later: recall what was browsed
hits = plugin.recall("AI agent frameworks", limit=10)
```

## API

| Method | Description |
|--------|-------------|
| `on_page_visit(url, title, content)` | Store a page visit |
| `on_step_end(step)` | browser-use step hook (auto-extracts URL) |
| `recall(query, limit)` | Semantic search over browse history |
