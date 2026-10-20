//! UTC microsecond timestamps, matching the Python persistence contract.
//!
//! Production writes `datetime.now(UTC).isoformat(timespec="microseconds")`
//! with `+00:00` rewritten to `Z`, and reads through
//! `datetime.fromisoformat(value.replace("Z", "+00:00"))`, requiring an
//! offset. These helpers preserve that output byte-for-byte and accept the
//! native `fromisoformat` calendar/week/basic date forms, arbitrary single
//! character date/time separators, comma or dot fractions, and offsets with
//! seconds/fractions. Python `datetime` has only microsecond precision and
//! cannot represent a leap second, so both are enforced here.

use chrono::{
    DateTime, Datelike, NaiveDate, SecondsFormat, TimeDelta, TimeZone, Timelike, Utc, Weekday,
};

use crate::error::PersistenceError;

/// Format a timestamp the way `Database._timestamp_to_iso` does.
///
/// The value is converted to UTC, nanoseconds are truncated to Python's
/// microsecond precision, and the suffix is `Z`. Python's `datetime` domain
/// (years `1..=9999`) is enforced. A `chrono` leap-second value cannot exist in
/// Python `datetime` and is refused rather than formatted.
pub fn timestamp_to_iso<Tz: TimeZone>(value: &DateTime<Tz>) -> Result<String, PersistenceError> {
    let utc = value.with_timezone(&Utc);
    if utc.timestamp_subsec_nanos() >= 1_000_000_000 {
        return Err(PersistenceError::TimestampLeapSecond);
    }
    let year = utc.year();
    if !(1..=9999).contains(&year) {
        return Err(PersistenceError::TimestampOutOfRange);
    }
    Ok(truncate_to_micros(utc).to_rfc3339_opts(SecondsFormat::Micros, true))
}

/// Parse a stored timestamp the way `Database._timestamp_from_iso` does.
///
/// `Z` is replaced with `+00:00` (all occurrences, as in Python) and the value
/// must carry an offset. The accepted spellings mirror
/// `datetime.fromisoformat`: calendar or ISO-week dates in extended, basic or
/// mixed forms; `HH`, `HH:MM`, `HH:MM:SS` (and basic equivalents); `.` or `,`
/// fractions truncated to microseconds; an arbitrary single-character
/// date/time separator; and `±HH`, `±HHMM`, `±HH:MM`, `±HHMMSS`,
/// `±HH:MM:SS` offsets with optional fractions. Leap seconds and offsets
/// outside the strict `(-24h, 24h)` range are refused.
pub fn timestamp_from_iso(value: &str) -> Result<DateTime<Utc>, PersistenceError> {
    let replaced = value.replace('Z', "+00:00");
    let parsed = parse_isoformat(&replaced).ok_or(PersistenceError::StoredTimestamp)?;
    let mut date = parsed.date;
    let mut hour = parsed.hour;
    if hour == 24 {
        if parsed.minute != 0 || parsed.second != 0 || parsed.microsecond != 0 {
            return Err(PersistenceError::StoredTimestamp);
        }
        date = date.succ_opt().ok_or(PersistenceError::StoredTimestamp)?;
        // Python rolls the calendar day before constructing the datetime, so
        // 9999-12-31T24:00 is rejected even if an offset would map it back
        // into year 9999 in UTC.
        if !(1..=9999).contains(&date.year()) {
            return Err(PersistenceError::StoredTimestamp);
        }
        hour = 0;
    }
    if hour > 23 || parsed.minute > 59 || parsed.second > 59 {
        // Python's datetime constructor rejects 60 seconds, so a stored leap
        // second must never become a typed native timestamp.
        return Err(PersistenceError::StoredTimestamp);
    }
    let naive = date
        .and_hms_micro_opt(hour, parsed.minute, parsed.second, parsed.microsecond)
        .ok_or(PersistenceError::StoredTimestamp)?;
    let utc_naive = naive
        .checked_sub_signed(TimeDelta::microseconds(parsed.offset_microseconds))
        .ok_or(PersistenceError::StoredTimestamp)?;
    let utc = DateTime::<Utc>::from_naive_utc_and_offset(utc_naive, Utc);
    if !(1..=9999).contains(&utc.year()) {
        return Err(PersistenceError::StoredTimestamp);
    }
    Ok(utc)
}

