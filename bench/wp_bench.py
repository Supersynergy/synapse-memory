#!/usr/bin/env python3
"""WP bench: MySQL 8.4 vs SQLite+FTS5 (Synapse stand-in).
Seeds 1k posts into both, measures 5 scenarios, reports results.
"""
import time, random, string, sqlite3, statistics, json
import mysql.connector

MYSQL_CFG = dict(host="127.0.0.1", port=3310, user="root", password="root", database="wp_bench")
SQLITE_PATH = "/tmp/wp_bench.db"
SEED_POSTS = 1000
RUNS = 5  # per scenario

LOREM = (
    "rust web framework async tokio axum hyper performance benchmark latency "
    "wordpress plugin database mysql postgres sqlite fts5 search indexing "
    "wasm javascript python golang docker kubernetes microservices cloud "
    "machine learning neural network embedding vector similarity cosine "
    "ecommerce woocommerce checkout cart product variant price stock inventory "
    "admin dashboard analytics report user role permission authentication jwt "
)

def rand_words(n=20):
    words = LOREM.split()
    return " ".join(random.choices(words, k=n))

def rand_title():
    words = LOREM.split()
    return " ".join(random.choices(words, k=6)).title()

# ── MySQL setup ────────────────────────────────────────────────────────────────

def mysql_seed():
    con = mysql.connector.connect(host="127.0.0.1", port=3310, user="root", password="root")
    cur = con.cursor()
    cur.execute("DROP DATABASE IF EXISTS wp_bench")
    cur.execute("CREATE DATABASE wp_bench CHARACTER SET utf8mb4")
    cur.execute("USE wp_bench")
    con.database = "wp_bench"

    cur.execute("""
        CREATE TABLE wp_posts (
            ID BIGINT AUTO_INCREMENT PRIMARY KEY,
            post_title TEXT NOT NULL,
            post_content LONGTEXT NOT NULL,
            post_status VARCHAR(20) DEFAULT 'publish',
            post_type VARCHAR(20) DEFAULT 'post',
            post_date DATETIME DEFAULT CURRENT_TIMESTAMP,
            post_author BIGINT DEFAULT 1,
            INDEX idx_status_type (post_status, post_type)
        ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
    """)
    cur.execute("""
        CREATE TABLE wp_postmeta (
            meta_id BIGINT AUTO_INCREMENT PRIMARY KEY,
            post_id BIGINT NOT NULL,
            meta_key VARCHAR(255),
            meta_value LONGTEXT,
            INDEX idx_post_id (post_id),
            INDEX idx_meta_key (meta_key)
        ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
    """)
    cur.execute("""
        CREATE TABLE wp_options (
            option_id BIGINT AUTO_INCREMENT PRIMARY KEY,
            option_name VARCHAR(191) UNIQUE NOT NULL,
            option_value LONGTEXT NOT NULL,
            autoload VARCHAR(20) DEFAULT 'yes'
        ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
    """)

    # seed posts
    posts = [(rand_title(), rand_words(40)) for _ in range(SEED_POSTS)]
    # ensure some "rust web framework" hits for scenario 3
    for i in range(50):
        posts[i] = (f"Rust Web Framework Bench #{i}", rand_words(10) + " rust web framework async " + rand_words(10))
    cur.executemany(
        "INSERT INTO wp_posts (post_title, post_content) VALUES (%s, %s)",
        posts
    )
    # seed postmeta (5 per post)
    metas = []
    for pid in range(1, SEED_POSTS+1):
        for k in ("_thumbnail_id", "_edit_lock", "_edit_last", "views", "rating"):
            metas.append((pid, k, str(random.randint(1, 9999))))
    cur.executemany(
        "INSERT INTO wp_postmeta (post_id, meta_key, meta_value) VALUES (%s, %s, %s)",
        metas
    )
    # seed options (300 autoload)
    opts = [(f"option_{i}", "x"*random.randint(20,200), "yes") for i in range(300)]
    cur.executemany(
        "INSERT INTO wp_options (option_name, option_value, autoload) VALUES (%s, %s, %s)",
        opts
    )
    con.commit()
    con.close()
    print(f"MySQL seeded: {SEED_POSTS} posts, {len(metas)} postmeta, {len(opts)} options")

