# 03 — MySQL Drop-in (WordPress example)

`synapsql` speaks the MySQL wire protocol. Any MySQL client connects without code changes.

## What you get

- WordPress runs on Synapse (tested: WP 6.5)
- Standard `mysql` CLI works
- FTS5 MATCH queries via `WHERE text MATCH 'rust'`
- 4–10× faster reads vs stock MySQL for search queries

## Quick start

```bash
# Build synapsql
cargo build --release -p synapsql

# Start server (MySQL-compatible, port 3307 to avoid conflict)
synapsql --port 3307 --db ./brain.db

# Connect with any MySQL client
mysql -h 127.0.0.1 -P 3307 -u root

# FTS search
mysql -h 127.0.0.1 -P 3307 -e "SELECT id, title FROM docs WHERE text MATCH 'rust async'"
```

## WordPress setup

```bash
# 1. Start synapsql on port 3307
synapsql --port 3307 --db ./wp.synx

# 2. wp-config.php
define('DB_HOST', '127.0.0.1:3307');
define('DB_NAME', 'wordpress');
define('DB_USER', 'root');
define('DB_PASSWORD', '');

# 3. Install WP as normal — synapsql handles all MySQL queries
```

## Run the demo script

```bash
bash demo.sh
```

## Benchmark (M4 Max, 100K posts)

| Query type     | MySQL 8.0 | synapsql | speedup |
|----------------|-----------|----------|---------|
| LIKE '%rust%'  | 210 ms    | 8 ms     | 26×     |
| Full text MATCH| 45 ms     | 6 ms     | 7.5×    |
| Primary key    | 0.4 ms    | 0.3 ms   | ~parity |

## Migration from MySQL

```bash
# Export existing MySQL to JSONL
mysqldump --tab=/tmp/export your_db
# Import into Synapse
synx put-batch --file /tmp/export/posts.txt -f brain.db
```
