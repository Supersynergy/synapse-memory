# io_uring Linux Bench — 2026-05-13

## Config
- n = 100000 inserts, batch_size = 512
- OS: Linux aarch64 (colima Docker Ubuntu 24.04)
- Rust release build, --features io-uring

## Results

| Engine | Total (ms) | Inserts/s |
|--------|-----------|-----------|
| io_uring WAL | 5574 | 17939 |
| rusqlite WAL | 106 | 938331 |

**Speedup: 0.02×**

## Notes
- io_uring batch_size=512 SQEs per submit
- SQLite: WAL mode, synchronous=NORMAL, batch transactions
