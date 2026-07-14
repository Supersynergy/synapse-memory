//! Small temporal cue parser for event-time retrieval.
//!
//! Supports ISO/RFC3339 timestamps, English and German relative dates, month
//! names, and Q1-Q4 ranges. Gregorian/Unix conversion is implemented here so
//! the portable memory binary does not need a date-time dependency or timezone
//! database. All output is UTC; ambiguous natural-language dates use UTC today.

const DAY: i64 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateRange {
    pub lo: i64,
    pub hi: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CivilDate {
    year: i32,
    month: u32,
    day: u32,
}

fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i32, month: u32) -> Option<u32> {
    Some(match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => return None,
    })
}

/// Proleptic Gregorian civil date to days since 1970-01-01.
/// Algorithm: Howard Hinnant's public-domain civil calendar arithmetic.
fn days_from_civil(date: CivilDate) -> Option<i64> {
    if !(1..=9999).contains(&date.year)
        || date.day == 0
        || date.day > days_in_month(date.year, date.month)?
    {
        return None;
    }
    let mut year = date.year as i64;
    if date.month <= 2 {
        year -= 1;
    }
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = date.month as i64;
    let day_of_year =
        (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + date.day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

fn civil_from_days(days: i64) -> Option<CivilDate> {
    let z = days.checked_add(719_468)?;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    let year = i32::try_from(year).ok()?;
    let date = CivilDate {
        year,
        month: u32::try_from(month).ok()?,
        day: u32::try_from(day).ok()?,
    };
    days_from_civil(date).map(|_| date)
}

fn date(year: i32, month: u32, day: u32) -> Option<CivilDate> {
    let value = CivilDate { year, month, day };
    days_from_civil(value).map(|_| value)
}

fn add_days(value: CivilDate, days: i64) -> Option<CivilDate> {
    civil_from_days(days_from_civil(value)?.checked_add(days)?)
}

fn day_bounds(value: CivilDate) -> (i64, i64) {
    let lo = days_from_civil(value).unwrap_or(0) * DAY;
    (lo, lo + DAY - 1)
}

fn week_of(value: CivilDate) -> Option<(i64, i64)> {
    let days = days_from_civil(value)?;
    // 1970-01-01 was Thursday, index 3 when Monday is zero.
    let monday = days - (days + 3).rem_euclid(7);
    let lo = monday * DAY;
    Some((lo, lo + 7 * DAY - 1))
}

fn month_of(year: i32, month: u32) -> Option<(i64, i64)> {
    let first = date(year, month, 1)?;
    let next = if month == 12 {
        date(year + 1, 1, 1)?
    } else {
        date(year, month + 1, 1)?
    };
    Some((day_bounds(first).0, day_bounds(next).0 - 1))
}

fn year_of(year: i32) -> Option<(i64, i64)> {
    Some((
        day_bounds(date(year, 1, 1)?).0,
        day_bounds(date(year + 1, 1, 1)?).0 - 1,
    ))
}

fn parse_number<T: std::str::FromStr>(value: &str, start: usize, end: usize) -> Option<T> {
    value.get(start..end)?.parse().ok()
}

fn parse_date(value: &str) -> Option<CivilDate> {
    if value.len() != 10 {
        return None;
    }
    let separator = value.as_bytes().get(4).copied()?;
    if !matches!(separator, b'-' | b'/') || value.as_bytes().get(7).copied()? != separator {
        return None;
    }
    date(
        parse_number(value, 0, 4)?,
        parse_number(value, 5, 7)?,
        parse_number(value, 8, 10)?,
    )
}

/// Parse YYYY-MM-DD, YYYY/MM/DD, or RFC3339 to Unix seconds.
pub fn parse_timestamp(value: &str) -> Option<i64> {
    let value = value.trim();
    let date = parse_date(value.get(..10)?)?;
    let day_start = days_from_civil(date)?.checked_mul(DAY)?;
    if value.len() == 10 {
        return Some(day_start);
    }
    if value.len() < 19 || !matches!(value.as_bytes().get(10), Some(b'T' | b't' | b' ')) {
        return None;
    }
    if value.as_bytes().get(13) != Some(&b':') || value.as_bytes().get(16) != Some(&b':') {
        return None;
    }
    let hour: i64 = parse_number(value, 11, 13)?;
    let minute: i64 = parse_number(value, 14, 16)?;
    let second: i64 = parse_number(value, 17, 19)?;
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let mut suffix = value.get(19..)?;
    if let Some(fraction) = suffix.strip_prefix('.') {
        let digits = fraction
            .char_indices()
            .take_while(|(_, value)| value.is_ascii_digit())
            .last()
            .map(|(index, value)| index + value.len_utf8())
            .unwrap_or(0);
        if digits == 0 {
            return None;
        }
        suffix = fraction.get(digits..)?;
    }
    let offset = match suffix {
        "" | "Z" | "z" => 0,
        value
            if value.len() == 6
                && matches!(value.as_bytes().first(), Some(b'+' | b'-'))
                && value.as_bytes().get(3) == Some(&b':') =>
        {
            let hours: i64 = parse_number(value, 1, 3)?;
            let minutes: i64 = parse_number(value, 4, 6)?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            let seconds = hours * 3600 + minutes * 60;
            if value.starts_with('-') {
                -seconds
            } else {
                seconds
            }
        }
        _ => return None,
    };
    day_start
        .checked_add(hour * 3600 + minute * 60 + second)?
        .checked_sub(offset)
}

/// Canonical UTC representation used in metadata and context output.
pub fn format_timestamp(ts: i64) -> Option<String> {
    let days = ts.div_euclid(DAY);
    let seconds = ts.rem_euclid(DAY);
    let date = civil_from_days(days)?;
    Some(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        date.year,
        date.month,
        date.day,
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60
    ))
}

fn month_num(name: &str) -> Option<u32> {
    Some(match name {
        "january" | "jan" | "januar" => 1,
        "february" | "feb" | "februar" => 2,
        "march" | "mar" | "märz" | "maerz" => 3,
        "april" | "apr" => 4,
        "may" | "mai" => 5,
        "june" | "jun" | "juni" => 6,
        "july" | "jul" | "juli" => 7,
        "august" | "aug" => 8,
        "september" | "sep" | "sept" => 9,
        "october" | "oct" | "oktober" | "okt" => 10,
        "november" | "nov" => 11,
        "december" | "dec" | "dezember" | "dez" => 12,
        _ => return None,
    })
}

fn unit_days(unit: &str) -> Option<i64> {
    Some(match unit {
        "day" | "days" | "tag" | "tage" | "tagen" => 1,
        "week" | "weeks" | "woche" | "wochen" => 7,
        "month" | "months" | "monat" | "monate" | "monaten" => 30,
        "year" | "years" | "jahr" | "jahre" | "jahren" => 365,
        _ => return None,
    })
}

fn normalized_query(query: &str) -> String {
    let mut query = query.to_lowercase();
    for (from, to) in [
        ("gestern", "yesterday"),
        ("heute", "today"),
        ("morgen", "tomorrow"),
        ("letzte woche", "last week"),
        ("letzten woche", "last week"),
        ("diese woche", "this week"),
        ("dieser woche", "this week"),
        ("nächste woche", "next week"),
        ("naechste woche", "next week"),
        ("letzten monat", "last month"),
        ("letzter monat", "last month"),
        ("diesen monat", "this month"),
        ("dieser monat", "this month"),
        ("nächsten monat", "next month"),
        ("naechsten monat", "next month"),
        ("letztes jahr", "last year"),
        ("dieses jahr", "this year"),
        ("nächstes jahr", "next year"),
        ("naechstes jahr", "next year"),
    ] {
        query = query.replace(from, to);
    }
    query
}

fn clean_word(value: &str) -> &str {
    value.trim_matches(|character: char| !character.is_alphanumeric())
}

/// Parse query text to an event-time range relative to `now`.
pub fn parse_temporal(query: &str, now: i64) -> Option<DateRange> {
    let query = normalized_query(query);
    let today = civil_from_days(now.div_euclid(DAY))?;

    for token in query.split(|value: char| {
        !(value.is_ascii_digit() || matches!(value, '-' | '/' | 'T' | ':' | '+' | 'Z'))
    }) {
        if token.len() >= 10
            && let Some(timestamp) = parse_timestamp(token)
        {
            return Some(DateRange {
                lo: timestamp - timestamp.rem_euclid(DAY),
                hi: timestamp - timestamp.rem_euclid(DAY) + DAY - 1,
            });
        }
    }

    for (needle, offset) in [("yesterday", -1), ("today", 0), ("tomorrow", 1)] {
        if query.contains(needle) {
            let (lo, hi) = day_bounds(add_days(today, offset)?);
            return Some(DateRange { lo, hi });
        }
    }

    let words: Vec<&str> = query.split_whitespace().collect();
    for index in 0..words.len() {
        let numeric = words[index].trim_matches(|value: char| !value.is_ascii_digit());
        if let Ok(count) = numeric.parse::<i64>() {
            if index + 2 < words.len()
                && clean_word(words[index + 2]) == "ago"
                && let Some(days) = unit_days(clean_word(words[index + 1]))
            {
                let (lo, hi) = day_bounds(add_days(today, -count * days)?);
                return Some(DateRange { lo, hi });
            }
            if index > 0
                && index + 1 < words.len()
                && words[index - 1] == "in"
                && let Some(days) = unit_days(clean_word(words[index + 1]))
            {
                let (lo, hi) = day_bounds(add_days(today, count * days)?);
                return Some(DateRange { lo, hi });
            }
            if index > 0
                && index + 1 < words.len()
                && words[index - 1] == "vor"
                && let Some(days) = unit_days(clean_word(words[index + 1]))
            {
                let (lo, hi) = day_bounds(add_days(today, -count * days)?);
                return Some(DateRange { lo, hi });
            }
        }
    }

    for (needle, week_offset, month_offset) in [
        ("last week", -1, 0),
        ("this week", 0, 0),
        ("next week", 1, 0),
        ("last month", 0, -1),
        ("this month", 0, 0),
        ("next month", 0, 1),
    ] {
        if query.contains(needle) {
            if needle.ends_with("week") {
                let (lo, hi) = week_of(add_days(today, week_offset * 7)?)?;
                return Some(DateRange { lo, hi });
            }
            let month_index = today.year as i64 * 12 + today.month as i64 - 1 + month_offset;
            let year = i32::try_from(month_index.div_euclid(12)).ok()?;
            let month = u32::try_from(month_index.rem_euclid(12) + 1).ok()?;
            let (lo, hi) = month_of(year, month)?;
            return Some(DateRange { lo, hi });
        }
    }
    for (needle, offset) in [("last year", -1), ("this year", 0), ("next year", 1)] {
        if query.contains(needle) {
            return year_of(today.year + offset).map(|(lo, hi)| DateRange { lo, hi });
        }
    }

    for (index, word) in words.iter().enumerate() {
        let token = word.trim_matches(|value: char| !value.is_ascii_alphanumeric());
        let quarter = token
            .strip_prefix('q')
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|value| (1..=4).contains(value));
        if let Some(quarter) = quarter {
            let year = words
                .get(index + 1)
                .and_then(|value| {
                    value
                        .trim_matches(|character: char| !character.is_ascii_digit())
                        .parse::<i32>()
                        .ok()
                })
                .filter(|value| (1900..2100).contains(value))
                .unwrap_or(today.year);
            let first_month = (quarter - 1) * 3 + 1;
            let (lo, _) = month_of(year, first_month)?;
            let (next_year, next_month) = if quarter == 4 {
                (year + 1, 1)
            } else {
                (year, first_month + 3)
            };
            return Some(DateRange {
                lo,
                hi: month_of(next_year, next_month)?.0 - 1,
            });
        }
    }

    for (index, word) in words.iter().enumerate() {
        if let Some(month) = month_num(clean_word(word)) {
            let year = words
                .get(index + 1)
                .and_then(|value| {
                    value
                        .trim_matches(|character: char| !character.is_ascii_digit())
                        .parse::<i32>()
                        .ok()
                })
                .filter(|value| (1900..2100).contains(value))
                .unwrap_or(today.year);
            return month_of(year, month).map(|(lo, hi)| DateRange { lo, hi });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(year: i32, month: u32, day: u32) -> CivilDate {
        date(year, month, day).unwrap()
    }

    fn mk_now() -> i64 {
        day_bounds(d(2026, 5, 3)).0 + 12 * 3600
    }

    #[test]
    fn civil_epoch_and_leap_day_roundtrip() {
        assert_eq!(days_from_civil(d(1970, 1, 1)), Some(0));
        let leap = d(2024, 2, 29);
        assert_eq!(civil_from_days(days_from_civil(leap).unwrap()), Some(leap));
        assert!(date(2025, 2, 29).is_none());
    }

    #[test]
    fn relative_dates() {
        let yesterday = parse_temporal("what did I do yesterday?", mk_now()).unwrap();
        assert_eq!(yesterday.lo, day_bounds(d(2026, 5, 2)).0);
        assert_eq!(yesterday.hi, day_bounds(d(2026, 5, 2)).1);
        assert_eq!(
            parse_temporal("the meeting 3 days ago", mk_now())
                .unwrap()
                .lo,
            day_bounds(d(2026, 4, 30)).0
        );
    }

    #[test]
    fn explicit_iso() {
        assert_eq!(
            parse_temporal("on 2025-12-25 i went home", mk_now())
                .unwrap()
                .lo,
            day_bounds(d(2025, 12, 25)).0
        );
    }

    #[test]
    fn last_week() {
        assert_eq!(
            parse_temporal("last week's report", mk_now()).unwrap().lo,
            day_bounds(d(2026, 4, 20)).0
        );
    }

    #[test]
    fn month_names() {
        assert_eq!(
            parse_temporal("trip in march 2025", mk_now()).unwrap().lo,
            month_of(2025, 3).unwrap().0
        );
        assert_eq!(
            parse_temporal("Plan im März 2025", mk_now()).unwrap().lo,
            month_of(2025, 3).unwrap().0
        );
    }

    #[test]
    fn german_relative_date() {
        assert_eq!(
            parse_temporal("Entscheidung vor 3 Tagen?", mk_now())
                .unwrap()
                .lo,
            day_bounds(d(2026, 4, 30)).0
        );
    }

    #[test]
    fn quarter_range() {
        let range = parse_temporal("Roadmap Q2 2025", mk_now()).unwrap();
        assert_eq!(
            range,
            DateRange {
                lo: month_of(2025, 4).unwrap().0,
                hi: month_of(2025, 7).unwrap().0 - 1,
            }
        );
    }

    #[test]
    fn timestamp_roundtrip_and_offset() {
        let ts = parse_timestamp("2026-05-03T12:00:00+02:00").unwrap();
        assert_eq!(
            format_timestamp(ts).as_deref(),
            Some("2026-05-03T10:00:00Z")
        );
        let fractional = parse_timestamp("2026-05-03T10:00:00.123Z").unwrap();
        assert_eq!(fractional, ts);
    }

    #[test]
    fn no_match() {
        assert!(parse_temporal("hello world", mk_now()).is_none());
    }
}
