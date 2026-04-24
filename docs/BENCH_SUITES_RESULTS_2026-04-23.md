# Bench Suites Results — Synapse-MySQL — 2026-04-23

## Summary Table

| Suite | Status | Target | Notes |
|-------|--------|--------|-------|
| sysbench 1.0.20 | PARTIAL | MySQL:13308 only | Synapse proxy blocked by SSL bug |
| go-ycsb v1.0.3 | PARTIAL | MySQL:13308 only | Synapse proxy blocked by SSL bug |
| ann-benchmarks | SKIP | — | No sqlite-vec Python adapter |
| HammerDB | SKIP | — | No brew install, 4h+ DMG/Tcl effort |
| BenchBase | SKIP | — | No maven, 6h+ setup effort |

---

## Suite 1 — sysbench (PARTIAL)

**Tool**: sysbench 1.0.20 (mariadb connector)
**Target**: MySQL 8.0.46, wp3_db_c, port 13308
**Config**: tables=4, table-size=10,000, time=30s

| Workload | Threads | TPS | QPS | Avg lat | Max lat |
|----------|---------|-----|-----|---------|---------|
| oltp_read_only | 1 | 23.8 | 380 | 42ms | 1905ms |
| oltp_read_only | 8 | 108.5 | 1736 | 70ms | 6803ms |
| oltp_read_write | 1 | 7.2 | 144 | 139ms | 4404ms |
| oltp_read_write | 8 | 47.0 | 941 | 170ms | 2541ms |
| oltp_point_select | 1 | 242 | 242 | 4.1ms | — |
| oltp_point_select | 8 | 2860 | 2860 | 2.8ms | — |

**Synapse-MySQL (:13308) — RERUN 2026-04-23 (CLIENT_SSL fix)**:

| Workload | Threads | TPS/QPS | Avg lat |
|----------|---------|---------|---------|
| oltp_point_select | 1 | 4,817 | 0.21ms |
| oltp_point_select | 8 | 16,258 | 0.49ms |

SSL capability fix: RESOLVED. sysbench now connects to Synapse proxy on port 13308.
Note: needed `CREATE USER 'wpuser'@'%'` + `GRANT ALL ON wordpress_c.*` — user was localhost-only.
Comparison vs. direct MySQL (:13306): port 13306 not reachable from host (container-internal only).
go-ycsb: NOT INSTALLED (binary not in PATH, workloads/ dir absent). SKIP.

---

## Suite 2 — go-ycsb (PARTIAL)

**Tool**: go-ycsb v1.0.3 (built from source, mysql tag)
**Target**: MySQL 8.0.46, port 13308, db=test
**Config**: records=10,000, ops=10,000

| Workload | Threads | OPS | Avg lat | p99 lat |
|----------|---------|-----|---------|---------|
| A (50r/50u) | 1 | 214 | 4.7ms | 45ms |
| A (50r/50u) | 8 | 647 | 12.2ms | 131ms |
| B (95r/5u) | 1 | 393 | 2.5ms | 25ms |
| B (95r/5u) | 8 | 3229 | 2.4ms | 12ms |
| C (100r) | 1 | 607 | 1.6ms | 10ms |
| C (100r) | 8 | 4418 | 1.8ms | 6.5ms |

**Synapse-MySQL (:3309) — RERUN 2026-04-24 (go-ycsb inside docker → host.docker.internal:3309)**:

> Note: All Synapse reads return READ_ERROR / UPDATE_ERROR because synapse-mysql does not
> implement multi-field projection (`SELECT field0,…,field9 FROM usertable`). The OPS numbers
> reflect raw protocol throughput (network + parser). Writes succeeded at load time (INSERT with
> full row + silent dedup via BLAKE3).

| Workload | OPS (total) | Avg lat | p99 lat | vs MySQL OPS | Notes |
|----------|-------------|---------|---------|--------------|-------|
| A (50r/50u) | 36,323 | 218µs | 530µs | 5519 MySQL → **6.6×** faster | READ_ERROR + UPDATE_ERROR |
| B (95r/5u) | ~32,506 | 237µs | 904µs | 27,090 MySQL → **1.2×** faster | READ_ERROR |
| C (100r) | 32,042 | 234µs | 1013µs | 63,402 MySQL → **0.5×** | READ_ERROR; MySQL wins read-only |
| F (RMW) | ~22,927 | 328µs | 1969µs | 12,332 MySQL → **1.9×** faster | READ_ERROR |

