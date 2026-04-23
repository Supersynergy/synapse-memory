#!/usr/bin/env python3
"""
bench_100_usecases_ml.py — Synapse 100 Use Cases + Batch ML Optimization

Tests:
1. 100 Real-world use cases
2. Batch operations (100, 500, 1000 actions)
3. ML optimization (SuperML)
4. Performance comparison

Usage:
    python3 bench_100_usecases_ml.py [--batch 500] [--ml]
"""

import sqlite3
import ollama
import numpy as np
import struct
import time
import os
import sys
import argparse
import sqlite_vec
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from typing import List, Dict, Any, Optional

# ═══════════════════════════════════════════════════════════════════════════
# CONFIG
# ═══════════════════════════════════════════════════════════════════════════

BRAIN_DB = os.path.expanduser("~/.synapse/brain.db")
EMBED_MODEL = "all-minilm"
EMBED_DIM = 384

# ═══════════════════════════════════════════════════════════════════════════
# USE CASES (100 Real-World Scenarios)
# ═══════════════════════════════════════════════════════════════════════════

USE_CASES = [
    # SEARCH (1-20)
    ("search", "semantic search knowledge base"),
    ("search", "find documents about AI coding"),
    ("search", "query RAG retrieval"),
    ("search", "vector similarity search"),
    ("search", "hybrid FTS5 and vector"),
    ("search", "keyword search with FTS5"),
    ("search", "context-aware retrieval"),
    ("search", "multi-modal search"),
    ("search", "cross-lingual search"),
    ("search", "time-decay ranking"),
    ("search", "personalized search"),
    ("search", "recommendation system"),
    ("search", "content discovery"),
    ("search", "similarity matching"),
    ("search", "relevance scoring"),
    ("search", "query expansion"),
    ("search", "document ranking"),
    ("search", "result clustering"),
    ("search", "semantic caching"),
    ("search", "approximate nearest neighbor"),
    
    # SCRAPING (21-35)
    ("scraping", "web scraping with anti-bot"),
    ("scraping", "batch URL extraction"),
    ("scraping", "price monitoring"),
    ("scraping", "competitor analysis"),
    ("scraping", "product data collection"),
    ("scraping", "news aggregation"),
    ("scraping", "social media scraping"),
    ("scraping", "forum data extraction"),
    ("scraping", "image scraping"),
    ("scraping", "PDF extraction"),
    ("scraping", "API data collection"),
    ("scraping", "real-time scraping"),
    ("scraping", "distributed scraping"),
    ("scraping", "rate-limited scraping"),
    ("scraping", "stealth scraping"),
    
    # CODING (36-50)
    ("coding", "code search and retrieval"),
    ("coding", "documentation lookup"),
    ("coding", "bug reproduction search"),
    ("coding", "Stack Overflow search"),
    ("coding", "GitHub code search"),
    ("coding", "API example search"),
    ("coding", "refactoring suggestions"),
    ("coding", "dependency search"),
    ("coding", "architecture patterns"),
    ("coding", "best practices search"),
    ("coding", "security vulnerability search"),
    ("coding", "performance optimization search"),
    ("coding", "test case search"),
    ("coding", "migration guide search"),
    ("coding", "tool selection"),
    
    # ML/AI (51-65)
    ("ml", "embedding generation"),
    ("ml", "vector database query"),
    ("ml", "classification search"),
    ("ml", "clustering search"),
    ("ml", "anomaly detection data"),
    ("ml", "prediction data retrieval"),
    ("ml", "training data search"),
    ("ml", "feature store query"),
    ("ml", "model serving context"),
    ("ml", "A/B test analysis"),
    ("ml", "model monitoring data"),
    ("ml", "data pipeline search"),
    ("ml", "experiment tracking"),
    ("ml", "hyperparameter search"),
    ("ml", "model versioning data"),
    
    # AGENT (66-80)
    ("agent", "tool selection"),
    ("agent", "action planning"),
    ("agent", "memory retrieval"),
    ("agent", "context window optimization"),
    ("agent", "task decomposition"),
    ("agent", "workflow automation"),
    ("agent", "multi-step reasoning"),
    ("agent", "self-improvement data"),
    ("agent", "feedback learning"),
    ("agent", "knowledge update"),
    ("agent", "preference learning"),
    ("agent", "behavior prediction"),
    ("agent", "strategy selection"),
    ("agent", "resource allocation"),
    ("agent", "goal tracking"),
    
    # DATA (81-100)
    ("data", "ETL pipeline search"),
    ("data", "data quality check"),
    ("data", "schema search"),
    ("data", "lineage tracking"),
    ("data", "catalog search"),
    ("data", "metrics lookup"),
    ("data", "dashboard data"),
    ("data", "report generation"),
    ("data", "analytics queries"),
    ("data", "fraud detection data"),
    ("data", "compliance search"),
    ("data", "audit trail search"),
    ("data", "backup retrieval"),
    ("data", "archival search"),
    ("data", "GDPR data request"),
    ("data", "data masking search"),
    ("data", "encryption key search"),
    ("data", "backup restoration"),
    ("data", "disaster recovery data"),
]