# ── SQLite FTS5 setup ──────────────────────────────────────────────────────────

def sqlite_seed(posts_data):
    db = sqlite3.connect(SQLITE_PATH)
    db.execute("DROP TABLE IF EXISTS wp_posts")
    db.execute("DROP TABLE IF EXISTS wp_postmeta")
    db.execute("DROP TABLE IF EXISTS wp_options")
    db.execute("DROP TABLE IF EXISTS wp_posts_fts")
    db.execute("""
        CREATE TABLE wp_posts (
            ID INTEGER PRIMARY KEY,
            post_title TEXT NOT NULL,
            post_content TEXT NOT NULL,
            post_status TEXT DEFAULT 'publish',
            post_type TEXT DEFAULT 'post'
        )
    """)
    db.execute("""
        CREATE VIRTUAL TABLE wp_posts_fts USING fts5(
            post_title, post_content, content=wp_posts, content_rowid=ID,
            tokenize='porter ascii'
        )
    """)
    db.execute("""
        CREATE TABLE wp_postmeta (
            meta_id INTEGER PRIMARY KEY,
            post_id INTEGER,
            meta_key TEXT,
            meta_value TEXT
        )
    """)
    db.execute("CREATE INDEX IF NOT EXISTS idx_pm_post ON wp_postmeta(post_id)")
    db.execute("""
        CREATE TABLE wp_options (
            option_id INTEGER PRIMARY KEY,
            option_name TEXT UNIQUE,
            option_value TEXT,
            autoload TEXT DEFAULT 'yes'
        )
    """)

    db.executemany("INSERT INTO wp_posts (post_title, post_content) VALUES (?,?)", posts_data)
    db.execute("INSERT INTO wp_posts_fts(rowid, post_title, post_content) SELECT ID, post_title, post_content FROM wp_posts")

    metas = []
    for pid in range(1, SEED_POSTS+1):
        for k in ("_thumbnail_id", "_edit_lock", "_edit_last", "views", "rating"):
            metas.append((pid, k, str(random.randint(1,9999))))
    db.executemany("INSERT INTO wp_postmeta (post_id, meta_key, meta_value) VALUES (?,?,?)", metas)

    opts = [(f"option_{i}", "x"*random.randint(20,200), "yes") for i in range(300)]
    db.executemany("INSERT INTO wp_options (option_name, option_value, autoload) VALUES (?,?,?)", opts)

    db.commit()
    db.close()
    print(f"SQLite seeded: FTS5 index built")

# ── Timing helpers ─────────────────────────────────────────────────────────────

def bench_mysql(sql, params=None, runs=RUNS):
    con = mysql.connector.connect(**MYSQL_CFG)
    cur = con.cursor()
    times = []
    for _ in range(runs):
        t0 = time.perf_counter()
        cur.execute(sql, params or ())
        cur.fetchall()
        times.append((time.perf_counter() - t0) * 1000)
    con.close()
    return times

def bench_sqlite(sql, params=None, runs=RUNS):
    db = sqlite3.connect(SQLITE_PATH)
    times = []
    for _ in range(runs):
        t0 = time.perf_counter()
        db.execute(sql, params or ()).fetchall()
        times.append((time.perf_counter() - t0) * 1000)
    db.close()
    return times

def med(times): return round(statistics.median(times), 2)

# ── Main ───────────────────────────────────────────────────────────────────────

