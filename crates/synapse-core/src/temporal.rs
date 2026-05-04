//! Temporal cue parser: extract date hints from natural-language queries.
//! Returns optional (lo, hi) Unix-second range used as event_date filter.
//!
//! Patterns supported:
//! - "yesterday", "today", "tomorrow"
//! - "last/this/next week|month|year"
//! - "N days|weeks|months|years ago"
//! - "in N days|weeks|months"
//! - explicit "YYYY-MM-DD" or "YYYY/MM/DD"
//! - month names: "in March", "January 2025"
//!
//! Cheap regex+chrono only. No LLM.

use chrono::{Datelike, Duration, NaiveDate, TimeZone, Utc};

const DAY: i64 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateRange {
    pub lo: i64,
    pub hi: i64,
}

fn day_bounds(d: NaiveDate) -> (i64, i64) {
    let lo = Utc
        .from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap())
        .timestamp();
    (lo, lo + DAY - 1)
}

fn week_of(d: NaiveDate) -> (i64, i64) {
    let weekday = d.weekday().num_days_from_monday() as i64;
    let monday = d - Duration::days(weekday);
    let (lo, _) = day_bounds(monday);
    (lo, lo + 7 * DAY - 1)
}

fn month_of(year: i32, month: u32) -> Option<(i64, i64)> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let next_month = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)?
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)?
    };
    let (lo, _) = day_bounds(first);
    let (hi_start, _) = day_bounds(next_month);
    Some((lo, hi_start - 1))
}

fn year_of(year: i32) -> Option<(i64, i64)> {
    let first = NaiveDate::from_ymd_opt(year, 1, 1)?;
    let next = NaiveDate::from_ymd_opt(year + 1, 1, 1)?;
    let (lo, _) = day_bounds(first);
    let (hi_start, _) = day_bounds(next);
    Some((lo, hi_start - 1))
}

fn month_num(name: &str) -> Option<u32> {
    Some(match name {
        "january" | "jan" => 1,
        "february" | "feb" => 2,
        "march" | "mar" => 3,
        "april" | "apr" => 4,
        "may" => 5,
        "june" | "jun" => 6,
        "july" | "jul" => 7,
        "august" | "aug" => 8,
        "september" | "sep" | "sept" => 9,
        "october" | "oct" => 10,
        "november" | "nov" => 11,
        "december" | "dec" => 12,
        _ => return None,
    })
}

fn unit_days(unit: &str) -> Option<i64> {
    Some(match unit {
        "day" | "days" => 1,
        "week" | "weeks" => 7,
        "month" | "months" => 30,
        "year" | "years" => 365,
        _ => return None,
    })
}

