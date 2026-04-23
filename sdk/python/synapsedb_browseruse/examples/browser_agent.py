"""
browser-use + synapse memory plugin example.
Run: python examples/browser_agent.py
Requires: synapsed at /tmp/synapsedb.sock

In a real browser-use setup, pass plugin.on_step_end as a callback.
This example simulates page visits without requiring browser-use installed.
"""
from synapsedb_browseruse import SynapseMemoryPlugin

plugin = SynapseMemoryPlugin(embed=True)

# Simulate browsing history
plugin.on_page_visit(
    url="https://en.wikipedia.org/wiki/Paris",
    title="Paris - Wikipedia",
    content="Paris is the capital and largest city of France. Population ~2.1M.",
)
plugin.on_page_visit(
    url="https://en.wikipedia.org/wiki/Eiffel_Tower",
    title="Eiffel Tower - Wikipedia",
    content="The Eiffel Tower is a wrought-iron lattice tower on the Champ de Mars in Paris.",
)
plugin.on_page_visit(
    url="https://en.wikipedia.org/wiki/Berlin",
    title="Berlin - Wikipedia",
    content="Berlin is the capital and largest city of Germany.",
)

print("Stored 3 page visits.")

# Recall relevant browse history
results = plugin.recall("Paris landmarks", limit=5)
print(f"\nRecall 'Paris landmarks' ({len(results)} results):")
for r in results:
    print(f"  [{r.get('score', 0):.3f}] {r.get('uri')} — {r.get('text', '')[:60]}")

plugin.close()
