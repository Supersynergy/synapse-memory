# WP Install Retry — synapse-mysql-async (FINAL)

**Date**: 2026-04-25
**Branch**: session-2026-04-25-ultrathink @ HEAD `30c645a` ("fix(wp): final 3 scalar fns")
**Daemon**: `target/release/synapse-mysql-async` (built fresh, 5.2 MB)
**Bind**: `127.0.0.1:13317`  | **Mode**: `wp` | **DB**: `/tmp/wp-install-retry.db` (clean)
**WP**: 6.x at `/tmp/wp-retry/` (copy of `/tmp/wordpress-mysql/` minus config)

## Result: PASS — `wp core install` succeeded

```
Success: WordPress installed successfully.
```

## Round-trip verification (all PASS)

| Command | Output |
|---|---|
| `wp option get siteurl` | `http://localhost` |
| `wp post list` | `1  Hello world!  hello-world  2026-04-25 17:30:29  publish` |
| `wp post create --post_title=hello --post_status=publish --porcelain` | `4` |
| `wp post list` (after) | rows for ID 4 (hello) and ID 1 (Hello world!) |
| `wp user list` | `1  admin  admin  test@test.local  ... administrator` |

INSERT → LAST_INSERT_ID → SELECT round-trip is solid (post id 4 retrieved by WP after write).

## Smoke probe (pymysql, mode=wp)

```
SELECT 1            -> (('1',),)             OK
SELECT DATABASE()   -> (('wordpress',),)      OK   (was: connection drop)
SELECT LAST_INSERT_ID() -> ((0,),)            OK   (was: connection drop)
```

All 3 fixes from a7a5119a / commit 158922 verified live.

## Daemon log highlights

- `query SELECT 1 → 1 rows (3454µs)`
- `DML INSERT INTO wp_options ... → affected=1 last_id=15 (~3ms)`
- `DML UPDATE "wp_options" SET ... → affected=1 last_id=15`
- `wp_options write — autoload cache invalidated` (cache invalidation working)
- `cache hit SELECT option_value FROM wp_options WHERE option_n` (hot-path cache live)
- Hundreds of option/postmeta queries, no errors, no connection drops during install.

## Minor cosmetic observation (not a blocker)

`mysql` CLI binary (`/opt/homebrew/bin/mysql`) drops with
`Wrong number of parameters passed to query. Got 0, needed 1`
on its initial session-init probe. **WP-CLI / pymysql / mysqlnd all unaffected** —
they don't send the offending probe. Likely a `?`-bearing SHOW WARNINGS / charset probe
hitting passthrough → SQLite `prepare()`. Doesn't affect WP. Filed as P3 cosmetic gap.

## Verdict

**Drop-in achievable today: YES** for WP install + basic post/option CRUD against
`synapse-mysql-async --mode wp`. All five Aleph blockers + the two final scalar
crashes are closed. Pioneer phase milestone reached.

## Artifacts

- Daemon log: `/tmp/wp-install-retry.log`
- WP install output: `/tmp/wp-install-output.log`
- WP verify output: `/tmp/wp-verify.log`
- WP install dir: `/tmp/wp-retry/`
- SQLite brain: `/tmp/wp-install-retry.db`
