//! Pattern admission and matching.
//!
//! A schema pattern is hostile input: a tool author registers it, and every
//! tool argument is matched against it. The Python module therefore refuses a
//! pattern before it is ever compiled or run, and the refusal is decided from
//! the pattern's own text so that the decision is identical on every call.
//!
//! This module ports that admission analysis and adds a matcher with CPython
//! `re` semantics.
//!
//! The admission analysis has three parts, in the Python order:
//!
//! 1. the schema must bound the subject with a `maxLength`,
//! 2. the pattern must not use a construct outside the admitted subset
//!    (backreferences, lookaround assertions, conditional groups, inline
//!    flags, atomic or possessive groups, wildcard repetition),
//! 3. the pattern must not repeat a group that already contains a quantifier
//!    and must not admit more than one way to split a repetition.
//!
//! A refused pattern is never compiled and never executed.

mod engine;
mod unicode;

pub(crate) use engine::Pattern;

use crate::protocol::MAX_PATTERN_BACKTRACK_STATES;
use crate::protocol::MAX_PATTERN_INPUT_LENGTH;
use crate::protocol::MAX_PATTERN_LENGTH;
use crate::protocol::MAX_PATTERN_REPEAT;
use crate::protocol::MAX_PATTERN_UNBOUNDED_REPEATS;
use crate::protocol::error::ToolProtocolError;
use crate::protocol::pattern::engine::is_decimal;

/// The state budget of one pattern.
#[derive(Debug)]
struct PatternBudget {
    consumed: i128,
    states: u64,
    unbounded: u32,
}

impl PatternBudget {
    fn new() -> Self {
        Self {
            consumed: 0,
            states: 1,
            unbounded: 0,
        }
    }

    fn admit(&self, path: &str) -> Result<(), ToolProtocolError> {
        if self.states > MAX_PATTERN_BACKTRACK_STATES as u64 {
            return Err(ToolProtocolError::schema(
                "pattern exceeds the backtracking budget",
                path,
            ));
        }
        if self.consumed > MAX_PATTERN_INPUT_LENGTH as i128 {
            return Err(ToolProtocolError::schema(
                "pattern can match a string beyond the bounded input",
                path,
            ));
        }
        Ok(())
    }

    fn literal(&mut self, width: i128, path: &str) -> Result<(), ToolProtocolError> {
        self.consumed += width;
        self.admit(path)
    }

    fn repeat(
        &mut self,
        width: i128,
        maximum: Option<i128>,
        path: &str,
    ) -> Result<(), ToolProtocolError> {
        match maximum {
            None => {
                self.unbounded += 1;
                if self.unbounded > MAX_PATTERN_UNBOUNDED_REPEATS as u32 {
                    return Err(ToolProtocolError::schema(
                        "pattern repeats more than one unbounded atom",
                        path,
                    ));
                }
                self.states = self
                    .states
                    .saturating_mul((MAX_PATTERN_INPUT_LENGTH as u64) + 1);
            }
            Some(maximum) => {
                if maximum > MAX_PATTERN_REPEAT as i128 {
                    return Err(ToolProtocolError::schema(
                        "pattern repetition exceeds the bound",
                        path,
                    ));
                }
                self.states = self.states.saturating_mul((maximum as u64) + 1);
                self.consumed += width * maximum;
            }
        }
        self.admit(path)
    }
}

/// What the body of one open group contains.
#[derive(Debug, Default)]
struct PatternScope {
    alternative: bool,
    atoms: u32,
    width: i128,
}

/// The kinds of construct outside the admitted subset.
#[derive(Clone, Copy)]
enum UnsafeConstruct {
    Backreferences,
    LookaroundAssertions,
    ConditionalGroups,
    InlineFlags,
    AtomicAndPossessiveGroups,
    WildcardRepetition,
}

impl UnsafeConstruct {
    const fn name(self) -> &'static str {
        match self {
            Self::Backreferences => "backreferences",
            Self::LookaroundAssertions => "lookaround assertions",
            Self::ConditionalGroups => "conditional groups",
            Self::InlineFlags => "inline flags",
            Self::AtomicAndPossessiveGroups => "atomic and possessive groups",
            Self::WildcardRepetition => "wildcard repetition",
        }
    }
}

/// The constructs outside the admitted subset, in the Python order.
const UNSAFE_CONSTRUCTS: [UnsafeConstruct; 6] = [
    UnsafeConstruct::Backreferences,
    UnsafeConstruct::LookaroundAssertions,
    UnsafeConstruct::ConditionalGroups,
    UnsafeConstruct::InlineFlags,
    UnsafeConstruct::AtomicAndPossessiveGroups,
    UnsafeConstruct::WildcardRepetition,
];

