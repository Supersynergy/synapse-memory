#!/usr/bin/env python3
"""Seed WP schema + 1000 posts into MySQL-compatible server."""
import sys
import random
import subprocess

MODE = sys.argv[1] if len(sys.argv) > 1 else "docker"  # "docker" or "direct:PORT"

MARIADB_CONTAINER = "wordpress-test-mariadb-1"
SYNAPSE_PORT = "13306"


def run_sql(sql: str, db="wordpress") -> bool:
    if MODE == "docker":
        cmd = ["docker", "exec", "-i", MARIADB_CONTAINER,
               "mariadb", "-uroot", "-psynapse", db]
    else:
        port = MODE.replace("direct:", "")
        cmd = ["/opt/homebrew/Cellar/mariadb/12.2.2/bin/mariadb",
               "-h127.0.0.1", f"-P{port}", "-uroot", "-psynapse",
               "--skip-ssl", db]
    result = subprocess.run(cmd, input=sql.encode(), capture_output=True)
    if result.returncode != 0:
        err = result.stderr.decode()[:300]
        if "already exists" not in err and "Duplicate entry" not in err:
            print(f"  WARN [{MODE}]: {err}", file=sys.stderr)
    return result.returncode == 0


DDL = """
CREATE TABLE IF NOT EXISTS wp_users (
  ID bigint(20) unsigned NOT NULL AUTO_INCREMENT,
  user_login varchar(60) NOT NULL DEFAULT '',
  user_pass varchar(255) NOT NULL DEFAULT '',
  user_email varchar(100) NOT NULL DEFAULT '',
  user_registered datetime NOT NULL DEFAULT '0000-00-00 00:00:00',
  PRIMARY KEY (ID)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS wp_posts (
  ID bigint(20) unsigned NOT NULL AUTO_INCREMENT,
  post_author bigint(20) unsigned NOT NULL DEFAULT '0',
  post_date datetime NOT NULL DEFAULT '0000-00-00 00:00:00',
  post_content longtext NOT NULL,
  post_title text NOT NULL,
  post_status varchar(20) NOT NULL DEFAULT 'publish',
  post_type varchar(20) NOT NULL DEFAULT 'post',
  PRIMARY KEY (ID),
  FULLTEXT KEY post_fulltext (post_title,post_content)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS wp_postmeta (
  meta_id bigint(20) unsigned NOT NULL AUTO_INCREMENT,
  post_id bigint(20) unsigned NOT NULL DEFAULT '0',
  meta_key varchar(255) DEFAULT '',
  meta_value longtext,
  PRIMARY KEY (meta_id),
  KEY post_id (post_id),
  KEY meta_key (meta_key(191))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
"""

topics = [
    "rust web framework performance guide",
    "wordpress optimization tips 2024",
    "mysql fulltext search tutorial",
    "php performance benchmarks",
    "docker compose setup guide",
    "web server apache nginx comparison",
    "database indexing strategies",
    "content management systems review",
    "ecommerce checkout optimization",
    "admin dashboard best practices",
]

print(f"[seed_wp:{MODE}] Creating schema...")
run_sql(DDL)
run_sql("INSERT IGNORE INTO wp_users (ID,user_login,user_pass,user_email,user_registered) VALUES (1,'admin','hash','admin@example.com',NOW());")

# Check if already seeded
if MODE == "docker":
    result = subprocess.run(
        ["docker", "exec", "-i", MARIADB_CONTAINER,
         "mariadb", "-uroot", "-psynapse", "wordpress",
         "--silent", "--skip-column-names", "-e", "SELECT COUNT(*) FROM wp_posts;"],
        capture_output=True
    )
else:
    port = MODE.replace("direct:", "")
    result = subprocess.run(
        ["/opt/homebrew/Cellar/mariadb/12.2.2/bin/mariadb",
         "-h127.0.0.1", f"-P{port}", "-uroot", "-psynapse", "--skip-ssl", "wordpress",
         "--silent", "--skip-column-names", "-e", "SELECT COUNT(*) FROM wp_posts;"],
        capture_output=True
    )

count = int(result.stdout.strip() or "0")
if count >= 1000:
    print(f"[seed_wp:{MODE}] Already {count} posts, skip.")
    sys.exit(0)

print(f"[seed_wp:{MODE}] Seeding 1000 posts...")
rows = []
for i in range(1, 1001):
    topic = topics[i % len(topics)]
    content = (f"This post covers {topic}. " * 50).replace("'", "''")
    title = f"{topic} #{i}".replace("'", "''")
    rows.append(f"(1, NOW() - INTERVAL {i} HOUR, '{content}', '{title}', 'publish', 'post')")

for chunk_start in range(0, len(rows), 100):
    chunk = rows[chunk_start:chunk_start + 100]
    sql = "INSERT INTO wp_posts (post_author,post_date,post_content,post_title,post_status,post_type) VALUES " + ",".join(chunk) + ";"
    run_sql(sql)

print(f"[seed_wp:{MODE}] Seeding postmeta...")
meta_rows = []
for i in range(1, 1001):
    meta_rows.append(f"({i},'_thumbnail_id','{i + 1000}')")
    meta_rows.append(f"({i},'_price','{random.randint(10, 999)}.99')")
    meta_rows.append(f"({i},'_stock_status','instock')")

for chunk_start in range(0, len(meta_rows), 300):
    chunk = meta_rows[chunk_start:chunk_start + 300]
    sql = "INSERT INTO wp_postmeta (post_id,meta_key,meta_value) VALUES " + ",".join(chunk) + ";"
    run_sql(sql)

print(f"[seed_wp:{MODE}] Done.")
