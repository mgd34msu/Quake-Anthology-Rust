//! Host calendar conversion without external dependencies.
//!
//! The donor reads host `Date` fields, including the host time zone. This
//! module converts Unix time with exact civil-date arithmetic and resolves
//! the host zone from the POSIX footer of `/etc/localtime` on unix hosts,
//! falling back to UTC elsewhere.

use std::sync::OnceLock;

const DAY_SECONDS: i64 = 86_400;

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let adjusted = if month <= 2 { year - 1 } else { year };
    let era = if adjusted >= 0 { adjusted } else { adjusted - 399 } / 400;
    let year_of_era = adjusted - era * 400;
    let month_prime = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 { shifted } else { shifted - 146_096 } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 { month_prime + 3 } else { month_prime - 9 };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Sunday-based weekday of `days` since the epoch (1970-01-01 was Thursday).
fn weekday(days: i64) -> i64 {
    (days + 4).rem_euclid(7)
}

fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if is_leap(year) {
                29
            } else {
                28
            }
        }
    }
}

/// `SYSTEMTIME` fields for `millis` plus `offset_seconds` east of UTC:
/// year, month, weekday, day, hour, minute, second, milliseconds.
pub fn system_time_fields(millis: i64, offset_seconds: i64) -> [u16; 8] {
    let total = millis + offset_seconds * 1000;
    let days = total.div_euclid(DAY_SECONDS * 1000);
    let rem = total.rem_euclid(DAY_SECONDS * 1000);
    let (year, month, day) = civil_from_days(days);
    [
        year as u16,
        month as u16,
        weekday(days) as u16,
        day as u16,
        (rem / 3_600_000) as u16,
        ((rem / 60_000) % 60) as u16,
        ((rem / 1000) % 60) as u16,
        (rem % 1000) as u16,
    ]
}

