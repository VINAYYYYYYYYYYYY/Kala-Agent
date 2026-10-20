//! Recompute and verify native source digests.
//!
//! Production evaluator identities use the constants embedded by `build.rs`.
//! This module is the audit/test path: it applies the same rules to a source
//! tree without requiring Python and without being on any evaluator path.
//!
//! The package identity covers this crate's domain and shared Rust sources and
//! the source trees of the accepted local dependencies (`kala-budget`,
//! `kala-json`, `kala-contracts`). External crates, the compiler, the target
//! triple, profile flags, and built binaries are deliberately outside this
//! source-identity boundary; see [`crate::provenance`].

use std::path::Path;

pub use crate::source_digest_core::{
    DependencySourceRoot, DomainSourceDigests, SourceFileDigest, SourceRoots,
};

use crate::provenance::NativeSourceIdentity;

/// The local dependency crates whose source participates in native identity.
pub const LOCAL_DEPENDENCIES: [&str; 3] = ["kala-budget", "kala-json", "kala-contracts"];

/// The crate's own source roots for the tree that was built.
pub fn crate_source_roots() -> SourceRoots {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let crate_root = manifest_dir
        .parent()
        .expect("kala-domains has a parent crates directory");
    SourceRoots::new(
        manifest_dir.join("src"),
        LOCAL_DEPENDENCIES
            .iter()
            .map(|name| DependencySourceRoot::new(*name, crate_root.join(name))),
    )
}

/// Recompute the module and package digests of one domain.
pub fn digest_domain(roots: &SourceRoots, domain: &str) -> Result<DomainSourceDigests, String> {
    crate::source_digest_core::domain_source_digests(roots, domain)
}

/// List every Rust source file that contributes to one domain package digest.
pub fn source_files(roots: &SourceRoots, domain: &str) -> Result<Vec<SourceFileDigest>, String> {
    crate::source_digest_core::package_source_files(roots, domain)
}

/// The crate's own `src` directory, for rebuild/audit checks.
pub fn crate_src_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Recompute one domain's digests from the crate tree that was built.
pub fn current_domain_digests(domain: &str) -> Result<DomainSourceDigests, String> {
    digest_domain(&crate_source_roots(), domain)
}

/// Return the build-embedded digests, for direct comparison with
/// [`current_domain_digests`].
pub fn embedded_identity(domain: &str) -> Option<NativeSourceIdentity> {
    crate::provenance::native_source_identity(domain)
}
