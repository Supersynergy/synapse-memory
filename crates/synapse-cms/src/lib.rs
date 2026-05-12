//! synapse-cms — platform-aware query optimizer FRAMEWORK.
//!
//! Generic MariaDB/MySQL/PG drop-in optimizer. Per-platform adapter recognizes
//! query patterns + applies fast-path execution + cache hooks.
//!
//! Adapters today: WP. Planned: WooCommerce, Shopify GraphQL, Magento EAV,
//! Drupal entity_load, Ghost post-list, Strapi collection-api, generic ORM.
//!
//! Architecture:
//!   query SQL → classifier (regex/AST) → PlanHint → fast-path executor
//!                                                 → fall-through to wire/Store

pub mod adapters {
    pub mod wp;
    pub mod wp_optimizer;
}

pub use adapters::wp::{classify, WpPattern};
pub use adapters::wp_optimizer::AutoloadCache;

/// Generic adapter trait — every platform impls this.
pub trait PlatformAdapter: Send + Sync + 'static {
    type Pattern;
    fn name(&self) -> &'static str;
    fn classify(&self, sql: &str) -> Self::Pattern;
}

pub struct Wp;
impl PlatformAdapter for Wp {
    type Pattern = WpPattern;
    fn name(&self) -> &'static str { "wordpress" }
    fn classify(&self, sql: &str) -> WpPattern { classify(sql) }
}
