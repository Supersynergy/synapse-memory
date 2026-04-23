# Synapse Brain — Evaluation Suite

Comprehensive intelligence and search system for AI agents.

## Quick Start

```bash
# Query Synapse Brain
python3 context7_bridge.py "How does authentication work?"

# Analyze company
python3 company_analyzer.py --format markdown

# Ingest repository
python3 synapse_ingestor.py --path ~/projects/codebase

# Run 100 tests
python3 benchmark_100_tests.py
```

---

## Files

### Core Components

| File | Purpose |
|------|---------|
| `synapse_ingestor.py` | Multi-repository ingestion (10,000+ repos) |
| `context7_bridge.py` | Context7-style retrieval with chunks |
| `company_analyzer.py` | Company intelligence analysis |
| `benchmark_100_tests.py` | 100 comprehensive tests |

### Documentation

| File | Purpose |
|------|---------|
| `SPEC_AGENTMD.md` | Complete specification |
| `AGENT_INSTRUCTIONS.md` | Agent usage guide |
| `FTS5_OPTIMIZATION_RESEARCH.md` | FTS5 optimization guide |
| `USE_CASES_1000.md` | 1000 real-world use cases |
| `ENTREPRENEUR_TYPES.md` | 50 entrepreneur types |
| `README.md` | This file |

### Benchmarks

| File | Results |
|------|---------|
| `comprehensive_db_benchmark.py` | Database comparison |
| `benchmark_fts5_perspectives.py` | Multi-perspective FTS5 |
| `results/*.json` | Raw benchmark data |

---

## Performance

| Metric | Value |
|--------|-------|
| FTS5 Query | 44,158 ops/sec |
| p95 Latency | 0.029ms |
| Batch Size | 200 (optimal) |
| Cache Warming | +40% after 10 queries |

---

## Features

- **Multi-Repository Ingestion** — 10,000+ repos in parallel
- **Company Intelligence** — Org charts, tech stack, data flows
- **Context7 Retrieval** — Chunks, citations, cross-references
- **1000 Use Cases** — Real-world scenarios
- **50 Entrepreneurs** — Business contexts
- **100 Tests** — Comprehensive coverage

---

## Usage Examples

### Python

```python
from context7_bridge import Context7Bridge

bridge = Context7Bridge()
response = bridge.retrieve("authentication", limit=10)

for chunk in response.chunks:
    print(chunk.content)
```

### CLI

```bash
# Search
python3 context7_bridge.py "query" --mode hybrid

# Analyze
python3 company_analyzer.py --format markdown --output report.md

# Ingest
python3 synapse_ingestor.py --path /path/to/code --parallel 100
```

---

## Generated

April 23, 2026
