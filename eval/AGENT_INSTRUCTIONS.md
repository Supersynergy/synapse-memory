# AgentMD Instructions

**For AI Agents** — How to use Synapse Brain for maximum productivity

---

## Quick Start

### 1. Connection

```python
from synapse_ingestor import SynapseIngestor
from context7_bridge import Context7Bridge
from company_analyzer import CompanyAnalyzer

# Connect to Synapse Brain
ingestor = SynapseIngestor()
bridge = Context7Bridge()
analyzer = CompanyAnalyzer()
```

### 2. Query Syntax

```sql
-- Simple search
"What APIs does auth service expose?"

-- Structured query
SELECT * FROM docs WHERE type='api' AND service='auth'

-- Graph query (future)
MATCH (s:Service)-[:EXPOSES]->(a:API)
WHERE s.name = 'auth'
RETURN a
```

### 3. Ingestion

```bash
# Ingest repository
synapse ingest --path /path/to/codebase

# Ingest GitHub org
synapse ingest --org github:mycompany --token $GH_TOKEN

# Ingest 10,000 repos
synapse ingest --bulk repos.txt --parallel 100
```

---

## Capabilities

### Multi-Repository Intelligence

| Feature | Command |
|---------|--------|
| Scan repos | `synapse scan --org github:org` |
| Ingest code | `synapse ingest --path .` |
| Sync updates | `synapse sync --watch` |
| Search across repos | `synapse search "function name" --repos "repo1,repo2"` |

### Company Analysis

```bash
# Full analysis
synapse analyze --org

# Specific analysis
synapse analyze --tech-stack
synapse analyze --org-chart
synapse analyze --data-flows
synapse analyze --risks

# Export report
synapse analyze --format markdown --output report.md
```

### Context7-Style Retrieval

```python
# Get relevant chunks with citations
response = bridge.retrieve(
    query="How does authentication work?",
    mode="hybrid",
    limit=10,
    context_window=3
)

for chunk in response.chunks:
    print(f"{chunk.content}\n---\n")

for cite in response.citations:
    print(f"[{cite['rank']}] {cite['title']}")
```

---

## Query Patterns

### Technical Queries

```sql
-- Find all endpoints
SELECT * FROM docs WHERE doc_type='code' AND text LIKE '%@app.route%'

-- Find security issues
SELECT * FROM docs WHERE text LIKE '%password%' AND text LIKE '%='

-- Find tests
SELECT * FROM docs WHERE title LIKE '%test%'

-- Find documentation
SELECT * FROM docs WHERE doc_type='documentation'
```

### Business Queries

```sql
-- Find products
SELECT * FROM docs WHERE text LIKE '%product%' AND text LIKE '%service%'

-- Find teams
SELECT * FROM docs WHERE text LIKE '%team%' AND text LIKE '%owned%'

-- Find metrics
SELECT * FROM docs WHERE text LIKE '%kpi%' OR text LIKE '%metric%'
```

### Intelligence Queries

```sql
-- Org structure
SELECT * FROM docs WHERE text LIKE '%team%' AND text LIKE '%lead%'

-- Tech stack
SELECT * FROM docs WHERE text LIKE '%python%' OR text LIKE '%kubernetes%'

-- Data flows
SELECT * FROM docs WHERE text LIKE '%pipeline%' OR text LIKE '%flow%'
```

---

## Use Case Templates

### Template 1: Security Audit

```python
# Find security issues
response = bridge.retrieve(
    query="password secret token api_key credentials SQL injection",
    mode="fts5",
    limit=50
)

# Filter for actual issues
issues = []
for chunk in response.chunks:
    if any(s in chunk.content.lower() for s in ["password=", "secret=", "api_key="]):
        issues.append({
            "file": chunk.title,
            "content": chunk.content[:500],
            "severity": "HIGH"
        })
```

### Template 2: API Documentation

