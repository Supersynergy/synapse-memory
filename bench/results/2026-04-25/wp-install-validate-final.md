# WP Core Install Validation — Commit 8329f7d

**Date**: 2026-04-25
**Branch**: session-2026-04-25-ultrathink
**Daemon**: synapse-mysql-async (release, port 13319)
**DB**: /tmp/wp-validate-install.db
**WP**: /tmp/wpbench/wp-validate (cloned from wp-synapse)

## Result: VERIFIED — install + post create end-to-end PASS (after auth-plugin fix)

### Update 2026-04-25 (post-fix)
Root cause was NOT missing OK-packet `last_insert_id` propagation — that code path
(`shim.rs::on_query` and `on_execute` DML branches, lines 626-651 / 826-849) was
already correctly populating `OkResponse { last_insert_id, affected_rows, .. }`
since commit `8329f7d`.

The actual blocker was `default_auth_plugin = "caching_sha2_password"`, which
requires a public-key RSA exchange that opensrv-mysql cannot complete without
TLS. PHP mysqli (used by wpdb in wp-cli) silently failed the handshake and
reported "Error establishing a database connection". The earlier
`wp post create → id=4` success from agent a379fb5c was on a build with
`mysql_native_password` — the plugin was changed somewhere mid-session.

Switched to `mysql_native_password` (the wpdb / mysqli compat default). After
rebuild:

```
wp core install ...           → Success: WordPress installed successfully.
wp post create --porcelain    → 4
wp post get 4                 → full row dump (post_title=hello, status=publish)
wp post create --porcelain    → 5  (monotonic increment confirms LAST_INSERT_ID)
```

### Verified gates
1. wp core install → Success
2. wp post create #1 → **id=4** (non-zero)
3. wp post get 4 → full row, fields populated
4. wp post create #2 → **id=5** (monotonic, OK packet propagation works)

### Reproduction (final)
```
target/release/synapse-mysql-async -f /tmp/wp.db -b 127.0.0.1:13320 \
  --mode wp --root-password root &
wp config create --dbname=synapse --dbuser=root --dbpass=root \
  --dbhost=127.0.0.1:13320 --skip-check --force
wp core install --url=http://localhost --title=Test \
  --admin_user=admin --admin_password=admin \
  --admin_email=admin@test.local --skip-email
wp post create --post_title=hello --post_status=publish --porcelain   # → 4
```

---

## (Original report below — kept for history)

## Result: PARTIAL PASS — install succeeds, post-install INSERTs miss LAST_INSERT_ID

### What works (commit 8329f7d unblocks installer)
- `wp core install` exit 0: **"Success: WordPress installed successfully."**
- `wp option get siteurl` → `http://localhost`
- `wp post list` shows seeded `Hello world!` (ID=1, status=publish)
- All install-phase queries (SHOW VARIABLES canonical defaults, safe_table_name, $$ count, wp_options, wp_users, wp_usermeta, wp_terms, wp_term_taxonomy, wp_term_relationships, wp_options autoload cache invalidate) execute without error.

### What fails (next blocker)
- `wp post create --post_title=ValidateTest --post_status=publish` → "Success: Created post 0."
- New post gets ID=0 → unretrievable, no row visible. WordPress then fires `UPDATE wp_posts SET guid='' WHERE ID = 0` and `... WHERE ID IS NULL`.

### Root cause hypothesis
After `INSERT INTO wp_posts (...)` succeeds, the daemon does NOT return a non-zero `last_insert_id` to mysqli. wpdb reads `$wpdb->insert_id` → 0. PHP then walks `null`/0 references producing the warning cascade visible in stdout.

Failing query trace from `/tmp/wp-validate4.log`:

```
on_query:  INSERT INTO `wp_posts` (...) VALUES (0, '2026-04-25 18:53:56', ...,'ValidateTest',...)
rewritten: INSERT INTO "wp_posts" (...) VALUES (...)
on_query:  UPDATE `wp_posts` SET `guid` = '' WHERE `ID` = 0     ← consumer of LAST_INSERT_ID
on_query:  UPDATE `wp_posts` SET `guid` = '' WHERE `ID` IS NULL ← fallback retry
```

### Reproduction
```
nohup target/release/synapse-mysql-async \
  --db /tmp/wp-validate-install.db --bind 127.0.0.1:13319 --mode wp \
  > /tmp/wp-validate4.log 2>&1 &
cd /tmp/wpbench/wp-validate
wp core install --allow-root --skip-email --url=http://localhost \
  --title=Validate --admin_user=admin --admin_password=admin --admin_email=t@t.local
wp post create --allow-root --post_title=test --post_status=publish --porcelain
# → "0"
```

### Notes
- During session, target/release/synapse-mysql-async was rebuilt by another concurrent build and CLI flags changed mid-test (`-f/--file/--bind/--root-password` → `--db/--bind/--mode` only). Final daemon used `--db ... --bind ... --mode wp`. wp core install ran successfully on this final binary too.
- All 6/8 verification gates passed; gates 7-8 (post create + retrieve) failed on LAST_INSERT_ID propagation, NOT on the SHOW VARIABLES / safe_table_name / $$ fixes from 8329f7d.

### Next blocker (do NOT fix in this agent)
Wire `OkResponse::last_insert_id` (mysql wire packet) to SQLite `last_insert_rowid()` from the connection that ran the INSERT. Currently appears to ship 0.

### Files
- Daemon log: `/tmp/wp-validate4.log`
- wp-cli stdout: `/tmp/wp-validate-stdout.log`
- wp-config: `/tmp/wpbench/wp-validate/wp-config.php`
- DB: `/tmp/wp-validate-install.db`
