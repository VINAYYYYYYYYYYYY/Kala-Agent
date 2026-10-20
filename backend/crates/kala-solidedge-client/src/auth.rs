//! Bearer-token handling for the loopback bridge.
//!
//! The token is a shared secret between the bridge and this client. It is
//! read from a private file, it is never written to a log line, and it never
//! appears in a URL, an error message, or a journal record. The comparison
//! helper is constant time, exactly like the bridge's own check, so a local
//! process cannot learn the token by timing a rejection.

use std::path::Path;

use crate::errors::SolidEdgeClientError;

/// The smallest token the bridge accepts.
pub const MIN_TOKEN_CHARS: usize = 16;
/// The largest token the bridge accepts.
pub const MAX_TOKEN_CHARS: usize = 256;

/// Read the bearer token from a private file.
///
/// The file is read as ASCII, exactly like the Python client, and the token
/// is trimmed. An empty token is refused: it would be sent as an empty
/// credential and the bridge would reject it with an opaque 401.
pub fn read_token_file(path: &Path) -> Result<String, SolidEdgeClientError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(SolidEdgeClientError::authentication(
                "the bridge token file does not exist; run the bridge token command",
            ));
        }
        Err(_) => {
            return Err(SolidEdgeClientError::authentication(
                "the bridge token file could not be read",
            ));
        }
    };
    let text = String::from_utf8(bytes).map_err(|_| {
        SolidEdgeClientError::authentication("the bridge token file could not be read")
    })?;
    if !text.is_ascii() {
        return Err(SolidEdgeClientError::authentication(
            "the bridge token file could not be read",
        ));
    }
    let token = text.trim();
    if token.is_empty() {
        return Err(SolidEdgeClientError::authentication(
            "the bridge token is empty",
        ));
    }
    Ok(token.to_owned())
}

/// Compare a presented bearer token in constant time.
///
/// The bridge performs the same comparison. This client never receives a
/// token to verify, but the helper is kept so the scheme has exactly one
/// implementation.
pub fn token_matches(expected: &str, provided: &str) -> bool {
    if provided.is_empty() {
        return false;
    }
    let expected = expected.as_bytes();
    let provided = provided.as_bytes();
    if expected.len() != provided.len() {
        // Still compare, so the work does not depend on the length alone.
        let mut difference = 1_u8;
        for (left, right) in expected.iter().zip(provided.iter()) {
            difference |= left ^ right;
        }
        return difference == 0 && expected.len() == provided.len();
    }
    let mut difference = 0_u8;
    for (left, right) in expected.iter().zip(provided.iter()) {
        difference |= left ^ right;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::{read_token_file, token_matches};

    #[test]
    fn a_missing_token_file_is_an_authentication_error() {
        let error = read_token_file(std::path::Path::new("/nonexistent/bridge.token"))
            .expect_err("refused");
        assert!(
            error.message().contains("does not exist"),
            "{}",
            error.message()
        );
    }

    #[test]
    fn an_empty_token_file_is_refused() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("bridge.token");
        std::fs::write(&path, "   \n").expect("write");
        let error = read_token_file(&path).expect_err("refused");
        assert!(error.message().contains("empty"), "{}", error.message());
    }

    #[test]
    fn a_token_is_trimmed() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("bridge.token");
        std::fs::write(&path, "  secret-token  \n").expect("write");
        assert_eq!(read_token_file(&path).expect("token"), "secret-token");
    }

    #[test]
    fn comparison_is_exact() {
        assert!(token_matches("secret", "secret"));
        assert!(!token_matches("secret", "secrey"));
        assert!(!token_matches("secret", "secre"));
        assert!(!token_matches("secret", ""));
    }
}
