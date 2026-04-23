#!/bin/bash
set -e

# Copy WordPress files if not present
if [ ! -e /var/www/html/index.php ]; then
    echo "Copying WordPress files..."
    cp -r /usr/src/wordpress/* /var/www/html/
fi

# Ensure proper permissions
chown -R www-data:www-data /var/www/html

# Create wp-config.php if missing
if [ ! -e /var/www/html/wp-config.php ]; then
    echo "Creating wp-config.php..."
    cat > /var/www/html/wp-config.php << 'WPEOF'
<?php
define('DB_NAME', 'wordpress');
define('DB_USER', 'root');
define('DB_PASSWORD', 'synapse');
define('DB_HOST', 'localhost');
define('DB_CHARSET', 'utf8mb4');
define('DB_COLLATE', '');

define('AUTH_KEY',         'put your unique phrase here');
define('SECURE_AUTH_KEY',  'put your unique phrase here');
define('LOGGED_IN_KEY',    'put your unique phrase here');
define('NONCE_KEY',        'put your unique phrase here');
define('AUTH_SALT',        'put your unique phrase here');
define('SECURE_AUTH_SALT', 'put your unique phrase here');
define('LOGGED_IN_SALT',   'put your unique phrase here');
define('NONCE_SALT',       'put your unique phrase here');

$table_prefix = 'wp_';

define('WP_DEBUG', true);

if (!defined('ABSPATH')) {
    define('ABSPATH', __DIR__ . '/');
}
require_once ABSPATH . 'wp-settings.php';
WPEOF
fi

# Run the original command (apache2-foreground)
exec "$@"
