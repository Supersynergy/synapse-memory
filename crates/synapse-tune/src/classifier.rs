//! Bot-vs-User classifier — decide cache-skip per request.
//!
//! Heuristic v1: User-Agent regex + request-rate threshold.
//! P3: XGBoost trained on labeled (request, is_bot) data.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    RealUser,    // serve from cache + populate cache
    Bot,         // bypass cache, no cache populate
    Unknown,     // default safe path (cache, conservative TTL)
}

pub struct BotClassifier {
    rates: RwLock<HashMap<String, RateWindow>>,
    bot_rate_threshold: f64,  // requests/sec
}

#[derive(Clone, Copy)]
struct RateWindow {
    count: u32,
    started: Instant,
}

impl BotClassifier {
    pub fn new(bot_rate_threshold: f64) -> Self {
        Self {
            rates: RwLock::new(HashMap::new()),
            bot_rate_threshold,
        }
    }

    /// Classify request. `key` = IP/UA hash. `user_agent` = HTTP UA header.
    pub fn classify(&self, key: &str, user_agent: &str) -> Classification {
        // Fast path: known bot UA patterns
        if is_bot_ua(user_agent) { return Classification::Bot; }
        if is_real_browser(user_agent) {
            // Rate check — even browsers can be malicious-script
            if self.over_rate(key) { return Classification::Bot; }
            return Classification::RealUser;
        }
        Classification::Unknown
    }

    fn over_rate(&self, key: &str) -> bool {
        let now = Instant::now();
        if let Ok(mut g) = self.rates.write() {
            let entry = g.entry(key.into()).or_insert(RateWindow { count: 0, started: now });
            if now.duration_since(entry.started) > Duration::from_secs(1) {
                entry.count = 1;
                entry.started = now;
                false
            } else {
                entry.count += 1;
                (entry.count as f64) > self.bot_rate_threshold
            }
        } else {
            false
        }
    }
}

impl Default for BotClassifier {
    fn default() -> Self { Self::new(50.0) }  // 50 req/sec threshold
}

fn is_bot_ua(ua: &str) -> bool {
    let l = ua.to_lowercase();
    // Top-N bot fingerprints (Crawler/Bot/Spider/curl/wget/python/scrapy)
    const PATTERNS: &[&str] = &[
        "bot", "crawler", "spider", "curl/", "wget/", "python-requests",
        "scrapy", "ahrefs", "semrush", "googlebot", "bingbot", "yandex",
        "facebookexternalhit", "twitterbot", "linkedinbot", "slackbot",
        "headlesschrome", "phantomjs", "selenium",
    ];
    PATTERNS.iter().any(|p| l.contains(p))
}

fn is_real_browser(ua: &str) -> bool {
    let l = ua.to_lowercase();
    (l.contains("mozilla/") || l.contains("safari/") || l.contains("chrome/") || l.contains("firefox/"))
        && !is_bot_ua(ua)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_googlebot() {
        let c = BotClassifier::default();
        assert_eq!(
            c.classify("k1", "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)"),
            Classification::Bot
        );
    }
    #[test]
    fn detects_real_browser() {
        let c = BotClassifier::default();
        assert_eq!(
            c.classify("user1", "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605 Chrome/130.0"),
            Classification::RealUser
        );
    }
    #[test]
    fn detects_curl() {
        let c = BotClassifier::default();
        assert_eq!(c.classify("k", "curl/8.4.0"), Classification::Bot);
    }
    #[test]
    fn rate_threshold_marks_browser_as_bot() {
        let c = BotClassifier::new(5.0);
        let ua = "Mozilla/5.0 Chrome/130";
        for _ in 0..6 {
            c.classify("ip1", ua);
        }
        assert_eq!(c.classify("ip1", ua), Classification::Bot);
    }
}
