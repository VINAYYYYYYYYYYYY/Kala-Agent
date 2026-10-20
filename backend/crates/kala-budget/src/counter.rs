//! Arbitrary-size nonnegative integer counters.
//!
//! Python's budget counters are unbounded `int` values; the ledger must not
//! wrap or silently saturate them. [`Counter`] keeps the same arbitrary
//! precision by wrapping [`num_bigint::BigUint`].

use num_bigint::BigUint;
use num_traits::ToPrimitive;
use std::fmt;
use std::str::FromStr;

/// Error from constructing a [`Counter`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CounterError {
    /// The text is not a decimal integer.
    #[error("counter must be an integer")]
    Invalid,
    /// The value is negative; budget counters must be nonnegative.
    #[error("counter must be nonnegative")]
    Negative,
}

/// An arbitrary-precision nonnegative integer.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Counter(BigUint);

impl Counter {
    /// The counter value zero.
    #[must_use]
    pub fn zero() -> Self {
        Self(BigUint::default())
    }

    /// The counter value one.
    #[must_use]
    pub fn one() -> Self {
        Self(BigUint::from(1u32))
    }

    /// Parse a decimal integer (surrounding whitespace and interior
    /// underscores are accepted like Python's `int`).
    pub fn parse(text: &str) -> Result<Self, CounterError> {
        let trimmed = text.trim_matches(|character: char| character.is_whitespace());
        let (negative, digits) = match trimmed.as_bytes().first() {
            Some(b'-') => (true, &trimmed[1..]),
            Some(b'+') => (false, &trimmed[1..]),
            _ => (false, trimmed),
        };
        let mut cleaned = String::with_capacity(digits.len());
        let bytes = digits.as_bytes();
        for (index, byte) in bytes.iter().enumerate() {
            if *byte == b'_' {
                let previous_is_digit = index > 0 && bytes[index - 1].is_ascii_digit();
                let next_is_digit = index + 1 < bytes.len() && bytes[index + 1].is_ascii_digit();
                if !previous_is_digit || !next_is_digit {
                    return Err(CounterError::Invalid);
                }
                continue;
            }
            if !byte.is_ascii_digit() {
                return Err(CounterError::Invalid);
            }
            cleaned.push(char::from(*byte));
        }
        if cleaned.is_empty() {
            return Err(CounterError::Invalid);
        }
        let value = BigUint::parse_bytes(cleaned.as_bytes(), 10).ok_or(CounterError::Invalid)?;
        if negative && value != BigUint::default() {
            return Err(CounterError::Negative);
        }
        Ok(Self(value))
    }

    /// Convert a signed integer, rejecting negative values.
    pub fn try_from_i64(value: i64) -> Result<Self, CounterError> {
        if value < 0 {
            return Err(CounterError::Negative);
        }
        Ok(Self(BigUint::from(value.unsigned_abs())))
    }

    /// True when the counter is zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0 == BigUint::default()
    }

    /// Borrow the underlying arbitrary-precision integer.
    #[must_use]
    pub const fn as_biguint(&self) -> &BigUint {
        &self.0
    }

    /// Add two counters (arbitrary precision, never wraps).
    #[must_use]
    pub fn add(&self, other: &Self) -> Self {
        Self(&self.0 + &other.0)
    }

    /// Increment by one.
    #[must_use]
    pub fn increment(&self) -> Self {
        self.add(&Self::one())
    }

    /// Subtract `other` when it does not exceed `self`.
    #[must_use]
    pub fn checked_sub(&self, other: &Self) -> Option<Self> {
        if self.0 < other.0 {
            None
        } else {
            Some(Self(&self.0 - &other.0))
        }
    }

    /// Convert to `u64` when the value fits.
    #[must_use]
    pub fn to_u64(&self) -> Option<u64> {
        self.0.to_u64()
    }

    /// Decimal digits without separators.
    #[must_use]
    pub fn to_decimal_string(&self) -> String {
        self.0.to_str_radix(10)
    }
}

impl From<u64> for Counter {
    fn from(value: u64) -> Self {
        Self(BigUint::from(value))
    }
}

impl From<u32> for Counter {
    fn from(value: u32) -> Self {
        Self(BigUint::from(value))
    }
}

impl From<usize> for Counter {
    fn from(value: usize) -> Self {
        Self(BigUint::from(value))
    }
}

impl From<&Counter> for Counter {
    fn from(value: &Counter) -> Self {
        value.clone()
    }
}

impl From<BigUint> for Counter {
    fn from(value: BigUint) -> Self {
        Self(value)
    }
}

impl FromStr for Counter {
    type Err = CounterError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl fmt::Display for Counter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0.to_str_radix(10))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_arbitrary_precision_and_rejects_negatives() {
        let huge = Counter::parse("10000000000000000000000000000000000000000").expect("parse");
        assert_eq!(huge.to_decimal_string().len(), 41);
        assert_eq!(huge.add(&Counter::one()).to_decimal_string().len(), 41);
        assert_eq!(
            Counter::try_from_i64(-1).unwrap_err(),
            CounterError::Negative
        );
        assert_eq!(
            Counter::parse("-0").expect("negative zero"),
            Counter::zero()
        );
        assert_eq!(
            Counter::parse("1_0").expect("underscore"),
            Counter::from(10u32)
        );
        assert_eq!(Counter::parse("_1").unwrap_err(), CounterError::Invalid);
    }
}
