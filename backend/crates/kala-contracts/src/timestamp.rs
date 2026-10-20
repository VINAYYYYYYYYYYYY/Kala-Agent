//! Pydantic `AwareDatetime`-compatible timestamps.
//!
//! The Python contract stores timezone-aware `datetime` values. Pydantic
//! serializes them as RFC 3339 with a `Z` suffix for UTC, always six fractional
//! digits when microseconds are nonzero, and preserves a non-UTC offset. Its
//! parser accepts calendar dates with `T`, `t` or a space separator, optional
//! seconds, a dot or comma fraction truncated to microseconds, and an offset
//! written as `±HH:MM` or `±HHMM`. Numeric JSON values and all-digit strings
//! are Unix epoch seconds.
//!
//! This type stores local calendar fields plus the offset, so the wire bytes
//! match the Python value, and compares as an absolute instant the way aware
//! `datetime` values do.

use std::cmp::Ordering;
use std::fmt;

use kala_json::CanonicalValue;
use num_bigint::BigInt;
use num_traits::ToPrimitive;

use crate::error::ContractError;

/// A microsecond-precision, timezone-aware timestamp.
#[derive(Clone, Debug)]
pub struct Timestamp {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    microsecond: u32,
    offset_minutes: i32,
}

impl Timestamp {
    /// Build and validate a timestamp from calendar fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
        microsecond: u32,
        offset_minutes: i32,
    ) -> Result<Self, ContractError> {
        if !(1..=9999).contains(&year)
            || !(1..=12).contains(&month)
            || day == 0
            || day > days_in_month(year, month)
            || hour > 23
            || minute > 59
            || second > 59
            || microsecond > 999_999
            || !(-1439..=1439).contains(&offset_minutes)
        {
            return Err(invalid());
        }
        Ok(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            microsecond,
            offset_minutes,
        })
    }

    /// Parse an aware timestamp from contract JSON.
    pub fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        match value {
            CanonicalValue::Text(text) => {
                let utf8 = text.to_utf8().map_err(|_| invalid())?;
                Self::parse(&utf8)
            }
            CanonicalValue::Integer(number) => Self::from_epoch_integer(number),
            CanonicalValue::Float(number) => Self::from_epoch_float(*number),
            _ => Err(invalid()),
        }
    }

    /// Parse an aware ISO-8601/RFC-3339 timestamp or an epoch-seconds string.
    pub fn parse(value: &str) -> Result<Self, ContractError> {
        if value.is_empty() {
            return Err(invalid());
        }
        if is_epoch_string(value) {
            return parse_epoch_string(value);
        }
        let bytes = value.as_bytes();
        if bytes.len() < 16 {
            return Err(if bytes.len() < 10 { invalid() } else { naive() });
        }
        let Some((year, month, day)) = parse_date_prefix(bytes) else {
            return Err(invalid());
        };
        let separator = bytes[10];
        if separator != b'T' && separator != b't' && separator != b' ' && separator != b'_' {
            return Err(invalid());
        }
        let rest = &value[11..];
        // speedate also accepts U+2212 MINUS SIGN in an offset.
        let offset_index = rest
            .char_indices()
            .find(|(_, character)| matches!(character, '+' | '-' | 'Z' | 'z' | '\u{2212}'))
            .map(|(index, _)| index);
        let Some(offset_index) = offset_index else {
            return Err(naive());
        };
        let (time, offset) = rest.split_at(offset_index);
        let (hour, minute, second, microsecond) = parse_time(time)?;
        let offset_minutes = parse_offset(offset)?;
        Self::new(
            year,
            month,
            day,
            hour,
            minute,
            second,
            microsecond,
            offset_minutes,
        )
    }

    /// Build from Unix epoch seconds (UTC).
    pub fn from_unix_seconds(seconds: i64) -> Result<Self, ContractError> {
        Self::from_absolute_micros(i128::from(seconds) * 1_000_000)
    }

    /// Build from Unix epoch microseconds (UTC).
    pub fn from_absolute_micros(micros: i128) -> Result<Self, ContractError> {
        let days = micros.div_euclid(86_400_000_000);
        if !(-719_162..=2_932_896).contains(&days) {
            return Err(invalid());
        }
        let day_micros = micros.rem_euclid(86_400_000_000);
        let day_micros = i64::try_from(day_micros).map_err(|_| invalid())?;
        let days = i64::try_from(days).map_err(|_| invalid())?;
        let (year, month, day) = civil_from_days(days);
        let second_of_day = day_micros / 1_000_000;
        let microsecond = (day_micros % 1_000_000) as u32;
        Self::new(
            year,
            month,
            day,
            (second_of_day / 3600) as u32,
            ((second_of_day % 3600) / 60) as u32,
            (second_of_day % 60) as u32,
            microsecond,
            0,
        )
    }

    /// Calendar year (`1..=9999`).
    pub fn year(&self) -> i32 {
        self.year
    }
    /// Calendar month (`1..=12`).
    pub fn month(&self) -> u32 {
        self.month
    }
    /// Calendar day.
    pub fn day(&self) -> u32 {
        self.day
    }
    /// Hour (`0..=23`).
    pub fn hour(&self) -> u32 {
        self.hour
    }
    /// Minute.
    pub fn minute(&self) -> u32 {
        self.minute
    }
    /// Second.
    pub fn second(&self) -> u32 {
        self.second
    }
    /// Microsecond.
    pub fn microsecond(&self) -> u32 {
        self.microsecond
    }
    /// UTC offset in minutes (`-1439..=1439`).
    pub fn offset_minutes(&self) -> i32 {
        self.offset_minutes
    }

    /// Absolute instant expressed as microseconds since the Unix epoch.
    pub fn absolute_micros(&self) -> i128 {
        let days = i128::from(days_from_civil(self.year, self.month, self.day));
        let seconds = days * 86_400
            + i128::from(self.hour) * 3600
            + i128::from(self.minute) * 60
            + i128::from(self.second);
        seconds * 1_000_000 + i128::from(self.microsecond)
            - i128::from(self.offset_minutes) * 60 * 1_000_000
    }

    /// The Pydantic `model_dump(mode="json")` text.
    pub fn to_wire_string(&self) -> String {
        format!("{self}")
    }

    fn from_epoch_integer(number: &BigInt) -> Result<Self, ContractError> {
        let count = number.to_i128().ok_or_else(invalid)?;
        let milliseconds = count.unsigned_abs() > 20_000_000_000u128;
        let micros = if milliseconds {
            count.checked_mul(1_000).ok_or_else(invalid)?
        } else {
            count.checked_mul(1_000_000).ok_or_else(invalid)?
        };
        Self::from_absolute_micros(micros)
    }

    fn from_epoch_float(value: f64) -> Result<Self, ContractError> {
        if !value.is_finite() {
            return Err(invalid());
        }
        // speedate (used by pydantic-core) infers milliseconds above 2e10.
        // Its float conversion is intentionally reproduced here: the
        // microsecond part is the rounded absolute fractional part of the
        // truncation toward zero, then added to the floor second. That is why
        // negative sub-second values land one second below zero in Pydantic.
        let milliseconds = value.abs() > 20_000_000_000.0;
        let floor = value.floor();
        let truncated = value.trunc();
        if floor < i128::MIN as f64 || floor > i128::MAX as f64 {
            return Err(invalid());
        }
        let fraction = value - truncated;
        let micros = if milliseconds {
            (fraction * 1_000.0).round().abs()
        } else {
            (fraction * 1_000_000.0).round().abs()
        };
        let count = floor as i128;
        let total = if milliseconds {
            count
                .checked_mul(1_000)
                .and_then(|value| value.checked_add(micros as i128))
        } else {
            count
                .checked_mul(1_000_000)
                .and_then(|value| value.checked_add(micros as i128))
        }
        .ok_or_else(invalid)?;
        Self::from_absolute_micros(total)
    }
}

