//! Decimal addition and subtraction ported from `libmpdec`'s `_mpd_qaddsub`.
//!
//! The key property is that operand alignment is bounded: when the smaller
//! operand lies below the retained precision window of the larger one it is
//! replaced by a one-digit "tiny" value that produces the same rounded
//! result, so exponents differing by millions cost only a few dozen digits.

use super::round::{finalize, pow10};
use super::{Context, Decimal, DecimalError};
use num_bigint::{BigInt, BigUint, Sign};

pub(super) fn add(
    left: &Decimal,
    right: &Decimal,
    context: &Context,
) -> Result<Decimal, DecimalError> {
    // libmpdec aligns by stored exponent: the operand with the larger
    // exponent becomes "big".
    let (mut big, mut small) = if right.exponent() > left.exponent() {
        (right.clone(), left.clone())
    } else {
        (left.clone(), right.clone())
    };

    if big.coefficient() != &BigUint::default() && big.exponent() != small.exponent() {
        let big_digits = big.digit_count() as i64;
        let precision = i64::from(context.precision());
        let cutoff = big
            .exponent()
            .checked_sub(1)
            .and_then(|value| {
                value.checked_add(if big_digits > precision {
                    0
                } else {
                    big_digits - precision - 1
                })
            })
            .ok_or(DecimalError::ExponentOutOfRange)?;
        if small.adjusted_exponent() < cutoff {
            let coefficient = if small.is_zero() {
                BigUint::default()
            } else {
                BigUint::from(1u32)
            };
            small = Decimal::from_parts(small.is_negative(), coefficient, cutoff);
        }
        let shift = big
            .exponent()
            .checked_sub(small.exponent())
            .ok_or(DecimalError::ExponentOutOfRange)?;
        let shift = u64::try_from(shift).map_err(|_| DecimalError::ExponentOutOfRange)?;
        big = Decimal::from_parts(
            big.is_negative(),
            big.coefficient() * pow10(shift),
            small.exponent(),
        );
    }

    let result_exponent = small.exponent();
    let sum = signed(&big) + signed(&small);
    let negative = match sum.sign() {
        Sign::Minus => true,
        Sign::Plus => false,
        Sign::NoSign => left.is_negative() && right.is_negative(),
    };
    let mut result = Decimal::from_parts(negative, sum.magnitude().clone(), result_exponent);
    finalize(&mut result, context)?;
    Ok(result)
}

fn signed(value: &Decimal) -> BigInt {
    BigInt::from_biguint(
        if value.is_negative() {
            Sign::Minus
        } else {
            Sign::Plus
        },
        value.coefficient().clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decimal(text: &str) -> Decimal {
        Decimal::parse(text).expect("valid decimal")
    }

    fn add_text(left: &str, right: &str) -> String {
        decimal(left)
            .add(&decimal(right))
            .expect("addition succeeds")
            .to_python_string()
    }

    #[test]
    fn preserves_scale_and_ties_to_even() {
        assert_eq!(add_text("0.10", "0.2"), "0.30");
        assert_eq!(add_text("0.1", "0.2"), "0.3");
        assert_eq!(add_text("1.00", "2.00"), "3.00");
        assert_eq!(add_text("0.5", "0.5"), "1.0");
        assert_eq!(add_text("1", "2"), "3");
        assert_eq!(add_text("1.00", "-1.00"), "0.00");
        assert_eq!(
            add_text("1.2345678901234567890123456785", "1e-40"),
            "1.234567890123456789012345679"
        );
    }

    #[test]
    fn handles_far_apart_exponents_without_huge_allocations() {
        assert_eq!(
            add_text("1e+999999", "1e-999999"),
            "1.000000000000000000000000000E+999999"
        );
        assert_eq!(add_text("1e+999999", "-1e+999999"), "0E+999999");
        assert_eq!(add_text("1", "0E-1000000"), "1.000000000000000000000000000");
    }

    #[test]
    fn overflow_and_underflow_follow_the_default_context() {
        assert_eq!(
            decimal("9e999999").add(&decimal("9e999999")).unwrap_err(),
            DecimalError::Overflow
        );
        assert_eq!(add_text("1e-1000000", "1e-1000000"), "2E-1000000");
        assert_eq!(add_text("1e-999999999", "1e-999999999"), "0E-1000026");
    }

    #[test]
    fn signed_zero_results_match_cpython() {
        assert_eq!(add_text("-0", "-0"), "-0");
        assert_eq!(add_text("-0", "0"), "0");
        assert_eq!(add_text("0E+5", "0E-5"), "0.00000");
    }
}