```python
# Find all API endpoints
response = bridge.retrieve(
    query="@app.route @router api endpoint",
    mode="fts5",
    limit=100
)

# Extract endpoints
endpoints = []
for chunk in response.chunks:
    import re
    matches = re.findall(r'@(?:app\.)?(?:route|router|get|post|put|delete)\(["\']([^"\']+)["\']', chunk.content)
    for endpoint in matches:
        endpoints.append({
            "path": endpoint,
            "file": chunk.title
        })
```

### Template 3: Company Intelligence

```python
# Analyze entire company
analysis = analyzer.analyze_all()

# Get summary
print(f"Teams: {analysis.org_chart['total_teams']}")
print(f"Technologies: {analysis.tech_stack['total_technologies']}")
print(f"Products: {len(analysis.products)}")
print(f"Risks: {len(analysis.risks)}")

# Export full report
report = analyzer.export_report("markdown")
print(report)
```

### Template 4: Code Search

```python
# Find function definition
response = bridge.retrieve(
    query="def authenticate function",
    mode="hybrid",
    limit=10
)

# Get full context
for chunk in response.chunks:
    if "def authenticate" in chunk.content:
        print(chunk.content)
```

---

## Performance Tips

### Speed Optimization

```python
# Use FTS5 for keyword search (fastest)
response = bridge.retrieve(query="keyword", mode="fts5")

# Pre-compute embeddings for semantic search
# (Ollama must be running)

# Use caching for repeated queries
from functools import lru_cache

@lru_cache(maxsize=1000)
def cached_search(query):
    return bridge.retrieve(query)
```

### Batch Operations

```python
# Ingest many files in parallel
stats = ingestor.ingest_directory(
    path="/path/to/repos",
    parallel=100,  # 100 parallel workers
    progress_callback=lambda s: print(f"{s.files_ingested}/s")
)
```

---

## Error Handling

```python
try:
    response = bridge.retrieve(query="test")
except sqlite3.OperationalError as e:
    print(f"FTS5 error: {e}")
    # Fallback to basic search
    
try:
    stats = ingestor.ingest_directory(path=".")
except Exception as e:
    print(f"Ingestion error: {e}")
    # Retry or skip
```

---

## File Locations

| Purpose | File |
|---------|------|
| Ingestion | `synapse_ingestor.py` |
| Context Retrieval | `context7_bridge.py` |
| Company Analysis | `company_analyzer.py` |
| Benchmarks | `benchmark_100_tests.py` |
| 1000 Use Cases | `USE_CASES_1000.md` |
| 50 Entrepreneurs | `ENTREPRENEUR_TYPES.md` |
| Optimization | `FTS5_OPTIMIZATION_RESEARCH.md` |
| Spec | `SPEC_AGENTMD.md` |

---

## Examples

### Example 1: Find All Secrets

```bash
python3 -c "
from context7_bridge import Context7Bridge
bridge = Context7Bridge()
response = bridge.retrieve('password secret token api_key', limit=100)
for chunk in response.chunks:
    if 'password' in chunk.content.lower():
        print(f'File: {chunk.title}')
        print(chunk.content[:300])
        print('---')
"
```

### Example 2: Company Report

```bash
python3 company_analyzer.py --format markdown --output company_report.md
```

### Example 3: Ingest Repository

```bash
python3 synapse_ingestor.py --path ~/projects/myapp --parallel 50
```

### Example 4: Multi-Repo Search

```bash
python3 -c "
from context7_bridge import Context7Bridge
bridge = Context7Bridge()
# Search across all ingested repos
response = bridge.retrieve('authentication middleware', limit=50)
print(f'Found {len(response.citations)} documents')
"
```

---

## Troubleshooting

| Issue | Solution |
|-------|----------|
| Slow queries | Increase `mmap_size` and `cache_size` |
| Empty results | Check FTS5 index exists |
| Ingestion fails | Check file permissions |
| Memory issues | Use smaller batch sizes |

---

**Last Updated:** April 23, 2026
**Version:** 1.0
