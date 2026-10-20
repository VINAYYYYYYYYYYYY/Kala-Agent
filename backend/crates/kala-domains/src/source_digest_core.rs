//! Shared native source-digest rules.
//!
//! This file is compiled twice on purpose: once as a private library module,
//! and once by `build.rs` through `#[path = ...]`. Keeping one implementation
//! means the embedded build-time constants and the test/recompute path cannot
//! drift apart.
//!
//! A domain package digest is the SHA-256 of a canonical JSON array of
//! `[label, sha256]` pairs covering:
//!
//! * every `.rs` file under `src/<domain>/`, labelled
//!   `rust-package:<path relative to the domain directory>`;
//! * every shared module directly under `src/`, labelled
//!   `rust-shared:<file name>`;
//! * every behavior-determining local dependency source tree, labelled
//!   `rust-dependency:<crate>/<path relative to the crate root>`. The tree
//!   includes the dependency's `Cargo.toml`, optional `build.rs`, and every
//!   `.rs` file under its `src/` directory.
//!
//! Local dependencies are the accepted read-only crates declared by this
//! crate (`kala-budget`, `kala-json`, `kala-contracts`). External crates,
//! compiler version, target triple, profile flags, and built binaries are not
//! covered by this source identity; the provenance record documents that
//! boundary explicitly.
//!
//! The evaluator module digest is the SHA-256 of
//! `src/<domain>/evaluation.rs`. All identities are native Rust identities; no
//! Python file, bytecode, package, or interpreter participates.

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use kala_json::canonical_json_digest;
use sha2::{Digest, Sha256};

/// The source digests identifying one native domain implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainSourceDigests {
    /// SHA-256 of the domain's `evaluation.rs`.
    pub module_digest: String,
    /// Composite digest over the domain package, shared modules, and local
    /// dependency source trees.
    pub package_digest: String,
}

/// One labelled source file digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFileDigest {
    /// Digest label used by the package digest payload.
    pub label: String,
    /// Path relative to the source tree root, or to the dependency crate root.
    pub relative_path: String,
    /// Lowercase SHA-256 of the file bytes.
    pub sha256: String,
}

/// One local dependency crate whose source participates in package identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependencySourceRoot {
    /// Stable dependency label, for example `kala-budget`.
    pub name: String,
    /// Crate root containing `Cargo.toml` and `src/`.
    pub root: PathBuf,
}

impl DependencySourceRoot {
    /// Build one labelled dependency source root.
    pub fn new(name: impl Into<String>, root: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            root: root.into(),
        }
    }
}

/// The crate and local-dependency source roots used for one audit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRoots {
    /// This crate's `src` directory.
    pub src_root: PathBuf,
    /// Behavior-determining local dependency crates.
    pub dependencies: Vec<DependencySourceRoot>,
}

impl SourceRoots {
    /// Build a source-root set.
    pub fn new(
        src_root: impl Into<PathBuf>,
        dependencies: impl IntoIterator<Item = DependencySourceRoot>,
    ) -> Self {
        Self {
            src_root: src_root.into(),
            dependencies: dependencies.into_iter().collect(),
        }
    }
}

/// Compute the module and package digests for one domain.
pub fn domain_source_digests(
    roots: &SourceRoots,
    domain: &str,
) -> Result<DomainSourceDigests, String> {
    let module_path = roots.src_root.join(domain).join("evaluation.rs");
    let module_digest = sha256_file(&module_path)?;
    let entries = package_entries(roots, domain)?;
    let payload = kala_json::CanonicalValue::Array(
        entries
            .iter()
            .map(|(label, digest)| {
                kala_json::CanonicalValue::Array(vec![
                    kala_json::CanonicalValue::text(label.as_str()),
                    kala_json::CanonicalValue::text(digest.as_str()),
                ])
            })
            .collect(),
    );
    let package_digest = canonical_json_digest(&payload)
        .map_err(|error| format!("cannot canonicalize the package source digest: {error}"))?
        .to_hex();
    Ok(DomainSourceDigests {
        module_digest,
        package_digest,
    })
}

/// List every source file that contributes to a domain package digest.
///
/// The returned order is the canonical label order used by the digest.
pub fn package_source_files(
    roots: &SourceRoots,
    domain: &str,
) -> Result<Vec<SourceFileDigest>, String> {
    let entries = package_entries(roots, domain)?;
    let mut files = Vec::with_capacity(entries.len());
    for (label, digest) in entries {
        let relative_path = if let Some(path) = label.strip_prefix("rust-package:") {
            format!("{domain}/{path}")
        } else if let Some(path) = label.strip_prefix("rust-shared:") {
            path.to_owned()
        } else if let Some(path) = label.strip_prefix("rust-dependency:") {
            path.to_owned()
        } else {
            label.clone()
        };
        files.push(SourceFileDigest {
            label,
            relative_path,
            sha256: digest,
        });
    }
    Ok(files)
}

/// SHA-256 of one file's bytes.
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    Ok(sha256_bytes(&bytes))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let output = hasher.finalize();
    let mut rendered = String::with_capacity(64);
    for byte in output {
        use std::fmt::Write as _;
        let _ = write!(rendered, "{byte:02x}");
    }
    rendered
}

fn package_entries(roots: &SourceRoots, domain: &str) -> Result<Vec<(String, String)>, String> {
    let mut entries: Vec<(String, String)> = Vec::new();
    for path in rust_files(&roots.src_root.join(domain))? {
        let relative = path
            .strip_prefix(roots.src_root.join(domain))
            .map_err(|error| format!("cannot relativize {}: {error}", path.display()))?;
        let label = format!("rust-package:{}", slash_path(relative));
        entries.push((label, sha256_file(&path)?));
    }
    for path in shared_files(&roots.src_root)? {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("a shared source file name is not UTF-8: {}", path.display()))?;
        entries.push((format!("rust-shared:{name}"), sha256_file(&path)?));
    }
    let mut names = BTreeSet::new();
    for dependency in &roots.dependencies {
        if !names.insert(dependency.name.clone()) {
            return Err(format!(
                "duplicate local dependency source label: {}",
                dependency.name
            ));
        }
        for path in dependency_files(&dependency.root)? {
            let relative = path.strip_prefix(&dependency.root).map_err(|error| {
                format!(
                    "cannot relativize dependency file {}: {error}",
                    path.display()
                )
            })?;
            let label = format!(
                "rust-dependency:{}/{}",
                dependency.name,
                slash_path(relative)
            );
            entries.push((label, sha256_file(&path)?));
        }
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(entries)
}

fn shared_files(src_root: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(src_root)
        .map_err(|error| format!("cannot read {}: {error}", src_root.display()))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read a directory entry: {error}"))?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("rs") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn dependency_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for name in ["Cargo.toml", "build.rs"] {
        let path = root.join(name);
        if path.is_file() {
            files.push(path);
        }
    }
    let src = root.join("src");
    if src.is_dir() {
        files.extend(rust_files(&src)?);
    } else {
        return Err(format!(
            "local dependency {} has no src directory",
            root.display()
        ));
    }
    files.sort();
    Ok(files)
}

fn rust_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let entries =
        fs::read_dir(root).map_err(|error| format!("cannot read {}: {error}", root.display()))?;
    let mut entries = entries.collect::<Result<Vec<_>, _>>().map_err(|error| {
        format!(
            "cannot read a directory entry under {}: {error}",
            root.display()
        )
    })?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            files.extend(rust_files(&path)?);
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            files.push(path);
        }
    }
    Ok(files)
}

fn slash_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
