//! Secure content-addressed artifact storage compatible with
//! `kala_agent.persistence.artifacts.ContentAddressedArtifactStore`.
//!
//! Objects live at `<root>/objects/<first-two-digest-hex>/<digest>` and are
//! immutable.  Every operation resolves the root, `objects` and bucket
//! components relatively from an open directory handle (Windows: native
//! `NtCreateFile` with a parent handle), rejects symlinks/reparse points, and
//! verifies the SHA-256 digest of the bytes it returns.
//!
//! Publication writes an exclusive temporary file in the destination bucket,
//! flushes it, then atomically publishes it under the digest name without
//! overwriting an existing object: a same-directory hard link on Unix and a
//! handle-based, no-replace rename on Windows.  A final identity check refuses
//! a publication whose bucket was renamed or replaced while it was in flight.
//!
//! Linux and other Unix targets use `rustix` descriptor-relative syscalls with
//! `O_NOFOLLOW`, `O_CLOEXEC`, `O_DIRECTORY`, `O_NONBLOCK` on object opens, and
//! `O_EXCL` on temporary creation.  Windows uses `windows-sys` and
//! `NtCreateFile`/`NtCreateFile`-relative handles, `FILE_OPEN_REPARSE_POINT`,
//! `FileIdInfo`/`GetFileInformationByHandle`, `FlushFileBuffers`, and
//! `SetFileInformationByHandle(FileRenameInfo)`; the unsafe blocks are kept in
//! `sys::windows` and each one wraps a documented Win32/NT call.
//!
//! The store holds no process-wide lock.  Each mutation is individually atomic,
//! so threads and processes may share one root without coordination.

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

mod error;
mod sys;

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use kala_json::Sha256Digest;
use sha2::{Digest, Sha256};

pub use error::ArtifactError;

/// Default maximum object size, matching the Python constructor default.
pub const DEFAULT_MAX_OBJECT_SIZE: u64 = 1_000_000_000;

/// Number of bucket prefix characters (`objects/<ab>/<digest>`).
const BUCKET_PREFIX_LENGTH: usize = 2;
const TEMPORARY_PREFIX: &str = ".artifact-";
const TEMPORARY_SUFFIX: &str = ".tmp";
const TEMPORARY_NAME_ATTEMPTS: usize = 8;
const TEMPORARY_NAME_BYTES: usize = 16;

/// An immutable content-addressed artifact store.
///
/// The type is `Send` and `Sync`: it contains only immutable configuration and
/// owned directory handles, and every operation opens its own components.
pub struct ContentAddressedArtifactStore {
    root: PathBuf,
    max_object_size: u64,
    root_dir: sys::Dir,
    objects_dir: sys::Dir,
}

impl std::fmt::Debug for ContentAddressedArtifactStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ContentAddressedArtifactStore")
            .field("root", &self.root)
            .field("max_object_size", &self.max_object_size)
            .finish_non_exhaustive()
    }
}

impl ContentAddressedArtifactStore {
    /// Open or create a store rooted at `root`.
    ///
    /// `max_object_size` is the inclusive upper bound for both newly written
    /// and read objects.  A value of `0` permits only the empty artifact.
    pub fn new(root: impl AsRef<Path>, max_object_size: u64) -> Result<Self, ArtifactError> {
        let expanded = expand_user(root.as_ref());
        if expanded.as_os_str().is_empty() {
            return Self::open_expanded(PathBuf::from("."), max_object_size);
        }
        Self::open_expanded(expanded, max_object_size)
    }

    /// Open or create a store at the `artifact_dir` already derived from
    /// application settings.
    ///
    /// The Python API receives a `Settings` object.  The native backend keeps
    /// configuration separate and passes `Settings::artifact_dir` here, so this
    /// constructor preserves the same effective root without a dependency on a
    /// Python-shaped settings type.
    pub fn from_artifact_dir(
        artifact_dir: impl AsRef<Path>,
        max_object_size: u64,
    ) -> Result<Self, ArtifactError> {
        Self::new(artifact_dir, max_object_size)
    }

    /// Alias for [`Self::from_artifact_dir`] for callers porting Python code.
    pub fn from_settings(
        artifact_dir: impl AsRef<Path>,
        max_object_size: u64,
    ) -> Result<Self, ArtifactError> {
        Self::from_artifact_dir(artifact_dir, max_object_size)
    }

