<?php
/**
 * synapsql uninstall — clean removal.
 */
if (!defined('WP_UNINSTALL_PLUGIN')) { exit; }

delete_option('synapsql_options');
delete_site_option('synapsql_options');

// Remove drop-in if installed
$dropin = WP_CONTENT_DIR . '/db.php';
if (file_exists($dropin)) {
    $content = @file_get_contents($dropin);
    if ($content && strpos($content, 'SYNAPSQL_DROPIN') !== false) {
        @unlink($dropin);
    }
}
