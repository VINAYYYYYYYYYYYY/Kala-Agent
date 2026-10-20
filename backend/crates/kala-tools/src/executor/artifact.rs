//! Artifact verification and classification.
//!
//! A tool is untrusted, so the executor never believes what a tool says about
//! its output. Every artifact is read through one descriptor, so the size
//! check, the digest check and the classification all see the same bytes, and
//! the classification a tool declares can never downgrade the verdict the
//! executor reached on its own.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use kala_contracts::{ArtifactClassification, ArtifactRef, ContractModel};
use kala_json::CanonicalValue;

use crate::executor::error::{ExecutorResult, ToolExecutorError};
use crate::executor::paths::{normpath, parts};
use crate::protocol::sha256_hex;

/// The number of header bytes the classifier inspects.
pub const CLASSIFICATION_HEADER_BYTES: usize = 4_096;

/// The file name suffixes that may hold reviewable text.
pub const TEXTUAL_MEDIA_SUFFIXES: [&str; 8] = [
    ".csv", ".json", ".md", ".text", ".toml", ".txt", ".yaml", ".yml",
];

/// The binary signatures the classifier recognises.
pub const BINARY_SIGNATURES: [(&[u8], &str); 7] = [
    (b"AC10", "image/vnd.dwg"),
    (b"ISO-10303", "model/step"),
    (b"PK\x03\x04", "application/zip"),
    (b"%PDF", "application/pdf"),
    (b"\x89PNG\r\n\x1a\n", "image/png"),
    (b"solid ", "model/stl"),
    (b"\x7fELF", "application/x-elf"),
];

/// Whether a payload is decodable text.
fn probe_is_textual(header: &[u8]) -> bool {
    if header.contains(&0u8) {
        return false;
    }
    std::str::from_utf8(header).is_ok()
}

/// Classify a header, refusing to trust a tool's own claim.
///
/// A recognised binary signature, a payload that is not decodable text, or a
/// format the kernel does not recognise all classify as `DesignBinary`. The
/// weaker claims a tool can make are never allowed to downgrade that verdict,
/// so an unrecognised file can never be published as reviewable text.
pub fn classify_header(
    header: &[u8],
    suffix: &str,
    declared: ArtifactClassification,
) -> ArtifactClassification {
    let header = &header[..header.len().min(CLASSIFICATION_HEADER_BYTES)];
    for (signature, _media_type) in BINARY_SIGNATURES {
        if header.starts_with(signature) {
            return ArtifactClassification::DesignBinary;
        }
    }
    if declared == ArtifactClassification::DesignBinary {
        return ArtifactClassification::DesignBinary;
    }
    if !TEXTUAL_MEDIA_SUFFIXES
        .iter()
        .any(|candidate| suffix.to_lowercase() == *candidate)
    {
        return ArtifactClassification::DesignBinary;
    }
    if !probe_is_textual(header) {
        return ArtifactClassification::DesignBinary;
    }
    ArtifactClassification::Text
}

/// Classify an artifact on disk, refusing to trust a tool's own claim.
pub fn derive_classification(
    path: &Path,
    declared: ArtifactClassification,
) -> ArtifactClassification {
    let header = match read_header(path) {
        Ok(header) => header,
        Err(_) => return ArtifactClassification::DesignBinary,
    };
    classify_header(&header, &file_suffix(path), declared)
}

/// Read the classification header of a file.
fn read_header(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut file = fs::File::open(path)?;
    let mut header = vec![0u8; CLASSIFICATION_HEADER_BYTES];
    let mut filled = 0usize;
    while filled < header.len() {
        match file.read(&mut header[filled..]) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    header.truncate(filled);
    Ok(header)
}

/// The lower case suffix of a path, the way `Path.suffix` reports it.
fn file_suffix(path: &Path) -> String {
    match path.extension() {
        Some(extension) => {
            let mut suffix = String::from(".");
            suffix.push_str(&extension.to_string_lossy());
            suffix
        }
        None => String::new(),
    }
}

/// The components of a parsed artifact URI.
struct ArtifactUriParts<'a> {
    scheme: &'a str,
    netloc: &'a str,
    path: &'a str,
    query: &'a str,
    fragment: &'a str,
}

