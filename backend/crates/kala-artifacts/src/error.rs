use std::io;
use std::path::PathBuf;
use std::string::FromUtf8Error;

use thiserror::Error;

/// Errors returned by [`crate::ContentAddressedArtifactStore`].
///
/// The Python implementation reports the same conditions through `ValueError`,
/// `FileNotFoundError`, `NotADirectoryError`, `OSError` and
/// `UnicodeDecodeError`.  This enum keeps the conditions distinguishable in
/// native code while preserving the Python messages where callers already rely
/// on them.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ArtifactError {
    /// The configured maximum object size is not representable.
    ///
    /// The Python constructor rejects values that are not `int` or that are
    /// negative.  Rust callers pass `u64`, so only the non-negative check is
    /// still meaningful.
    #[error("max_object_size must be a nonnegative integer")]
    InvalidMaxObjectSize,

    /// The configured root is itself a symlink or a Windows reparse point.
    #[error("artifact root cannot be a symlink")]
    RootSymlink,

    /// The configured root exists but is not a directory.
    #[error("artifact root must be a directory: {0}")]
    RootNotDirectory(PathBuf),

    /// The `<root>/objects` directory is a symlink or reparse point.
    #[error("artifact object directory cannot be a symlink")]
    ObjectRootSymlink,

    /// The `<root>/objects` directory exists but is not a directory.
    #[error("artifact object directory must be a directory: {0}")]
    ObjectRootNotDirectory(PathBuf),

    /// The `<root>/objects` directory no longer identifies the directory this
    /// store constructed itself around.
    #[error("artifact object directory was replaced")]
    ObjectRootReplaced,

    /// A bucket component is a symlink or reparse point.
    #[error("artifact object paths cannot traverse symlinks")]
    ObjectPathSymlink,

    /// A bucket name exists but is not a directory.
    #[error("artifact object parent must be a directory")]
    ObjectParentNotDirectory,

    /// A stored object is a symlink or reparse point.
    #[error("artifact objects cannot be symlinks")]
    ObjectSymlink,

    /// A stored object could not be opened safely for a reason other than the
    /// dedicated not-found/symlink/non-regular cases.
    #[error("artifact object could not be opened safely")]
    ObjectOpen,

    /// A stored object is not a regular file (directory, FIFO, socket, ...).
    #[error("artifact object must be a regular file")]
    NonRegular,

    /// A submitted digest is not a 64-character hexadecimal SHA-256 digest.
    #[error("digest must be a SHA-256 digest")]
    InvalidDigest,

    /// The bytes offered to `put_bytes`/`put_text` exceed the size limit.
    #[error("artifact exceeds the configured size limit")]
    TooLarge,

    /// Bytes read from the store exceed the configured size limit.
    #[error("stored artifact exceeds the configured size limit")]
    StoredTooLarge,

    /// The bytes read from the store do not hash to the requested digest.
    #[error("artifact digest verification failed")]
    DigestMismatch,

    /// An existing object with the requested name did not match its digest.
    #[error("existing artifact failed integrity verification")]
    ExistingIntegrity,

    /// Atomic publication could not be completed or the bucket at its path no
    /// longer identifies the bucket that was written.
    #[error("artifact publication failed")]
    PublicationFailed,

    /// Eight exclusive temporary names were already occupied.
    #[error("no temporary artifact name was available")]
    TemporaryNameExhausted,

    /// The requested object does not exist.  The payload is the normalized
    /// digest, matching `FileNotFoundError(digest)` in Python.
    #[error("{0}")]
    NotFound(String),

    /// Stored bytes are not valid UTF-8 for `get_text`.
    #[error("artifact text is not valid UTF-8: {0}")]
    InvalidUtf8(#[from] FromUtf8Error),

    /// A filesystem operation failed.
    #[error(transparent)]
    Io(#[from] io::Error),

    /// No hardened implementation exists for this target.
    #[error("artifact storage is not supported on this platform")]
    UnsupportedPlatform,
}

#[cfg(unix)]
impl From<rustix::io::Errno> for ArtifactError {
    fn from(error: rustix::io::Errno) -> Self {
        Self::Io(error.into())
    }
}
