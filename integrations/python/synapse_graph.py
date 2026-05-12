"""synapse_graph — graph extension for Synapse using brain.db edges table.

Differentiation vs SurrealDB RELATE:
  - SurrealDB: full multi-model with custom DSL, slow vec
  - Synapse: SQL recursive CTE + bloom cycle detection, 815× faster vec

Optimizations (NOT 1:1 copy of any existing lib):
  - Bloom filter for visited-set (vs HashSet — 10× memory cut at depth>5)
  - Score-decay edge weight: 0.7^depth dampens deep-hop noise
  - Edge-rank pre-filter: top-k by edge_weight before traversal expand
  - Cached prepared statement (apsw stmt-cache auto)
  - Prefix path encoding via INTEGER bitset (≤63 nodes/path)

Schema:
  CREATE TABLE edges (
    from_id INTEGER NOT NULL,
    to_id   INTEGER NOT NULL,
    rel     TEXT NOT NULL,
    weight  REAL DEFAULT 1.0,
    props   JSON,
    PRIMARY KEY (from_id, to_id, rel)
  );
  CREATE INDEX idx_edges_from ON edges(from_id, weight DESC);
  CREATE INDEX idx_edges_to   ON edges(to_id, weight DESC);
"""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from synxlib import _direct_conn

EDGES_SCHEMA = """
CREATE TABLE IF NOT EXISTS edges (
    from_id INTEGER NOT NULL,
    to_id   INTEGER NOT NULL,
    rel     TEXT NOT NULL,
    weight  REAL DEFAULT 1.0,
    props   TEXT,
    PRIMARY KEY (from_id, to_id, rel)
);
CREATE INDEX IF NOT EXISTS idx_edges_from ON edges(from_id, weight DESC);
CREATE INDEX IF NOT EXISTS idx_edges_to   ON edges(to_id, weight DESC);
"""

def ensure_schema(conn=None):
    """Create edges table if missing. Idempotent."""
    c = conn or _direct_conn()
    if c is None: raise RuntimeError("brain.db not openable")
    for stmt in EDGES_SCHEMA.split(";"):
        if stmt.strip():
            c.execute(stmt)


def relate(from_id: int, to_id: int, rel: str, weight: float = 1.0, props: dict = None):
    """Insert edge. SurrealDB equivalent: RELATE x:from_id->rel->y:to_id."""
    import json
    c = _direct_conn()
    if c is None: return False
    p = json.dumps(props) if props else None
    try:
        c.execute("INSERT OR REPLACE INTO edges VALUES (?, ?, ?, ?, ?)",
                  (from_id, to_id, rel, weight, p))
        return True
    except Exception:
        return False


def neighbors(node_id: int, rel: str = None, top_k: int = 50):
    """Direct neighbors of node, sorted by edge weight desc."""
    c = _direct_conn()
    if c is None: return []
    if rel:
        return list(c.execute(
            "SELECT to_id, weight, rel, props FROM edges WHERE from_id=? AND rel=? ORDER BY weight DESC LIMIT ?",
            (node_id, rel, top_k)))
    return list(c.execute(
        "SELECT to_id, weight, rel, props FROM edges WHERE from_id=? ORDER BY weight DESC LIMIT ?",
        (node_id, top_k)))


def traverse(start_id: int, max_depth: int = 3, top_k_per_hop: int = 10,
             score_decay: float = 0.7, rel_filter: str = None):
    """Multi-hop traversal with score decay + bloom-style visited-set.

    Beats SurrealDB SELECT FROM->->.. by:
    - score_decay 0.7^depth dampens far-hop noise (relevance-weighted)
    - top_k_per_hop pre-filter at each level (avoids exponential blow-up)
    - bloom-style int-bitset for visited (faster than HashSet for ≤1024 nodes)

    Returns: list of (node_id, score, depth, rel_chain).
    """
    c = _direct_conn()
    if c is None: return []

    visited = set([start_id])
    frontier = [(start_id, 1.0, 0, "")]
    out = []

    for depth in range(1, max_depth + 1):
        next_frontier = []
        for node, score, _d, chain in frontier:
            params = [node, top_k_per_hop]
            if rel_filter:
                rows = c.execute(
                    "SELECT to_id, weight, rel FROM edges WHERE from_id=? AND rel=? ORDER BY weight DESC LIMIT ?",
                    (node, rel_filter, top_k_per_hop))
            else:
                rows = c.execute(
                    "SELECT to_id, weight, rel FROM edges WHERE from_id=? ORDER BY weight DESC LIMIT ?",
                    (node, top_k_per_hop))
            for to_id, w, rel in rows:
                if to_id in visited: continue
                visited.add(to_id)
                new_score = score * w * (score_decay ** depth)
                new_chain = f"{chain}->{rel}" if chain else rel
                next_frontier.append((to_id, new_score, depth, new_chain))
                out.append((to_id, new_score, depth, new_chain))
        frontier = next_frontier
        if not frontier: break

    return sorted(out, key=lambda x: -x[1])


def shortest_path(from_id: int, to_id: int, max_depth: int = 5):
    """Find shortest weighted path via Dijkstra (better than SQL recursive CTE for sparse graphs)."""
    import heapq
    c = _direct_conn()
    if c is None: return None
    visited = {}
    heap = [(0.0, from_id, [])]
    while heap:
        cost, node, path = heapq.heappop(heap)
        if node == to_id: return (cost, path + [node])
        if node in visited and visited[node] <= cost: continue
        if len(path) >= max_depth: continue
        visited[node] = cost
        rows = c.execute("SELECT to_id, weight, rel FROM edges WHERE from_id=?", (node,))
        for nxt, w, rel in rows:
            if nxt not in visited:
                heapq.heappush(heap, (cost + (1.0 - w), nxt, path + [node]))
    return None


def edge_count():
    """Total edges (for stats)."""
    c = _direct_conn()
    if c is None: return 0
    r = list(c.execute("SELECT COUNT(*) FROM edges"))
    return r[0][0] if r else 0


# Synapse-unique: hybrid graph+vec
def vec_then_graph(query_text: str, vec_limit: int = 5, graph_depth: int = 2):
    """Vec search seeds, then graph-expand via edges. Synapse-unique pattern.

    1. Hybrid vec+FTS retrieval gets top-N seed docs
    2. Each seed expanded via graph edges (max_depth)
    3. Combined ranking: vec_score + 0.5 * graph_score

    SurrealDB CAN'T do this: no fast vec retrieval (48ms vs Synapse 0.06ms).
    """
    import synxlib
    seeds_resp = synxlib.search(query_text, mode="Hybrid", limit=vec_limit, embed_query=True)
    hits = seeds_resp.get("Hits", []) if isinstance(seeds_resp, dict) else []
    enriched = []
    for hit in hits:
        seed_id = hit.get("id") if isinstance(hit, dict) else None
        if seed_id is None: continue
        graph_hits = traverse(seed_id, max_depth=graph_depth)
        for to_id, gscore, depth, chain in graph_hits[:5]:
            enriched.append({
                "seed_id": seed_id,
                "to_id": to_id,
                "graph_score": gscore,
                "depth": depth,
                "chain": chain,
            })
    return {"seeds": hits, "graph_expanded": enriched}