impl PartialEq for Timestamp {
    fn eq(&self, other: &Self) -> bool {
        self.absolute_micros() == other.absolute_micros()
    }
}

impl Eq for Timestamp {}

impl PartialOrd for Timestamp {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Timestamp {
    fn cmp(&self, other: &Self) -> Ordering {
        self.absolute_micros().cmp(&other.absolute_micros())
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )?;
        if self.microsecond != 0 {
            write!(formatter, ".{:06}", self.microsecond)?;
        }
        if self.offset_minutes == 0 {
            formatter.write_str("Z")
        } else {
            let sign = if self.offset_minutes < 0 { '-' } else { '+' };
            let magnitude = self.offset_minutes.unsigned_abs();
            write!(
                formatter,
                "{sign}{:02}:{:02}",
                magnitude / 60,
                magnitude % 60
            )
        }
    }
}

fn invalid() -> ContractError {
    ContractError::new("Timestamp", "Input should be a valid datetime")
}

fn naive() -> ContractError {
    ContractError::new("Timestamp", "Input should have timezone info")
}

fn is_epoch_string(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    if matches!(bytes.first(), Some(b'+') | Some(b'-')) {
        index = 1;
    }
    let integer_start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    let integer_digits = index - integer_start;
    if index == bytes.len() {
        return integer_digits > 0;
    }
    if bytes[index] != b'.' {
        return false;
    }
    index += 1;
    let fraction_start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    let fraction_digits = index - fraction_start;
    integer_digits + fraction_digits > 0 && index == bytes.len()
}