/// Split a URI the way `urllib.parse.urlsplit` does.
fn split_uri(uri: &str) -> ArtifactUriParts<'_> {
    let (scheme, rest) = match split_scheme(uri) {
        Some((scheme, rest)) => (scheme, rest),
        None => ("", uri),
    };
    let (netloc, path_and_more) = if let Some(after) = rest.strip_prefix("//") {
        match after.find(['/', '?', '#']) {
            Some(index) => (&after[..index], &after[index..]),
            None => (after, ""),
        }
    } else {
        ("", rest)
    };
    let (before_fragment, fragment) = match path_and_more.split_once('#') {
        Some((left, right)) => (left, right),
        None => (path_and_more, ""),
    };
    let (path, query) = match before_fragment.split_once('?') {
        Some((left, right)) => (left, right),
        None => (before_fragment, ""),
    };
    ArtifactUriParts {
        scheme,
        netloc,
        path,
        query,
        fragment,
    }
}

/// The scheme of a URI, when the prefix is a scheme.
fn split_scheme(uri: &str) -> Option<(&str, &str)> {
    let index = uri.find(':')?;
    let scheme = &uri[..index];
    let bytes = scheme.as_bytes();
    if bytes.is_empty()
        || !bytes[0].is_ascii_alphabetic()
        || !bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'+' | b'-' | b'.'))
    {
        return None;
    }
    if scheme.contains('/') {
        return None;
    }
    Some((scheme, &uri[index + 1..]))
}

/// Decode the percent escapes of a URI path, the way `urllib.parse.unquote`
/// does.
fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = (bytes[index + 1] as char).to_digit(16);
            let low = (bytes[index + 2] as char).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                decoded.push(((high << 4) | low) as u8);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// Replace the classification of an artifact reference.
fn with_classification(
    artifact: &ArtifactRef,
    classification: ArtifactClassification,
) -> ExecutorResult<ArtifactRef> {
    let mut payload = artifact
        .to_wire_value()
        .to_canonical_value()
        .map_err(|error| {
            ToolExecutorError::workspace(format!("the artifact reference is invalid: {error}"))
        })?;
    if let CanonicalValue::Object(entries) = &mut payload {
        entries.insert(
            kala_json::JsonText::new("classification"),
            CanonicalValue::text(classification.as_str()),
        );
    }
    ArtifactRef::from_value(&payload).map_err(|error| {
        ToolExecutorError::workspace(format!("the artifact reference is invalid: {error}"))
    })
}

/// The outcome of verifying one artifact.
pub struct VerifiedArtifact {
    /// The artifact reference with the verified classification.
    pub artifact: ArtifactRef,
    /// The confined path the artifact was read from.
    pub path: PathBuf,
}

/// Verify the artifacts a tool reported.
///
/// The verification refuses a duplicate identifier, a remote URI, an
/// unexpected URI component, a relative path, a path that leaves the
/// workspace, a symlink on the way, a size that does not match the report and
/// a digest that does not match the bytes.
pub fn verify_artifacts(
    artifacts: &[ArtifactRef],
    workspace: &Path,
    max_artifact_bytes: u64,
    result_sink: Option<&dyn Fn(&Path)>,
) -> ExecutorResult<Vec<VerifiedArtifact>> {
    let mut verified: Vec<VerifiedArtifact> = Vec::with_capacity(artifacts.len());
    let mut published: Vec<PathBuf> = Vec::with_capacity(artifacts.len());
    let mut seen: Vec<String> = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        let artifact_id = artifact.artifact_id().to_owned();
        if seen.contains(&artifact_id) {
            return Err(ToolExecutorError::workspace(
                "the tool reported duplicate artifact identifiers",
            ));
        }
        seen.push(artifact_id);
        let uri = artifact.uri().as_text().to_utf8().unwrap_or_default();
        let parts = split_uri(&uri);
        if parts.scheme != "file" {
            return Err(ToolExecutorError::workspace(
                "only local file artifacts are accepted from a tool",
            ));
        }
        if !parts.query.is_empty()
            || !parts.fragment.is_empty()
            || !(parts.netloc.is_empty() || parts.netloc == "localhost")
        {
            return Err(ToolExecutorError::workspace(
                "the artifact uri contains unexpected components",
            ));
        }
        let candidate = PathBuf::from(unquote(parts.path));
        if !candidate.is_absolute() {
            return Err(ToolExecutorError::workspace(
                "the artifact uri must contain an absolute path",
            ));
        }
        let resolved = confine(workspace, &candidate)?;
        let data = read_artifact_once(&resolved, artifact_size(artifact), max_artifact_bytes)?;
        if sha256_hex(&data) != artifact.digest().to_hex() {
            return Err(ToolExecutorError::workspace(
                "the artifact digest verification failed",
            ));
        }
        let classification =
            classify_header(&data, &file_suffix(&resolved), artifact.classification());
        let artifact = with_classification(artifact, classification)?;
        verified.push(VerifiedArtifact {
            artifact,
            path: resolved.clone(),
        });
        published.push(resolved);
    }
    if let Some(sink) = result_sink {
        for path in &published {
            sink(path);
        }
    }
    Ok(verified)
}

