//! Timezone-aware, microsecond `datetime` values with CPython parsing rules.
//!
//! The Python kernel constructs aware `datetime` values and serializes them
//! with `datetime.isoformat()`. Persisted policy/workflow state is read back
//! with `datetime.fromisoformat`, which accepts more spellings than RFC 3339:
//! calendar, basic and ISO-week dates; an arbitrary single-character date/time
//! separator; `HH`, `HH:MM`, `HH:MM:SS` and basic time forms; comma or dot
//! fractions (truncated to microseconds); and offsets with optional seconds
//! and fractions. `Z` is accepted for the policy loader (Python 3.11+) and is
//! rewritten by the workflow loader before parsing.
//!
//! Values are stored as an absolute instant in microseconds. Python
//! `datetime` has no finer precision, and every constructor path in the kernel
//! normalizes to UTC before storing, so UTC is the only retained presentation.

use std::fmt;

use chrono::{Datelike, NaiveDate, Utc, Weekday};

/// A Python `datetime` with an aware timezone, normalized to UTC.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct KernelTimestamp {
    micros: i128,
}

/// Why an ISO timestamp could not become the aware value the kernel requires.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IsoformatError {
    /// The text is not an accepted `datetime.fromisoformat` spelling.
    Invalid,
    /// The text parsed, but carries no UTC offset.
    Naive,
}

impl KernelTimestamp {
    /// Build from Unix epoch microseconds.
    pub fn from_unix_micros(micros: i128) -> Result<Self, IsoformatError> {
        let candidate = Self { micros };
        if candidate.in_python_range() {
            Ok(candidate)
        } else {
            Err(IsoformatError::Invalid)
        }
    }

    /// Build from Unix epoch seconds.
    pub fn from_unix_seconds(seconds: i64) -> Result<Self, IsoformatError> {
        Self::from_unix_micros(i128::from(seconds) * 1_000_000)
    }

    /// Build from civil fields and an offset in seconds east of UTC.
    #[allow(clippy::too_many_arguments)]
    pub fn from_civil(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
        microsecond: u32,
        offset_seconds: i64,
    ) -> Result<Self, IsoformatError> {
        if !(1..=9999).contains(&year)
            || !(1..=12).contains(&month)
            || hour > 23
            || minute > 59
            || second > 59
            || microsecond > 999_999
            || !(-86_399..=86_399).contains(&offset_seconds)
        {
            return Err(IsoformatError::Invalid);
        }
        let date = NaiveDate::from_ymd_opt(year, month, day).ok_or(IsoformatError::Invalid)?;
        let naive_micros = civil_to_micros(date, hour, minute, second, microsecond);
        Self::from_unix_micros(naive_micros - i128::from(offset_seconds) * 1_000_000)
    }

    /// The current UTC time, truncated to Python's microsecond precision.
    pub fn now() -> Self {
        let now = Utc::now();
        let micros =
            i128::from(now.timestamp()) * 1_000_000 + i128::from(now.timestamp_subsec_micros());
        Self { micros }
    }

    /// Absolute instant in microseconds since the Unix epoch.
    pub const fn unix_micros(self) -> i128 {
        self.micros
    }