def main():
    random.seed(42)
    print("=== WP Bench: MySQL 8.4 vs SQLite+FTS5 (Synapse stand-in) ===\n")

    mysql_seed()
    # Grab same posts data for SQLite seeding
    con = mysql.connector.connect(**MYSQL_CFG)
    posts_data = con.cursor()
    posts_data.execute("SELECT post_title, post_content FROM wp_posts ORDER BY ID")
    rows = posts_data.fetchall()
    con.close()
    sqlite_seed(rows)

    results = []

    # S1: Home — fetch latest 10 published posts + options autoload
    s1_mysql = """
        SELECT p.ID, p.post_title, p.post_date
        FROM wp_posts p
        WHERE p.post_status='publish' AND p.post_type='post'
        ORDER BY p.post_date DESC LIMIT 10
    """
    s1_sqlite = s1_mysql.replace("%s","?")
    # also fetch autoload options (every WP request does this)
    s1_opts_mysql = "SELECT option_name, option_value FROM wp_options WHERE autoload='yes'"
    s1_opts_sqlite = s1_opts_mysql

    def bench_s1_mysql():
        con = mysql.connector.connect(**MYSQL_CFG)
        cur = con.cursor()
        times = []
        for _ in range(RUNS):
            t0 = time.perf_counter()
            cur.execute(s1_mysql); cur.fetchall()
            cur.execute(s1_opts_mysql); cur.fetchall()
            times.append((time.perf_counter() - t0) * 1000)
        con.close()
        return times

    def bench_s1_sqlite():
        db = sqlite3.connect(SQLITE_PATH)
        times = []
        for _ in range(RUNS):
            t0 = time.perf_counter()
            db.execute(s1_sqlite).fetchall()
            db.execute(s1_opts_sqlite).fetchall()
            times.append((time.perf_counter() - t0) * 1000)
        db.close()
        return times

    m1 = bench_s1_mysql(); sq1 = bench_s1_sqlite()
    results.append(("S1 Home (posts+options)", med(m1), med(sq1)))

    # S2: Single post + related (5 posts sharing postmeta keys)
    s2_mysql = """
        SELECT p.*, pm.meta_key, pm.meta_value
        FROM wp_posts p
        LEFT JOIN wp_postmeta pm ON pm.post_id = p.ID
        WHERE p.ID = 42 AND p.post_status='publish'
    """
    s2_related_mysql = """
        SELECT p.ID, p.post_title FROM wp_posts p
        WHERE p.post_status='publish' AND p.post_type='post'
        AND p.ID != 42 ORDER BY RAND() LIMIT 5
    """
    s2_related_sqlite = """
        SELECT p.ID, p.post_title FROM wp_posts p
        WHERE p.post_status='publish' AND p.post_type='post'
        AND p.ID != 42 ORDER BY RANDOM() LIMIT 5
    """

    def bench_s2_mysql():
        con = mysql.connector.connect(**MYSQL_CFG)
        cur = con.cursor()
        times = []
        for _ in range(RUNS):
            t0 = time.perf_counter()
            cur.execute(s2_mysql); cur.fetchall()
            cur.execute(s2_related_mysql); cur.fetchall()
            times.append((time.perf_counter() - t0) * 1000)
        con.close()
        return times

    def bench_s2_sqlite():
        db = sqlite3.connect(SQLITE_PATH)
        s2_sq = s2_mysql
        times = []
        for _ in range(RUNS):
            t0 = time.perf_counter()
            db.execute(s2_sq).fetchall()
            db.execute(s2_related_sqlite).fetchall()
            times.append((time.perf_counter() - t0) * 1000)
        db.close()
        return times

    m2 = bench_s2_mysql(); sq2 = bench_s2_sqlite()
    results.append(("S2 Single post+related", med(m2), med(sq2)))

    # S3: Search "rust web framework" — LIKE vs FTS5
    s3_mysql = """
        SELECT ID, post_title FROM wp_posts
        WHERE post_status='publish'
        AND (post_title LIKE %s OR post_content LIKE %s)
        AND (post_title LIKE %s OR post_content LIKE %s)
        AND (post_title LIKE %s OR post_content LIKE %s)
    """
    s3_sqlite_fts = """
        SELECT p.ID, p.post_title
        FROM wp_posts_fts f JOIN wp_posts p ON p.ID = f.rowid
        WHERE wp_posts_fts MATCH 'rust web framework'
        LIMIT 20
    """
    m3 = bench_mysql(s3_mysql, ('%rust%','%rust%','%web%','%web%','%framework%','%framework%'))
    sq3 = bench_sqlite(s3_sqlite_fts)
    results.append(("S3 Search 'rust web framework'", med(m3), med(sq3)))

    # S4: wp-admin list posts (paginated, with postmeta join)
    s4_mysql = """
        SELECT SQL_CALC_FOUND_ROWS p.ID, p.post_title, p.post_date, p.post_status
        FROM wp_posts p
        WHERE p.post_type='post'
        ORDER BY p.post_date DESC LIMIT 20 OFFSET 0
    """
    s4_sqlite = """
        SELECT p.ID, p.post_title, p.post_date, p.post_status
        FROM wp_posts p
        WHERE p.post_type='post'
        ORDER BY rowid DESC LIMIT 20 OFFSET 0
    """
    m4 = bench_mysql(s4_mysql); sq4 = bench_sqlite(s4_sqlite)
    results.append(("S4 wp-admin posts list", med(m4), med(sq4)))

    # S5: WC-style product query (simulate with JOIN + meta filter)
    s5_mysql = """
        SELECT p.ID, p.post_title,
               MAX(CASE WHEN pm.meta_key='rating' THEN pm.meta_value END) as rating,
               MAX(CASE WHEN pm.meta_key='views' THEN pm.meta_value END) as views
        FROM wp_posts p
        JOIN wp_postmeta pm ON pm.post_id = p.ID
        WHERE p.post_status='publish'
        GROUP BY p.ID
        ORDER BY p.ID DESC LIMIT 12
    """
    s5_sqlite = s5_mysql
    m5 = bench_mysql(s5_mysql); sq5 = bench_sqlite(s5_sqlite)
    results.append(("S5 WC product archive (JOIN+GROUP)", med(m5), med(sq5)))

    # ── Report ──────────────────────────────────────────────────────────────────
    print(f"\n{'Scenario':<38} {'MySQL(ms)':>10} {'SQLite+FTS5':>12} {'Speedup':>9}")
    print("-"*73)
    for name, mysql_ms, sq_ms in results:
        gain = mysql_ms / sq_ms if sq_ms > 0 else float('inf')
        flag = " ⭐" if gain >= 10 else (" ✓" if gain >= 2 else " ⚠")
        print(f"{name:<38} {mysql_ms:>10.2f} {sq_ms:>12.2f} {gain:>8.1f}×{flag}")

    print("\nNotes:")
    print("  • MySQL: local Docker 8.4, 1 container, no tuning (innodb_buffer_pool default)")
    print("  • SQLite+FTS5: in-process, porter tokenizer, no HTTP overhead")
    print("  • synapse-mysql wire-proxy NOT running (scaffold only, on_query stub)")
    print("  • Real WP overhead (PHP, Apache, plugin stack) NOT included here")
    print("  • S3 FTS5 gain representative: LIKE is unindexed fullscan on MySQL")
    print("  • Production Synapse would add MRL-128 semantic layer on top of FTS5")

    # Save JSON results
    out = {"scenarios": [{"name":n,"mysql_ms":m,"sqlite_fts5_ms":s,"speedup":round(m/s,2) if s>0 else None}
                          for n,m,s in results]}
    with open("/tmp/wp_bench_results.json","w") as f:
        json.dump(out, f, indent=2)
    print(f"\nResults saved: /tmp/wp_bench_results.json")

if __name__ == "__main__":
    main()
