//! Deterministic offline local chat provider.
//!
//! This is a line-for-line port of `kala_agent.studio.providers.local`:
//! the same keyword/suite tables, the same regex grammar (including the
//! lookbehind behaviour implemented explicitly), the same float operation
//! order, the same CPython `:.3f`/`:.3g` formatting and the same 1024-code-
//! point response cap. The result is byte-identical for the oracle vectors
//! in `tests/data/local_chat_vectors.json`.

use std::sync::LazyLock;

use regex::Regex;
use sha2::{Digest, Sha256};

use super::{ChatOutcome, ProviderFailure};
use crate::pytext;

pub const DEFAULT_LOCAL_MODEL: &str = "deterministic-paraphrase-v1";
pub const MAX_PROMPT_LENGTH: usize = 4_096;
pub const MAX_RESPONSE_LENGTH: usize = 1_024;

const NUMBER_PATTERN: &str = r"\d+(?:\.\d+)?";
const SPACE_CLASS: &str = r"[\s\x1c-\x1f]";
const UNIT_PATTERN: &str = r"(mm|cm|m)?";

static TRIPLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"({NUMBER_PATTERN}){SPACE_CLASS}*(?:x|×|\*|by){SPACE_CLASS}*({NUMBER_PATTERN}){SPACE_CLASS}*(?:x|×|\*|by){SPACE_CLASS}*({NUMBER_PATTERN}){SPACE_CLASS}*{UNIT_PATTERN}"
    ))
    .expect("triple regex")
});

static NUMBER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"({NUMBER_PATTERN}){SPACE_CLASS}*{UNIT_PATTERN}")).expect("number regex")
});

const PRIMITIVE_KEYWORDS: &[(&str, &str)] = &[
    ("box", "box"),
    ("block", "box"),
    ("bracket", "box"),
    ("plate", "box"),
    ("cylinder", "cylinder"),
    ("shaft", "cylinder"),
    ("pin", "cylinder"),
    ("rod", "cylinder"),
    ("pipe", "cylinder"),
    ("sphere", "sphere"),
    ("ball", "sphere"),
    ("bearing", "sphere"),
];

const SUITE_NAMES: &[(&str, &str)] = &[
    ("solidworks", "SOLIDWORKS"),
    ("solid edge", "Solid Edge"),
    ("solidedge", "Solid Edge"),
    ("autocad", "AutoCAD"),
    ("fusion 360", "Fusion 360"),
    ("fusion360", "Fusion 360"),
    ("freecad", "FreeCAD"),
    ("openscad", "OpenSCAD"),
    ("ansys", "ANSYS Mechanical"),
    ("abaqus", "Abaqus"),
];

/// Return a deterministic offline mechanical-engineering reply.
///
/// # Errors
///
/// Returns a sanitized `ProviderProtocolError` equivalent when the prompt is
/// longer than the provider's bounded length.
pub fn local_chat(prompt: &str, model: Option<&str>) -> Result<ChatOutcome, ProviderFailure> {
    if prompt.chars().count() > MAX_PROMPT_LENGTH {
        return Err(ProviderFailure::Protocol(
            "prompt exceeds the local provider's bounded length".to_owned(),
        ));
    }
    let mut summary = first_sentences(prompt);
    if summary.is_empty() {
        summary = "(empty prompt)".to_owned();
    }
    let sections = [
        reasoning_hint(prompt),
        estimate(prompt),
        opencascade_note(prompt),
        summary,
    ];
    let joined = sections
        .into_iter()
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let body: String = joined.chars().take(MAX_RESPONSE_LENGTH).collect();
    Ok(ChatOutcome::new(
        body,
        model
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_LOCAL_MODEL)
            .to_owned(),
        true,
    ))
}