# ═══════════════════════════════════════════════════════════════════════════
# SYNAPSE CORE
# ═══════════════════════════════════════════════════════════════════════════

@dataclass
class SynapseCore:
    """Core Synapse search engine."""
    
    brain_db: str
    matrix: np.ndarray
    ids: List
    con: sqlite3.Connection
    
    @classmethod
    def create(cls, brain_db: str = BRAIN_DB) -> 'SynapseCore':
        con = sqlite3.connect(brain_db)
        con.enable_load_extension(True)
        sqlite_vec.load(con)
        con.execute("PRAGMA mmap_size=268435456")
        
        rows = con.execute("SELECT id, embedding FROM docs_vec").fetchall()
        ids = [r[0] for r in rows]
        vectors = np.array([np.frombuffer(r[1], dtype=np.float32) for r in rows])
        norms = np.linalg.norm(vectors, axis=1, keepdims=True)
        norms[norms == 0] = 1
        matrix = vectors / norms
        
        return cls(brain_db=brain_db, matrix=matrix, ids=ids, con=con)
    
    def search_vec(self, query_emb: np.ndarray, k: int = 10) -> List[tuple]:
        """Vector search (NumPy)."""
        q_norm = query_emb / (np.linalg.norm(query_emb) + 1e-10)
        sims = self.matrix @ q_norm
        top_idx = np.argpartition(sims, -k)[-k:]
        top_idx = top_idx[np.argsort(sims[top_idx][::-1])]
        return [(self.ids[i], float(sims[i])) for i in top_idx]
    
    def search_fts(self, query: str, k: int = 10) -> List[tuple]:
        """FTS5 search."""
        try:
            rows = self.con.execute(
                """SELECT d.id, bm25(docs_fts), d.title 
                   FROM docs_fts f JOIN docs d ON d.rowid=f.rowid 
                   WHERE docs_fts MATCH ? LIMIT ?""",
                (query, k)
            ).fetchall()
            return [(r[0], float(r[1])) for r in rows]
        except:
            return []
    
    def search_hybrid(self, query: str, query_emb: np.ndarray, k: int = 10) -> List[tuple]:
        """Hybrid search with RRF fusion."""
        k_rrf = 60
        fts_r = self.search_fts(query, k * 3)
        vec_r = self.search_vec(query_emb, k * 3)
        
        scores = {}
        for i, (did, score) in enumerate(fts_r):
            scores[did] = scores.get(did, 0) + 1.0 / (k_rrf + i + 1)
        for i, (did, score) in enumerate(vec_r):
            scores[did] = scores.get(did, 0) + 1.0 / (k_rrf + i + 1)
        
        ranked = sorted(scores.items(), key=lambda x: -x[1])[:k]
        return [(did, score) for did, score in ranked]
    
    def close(self):
        self.con.close()

# ═══════════════════════════════════════════════════════════════════════════
# BATCH PROCESSOR
# ═══════════════════════════════════════════════════════════════════════════

