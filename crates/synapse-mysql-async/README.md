# synapse-mysql-async

Zero-config MySQL wire-protocol drop-in backed by SQLite. Run WordPress, Drupal, and Joomla against Synapse with no CMS changes beyond `DB_HOST`.

## Install

### macOS (Homebrew)

```bash
brew tap supersynergy/tap
brew install supersynergy/tap/synapse-mysql
brew services start synapse-mysql
```

### Linux (one-liner)

```bash
curl -fsSL https://github.com/Supersynergy/synapse/releases/latest/download/install.sh | bash
```

### Cargo

```bash
cargo install synapse-mysql-async
mkdir -p /var/lib/synapse
synapse-mysql-async -f /var/lib/synapse/default.db -b 0.0.0.0:3306
```

## Default Credentials

| Field    | Value       |
|----------|-------------|
| host     | 127.0.0.1   |
| port     | 3306        |
| user     | root        |
| password | synapse     |

No changes needed to MySQL client libraries, ORMs, or CMS drivers.

## CMS Quick-Start

### WordPress

```bash
wp config set DB_HOST 127.0.0.1
# DB_USER=root, DB_PASSWORD=synapse already match defaults
```

### Drupal

```php
$databases['default']['default']['host'] = '127.0.0.1';
$databases['default']['default']['username'] = 'root';
$databases['default']['default']['password'] = 'synapse';
```

### Joomla

Edit `configuration.php`:
```php
public $host = '127.0.0.1';
public $user = 'root';
public $password = 'synapse';
```

## Data

- Default data dir: `/var/lib/synapse/` (Linux) or `/usr/local/var/synapse/` (macOS)
- Each database maps to a `.db` SQLite file
- Auto-created on first connection

## Service Management

```bash
# macOS
brew services start|stop|restart synapse-mysql

# Linux
systemctl start|stop|restart synapse-mysqld
systemctl status synapse-mysqld
```

## Smoke Test

```python
import pymysql
c = pymysql.connect(host='127.0.0.1', port=3306, user='root', password='synapse', database='wordpress')
cur = c.cursor()
cur.execute('SELECT VERSION()')
print(cur.fetchone())
c.close()
```
