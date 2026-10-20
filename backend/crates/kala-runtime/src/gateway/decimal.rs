//! Exact decimal multiplication and division with CPython semantics.
//!
//! The accepted `kala-budget` crate owns the CPython-compatible [`Decimal`]
//! type, but its public surface exposes only addition and subtraction:
//!
//! ```text
//! $ grep -n 'pub fn ' backend/crates/kala-budget/src/decimal/mod.rs
//! pub fn add ... pub fn sub ... (no multiplication, no division)
//! ```
//!
//! The gateway needs both: the provider cost is
//! `(input_tokens * input_cost + output_tokens * output_cost) / 1_000_000`.
//! This module therefore implements the two missing operations locally,
//! against the CPython default context (precision 28, `ROUND_HALF_EVEN`,
//! `Emax = 999999`, `Emin = -999999`), without touching the accepted crate.
//!
//! The multiplication is the exact product, rounded once to the context
//! precision. The division reproduces `libmpdec`'s `_mpd_qdiv`: the quotient
//! is computed by long division, the result keeps the shortest exact
//! representation when the quotient terminates, and a non-terminating
//! quotient is rounded to the context precision with the correct "sticky"
//! digit.

use kala_budget::{Decimal, PYTHON_DEFAULT_EMAX, PYTHON_DEFAULT_EMIN, PYTHON_DEFAULT_PRECISION};
use num_bigint::BigUint;
use num_traits::{One, ToPrimitive, Zero};

/// A decimal arithmetic failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecimalArithmeticError {
    /// The result is outside the context's exponent range (`Overflow`).
    Overflow,
    /// The divisor was zero (`DivisionByZero`).
    DivisionByZero,
    /// The exponents could not be represented (`InvalidOperation`).
    InvalidOperation,
}

impl DecimalArithmeticError {
    /// The name of the Python `decimal` exception class.
    pub const fn python_class_name(self) -> &'static str {
        match self {
            Self::Overflow => "Overflow",
            Self::DivisionByZero => "DivisionByZero",
            Self::InvalidOperation => "InvalidOperation",
        }
    }
}

impl std::fmt::Display for DecimalArithmeticError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.python_class_name())
    }
}

impl std::error::Error for DecimalArithmeticError {}

fn pow10(power: u64) -> BigUint {
    BigUint::from(10u32).pow(u32::try_from(power).expect("a power of ten fits in u32"))
}

/// The `libmpdec` rounding indicator of a right shift: `0..9`, where only `5`
/// means an exact tie and `6..9` carry the "sticky" rest.
fn shift_right_round(coefficient: &BigUint, shift: u64) -> (BigUint, u8) {
    if coefficient.is_zero() || shift == 0 {
        return (coefficient.clone(), 0);
    }
    let digits = coefficient.to_str_radix(10).len() as u64;
    if shift > digits {
        return (BigUint::zero(), 1);
    }
    if shift == digits {
        let leading = coefficient.to_str_radix(10).bytes().next().unwrap_or(b'0') - b'0';
        let rest = (coefficient % pow10(digits - 1)) != BigUint::zero();
        return (BigUint::zero(), combine_indicator(leading, rest));
    }
    let divisor = pow10(shift);
    let quotient = coefficient / &divisor;
    let remainder = coefficient % &divisor;
    let leading = (&remainder / pow10(shift - 1))
        .to_str_radix(10)
        .bytes()
        .next()
        .unwrap_or(b'0')
        - b'0';
    let rest = (&remainder % pow10(shift - 1)) != BigUint::zero();
    (quotient, combine_indicator(leading, rest))
}

fn combine_indicator(first: u8, rest: bool) -> u8 {
    if (first == 0 || first == 5) && rest {
        first + 1
    } else {
        first
    }
}

fn rounds_away(indicator: u8, coefficient: &BigUint) -> bool {
    match indicator {
        0..=4 => false,
        5 => (coefficient % BigUint::from(2u32)).is_one(),
        _ => true,
    }
}

fn adjusted_exponent(exponent: i64, coefficient: &BigUint) -> Result<i64, DecimalArithmeticError> {
    let digits = if coefficient.is_zero() {
        1i128
    } else {
        i128::from(coefficient.to_str_radix(10).len() as u64)
    };
    let adjusted = i128::from(exponent) + digits - 1;
    i64::try_from(adjusted).map_err(|_| DecimalArithmeticError::InvalidOperation)
}

