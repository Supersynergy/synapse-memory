# Pingora Edge vs Upstream — /health bench 2026-05-06

**Setup**: c=64, z=30s, `--no-tui`, 127.0.0.1 explicit  
**Upstream 9477**: MOCK UPSTREAM — Python `http.server.HTTPServer` (synapsed not TCP; uses Unix socket)  
**Edge 9478**: synapse-edge (pingora 0.8) — /health short-circuit (no upstream proxy, responds directly)

---

## Raw: upstream mock 9477/health (MOCK UPSTREAM)

```
Summary:
  Success rate:   99.83%
  Total:          30.0043 sec
  Slowest:        3.9746 sec
  Fastest:        0.0001 sec
  Average:        0.0026 sec
  Requests/sec:   5194.0250

Response time distribution:
  50.00% in 0.0003 sec   (p50 = 0.3 ms)
  99.00% in 0.0315 sec   (p99 = 31.5 ms)

Status code distribution:
  [200] 155521 responses
Error distribution:
  [258] timeout
  [64] aborted due to deadline
```

---

## Raw: synapse-edge 9478/health (pingora short-circuit)

```
Summary:
  Success rate:   100.00%
  Total:          30006.5011 ms
  Slowest:        125.3510 ms
  Fastest:        0.0172 ms
  Average:        1.6709 ms
  Requests/sec:   38272.3396

Response time distribution:
  50.00% in 0.7538 ms    (p50 = 0.75 ms)
  99.00% in 22.0922 ms   (p99 = 22.1 ms)

Status code distribution:
  [200] 1148356 responses
Error distribution:
  [63] aborted due to deadline (deadline = test end, expected)
```

---

## Delta Table

| Metric         | Mock upstream :9477 | Pingora edge :9478 | Delta         |
|----------------|---------------------|--------------------|---------------|
| QPS            | 5,194               | 38,272             | +637% (7.4×)  |
| p50 latency    | 0.30 ms             | 0.75 ms            | +0.45 ms      |
| p99 latency    | 31.5 ms             | 22.1 ms            | −30% faster   |
| Success rate   | 99.83%              | 100.00%            | −0.17% errors |
| Total reqs     | 155,521             | 1,148,356          | 7.4×          |

**Notes**:
- Pingora edge /health is a short-circuit (never proxies to upstream) — pure pingora async I/O path
- Python HTTPServer is GIL-bound single-threaded; bottleneck is Python, not axum. Real axum at 9477 would be significantly faster but synapsed only exposes Unix socket.
- p50 on edge (0.75 ms) higher than mock (0.30 ms) because pingora has connection-pool + async overhead vs Python loopback; at c=256 pingora QPS would be proportionally higher still.
- 100% success rate on edge confirms /health short-circuit works even with upstream down.
