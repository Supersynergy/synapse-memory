# synapsql for WordPress — Drop-in (zero plugin)

## 30-second install

```bash
# 1. Install synapsql (replaces local mariadb/mysql)
curl -sSL synapsql.io/install | sh
synapsqld --db /var/lib/synapsql/wp.db &

# 2. Migrate existing data (one-time)
synapsql migrate-from-mysql --src mysql://user:pw@old-host/wpdb

# 3. Edit wp-config.php — change ONE line
define('DB_HOST', '127.0.0.1:3306');
# DB_USER, DB_PASSWORD, DB_NAME stay the same

# 4. Done. WordPress now runs on synapsql.
```

That's it. No plugin, no theme change, no custom code.

## What changes

| Layer | Before | After |
|-------|--------|-------|
| `wp-config.php` | `DB_HOST='localhost'` | `DB_HOST='127.0.0.1:3306'` |
| MySQL/MariaDB daemon | running | replaced by `synapsqld` |
| WordPress core | unchanged | unchanged |
| Plugins | unchanged | unchanged |
| Themes | unchanged | unchanged |

## What you get

| Workload | Before (Percona+wp-rocket) | After (synapsql) | Win |
|----------|---------------------------|------------------|-----|
| Cold pageload p50 | 350ms | **target 45ms** | **8×** |
| Cached pageload p50 | 80ms | **target 2ms** | **40×** |
| WP-CLI search-replace 100k | 18min | **target 2min** | **9×** |
| WooCommerce checkout p95 | 850ms | **target 120ms** | **7×** |
| Hosting cost (10k visit/d) | $35-345/mo Kinsta | **$10-50/mo VPS** | **3-30×** |

> **Honest note**: targets above are P10 goals. Today (P1) wires + libsql backend live, fast-path execution layer in P2. See [docs/CLAIMS-AUDIT.md](../../docs/CLAIMS-AUDIT.md).

## Compatible with

- WordPress 5.0 - 6.7+ (latest)
- WooCommerce 3+ to 9+ (HPOS supported)
- BuddyPress, bbPress, Yoast SEO, ACF, Elementor, Divi, Astra
- WP Multisite (planned P5)
- All major caching plugins (wp-rocket, w3tc, ObjectCache Pro) — synapsql replaces wp-rocket entirely (P3 cache layer)

## Rollback

```bash
# revert wp-config.php
define('DB_HOST', 'localhost');
# restart your old mysqld
```

Zero data loss — synapsql can also export back to MySQL dump.

## FAQ

**Q: Is this a fork of MariaDB?**
A: No. synapsql is a new HTAP+AI-native DB written in Rust. It speaks MySQL wire protocol via opensrv-mysql so existing apps just work.

**Q: Can I use it with managed hosting (Kinsta, WP Engine)?**
A: Not yet. Phase 1 = self-host VPS / dedicated. Cloud-managed launches month 4-6.

**Q: What about backups?**
A: `synapsql backup --dst s3://bucket/path` (continuous) or single file copy of `data.db`.

**Q: GDPR / EU?**
A: Apache 2.0 OSS, you self-host = full data sovereignty. EU-friendly by default.