fn normalise(text: &str) -> String {
    let trimmed = pytext::trim(text);
    let mut out = String::with_capacity(trimmed.len());
    let mut in_space = false;
    for character in trimmed.chars() {
        if pytext::is_space(character) {
            if !in_space {
                out.push(' ');
                in_space = true;
            }
        } else {
            out.push(character);
            in_space = false;
        }
    }
    out
}

fn first_sentences(text: &str) -> String {
    let cleaned = normalise(text);
    if cleaned.is_empty() {
        return String::new();
    }
    let characters: Vec<char> = cleaned.chars().collect();
    let mut pieces = Vec::new();
    let mut cursor = 0usize;
    while cursor < characters.len() && pieces.len() < 3 {
        let mut chosen = None;
        for marker in ['.', '!', '?'] {
            if let Some(offset) = characters[cursor..]
                .iter()
                .position(|character| *character == marker)
            {
                chosen = Some(cursor + offset);
                break;
            }
        }
        match chosen {
            Some(position) => {
                let piece: String = characters[cursor..=position].iter().collect();
                pieces.push(piece.trim().to_owned());
                cursor = position + 1;
            }
            None => {
                let piece: String = characters[cursor..].iter().collect();
                pieces.push(piece.trim().to_owned());
                cursor = characters.len();
            }
        }
    }
    let summary = pieces
        .into_iter()
        .filter(|piece| !piece.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    summary.chars().take(MAX_RESPONSE_LENGTH).collect()
}

fn reasoning_hint(prompt: &str) -> String {
    let cleaned = normalise(prompt);
    let digest = Sha256::digest(cleaned.as_bytes());
    let mut fingerprint = String::with_capacity(8);
    for byte in digest.iter().take(4) {
        fingerprint.push_str(&format!("{byte:02x}"));
    }
    format!(
        "Local helper. Reasoning trace id {fingerprint}. Deterministic offline mechanical-engineering helper; no network egress, no solver runs and no fabricated results."
    )
}

fn to_mm(value: f64, unit: Option<&str>) -> f64 {
    match unit.unwrap_or("mm") {
        "cm" => value * 10.0,
        "m" => value * 1_000.0,
        _ => value,
    }
}

/// CPython `float()` over a decimal token. Unicode decimal digits are
/// accepted by CPython, so map them to ASCII before parsing.
fn parse_python_float(text: &str) -> Option<f64> {
    let mut ascii = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_ascii_digit() || character == '.' {
            ascii.push(character);
        } else {
            let digit = character.to_digit(10)?;
            ascii.push(char::from(b'0' + digit as u8));
        }
    }
    ascii.parse::<f64>().ok()
}

fn format_number(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-inf"
        } else {
            "inf"
        }
        .to_owned();
    }
    let magnitude = value.abs();
    if magnitude != 0.0 && (magnitude >= 1e15 || magnitude < 1e-6) {
        return format_general(value, 3);
    }
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

/// CPython `format(value, ".3g")` for a finite non-zero value.
fn format_general(value: f64, precision: usize) -> String {
    let negative = value.is_sign_negative();
    let magnitude = value.abs();
    let rendered = format!("{:.*e}", precision.saturating_sub(1), magnitude);
    let (mantissa, exponent_text) = rendered.split_once('e').unwrap_or((&rendered, "0"));
    let exponent: i32 = exponent_text.parse().unwrap_or(0);
    let mut digits: String = mantissa
        .chars()
        .filter(|character| *character != '.')
        .collect();
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    let sign = if negative { "-" } else { "" };
    if exponent < -4 || exponent >= precision as i32 {
        let mut output = String::new();
        output.push_str(sign);
        output.push(digits.as_bytes()[0] as char);
        if digits.len() > 1 {
            output.push('.');
            output.push_str(&digits[1..]);
        }
        output.push('e');
        if exponent < 0 {
            output.push('-');
        } else {
            output.push('+');
        }
        let magnitude = exponent.unsigned_abs();
        if magnitude < 10 {
            output.push('0');
        }
        output.push_str(&magnitude.to_string());
        return output;
    }
    let decimal_position = exponent + 1;
    let mut output = String::new();
    output.push_str(sign);
    if decimal_position <= 0 {
        output.push_str("0.");
        for _ in 0..(-decimal_position) {
            output.push('0');
        }
        output.push_str(&digits);
    } else {
        let position = decimal_position as usize;
        if position >= digits.len() {
            output.push_str(&digits);
            for _ in digits.len()..position {
                output.push('0');
            }
        } else {
            output.push_str(&digits[..position]);
            output.push('.');
            output.push_str(&digits[position..]);
        }
    }
    if output.contains('.') {
        while output.ends_with('0') {
            output.pop();
        }
        if output.ends_with('.') {
            output.pop();
        }
    }
    output
}

