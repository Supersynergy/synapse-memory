//! synapse-wp — WordPress-aware query optimizer.
//!
//! Layer 1 of 100×WP plan (docs/ARCHITECTURE-100X-WP.md).
//! Recognizes top-50 `WP_Query` patterns + `wp_options` autoload + meta-query joins,
//! rewrites to fast-path execution.

// AutoloadCache lives in adapters::wp_optimizer

use once_cell::sync::Lazy;
use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WpPattern {
    /// `SELECT option_value FROM wp_options WHERE option_name = ?`
    SingleOption,
    /// `SELECT option_name, option_value FROM wp_options WHERE autoload = 'yes'`
    AutoloadAll,
    /// `SELECT SQL_CALC_FOUND_ROWS ... FROM wp_posts WHERE ... ORDER BY ... LIMIT ?,?`
    PostsListing,
    /// `SELECT FOUND_ROWS()` — eliminate via tracked scan
    FoundRows,
    /// `SELECT * FROM wp_postmeta WHERE post_id IN (?,?,...) AND meta_key = ?`
    PostmetaJoin,
    /// `SELECT * FROM wp_users WHERE user_login = ?`
    UserLookup,
    /// `SELECT * FROM wp_terms ...` taxonomy lookup
    TermLookup,
    /// `SELECT * FROM wp_comments WHERE comment_post_ID = ? AND comment_approved = '1'`
    CommentsForPost,
    /// Unknown — passthrough.
    Other,
}

static RE_AUTOLOAD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)SELECT\s+option_name,?\s*option_value\s+FROM\s+\w*options\s+WHERE\s+autoload\s*=\s*'?yes'?").unwrap()
});
static RE_SINGLE_OPT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)SELECT\s+option_value\s+FROM\s+\w*options\s+WHERE\s+option_name\s*=").unwrap()
});
static RE_FOUND_ROWS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)SELECT\s+FOUND_ROWS\(\)").unwrap());
static RE_POSTS_LISTING: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)SQL_CALC_FOUND_ROWS.*FROM\s+\w*posts\b").unwrap());
static RE_POSTMETA: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)FROM\s+\w*postmeta\b.*post_id\s+IN").unwrap());
static RE_USERS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)FROM\s+\w*users\s+WHERE\s+user_login\s*=").unwrap());
static RE_TERMS: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)FROM\s+\w*terms\b").unwrap());
static RE_COMMENTS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)FROM\s+\w*comments\s+WHERE\s+comment_post_ID").unwrap());

pub fn classify(sql: &str) -> WpPattern {
    if RE_AUTOLOAD.is_match(sql) {
        return WpPattern::AutoloadAll;
    }
    if RE_SINGLE_OPT.is_match(sql) {
        return WpPattern::SingleOption;
    }
    if RE_FOUND_ROWS.is_match(sql) {
        return WpPattern::FoundRows;
    }
    if RE_POSTS_LISTING.is_match(sql) {
        return WpPattern::PostsListing;
    }
    if RE_POSTMETA.is_match(sql) {
        return WpPattern::PostmetaJoin;
    }
    if RE_USERS.is_match(sql) {
        return WpPattern::UserLookup;
    }
    if RE_TERMS.is_match(sql) {
        return WpPattern::TermLookup;
    }
    if RE_COMMENTS.is_match(sql) {
        return WpPattern::CommentsForPost;
    }
    WpPattern::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_autoload() {
        assert_eq!(
            classify("SELECT option_name, option_value FROM wp_options WHERE autoload = 'yes'"),
            WpPattern::AutoloadAll
        );
    }
    #[test]
    fn classifies_single_option() {
        assert_eq!(
            classify("SELECT option_value FROM wp_options WHERE option_name = 'siteurl'"),
            WpPattern::SingleOption
        );
    }
    #[test]
    fn classifies_found_rows() {
        assert_eq!(classify("SELECT FOUND_ROWS()"), WpPattern::FoundRows);
    }
    #[test]
    fn classifies_posts_listing() {
        assert_eq!(
            classify(
                "SELECT SQL_CALC_FOUND_ROWS wp_posts.* FROM wp_posts WHERE post_status='publish' ORDER BY post_date DESC LIMIT 0,10"
            ),
            WpPattern::PostsListing
        );
    }
    #[test]
    fn classifies_postmeta() {
        assert_eq!(
            classify(
                "SELECT post_id, meta_key, meta_value FROM wp_postmeta WHERE post_id IN (1,2,3) AND meta_key = '_thumbnail_id'"
            ),
            WpPattern::PostmetaJoin
        );
    }
    #[test]
    fn unknown_passthrough() {
        assert_eq!(classify("SELECT 1"), WpPattern::Other);
    }
    #[test]
    fn case_insensitive() {
        assert_eq!(classify("select FOUND_ROWS()"), WpPattern::FoundRows);
    }
}
