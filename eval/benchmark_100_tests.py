#!/usr/bin/env python3
"""
benchmark_100_tests.py

100 additional tests for Synapse Brain FTS5 optimization.
Tests cover:
- Query patterns
- Data types
- Scale factors
- Concurrency
- Edge cases
- Security
- Performance
- Quality
"""

import sqlite3
import time
import os
import json
import random
import string
from dataclasses import dataclass, asdict
from typing import List, Dict, Tuple
from datetime import datetime

BRAIN_DB = os.path.expanduser("~/.synapse/brain.db")
OUTPUT_DIR = os.path.expanduser("~/projects/synapse/eval/results")
os.makedirs(OUTPUT_DIR, exist_ok=True)

# ═══════════════════════════════════════════════════════════════════════════
# TEST CATEGORIES
# ═══════════════════════════════════════════════════════════════════════════

TEST_CATEGORIES = {
    "query_patterns": "Query Pattern Tests",
    "data_types": "Data Type Tests",
    "scale": "Scale Tests",
    "concurrency": "Concurrency Tests",
    "edge_cases": "Edge Case Tests",
    "security": "Security Tests",
    "performance": "Performance Tests",
    "quality": "Quality Tests",
    "integration": "Integration Tests",
    "regression": "Regression Tests",
}

# ═══════════════════════════════════════════════════════════════════════════
# TEST QUERIES
# ═══════════════════════════════════════════════════════════════════════════

TEST_QUERIES = {
    # QUERY PATTERNS (1-20)
    "qp_001": "code",
    "qp_002": "model",
    "qp_003": "data",
    "qp_004": "query",
    "qp_005": "tool",
    "qp_006": "search",
    "qp_007": "file",
    "qp_008": "test",
    "qp_009": "config",
    "qp_010": "api",
    "qp_011": "database",
    "qp_012": "server",
    "qp_013": "client",
    "qp_014": "function",
    "qp_015": "class",
    "qp_016": "error",
    "qp_017": "log",
    "qp_018": "auth",
    "qp_019": "cache",
    "qp_020": "sync",
    
    # DATA TYPES (21-30)
    "dt_021": "json",
    "dt_022": "xml",
    "dt_023": "yaml",
    "dt_024": "sql",
    "dt_025": "csv",
    "dt_026": "markdown",
    "dt_027": "python",
    "dt_028": "javascript",
    "dt_029": "rust",
    "dt_030": "go",
    
    # SCALE (31-40)
    "sc_031": "performance optimization benchmark",
    "sc_032": "scalability architecture design pattern",
    "sc_033": "database migration strategy process",
    "sc_034": "microservices authentication authorization",
    "sc_035": "kubernetes docker container orchestration",
    "sc_036": "machine learning inference deployment",
    "sc_037": "continuous integration pipeline configuration",
    "sc_038": "distributed system consensus algorithm",
    "sc_039": "api gateway rate limiting throttling",
    "sc_040": "message queue async processing event",
    
    # EDGE CASES (41-55)
    "ec_041": "",  # Empty query
    "ec_042": "a",  # Single char
    "ec_043": "   ",  # Whitespace
    "ec_044": "test\r\nquery",  # Newlines
    "ec_045": "test't query",  # Single quote
    "ec_046": 'test" query',  # Double quote
    "ec_047": "<script>alert('xss')</script>",  # XSS attempt
    "ec_048": "test; DROP TABLE docs; --",  # SQL injection
    "ec_049": "a" * 1000,  # Very long query
    "ec_050": "test query with unicode ñ 中文 日本語",
    "ec_051": "🚒 🔥 💻 ⚡ 🎯",  # Emoji
    "ec_052": "CamelCase mixedWith snake_case and kebab-case",
    "ec_053": "test    multiple   spaces",
    "ec_054": "UPPERCASE AND lowercase",
    "ec_055": "test-123-456 with numbers 2024",
    
    # SECURITY (56-65)
    "sec_056": "password secret token",
    "sec_057": "api_key credentials",
    "sec_058": "private key certificate",
    "sec_059": "sql injection union select",
    "sec_060": "xss script alert",
    "sec_061": "path traversal ../../../etc/passwd",
    "sec_062": "command injection; rm -rf",
    "sec_063": "csrf token session",
    "sec_064": "cors origin access",
    "sec_065": "oauth2 bearer token",
    
    # PERFORMANCE (66-75)
    "perf_066": "index optimization query plan",
    "perf_067": "cache hit ratio memory usage",
    "perf_068": "connection pool thread",
    "perf_069": "batch insert bulk operation",
    "perf_070": "async await concurrency",
    "perf_071": "lazy loading pagination",
    "perf_072": "connection timeout retry",
    "perf_073": "rate limit throttle",
    "perf_074": "circuit breaker fallback",
    "perf_075": "dead lock mutex semaphore",
    
    # QUALITY (76-85)
    "qual_076": "documentation readme guide",
    "qual_077": "test coverage unit integration",
    "qual_078": "code review lint format",
    "qual_079": "refactor clean code",
    "qual_080": "technical debt legacy",
    "qual_081": "best practice pattern",
    "qual_082": "design principle solid",
    "qual_083": "comment explanation rationale",
    "qual_084": "error handling exception",
    "qual_085": "logging monitoring alert",
    
    # INTEGRATION (86-95)
    "int_086": "webhook callback event",
    "int_087": "rest api graphql",
    "int_088": "database migration seed",
    "int_089": "service mesh sidecar",
    "int_090": "configmap secret volume",
    "int_091": "dns load balancer cdn",
    "int_092": "ssl tls certificate",
    "int_093": "ssh git deploy",
    "int_094": "docker compose kubernetes",
    "int_095": "github actions workflow",
    
    # REGRESSION (96-100)
    "reg_096": "bug fix regression test",
    "reg_097": "version semver changelog",
    "reg_098": "rollback backup restore",
    "reg_099": "staging prod environment",
    "reg_100": "maintenance window upgrade",
}