/// Whether the pattern uses a construct outside the admitted subset.
fn uses_unsafe_construct(pattern: &[u32], construct: UnsafeConstruct) -> bool {
    match construct {
        UnsafeConstruct::Backreferences => {
            for index in 0..pattern.len() {
                if pattern[index] != u32::from(b'\\') {
                    continue;
                }
                let Some(next) = pattern.get(index + 1) else {
                    continue;
                };
                if (u32::from(b'1')..=u32::from(b'9')).contains(next) {
                    return true;
                }
                if (*next == u32::from(b'k') || *next == u32::from(b'g'))
                    && pattern.get(index + 2) == Some(&u32::from(b'<'))
                {
                    return true;
                }
            }
            contains_sequence(pattern, b"(?P=")
        }
        UnsafeConstruct::LookaroundAssertions => {
            contains_sequence(pattern, b"(?=")
                || contains_sequence(pattern, b"(?!")
                || contains_sequence(pattern, b"(?<=")
                || contains_sequence(pattern, b"(?<!")
        }
        UnsafeConstruct::ConditionalGroups => contains_sequence(pattern, b"(?("),
        UnsafeConstruct::InlineFlags => {
            for index in 0..pattern.len() {
                if !matches_bytes(pattern, index, b"(?") {
                    continue;
                }
                let mut cursor = index + 2;
                let letters = count_ascii_letters(pattern, cursor);
                if letters == 0 {
                    continue;
                }
                cursor += letters;
                if cursor < pattern.len() && pattern[cursor] == u32::from(b'-') {
                    let after = count_ascii_letters(pattern, cursor + 1);
                    if after > 0 {
                        cursor += 1 + after;
                    }
                }
                if cursor < pattern.len()
                    && (pattern[cursor] == u32::from(b')') || pattern[cursor] == u32::from(b':'))
                {
                    return true;
                }
            }
            false
        }
        UnsafeConstruct::AtomicAndPossessiveGroups => {
            if contains_sequence(pattern, b"(?>") {
                return true;
            }
            for index in 0..pattern.len() {
                if pattern[index] != u32::from(b'\\') {
                    continue;
                }
                let Some(next) = pattern.get(index + 1).copied() else {
                    continue;
                };
                if next == u32::from(b'+') || next == u32::from(b'*') {
                    return true;
                }
            }
            false
        }
        UnsafeConstruct::WildcardRepetition => {
            for index in 0..pattern.len() {
                if pattern[index] != u32::from(b'.') {
                    continue;
                }
                let mut cursor = index + 1;
                while cursor < pattern.len() && is_space(pattern[cursor]) {
                    cursor += 1;
                }
                if cursor >= pattern.len() {
                    continue;
                }
                if pattern[cursor] == u32::from(b'*') || pattern[cursor] == u32::from(b'+') {
                    return true;
                }
                if pattern[cursor] == u32::from(b'{') {
                    let mut digits = cursor + 1;
                    while digits < pattern.len() && is_decimal(pattern[digits]) {
                        digits += 1;
                    }
                    if digits > cursor + 1 && pattern.get(digits) == Some(&u32::from(b',')) {
                        return true;
                    }
                }
            }
            false
        }
    }
}

/// Whether the pattern contains the given byte sequence.
fn contains_sequence(pattern: &[u32], sequence: &[u8]) -> bool {
    if sequence.is_empty() || pattern.len() < sequence.len() {
        return false;
    }
    for start in 0..=(pattern.len() - sequence.len()) {
        if sequence
            .iter()
            .enumerate()
            .all(|(offset, byte)| pattern[start + offset] == u32::from(*byte))
        {
            return true;
        }
    }
    false
}

/// Whether the pattern has the given bytes at `index`.
fn matches_bytes(pattern: &[u32], index: usize, expected: &[u8]) -> bool {
    expected
        .iter()
        .enumerate()
        .all(|(offset, byte)| pattern.get(index + offset) == Some(&u32::from(*byte)))
}

/// The number of ASCII letters at `index`.
fn count_ascii_letters(pattern: &[u32], index: usize) -> usize {
    let mut count = 0usize;
    while index + count < pattern.len() {
        let code_point = pattern[index + count];
        if !(u32::from(b'a')..=u32::from(b'z')).contains(&code_point)
            && !(u32::from(b'A')..=u32::from(b'Z')).contains(&code_point)
        {
            break;
        }
        count += 1;
    }
    count
}