fn parse_epoch_string(value: &str) -> Result<Timestamp, ContractError> {
    if let Ok(integer) = value.parse::<i128>() {
        return Timestamp::from_absolute_micros(
            integer.checked_mul(1_000_000).ok_or_else(invalid)?,
        );
    }
    let float: f64 = value.parse().map_err(|_| invalid())?;
    Timestamp::from_epoch_float(float)
}

fn parse_date_prefix(bytes: &[u8]) -> Option<(i32, u32, u32)> {
    if bytes.len() < 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes[..4].iter().all(u8::is_ascii_digit)
        || !bytes[5..7].iter().all(u8::is_ascii_digit)
        || !bytes[8..10].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let year = std::str::from_utf8(&bytes[..4]).ok()?.parse().ok()?;
    let month = std::str::from_utf8(&bytes[5..7]).ok()?.parse().ok()?;
    let day = std::str::from_utf8(&bytes[8..10]).ok()?.parse().ok()?;
    if !(1..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
    {
        return None;
    }
    Some((year, month, day))
}

fn parse_time(value: &str) -> Result<(u32, u32, u32, u32), ContractError> {
    let bytes = value.as_bytes();
    if bytes.len() < 5 {
        return Err(invalid());
    }
    let hour = read_two(bytes, 0)?;
    if bytes[2] != b':' {
        return Err(invalid());
    }
    let minute = read_two(bytes, 3)?;
    let mut second = 0;
    let mut microsecond = 0;
    if bytes.len() > 5 {
        if bytes[5] != b':' {
            return Err(invalid());
        }
        if bytes.len() < 8 {
            return Err(invalid());
        }
        second = read_two(bytes, 6)?;
        if second > 59 {
            return Err(invalid());
        }
        if bytes.len() > 8 {
            let fraction_marker = bytes[8];
            if fraction_marker != b'.' && fraction_marker != b',' {
                return Err(invalid());
            }
            let fraction = &bytes[9..];
            if fraction.is_empty() || !fraction.iter().all(u8::is_ascii_digit) {
                return Err(invalid());
            }
            let mut value = 0u32;
            for (index, byte) in fraction.iter().take(6).enumerate() {
                value += u32::from(byte - b'0') * 10u32.pow(5 - index as u32);
            }
            microsecond = value;
        }
    }
    if hour > 23 || minute > 59 {
        return Err(invalid());
    }
    Ok((hour, minute, second, microsecond))
}

fn parse_offset(value: &str) -> Result<i32, ContractError> {
    if value == "Z" || value == "z" {
        return Ok(0);
    }
    let value = value
        .strip_prefix('\u{2212}')
        .map_or_else(|| value.to_owned(), |rest| format!("-{rest}"));
    let bytes = value.as_bytes();
    let sign = match bytes.first() {
        Some(b'+') => 1,
        Some(b'-') => -1,
        _ => return Err(invalid()),
    };
    let digits = &value[1..];
    let (hour, minute) = match digits.len() {
        5 if digits.as_bytes()[2] == b':' => (
            read_two(digits.as_bytes(), 0)?,
            read_two(digits.as_bytes(), 3)?,
        ),
        4 => (
            read_two(digits.as_bytes(), 0)?,
            read_two(digits.as_bytes(), 2)?,
        ),
        _ => return Err(invalid()),
    };
    if hour > 23 || minute > 59 {
        return Err(invalid());
    }
    Ok(sign * (hour as i32 * 60 + minute as i32))
}

fn read_two(bytes: &[u8], start: usize) -> Result<u32, ContractError> {
    if start + 2 > bytes.len() || !bytes[start..start + 2].iter().all(u8::is_ascii_digit) {
        return Err(invalid());
    }
    Ok(u32::from(bytes[start] - b'0') * 10 + u32::from(bytes[start + 1] - b'0'))
}

pub(crate) fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub(crate) fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
pub(crate) fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let mut year = i64::from(year);
    let month = i64::from(month);
    let day = i64::from(day);
    if month <= 2 {
        year -= 1;
    }
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Inverse of [`days_from_civil`].
pub(crate) fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
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
    (year as i32, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_pydantic_spellings() {
        let cases = [
            ("2026-01-02T03:04:05Z", "2026-01-02T03:04:05Z"),
            ("2026-01-02 03:04:05+0530", "2026-01-02T03:04:05+05:30"),
            (
                "2026-01-02t03:04:05.123456789z",
                "2026-01-02T03:04:05.123456Z",
            ),
            ("2026-01-02T03:04Z", "2026-01-02T03:04:00Z"),
            ("001767323045", "2026-01-02T03:04:05Z"),
            ("+2026", "1970-01-01T00:33:46Z"),
            ("-1.5", "1969-12-31T23:59:58.500000Z"),
        ];
        for (input, expected) in cases {
            let parsed = Timestamp::parse(input).expect(input);
            assert_eq!(parsed.to_wire_string(), expected, "{input}");
        }
    }

    #[test]
    fn compares_absolute_instants() {
        let utc = Timestamp::parse("2026-01-02T03:04:05Z").unwrap();
        let plus = Timestamp::parse("2026-01-02T08:34:05+05:30").unwrap();
        assert_eq!(utc, plus);
        assert!(Timestamp::parse("2026-01-02T03:04:05.000001Z").unwrap() > utc);
    }

    #[test]
    fn rejects_naive_and_out_of_range_values() {
        assert_eq!(
            Timestamp::parse("2026-01-02T03:04:05")
                .unwrap_err()
                .message(),
            "Input should have timezone info"
        );
        assert!(Timestamp::parse("2026-02-29T03:04:05Z").is_err());
        assert!(Timestamp::parse("2024-02-29T03:04:05Z").is_ok());
        assert!(Timestamp::parse("2026-01-02T24:00:00Z").is_err());
        assert!(Timestamp::parse("2026-01-02T03:04:05+24:00").is_err());
        assert!(Timestamp::parse("10000-01-01T00:00:00Z").is_err());
    }
}
