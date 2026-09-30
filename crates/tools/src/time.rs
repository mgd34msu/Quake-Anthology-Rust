//! UTC timestamps (`Date.toISOString` shape) and ISO parsing.
//!
//! Timestamps render as `YYYY-MM-DDTHH:MM:SS.mmmZ` with millisecond precision.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::ToolsError;

/// Current UTC time rendered like `Date.toISOString`.
#[must_use]
pub fn now_iso() -> String {
    render_iso(now_millis())
}

/// Current UTC time in whole milliseconds since the epoch.
#[must_use]
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// Render epoch milliseconds as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
#[must_use]
pub fn render_iso(millis: i64) -> String {
    let secs = millis.div_euclid(1000);
    let ms = millis.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let time = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{ms:03}Z",
        time / 3600,
        time % 3600 / 60,
        time % 60
    )
}

/// Parse a canonical `YYYY-MM-DDTHH:MM:SS.mmmZ` timestamp to epoch milliseconds.
///
/// This mirrors the `new Date(text).toISOString() === text` round-trip check:
/// only the exact canonical shape is accepted.
pub fn parse_iso(text: &str) -> Result<i64, ToolsError> {
    let bytes = text.as_bytes();
    if bytes.len() != 24
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'.'
        || bytes[23] != b'Z'
    {
        return Err(ToolsError::parse("timestamp must be an ISO UTC timestamp"));
    }
    let digits = |range: std::ops::Range<usize>| -> Result<i64, ToolsError> {
        text[range]
            .parse::<i64>()
            .map_err(|_| ToolsError::parse("timestamp must be an ISO UTC timestamp"))
    };
    let year = digits(0..4)?;
    let month = digits(5..7)?;
    let day = digits(8..10)?;
    let hour = digits(11..13)?;
    let minute = digits(14..16)?;
    let second = digits(17..19)?;
    let millis = digits(20..23)?;
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(ToolsError::parse("timestamp must be an ISO UTC timestamp"));
    }
    let days = days_from_civil(year, month, day);
    Ok(days * 86_400_000 + hour * 3_600_000 + minute * 60_000 + second * 1000 + millis)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let adjusted = if month <= 2 { year - 1 } else { year };
    let era = adjusted.div_euclid(400);
    let year_of_era = adjusted.rem_euclid(400);
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_renders_canonically() {
        assert_eq!(render_iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(render_iso(1_714_000_000_123), "2024-04-24T23:06:40.123Z");
        assert_eq!(render_iso(-1), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn round_trip() {
        for millis in [0, 1, -1, 86_400_000, 1_714_000_000_123, 253_402_300_799_000] {
            assert_eq!(parse_iso(&render_iso(millis)).unwrap(), millis);
        }
    }

    #[test]
    fn rejects_non_canonical() {
        assert!(parse_iso("2024-03-09T10:26:40Z").is_err());
        assert!(parse_iso("2024-13-09T10:26:40.123Z").is_err());
        assert!(parse_iso("2024-02-30T10:26:40.123Z").is_err());
        assert!(parse_iso("2024-03-09 10:26:40.123Z").is_err());
        assert!(parse_iso("2024-03-09T10:26:40.123+00:00").is_err());
    }
}
