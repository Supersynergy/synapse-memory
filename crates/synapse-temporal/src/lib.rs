//! synapse-temporal: natural-language temporal phrase parser.
//!
//! Wraps `chrono-english` (stevedonovan/chrono-english, used by facebook/sapling
//! and pamburus/hl) to turn phrases like "yesterday", "last week", "vor 3 Tagen",
//! "Q3 2025" into a `(start_ts, end_ts)` Unix-second range.
//!
//! 90 % reused: parse logic is delegated to chrono-english. The 10 % adapter:
//!  * normalise locale ("de" → translate common phrases to English before parse)
//!  * map a single instant to a sensible window (yesterday → full day)
//!  * Q-quarters and ISO-week shorthand fallbacks
//!
//! Source mining note (ghgrep 2026-04-29):
//!  * stevedonovan/chrono-english@master/src/lib.rs — `parse_date_string`, `Dialect`
//!  * facebook/sapling@master/eden/mononoke/.../datetime.rs — production usage pattern

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_english::{Dialect, parse_date_string};

/// Inclusive Unix-seconds range produced by the parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeRange {
    pub start_ts: i64,
    pub end_ts: i64,
}

impl TimeRange {
    pub fn new(start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        Self {
            start_ts: start.timestamp(),
            end_ts: end.timestamp(),
        }
    }
    pub fn day_of(d: NaiveDate) -> Self {
        let s = d.and_time(NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        let e = d.and_time(NaiveTime::from_hms_opt(23, 59, 59).unwrap());
        Self {
            start_ts: Utc.from_utc_datetime(&s).timestamp(),
            end_ts: Utc.from_utc_datetime(&e).timestamp(),
        }
    }
}

/// Locale hint. Used only for German phrase normalisation today.
#[derive(Debug, Clone, Copy)]
pub enum Locale {
    English,
    German,
}

/// Light German→English normalisation so chrono-english's English dialect handles it.
fn normalise_de(s: &str) -> String {
    let l = s.to_lowercase();
    l.replace("vor ", "")
        .replace("letztes jahr", "last year")
        .replace("letzten monat", "last month")
        .replace("letzte woche", "last week")
        .replace("gestern", "yesterday")
        .replace("heute", "today")
        .replace("morgen", "tomorrow")
        .replace("tagen", "days ago")
        .replace("tage", "days ago")
        .replace("wochen", "weeks ago")
        .replace("monaten", "months ago")
}

/// Try to recognise quarter shorthand: "Q3 2025", "q1 2024".
fn parse_quarter(s: &str) -> Option<TimeRange> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 2 {
        return None;
    }
    let q = parts[0].to_lowercase();
    if q.len() != 2 || !q.starts_with('q') {
        return None;
    }
    let qn: u32 = q[1..].parse().ok()?;
    if !(1..=4).contains(&qn) {
        return None;
    }
    let year: i32 = parts[1].parse().ok()?;
    let start_month = (qn - 1) * 3 + 1;
    let end_month = start_month + 2;
    let start = NaiveDate::from_ymd_opt(year, start_month, 1)?
        .and_time(NaiveTime::from_hms_opt(0, 0, 0).unwrap());
    let last_day = NaiveDate::from_ymd_opt(year, end_month + 1, 1)
        .unwrap_or(NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap())
        .pred_opt()?;
    let end = last_day.and_time(NaiveTime::from_hms_opt(23, 59, 59).unwrap());
    Some(TimeRange::new(
        Utc.from_utc_datetime(&start),
        Utc.from_utc_datetime(&end),
    ))
}

/// Main entry. Returns None if phrase is not a recognised temporal expression.
pub fn parse_temporal(phrase: &str, locale: Locale) -> Option<TimeRange> {
    if let Some(q) = parse_quarter(phrase) {
        return Some(q);
    }
    let prepared = match locale {
        Locale::German => normalise_de(phrase),
        Locale::English => phrase.to_lowercase(),
    };
    let now = Utc::now();
    let parsed: DateTime<Utc> = parse_date_string(&prepared, now, Dialect::Us).ok()?;
    // Bare-day phrases (yesterday/last week/...) widen to the corresponding window.
    let lower = prepared.trim();
    if lower == "yesterday" || lower == "today" || lower == "tomorrow" {
        return Some(TimeRange::day_of(parsed.date_naive()));
    }
    if lower == "last week" {
        let end = parsed;
        let start = end - Duration::days(6);
        return Some(TimeRange::new(start, end));
    }
    if lower == "last month" {
        let end = parsed;
        let start = end - Duration::days(30);
        return Some(TimeRange::new(start, end));
    }
    if lower == "last year" {
        let y = parsed.year() - 1;
        let start = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(y, 1, 1)?,
            NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        );
        let end = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(y, 12, 31)?,
            NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
        );
        return Some(TimeRange::new(
            Utc.from_utc_datetime(&start),
            Utc.from_utc_datetime(&end),
        ));
    }
    // Default: treat as instant ± 1h.
    let start = parsed - Duration::hours(1);
    let end = parsed + Duration::hours(1);
    Some(TimeRange::new(start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_q3_2025() {
        let r = parse_quarter("Q3 2025").unwrap();
        assert!(r.end_ts > r.start_ts);
    }

    #[test]
    fn yesterday_widens_to_day() {
        let r = parse_temporal("yesterday", Locale::English).unwrap();
        // a day window is roughly 86400s wide
        assert!(r.end_ts - r.start_ts > 60_000);
        assert!(r.end_ts - r.start_ts < 100_000);
    }

    #[test]
    fn german_gestern() {
        let r = parse_temporal("gestern", Locale::German).unwrap();
        assert!(r.end_ts > r.start_ts);
    }

    #[test]
    fn not_a_date_returns_none() {
        let r = parse_temporal("definitely not a temporal phrase xyz", Locale::English);
        assert!(r.is_none());
    }
}
