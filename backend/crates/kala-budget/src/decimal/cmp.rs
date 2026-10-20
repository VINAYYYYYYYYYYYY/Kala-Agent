//! Exact numeric comparison for [`Decimal`], independent of any context.

use super::Decimal;
use std::cmp::Ordering;

pub(super) fn compare(left: &Decimal, right: &Decimal) -> Ordering {
    // Zeros compare equal regardless of sign or stored exponent.
    if left.is_zero() && right.is_zero() {
        return Ordering::Equal;
    }
    if left.is_zero() {
        return if right.is_negative() {
            Ordering::Greater
        } else {
            Ordering::Less
        };
    }
    if right.is_zero() {
        return if left.is_negative() {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    let left_negative = left.is_negative();
    let right_negative = right.is_negative();
    if left_negative != right_negative {
        return if left_negative {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    let magnitude = compare_magnitude(left, right);
    if left_negative {
        magnitude.reverse()
    } else {
        magnitude
    }
}

fn compare_magnitude(left: &Decimal, right: &Decimal) -> Ordering {
    let left_adjusted = left.adjusted_exponent();
    let right_adjusted = right.adjusted_exponent();
    if left_adjusted != right_adjusted {
        return left_adjusted.cmp(&right_adjusted);
    }
    // Same adjusted exponent: compare the significant digits top-aligned.
    let left_digits = left.coefficient().to_str_radix(10);
    let right_digits = right.coefficient().to_str_radix(10);
    let width = left_digits.len().max(right_digits.len());
    let left_bytes = left_digits.as_bytes();
    let right_bytes = right_digits.as_bytes();
    for index in 0..width {
        let left_digit = left_bytes.get(index).copied().unwrap_or(b'0');
        let right_digit = right_bytes.get(index).copied().unwrap_or(b'0');
        if left_digit != right_digit {
            return left_digit.cmp(&right_digit);
        }
    }
    Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    fn decimal(text: &str) -> Decimal {
        Decimal::parse(text).expect("valid decimal")
    }

    #[test]
    fn compares_scale_and_exponent_variants_exactly() {
        assert_eq!(decimal("1.0").compare(&decimal("1")), Ordering::Equal);
        assert_eq!(decimal("0.10").compare(&decimal("0.1")), Ordering::Equal);
        assert_eq!(decimal("-0.00").compare(&decimal("0")), Ordering::Equal);
        assert_eq!(decimal("1e40").compare(&decimal("1e39")), Ordering::Greater);
        assert_eq!(
            decimal("1e-999999").compare(&decimal("1e-1000000")),
            Ordering::Greater
        );
        assert_eq!(
            decimal("123456789012345678901234567890.12345")
                .compare(&decimal("123456789012345678901234567890.12344")),
            Ordering::Greater
        );
        assert_eq!(decimal("-1").compare(&decimal("0")), Ordering::Less);
        assert_eq!(decimal("0.70").compare(&decimal("0")), Ordering::Greater);
        assert_eq!(decimal("0").compare(&decimal("0.70")), Ordering::Less);
        assert_eq!(decimal("-0.70").compare(&decimal("0")), Ordering::Less);
        assert_eq!(decimal("0").compare(&decimal("-0.70")), Ordering::Greater);
        assert_eq!(decimal("-0.00").compare(&decimal("0E+5")), Ordering::Equal);
    }
}
