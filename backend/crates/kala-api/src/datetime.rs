//! Timezone aware timestamps, as Pydantic reads and writes them.
//!
//! The Python source stores `AwareDatetime` values, which Pydantic parses with
//! `speedate` and writes as RFC 3339: a `Z` suffix for UTC, six fractional
//! digits when the microseconds are not zero, and the offset of a value that is
//! not in UTC. A value without an offset is refused.
//!
//! The failure vocabulary is the one Pydantic renders, and it was taken from
//! the live Python source:
//!
//! * a value that is not a timestamp is `datetime_type`, "Input should be a
//!   valid datetime";
//! * a timestamp that cannot be read is `datetime_from_date_parsing`,
//!   "Input should be a valid datetime or date, {reason}", with the reason in
//!   the context;
//! * a timestamp without an offset is `timezone_aware`, "Input should have
//!   timezone info".

use kala_json::CanonicalValue;

use crate::validation::{Issue, Location, kind};

/// The reasons `speedate` reports, which Pydantic puts in the context.
pub mod reason {
    /// The input is shorter than a date.
    pub const TOO_SHORT: &str = "input is too short";
    /// The year is not four digits.
    pub const INVALID_YEAR: &str = "invalid character in year";
    /// The month is not two digits.
    pub const INVALID_MONTH: &str = "invalid character in month";
    /// The day is not two digits.
    pub const INVALID_DAY: &str = "invalid character in day";
    /// The date is not separated by hyphens.
    pub const INVALID_SEPARATOR: &str = "invalid date separator, expected `-`";
    /// The month is outside the calendar.
    pub const MONTH_RANGE: &str = "month value is outside expected range of 1-12";
    /// The day is outside the calendar.
    pub const DAY_RANGE: &str = "day value is outside expected range";
    /// The time or the offset does not fit the expected shape.
    pub const EXTRA_CHARACTERS: &str = "unexpected extra characters at the end of the input";
}

