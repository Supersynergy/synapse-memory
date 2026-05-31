# OLTP Fair Bench — Synapse vs mysql

Date: 2026-04-25
Host: Darwin 24.5.0 arm64
Workload: `sysbench oltp_point_select` (10k rows, 10s, threads {1,4,8,16})

## Caveats
- Synapse advertises a self-signed TLS cert because sysbench's libmariadb
  refuses to dial a non-TLS server even with `--mysql-ssl=off`. Client
  selects plaintext via that flag → wire path identical to a real
  TLS-disabled deployment.
- Synapse backend: SQLite WAL + 512 MB mmap + 64 MB page cache.
- Prepared-statement LRU bumped to 256 (rusqlite `prepare_cached`),
  collapses repeat-parse cost on the hot `SELECT c FROM sbtest1 WHERE id=?`.
- Single host, loopback only, no network noise.
- Ref server: `mysql` (9.6.0) on 127.0.0.1:3306.

## Results

| target | threads | tps | qps | p95 ms |
|---|---:|---:|---:|---:|
| synapse | 1 | 322.55 | 322.55 | 0.0 |
| synapse | 4 | 1085.93 | 1085.93 | 0.0 |
| synapse | 8 | 1552.79 | 1552.79 | 0.0 |
| synapse | 16 | 2005.52 | 2005.52 | 0.0 |
| mysql | 1 | 10153.28 | 10153.28 | 0.0 |
| mysql | 4 | 36694.34 | 36694.34 | 0.0 |
| mysql | 8 | 44887.79 | 44887.79 | 0.0 |
| mysql | 16 | 64725.55 | 64725.55 | 0.0 |

## Artifacts
- raw: `/Users/master/projects/synapse/bench/results/2026-04-25/oltp-fair.json`
- daemon log: `/tmp/synx-async-fair.log`