/// CPython's `str.isspace()`.
fn is_space(code_point: u32) -> bool {
    engine::is_space(code_point)
}

/// The index just past a quantifier at `index`, or `None`.
fn quantifier_at(pattern: &[u32], index: usize) -> Option<usize> {
    let character = pattern.get(index).copied()?;
    if character == u32::from(b'*') || character == u32::from(b'+') {
        return Some(index + 1);
    }
    if character == u32::from(b'?') {
        let previous = if index == 0 {
            None
        } else {
            pattern.get(index - 1).copied()
        };
        if previous != Some(u32::from(b'?')) && previous != Some(u32::from(b':')) {
            return Some(index + 1);
        }
        return None;
    }
    if character == u32::from(b'{') {
        if let Some(end) = repeat_spec_end(pattern, index) {
            return Some(end);
        }
    }
    None
}

/// The end index of a `{n}`, `{n,}` or `{n,m}` repetition at `index`.
///
/// The repetition specification accepts one to nine digits, exactly like the
/// Python module's `_PATTERN_REPEAT_SPEC`.
fn repeat_spec_end(pattern: &[u32], index: usize) -> Option<usize> {
    let mut cursor = index + 1;
    let digits_start = cursor;
    while cursor < pattern.len() && (u32::from(b'0')..=u32::from(b'9')).contains(&pattern[cursor]) {
        cursor += 1;
        if cursor - digits_start > 9 {
            return None;
        }
    }
    if cursor == digits_start {
        return None;
    }
    let has_comma = pattern.get(cursor) == Some(&u32::from(b','));
    if has_comma {
        cursor += 1;
        let maximum_start = cursor;
        while cursor < pattern.len()
            && (u32::from(b'0')..=u32::from(b'9')).contains(&pattern[cursor])
        {
            cursor += 1;
            if cursor - maximum_start > 9 {
                return None;
            }
        }
    }
    if pattern.get(cursor) != Some(&u32::from(b'}')) {
        return None;
    }
    Some(cursor + 1)
}

/// The end index of a quantifier, refusing a possessive modifier.
fn modifier_end(pattern: &[u32], index: usize, path: &str) -> Result<usize, ToolProtocolError> {
    let marker = pattern.get(index).copied();
    if marker == Some(u32::from(b'+')) {
        return Err(ToolProtocolError::schema(
            "pattern uses a possessive quantifier",
            path,
        ));
    }
    if marker == Some(u32::from(b'?')) {
        return Ok(index + 1);
    }
    Ok(index)
}

/// The width, upper bound and end of the quantifier at `index`.
fn repeated(
    pattern: &[u32],
    index: usize,
    width: i128,
    path: &str,
) -> Result<Option<(i128, Option<i128>, usize)>, ToolProtocolError> {
    let Some(quantifier) = quantifier_at(pattern, index) else {
        return Ok(None);
    };
    let character = pattern[index];
    if character == u32::from(b'*') || character == u32::from(b'+') {
        return Ok(Some((
            width,
            None,
            modifier_end(pattern, quantifier, path)?,
        )));
    }
    if character == u32::from(b'?') {
        return Ok(Some((
            width,
            Some(1),
            modifier_end(pattern, quantifier, path)?,
        )));
    }
    let Some(end) = repeat_spec_end(pattern, index) else {
        return Err(invalid_pattern(path));
    };
    let body = &pattern[index + 1..end - 1];
    let (minimum, maximum) = if body.contains(&u32::from(b',')) {
        let comma = body
            .iter()
            .position(|value| *value == u32::from(b','))
            .unwrap_or(0);
        let minimum: i128 = digits_to_i128(&body[..comma]);
        let maximum_text = &body[comma + 1..];
        if maximum_text.is_empty() {
            (minimum, None)
        } else {
            (minimum, Some(digits_to_i128(maximum_text)))
        }
    } else {
        let minimum: i128 = digits_to_i128(body);
        (minimum, Some(minimum))
    };
    if let Some(maximum) = maximum {
        if maximum < minimum {
            return Err(invalid_pattern(path));
        }
    }
    Ok(Some((
        width,
        maximum,
        modifier_end(pattern, quantifier, path)?,
    )))
}