/// A microsecond precision, timezone aware timestamp.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// The `model_dump(mode="json")` text of the timestamp.
    #[must_use]
    pub fn to_wire_string(&self) -> String {
        let mut out = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        );
        if self.microsecond != 0 {
            out.push_str(&format!(".{:06}", self.microsecond));
        }
        if self.offset_minutes == 0 {
            out.push('Z');
        } else {
            let sign = if self.offset_minutes < 0 { '-' } else { '+' };
            let magnitude = self.offset_minutes.unsigned_abs();
            out.push_str(&format!(
                "{sign}{:02}:{:02}",
                magnitude / 60,
                magnitude % 60
            ));
        }
        out
    }

    /// Build a timestamp from its calendar fields.
    #[allow(clippy::too_many_arguments)]
    fn new(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
        microsecond: u32,
        offset_minutes: i32,
    ) -> Option<Self> {
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
            return None;
        }
        Some(Self {
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

    /// Build a timestamp from an absolute number of microseconds.
    fn from_absolute_micros(micros: i128) -> Option<Self> {
        let days = micros.div_euclid(86_400_000_000);
        if !(-719_162..=2_932_896).contains(&days) {
            return None;
        }
        let day_micros = micros.rem_euclid(86_400_000_000);
        let day_micros = i64::try_from(day_micros).ok()?;
        let days = i64::try_from(days).ok()?;
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
}

/// Read a timestamp the way Pydantic reads an `AwareDatetime`.
pub fn read_timestamp(
    value: &CanonicalValue,
    location: &[Location],
) -> Result<Timestamp, Box<Issue>> {
    match value {
        CanonicalValue::Text(text) => {
            let text = text.to_utf8().map_err(|_| {
                Issue::new(
                    kind::DATETIME_TYPE,
                    location.to_vec(),
                    "Input should be a valid datetime",
                    value,
                )
            })?;
            parse_timestamp(&text, value, location)
        }
        CanonicalValue::Integer(number) => {
            let seconds = number
                .to_string()
                .parse::<i128>()
                .map_err(|_| parsing_issue(value, location, reason::EXTRA_CHARACTERS))?;
            Timestamp::from_absolute_micros(
                seconds
                    .checked_mul(1_000_000)
                    .ok_or_else(|| parsing_issue(value, location, reason::EXTRA_CHARACTERS))?,
            )
            .ok_or_else(|| parsing_issue(value, location, reason::EXTRA_CHARACTERS))
        }
        CanonicalValue::Float(number) => {
            let seconds = epoch_seconds(*number);
            Timestamp::from_absolute_micros(seconds)
                .ok_or_else(|| parsing_issue(value, location, reason::EXTRA_CHARACTERS))
        }
        _ => Err(Box::new(Issue::new(
            kind::DATETIME_TYPE,
            location.to_vec(),
            "Input should be a valid datetime",
            value,
        ))),
    }
}

/// The failure of a timestamp that cannot be read.
fn parsing_issue(value: &CanonicalValue, location: &[Location], reason: &str) -> Box<Issue> {
    Box::new(Issue::with_context(
        kind::DATETIME_PARSING,
        location.to_vec(),
        format!("Input should be a valid datetime or date, {reason}"),
        value,
        vec![(
            "error".to_owned(),
            crate::validation::Context::Text(reason.to_owned()),
        )],
    ))
}

/// The failure of a timestamp that carries no offset.
fn naive_issue(value: &CanonicalValue, location: &[Location]) -> Box<Issue> {
    Box::new(Issue::new(
        kind::TIMEZONE_AWARE,
        location.to_vec(),
        "Input should have timezone info",
        value,
    ))
}

/// Parse a timestamp from its text.
fn parse_timestamp(
    text: &str,
    value: &CanonicalValue,
    location: &[Location],
) -> Result<Timestamp, Box<Issue>> {
    if let Some(seconds) = epoch_string(text) {
        return Timestamp::from_absolute_micros(seconds)
            .ok_or_else(|| parsing_issue(value, location, reason::EXTRA_CHARACTERS));
    }
    let bytes = text.as_bytes();
    if bytes.len() < 10 {
        return Err(parsing_issue(value, location, reason::TOO_SHORT));
    }
    let year = read_digits(bytes, 0, 4)
        .ok_or_else(|| parsing_issue(value, location, reason::INVALID_YEAR))? as i32;
    if bytes[4] != b'-' {
        return Err(parsing_issue(value, location, reason::INVALID_SEPARATOR));
    }
    let month = read_digits(bytes, 5, 2)
        .ok_or_else(|| parsing_issue(value, location, reason::INVALID_MONTH))?;
    if !(1..=12).contains(&month) {
        return Err(parsing_issue(value, location, reason::MONTH_RANGE));
    }
    if bytes[7] != b'-' {
        return Err(parsing_issue(value, location, reason::INVALID_SEPARATOR));
    }
    let day = read_digits(bytes, 8, 2)
        .ok_or_else(|| parsing_issue(value, location, reason::INVALID_DAY))?;
    if day == 0 || day > days_in_month(year, month) {
        return Err(parsing_issue(value, location, reason::DAY_RANGE));
    }
    if bytes.len() == 10 {
        return Err(naive_issue(value, location));
    }
    if !matches!(bytes[10], b'T' | b't' | b' ') {
        return Err(parsing_issue(value, location, reason::EXTRA_CHARACTERS));
    }
    let (hour, minute, second, microsecond, mut index) = parse_time(bytes, 11, value, location)?;
    if index >= bytes.len() {
        return Err(naive_issue(value, location));
    }
    let offset_minutes = parse_offset(bytes, &mut index)
        .ok_or_else(|| parsing_issue(value, location, reason::EXTRA_CHARACTERS))?;
    if index != bytes.len() {
        return Err(parsing_issue(value, location, reason::EXTRA_CHARACTERS));
    }
    Timestamp::new(
        year,
        month,
        day,
        hour,
        minute,
        second,
        microsecond,
        offset_minutes,
    )
    .ok_or_else(|| parsing_issue(value, location, reason::EXTRA_CHARACTERS))
}

/// Parse the time of a timestamp.
fn parse_time(
    bytes: &[u8],
    start: usize,
    value: &CanonicalValue,
    location: &[Location],
) -> Result<(u32, u32, u32, u32, usize), Box<Issue>> {
    let extra = || parsing_issue(value, location, reason::EXTRA_CHARACTERS);
    if bytes.len() < start + 5 {
        return Err(extra());
    }
    let hour = read_digits(bytes, start, 2).ok_or_else(extra)?;
    if bytes[start + 2] != b':' {
        return Err(extra());
    }
    let minute = read_digits(bytes, start + 3, 2).ok_or_else(extra)?;
    if hour > 23 || minute > 59 {
        return Err(extra());
    }
    let mut index = start + 5;
    let mut second = 0u32;
    let mut microsecond = 0u32;
    if bytes.len() > index && bytes[index] == b':' {
        if bytes.len() < index + 3 {
            return Err(extra());
        }
        second = read_digits(bytes, index + 1, 2).ok_or_else(extra)?;
        if second > 59 {
            return Err(extra());
        }
        index += 3;
        if bytes.len() > index && matches!(bytes[index], b'.' | b',') {
            let fraction_start = index + 1;
            let mut end = fraction_start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end == fraction_start {
                return Err(extra());
            }
            let mut value_of_fraction = 0u32;
            for (index, byte) in bytes[fraction_start..end].iter().take(6).enumerate() {
                value_of_fraction += u32::from(byte - b'0') * 10u32.pow(5 - index as u32);
            }
            microsecond = value_of_fraction;
            index = end;
        }
    }
    Ok((hour, minute, second, microsecond, index))
}

/// Parse the offset of a timestamp.
fn parse_offset(bytes: &[u8], index: &mut usize) -> Option<i32> {
    match bytes.get(*index)? {
        b'Z' | b'z' => {
            *index += 1;
            Some(0)
        }
        sign @ (b'+' | b'-') => {
            let sign = if *sign == b'-' { -1 } else { 1 };
            let rest = &bytes[*index + 1..];
            let (hour, minute) = if rest.len() >= 5 && rest[2] == b':' {
                (read_digits(rest, 0, 2)?, read_digits(rest, 3, 2)?)
            } else if rest.len() >= 4 {
                (read_digits(rest, 0, 2)?, read_digits(rest, 2, 2)?)
            } else {
                return None;
            };
            if hour > 23 || minute > 59 {
                return None;
            }
            *index += if rest.len() >= 5 && rest[2] == b':' {
                6
            } else {
                5
            };
            Some(sign * (hour as i32 * 60 + minute as i32))
        }
        _ => None,
    }
}

/// Read two or four decimal digits.
fn read_digits(bytes: &[u8], start: usize, length: usize) -> Option<u32> {
    if start + length > bytes.len() {
        return None;
    }
    let mut value = 0u32;
    for byte in &bytes[start..start + length] {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value * 10 + u32::from(byte - b'0');
    }
    Some(value)
}

/// The epoch seconds of a string that spells a number.
fn epoch_string(text: &str) -> Option<i128> {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    if matches!(bytes.first(), Some(b'+') | Some(b'-')) {
        index = 1;
    }
    let integer_start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    let integer_digits = index - integer_start;
    if index == bytes.len() {
        if integer_digits == 0 {
            return None;
        }
        return text
            .parse::<i128>()
            .ok()
            .and_then(|seconds| seconds.checked_mul(1_000_000));
    }
    if bytes[index] != b'.' {
        return None;
    }
    index += 1;
    let fraction_start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    let fraction_digits = index - fraction_start;
    if integer_digits + fraction_digits == 0 || index != bytes.len() {
        return None;
    }
    let number: f64 = text.parse().ok()?;
    Some((number * 1_000_000.0).round() as i128)
}

/// The epoch microseconds of a floating point number.
///
/// `speedate` reads a number above `2e10` as milliseconds and a number below
/// it as seconds.
fn epoch_seconds(value: f64) -> i128 {
    if !value.is_finite() {
        return 0;
    }
    let milliseconds = value.abs() > 20_000_000_000.0;
    let scaled = if milliseconds {
        value * 1_000.0
    } else {
        value * 1_000_000.0
    };
    scaled.round() as i128
}

/// Whether a year is a leap year.
fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// The number of days of a month.
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// The calendar date of a number of days since the Unix epoch.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
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

    fn parse(text: &str) -> Result<Timestamp, Box<Issue>> {
        let value = CanonicalValue::from(text);
        read_timestamp(&value, &[])
    }

    #[test]
    fn parses_and_formats_the_pydantic_spellings() {
        for (input, expected) in [
            ("2026-01-02T03:04:05Z", "2026-01-02T03:04:05Z"),
            ("2026-01-02 03:04:05Z", "2026-01-02T03:04:05Z"),
            ("2026-01-02t03:04:05z", "2026-01-02T03:04:05Z"),
            ("2026-01-02T03:04Z", "2026-01-02T03:04:00Z"),
            ("2026-01-02T03:04:05.5Z", "2026-01-02T03:04:05.500000Z"),
            ("2026-01-02T03:04:05,5Z", "2026-01-02T03:04:05.500000Z"),
            ("2026-01-02T03:04:05+05:30", "2026-01-02T03:04:05+05:30"),
            ("2026-01-02T03:04:05-0530", "2026-01-02T03:04:05-05:30"),
            (
                "2026-01-02T03:04:05.123456789Z",
                "2026-01-02T03:04:05.123456Z",
            ),
            ("2026", "1970-01-01T00:33:46Z"),
            ("2", "1970-01-01T00:00:02Z"),
        ] {
            let parsed = parse(input).expect(input);
            assert_eq!(parsed.to_wire_string(), expected, "{input}");
        }
    }

    #[test]
    fn refuses_a_timestamp_without_an_offset() {
        let issue = parse("2026-01-02T03:04:05").unwrap_err();
        assert_eq!(issue.kind, kind::TIMEZONE_AWARE);
        assert_eq!(issue.message, "Input should have timezone info");
        let issue = parse("2026-01-02").unwrap_err();
        assert_eq!(issue.kind, kind::TIMEZONE_AWARE);
    }

    #[test]
    fn reports_the_pydantic_reasons() {
        for (input, expected) in [
            ("not-a-date", reason::INVALID_YEAR),
            ("2026-1-2T03:04:05Z", reason::INVALID_MONTH),
            ("2026/01/02T03:04:05Z", reason::INVALID_SEPARATOR),
            ("2026-13-02T03:04:05Z", reason::MONTH_RANGE),
            ("2026-01-32T03:04:05Z", reason::DAY_RANGE),
            ("2026-01-02T25:04:05Z", reason::EXTRA_CHARACTERS),
            ("2026-01-02T03:04:05Zjunk", reason::EXTRA_CHARACTERS),
            ("2026-0", reason::TOO_SHORT),
        ] {
            let issue = parse(input).unwrap_err();
            assert_eq!(issue.kind, kind::DATETIME_PARSING, "{input}");
            assert_eq!(
                issue.message,
                format!("Input should be a valid datetime or date, {expected}"),
                "{input}"
            );
        }
    }

    #[test]
    fn refuses_a_value_that_is_not_a_timestamp() {
        let value = CanonicalValue::from(true);
        let issue = read_timestamp(&value, &[]).unwrap_err();
        assert_eq!(issue.kind, kind::DATETIME_TYPE);
    }
}
