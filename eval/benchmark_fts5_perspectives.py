#!/usr/bin/env python3
"""
benchmark_fts5_perspectives.py

Fast FTS5 benchmark from multiple perspectives:
- Query length
- Time constraint
- Entrepreneur type
- Batch size
"""

import sqlite3
import time
import json
import os
from dataclasses import dataclass, asdict
from typing import List
from datetime import datetime

BRAIN_DB = os.path.expanduser("~/.synapse/brain.db")
OUTPUT_DIR = os.path.expanduser("~/projects/synapse/eval/results")
os.makedirs(OUTPUT_DIR, exist_ok=True)

# Use cases
USE_CASES = [
    # Short (1-3 words)
    "search", "code", "model", "data", "query", "tool", "agent", "memory",
    "search AI", "search code", "search model", "search data", "search query",
    "find document", "find file", "find text", "find info", "find record",
    "lookup knowledge", "lookup code", "lookup docs", "lookup info",
    
    # Medium (4-6 words)
    "semantic search knowledge base", "full-text search documentation",
    "vector similarity search embeddings", "hybrid search combining text",
    "keyword search with filters", "prefix search autocomplete",
    "fuzzy search typo tolerance", "phrase search exact match",
    "boolean search AND OR NOT", "cross-lingual search translation",
    "voice search query parsing", "image search by description",
    "code search snippets repository", "patent search prior art database",
    "academic paper search engine", "legal document search system",
    "medical literature search database", "news article search archives",
    "social media search trending", "product catalog search e-commerce",
    "job posting search database", "resume search candidate matching",
    "real estate listing search MLS", "recipe search ingredients",
    "music search lyrics recognition", "podcast search transcripts",
    
    # Long (7+ words)
    "semantic search knowledge base for artificial intelligence applications",
    "full-text search documentation system with natural language processing",
    "vector similarity search embeddings for machine learning models",
    "hybrid search combining text and semantic understanding capabilities",
    "keyword search with filters and advanced query operators",
    "prefix search autocomplete suggestions with machine learning",
    "fuzzy search typo tolerance for better user experience",
    "phrase search exact match for precise information retrieval",
    "boolean search AND OR NOT operators for complex queries",
    "cross-lingual search translation across multiple languages",
]

# Entrepreneur types with query patterns
ENTREPRENEUR_TYPES = {
    "tech_founder": ["code search", "API documentation", "bug tracker search", "technical spec search"],
    "ecommerce_merchant": ["product search", "customer review search", "inventory search", "supplier search"],
    "content_creator": ["trending topic search", "SEO keyword search", "content idea search", "viral content search"],
    "consultant": ["best practice search", "case study search", "industry report search", "expertise search"],
    "real_estate_agent": ["property listing search", "MLS search", "comparable sales search", "neighborhood data search"],
    "healthcare_practitioner": ["ICD code search", "drug interaction search", "treatment protocol search", "medical literature search"],
    "financial_advisor": ["investment search", "portfolio search", "market data search", "regulatory search"],
    "lawyer": ["case law search", "statute search", "contract clause search", "precedent search"],
    "recruiter": ["resume search", "candidate search", "skill database search", "job posting search"],
    "marketer": ["campaign search", "keyword research", "competitor analysis", "trend search"],
}

@dataclass
class BenchmarkResult:
    engine: str
    perspective: str
    query_count: int
    total_time_ms: float
    avg_time_ms: float
    ops_per_sec: float
    p95_ms: float
    p99_ms: float

def run_fts5_benchmark(queries: List[str], perspective: str, iterations: int = 100) -> List[BenchmarkResult]:
    """Run FTS5 benchmark."""
    con = sqlite3.connect(BRAIN_DB)
    con.execute("PRAGMA mmap_size=268435456")
    con.execute("PRAGMA cache_size=-64000")
    
    results = []
    
    # Direct FTS5 (no JOIN)
    times1 = []
    for _ in range(iterations):
        for q in queries:
            t0 = time.perf_counter()
            con.execute("SELECT * FROM docs_fts WHERE docs_fts MATCH ? LIMIT 10", (q,)).fetchall()
            times1.append((time.perf_counter() - t0) * 1000)
    
    times1.sort()
    results.append(BenchmarkResult(
        engine="FTS5 (no JOIN)",
        perspective=perspective,
        query_count=len(queries) * iterations,
        total_time_ms=sum(times1),
        avg_time_ms=sum(times1) / len(times1),
        ops_per_sec=1000 / (sum(times1) / len(times1)),
        p95_ms=times1[int(len(times1) * 0.95)],
        p99_ms=times1[int(len(times1) * 0.99)],
    ))
    
    # WITH JOIN
    times2 = []
    for _ in range(iterations):
        for q in queries:
            t0 = time.perf_counter()
            con.execute("""
                SELECT d.id, d.title, d.text 
                FROM docs_fts f 
                JOIN docs d ON d.rowid = f.rowid
                WHERE docs_fts MATCH ? LIMIT 10
            """, (q,)).fetchall()
            times2.append((time.perf_counter() - t0) * 1000)
    
    times2.sort()
    results.append(BenchmarkResult(
        engine="FTS5 (with JOIN)",
        perspective=perspective,
        query_count=len(queries) * iterations,
        total_time_ms=sum(times2),
        avg_time_ms=sum(times2) / len(times2),
        ops_per_sec=1000 / (sum(times2) / len(times2)),
        p95_ms=times2[int(len(times2) * 0.95)],
        p99_ms=times2[int(len(times2) * 0.99)],
    ))
    
    con.close()
    return results

