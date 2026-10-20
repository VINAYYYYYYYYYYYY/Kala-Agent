//! Arbitrary-precision decimal values with CPython `decimal` semantics.
//!
//! The budget kernel accepts arbitrary finite nonnegative [`Decimal`] costs.
//! Values keep their stored coefficient and exponent exactly (including
//! trailing zeros and signed zero), while arithmetic applies the CPython
//! default context (precision 28, `ROUND_HALF_EVEN`, `Emax = 999999`,
//! `Emin = -999999`) the same way `decimal` does.
//!
//! The implementation is a native Rust port of the relevant `libmpdec`
//! behaviour used by CPython: operand alignment is bounded by the context
//! precision (or the operand coefficient length), so exponents differing by
//! millions never allocate a power of ten of that size, and gentle rounding
//! indicators ("sticky" digits) are preserved for exact ties-to-even results.

mod arith;
mod cmp;
mod format;
mod parse;
mod round;

pub use crate::decimal::parse::DecimalParseError;

use num_bigint::BigUint;
use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

/// CPython's default decimal context precision.
pub const PYTHON_DEFAULT_PRECISION: u32 = 28;
/// CPython's default decimal context `Emax`.
pub const PYTHON_DEFAULT_EMAX: i64 = 999_999;
/// CPython's default decimal context `Emin`.
pub const PYTHON_DEFAULT_EMIN: i64 = -999_999;
/// Largest exponent CPython accepts when constructing from a decimal string.
pub const PYTHON_MAX_EMAX: i64 = 999_999_999_999_999_999;
/// Smallest exponent CPython accepts when constructing from a decimal string
/// (`decimal.MIN_ETINY`).
pub const PYTHON_MIN_ETINY: i64 = -1_999_999_999_999_999_997;

/// Rounding modes supported by [`Context`]. The budget kernel only relies on
/// the CPython default [`RoundingMode::HalfEven`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundingMode {
    /// Round to nearest, ties to even.
    HalfEven,
}

/// Arithmetic context mirroring the CPython `decimal.Context` fields that
/// affect addition, subtraction and finalization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    precision: u32,
    rounding: RoundingMode,
    emax: i64,
    emin: i64,
}

impl Context {
    /// The CPython default context used by the budget kernel.
    #[must_use]
    pub const fn python_default() -> Self {
        Self {
            precision: PYTHON_DEFAULT_PRECISION,
            rounding: RoundingMode::HalfEven,
            emax: PYTHON_DEFAULT_EMAX,
            emin: PYTHON_DEFAULT_EMIN,
        }
    }

    /// Construct an explicit context. `precision` must be positive.
    ///
    /// Trusted construction API: CPython rejects non-positive precision and
    /// `emax < emin`, and rounding with a zero precision would panic. The
    /// budget kernel only uses [`Context::python_default`].
    #[must_use]
    pub const fn new(precision: u32, rounding: RoundingMode, emax: i64, emin: i64) -> Self {
        Self {
            precision,
            rounding,
            emax,
            emin,
        }
    }

    /// Context precision (total significant digits).
    #[must_use]
    pub const fn precision(self) -> u32 {
        self.precision
    }

    /// Context rounding mode.
    #[must_use]
    pub const fn rounding(self) -> RoundingMode {
        self.rounding
    }

    /// Context maximum adjusted exponent.
    #[must_use]
    pub const fn emax(self) -> i64 {
        self.emax
    }

    /// Context minimum adjusted exponent.
    #[must_use]
    pub const fn emin(self) -> i64 {
        self.emin
    }

    /// Smallest exponent a result may have before being rounded to zero.
    #[must_use]
    pub const fn etiny(self) -> i64 {
        self.emin - (self.precision as i64 - 1)
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::python_default()
    }
}

/// Errors raised by decimal arithmetic.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DecimalError {
    /// Arithmetic overflowed the context exponent range (`decimal.Overflow`).
    #[error("decimal overflow: result exponent is above the context Emax")]
    Overflow,
    /// An exponent calculation could not be represented in the supported range.
    #[error("decimal exponent is outside the supported range")]
    ExponentOutOfRange,
}

/// A finite arbitrary-precision decimal value.
///
/// The value is `(-1)^sign * coefficient * 10^exponent`. A zero coefficient
/// keeps its sign and exponent, matching `decimal.Decimal("-0.00")`.
#[derive(Clone, Debug, Default)]
pub struct Decimal {
    sign: bool,
    coefficient: BigUint,
    exponent: i64,
}

impl Decimal {
    /// Construct a decimal from its exact parts.
    ///
    /// A zero coefficient is allowed and preserves the supplied sign and
    /// exponent, matching `decimal.Decimal("-0.00")`.
    ///
    /// This is a trusted typed API: it does not re-validate `libmpdec`'s
    /// construction bounds. Callers should keep `exponent` within
    /// [`PYTHON_MIN_ETINY`]`..=`[`PYTHON_MAX_EMAX`] and, for a nonzero
    /// coefficient, keep the adjusted exponent at or below `PYTHON_MAX_EMAX`;
    /// [`Decimal::parse`] is the validating text entry point. Out-of-bounds
    /// values are still formatted, compared and added without panicking, but
    /// their behaviour is not claimed to match CPython.
    #[must_use]
    pub fn from_parts(sign: bool, coefficient: BigUint, exponent: i64) -> Self {
        Self {
            sign,
            coefficient,
            exponent,
        }
    }