    /// Python `datetime.isoformat()` after normalization to UTC.
    ///
    /// The offset is always `+00:00`; the fraction is present only when it is
    /// nonzero and always has six digits, as in CPython.
    pub fn isoformat(self) -> String {
        let (date, hour, minute, second, microsecond) = self.civil();
        let mut out = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            date.year(),
            date.month(),
            date.day(),
            hour,
            minute,
            second
        );
        if microsecond != 0 {
            out.push_str(&format!(".{microsecond:06}"));
        }
        out.push_str("+00:00");
        out
    }

    /// Parse with policy-state semantics: `datetime.fromisoformat(text)`.
    pub fn parse_isoformat(value: &str) -> Result<Self, IsoformatError> {
        parse_isoformat(value, false)
    }

    /// Parse with workflow-state semantics:
    /// `datetime.fromisoformat(text.replace("Z", "+00:00"))`.
    pub fn parse_isoformat_z_replaced(value: &str) -> Result<Self, IsoformatError> {
        parse_isoformat(value, true)
    }

    /// The UTC calendar fields `(year, month, day, hour, minute, second,
    /// microsecond)`.
    pub fn utc_fields(self) -> (i32, u32, u32, u32, u32, u32, u32) {
        let (date, hour, minute, second, microsecond) = self.civil();
        (
            date.year(),
            date.month(),
            date.day(),
            hour,
            minute,
            second,
            microsecond,
        )
    }

    fn civil(self) -> (NaiveDate, u32, u32, u32, u32) {
        // The constructor and parser keep the value inside the Python year
        // range, so the chrono conversions below cannot fail for a constructed
        // value. A saturating fallback keeps Display total for any future
        // trusted constructor.
        let micros_per_day = 86_400_000_000_i128;
        let days = self.micros.div_euclid(micros_per_day);
        let day_micros = self.micros.rem_euclid(micros_per_day);
        let days = i64::try_from(days).unwrap_or(i64::MAX);
        let seconds = i64::try_from(day_micros / 1_000_000).unwrap_or(0);
        let microsecond = u32::try_from(day_micros % 1_000_000).unwrap_or(0);
        let date = NaiveDate::from_num_days_from_ce_opt(719_163 + days as i32)
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(1970, 1, 1).expect("epoch date"));
        (
            date,
            (seconds / 3600) as u32,
            ((seconds % 3600) / 60) as u32,
            (seconds % 60) as u32,
            microsecond,
        )
    }

    fn in_python_range(self) -> bool {
        let micros_per_day = 86_400_000_000_i128;
        let days = self.micros.div_euclid(micros_per_day);
        // 0001-01-01 through 9999-12-31 in days since the Unix epoch.
        (-719_162..=2_932_896).contains(&days)
    }
}

impl fmt::Display for KernelTimestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.isoformat())
    }
}

fn civil_to_micros(date: NaiveDate, hour: u32, minute: u32, second: u32, microsecond: u32) -> i128 {
    let days = i128::from(date.num_days_from_ce()) - 719_163;
    (days * 86_400 + i128::from(hour) * 3600 + i128::from(minute) * 60 + i128::from(second))
        * 1_000_000
        + i128::from(microsecond)
}