/// The value of a run of decimal digits.
fn digits_to_i128(digits: &[u32]) -> i128 {
    let mut value = 0i128;
    for digit in digits {
        value = value
            .saturating_mul(10)
            .saturating_add(i128::from(digit - u32::from(b'0')));
    }
    value
}

/// The "pattern is not a valid regular expression" error.
fn invalid_pattern(path: &str) -> ToolProtocolError {
    ToolProtocolError::schema("pattern is not a valid regular expression", path)
}

/// Refuse a pattern that repeats a group whose body already has a quantifier.
fn reject_nested_repetition(pattern: &[u32], path: &str) -> Result<(), ToolProtocolError> {
    let mut open_groups: Vec<bool> = Vec::new();
    let mut index = 0usize;
    let length = pattern.len();
    while index < length {
        let character = pattern[index];
        if character == u32::from(b'\\') {
            index += 2;
            continue;
        }
        if character == u32::from(b'[') {
            index += 1;
            while index < length && pattern[index] != u32::from(b']') {
                index += if pattern[index] == u32::from(b'\\') {
                    2
                } else {
                    1
                };
            }
            index += 1;
            continue;
        }
        if character == u32::from(b'(') {
            open_groups.push(false);
            index += if matches_bytes(pattern, index, b"(?:") {
                3
            } else {
                1
            };
            continue;
        }
        if character == u32::from(b')') {
            if open_groups.is_empty() {
                return Err(ToolProtocolError::schema(
                    "pattern has an unbalanced group",
                    path,
                ));
            }
            let body_is_quantified = open_groups.pop().unwrap_or(false);
            index += 1;
            let Some(quantified) = quantifier_at(pattern, index) else {
                continue;
            };
            if body_is_quantified {
                return Err(ToolProtocolError::schema(
                    "pattern repeats a group that already contains a quantifier",
                    path,
                ));
            }
            if let Some(last) = open_groups.last_mut() {
                *last = true;
            }
            index = quantified;
            continue;
        }
        if !open_groups.is_empty() && quantifier_at(pattern, index).is_some() {
            if let Some(last) = open_groups.last_mut() {
                *last = true;
            }
        }
        index += 1;
    }
    if !open_groups.is_empty() {
        return Err(ToolProtocolError::schema(
            "pattern has an unbalanced group",
            path,
        ));
    }
    Ok(())
}

/// The start of the body of the group at `index`.
fn group_body_start(pattern: &[u32], index: usize, path: &str) -> Result<usize, ToolProtocolError> {
    if pattern.get(index + 1).copied() != Some(u32::from(b'?')) {
        return Ok(index + 1);
    }
    if pattern.get(index + 2).copied() == Some(u32::from(b':')) {
        return Ok(index + 3);
    }
    if pattern.get(index + 2).copied() == Some(u32::from(b'P'))
        && pattern.get(index + 3).copied() == Some(u32::from(b'<'))
    {
        let mut cursor = index + 4;
        while cursor < pattern.len() && pattern[cursor] != u32::from(b'>') {
            cursor += 1;
        }
        if cursor >= pattern.len() {
            return Err(invalid_pattern(path));
        }
        return Ok(cursor + 1);
    }
    Err(ToolProtocolError::schema(
        "pattern uses a group construct outside the admitted subset",
        path,
    ))
}

/// The end of an escaped atom at `index`.
fn escaped_atom_end(pattern: &[u32], index: usize, path: &str) -> Result<usize, ToolProtocolError> {
    if index + 1 >= pattern.len() {
        return Err(invalid_pattern(path));
    }
    Ok(index + 2)
}

/// The end of a character class at `index`.
fn character_class_end(
    pattern: &[u32],
    index: usize,
    path: &str,
) -> Result<usize, ToolProtocolError> {
    let mut cursor = index + 1;
    if pattern.get(cursor).copied() == Some(u32::from(b'^')) {
        cursor += 1;
    }
    if pattern.get(cursor).copied() == Some(u32::from(b']')) {
        cursor += 1;
    }
    while cursor < pattern.len() && pattern[cursor] != u32::from(b']') {
        cursor += if pattern[cursor] == u32::from(b'\\') {
            2
        } else {
            1
        };
    }
    if cursor >= pattern.len() {
        return Err(invalid_pattern(path));
    }
    Ok(cursor + 1)
}