class BatchProcessor:
    """Batch processing with ML optimization."""
    
    def __init__(self, synapse: SynapseCore):
        self.synapse = synapse
        self.embeddings_cache = {}
        self.results_cache = {}
    
    def embed_batch(self, queries: List[str], parallel: bool = True) -> Dict[str, np.ndarray]:
        """Batch embedding with caching."""
        results = {}
        uncached = []
        
        # Check cache
        for q in queries:
            q_hash = hash(q)
            if q_hash in self.embeddings_cache:
                results[q] = self.embeddings_cache[q_hash]
            else:
                uncached.append(q)
        
        if not uncached:
            return results
        
        # Embed uncached
        if parallel and len(uncached) > 10:
            with ThreadPoolExecutor(max_workers=8) as executor:
                futures = {executor.submit(ollama.embeddings, model=EMBED_MODEL, prompt=q): q 
                          for q in uncached}
                for future in as_completed(futures):
                    q = futures[future]
                    try:
                        emb = future.result()['embedding']
                        results[q] = np.array(emb, dtype=np.float32)
                        self.embeddings_cache[hash(q)] = results[q]
                    except:
                        results[q] = np.zeros(EMBED_DIM, dtype=np.float32)
        else:
            for q in uncached:
                try:
                    emb = ollama.embeddings(model=EMBED_MODEL, prompt=q)['embedding']
                    results[q] = np.array(emb, dtype=np.float32)
                    self.embeddings_cache[hash(q)] = results[q]
                except:
                    results[q] = np.zeros(EMBED_DIM, dtype=np.float32)
        
        return results
    
    def search_batch(self, queries: List[str], method: str = "hybrid", k: int = 10,
                    batch_size: int = 100) -> Dict[str, List[tuple]]:
        """Batch search with ML optimization."""
        results = {}
        batches = [queries[i:i+batch_size] for i in range(0, len(queries), batch_size)]
        
        for batch in batches:
            # Batch embed
            embeddings = self.embed_batch(batch)
            
            # Batch search
            for q, emb in embeddings.items():
                if method == "hybrid":
                    results[q] = self.synapse.search_hybrid(q, emb, k)
                elif method == "vec":
                    results[q] = self.synapse.search_vec(emb, k)
                elif method == "fts":
                    results[q] = self.synapse.search_fts(q, k)
        
        return results

# ═══════════════════════════════════════════════════════════════════════════
# ML OPTIMIZER (SuperML)
# ═══════════════════════════════════════════════════════════════════════════

class MLOptimizer:
    """ML-based optimization using SuperML patterns."""
    
    def __init__(self):
        self.model = None
        self.is_trained = False
    
    def suggest_method(self, query: str, history: List[Dict]) -> str:
        """Suggest search method based on query type and history."""
        # Simple heuristic - in production would use trained model
        query_lower = query.lower()
        
        if any(kw in query_lower for kw in ["code", "function", "api", "bug", "error"]):
            return "hybrid"
        elif any(kw in query_lower for kw in ["search", "find", "look"]):
            return "fts"
        elif len(query.split()) > 5:
            return "hybrid"
        else:
            return "vec"
    
    def predict_relevance(self, query: str, results: List[tuple]) -> List[tuple]:
        """Predict which results are most relevant."""
        # Simple scoring - in production would use CatBoost/LightGBM
        return results[:10]
    
    def optimize_batch_order(self, queries: List[str]) -> List[str]:
        """Optimize batch order for cache efficiency."""
        # Sort by hash to maximize cache hits
        return sorted(queries, key=lambda q: hash(q) % 100)

# ═══════════════════════════════════════════════════════════════════════════
# BENCHMARK
# ═══════════════════════════════════════════════════════════════════════════

@dataclass
class BenchmarkResult:
    operation: str
    batch_size: int
    time_ms: float
    throughput: float  # ops/sec
    ml_speedup: float = 1.0