/// Apply the CPython default context to an exact result: check the exponent
/// range, then round to the context precision.
fn finalize(
    sign: bool,
    coefficient: BigUint,
    exponent: i64,
) -> Result<Decimal, DecimalArithmeticError> {
    let mut coefficient = coefficient;
    let mut exponent = exponent;
    let precision = u64::from(PYTHON_DEFAULT_PRECISION);
    let etiny = PYTHON_DEFAULT_EMIN - (PYTHON_DEFAULT_PRECISION as i64 - 1);
    let adjusted = adjusted_exponent(exponent, &coefficient)?;

    if adjusted > PYTHON_DEFAULT_EMAX {
        if coefficient.is_zero() {
            return Ok(Decimal::from_parts(sign, coefficient, PYTHON_DEFAULT_EMAX));
        }
        return Err(DecimalArithmeticError::Overflow);
    }

    if adjusted < PYTHON_DEFAULT_EMIN {
        if coefficient.is_zero() {
            if exponent < etiny {
                exponent = etiny;
            }
            return Ok(Decimal::from_parts(sign, coefficient, exponent));
        }
        if exponent < etiny {
            let shift = u64::try_from(etiny - exponent)
                .map_err(|_| DecimalArithmeticError::InvalidOperation)?;
            let (quotient, indicator) = shift_right_round(&coefficient, shift);
            coefficient = quotient;
            exponent = etiny;
            if rounds_away(indicator, &coefficient) {
                coefficient += BigUint::one();
            }
        }
    }

    let digits = if coefficient.is_zero() {
        1
    } else {
        coefficient.to_str_radix(10).len() as u64
    };
    if digits > precision {
        let shift = digits - precision;
        let (quotient, indicator) = shift_right_round(&coefficient, shift);
        exponent = exponent
            .checked_add(
                i64::try_from(shift).map_err(|_| DecimalArithmeticError::InvalidOperation)?,
            )
            .ok_or(DecimalArithmeticError::InvalidOperation)?;
        coefficient = quotient;
        if rounds_away(indicator, &coefficient) {
            coefficient += BigUint::one();
            if coefficient.to_str_radix(10).len() as u64 > precision {
                coefficient = pow10(precision - 1);
                exponent = exponent
                    .checked_add(1)
                    .ok_or(DecimalArithmeticError::InvalidOperation)?;
                if adjusted_exponent(exponent, &coefficient)? > PYTHON_DEFAULT_EMAX {
                    return Err(DecimalArithmeticError::Overflow);
                }
            }
        }
    }

    Ok(Decimal::from_parts(sign, coefficient, exponent))
}

/// Multiply two decimals with the CPython default context.
pub fn multiply(left: &Decimal, right: &Decimal) -> Result<Decimal, DecimalArithmeticError> {
    let sign = left.is_negative() != right.is_negative();
    let coefficient = left.coefficient() * right.coefficient();
    let exponent = left
        .exponent()
        .checked_add(right.exponent())
        .ok_or(DecimalArithmeticError::InvalidOperation)?;
    finalize(sign, coefficient, exponent)
}

/// The digits of a quotient, produced by long division.
struct DigitSource {
    integer: Vec<u8>,
    fraction: Vec<u8>,
    remainder: BigUint,
    divisor: BigUint,
}

impl DigitSource {
    fn new(numerator: BigUint, divisor: BigUint) -> Self {
        let integer = &numerator / &divisor;
        let remainder = numerator % &divisor;
        let integer = if integer.is_zero() {
            Vec::new()
        } else {
            integer
                .to_str_radix(10)
                .bytes()
                .map(|byte| byte - b'0')
                .collect()
        };
        Self {
            integer,
            fraction: Vec::new(),
            remainder,
            divisor,
        }
    }

    fn at(&mut self, index: usize) -> u8 {
        if index < self.integer.len() {
            return self.integer[index];
        }
        let offset = index - self.integer.len();
        while self.fraction.len() <= offset {
            self.remainder *= BigUint::from(10u32);
            let digit = (&self.remainder / &self.divisor).to_u8().unwrap_or(0);
            self.remainder %= &self.divisor;
            self.fraction.push(digit);
        }
        self.fraction[offset]
    }

    /// The index of the first nonzero digit.
    fn first_significant(&mut self) -> usize {
        let mut index = 0usize;
        loop {
            if self.at(index) != 0 {
                return index;
            }
            index += 1;
        }
    }

    /// Whether every digit from `from` onwards is zero.
    fn tail_is_zero(&mut self, from: usize) -> bool {
        let mut index = from;
        while index < self.integer.len() {
            if self.integer[index] != 0 {
                return false;
            }
            index += 1;
        }
        loop {
            let digit = self.at(index);
            if digit != 0 {
                return false;
            }
            index += 1;
            if self.remainder.is_zero() {
                return true;
            }
        }
    }
}