fn detect_kind(lowered: &str) -> Option<&'static str> {
    for (keyword, kind) in PRIMITIVE_KEYWORDS {
        if contains_word(lowered, keyword) {
            return Some(kind);
        }
    }
    None
}

fn contains_word(text: &str, word: &str) -> bool {
    let mut start = 0;
    while let Some(offset) = text[start..].find(word) {
        let position = start + offset;
        let before = text[..position].chars().next_back();
        let after = text[position + word.len()..].chars().next();
        let before_word = before.is_some_and(is_word_character);
        let after_word = after.is_some_and(is_word_character);
        if !before_word && !after_word {
            return true;
        }
        start = position + word.len();
    }
    false
}

fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn detect_suites(lowered: &str) -> Vec<&'static str> {
    let mut found = Vec::new();
    for (keyword, label) in SUITE_NAMES {
        if lowered.contains(keyword) && !found.contains(label) {
            found.push(*label);
        }
    }
    found
}

fn label_value(lowered: &str, labels: &[&str]) -> Option<f64> {
    for label in labels {
        let mut start = 0;
        while let Some(offset) = lowered[start..].find(label) {
            let position = start + offset;
            let before = lowered[..position].chars().next_back();
            if before.is_some_and(is_word_character) {
                start = position + label.len();
                continue;
            }
            let mut cursor = position + label.len();
            cursor = skip_whitespace(lowered, cursor);
            if let Some(character) = lowered[cursor..].chars().next() {
                if character == ':' || character == '=' {
                    cursor += character.len_utf8();
                    cursor = skip_whitespace(lowered, cursor);
                }
            }
            if let Some((number, unit, _end)) = scan_number_unit(lowered, cursor) {
                return Some(to_mm(number, unit));
            }
            start = position + label.len();
        }
    }
    None
}

fn skip_whitespace(text: &str, mut cursor: usize) -> usize {
    while let Some(character) = text[cursor..].chars().next() {
        if !pytext::is_space(character) {
            break;
        }
        cursor += character.len_utf8();
    }
    cursor
}

// Python's `\d` is Unicode-aware; the ASCII-only lint suggestion would
// silently narrow accepted prompts.
#[allow(clippy::is_digit_ascii_radix)]
fn scan_number_unit(text: &str, start: usize) -> Option<(f64, Option<&str>, usize)> {
    let mut cursor = start;
    let mut number = String::new();
    let mut saw_digit = false;
    while let Some(character) = text[cursor..].chars().next() {
        if character.is_digit(10) {
            number.push(character);
            saw_digit = true;
            cursor += character.len_utf8();
        } else {
            break;
        }
    }
    if !saw_digit {
        return None;
    }
    if text[cursor..].starts_with('.') {
        let after_dot = &text[cursor + 1..];
        if after_dot
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
        {
            number.push('.');
            cursor += 1;
            while let Some(character) = text[cursor..].chars().next() {
                if character.is_digit(10) {
                    number.push(character);
                    cursor += character.len_utf8();
                } else {
                    break;
                }
            }
        }
    }
    let value = parse_python_float(&number)?;
    cursor = skip_whitespace(text, cursor);
    let mut unit = None;
    for candidate in ["mm", "cm", "m"] {
        if text[cursor..].starts_with(candidate) {
            unit = Some(candidate);
            cursor += candidate.len();
            break;
        }
    }
    Some((value, unit, cursor))
}