def main():
    print("""
╔═══════════════════════════════════════════════════════════════════════╗
║     FTS5 MULTI-PERSPECTIVE BENCHMARK                          ║
╚═══════════════════════════════════════════════════════════════════════╝
""")
    
    all_results = []
    
    # PERSPECTIVE 1: Query Length
    print("\n📊 PERSPECTIVE 1: Query Length")
    print("-" * 40)
    
    short = USE_CASES[0:10]
    medium = USE_CASES[10:20]
    long = USE_CASES[20:30]
    
    for queries, name in [(short, "short"), (medium, "medium"), (long, "long")]:
        results = run_fts5_benchmark(queries, f"query_length_{name}", iterations=100)
        all_results.extend(results)
        for r in results:
            print(f"  {r.engine:15} | {name:8} | {r.ops_per_sec:>9,.0f} ops/s | p95: {r.p95_ms:.3f}ms")
    
    # PERSPECTIVE 2: Time Constraint
    print("\n📊 PERSPECTIVE 2: Time Constraint")
    print("-" * 40)
    
    time_constraints = {
        "instant": (USE_CASES[0:10], 50),
        "fast": (USE_CASES[0:15], 100),
        "accurate": (USE_CASES[0:20], 200),
        "thorough": (USE_CASES, 500),
    }
    
    for constraint, (queries, iters) in time_constraints.items():
        results = run_fts5_benchmark(queries, f"time_{constraint}", iterations=10)
        all_results.extend(results)
        for r in results:
            print(f"  {r.engine:15} | {constraint:8} | {r.ops_per_sec:>9,.0f} ops/s | p95: {r.p95_ms:.3f}ms")
    
    # PERSPECTIVE 3: Entrepreneur Type
    print("\n📊 PERSPECTIVE 3: Entrepreneur Type")
    print("-" * 40)
    
    for ent_type, queries in list(ENTREPRENEUR_TYPES.items())[:5]:
        results = run_fts5_benchmark(queries, f"entrepreneur_{ent_type}", iterations=50)
        all_results.extend(results)
        for r in results:
            print(f"  {r.engine:15} | {ent_type:12} | {r.ops_per_sec:>9,.0f} ops/s | p95: {r.p95_ms:.3f}ms")
    
    # PERSPECTIVE 4: Batch Size
    print("\n📊 PERSPECTIVE 4: Batch Size Scaling")
    print("-" * 40)
    
    batch_sizes = [10, 50, 100, 250, 500]
    for batch_size in batch_sizes:
        queries = USE_CASES[0:batch_size] * (100 // batch_size + 1)
        queries = queries[:batch_size]
        results = run_fts5_benchmark(queries, f"batch_{batch_size}", iterations=10)
        all_results.extend(results)
        for r in results:
            print(f"  {r.engine:15} | batch={batch_size:4} | {r.ops_per_sec:>9,.0f} ops/s | p95: {r.p95_ms:.3f}ms")
    
    # SUMMARY
    print("\n" + "="*80)
    print("📊 FINAL SUMMARY")
    print("="*80)
    
    # Best result by engine
    by_engine = {}
    for r in all_results:
        if r.engine not in by_engine or r.ops_per_sec > by_engine[r.engine].ops_per_sec:
            by_engine[r.engine] = r
    
    print("\n┌─────────────────────┬─────────────────────┬───────────┬───────────┐")
    print("│ Engine              │ Best Perspective    │ ops/sec   │ p95_ms   │")
    print("├─────────────────────┼─────────────────────┼───────────┼───────────┤")
    for engine, result in sorted(by_engine.items(), key=lambda x: -x[1].ops_per_sec):
        print(f"│ {engine:19} │ {result.perspective:19} │ {result.ops_per_sec:9,.0f} │ {result.p95_ms:8.3f} │")
    print("└─────────────────────┴─────────────────────┴───────────┴───────────┘")
    
    # Save results
    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    results_file = f"{OUTPUT_DIR}/fts5_perspectives_{timestamp}.json"
    
    with open(results_file, "w") as f:
        json.dump([asdict(r) for r in all_results], f, indent=2)
    
    print(f"\n✅ Results saved to: {results_file}")

if __name__ == "__main__":
    main()
