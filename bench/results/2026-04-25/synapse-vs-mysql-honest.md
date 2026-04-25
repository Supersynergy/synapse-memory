# Synapse vs MariaDB — Honest Sysbench 2026-04-25

## Setup
- **Synapse**: `synapse-mysql-async` (opensrv-mysql v0.10), port 13312, SQLite `/tmp/sysbench.db`, 10k rows
- **MariaDB**: port 3306, `SELECT 1` (no data load needed for baseline)
- **Client**: pymysql, Python threading, M4 Max, 128 GB RAM
- **Query (Synapse)**: `SELECT * FROM sbtest1 WHERE id=1`
- **Query (MariaDB)**: `SELECT 1` (no row fetch overhead — MariaDB advantage)

## Results

### synapse-mysql-async (SQLite backend, point-select)
| Threads | Queries | Elapsed | OPS   | p50 (ms) | p95 (ms) |
|---------|---------|---------|-------|----------|----------|
| 1       | 2,000   | 9.58s   | 209   | 3.565    | 7.148    |
| 4       | 8,000   | 15.54s  | 515   | 6.171    | 13.504   |
| 8       | 16,000  | 13.96s  | 1,146 | 6.166    | 11.628   |
| 16      | 32,000  | 22.16s  | 1,444 | 8.113    | 27.013   |

### MariaDB baseline (SELECT 1 — no disk I/O)
| Threads | Queries | Elapsed | OPS    | p50 (ms) | p95 (ms) |
|---------|---------|---------|--------|----------|----------|
| 1       | 2,000   | 0.15s   | 13,480 | 0.066    | 0.136    |
| 4       | 8,000   | 0.65s   | 12,218 | 0.291    | 0.622    |
| 8       | 16,000  | 2.10s   | 7,626  | 0.948    | 2.030    |
| 16      | 32,000  | 4.19s   | 7,646  | 1.881    | 4.208    |

### Delta
| Threads | Synapse OPS | MariaDB OPS | Ratio (MariaDB/Synapse) |
|---------|-------------|-------------|------------------------|
| 1       | 209         | 13,480      | **64×**                |
| 4       | 515         | 12,218      | **24×**                |
| 8       | 1,146       | 7,626       | **7×**                 |
| 16      | 1,444       | 7,646       | **5×**                 |

> **Caveat**: MariaDB query is `SELECT 1` (pure in-memory), Synapse query is a real point-select with SQLite I/O + row parsing. Gap would narrow if MariaDB ran the same point-select workload. True apples-to-apples requires sysbench with TLS disabled.

## GIL Note
Python's GIL limits true parallelism in the benchmark client. At 16 threads the GIL contention inflates client-side latency and suppresses measured OPS. Use `multiprocessing.Pool` or a Rust/Go bench client for GIL-free results at high thread counts.

## sysbench TLS Blocker
sysbench 1.0.20 mandates TLS by default against MySQL protocol servers:
```
FATAL: error 2026: SSL is required, but the server does not support it
```

**Fix**: Add `--mysql-ssl=false` flag to sysbench CLI, **and** implement `ssl_mode=DISABLED` handshake response in `synapse-mysql-async/src/main.rs`. In opensrv-mysql v0.10, return `CLIENT_SSL` capability = 0 in the server handshake packet, or handle the SSL upgrade request with a plain rejection so sysbench falls back to cleartext.

## Next Steps
1. Disable TLS requirement in opensrv-mysql handshake (server-side `SSL_CIPHER` = empty, cap flag `CLIENT_SSL=0`)
2. Re-run: `sysbench oltp_point_select --mysql-ssl=false ...`
3. Add `multiprocessing`-based bench client for GIL-free 16/32/64 thread results
4. Load actual sbtest1 into MariaDB for true apples-to-apples comparison