#[allow(clippy::is_digit_ascii_radix)]
fn numbers(lowered: &str) -> Vec<f64> {
    let mut values = Vec::new();
    for captures in NUMBER_RE.captures_iter(lowered) {
        let Some(whole) = captures.get(0) else {
            continue;
        };
        let before = lowered[..whole.start()].chars().next_back();
        if before.is_some_and(|character| is_word_character(character) || character == '.') {
            continue;
        }
        let Some(number) = captures
            .get(1)
            .and_then(|value| parse_python_float(value.as_str()))
        else {
            continue;
        };
        values.push(to_mm(number, captures.get(2).map(|unit| unit.as_str())));
    }
    values
}

fn box_estimate(size_x: f64, size_y: f64, size_z: f64) -> String {
    let volume = size_x * size_y * size_z;
    let area = 2.0 * (size_x * size_y + size_y * size_z + size_z * size_x);
    let dims = format!(
        "{}x{}x{}",
        format_number(size_x),
        format_number(size_y),
        format_number(size_z)
    );
    format!(
        "Nominal analytic estimate (box {dims} mm): volume {} mm^3 ({} cm^3), surface area {} mm^2.",
        format_number(volume),
        format_number(volume / 1000.0),
        format_number(area)
    )
}

fn cylinder_estimate(radius: f64, height: f64) -> String {
    let volume = std::f64::consts::PI * radius * radius * height;
    let area = 2.0 * std::f64::consts::PI * radius * (radius + height);
    format!(
        "Nominal analytic estimate (cylinder r={} h={} mm): volume {} mm^3 ({} cm^3), surface area {} mm^2.",
        format_number(radius),
        format_number(height),
        format_number(volume),
        format_number(volume / 1000.0),
        format_number(area)
    )
}

fn sphere_estimate(radius: f64) -> String {
    let volume = 4.0 / 3.0 * std::f64::consts::PI * radius.powf(3.0);
    let area = 4.0 * std::f64::consts::PI * radius * radius;
    format!(
        "Nominal analytic estimate (sphere r={} mm): volume {} mm^3 ({} cm^3), surface area {} mm^2.",
        format_number(radius),
        format_number(volume),
        format_number(volume / 1000.0),
        format_number(area)
    )
}

fn estimate(prompt: &str) -> String {
    let lowered = prompt.to_lowercase();
    let kind = detect_kind(&lowered);

    if let Some(captures) = TRIPLE_RE.captures(&lowered) {
        let unit = captures.get(4).map(|value| value.as_str());
        let first = captures
            .get(1)
            .and_then(|value| parse_python_float(value.as_str()))
            .unwrap_or(f64::NAN);
        let second = captures
            .get(2)
            .and_then(|value| parse_python_float(value.as_str()))
            .unwrap_or(f64::NAN);
        let third = captures
            .get(3)
            .and_then(|value| parse_python_float(value.as_str()))
            .unwrap_or(f64::NAN);
        return box_estimate(to_mm(first, unit), to_mm(second, unit), to_mm(third, unit));
    }

    let size_x = label_value(&lowered, &["size_x", "size x", "x"]);
    let size_y = label_value(&lowered, &["size_y", "size y", "y"]);
    let size_z = label_value(&lowered, &["size_z", "size z", "z"]);
    if let (Some(size_x), Some(size_y), Some(size_z)) = (size_x, size_y, size_z) {
        return box_estimate(size_x, size_y, size_z);
    }

    let radius = label_value(&lowered, &["radius", "r"]);
    let height = label_value(&lowered, &["height", "h"]);
    let values = numbers(&lowered);

    if kind == Some("sphere") {
        let candidate = radius.or_else(|| values.first().copied());
        if let Some(candidate) = candidate {
            return sphere_estimate(candidate);
        }
    }
    if kind == Some("cylinder") {
        let candidate_radius = radius.or_else(|| values.first().copied());
        let candidate_height = height.or_else(|| values.get(1).copied());
        if let (Some(candidate_radius), Some(candidate_height)) =
            (candidate_radius, candidate_height)
        {
            return cylinder_estimate(candidate_radius, candidate_height);
        }
    }
    if kind == Some("box") && values.len() >= 3 {
        return box_estimate(values[0], values[1], values[2]);
    }
    if let (Some(radius), Some(height)) = (radius, height) {
        return cylinder_estimate(radius, height);
    }
    if let Some(radius) = radius {
        return sphere_estimate(radius);
    }
    if let Some(height) = height {
        if let Some(candidate) = values.first() {
            return cylinder_estimate(*candidate, height);
        }
    }
    if kind.is_none() && values.len() >= 3 {
        return box_estimate(values[0], values[1], values[2]);
    }
    String::new()
}

