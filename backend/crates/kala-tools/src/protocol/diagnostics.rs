//! Diagnostics for a failed tool run.
//!
//! A tool is untrusted, so its diagnostics are bounded, stripped of control
//! characters and stripped of host paths before they reach a log or a user.

use crate::protocol::MAX_SANITIZED_DETAIL_LENGTH;
use crate::protocol::error::ToolProtocolError;

/// Collapse a run of whitespace into a single space.
///
/// This is `re.sub(r"\s+", " ", text)` with CPython's whitespace class.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for character in text.chars() {
        if is_python_space(character) {
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

/// CPython's `str.isspace()` for a character.
fn is_python_space(character: char) -> bool {
    matches!(
        character,
        '\t'..='\r'
            | '\u{1c}'..='\u{1f}'
            | ' '
            | '\u{85}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

/// CPython's `str.strip()` for a character.
///
/// CPython strips every character whose `str.isspace()` is true.
fn is_python_strip_space(character: char) -> bool {
    crate::protocol::text::is_python_whitespace(u32::from(character))
}

/// Summarize a failure for a log or a user.
///
/// The class name is reported first, then the message. A replacement string
/// redacts a host path.
pub fn summarize_error(error: &ToolProtocolError, replacement: Option<&str>) -> String {
    let name = error.python_class_name();
    let detail = error.to_string();
    let detail = strip_python_whitespace(&detail);
    if detail.is_empty() {
        return name.to_owned();
    }
    let detail = match replacement {
        Some(replacement) => detail.replace(replacement, "<workspace>"),
        None => detail,
    };
    let detail = collapse_whitespace(&detail);
    let detail: String = detail.chars().take(512).collect();
    format!("{name}: {detail}")
}

/// Strip CPython's whitespace from both ends of a text.
fn strip_python_whitespace(text: &str) -> String {
    text.trim_matches(is_python_strip_space).to_owned()
}

/// Turn a child's standard error into a short, sanitized diagnostic.
///
/// An import failure or a sandbox setup refusal is only actionable if its
/// message survives, so the first and the last line are kept: for a Python
/// traceback the last line carries the exception. A child is untrusted, so the
/// sample is bounded, stripped of control characters and has host paths
/// replaced.
pub fn sanitize_child_stderr(payload: &[u8], replacement: Option<&str>) -> String {
    if payload.is_empty() {
        return String::new();
    }
    let limit = MAX_SANITIZED_DETAIL_LENGTH * 2;
    let slice = &payload[..payload.len().min(limit)];
    let text = String::from_utf8_lossy(slice);
    let mut filtered = String::with_capacity(text.len());
    for character in text.chars() {
        if character == '\n' || character >= ' ' {
            filtered.push(character);
        }
    }
    let filtered = match replacement {
        Some(replacement) if !replacement.is_empty() => {
            filtered.replace(replacement, "<workspace>")
        }
        _ => filtered,
    };
    let mut selected: Vec<String> = Vec::new();
    for line in split_lines(&filtered) {
        let line = collapse_whitespace(&line);
        let line = strip_python_whitespace(&line);
        if !line.is_empty() {
            selected.push(line);
        }
    }
    if selected.len() > 2 {
        let first = selected[0].clone();
        let last = selected[selected.len() - 1].clone();
        selected = vec![first, "...".to_owned(), last];
    }
    selected
        .join(" | ")
        .chars()
        .take(MAX_SANITIZED_DETAIL_LENGTH)
        .collect()
}

/// Split text into lines the way CPython's `str.splitlines` does.
fn split_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\n' => {
                lines.push(std::mem::take(&mut current));
            }
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                lines.push(std::mem::take(&mut current));
            }
            '\u{2028}' | '\u{2029}' => {
                lines.push(std::mem::take(&mut current));
            }
            other => current.push(other),
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}
