//! Credential-shaped text redaction.
//!
//! This is the port of the Python module's secret-redaction layer. The
//! Python implementation is a list of eight regular expressions applied in
//! order, each to the result of the previous one:
//!
//! 1. a PEM private key block,
//! 2. an `Authorization`/`Proxy-Authorization` header,
//! 3. an `sk-` prefixed key,
//! 4. a `Bearer` token,
//! 5. a GitHub token,
//! 6. an AWS access key id,
//! 7. a quoted `"secret name": "value"` pair,
//! 8. a `secret name = value` assignment.
//!
//! Five of the eight patterns have no look-around and are matched with the
//! `regex` crate. The remaining three use a look-behind, a look-ahead or
//! both; the `regex` crate has no look-around, so those are matched with the
//! explicit backtracking matcher below, which reproduces CPython's `re`
//! search order (leftmost start position, then the pattern's own
//! backtracking order) exactly.
//!
//! No credential ever reaches a log, a transcript or an error message through
//! this module: the replacements are the literal `<redacted>` marker.

use std::sync::LazyLock;

use kala_json::CanonicalValue;
use regex::Regex;

use super::text::{is_ascii_alphanumeric, is_python_whitespace};

/// The marker that replaces every redacted value.
pub const REDACTED: &str = "<redacted>";

static PEM_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?s)-----BEGIN(?: [A-Z0-9]+)* PRIVATE KEY-----.*?(?:-----END(?: [A-Z0-9]+)* PRIVATE KEY-----|\z)",
    )
    .expect("the PEM pattern compiles")
});

static OPENAI_KEY_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bsk-[A-Za-z0-9_-]{12,}").expect("the sk- pattern compiles"));

static BEARER_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bbearer\s+[A-Za-z0-9._-]{12,}").expect("the bearer pattern compiles")
});

static GITHUB_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bgh[pousr]_[A-Za-z0-9]{16,}").expect("the gh pattern compiles")
});

static AWS_KEY_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:AKIA|ASIA|AGPA|AIDA|AROA|ANPA)[0-9A-Z]{16}\b")
        .expect("the AWS key pattern compiles")
});

static AUTHORIZATION_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:proxy-)?authorization[ \t]*:[ \t]*([^\r\n]+)")
        .expect("the authorization pattern compiles")
});

// ---------------------------------------------------------------------------
// The secret-name matcher.
//
// The Python pattern is
//
// ```text
// (?<![A-Za-z0-9])(?:[A-Za-z0-9]+[_-])*(?:KEYS)(?![A-Za-z0-9])
// ```
//
// The look-behind and the look-ahead are the only parts the `regex` crate
// cannot express, so the matcher below enumerates the candidates at one
// start position in CPython's backtracking order: the longest prefix first,
// then the key alternatives in their declared order, then each alternative's
// own internal alternatives.
// ---------------------------------------------------------------------------

