import sys, os

# Make all three adapter packages importable without install
for pkg in ("synapse_vectorbt", "synapse_qlib", "synapse_nautilus"):
    p = os.path.join(os.path.dirname(__file__), pkg)
    if p not in sys.path:
        sys.path.insert(0, p)
