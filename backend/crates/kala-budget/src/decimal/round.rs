//! Context finalization ported from `libmpdec`'s `mpd_qfinalize`.

use super::{Context, Decimal, DecimalError, RoundingMode};
use num_bigint::BigUint;
use num_traits::One;

/// Return `10^power` as a `BigUint`.
pub(super) fn pow10(power: u64) -> BigUint {
    BigUint::from(10u32).pow(u32::try_from(power).expect("power of ten fits in u32"))
}

fn digit_value(value: &BigUint) -> u8 {
    value
        .to_str_radix(10)
        .bytes()
        .next()
        .map(|byte| byte - b'0')
        .unwrap_or(0)
}

/// Right-shift the coefficient by `shift` decimal digits, returning the
/// quotient and the `libmpdec` rounding indicator (`0..9`, with `6..9`
/// carrying the "sticky" rest so that only `5` means an exact tie).
pub(super) fn shift_right_round(coefficient: &BigUint, shift: u64) -> (BigUint, u8) {
    if coefficient == &BigUint::default() || shift == 0 {
        return (coefficient.clone(), 0);
    }
    let digits = coefficient.to_str_radix(10).len() as u64;
    if shift > digits {
        return (BigUint::default(), 1);
    }
    if shift == digits {
        let leading = digit_value(coefficient);
        let rest = (coefficient % pow10(digits - 1)) != BigUint::default();
        return (BigUint::default(), combine_indicator(leading, rest));
    }
    let divisor = pow10(shift);
    let quotient = coefficient / &divisor;
    let remainder = coefficient % &divisor;
    let leading = digit_value(&(&remainder / pow10(shift - 1)));
    let rest = (&remainder % pow10(shift - 1)) != BigUint::default();
    (quotient, combine_indicator(leading, rest))
}

fn combine_indicator(first: u8, rest: bool) -> u8 {
    if (first == 0 || first == 5) && rest {
        first + 1
    } else {
        first
    }
}

fn is_odd(coefficient: &BigUint) -> bool {
    (coefficient % BigUint::from(2u32)) == BigUint::one()
}

fn rounds_away(indicator: u8, coefficient: &BigUint, rounding: RoundingMode) -> bool {
    match rounding {
        RoundingMode::HalfEven => match indicator {
            0..=4 => false,
            5 => is_odd(coefficient),
            _ => true,
        },
    }
}

/// Finalize a result exactly like `mpd_qfinalize`: check the exponent range,
/// then round to the context precision.
pub(super) fn finalize(value: &mut Decimal, context: &Context) -> Result<(), DecimalError> {
    let sign = value.is_negative();
    let mut coefficient = value.coefficient().clone();
    let mut exponent = value.exponent();

    let etiny = context.etiny();
    let adjusted = adjusted_exponent(exponent, &coefficient)?;

    if adjusted > context.emax() {
        if coefficient == BigUint::default() {
            // A zero result with an out-of-range exponent is clamped to Emax.
            *value = Decimal::from_parts(sign, coefficient, context.emax());
            return Ok(());
        }
        return Err(DecimalError::Overflow);
    }

    if adjusted < context.emin() {
        if coefficient == BigUint::default() {
            if exponent < etiny {
                exponent = etiny;
            }
            *value = Decimal::from_parts(sign, coefficient, exponent);
            return Ok(());
        }
        if exponent < etiny {
            let shift =
                u64::try_from(etiny - exponent).map_err(|_| DecimalError::ExponentOutOfRange)?;
            let (quotient, indicator) = shift_right_round(&coefficient, shift);
            coefficient = quotient;
            exponent = etiny;
            if rounds_away(indicator, &coefficient, context.rounding()) {
                coefficient += BigUint::one();
            }
        }
    }

    let digits = if coefficient == BigUint::default() {
        1
    } else {
        coefficient.to_str_radix(10).len() as u64
    };
    if digits > u64::from(context.precision()) {
        let shift = digits - u64::from(context.precision());
        let (quotient, indicator) = shift_right_round(&coefficient, shift);
        exponent = exponent
            .checked_add(i64::try_from(shift).map_err(|_| DecimalError::ExponentOutOfRange)?)
            .ok_or(DecimalError::ExponentOutOfRange)?;
        coefficient = quotient;
        if rounds_away(indicator, &coefficient, context.rounding()) {
            coefficient += BigUint::one();
            if coefficient.to_str_radix(10).len() as u64 > u64::from(context.precision()) {
                // Only all-nines carry can overflow the precision: the
                // coefficient is exactly 10^precision after the increment.
                coefficient = pow10(u64::from(context.precision()) - 1);
                exponent = exponent
                    .checked_add(1)
                    .ok_or(DecimalError::ExponentOutOfRange)?;
                if adjusted_exponent(exponent, &coefficient)? > context.emax() {
                    return Err(DecimalError::Overflow);
                }
            }
        }
    }

    *value = Decimal::from_parts(sign, coefficient, exponent);
    Ok(())
}

fn adjusted_exponent(exponent: i64, coefficient: &BigUint) -> Result<i64, DecimalError> {
    let digits = if coefficient == &BigUint::default() {
        1i128
    } else {
        i128::from(coefficient.to_str_radix(10).len() as u64)
    };
    let adjusted = i128::from(exponent) + digits - 1;
    i64::try_from(adjusted).map_err(|_| DecimalError::ExponentOutOfRange)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_right_indicator_marks_sticky_digits() {
        let ten = BigUint::from(10u32);
        let (quotient, indicator) = shift_right_round(&ten, 1);
        assert_eq!(quotient, BigUint::from(1u32));
        assert_eq!(indicator, 0);
        let (quotient, indicator) = shift_right_round(&BigUint::from(15u32), 1);
        assert_eq!(quotient, BigUint::from(1u32));
        assert_eq!(indicator, 5);
        let (quotient, indicator) = shift_right_round(&BigUint::from(150u32), 1);
        assert_eq!(quotient, BigUint::from(15u32));
        assert_eq!(indicator, 0);
        let (quotient, indicator) = shift_right_round(&BigUint::from(151u32), 1);
        assert_eq!(quotient, BigUint::from(15u32));
        assert_eq!(indicator, 1);
        let (quotient, indicator) = shift_right_round(&BigUint::from(150u32), 2);
        assert_eq!(quotient, BigUint::from(1u32));
        assert_eq!(indicator, 5);
        let (quotient, indicator) = shift_right_round(&BigUint::from(150u32), 3);
        assert_eq!(quotient, BigUint::default());
        assert_eq!(indicator, 1);
    }
}