/// Refuse a pattern that can be matched in more than one bounded way.
fn reject_ambiguous_pattern(pattern: &[u32], path: &str) -> Result<(), ToolProtocolError> {
    let mut scopes: Vec<PatternScope> = Vec::new();
    let mut budget = PatternBudget::new();
    let mut quantified = false;
    let mut index = 0usize;
    let length = pattern.len();
    while index < length {
        let character = pattern[index];
        if character == u32::from(b'(') {
            scopes.push(PatternScope::default());
            index = group_body_start(pattern, index, path)?;
            quantified = false;
            continue;
        }
        if character == u32::from(b')') {
            let Some(scope) = scopes.pop() else {
                return Err(ToolProtocolError::schema(
                    "pattern has an unbalanced group",
                    path,
                ));
            };
            let repeated = repeated(pattern, index + 1, scope.width, path)?;
            let Some((width, maximum, end)) = repeated else {
                index += 1;
                quantified = false;
                continue;
            };
            if scope.atoms == 0 {
                return Err(ToolProtocolError::schema(
                    "pattern repeats an empty group",
                    path,
                ));
            }
            if scope.alternative {
                return Err(ToolProtocolError::schema(
                    "pattern repeats a group whose body has alternatives",
                    path,
                ));
            }
            if quantified {
                return Err(ToolProtocolError::schema(
                    "pattern repeats adjacent atoms",
                    path,
                ));
            }
            budget.repeat(width, maximum, path)?;
            index = end;
            quantified = true;
            continue;
        }
        if character == u32::from(b'|') {
            if let Some(scope) = scopes.last_mut() {
                scope.alternative = true;
            }
            index += 1;
            quantified = false;
            continue;
        }
        if character == u32::from(b'^') || character == u32::from(b'$') {
            index += 1;
            quantified = false;
            continue;
        }
        if character == u32::from(b'\\') {
            index = escaped_atom_end(pattern, index, path)?;
        } else if character == u32::from(b'[') {
            index = character_class_end(pattern, index, path)?;
        } else if character == u32::from(b'.') {
            index += 1;
        } else if character == u32::from(b'*')
            || character == u32::from(b'+')
            || character == u32::from(b'?')
            || repeat_spec_end(pattern, index).is_some()
        {
            return Err(invalid_pattern(path));
        } else {
            index += 1;
        }
        if let Some(scope) = scopes.last_mut() {
            scope.atoms += 1;
            scope.width += 1;
        }
        let repeated = repeated(pattern, index, 1, path)?;
        match repeated {
            None => {
                budget.literal(1, path)?;
                quantified = false;
            }
            Some((width, maximum, end)) => {
                if quantified {
                    return Err(ToolProtocolError::schema(
                        "pattern repeats adjacent atoms",
                        path,
                    ));
                }
                budget.repeat(width, maximum, path)?;
                index = end;
                quantified = true;
            }
        }
    }
    if !scopes.is_empty() {
        return Err(ToolProtocolError::schema(
            "pattern has an unbalanced group",
            path,
        ));
    }
    Ok(())
}

/// Compile a pattern that cannot be made to run long on a bounded subject.
///
/// Admission is decided from the pattern's own text, before anything is
/// compiled and before the pattern is ever run against a value, and the same
/// decision is reached on every call: the schema must cap the string, the
/// pattern must be short, and the pattern must fit a bounded backtracking
/// budget. A refused pattern is never compiled and never executed, so a tool
/// author cannot buy an unbounded regular expression by registering a hostile
/// schema.
pub(crate) fn bounded_pattern(
    pattern: &[u32],
    maximum_length: Option<i128>,
    path: &str,
) -> Result<Pattern, ToolProtocolError> {
    let Some(maximum_length) = maximum_length else {
        return Err(ToolProtocolError::schema(
            "pattern requires a maxLength bound",
            path,
        ));
    };
    if maximum_length <= 0 || maximum_length > MAX_PATTERN_INPUT_LENGTH as i128 {
        return Err(ToolProtocolError::schema(
            format!("pattern requires a maxLength between 1 and {MAX_PATTERN_INPUT_LENGTH}"),
            path,
        ));
    }
    if pattern.len() > MAX_PATTERN_LENGTH {
        return Err(ToolProtocolError::schema(
            "pattern exceeds the bounded length",
            path,
        ));
    }
    for construct in UNSAFE_CONSTRUCTS {
        if uses_unsafe_construct(pattern, construct) {
            return Err(ToolProtocolError::schema(
                format!("pattern uses an unsafe construct: {}", construct.name()),
                path,
            ));
        }
    }
    reject_nested_repetition(pattern, path)?;
    reject_ambiguous_pattern(pattern, path)?;
    Pattern::compile(pattern).map_err(|_| invalid_pattern(path))
}
