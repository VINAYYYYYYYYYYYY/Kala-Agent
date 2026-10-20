//! Typed failures for the native Solid Edge client.
//!
//! These are not wire types: the bridge owns the wire. The kinds mirror the
//! Python client (`CompileError`, `ClientError`, `AuthenticationError`,
//! `ProtocolError`, `NetworkError`, `ArtifactError`) so a refusal from the
//! lowering stage, a refusal from the bridge, and a transport failure stay
//! distinguishable. A lowering refusal happens before a socket is opened.
//!
//! Every message is bounded and normalized exactly like the Python base
//! exception: carriage returns and newlines become spaces, runs of whitespace
//! collapse to one space, and the result is truncated to 500 code points. A
//! message therefore never carries a newline into a log line.

use std::fmt;

use crate::protocol::ErrorCode;

/// Maximum number of code points kept in one failure message.
pub const MAX_MESSAGE_CHARS: usize = 500;

/// Which stage refused the work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ErrorKind {
    /// The construction script is outside the supported Solid Edge subset.
    Compile,
    /// The local bridge could not be reached or refused the request.
    Client,
    /// The bearer token was missing, unreadable, or rejected.
    Authentication,
    /// The bridge answered with something outside protocol v1.
    Protocol,
    /// A socket, timeout, or transfer failure. A submission may be ambiguous.
    Network,
    /// A downloaded artifact did not match its declared size or digest.
    Artifact,
}

impl ErrorKind {
    /// The Python exception class this kind stands for.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Compile => "CompileError",
            Self::Client => "ClientError",
            Self::Authentication => "AuthenticationError",
            Self::Protocol => "ProtocolError",
            Self::Network => "NetworkError",
            Self::Artifact => "ArtifactError",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        formatter.write_str(self.as_str())
    }
}

/// One bounded, typed client failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SolidEdgeClientError {
    kind: ErrorKind,
    message: String,
    wire_code: Option<ErrorCode>,
}

impl SolidEdgeClientError {
    /// A lowering refusal, raised before any transport exists.
    pub fn compile(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Compile, message)
    }

    /// A refusal by the bridge or by the local client.
    pub fn client(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Client, message)
    }

    /// A missing, unreadable, or rejected bearer token.
    pub fn authentication(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Authentication, message)
    }

    /// A response outside protocol v1.
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Protocol, message)
    }

    /// A transport failure.
    pub fn network(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Network, message)
    }

    /// A downloaded artifact that did not match its declaration.
    pub fn artifact(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Artifact, message)
    }

    /// A failure that carries the bridge's stable error code.
    pub fn with_code(kind: ErrorKind, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: normalize(&message.into()),
            wire_code: Some(code),
        }
    }

    fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: normalize(&message.into()),
            wire_code: None,
        }
    }

    /// The stage that refused the work.
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The bounded message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The bridge error code, when the failure came from a typed envelope.
    pub const fn wire_code(&self) -> Option<ErrorCode> {
        self.wire_code
    }

    /// Whether this is a lowering refusal.
    pub const fn is_compile(&self) -> bool {
        matches!(self.kind, ErrorKind::Compile)
    }

    /// Whether this is a transport failure.
    pub const fn is_network(&self) -> bool {
        matches!(self.kind, ErrorKind::Network)
    }
}

impl fmt::Display for SolidEdgeClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SolidEdgeClientError {}

/// Normalize a message the way the Python base exception does.
///
/// Carriage returns and newlines become spaces, every run of whitespace
/// collapses to one space, and the text is truncated to 500 code points.
fn normalize(message: &str) -> String {
    let replaced: String = message
        .chars()
        .map(|character| match character {
            '\r' | '\n' => ' ',
            other => other,
        })
        .collect();
    let collapsed = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return "solid edge client failure".to_owned();
    }
    collapsed.chars().take(MAX_MESSAGE_CHARS).collect()
}

/// The Python `repr()` of a string, used inside refusal messages.
///
/// The lowering refusals quote identifiers with `!r`, so the Rust messages must
/// quote them the same way: single quotes normally, double quotes when the
/// text contains a single quote but no double quote, and escapes for the
/// backslash, the quote, and every non-printable code point.
pub fn python_repr(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(value.len() + 2);
    out.push(quote);
    for character in value.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other if other == quote => {
                out.push('\\');
                out.push(other);
            }
            other if is_printable(other) => out.push(other),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            other => {
                let point = other as u32;
                if point < 0x100 {
                    out.push_str(&format!("\\x{point:02x}"));
                } else if point < 0x1_0000 {
                    out.push_str(&format!("\\u{point:04x}"));
                } else {
                    out.push_str(&format!("\\U{point:08x}"));
                }
            }
        }
    }
    out.push(quote);
    out
}

/// Whether a character is printable in the CPython `repr()` sense.
///
/// ASCII graphics and every non-control character above U+00A0 are printed as
/// themselves; the C0 and C1 control ranges are escaped.
fn is_printable(character: char) -> bool {
    match character {
        '\u{20}'..='\u{7e}' => true,
        '\u{00}'..='\u{1f}' | '\u{7f}'..='\u{9f}' => false,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::{ErrorKind, SolidEdgeClientError, normalize, python_repr};

    #[test]
    fn messages_are_bounded_and_single_line() {
        let error = SolidEdgeClientError::compile("a\nb\r\n  c   d");
        assert_eq!(error.message(), "a b c d");
        assert_eq!(error.kind(), ErrorKind::Compile);
        assert!(error.is_compile());
    }

    #[test]
    fn an_empty_message_has_a_fallback() {
        assert_eq!(normalize("   \n\t "), "solid edge client failure");
    }

    #[test]
    fn a_long_message_is_truncated_to_code_points() {
        let error = SolidEdgeClientError::client("x".repeat(600));
        assert_eq!(error.message().chars().count(), 500);
    }

    #[test]
    fn repr_matches_python() {
        assert_eq!(python_repr("plate"), "'plate'");
        assert_eq!(python_repr("it's"), "\"it's\"");
        assert_eq!(python_repr("a\"b"), "'a\"b'");
        assert_eq!(python_repr("a\\b"), "'a\\\\b'");
        assert_eq!(python_repr("a\nb"), "'a\\nb'");
    }
}