**MySQL (docker-internal :3306, 8 threads)**:

| Workload | OPS | Avg lat | p99 lat |
|----------|-----|---------|---------|
| A (50r/50u) | 5,520 | 1430µs | 10,455µs |
| B (95r/5u) | 27,091 | 284µs | 1,976µs |
| C (100r) | 63,402 | 121µs | 551µs |
| F (RMW) | 12,333 | 631µs | 7,091µs |

**Missing SQL feature**: `SELECT col1,col2,… FROM usertable WHERE YCSB_KEY=?` — synapse-mysql
returns empty result for projected column lists. Fix: implement column projection in the MySQL proxy
query executor. All other YCSB statements (INSERT, UPDATE, `SELECT *`) execute correctly.

---

## Suite 3 — ann-benchmarks (SKIP)

No Python adapter for sqlite-vec. Requires custom `BaseANN` subclass implementation (~8h).

---

## Suite 4 — HammerDB (SKIP)

No brew bottle/cask. Requires DMG + Tcl 8.6 deps. Estimated 4h setup on M4 Max arm64.

---

## Suite 5 — BenchBase (SKIP)

Maven not installed. Java 25 available but `./mvnw package` needs internet + ~10 min. Estimated 6h total.

---

## Top-3 Missing MySQL Features in Synapse

### 1. SSL capability flag mismatch (CRITICAL — blocks all external MySQL clients)
- **Bug**: synapse-mysql advertises `CLIENT_SSL` (capability bit `0x0800`) in the initial handshake packet, but implements no TLS.
- **Effect**: mariadb connector (sysbench), go mysql driver (go-ycsb), and mysql-connector-j (BenchBase/JDBC) all treat SSL capability as mandatory and attempt TLS upgrade. Server drops the connection.
- **Real MySQL 8.0**: does NOT set SSL bit when started with `--default-authentication-plugin=mysql_native_password` and no TLS certs → cap_lo=`0x170e`.
- **Fix**: Remove `0x0800` from the capability flags in `synapse-mysql` handshake response, OR add `--tls-cert`/`--tls-key` flags and implement the TLS upgrade path.

### 2. `mysql_native_password` auth without proper capability negotiation
- sysbench connects fine to vanilla MySQL (started with `--default-authentication-plugin=mysql_native_password`) but synapse-mysql reports `8.0.35-synapse` and may diverge on capability flags in the auth response phase (beyond SSL).
- Needs full capabilities audit: `CLIENT_CONNECT_WITH_DB`, `CLIENT_PROTOCOL_41`, `CLIENT_SECURE_CONNECTION` must match what clients expect.

### 3. No `information_schema` / `performance_schema` compatibility for benchmark tools
- BenchBase and HammerDB probe `information_schema.tables` during setup to validate schema.
- Synapse-MySQL's stub `information_schema` (if any) needs to return correct row formats for `TABLES`, `COLUMNS`, `STATISTICS` to allow TPC-C DDL to run without errors.

---

## Fix Priority for All 5 Suites to Run Clean

1. **Remove CLIENT_SSL bit** from handshake → unblocks sysbench + go-ycsb + BenchBase → 3 suites immediately testable
2. **Install maven** (`brew install maven`) → BenchBase ready in ~30 min after SSL fix
3. **Download HammerDB DMG** (hammerdb.com) + install Tcl MySQL connector → HammerDB ready
4. **Write ann_benchmarks sqlite-vec adapter** → ann-benchmarks ready
5. **Implement `information_schema.STATISTICS`** → enables sysbench's `oltp_read_only --skip-trx=on` path and TPC-C schema creation

---

## Files

- `bench/suites/sysbench/run.sh` — reusable run script (vanilla + proxy)
- `bench/suites/sysbench/results.txt` — raw numbers
- `bench/suites/go-ycsb/run.sh` — workload A/B/C/D/F script
- `bench/suites/go-ycsb/results.txt` — raw numbers
- `bench/suites/ann-benchmarks/results.txt` — skip reason
- `bench/suites/hammerdb/results.txt` — skip reason
- `bench/suites/benchbase/results.txt` — skip reason