    fn open_expanded(expanded: PathBuf, max_object_size: u64) -> Result<Self, ArtifactError> {
        // Reject a symlink root before any creation can follow it.  The
        // platform opener repeats this check with a handle and also catches
        // Windows junctions/reparse points.
        let existed = match fs::symlink_metadata(&expanded) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ArtifactError::RootSymlink);
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(ArtifactError::RootNotDirectory(expanded));
            }
            Ok(_) => true,
            Err(_) => false,
        };

        if !existed {
            if let Some(parent) = expanded.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent)?;
                }
            }
            let builder = fs::DirBuilder::new();
            #[cfg(unix)]
            let builder = {
                use std::os::unix::fs::DirBuilderExt;
                // Matches `Path.mkdir(mode=0o700)` for the final component.
                // An existing directory is never chmodded.
                let mut builder = builder;
                builder.mode(0o700);
                builder
            };
            match builder.create(&expanded) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => match fs::symlink_metadata(&expanded) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        return Err(ArtifactError::RootSymlink);
                    }
                    Ok(metadata) if !metadata.is_dir() => {
                        return Err(ArtifactError::RootNotDirectory(expanded));
                    }
                    _ => return Err(ArtifactError::Io(error)),
                },
            }
        }

        // The root handle is opened with `O_NOFOLLOW` (Unix) or
        // `FILE_OPEN_REPARSE_POINT` (Windows) and held for the life of the
        // store.  A later rename/replacement of the path cannot redirect an
        // operation onto another directory.
        let root_dir = sys::Dir::open_root(&expanded)?;
        let canonical_root = canonical_root_path(&expanded)?;
        let objects_dir = root_dir.ensure_objects(&canonical_root)?;
        let store = Self {
            root: canonical_root,
            max_object_size,
            root_dir,
            objects_dir,
        };
        store.verify_objects_path()?;
        Ok(store)
    }

    /// Resolved root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Inclusive maximum object size.
    pub fn max_object_size(&self) -> u64 {
        self.max_object_size
    }

    /// Return `objects/<first-two-digest-hex>/<digest>` for a normalized digest.
    ///
    /// Invalid digests are rejected with [`ArtifactError::InvalidDigest`].
    pub fn key_for(&self, digest: &str) -> Result<String, ArtifactError> {
        let normalized = normalize_digest(digest)?;
        Ok(format!(
            "objects/{}/{normalized}",
            &normalized[..BUCKET_PREFIX_LENGTH]
        ))
    }

    /// Store `data` and return its lowercase SHA-256 digest.
    pub fn put_bytes(&self, data: &[u8]) -> Result<String, ArtifactError> {
        if data.len() as u64 > self.max_object_size {
            return Err(ArtifactError::TooLarge);
        }
        let digest = sha256_hex(data);
        self.verify_objects_path()?;
        let bucket = self.objects_dir.open_bucket(
            &digest[..BUCKET_PREFIX_LENGTH],
            &digest,
            true,
            self.max_object_size,
        )?;
        bucket.publish(&digest, data)?;
        // Re-validate the `<root>/objects` identity after publication: a swap
        // in the middle of the operation must not be reported as success.
        self.verify_objects_path()?;
        Ok(digest)
    }

    /// Store the UTF-8 encoding of `text` and return its digest.
    pub fn put_text(&self, text: &str) -> Result<String, ArtifactError> {
        self.put_bytes(text.as_bytes())
    }

    /// Read and digest-verify an object.
    pub fn get_bytes(&self, digest: &str) -> Result<Vec<u8>, ArtifactError> {
        let normalized = normalize_digest(digest)?;
        self.verify_objects_path()?;
        let bucket = self.objects_dir.open_bucket(
            &normalized[..BUCKET_PREFIX_LENGTH],
            &normalized,
            false,
            self.max_object_size,
        )?;
        bucket.read_object(&normalized)
    }

    /// Read a digest-verified object and decode it as UTF-8.
    pub fn get_text(&self, digest: &str) -> Result<String, ArtifactError> {
        String::from_utf8(self.get_bytes(digest)?).map_err(ArtifactError::from)
    }

    /// Return whether a digest-addressed object exists and verifies.
    ///
    /// Matches Python: only a missing object yields `false`; an invalid digest,
    /// a symlinked bucket/object, a corrupt object or an oversized stored
    /// object is reported as an error.
    pub fn contains(&self, digest: &str) -> Result<bool, ArtifactError> {
        let normalized = normalize_digest(digest)?;
        self.verify_objects_path()?;
        let bucket = match self.objects_dir.open_bucket(
            &normalized[..BUCKET_PREFIX_LENGTH],
            &normalized,
            false,
            self.max_object_size,
        ) {
            Ok(bucket) => bucket,
            Err(ArtifactError::NotFound(_)) => return Ok(false),
            Err(error) => return Err(error),
        };
        match bucket.read_object(&normalized) {
            Ok(_) => Ok(true),
            Err(ArtifactError::NotFound(_)) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Return the verified size of an object.
    pub fn get_size(&self, digest: &str) -> Result<u64, ArtifactError> {
        Ok(self.get_bytes(digest)?.len() as u64)
    }

    fn verify_objects_path(&self) -> Result<(), ArtifactError> {
        self.root_dir
            .verify_objects_path(&self.objects_dir, &self.root)
    }
}

/// Normalize a digest exactly like Python's `_normalize_digest`: trim the
/// Python `str.strip()` whitespace set, apply full Unicode lowercasing, then
/// require exactly 64 ASCII hexadecimal digits.
pub fn normalize_digest(value: &str) -> Result<String, ArtifactError> {
    let trimmed = python_strip(value);
    let mut normalized = String::with_capacity(trimmed.len());
    for character in trimmed.chars() {
        for lowered in character.to_lowercase() {
            if !lowered.is_ascii_hexdigit() {
                return Err(ArtifactError::InvalidDigest);
            }
            normalized.push(lowered);
        }
    }
    if normalized.len() != 64 {
        return Err(ArtifactError::InvalidDigest);
    }
    Ok(normalized)
}

fn python_strip(value: &str) -> &str {
    // CPython's `str.strip()` whitespace set also includes the information
    // separators U+001C..=U+001F, which Rust's `char::is_whitespace` omits.
    value.trim_matches(|character: char| {
        character.is_whitespace() || matches!(character, '\u{001c}'..='\u{001f}')
    })
}

pub(crate) fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&hasher.finalize());
    Sha256Digest::from_bytes(bytes).to_hex()
}