    /// The exact decimal zero with exponent 0 and positive sign.
    #[must_use]
    pub fn zero() -> Self {
        Self::from_parts(false, BigUint::default(), 0)
    }

    /// Parse a decimal literal using CPython's accepted grammar.
    ///
    /// Surrounding whitespace and interior underscores are accepted like
    /// `decimal.Decimal`. Non-finite literals (`NaN`, `sNaN`, `Infinity`) are
    /// rejected because the budget kernel only accepts finite values.
    pub fn parse(text: &str) -> Result<Self, DecimalParseError> {
        parse::parse_decimal(text)
    }

    /// False when the value is negative (including negative zero).
    #[must_use]
    pub const fn is_negative(&self) -> bool {
        self.sign
    }

    /// True when the coefficient is zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.coefficient == BigUint::default()
    }

    /// True when the value is a negative zero.
    #[must_use]
    pub fn is_negative_zero(&self) -> bool {
        self.sign && self.is_zero()
    }

    /// Stored exponent (power of ten applied to the coefficient).
    #[must_use]
    pub const fn exponent(&self) -> i64 {
        self.exponent
    }

    /// Stored coefficient magnitude.
    #[must_use]
    pub const fn coefficient(&self) -> &BigUint {
        &self.coefficient
    }

    /// Number of significant decimal digits (`1` for zero).
    #[must_use]
    pub fn digit_count(&self) -> u64 {
        if self.is_zero() {
            1
        } else {
            self.coefficient.to_str_radix(10).len() as u64
        }
    }

    /// Adjusted exponent: the exponent of the most significant digit.
    #[must_use]
    pub fn adjusted_exponent(&self) -> i64 {
        let digits = if self.is_zero() {
            1i128
        } else {
            i128::from(self.digit_count())
        };
        let adjusted = i128::from(self.exponent) + digits - 1;
        i64::try_from(adjusted).unwrap_or(if adjusted < 0 { i64::MIN } else { i64::MAX })
    }

    /// Return the value with the sign flipped, including zeros.
    #[must_use]
    pub fn negated(&self) -> Self {
        Self::from_parts(!self.sign, self.coefficient.clone(), self.exponent)
    }

    /// Exact addition under the CPython default context.
    pub fn add(&self, other: &Self) -> Result<Self, DecimalError> {
        arith::add(self, other, &Context::python_default())
    }

    /// Exact subtraction under the CPython default context.
    pub fn sub(&self, other: &Self) -> Result<Self, DecimalError> {
        arith::add(self, &other.negated(), &Context::python_default())
    }

    /// Exact addition under an explicit context.
    pub fn add_with_context(&self, other: &Self, context: &Context) -> Result<Self, DecimalError> {
        arith::add(self, other, context)
    }

    /// Exact subtraction under an explicit context.
    pub fn sub_with_context(&self, other: &Self, context: &Context) -> Result<Self, DecimalError> {
        arith::add(self, &other.negated(), context)
    }

    /// Apply context rounding without changing the value when it already fits
    /// (the CPython unary-plus operation).
    pub fn rounded_with_context(&self, context: &Context) -> Result<Self, DecimalError> {
        let mut result = self.clone();
        round::finalize(&mut result, context)?;
        Ok(result)
    }

    /// Exact numeric comparison, unrelated to the arithmetic context.
    #[must_use]
    pub fn compare(&self, other: &Self) -> Ordering {
        cmp::compare(self, other)
    }

    /// Digit tuple `(sign, digits, exponent)` mirroring `Decimal.as_tuple()`.
    ///
    /// `sign` is `0` or `1`, `digits` is the coefficient digit vector
    /// (`[0]` for zero) and `exponent` is the stored exponent.
    #[must_use]
    pub fn as_tuple(&self) -> (u8, Vec<u8>, i64) {
        let digits = if self.is_zero() {
            vec![0]
        } else {
            self.coefficient
                .to_str_radix(10)
                .bytes()
                .map(|byte| byte - b'0')
                .collect()
        };
        (u8::from(self.sign), digits, self.exponent)
    }

    /// CPython `str(Decimal)` representation.
    #[must_use]
    pub fn to_python_string(&self) -> String {
        format::to_python_string(self)
    }

    /// Python `repr(float)` text, used when the kernel stringifies a float
    /// before constructing a decimal (for example `Decimal(str(cost))`).
    #[must_use]
    pub fn python_float_repr(value: f64) -> String {
        format::python_float_repr(value)
    }

    /// Build a decimal the way `_nonnegative_decimal` does for Python floats:
    /// round-trip through `str(float)` first.
    pub fn from_python_float(value: f64) -> Result<Self, DecimalParseError> {
        if !value.is_finite() {
            return Err(DecimalParseError::NonFinite);
        }
        Self::parse(&format::python_float_repr(value))
    }
}

impl FromStr for Decimal {
    type Err = DecimalParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl PartialEq for Decimal {
    fn eq(&self, other: &Self) -> bool {
        cmp::compare(self, other) == Ordering::Equal
    }
}

impl Eq for Decimal {}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> Ordering {
        cmp::compare(self, other)
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&format::to_python_string(self))
    }
}