/// Truncate a timestamp to the microseconds a Python `datetime` can hold.
///
/// `datetime` has no nanosecond field, so a native `Utc::now()` must be
/// normalized before it becomes the stored and returned timestamp. A leap
/// second (nanoseconds >= 1,000,000,000) is left intact so
/// [`timestamp_to_iso`] can refuse it instead of silently changing its value.
pub fn truncate_to_micros(value: DateTime<Utc>) -> DateTime<Utc> {
    let nanos = value.timestamp_subsec_nanos();
    if nanos >= 1_000_000_000 {
        return value;
    }
    let remainder = nanos % 1_000;
    if remainder == 0 {
        return value;
    }
    value.with_nanosecond(nanos - remainder).unwrap_or(value)
}

struct ParsedIsoTimestamp {
    date: NaiveDate,
    hour: u32,
    minute: u32,
    second: u32,
    microsecond: u32,
    offset_microseconds: i64,
}

fn parse_isoformat(value: &str) -> Option<ParsedIsoTimestamp> {
    let separator = find_datetime_separator(value)?;
    let date_bytes = value.as_bytes().get(..separator)?;
    if !date_bytes.is_ascii() {
        return None;
    }
    let date_text = std::str::from_utf8(date_bytes).ok()?;
    let date = parse_date(date_text)?;
    if separator == value.len() {
        // A date without a time is naive; the persistence contract requires
        // an explicit offset on every stored timestamp.
        return None;
    }
    let rest = value.get(separator..)?;
    let separator_char = rest.chars().next()?;
    let time_start = separator + separator_char.len_utf8();
    let time_text = value.get(time_start..)?;
    let (hour, minute, second, microsecond, offset_microseconds) =
        parse_time_and_offset(time_text)?;
    Some(ParsedIsoTimestamp {
        date,
        hour,
        minute,
        second,
        microsecond,
        offset_microseconds,
    })
}

/// Python's `_find_isoformat_datetime_separator`, operating on the ASCII date
/// prefix. The separator itself may be any single Unicode character.
fn find_datetime_separator(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    let length = bytes.len();
    if length < 7 {
        return None;
    }
    if length == 7 {
        return Some(7);
    }
    if bytes[4] == b'-' {
        if bytes[5] == b'W' {
            if length < 8 {
                return None;
            }
            if length > 8 && bytes[8] == b'-' {
                if length == 9 {
                    return None;
                }
                if length > 10 && bytes[10].is_ascii_digit() {
                    return Some(8);
                }
                return Some(10);
            }
            return Some(8);
        }
        return Some(10);
    }
    if bytes[4] == b'W' {
        let mut index = 7;
        while index < length && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index < 9 {
            return Some(index);
        }
        return Some(if index % 2 == 0 { 7 } else { 8 });
    }
    Some(8)
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    let bytes = value.as_bytes();
    if !value.is_ascii() || !matches!(bytes.len(), 7 | 8 | 10) {
        return None;
    }
    let year = read_digits(bytes, 0, 4)? as i32;
    if !(1..=9999).contains(&year) {
        return None;
    }
    let has_separator = bytes[4] == b'-';
    let mut position = 4 + usize::from(has_separator);
    if bytes.get(position) == Some(&b'W') {
        position += 1;
        let week = read_digits(bytes, position, 2)?;
        position += 2;
        let mut day = 1;
        if bytes.len() > position {
            if (bytes[position] == b'-') != has_separator {
                return None;
            }
            position += usize::from(has_separator);
            day = read_digits(bytes, position, 1)?;
        }
        if !(1..=53).contains(&week) || !(1..=7).contains(&day) {
            return None;
        }
        let weekday = match day {
            1 => Weekday::Mon,
            2 => Weekday::Tue,
            3 => Weekday::Wed,
            4 => Weekday::Thu,
            5 => Weekday::Fri,
            6 => Weekday::Sat,
            7 => Weekday::Sun,
            _ => return None,
        };
        return NaiveDate::from_isoywd_opt(year, week, weekday);
    }
    let month = read_digits(bytes, position, 2)?;
    position += 2;
    if (bytes.get(position) == Some(&b'-')) != has_separator {
        return None;
    }
    position += usize::from(has_separator);
    let day = read_digits(bytes, position, 2)?;
    NaiveDate::from_ymd_opt(year, month, day)
}