/// One piece of a secret key alternative.
#[derive(Clone, Copy)]
enum Part {
    /// A literal that must match exactly (ASCII case-insensitively).
    Literal(&'static str),
    /// An optional single byte from a set, greedy (present first).
    Optional(&'static [u8]),
}

const SEPARATOR: &[u8] = b"_-";
const QUOTES: &[u8] = b"\"'";

fn literal_matches(text: &[u8], pos: usize, literal: &str) -> bool {
    let bytes = literal.as_bytes();
    if text.len() < pos + bytes.len() {
        return false;
    }
    text[pos..pos + bytes.len()]
        .iter()
        .zip(bytes)
        .all(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn collect_parts(text: &[u8], pos: usize, parts: &[Part], out: &mut Vec<usize>) {
    let Some((head, rest)) = parts.split_first() else {
        out.push(pos);
        return;
    };
    match head {
        Part::Literal(literal) => {
            if literal_matches(text, pos, literal) {
                collect_parts(text, pos + literal.len(), rest, out);
            }
        }
        Part::Optional(set) => {
            if pos < text.len() && set.contains(&text[pos]) {
                collect_parts(text, pos + 1, rest, out);
            }
            collect_parts(text, pos, rest, out);
        }
    }
}

/// Every end offset at which `parts` matches, in CPython backtracking order.
fn match_parts(text: &[u8], pos: usize, parts: &[Part]) -> Vec<usize> {
    let mut out = Vec::new();
    collect_parts(text, pos, parts, &mut out);
    out
}

/// The end offsets of every secret-key alternative, in the declared order.
fn key_candidates(text: &[u8], pos: usize) -> Vec<usize> {
    let mut out = Vec::new();
    // aws[_-]?(?:secret[_-]?access[_-]?key|access[_-]?key[_-]?id|session[_-]?token)
    for outer in match_parts(
        text,
        pos,
        &[Part::Literal("aws"), Part::Optional(SEPARATOR)],
    ) {
        for inner in [
            &[
                Part::Literal("secret"),
                Part::Optional(SEPARATOR),
                Part::Literal("access"),
                Part::Optional(SEPARATOR),
                Part::Literal("key"),
            ][..],
            &[
                Part::Literal("access"),
                Part::Optional(SEPARATOR),
                Part::Literal("key"),
                Part::Optional(SEPARATOR),
                Part::Literal("id"),
            ][..],
            &[
                Part::Literal("session"),
                Part::Optional(SEPARATOR),
                Part::Literal("token"),
            ][..],
        ] {
            out.extend(match_parts(text, outer, inner));
        }
    }
    out.extend(match_parts(
        text,
        pos,
        &[
            Part::Literal("api"),
            Part::Optional(SEPARATOR),
            Part::Literal("key"),
        ],
    ));
    out.extend(match_parts(
        text,
        pos,
        &[
            Part::Literal("api"),
            Part::Optional(SEPARATOR),
            Part::Literal("secret"),
        ],
    ));
    out.extend(match_parts(
        text,
        pos,
        &[
            Part::Literal("client"),
            Part::Optional(SEPARATOR),
            Part::Literal("secret"),
        ],
    ));
    out.extend(match_parts(
        text,
        pos,
        &[
            Part::Literal("secret"),
            Part::Optional(SEPARATOR),
            Part::Literal("key"),
        ],
    ));
    out.extend(match_parts(
        text,
        pos,
        &[
            Part::Literal("private"),
            Part::Optional(SEPARATOR),
            Part::Literal("key"),
        ],
    ));
    out.extend(match_parts(
        text,
        pos,
        &[
            Part::Literal("access"),
            Part::Optional(SEPARATOR),
            Part::Literal("key"),
        ],
    ));
    out.extend(match_parts(text, pos, &[Part::Literal("passphrase")]));
    out.extend(match_parts(text, pos, &[Part::Literal("password")]));
    out.extend(match_parts(text, pos, &[Part::Literal("passwd")]));
    out.extend(match_parts(text, pos, &[Part::Literal("secret")]));
    out.extend(match_parts(text, pos, &[Part::Literal("token")]));
    out.extend(match_parts(
        text,
        pos,
        &[Part::Literal("credential"), Part::Optional(b"s")],
    ));
    out.extend(match_parts(text, pos, &[Part::Literal("authorization")]));
    out
}

/// The end offsets of the greedy `(?:[A-Za-z0-9]+[_-])*` prefix.
fn prefix_segment_ends(text: &[u8], pos: usize) -> Vec<usize> {
    let mut ends = vec![pos];
    let mut current = pos;
    loop {
        let mut run_end = current;
        while run_end < text.len() && text[run_end].is_ascii_alphanumeric() {
            run_end += 1;
        }
        if run_end == current {
            break;
        }
        if run_end < text.len() && (text[run_end] == b'_' || text[run_end] == b'-') {
            current = run_end + 1;
            ends.push(current);
        } else {
            break;
        }
    }
    ends
}

/// Every secret-name match that starts at `pos`, in CPython backtracking
/// order. The trailing look-ahead is applied to every candidate.
fn name_candidates(text: &[u8], pos: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let ends = prefix_segment_ends(text, pos);
    for index in (0..ends.len()).rev() {
        for candidate in key_candidates(text, ends[index]) {
            if candidate == text.len() || !is_ascii_alphanumeric(text[candidate]) {
                out.push(candidate);
            }
        }
    }
    out
}

/// The leftmost secret-name match at or after `start`, with every candidate
/// that starts there.
fn find_name(text: &[u8], start: usize) -> Option<(usize, Vec<usize>)> {
    let mut position = start;
    while position <= text.len() {
        if position < text.len() && !is_ascii_alphanumeric(text[position]) {
            position += 1;
            continue;
        }
        if position > 0 && is_ascii_alphanumeric(text[position - 1]) {
            position += 1;
            continue;
        }
        let candidates = name_candidates(text, position);
        if !candidates.is_empty() {
            return Some((position, candidates));
        }
        position += 1;
    }
    None
}

/// Whether the value names a credential-shaped key.
#[must_use]
pub fn is_secret_name(value: &str) -> bool {
    find_name(value.as_bytes(), 0).is_some()
}

// ---------------------------------------------------------------------------
// The assignment matcher.
//
// The Python pattern is
//
// ```text
// NAME ["']? \s*[:=]\s* ["']? (?!<redacted>) [^\s"',;]+ ["']?
// ```
//
// The look-ahead refuses a value that is already the redaction marker.
// ---------------------------------------------------------------------------

fn whitespace_run(text: &str, pos: usize) -> usize {
    let mut end = pos;
    for (offset, character) in text[pos..].char_indices() {
        if !is_python_whitespace(character) {
            break;
        }
        end = pos + offset + character.len_utf8();
    }
    end
}

fn starts_with_marker_ignore_case(text: &str, pos: usize) -> bool {
    let marker = REDACTED.as_bytes();
    if text.len() < pos + marker.len() {
        return false;
    }
    text.as_bytes()[pos..pos + marker.len()]
        .iter()
        .zip(marker)
        .all(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn is_value_stop(character: char) -> bool {
    character == '"'
        || character == '\''
        || character == ','
        || character == ';'
        || is_python_whitespace(character)
}

/// Every end offset at which the assignment matches, in CPython backtracking
/// order.
fn assignment_candidates(text: &str, pos: usize) -> Vec<usize> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    for outer_quote in [true, false] {
        let mut cursor = pos;
        if outer_quote {
            if cursor < bytes.len() && QUOTES.contains(&bytes[cursor]) {
                cursor += 1;
            } else {
                continue;
            }
        }
        cursor = whitespace_run(text, cursor);
        if cursor >= bytes.len() || (bytes[cursor] != b':' && bytes[cursor] != b'=') {
            continue;
        }
        cursor += 1;
        cursor = whitespace_run(text, cursor);
        for value_quote in [true, false] {
            let mut start = cursor;
            if value_quote {
                if start < bytes.len() && QUOTES.contains(&bytes[start]) {
                    start += 1;
                } else {
                    continue;
                }
            }
            if starts_with_marker_ignore_case(text, start) {
                continue;
            }
            let mut end = start;
            for (offset, character) in text[start..].char_indices() {
                if is_value_stop(character) {
                    break;
                }
                end = start + offset + character.len_utf8();
            }
            if end == start {
                continue;
            }
            if end < bytes.len() && QUOTES.contains(&bytes[end]) {
                out.push(end + 1);
            } else {
                out.push(end);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The quoted-pair matcher.
//
// The Python pattern is
//
// ```text
// " NAME " \s*:\s* " (?!<redacted>") (?:[^"\\]|\\.)+ "
// ```
//
// The name boundaries are implied by the surrounding quotes, so only the
// value look-ahead needs explicit handling.
// ---------------------------------------------------------------------------

fn scan_quoted_value(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = start;
    let mut elements = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'"' {
            return if elements > 0 { Some(index) } else { None };
        }
        if byte == b'\\' {
            let next = text[index + 1..].chars().next();
            match next {
                Some(character) if character != '\n' => {
                    index += 1 + character.len_utf8();
                    elements += 1;
                }
                _ => return None,
            }
        } else {
            let character = text[index..].chars().next()?;
            index += character.len_utf8();
            elements += 1;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// The redaction passes.
// ---------------------------------------------------------------------------

fn replace_all(text: &str, pattern: &Regex, findings: &mut usize) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    for capture in pattern.captures_iter(text) {
        let whole = capture.get(0).expect("a whole match");
        out.push_str(&text[last..whole.start()]);
        out.push_str(REDACTED);
        last = whole.end();
        *findings += 1;
    }
    out.push_str(&text[last..]);
    out
}

fn redact_authorization(text: &str, findings: &mut usize) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    let mut search_start = 0usize;
    while search_start <= text.len() {
        let Some(capture) = AUTHORIZATION_PATTERN.captures_at(text, search_start) else {
            break;
        };
        let whole = capture.get(0).expect("a whole match");
        let value = capture.get(1).expect("the header value");
        if starts_with_marker_ignore_case(text, value.start()) {
            // The Python look-ahead refuses an already redacted value; the
            // search continues one character later so a later header on the
            // same line is still considered.
            search_start = whole.start() + 1;
            continue;
        }
        out.push_str(&text[last..value.start()]);
        out.push_str(REDACTED);
        last = value.end();
        *findings += 1;
        search_start = whole.end();
    }
    out.push_str(&text[last..]);
    out
}

fn redact_quoted_pairs(text: &str, findings: &mut usize) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    let mut search_start = 0usize;
    while search_start < bytes.len() {
        let Some(quote) = bytes[search_start..]
            .iter()
            .position(|byte| *byte == b'"')
            .map(|offset| search_start + offset)
        else {
            break;
        };
        let mut matched = None;
        for name_end in name_candidates(bytes, quote + 1) {
            if bytes.get(name_end) != Some(&b'"') {
                continue;
            }
            let mut cursor = name_end + 1;
            cursor = whitespace_run(text, cursor);
            if bytes.get(cursor) != Some(&b':') {
                continue;
            }
            cursor += 1;
            cursor = whitespace_run(text, cursor);
            if bytes.get(cursor) != Some(&b'"') {
                continue;
            }
            let value_start = cursor + 1;
            if starts_with_marker_ignore_case(text, value_start)
                && bytes.get(value_start + REDACTED.len()) == Some(&b'"')
            {
                continue;
            }
            let Some(value_end) = scan_quoted_value(text, value_start) else {
                continue;
            };
            matched = Some((value_start, value_end));
            break;
        }
        let Some((value_start, value_end)) = matched else {
            search_start = quote + 1;
            continue;
        };
        out.push_str(&text[last..value_start]);
        out.push_str(REDACTED);
        out.push('"');
        last = value_end + 1;
        *findings += 1;
        search_start = last;
    }
    out.push_str(&text[last..]);
    out
}

fn redact_assignments(text: &str, findings: &mut usize) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    let mut search_start = 0usize;
    while search_start <= bytes.len() {
        let Some((start, candidates)) = find_name(bytes, search_start) else {
            break;
        };
        let mut matched = None;
        for name_end in candidates {
            if let Some(assignment_end) = assignment_candidates(text, name_end).into_iter().next() {
                matched = Some(assignment_end);
            }
            if matched.is_some() {
                break;
            }
        }
        let Some(assignment_end) = matched else {
            search_start = start + 1;
            continue;
        };
        out.push_str(&text[last..start]);
        out.push_str(REDACTED);
        last = assignment_end;
        *findings += 1;
        search_start = assignment_end;
    }
    out.push_str(&text[last..]);
    out
}

/// Remove credential-shaped text and report how many findings were replaced.
///
/// The result is the Python `redact_secrets` behaviour: the redacted text and
/// the number of substitutions the eight patterns made.
#[must_use]
pub fn redact_secrets(text: &str) -> (String, usize) {
    let mut findings = 0usize;
    let mut current = replace_all(text, &PEM_PATTERN, &mut findings);
    current = redact_authorization(&current, &mut findings);
    current = replace_all(&current, &OPENAI_KEY_PATTERN, &mut findings);
    current = replace_all(&current, &BEARER_PATTERN, &mut findings);
    current = replace_all(&current, &GITHUB_PATTERN, &mut findings);
    current = replace_all(&current, &AWS_KEY_PATTERN, &mut findings);
    current = redact_quoted_pairs(&current, &mut findings);
    current = redact_assignments(&current, &mut findings);
    (current, findings)
}

/// Truncate text to a byte budget without splitting a character.
///
/// The Python implementation encodes the text, slices the encoded bytes and
/// decodes the slice with `errors="ignore"`, so a character that straddles the
/// cut is dropped whole.
#[must_use]
pub fn truncate_encoded(text: &str, maximum_bytes: usize) -> String {
    if text.len() <= maximum_bytes {
        return text.to_owned();
    }
    let mut end = maximum_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Redact the credential-shaped parts of a JSON value's metadata.
///
/// The Python implementation walks the mapping, replaces every value whose
/// key names a credential, redacts every string value and recurses into
/// nested mappings and lists. The Rust port keeps the same shape but uses an
/// explicit stack so a hostile document cannot overflow the call stack.
#[must_use]
pub fn redact_metadata(value: &CanonicalValue) -> (CanonicalValue, usize) {
    let mut findings = 0usize;
    let mut stack: Vec<Frame<'_>> = Vec::new();
    let mut results: Vec<CanonicalValue> = Vec::new();
    match value {
        CanonicalValue::Object(entries) => stack.push(Frame::object(entries)),
        CanonicalValue::Array(items) => stack.push(Frame::array(items)),
        other => return (other.clone(), 0),
    }
    while let Some(frame) = stack.last_mut() {
        if frame.pending() {
            let child = results.pop().expect("a child result");
            frame.accept(child);
            continue;
        }
        let Some((key, item)) = frame.next_item() else {
            let completed = frame.finish();
            stack.pop();
            results.push(completed);
            continue;
        };
        let key_text = key.map(|key| key.to_string()).unwrap_or_default();
        if key.is_some() && is_secret_name(&key_text) {
            findings += 1;
            frame.push_entry(key, CanonicalValue::text(REDACTED));
            continue;
        }
        match item {
            CanonicalValue::Text(inner) => {
                let (text, count) = redact_secrets(&inner.to_string());
                findings += count;
                frame.push_entry(key, CanonicalValue::text(text));
            }
            CanonicalValue::Object(entries) => {
                frame.expect_child(key);
                stack.push(Frame::object(entries));
            }
            CanonicalValue::Array(items) => {
                frame.expect_child(key);
                stack.push(Frame::array(items));
            }
            other => frame.push_entry(key, other.clone()),
        }
    }
    let redacted = results.pop().expect("the root result");
    (redacted, findings)
}

/// One frame of the explicit metadata traversal.
enum Frame<'a> {
    /// A JSON object being rebuilt.
    Object {
        entries: Vec<(&'a kala_json::JsonText, &'a CanonicalValue)>,
        index: usize,
        output: Vec<(kala_json::JsonText, CanonicalValue)>,
        pending_key: Option<kala_json::JsonText>,
    },
    /// A JSON array being rebuilt.
    Array {
        items: Vec<&'a CanonicalValue>,
        index: usize,
        output: Vec<CanonicalValue>,
        pending: bool,
    },
}

impl<'a> Frame<'a> {
    fn object(entries: &'a kala_json::CanonicalObject) -> Self {
        Self::Object {
            entries: entries.iter().collect(),
            index: 0,
            output: Vec::new(),
            pending_key: None,
        }
    }

    fn array(items: &'a [CanonicalValue]) -> Self {
        Self::Array {
            items: items.iter().collect(),
            index: 0,
            output: Vec::new(),
            pending: false,
        }
    }

    fn pending(&self) -> bool {
        match self {
            Self::Object { pending_key, .. } => pending_key.is_some(),
            Self::Array { pending, .. } => *pending,
        }
    }

    fn expect_child(&mut self, key: Option<&'a kala_json::JsonText>) {
        match self {
            Self::Object { pending_key, .. } => {
                *pending_key = key.cloned();
            }
            Self::Array { pending, .. } => *pending = true,
        }
    }

    fn accept(&mut self, child: CanonicalValue) {
        match self {
            Self::Object {
                output,
                pending_key,
                ..
            } => {
                let key = pending_key
                    .take()
                    .expect("an object frame accepts a child result");
                output.push((key, child));
            }
            Self::Array {
                output, pending, ..
            } => {
                output.push(child);
                *pending = false;
            }
        }
    }

    fn next_item(&mut self) -> Option<(Option<&'a kala_json::JsonText>, &'a CanonicalValue)> {
        match self {
            Self::Object { entries, index, .. } => {
                let (key, item) = *entries.get(*index)?;
                *index += 1;
                Some((Some(key), item))
            }
            Self::Array { items, index, .. } => {
                let item = *items.get(*index)?;
                *index += 1;
                Some((None, item))
            }
        }
    }

    fn push_entry(&mut self, key: Option<&'a kala_json::JsonText>, value: CanonicalValue) {
        match self {
            Self::Object { output, .. } => {
                output.push((key.expect("an object entry has a key").clone(), value))
            }
            Self::Array { output, .. } => output.push(value),
        }
    }

    fn finish(&mut self) -> CanonicalValue {
        match self {
            Self::Object { output, .. } => CanonicalValue::object(std::mem::take(output)),
            Self::Array { output, .. } => CanonicalValue::Array(std::mem::take(output)),
        }
    }
}