/// The reported size of an artifact.
fn artifact_size(artifact: &ArtifactRef) -> u64 {
    artifact.size_bytes().to_u64().unwrap_or(u64::MAX)
}

/// Confine an artifact path to the call workspace.
///
/// The path may not be relative, it may not contain a traversal segment, it
/// may not leave the workspace and it may not traverse a symlink.
pub fn confine(workspace: &Path, candidate: &Path) -> ExecutorResult<PathBuf> {
    let base = kala_sandbox::resolve_strict_false(workspace);
    if !base.is_dir() {
        return Err(ToolExecutorError::workspace(
            "the call workspace does not exist",
        ));
    }
    if !candidate.is_absolute() {
        return Err(ToolExecutorError::workspace(
            "artifact paths must be absolute",
        ));
    }
    let lexical = normpath(candidate);
    if parts(&lexical).iter().any(|part| part == "..") {
        return Err(ToolExecutorError::workspace(
            "artifact paths cannot contain traversal segments",
        ));
    }
    if !lexical.starts_with(&base) {
        return Err(ToolExecutorError::workspace(
            "the artifact path escapes the call workspace",
        ));
    }
    let relative: Vec<String> = parts(&lexical)
        .iter()
        .skip(parts(&base).len())
        .cloned()
        .collect();
    let mut current = base.clone();
    for part in relative {
        current.push(part);
        if current.is_symlink() {
            return Err(ToolExecutorError::workspace(
                "artifact paths cannot traverse symlinks",
            ));
        }
    }
    let resolved = kala_sandbox::resolve_strict_false(candidate);
    if !resolved.starts_with(&base) {
        return Err(ToolExecutorError::workspace(
            "the artifact path escapes the call workspace",
        ));
    }
    Ok(resolved)
}

/// Read an artifact through one descriptor so every check sees the same bytes.
pub fn read_artifact_once(
    path: &Path,
    reported_size: u64,
    max_artifact_bytes: u64,
) -> ExecutorResult<Vec<u8>> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // `O_NOFOLLOW` keeps the final component from being a symlink, even
        // if it was swapped in after the confinement check.
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path).map_err(|_| {
        ToolExecutorError::workspace("the artifact is not a regular file inside the workspace")
    })?;
    let metadata = file.metadata().map_err(|_| {
        ToolExecutorError::workspace("the artifact is not a regular file inside the workspace")
    })?;
    if !metadata.is_file() {
        return Err(ToolExecutorError::workspace(
            "the artifact is not a regular file inside the workspace",
        ));
    }
    if metadata.len() > max_artifact_bytes {
        return Err(ToolExecutorError::workspace(
            "the artifact exceeds the bounded artifact size",
        ));
    }
    if metadata.len() != reported_size {
        return Err(ToolExecutorError::workspace(
            "the artifact size does not match the reported size",
        ));
    }
    let mut data: Vec<u8> = Vec::new();
    let mut remaining = max_artifact_bytes.saturating_add(1);
    let mut buffer = [0u8; 65_536];
    while remaining > 0 {
        let wanted = remaining.min(buffer.len() as u64) as usize;
        match file.read(&mut buffer[..wanted]) {
            Ok(0) => break,
            Ok(count) => {
                data.extend_from_slice(&buffer[..count]);
                remaining -= count as u64;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                return Err(ToolExecutorError::workspace(
                    "the artifact could not be read",
                ));
            }
        }
    }
    if data.len() as u64 != reported_size {
        return Err(ToolExecutorError::workspace(
            "the artifact size does not match the reported size",
        ));
    }
    Ok(data)
}