fn parse_time_and_offset(value: &str) -> Option<(u32, u32, u32, u32, i64)> {
    if !value.is_ascii() {
        return None;
    }
    let offset_start = value.find(['+', '-']);
    let (time_text, offset_text) = match offset_start {
        Some(index) => (value.get(..index)?, Some(value.get(index..)?)),
        None => (value, None),
    };
    let (hour, minute, second, microsecond) = parse_time_components(time_text)?;
    let offset_microseconds = parse_offset(offset_text?)?;
    Some((hour, minute, second, microsecond, offset_microseconds))
}

/// Python's C `parse_hh_mm_ss_ff` for `HH[:?MM[:?SS[{.,}fff]]]` plus basic
/// equivalents, returning `(hour, minute, second, microsecond, trailing)`.
///
/// `trailing` is Python's non-negative return value: `1` means the parser
/// stopped before the end of its slice. The time-of-day caller accepts either
/// (Python ignores a trailing value when an offset follows), while an offset
/// requires `0`.
fn parse_hh_mm_ss_ff(value: &str) -> Option<(u32, u32, u32, u32, u8)> {
    let bytes = value.as_bytes();
    if !value.is_ascii() {
        return None;
    }
    let length = bytes.len();
    let mut values = [0_u32; 3];
    let mut position = 0_usize;
    let mut has_separator = true;
    let mut fraction_start = None;

    for index in 0..3 {
        let digits = bytes.get(position..position + 2)?;
        if !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        values[index] = u32::from(digits[0] - b'0') * 10 + u32::from(digits[1] - b'0');
        position += 2;
        let character = if position < length {
            let character = bytes[position];
            position += 1;
            character
        } else {
            position += 1;
            0
        };
        if index == 0 {
            has_separator = character == b':';
        }
        if matches!(character, b'.' | b',') {
            if index < 2 || position >= length {
                return None;
            }
            fraction_start = Some(position);
            break;
        } else if position >= length {
            return Some((values[0], values[1], values[2], 0, u8::from(character != 0)));
        } else if has_separator && character == b':' {
            if index == 2 {
                return None;
            }
        } else if !has_separator {
            position -= 1;
        } else {
            return None;
        }
    }

    let start = fraction_start.unwrap_or(position);
    let remaining = length.saturating_sub(start);
    let take = remaining.min(6);
    if take == 0 {
        return None;
    }
    let mut microsecond = 0_u32;
    for digit in &bytes[start..start + take] {
        if !digit.is_ascii_digit() {
            return None;
        }
        microsecond = microsecond * 10 + u32::from(digit - b'0');
    }
    for _ in take..6 {
        microsecond *= 10;
    }
    position = start + take;
    while position < length && bytes[position].is_ascii_digit() {
        position += 1;
    }
    Some((
        values[0],
        values[1],
        values[2],
        microsecond,
        u8::from(position != length),
    ))
}

/// Parse the time of day, where Python ignores a trailing non-zero parser
/// status as long as an offset follows.
fn parse_time_components(value: &str) -> Option<(u32, u32, u32, u32)> {
    let (hour, minute, second, microsecond, _trailing) = parse_hh_mm_ss_ff(value)?;
    Some((hour, minute, second, microsecond))
}