pub(crate) fn random_temporary_name() -> Result<String, ArtifactError> {
    let mut bytes = [0u8; TEMPORARY_NAME_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ArtifactError::Io(std::io::Error::other(format!(
            "operating-system randomness is unavailable: {error}"
        )))
    })?;
    let mut name = String::with_capacity(
        TEMPORARY_PREFIX.len() + TEMPORARY_NAME_BYTES * 2 + TEMPORARY_SUFFIX.len(),
    );
    name.push_str(TEMPORARY_PREFIX);
    for byte in bytes {
        use std::fmt::Write as _;
        // Writing to a String cannot fail.
        let _ = write!(name, "{byte:02x}");
    }
    name.push_str(TEMPORARY_SUFFIX);
    Ok(name)
}

pub(crate) fn temporary_name_attempts() -> usize {
    TEMPORARY_NAME_ATTEMPTS
}

pub(crate) fn read_limited(
    file: std::fs::File,
    max_object_size: u64,
) -> Result<Vec<u8>, ArtifactError> {
    let mut data = Vec::new();
    let limit = max_object_size.saturating_add(1);
    file.take(limit).read_to_end(&mut data)?;
    if data.len() as u64 > max_object_size {
        return Err(ArtifactError::StoredTooLarge);
    }
    Ok(data)
}

pub(crate) fn verify_digest(digest: &str, data: &[u8]) -> Result<(), ArtifactError> {
    if sha256_hex(data) != digest {
        return Err(ArtifactError::DigestMismatch);
    }
    Ok(())
}

fn expand_user(path: &Path) -> PathBuf {
    expand_user_with_home(path, current_home())
}

/// Canonicalize the root for the public `root()` property.
///
/// On Windows, `std::fs::canonicalize` returns an extended-length `\\?\` path.
/// Python's `Path.resolve()` returns the familiar `C:\...` form, so the
/// verbatim prefix is removed (and `\\?\UNC\` becomes `\\`) while operations
/// keep using the validated handle rather than the display path.
fn canonical_root_path(path: &Path) -> std::io::Result<PathBuf> {
    let canonical = fs::canonicalize(path)?;
    #[cfg(windows)]
    {
        let text = canonical.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return Ok(PathBuf::from(format!(r"\\{rest}")));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            if rest.as_bytes().get(1) == Some(&b':') {
                return Ok(PathBuf::from(rest.to_string()));
            }
        }
    }
    Ok(canonical)
}

fn expand_user_with_home(path: &Path, home: Option<PathBuf>) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path.to_path_buf();
    };

    if text == "~" {
        if let Some(home) = home {
            return home;
        }
        return path.to_path_buf();
    }
    if let Some(rest) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        if let Some(home) = home {
            return home.join(rest);
        }
        return path.to_path_buf();
    }
    if let Some(user) = text.strip_prefix('~') {
        let (user, rest) = match user.find(['/', '\\']) {
            Some(index) => (&user[..index], &user[index + 1..]),
            None => (user, ""),
        };
        if !user.is_empty() {
            if let Some(home) = user_home(user) {
                return if rest.is_empty() {
                    home
                } else {
                    home.join(rest)
                };
            }
        }
    }
    path.to_path_buf()
}