fn digits_to_coefficient(digits: &[u8]) -> BigUint {
    let mut text = String::with_capacity(digits.len());
    for digit in digits {
        text.push(char::from(b'0' + digit));
    }
    BigUint::parse_bytes(text.as_bytes(), 10).unwrap_or_else(BigUint::zero)
}

/// Divide two decimals with the CPython default context.
///
/// The result keeps the shortest exact representation when the quotient
/// terminates, exactly like `libmpdec`'s `_mpd_qdiv`.
pub fn divide(left: &Decimal, right: &Decimal) -> Result<Decimal, DecimalArithmeticError> {
    if right.is_zero() {
        // CPython raises `InvalidOperation` for `0/0` and `DivisionByZero`
        // for a nonzero dividend.
        return Err(if left.is_zero() {
            DecimalArithmeticError::InvalidOperation
        } else {
            DecimalArithmeticError::DivisionByZero
        });
    }
    let sign = left.is_negative() != right.is_negative();
    let scale = left
        .exponent()
        .checked_sub(right.exponent())
        .ok_or(DecimalArithmeticError::InvalidOperation)?;
    if left.coefficient().is_zero() {
        // A zero dividend keeps the ideal exponent, like CPython.
        return Ok(Decimal::from_parts(sign, BigUint::zero(), scale));
    }

    let mut source = DigitSource::new(left.coefficient().clone(), right.coefficient().clone());
    let integer_len = source.integer.len() as i64;
    let point = integer_len
        .checked_add(scale)
        .ok_or(DecimalArithmeticError::InvalidOperation)?;
    // The ideal exponent of the division, the way `libmpdec` computes it.
    let ideal = left
        .exponent()
        .checked_sub(right.exponent())
        .ok_or(DecimalArithmeticError::InvalidOperation)?;
    let leading = source.first_significant();
    let adjusted = point
        .checked_sub(leading as i64)
        .and_then(|value| value.checked_sub(1))
        .ok_or(DecimalArithmeticError::InvalidOperation)?;
    if adjusted > PYTHON_DEFAULT_EMAX {
        return Err(DecimalArithmeticError::Overflow);
    }

    let precision = PYTHON_DEFAULT_PRECISION as usize;
    let mut digits = Vec::with_capacity(precision + 1);
    for index in 0..=precision {
        digits.push(source.at(leading + index));
    }

    // A terminating quotient keeps its exact shortest representation, scaled
    // towards the ideal exponent the way `libmpdec` does: the coefficient
    // grows by the trailing zeros and the exponent shrinks.
    let terminated = source.remainder.is_zero();
    if terminated {
        let last = digits
            .iter()
            .rposition(|digit| *digit != 0)
            .map(|index| index + 1)
            .unwrap_or(0);
        if last > 0 && last <= precision {
            let minimal = digits_to_coefficient(&digits[..last]);
            let minimal_exponent = adjusted
                .checked_sub((last as i64) - 1)
                .ok_or(DecimalArithmeticError::InvalidOperation)?;
            let exponent = minimal_exponent.min(ideal);
            let scale = minimal_exponent - exponent;
            if scale >= 0 && (scale as u64) <= precision as u64 {
                let scaled = &minimal * pow10(scale as u64);
                if scaled.to_str_radix(10).len() as u64 <= precision as u64 {
                    return finalize(sign, scaled, exponent);
                }
            }
        }
    }

    // A non-terminating quotient is rounded to the context precision.
    let coefficient = digits_to_coefficient(&digits[..precision]);
    let guard = digits[precision];
    let tail_nonzero = !source.tail_is_zero(leading + precision + 1);
    let odd = (&coefficient % BigUint::from(2u32)).is_one();
    let round_up = match guard {
        0..=4 => false,
        5 => tail_nonzero || odd,
        _ => true,
    };
    let mut coefficient = coefficient;
    let mut exponent = adjusted
        .checked_sub(precision as i64 - 1)
        .ok_or(DecimalArithmeticError::InvalidOperation)?;
    if round_up {
        coefficient += BigUint::one();
        if coefficient.to_str_radix(10).len() as u64 > precision as u64 {
            coefficient = pow10(precision as u64 - 1);
            exponent = exponent
                .checked_add(1)
                .ok_or(DecimalArithmeticError::InvalidOperation)?;
        }
    }
    finalize(sign, coefficient, exponent)
}