fn parse_offset(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    if !value.is_ascii() {
        return None;
    }
    let sign = match bytes.first()? {
        b'+' => 1_i64,
        b'-' => -1_i64,
        _ => return None,
    };
    let (hours, minutes, seconds, micros, trailing) = parse_hh_mm_ss_ff(&value[1..])?;
    if trailing != 0 {
        return None;
    }
    let total = (i64::from(hours) * 60 + i64::from(minutes)) * 60 + i64::from(seconds);
    let total = total * 1_000_000 + i64::from(micros);
    let total = sign * total;
    // Python's timezone constructor accepts normalized minute/second values
    // but requires the resulting offset to be strictly within +/- 24 hours.
    if total <= -86_400_000_000 || total >= 86_400_000_000 {
        return None;
    }
    Some(total)
}

fn read_digits(bytes: &[u8], start: usize, count: usize) -> Option<u32> {
    let slice = bytes.get(start..start + count)?;
    let mut value = 0_u32;
    for digit in slice {
        if !digit.is_ascii_digit() {
            return None;
        }
        value = value * 10 + u32::from(digit - b'0');
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_python_microseconds() {
        let value: DateTime<Utc> = DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
            .unwrap()
            .into();
        assert_eq!(
            timestamp_to_iso(&value).unwrap(),
            "2026-01-01T00:00:00.000000Z"
        );
    }

    #[test]
    fn parses_rfc3339_and_iso_offset_spellings() {
        for text in [
            "2026-01-01T00:00:00.000000Z",
            "2026-01-01T00:00:00.000000+00:00",
            "2026-01-01 00:00:00+0000",
            "2026-01-01T00:00+00:00",
            "2026-W01-4T00:00:00+00:00",
            "2026-01-01X00:00:00+00:00",
            "2026-01-01T00:00:00+05:30:15",
            "2026-01-01T00:00:00+05:30:15.123456",
            "2026-01-01T00:00:00,123456Z",
        ] {
            assert!(timestamp_from_iso(text).is_ok(), "rejected {text}");
        }
        assert!(timestamp_from_iso("2026-01-01T00:00:00").is_err());
        assert!(timestamp_from_iso("2026-01-01T00:00:60Z").is_err());
        assert!(timestamp_from_iso("not-a-timestamp").is_err());
    }

    #[test]
    fn truncates_nanoseconds_to_python_precision() {
        let value = DateTime::parse_from_rfc3339("2026-01-01T00:00:00.123456789+00:00")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            timestamp_to_iso(&value).unwrap(),
            "2026-01-01T00:00:00.123456Z"
        );
        let parsed = timestamp_from_iso("2026-01-01T00:00:00.123456789Z").unwrap();
        assert_eq!(parsed.timestamp_subsec_nanos(), 123_456_000);
        assert_eq!(
            timestamp_to_iso(&parsed).unwrap(),
            "2026-01-01T00:00:00.123456Z"
        );
    }

    #[test]
    fn converts_offset_timestamps_to_utc_z() {
        let offset = DateTime::parse_from_rfc3339("2026-01-01T05:30:00+05:30").unwrap();
        assert_eq!(
            timestamp_to_iso(&offset).unwrap(),
            "2026-01-01T00:00:00.000000Z"
        );
    }

    #[test]
    fn rejects_leap_second_values() {
        let leap = DateTime::parse_from_rfc3339("2026-01-01T00:00:60+00:00").unwrap();
        assert!(timestamp_to_iso(&leap).is_err());
        assert!(timestamp_from_iso("2026-01-01T00:00:60Z").is_err());
    }

    #[test]
    fn accepts_24_hour_rollover_and_offset_seconds() {
        assert_eq!(
            timestamp_to_iso(&timestamp_from_iso("2026-01-01T24:00:00Z").unwrap()).unwrap(),
            "2026-01-02T00:00:00.000000Z"
        );
        assert_eq!(
            timestamp_to_iso(&timestamp_from_iso("2026-01-01T00:00:00+05:30:15.123456").unwrap())
                .unwrap(),
            "2025-12-31T18:29:44.876544Z"
        );
    }
}
