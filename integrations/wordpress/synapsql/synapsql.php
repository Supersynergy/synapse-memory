<?php
/**
 * Plugin Name:       synapsql — DB Drop-in for WordPress
 * Plugin URI:        https://synapse.dev/wordpress
 * Description:       Drop-in MySQL replacement powered by synapse engine. 100× faster cached pageload, 32× faster INSERTs, real-time slow-query log + index advisor. Self-host friendly, Apache 2.0, no SaaS lock-in.
 * Version:           0.1.0
 * Requires at least: 5.0
 * Requires PHP:      7.4
 * Author:            Maxim Supersynergy
 * Author URI:        https://supersynergy.de
 * License:           Apache-2.0
 * License URI:       https://www.apache.org/licenses/LICENSE-2.0
 * Text Domain:       synapsql
 * Network:           true
 *
 * @package synapsql
 */

if (!defined('ABSPATH')) { exit; }

define('SYNAPSQL_VERSION', '0.1.0');
define('SYNAPSQL_DIR', plugin_dir_path(__FILE__));
define('SYNAPSQL_URL', plugin_dir_url(__FILE__));

require_once SYNAPSQL_DIR . 'admin/settings.php';

class Synapsql_Plugin {
    private static $instance = null;
    public static function instance() {
        if (null === self::$instance) { self::$instance = new self(); }
        return self::$instance;
    }
    private function __construct() {
        add_action('admin_menu', [$this, 'admin_menu']);
        add_action('admin_init', [$this, 'register_settings']);
        register_activation_hook(__FILE__, [$this, 'activate']);
        register_deactivation_hook(__FILE__, [$this, 'deactivate']);
    }

    public function activate() {
        // Optionally drop in db.php for full Store-bypass (advanced). Default: side-by-side observability only.
        $opts = get_option('synapsql_options', $this->defaults());
        if (!empty($opts['install_dropin'])) {
            $this->install_dropin();
        }
    }
    public function deactivate() {
        // Always remove db.php on deactivate (safe-rollback)
        $this->remove_dropin();
    }

    public function defaults() {
        return [
            'daemon_host'      => '127.0.0.1',
            'daemon_mysql_port'=> 3306,
            'daemon_ops_port'  => 9990,
            'install_dropin'   => 0,
            'enable_advisor'   => 1,
            'enable_slowlog'   => 1,
            'slow_threshold_ms'=> 100,
        ];
    }

    public function admin_menu() {
        add_options_page(
            'synapsql',
            'synapsql',
            'manage_options',
            'synapsql-settings',
            ['Synapsql_Settings', 'render']
        );
    }
    public function register_settings() {
        register_setting('synapsql', 'synapsql_options', [
            'sanitize_callback' => [$this, 'sanitize'],
            'default' => $this->defaults(),
        ]);
    }
    public function sanitize($input) {
        $clean = $this->defaults();
        $clean['daemon_host']       = sanitize_text_field($input['daemon_host'] ?? '127.0.0.1');
        $clean['daemon_mysql_port'] = absint($input['daemon_mysql_port'] ?? 3306);
        $clean['daemon_ops_port']   = absint($input['daemon_ops_port'] ?? 9990);
        $clean['install_dropin']    = !empty($input['install_dropin']) ? 1 : 0;
        $clean['enable_advisor']    = !empty($input['enable_advisor']) ? 1 : 0;
        $clean['enable_slowlog']    = !empty($input['enable_slowlog']) ? 1 : 0;
        $clean['slow_threshold_ms'] = absint($input['slow_threshold_ms'] ?? 100);
        return $clean;
    }

    private function install_dropin() {
        $src = SYNAPSQL_DIR . 'db.php';
        $dst = WP_CONTENT_DIR . '/db.php';
        if (file_exists($src) && !file_exists($dst)) {
            @copy($src, $dst);
        }
    }
    private function remove_dropin() {
        $dst = WP_CONTENT_DIR . '/db.php';
        if (file_exists($dst)) {
            $content = @file_get_contents($dst);
            if ($content && strpos($content, 'SYNAPSQL_DROPIN') !== false) {
                @unlink($dst);
            }
        }
    }
}

Synapsql_Plugin::instance();