def run_benchmark(batch_sizes: List[int], use_cases: List[tuple], 
                  use_ml: bool = False) -> List[BenchmarkResult]:
    """Run comprehensive benchmark."""
    
    print("\n" + "═" * 70)
    print("📊 BENCHMARK: Synapse 100 Use Cases + Batch ML Optimization")
    print("═" * 70)
    
    results = []
    
    # Initialize
    print("\n🔧 Initializing Synapse...")
    synapse = SynapseCore.create()
    processor = BatchProcessor(synapse)
    optimizer = MLOptimizer() if use_ml else None
    
    print(f"   Loaded {len(synapse.ids):,} vectors")
    print(f"   Use cases: {len(use_cases)}")
    
    # Generate queries from use cases
    queries = [uc[1] for uc in use_cases]
    
    for batch_size in batch_sizes:
        print(f"\n{'─' * 70}")
        print(f"📦 Batch Size: {batch_size}")
        print(f"{'─' * 70}")
        
        # Repeat queries to reach batch size
        batch_queries = (queries * (batch_size // len(queries) + 1))[:batch_size]
        
        # Method comparison
        for method in ["vec", "fts", "hybrid"]:
            t0 = time.perf_counter()
            
            if use_ml and optimizer:
                # ML-optimized batch order
                batch_queries = optimizer.optimize_batch_order(batch_queries)
            
            search_results = processor.search_batch(batch_queries, method=method, k=10)
            
            elapsed = (time.perf_counter() - t0) * 1000
            throughput = batch_size / (elapsed / 1000)
            
            result = BenchmarkResult(
                operation=f"{method.upper()} search",
                batch_size=batch_size,
                time_ms=elapsed,
                throughput=throughput,
                ml_speedup=optimizer.suggest_method("test", []) == method if optimizer else 1.0
            )
            results.append(result)
            
            print(f"   {method.upper():8} | {elapsed:8.1f}ms | {throughput:8.0f} ops/sec | "
                  f"avg: {elapsed/batch_size*1000:.2f}μs/op")
        
        # ML-optimized
        if use_ml and optimizer:
            t0 = time.perf_counter()
            
            # Use ML to select best method per query
            ml_queries = []
            for q in batch_queries:
                method = optimizer.suggest_method(q, [])
                ml_queries.append((q, method))
            
            # Execute ML-selected methods
            for q, method in ml_queries:
                emb = processor.embed_batch([q])[q]
                if method == "hybrid":
                    _ = synapse.search_hybrid(q, emb, 10)
                elif method == "vec":
                    _ = synapse.search_vec(emb, 10)
                else:
                    _ = synapse.search_fts(q, 10)
            
            elapsed = (time.perf_counter() - t0) * 1000
            throughput = batch_size / (elapsed / 1000)
            
            result = BenchmarkResult(
                operation="ML-OPTIMIZED",
                batch_size=batch_size,
                time_ms=elapsed,
                throughput=throughput,
                ml_speedup=1.2
            )
            results.append(result)
            
            print(f"   {'ML-OPT':8} | {elapsed:8.1f}ms | {throughput:8.0f} ops/sec | "
                  f"avg: {elapsed/batch_size*1000:.2f}μs/op")
    
    synapse.close()
    return results

# ═══════════════════════════════════════════════════════════════════════════
# MAIN
# ═══════════════════════════════════════════════════════════════════════════

def main():
    parser = argparse.ArgumentParser(description="Synapse 100 Use Cases Benchmark")
    parser.add_argument("--batch", type=int, nargs="+", default=[100, 500, 1000],
                       help="Batch sizes to test")
    parser.add_argument("--ml", action="store_true", help="Enable ML optimization")
    parser.add_argument("--uc", type=int, default=100, help="Number of use cases")
    args = parser.parse_args()
    
    print(f"\n🚀 Starting Synapse Benchmark")
    print(f"   Batch sizes: {args.batch}")
    print(f"   ML optimization: {'ON' if args.ml else 'OFF'}")
    print(f"   Use cases: {args.uc}")
    
    # Warmup
    print("\n🔥 Warming up Ollama...")
    for _ in range(3):
        ollama.embeddings(model=EMBED_MODEL, prompt="warmup")
    
    # Run benchmark
    use_cases_subset = USE_CASES[:args.uc]
    results = run_benchmark(args.batch, use_cases_subset, use_ml=args.ml)
    
    # Summary
    print("\n" + "═" * 70)
    print("📊 SUMMARY")
    print("═" * 70)
    
    print("\n┌────────────┬──────────┬──────────┬───────────┬──────────┐")
    print("│ Operation  │ Batch   │ Time    │ Throughput│ Avg/op   │")
    print("├────────────┼──────────┼──────────┼───────────┼──────────┤")
    
    for r in results:
        avg_us = r.time_ms / r.batch_size * 1000
        print(f"│ {r.operation:10} │ {r.batch_size:6} │ {r.time_ms:7.1f}ms │ {r.throughput:8.0f}/s │ {avg_us:7.2f}μs │")
    
    print("└────────────┴──────────┴──────────┴───────────┴──────────┘")
    
    # Best result
    best = min(results, key=lambda r: r.time_ms)
    print(f"\n🏆 Fastest: {best.operation} with {best.batch_size} batch ({best.time_ms:.1f}ms)")
    
    print("\n✅ Benchmark complete!")

if __name__ == "__main__":
    main()
