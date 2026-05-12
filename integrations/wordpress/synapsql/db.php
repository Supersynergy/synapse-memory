<?php
/**
 * SYNAPSQL_DROPIN — wp-content/db.php replacement.
 *
 * Subclasses wpdb to route queries through synapse-server daemon (MySQL wire),
 * with auto-fallback to original DB_HOST when daemon unreachable.
 *
 * Activated by synapsql plugin when 'install_dropin' option enabled.
 * Removal: deactivate plugin OR rm wp-content/db.php.
 */
if (!defined('ABSPATH')) { exit; }

if (!class_exists('Synapsql_DB')) {
    class Synapsql_DB extends wpdb {
        private $synapse_host;
        private $synapse_port;
        private $synapse_active = false;
        private $fallback_host;

        public function __construct($dbuser, $dbpassword, $dbname, $dbhost) {
            // Read plugin config
            $opts = function_exists('get_option') ? get_option('synapsql_options', []) : [];
            $this->synapse_host = $opts['daemon_host'] ?? '127.0.0.1';
            $this->synapse_port = absint($opts['daemon_mysql_port'] ?? 3306);
            $this->fallback_host = $dbhost;

            $synapse_endpoint = "{$this->synapse_host}:{$this->synapse_port}";

            // Health-check via TCP socket (cheap, ~1ms)
            if ($this->ping_synapse()) {
                $this->synapse_active = true;
                parent::__construct($dbuser, $dbpassword, $dbname, $synapse_endpoint);
            } else {
                // Fall through to original DB_HOST
                parent::__construct($dbuser, $dbpassword, $dbname, $dbhost);
            }
        }

        private function ping_synapse() {
            $errno = 0; $errstr = '';
            $fp = @fsockopen($this->synapse_host, $this->synapse_port, $errno, $errstr, 0.1);
            if ($fp) { fclose($fp); return true; }
            return false;
        }

        public function is_synapse_active() { return $this->synapse_active; }
    }
}

global $wpdb;
$wpdb = new Synapsql_DB(DB_USER, DB_PASSWORD, DB_NAME, DB_HOST);