fn current_home() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            if !profile.is_empty() {
                return Some(PathBuf::from(profile));
            }
        }
        let drive = std::env::var_os("HOMEDRIVE")?;
        let path = std::env::var_os("HOMEPATH")?;
        if drive.is_empty() || path.is_empty() {
            return None;
        }
        let mut combined = drive;
        combined.push(path);
        Some(PathBuf::from(combined))
    }
    #[cfg(unix)]
    {
        if let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) {
            return Some(PathBuf::from(home));
        }
        // CPython falls back to the password database for the current user.
        // SAFETY: `getuid` has no preconditions.
        uid_home(unsafe { libc::getuid() })
    }
    #[cfg(not(any(windows, unix)))]
    {
        std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
    }
}

#[cfg(unix)]
fn uid_home(uid: libc::uid_t) -> Option<PathBuf> {
    let mut buffer = vec![0u8; 16_384];
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: `entry` and `buffer` are writable for the duration of the call
    // and `result` points to a writable pointer.
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            &mut entry,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    passwd_home(&entry, result, status)
}

#[cfg(unix)]
fn user_home(user: &str) -> Option<PathBuf> {
    use std::ffi::CString;

    let user = CString::new(user).ok()?;
    let mut buffer = vec![0u8; 16_384];
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: `user` is a valid NUL-terminated string, `entry` and `buffer`
    // are writable for the duration of the call, and `result` points to a
    // writable pointer.
    let status = unsafe {
        libc::getpwnam_r(
            user.as_ptr(),
            &mut entry,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    passwd_home(&entry, result, status)
}

#[cfg(unix)]
fn passwd_home(
    entry: &libc::passwd,
    result: *mut libc::passwd,
    status: libc::c_int,
) -> Option<PathBuf> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;

    if status != 0 || result.is_null() {
        return None;
    }
    // SAFETY: on success the lookup functions set `entry.pw_dir` to a
    // NUL-terminated string inside the caller's buffer, which is still alive.
    let home = unsafe { CStr::from_ptr(entry.pw_dir) };
    if home.to_bytes().is_empty() {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(home.to_bytes())))
}

#[cfg(not(unix))]
fn user_home(_user: &str) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_normalization_matches_python_trim_and_lower() {
        let digest = "a".repeat(64);
        assert_eq!(normalize_digest(&digest).unwrap(), digest);
        assert_eq!(
            normalize_digest(&format!("\u{1c} {} \u{3000}", digest.to_uppercase())).unwrap(),
            digest
        );
        assert!(normalize_digest(&"a".repeat(63)).is_err());
        assert!(normalize_digest(&"z".repeat(64)).is_err());
        assert!(normalize_digest("../outside").is_err());
        assert!(normalize_digest("").is_err());
    }

    #[test]
    fn key_layout_matches_python() {
        let directory = tempfile::tempdir().unwrap();
        let store = ContentAddressedArtifactStore::new(directory.path(), 16).unwrap();
        let digest = sha256_hex(b"key");
        assert_eq!(
            store.key_for(&digest).unwrap(),
            format!("objects/{}/{digest}", &digest[..2])
        );
    }

    #[test]
    fn temporary_names_are_exclusive_looking() {
        let first = random_temporary_name().unwrap();
        let second = random_temporary_name().unwrap();
        assert!(first.starts_with(".artifact-"));
        assert!(first.ends_with(".tmp"));
        assert_eq!(first.len(), ".artifact-".len() + 32 + ".tmp".len());
        assert_ne!(first, second);
    }

    #[test]
    fn user_expansion_supports_current_user() {
        assert_eq!(
            expand_user_with_home(Path::new("~/artifacts"), Some(PathBuf::from("/home/test"))),
            PathBuf::from("/home/test/artifacts")
        );
        assert_eq!(
            expand_user_with_home(Path::new("~"), Some(PathBuf::from("/home/test"))),
            PathBuf::from("/home/test")
        );
        assert_eq!(
            expand_user_with_home(
                Path::new("relative/path"),
                Some(PathBuf::from("/home/test"))
            ),
            PathBuf::from("relative/path")
        );
        assert_eq!(
            expand_user_with_home(Path::new("~/artifacts"), None),
            PathBuf::from("~/artifacts")
        );
    }
}