/// Parse query text → optional date range relative to `now`.
/// Returns first match wins; tighter pattern checked first.
pub fn parse_temporal(query: &str, now: i64) -> Option<DateRange> {
    let q = query.to_lowercase();
    let today = Utc.timestamp_opt(now, 0).single()?.date_naive();

    // explicit YYYY-MM-DD or YYYY/MM/DD
    for fmt in &["%Y-%m-%d", "%Y/%m/%d"] {
        for tok in q.split(|c: char| !(c.is_ascii_digit() || c == '-' || c == '/')) {
            if let Ok(d) = NaiveDate::parse_from_str(tok, fmt) {
                let (lo, hi) = day_bounds(d);
                return Some(DateRange { lo, hi });
            }
        }
    }

    // simple keywords
    if q.contains("yesterday") {
        let (lo, hi) = day_bounds(today - Duration::days(1));
        return Some(DateRange { lo, hi });
    }
    if q.contains("today") {
        let (lo, hi) = day_bounds(today);
        return Some(DateRange { lo, hi });
    }
    if q.contains("tomorrow") {
        let (lo, hi) = day_bounds(today + Duration::days(1));
        return Some(DateRange { lo, hi });
    }

    // "N <unit> ago"
    let words: Vec<&str> = q.split_whitespace().collect();
    for i in 0..words.len() {
        if let Ok(n) = words[i].parse::<i64>() {
            if i + 2 < words.len() && words[i + 2] == "ago" {
                if let Some(d) = unit_days(words[i + 1]) {
                    let target = today - Duration::days(n * d);
                    let (lo, hi) = day_bounds(target);
                    return Some(DateRange { lo, hi });
                }
            }
            if i + 1 < words.len() && words[i - 1.min(i)] == "in" {
                if let Some(d) = unit_days(words[i + 1]) {
                    let target = today + Duration::days(n * d);
                    let (lo, hi) = day_bounds(target);
                    return Some(DateRange { lo, hi });
                }
            }
        }
    }

    // last/this/next week|month|year
    for (kw, offset_weeks, offset_months) in &[
        ("last week", -1i64, 0i64),
        ("this week", 0, 0),
        ("next week", 1, 0),
        ("last month", 0, -1),
        ("this month", 0, 0),
        ("next month", 0, 1),
    ] {
        if q.contains(kw) {
            let mut date = today + Duration::days(offset_weeks * 7);
            if *offset_months != 0 {
                let mut m = date.month() as i64 + offset_months;
                let mut y = date.year();
                while m < 1 {
                    m += 12;
                    y -= 1;
                }
                while m > 12 {
                    m -= 12;
                    y += 1;
                }
                date = NaiveDate::from_ymd_opt(y, m as u32, 1)?;
            }
            if kw.ends_with("week") {
                let (lo, hi) = week_of(date);
                return Some(DateRange { lo, hi });
            }
            if kw.ends_with("month") {
                let (lo, hi) = month_of(date.year(), date.month())?;
                return Some(DateRange { lo, hi });
            }
        }
    }
    if q.contains("last year") {
        return year_of(today.year() - 1).map(|(lo, hi)| DateRange { lo, hi });
    }
    if q.contains("this year") {
        return year_of(today.year()).map(|(lo, hi)| DateRange { lo, hi });
    }
    if q.contains("next year") {
        return year_of(today.year() + 1).map(|(lo, hi)| DateRange { lo, hi });
    }

    // month name [+ optional year]
    for w_idx in 0..words.len() {
        if let Some(m) = month_num(words[w_idx]) {
            let mut year = today.year();
            if w_idx + 1 < words.len() {
                if let Ok(y) = words[w_idx + 1].parse::<i32>() {
                    if (1900..2100).contains(&y) {
                        year = y;
                    }
                }
            }
            return month_of(year, m).map(|(lo, hi)| DateRange { lo, hi });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_now() -> i64 {
        // 2026-05-03 12:00:00 UTC
        Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 0).unwrap().timestamp()
    }

    #[test]
    fn yesterday() {
        let r = parse_temporal("what did I do yesterday?", mk_now()).unwrap();
        let (lo, hi) = day_bounds(NaiveDate::from_ymd_opt(2026, 5, 2).unwrap());
        assert_eq!(r.lo, lo);
        assert_eq!(r.hi, hi);
    }

    #[test]
    fn n_days_ago() {
        let r = parse_temporal("the meeting 3 days ago", mk_now()).unwrap();
        let (lo, _) = day_bounds(NaiveDate::from_ymd_opt(2026, 4, 30).unwrap());
        assert_eq!(r.lo, lo);
    }

    #[test]
    fn explicit_iso() {
        let r = parse_temporal("on 2025-12-25 i went home", mk_now()).unwrap();
        let (lo, _) = day_bounds(NaiveDate::from_ymd_opt(2025, 12, 25).unwrap());
        assert_eq!(r.lo, lo);
    }

    #[test]
    fn last_week() {
        let r = parse_temporal("last week's report", mk_now()).unwrap();
        // 2026-05-03 is Sunday, last week = Mon 2026-04-20 to Sun 2026-04-26
        let mon = NaiveDate::from_ymd_opt(2026, 4, 20).unwrap();
        let (lo, _) = day_bounds(mon);
        assert_eq!(r.lo, lo);
    }

    #[test]
    fn month_name() {
        let r = parse_temporal("trip in march 2025", mk_now()).unwrap();
        let (lo, _) = month_of(2025, 3).unwrap();
        assert_eq!(r.lo, lo);
    }

    #[test]
    fn no_match() {
        assert!(parse_temporal("hello world", mk_now()).is_none());
    }
}