fn opencascade_note(prompt: &str) -> String {
    let suites = detect_suites(&prompt.to_lowercase());
    let mut note = "OpenCascade (OCP) workbench is active by default for quick geometry/envelope estimates and neutral STEP/STL handoff to external CAD/CAE, and for complete standalone STEP/STL builds inside Kala Studio. One shared workflow for every user, without tiers; FEA, CFD, stress and thermal solver numbers are never fabricated.".to_owned();
    if !suites.is_empty() {
        note.push_str(" Neutral STEP/STL exports are ready to import into ");
        note.push_str(&suites.join(", "));
        note.push_str("; the studio does not drive a native vendor API.");
    }
    note
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_matches_python_examples() {
        assert_eq!(format_number(0.0), "0");
        assert_eq!(format_number(-0.0), "-0");
        assert_eq!(format_number(10.0), "10");
        assert_eq!(format_number(123.4567), "123.457");
        assert_eq!(format_number(1e15), "1e+15");
        assert_eq!(format_number(999.9), "999.9");
        assert_eq!(format_number(0.0001), "0");
        assert_eq!(format_number(0.00001), "0");
        assert_eq!(format_number(1e-7), "1e-07");
    }

    #[test]
    fn first_sentences_preserves_python_marker_order() {
        assert_eq!(
            first_sentences("First! Second? Third. Fourth."),
            "First! Second? Third. Fourth."
        );
        assert_eq!(first_sentences("one. two"), "one. two");
        assert_eq!(first_sentences(""), "");
    }

    #[test]
    fn oracle_vectors_match_cpython_output() {
        let raw = include_str!("../../tests/data/local_chat_vectors.json");
        let vectors: serde_json::Value = serde_json::from_str(raw).unwrap();
        let mut failures = Vec::new();
        for vector in vectors.as_array().unwrap() {
            let prompt = vector["prompt"].as_str().unwrap();
            let request_model = vector["request_model"].as_str();
            let outcome = local_chat(prompt, request_model).unwrap();
            if outcome.text != vector["text"].as_str().unwrap()
                || outcome.model != vector["model"].as_str().unwrap()
                || outcome.offline != vector["offline"].as_bool().unwrap()
            {
                failures.push(format!(
                    "prompt={prompt:?} expected_text={:?} actual_text={:?}",
                    vector["text"].as_str().unwrap(),
                    outcome.text
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn lookbehind_rules_are_explicit() {
        assert_eq!(numbers("x10 20 30"), vec![20.0, 30.0]);
        assert_eq!(numbers("1.2.3"), vec![1.2]);
        assert_eq!(label_value("x=1 size_y: 2", &["x"]), Some(1.0));
        assert_eq!(label_value("size_x=1", &["x"]), None);
    }
}
