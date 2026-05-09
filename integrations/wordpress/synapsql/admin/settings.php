<?php
/**
 * Settings page for synapsql.
 */
if (!defined('ABSPATH')) { exit; }

class Synapsql_Settings {
    public static function render() {
        if (!current_user_can('manage_options')) { return; }
        $opts = get_option('synapsql_options', []);
        $health = self::check_health($opts);
        ?>
        <div class="wrap">
            <h1>synapsql — synapse engine for WordPress</h1>

            <h2>Daemon Status</h2>
            <table class="widefat" style="max-width:600px">
                <tr><th>Health</th><td><?php echo $health['ok'] ? '✅ <strong>Connected</strong>' : '❌ <strong>Not reachable</strong>'; ?></td></tr>
                <?php if ($health['ok']): ?>
                <tr><th>Version</th><td><?php echo esc_html($health['version'] ?? '?'); ?></td></tr>
                <?php else: ?>
                <tr><th>Error</th><td><?php echo esc_html($health['error'] ?? 'unknown'); ?></td></tr>
                <?php endif; ?>
            </table>

            <h2>Configuration</h2>
            <form method="post" action="options.php">
                <?php settings_fields('synapsql'); ?>
                <table class="form-table">
                    <tr>
                        <th><label for="daemon_host">Daemon Host</label></th>
                        <td><input name="synapsql_options[daemon_host]" type="text" value="<?php echo esc_attr($opts['daemon_host'] ?? '127.0.0.1'); ?>" class="regular-text"></td>
                    </tr>
                    <tr>
                        <th><label for="daemon_mysql_port">MySQL Port</label></th>
                        <td><input name="synapsql_options[daemon_mysql_port]" type="number" value="<?php echo esc_attr($opts['daemon_mysql_port'] ?? 3306); ?>" min="1" max="65535"></td>
                    </tr>
                    <tr>
                        <th><label for="daemon_ops_port">Ops HTTP Port</label></th>
                        <td><input name="synapsql_options[daemon_ops_port]" type="number" value="<?php echo esc_attr($opts['daemon_ops_port'] ?? 9990); ?>" min="1" max="65535"></td>
                    </tr>
                    <tr>
                        <th>Install db.php drop-in</th>
                        <td>
                            <label><input name="synapsql_options[install_dropin]" type="checkbox" value="1" <?php checked(!empty($opts['install_dropin'])); ?>>
                            Route WordPress DB through synapse-server (advanced; requires daemon running)</label>
                            <p class="description">Off = side-by-side observability only. On = full DB replacement.</p>
                        </td>
                    </tr>
                    <tr>
                        <th>Enable Advisor</th>
                        <td><label><input name="synapsql_options[enable_advisor]" type="checkbox" value="1" <?php checked(!empty($opts['enable_advisor'])); ?>> Show index recommendations</label></td>
                    </tr>
                    <tr>
                        <th>Enable Slow-Query Log</th>
                        <td><label><input name="synapsql_options[enable_slowlog]" type="checkbox" value="1" <?php checked(!empty($opts['enable_slowlog'])); ?>> Record queries above threshold</label></td>
                    </tr>
                    <tr>
                        <th><label for="slow_threshold_ms">Slow Threshold (ms)</label></th>
                        <td><input name="synapsql_options[slow_threshold_ms]" type="number" value="<?php echo esc_attr($opts['slow_threshold_ms'] ?? 100); ?>" min="1"></td>
                    </tr>
                </table>
                <?php submit_button(); ?>
            </form>

            <?php if ($health['ok'] && !empty($opts['enable_slowlog'])): ?>
            <h2>Top-10 Slow Queries</h2>
            <?php self::render_slowlog($opts); ?>
            <?php endif; ?>

            <h2>Install Daemon</h2>
            <pre style="background:#f6f8fa;padding:1em;border-radius:4px">
# Linux/macOS:
curl -sSL https://synapse.dev/install | sh
synapse-server --mysql 127.0.0.1:3306 --ops-http 127.0.0.1:9990 --db /var/lib/synapse/wp.db --turbo --autolearn &amp;

# Or Docker:
docker run -d -p 3306:3306 -p 9990:9990 -v synapse-data:/data synapse/server:latest \
  --mysql 0.0.0.0:3306 --ops-http 0.0.0.0:9990 --db /data/wp.db --turbo --autolearn
            </pre>
        </div>
        <?php
    }

    private static function check_health($opts) {
        $host = $opts['daemon_host'] ?? '127.0.0.1';
        $port = absint($opts['daemon_ops_port'] ?? 9990);
        $url = "http://{$host}:{$port}/ops/health";
        $r = wp_remote_get($url, ['timeout' => 2]);
        if (is_wp_error($r)) { return ['ok' => false, 'error' => $r->get_error_message()]; }
        $body = wp_remote_retrieve_body($r);
        $data = json_decode($body, true);
        if (!$data) { return ['ok' => false, 'error' => 'invalid response']; }
        return ['ok' => ($data['status'] ?? '') === 'ok', 'version' => $data['version'] ?? null];
    }

    private static function render_slowlog($opts) {
        $host = $opts['daemon_host'] ?? '127.0.0.1';
        $port = absint($opts['daemon_ops_port'] ?? 9990);
        $r = wp_remote_get("http://{$host}:{$port}/ops/slowlog/top?n=10", ['timeout' => 2]);
        if (is_wp_error($r)) {
            echo '<p>Error: ' . esc_html($r->get_error_message()) . '</p>';
            return;
        }
        $data = json_decode(wp_remote_retrieve_body($r), true);
        if (empty($data['entries'])) { echo '<p>No slow queries recorded yet.</p>'; return; }
        echo '<table class="widefat striped"><thead><tr><th>Duration (µs)</th><th>SQL</th><th>Time</th></tr></thead><tbody>';
        foreach ($data['entries'] as $e) {
            echo '<tr>';
            echo '<td>' . esc_html(number_format($e['duration_us'])) . '</td>';
            echo '<td><code style="font-size:11px">' . esc_html(substr($e['sql'], 0, 200)) . '</code></td>';
            echo '<td>' . esc_html(date('H:i:s', $e['timestamp_unix'])) . '</td>';
            echo '</tr>';
        }
        echo '</tbody></table>';
    }
}