/// `struct tm` fields for `secs` plus `offset_seconds` east of UTC:
/// sec, min, hour, mday, mon, year-1900, wday, yday, isdst.
pub fn tm_fields(secs: i64, offset_seconds: i64, is_dst: bool) -> [i32; 9] {
    let total = secs + offset_seconds;
    let days = total.div_euclid(DAY_SECONDS);
    let rem = total.rem_euclid(DAY_SECONDS);
    let (year, month, day) = civil_from_days(days);
    [
        (rem % 60) as i32,
        ((rem / 60) % 60) as i32,
        (rem / 3600) as i32,
        day as i32,
        (month - 1) as i32,
        (year - 1900) as i32,
        weekday(days) as i32,
        (days - days_from_civil(year, 1, 1)) as i32,
        i32::from(is_dst),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DstRule {
    Month { month: i64, week: i64, day: i64 },
    Julian { day: i64 },
    Ordinal { day: i64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Transition {
    rule: DstRule,
    time_seconds: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PosixZone {
    std_east: i64,
    dst_east: Option<i64>,
    start: Option<Transition>,
    end: Option<Transition>,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(text: &'a str) -> Self {
        Self { bytes: text.as_bytes(), at: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn take(&mut self) -> Option<u8> {
        let value = self.peek()?;
        self.at += 1;
        Some(value)
    }

    fn name(&mut self) -> Option<()> {
        if self.peek() == Some(b'<') {
            self.take();
            while self.take().is_some_and(|c| c != b'>') {}
            return Some(());
        }
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.at += 1;
        }
        if self.at > start { Some(()) } else { None }
    }

    fn number(&mut self) -> Option<i64> {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.at += 1;
        }
        if self.at == start {
            return None;
        }
        std::str::from_utf8(&self.bytes[start..self.at]).ok()?.parse().ok()
    }

    fn offset(&mut self) -> Option<i64> {
        let negative = match self.peek() {
            Some(b'-') => {
                self.take();
                true
            }
            Some(b'+') => {
                self.take();
                false
            }
            _ => false,
        };
        let hours = self.number()?;
        let mut seconds = hours * 3600;
        for scale in [60, 1] {
            if self.peek() != Some(b':') {
                break;
            }
            self.take();
            seconds += self.number().unwrap_or(0) * scale;
        }
        Some(if negative { -seconds } else { seconds })
    }

    fn rule(&mut self) -> Option<Transition> {
        let rule = if self.peek() == Some(b'M') {
            self.take();
            let month = self.number()?;
            if self.take() != Some(b'.') {
                return None;
            }
            let week = self.number()?;
            if self.take() != Some(b'.') {
                return None;
            }
            DstRule::Month { month, week, day: self.number()? }
        } else if self.peek() == Some(b'J') {
            self.take();
            DstRule::Julian { day: self.number()? }
        } else {
            DstRule::Ordinal { day: self.number()? }
        };
        let time_seconds = if self.peek() == Some(b'/') {
            self.take();
            self.offset().unwrap_or(2 * 3600)
        } else {
            2 * 3600
        };
        Some(Transition { rule, time_seconds })
    }
}

fn parse_posix_zone(footer: &str) -> Option<PosixZone> {
    let mut cursor = Cursor::new(footer.trim());
    cursor.name()?;
    // POSIX offsets run west of UTC; negate to seconds east.
    let std_east = -cursor.offset()?;
    if cursor.peek().is_none() || cursor.peek() == Some(b',') {
        return Some(PosixZone { std_east, dst_east: None, start: None, end: None });
    }
    cursor.name()?;
    let dst_east = if cursor.peek().is_some_and(|c| c != b',') {
        -cursor.offset()?
    } else {
        std_east + 3600
    };
    if cursor.take() != Some(b',') {
        return None;
    }
    let start = cursor.rule()?;
    if cursor.take() != Some(b',') {
        return None;
    }
    let end = cursor.rule()?;
    Some(PosixZone { std_east, dst_east: Some(dst_east), start: Some(start), end: Some(end) })
}

fn transition_days(year: i64, transition: &Transition) -> i64 {
    match transition.rule {
        DstRule::Ordinal { day } => days_from_civil(year, 1, 1) + day,
        DstRule::Julian { day } => {
            let mut days = days_from_civil(year, 1, 1) + day - 1;
            if is_leap(year) && day >= 60 {
                days += 1;
            }
            days
        }
        DstRule::Month { month, week, day } => {
            let first = weekday(days_from_civil(year, month, 1));
            let mut date = 1 + (day - first).rem_euclid(7) + (week - 1) * 7;
            if week == 5 && date > days_in_month(year, month) {
                date -= 7;
            }
            days_from_civil(year, month, date)
        }
    }
}

fn zone_at(zone: &PosixZone, unix_secs: i64) -> (i64, bool) {
    let (Some(dst_east), Some(start), Some(end)) = (zone.dst_east, zone.start, zone.end) else {
        return (zone.std_east, false);
    };
    let (year, _, _) = civil_from_days(unix_secs.div_euclid(DAY_SECONDS));
    let start_utc = transition_days(year, &start) * DAY_SECONDS + start.time_seconds - zone.std_east;
    let end_utc = transition_days(year, &end) * DAY_SECONDS + end.time_seconds - dst_east;
    let dst = if start_utc <= end_utc {
        start_utc <= unix_secs && unix_secs < end_utc
    } else {
        unix_secs >= start_utc || unix_secs < end_utc
    };
    (if dst { dst_east } else { zone.std_east }, dst)
}

static HOST_ZONE: OnceLock<Option<PosixZone>> = OnceLock::new();

fn host_zone() -> Option<PosixZone> {
    *HOST_ZONE.get_or_init(|| {
        #[cfg(unix)]
        {
            let bytes = std::fs::read("/etc/localtime").ok()?;
            let text = std::str::from_utf8(&bytes).ok()?;
            let footer = text.trim_end().rsplit('\n').next()?;
            // TZif footers carry exactly one POSIX rule line.
            if footer.contains(',') || footer.bytes().any(|b| b.is_ascii_digit()) {
                parse_posix_zone(footer)
            } else {
                None
            }
        }
        #[cfg(not(unix))]
        {
            None
        }
    })
}

/// Host UTC offset in seconds east of UTC and DST flag at `unix_secs`.
pub fn local_offset_at(unix_secs: i64) -> (i64, bool) {
    host_zone().map(|zone| zone_at(&zone, unix_secs)).unwrap_or((0, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip() {
        for days in [-719_468, -1, 0, 1, 19_000, 20_000, 100_000] {
            let (year, month, day) = civil_from_days(days);
            assert_eq!(days_from_civil(year, month, day), days);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_000), (2022, 1, 8));
    }

    #[test]
    fn system_fields_known_instant() {
        // 2023-11-14T22:13:20.123Z.
        let fields = system_time_fields(1_700_000_000_123, 0);
        assert_eq!(fields, [2023, 11, 14, 2, 22, 13, 20, 123]);
    }

    #[test]
    fn tm_fields_known_instant() {
        let fields = tm_fields(1_700_000_000, 0, false);
        assert_eq!(fields, [20, 13, 22, 14, 10, 123, 2, 317, 0]);
    }

    #[test]
    fn posix_zone_dst_bounds() {
        let zone = parse_posix_zone("EST5EDT,M3.2.0,M11.1.0").expect("parse");
        assert_eq!(zone.std_east, -5 * 3600);
        assert_eq!(zone.dst_east, Some(-4 * 3600));
        // 2023-07-01T00:00:00Z is DST; 2023-01-01T00:00:00Z is standard.
        assert_eq!(zone_at(&zone, 1_688_169_600), (-4 * 3600, true));
        assert_eq!(zone_at(&zone, 1_672_531_200), (-5 * 3600, false));
    }

    #[test]
    fn posix_zone_without_dst() {
        let zone = parse_posix_zone("UTC0").expect("parse");
        assert_eq!(zone_at(&zone, 1_688_169_600), (0, false));
    }
}