fn parse_isoformat(value: &str, replace_z: bool) -> Result<KernelTimestamp, IsoformatError> {
    let owned;
    let value = if replace_z {
        owned = value.replace('Z', "+00:00");
        owned.as_str()
    } else {
        value
    };
    if value.is_empty() {
        return Err(IsoformatError::Invalid);
    }
    let separator = find_datetime_separator(value).ok_or(IsoformatError::Invalid)?;
    let date_bytes = value
        .as_bytes()
        .get(..separator)
        .ok_or(IsoformatError::Invalid)?;
    if !date_bytes.is_ascii() {
        return Err(IsoformatError::Invalid);
    }
    let date_text = std::str::from_utf8(date_bytes).map_err(|_| IsoformatError::Invalid)?;
    let date = parse_date(date_text).ok_or(IsoformatError::Invalid)?;
    if separator == value.len() {
        // A date-only value is a valid naive datetime in CPython; the kernel
        // requires an explicit offset on every stored timestamp.
        return Err(IsoformatError::Naive);
    }
    let rest = value.get(separator..).ok_or(IsoformatError::Invalid)?;
    let separator_char = rest.chars().next().ok_or(IsoformatError::Invalid)?;
    let time_start = separator + separator_char.len_utf8();
    let time_text = value.get(time_start..).ok_or(IsoformatError::Invalid)?;
    let (hour, minute, second, microsecond, offset) =
        parse_time_and_offset(time_text).ok_or(IsoformatError::Invalid)?;
    let Some(offset_microseconds) = offset else {
        return Err(IsoformatError::Naive);
    };
    let mut date = date;
    let mut hour = hour;
    if hour == 24 {
        if minute != 0 || second != 0 || microsecond != 0 {
            return Err(IsoformatError::Invalid);
        }
        date = date.succ_opt().ok_or(IsoformatError::Invalid)?;
        if !(1..=9999).contains(&date.year()) {
            return Err(IsoformatError::Invalid);
        }
        hour = 0;
    }
    if hour > 23 || minute > 59 || second > 59 {
        return Err(IsoformatError::Invalid);
    }
    let _ = date.year();
    let micros =
        civil_to_micros(date, hour, minute, second, microsecond) - i128::from(offset_microseconds);
    let candidate = KernelTimestamp { micros };
    if candidate.in_python_range() {
        Ok(candidate)
    } else {
        Err(IsoformatError::Invalid)
    }
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

/// Parse the time-of-day and an optional offset.
///
/// The result is `(hour, minute, second, microsecond, offset_microseconds)`.
/// `None` offset means the text was a valid naive time.
fn parse_time_and_offset(value: &str) -> Option<(u32, u32, u32, u32, Option<i64>)> {
    if !value.is_ascii() {
        return None;
    }
    // A trailing uppercase `Z` is the only non-numeric offset spelling.
    let (time_text, offset_text) = if let Some(stripped) = value.strip_suffix('Z') {
        (stripped, Some("+00:00"))
    } else {
        let offset_start = value.find(['+', '-']);
        match offset_start {
            Some(index) => (value.get(..index)?, Some(value.get(index..)?)),
            None => (value, None),
        }
    };
    let (hour, minute, second, microsecond, trailing) = parse_hh_mm_ss_ff(time_text)?;
    let offset_microseconds = match offset_text {
        Some(offset_text) => {
            let (hours, minutes, seconds, micros, offset_trailing) =
                parse_hh_mm_ss_ff(offset_text.get(1..)?)?;
            if offset_trailing != 0 {
                return None;
            }
            let sign = match offset_text.as_bytes().first()? {
                b'+' => 1_i64,
                b'-' => -1_i64,
                _ => return None,
            };
            let total = (i64::from(hours) * 60 + i64::from(minutes)) * 60 + i64::from(seconds);
            let total = total * 1_000_000 + i64::from(micros);
            let total = sign * total;
            if total <= -86_400_000_000 || total >= 86_400_000_000 {
                return None;
            }
            Some(total)
        }
        None => {
            // Without a following offset Python requires the time parser to
            // consume the whole slice.
            if trailing != 0 {
                return None;
            }
            None
        }
    };
    Some((hour, minute, second, microsecond, offset_microseconds))
}

/// Python's C `parse_hh_mm_ss_ff` for `HH[:?MM[:?SS[{.,}fff]]]` plus basic
/// equivalents, returning `(hour, minute, second, microsecond, trailing)`.
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

#[doc(hidden)]
pub fn _utc_date_from_micros(micros: i128) -> NaiveDate {
    KernelTimestamp { micros }.civil().0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_python_spellings_and_normalizes_to_utc() {
        let cases = [
            ("2026-01-02T03:04:05Z", "2026-01-02T03:04:05+00:00"),
            ("2026-01-02T03:04:05+00:00", "2026-01-02T03:04:05+00:00"),
            ("2026-01-02 03:04:05+0530", "2026-01-01T21:34:05+00:00"),
            (
                "2026-01-02t03:04:05.123456789Z",
                "2026-01-02T03:04:05.123456+00:00",
            ),
            ("2026-01-02T03:04Z", "2026-01-02T03:04:00+00:00"),
            ("2026-W01-4T00:00:00+00:00", "2026-01-01T00:00:00+00:00"),
            ("2026W014T00:00:00+00:00", "2026-01-01T00:00:00+00:00"),
            ("20260102T030405Z", "2026-01-02T03:04:05+00:00"),
            ("2026-01-02T24:00:00+00:00", "2026-01-03T00:00:00+00:00"),
            ("2026-01-02T03:04:05X+00:00", "2026-01-02T03:04:05+00:00"),
            (
                "2026-01-02T03:04:05+05:30:15.5",
                "2026-01-01T21:33:49.500000+00:00",
            ),
        ];
        for (input, expected) in cases {
            let parsed = KernelTimestamp::parse_isoformat(input).expect(input);
            assert_eq!(parsed.isoformat(), expected, "{input}");
        }
    }

    #[test]
    fn distinguishes_naive_from_invalid() {
        assert_eq!(
            KernelTimestamp::parse_isoformat("2026-01-02T03:04:05"),
            Err(IsoformatError::Naive)
        );
        assert_eq!(
            KernelTimestamp::parse_isoformat("2026-01-02"),
            Err(IsoformatError::Naive)
        );
        assert_eq!(
            KernelTimestamp::parse_isoformat("2026-01-02T03:04:05z"),
            Err(IsoformatError::Invalid)
        );
        assert_eq!(
            KernelTimestamp::parse_isoformat("2026-01-02T03:04:60Z"),
            Err(IsoformatError::Invalid)
        );
        assert_eq!(
            KernelTimestamp::parse_isoformat("2026-02-29T00:00:00Z"),
            Err(IsoformatError::Invalid)
        );
    }

    #[test]
    fn workflow_loader_replaces_uppercase_z_first() {
        assert_eq!(
            KernelTimestamp::parse_isoformat_z_replaced("2026-01-02T03:04:05Z")
                .expect("valid")
                .isoformat(),
            "2026-01-02T03:04:05+00:00"
        );
    }
}
