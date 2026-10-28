//! Token usage reported by one language-model provider call.
//!
//! The Studio never estimates usage. A provider that does not report a count
//! leaves it `None`, and a turn whose calls all stayed silent answers `null`.
//! Parsing is deliberately forgiving: a malformed `usage` block is ignored,
//! never a reason to fail a turn, and the values are never logged with the
//! prompt text.

use serde_json::Value;

/// The token counts one provider call reported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChatUsage {
    /// The prompt tokens the provider billed, when it reported them.
    pub input_tokens: Option<u64>,
    /// The completion tokens the provider billed, when it reported them.
    pub output_tokens: Option<u64>,
}

impl ChatUsage {
    /// The usage of a call that reported nothing.
    pub const NONE: Self = Self {
        input_tokens: None,
        output_tokens: None,
    };

    /// Whether the provider reported any count at all.
    #[must_use]
    pub fn is_reported(&self) -> bool {
        self.input_tokens.is_some() || self.output_tokens.is_some()
    }

    /// Add the counts of another call of the same turn.
    ///
    /// A field only one call reported keeps that call's value; a field two
    /// calls reported is their sum. A saturated sum is still a reported
    /// field, so a hostile count can never turn a number into "not reported".
    #[must_use]
    pub fn merge(self, other: Self) -> Self {
        Self {
            input_tokens: add_optional(self.input_tokens, other.input_tokens),
            output_tokens: add_optional(self.output_tokens, other.output_tokens),
        }
    }
}

fn add_optional(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        (left, right) => left.or(right),
    }
}

/// Read one non-negative integer count from a JSON field.
///
/// A float, a string, a negative number, a non-finite constant or a missing
/// field is "not reported": the Studio never rounds or guesses a count.
#[must_use]
pub(crate) fn count(value: Option<&Value>) -> Option<u64> {
    match value {
        Some(Value::Number(number)) => number.as_u64(),
        _ => None,
    }
}

/// The sum of every count that is present.
///
/// Claude Code reports one billable input count as three fields; the sum is
/// the input count only when at least one of them is present.
#[must_use]
pub(crate) fn sum<I>(values: I) -> Option<u64>
where
    I: IntoIterator<Item = Option<u64>>,
{
    let mut total: Option<u64> = None;
    for value in values.into_iter().flatten() {
        total = Some(total.unwrap_or(0).saturating_add(value));
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_usage_is_not_reported() {
        assert!(!ChatUsage::NONE.is_reported());
        assert_eq!(ChatUsage::NONE.merge(ChatUsage::NONE), ChatUsage::NONE);
    }

    #[test]
    fn merging_adds_what_both_calls_reported_and_keeps_what_one_did() {
        let first = ChatUsage {
            input_tokens: Some(10),
            output_tokens: None,
        };
        let second = ChatUsage {
            input_tokens: Some(5),
            output_tokens: Some(7),
        };
        assert_eq!(
            first.merge(second),
            ChatUsage {
                input_tokens: Some(15),
                output_tokens: Some(7),
            }
        );
    }

    #[test]
    fn a_hostile_count_saturates_instead_of_disappearing() {
        let usage = ChatUsage {
            input_tokens: Some(u64::MAX),
            output_tokens: Some(u64::MAX),
        };
        let merged = usage.merge(usage);
        assert_eq!(merged.input_tokens, Some(u64::MAX));
        assert!(merged.is_reported());
    }

    #[test]
    fn only_a_real_non_negative_integer_is_a_count() {
        assert_eq!(count(Some(&Value::from(12))), Some(12));
        assert_eq!(count(Some(&Value::from(12.0))), None);
        assert_eq!(count(Some(&Value::from(-1))), None);
        assert_eq!(count(Some(&Value::from("12"))), None);
        assert_eq!(count(Some(&Value::Null)), None);
        assert_eq!(count(None), None);
    }

    #[test]
    fn the_claude_input_is_the_sum_of_the_present_fields() {
        assert_eq!(sum([Some(1), Some(2), None]), Some(3));
        assert_eq!(sum([None, None, None]), None);
    }
}
