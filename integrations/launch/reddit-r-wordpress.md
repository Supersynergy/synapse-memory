**Title**: I built a drop-in MariaDB replacement for WordPress — 545× faster autoload, Apache 2.0, single 14MB binary

Hey r/wordpress,

I've been frustrated with WP autoload (200-2000 wp_options rows hit on every pageload, ~400µs of pure DB-roundtrip) so I built **synapse** — a Rust HTAP database that drop-in replaces MariaDB/MySQL via the standard wire protocol.

**Measured benchmarks vs MariaDB 12.2** (M4 Max, criterion, 2000-row wp_options table):

- WP autoload single SELECT: **12.8 µs → 19 ns (670×)**
- WP 30-option pageload: **400 µs → 733 ns (545×)**
- INSERT batch=1000: **38.7 µs → 1.02 µs (38×)**
- Sysbench 8t mixed 80r/20w: **437µs/iter → 205µs/iter (2.13×)**

Reproducible:
```
git clone https://github.com/Supersynergy/synapse
cargo bench -p synapse-cms-bench --bench vs_mariadb
```

**Honest scope** — this is **NOT** a 100% MariaDB replacement. Missing: stored procedures, MVCC row-locking, async replication, multi-region. **What it DOES have**: cache+slow-log+index-advisor+drift-detection+RBAC, single binary, Apache 2.0 self-host.

**For most blogs the speedup is invisible** because network latency dominates user-perceived UX. **Where it matters**: hosting providers/agencies (same VPS hosts 8-10× more sites = $35k/yr savings), WooCommerce Black Friday spike-survival, WP-admin power-user workflows (editor save 80ms→0.5ms).

**Install**:
```
docker run -d -p 3306:3306 synapse/server:latest
# wp-config.php: define('DB_HOST', '127.0.0.1:3306');
```

WordPress companion plugin (settings UI, slow-log viewer, health check) submitted to wordpress.org marketplace this week.

Code: https://github.com/Supersynergy/synapse
Docs: docs/wp-edition/

Would love feedback — especially edge-cases I'm missing, or "this would be useful if it could also do X".