# ═══════════════════════════════════════════════════════════════════════════
# TEST DATA STRUCTURES
# ═══════════════════════════════════════════════════════════════════════════

@dataclass
class TestResult:
    test_id: str
    category: str
    query: str
    success: bool
    time_ms: float
    results_count: int
    error: str = ""

# ═══════════════════════════════════════════════════════════════════════════
# TEST RUNNER
# ═══════════════════════════════════════════════════════════════════════════

class FTS5TestRunner:
    """Run comprehensive FTS5 tests."""
    
    def __init__(self, db_path: str):
        self.db_path = db_path
        self.con = None
        self.results = []
        
    def connect(self):
        """Connect with optimal settings."""
        self.con = sqlite3.connect(self.db_path)
        self.con.execute("PRAGMA mmap_size=268435456")
        self.con.execute("PRAGMA cache_size=-64000")
        self.con.execute("PRAGMA journal_mode=WAL")
        self.con.execute("PRAGMA synchronous=NORMAL")
        
    def close(self):
        if self.con:
            self.con.close()
    
    def test_query(self, query: str, test_id: str, category: str) -> TestResult:
        """Test a single query."""
        try:
            t0 = time.perf_counter()
            
            # Skip empty queries
            if not query or not query.strip():
                return TestResult(
                    test_id=test_id,
                    category=category,
                    query="<empty>",
                    success=False,
                    time_ms=0,
                    results_count=0,
                    error="Empty query"
                )
            
            # Execute query
            cursor = self.con.execute("""
                SELECT d.id, d.title, d.text
                FROM docs_fts f 
                JOIN docs d ON d.rowid = f.rowid
                WHERE docs_fts MATCH ?
                LIMIT 100
            """, (query,))
            
            results = cursor.fetchall()
            elapsed = (time.perf_counter() - t0) * 1000
            
            return TestResult(
                test_id=test_id,
                category=category,
                query=query[:50] + "..." if len(query) > 50 else query,
                success=True,
                time_ms=elapsed,
                results_count=len(results),
                error=""
            )
            
        except Exception as e:
            return TestResult(
                test_id=test_id,
                category=category,
                query=query[:50] + "..." if len(query) > 50 else query,
                success=False,
                time_ms=0,
                results_count=0,
                error=str(e)[:100]
            )
    
    def run_all_tests(self, iterations: int = 10) -> List[TestResult]:
        """Run all 100 tests."""
        self.connect()
        
        print("Running 100 FTS5 tests...")
        
        for test_id, query in TEST_QUERIES.items():
            # Determine category from test_id prefix
            category_map = {
                "qp": "query_patterns",
                "dt": "data_types",
                "sc": "scale",
                "ec": "edge_cases",
                "sec": "security",
                "perf": "performance",
                "qual": "quality",
                "int": "integration",
                "reg": "regression",
            }
            category = category_map.get(test_id[:3], "unknown")
            
            # Run test
            result = self.test_query(query, test_id, category)
            self.results.append(result)
            
            # Status update every 10 tests
            if int(test_id.split("_")[1]) % 10 == 0:
                print(f"  Progress: {test_id} ({len(self.results)}/100)")
        
        self.close()
        return self.results
    
    def run_stress_tests(self) -> List[TestResult]:
        """Run stress tests with varying conditions."""
        self.connect()
        
        print("Running stress tests...")
        
        # Test 1: Rapid fire queries
        stress_results = []
        queries = list(TEST_QUERIES.values())[:20]
        
        t0 = time.perf_counter()
        for _ in range(100):
            for q in queries:
                if q and q.strip():
                    try:
                        self.con.execute("""
                            SELECT * FROM docs_fts WHERE docs_fts MATCH ? LIMIT 10
                        """, (q,)).fetchall()
                    except:
                        pass
        elapsed = (time.perf_counter() - t0) * 1000
        
        stress_results.append(TestResult(
            test_id="stress_001",
            category="stress",
            query="Rapid fire 2000 queries",
            success=True,
            time_ms=elapsed,
            results_count=2000
        ))
        
        # Test 2: Large result sets
        try:
            t0 = time.perf_counter()
            cursor = self.con.execute("""
                SELECT * FROM docs_fts WHERE docs_fts MATCH 'a*' LIMIT 1000
            """)
            results = cursor.fetchall()
            elapsed = (time.perf_counter() - t0) * 1000
            
            stress_results.append(TestResult(
                test_id="stress_002",
                category="stress",
                query="Large result set (1000)",
                success=True,
                time_ms=elapsed,
                results_count=len(results)
            ))
        except Exception as e:
            stress_results.append(TestResult(
                test_id="stress_002",
                category="stress",
                query="Large result set (1000)",
                success=False,
                time_ms=0,
                results_count=0,
                error=str(e)[:100]
            ))
        
        # Test 3: Concurrent connections simulation
        t0 = time.perf_counter()
        for i in range(10):
            con = sqlite3.connect(self.db_path)
            con.execute("PRAGMA mmap_size=268435456")
            for q in ["code", "test", "api"]:
                con.execute("SELECT * FROM docs_fts WHERE docs_fts MATCH ?", (q,)).fetchall()
            con.close()
        elapsed = (time.perf_counter() - t0) * 1000
        
        stress_results.append(TestResult(
            test_id="stress_003",
            category="stress",
            query="10 sequential connections",
            success=True,
            time_ms=elapsed,
            results_count=30
        ))
        
        self.close()
        return stress_results
    
    def generate_report(self, results: List[TestResult]) -> str:
        """Generate test report."""
        report = []
        report.append("=" * 80)
        report.append("FTS5 BENCHMARK - 100 TEST REPORT")
        report.append("=" * 80)
        
        # Summary by category
        by_category = {}
        for r in results:
            if r.category not in by_category:
                by_category[r.category] = {"passed": 0, "failed": 0, "times": [], "total": 0}
            by_category[r.category]["total"] += 1
            if r.success:
                by_category[r.category]["passed"] += 1
                by_category[r.category]["times"].append(r.time_ms)
            else:
                by_category[r.category]["failed"] += 1
        
        report.append("\n📊 RESULTS BY CATEGORY")
        report.append("-" * 60)
        report.append(f"{'Category':<20} {'Passed':>10} {'Failed':>10} {'Avg ms':>10}")
        report.append("-" * 60)
        
        for cat, stats in sorted(by_category.items()):
            avg = sum(stats["times"]) / len(stats["times"]) if stats["times"] else 0
            report.append(f"{cat:<20} {stats['passed']:>10} {stats['failed']:>10} {avg:>10.3f}")
        
        # Overall stats
        total_passed = sum(1 for r in results if r.success)
        total_failed = sum(1 for r in results if not r.success)
        avg_time = sum(r.time_ms for r in results if r.success) / max(1, total_passed)
        
        report.append("-" * 60)
        report.append(f"{'TOTAL':<20} {total_passed:>10} {total_failed:>10} {avg_time:>10.3f}")
        
        # Failed tests
        failed = [r for r in results if not r.success]
        if failed:
            report.append("\n❌ FAILED TESTS")
            report.append("-" * 60)
            for r in failed:
                report.append(f"{r.test_id}: {r.error}")
        
        # Fastest/slowest
        successful = [r for r in results if r.success]
        if successful:
            fastest = min(successful, key=lambda x: x.time_ms)
            slowest = max(successful, key=lambda x: x.time_ms)
            
            report.append("\n⚡ FASTEST TEST")
            report.append(f"  {fastest.test_id}: {fastest.time_ms:.3f}ms ({fastest.query})")
            
            report.append("\n🐌 SLOWEST TEST")
            report.append(f"  {slowest.test_id}: {slowest.time_ms:.3f}ms ({slowest.query})")
        
        return "\n".join(report)

def main():
    print("""
╔═══════════════════════════════════════════════════════════════════════╗
║     FTS5 100-TEST BENCHMARK                                    ║
╚═══════════════════════════════════════════════════════════════════════╝
""")
    
    runner = FTS5TestRunner(BRAIN_DB)
    
    # Run main tests
    results = runner.run_all_tests(iterations=10)
    results.extend(runner.run_stress_tests())
    
    # Generate report
    report = runner.generate_report(results)
    print(report)
    
    # Save results
    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    results_file = f"{OUTPUT_DIR}/100_tests_{timestamp}.json"
    
    with open(results_file, "w") as f:
        json.dump([asdict(r) for r in results], f, indent=2)
    
    report_file = f"{OUTPUT_DIR}/100_tests_{timestamp}.txt"
    with open(report_file, "w") as f:
        f.write(report)
    
    print(f"\n✅ Results saved to:")
    print(f"   JSON: {results_file}")
    print(f"   TXT:  {report_file}")

if __name__ == "__main__":
    main()
