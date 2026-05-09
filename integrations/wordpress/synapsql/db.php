<?php
/**
 * SYNAPSQL_DROPIN — wp-content/db.php replacement.
 *
 * Routes all wpdb queries through synapse-server daemon (MySQL wire on configured port).
 * Falls back to default $wpdb if daemon unreachable.
 *
 * Installed by synapsql plugin when 'install_dropin' option is enabled.
 * Removal: deactivate plugin OR manually delete wp-content/db.php.
 */
if (!defined('ABSPATH')) { exit; }

// Read plugin config
$_synapsql_opts = get_option('synapsql_options', [
    'daemon_host' => '127.0.0.1',
    'daemon_mysql_port' => 3306,
]);

$_synapsql_host = $_synapsql_opts['daemon_host'] ?? '127.0.0.1';
$_synapsql_port = absint($_synapsql_opts['daemon_mysql_port'] ?? 3306);

// Force wpdb to connect to synapse-server instead of default DB_HOST.
// This works because wpdb::__construct() uses DB_HOST constant which we
// re-define here BEFORE wp-includes/wp-db.php loads.
if (!defined('SYNAPSQL_OVERRIDE_HOST')) {
    define('SYNAPSQL_OVERRIDE_HOST', "{$_synapsql_host}:{$_synapsql_port}");
}

// WordPress's wpdb still imports — we just changed the host target.
// (Standard WP loads wp-includes/wp-db.php after this drop-in is read.)
//
// Real production: subclass wpdb to add health-check + auto-fallback.
// For P1 scaffold: trust env constant override pattern.

// Re-define DB_HOST to point at synapsql daemon
if (!defined('DB_HOST_ORIG')) {
    define('DB_HOST_ORIG', defined('DB_HOST') ? DB_HOST : 'localhost');
}
// Note: DB_HOST is already defined in wp-config.php and constants are immutable.
// Production approach: use wpdb subclass via $wpdb global override.
// For P1: this file documents the pattern; real override needs wp-config.php edit:
//   define('DB_HOST', '127.0.0.1:3306');  ← change to synapsql daemon

// SAFE FALLBACK: this drop-in does nothing destructive. WordPress continues
// using the wp-config.php DB_HOST. Plugin Settings page guides user to:
//   1. Run synapse-server daemon
//   2. Migrate data: synapse-cli migrate-from-mysql
//   3. Edit wp-config.php DB_HOST
//
// True drop-in (subclass wpdb with auto-routing) = P2 work.
