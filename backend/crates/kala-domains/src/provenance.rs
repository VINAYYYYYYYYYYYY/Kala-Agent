//! Explicit native Rust implementation provenance.
//!
//! The Python evaluator digest (`evaluator-implementation-digest-v2`) hashes
//! Python source files. That identity cannot be reproduced from Rust sources
//! and is not relabelled here. Native evaluators use
//! `evaluator-implementation-digest-v3-rust`, bind native Rust source digests
//! embedded by `build.rs`, and carry an explicit `rust` language marker in the
//! digest payload.
//!
//! The native package source digest covers this crate's domain/shared Rust
//! sources **and** the source trees of the accepted local dependencies
//! (`kala-budget`, `kala-json`, `kala-contracts`), because evaluator formulas,
//! canonical JSON, contract models, and score rounding call into them. External
//! crates, the compiler, target triple, profile flags, and built binaries are
//! outside this identity; that boundary is stated in the compatibility record.
//!
//! The legacy Python package and implementation digests are retained only in
//! [`legacy`] as clearly named historical constants and in
//! [`ProvenanceCompatibilityRecord`], which states that native identities are
//! different and that prepared/approved legacy bindings require explicit
//! re-preparation and re-approval.

use kala_json::CanonicalValue;

use crate::error::DomainError;
use crate::package::DomainPackageApi;
use crate::registry::{DomainName, package_for};
use crate::spec::{
    LEGACY_PYTHON_IMPLEMENTATION_DIGEST_VERSION, MAX_IMPLEMENTATION_NAME_LENGTH,
    NATIVE_IMPLEMENTATION_DIGEST_VERSION, NATIVE_IMPLEMENTATION_LANGUAGE, canonical_digest,
    require_capabilities, require_digest, require_identifier,
};

mod generated {
    include!(concat!(env!("OUT_DIR"), "/native_source_digests.rs"));
}

/// The embedded module/package digests for one native domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeSourceIdentity {
    /// SHA-256 of `src/<domain>/evaluation.rs`.
    pub module_digest: &'static str,
    /// Digest over the domain package, shared Rust modules, and accepted local
    /// dependency source trees.
    pub package_digest: &'static str,
}

/// Return the build-embedded source identity for one domain name.
pub fn native_source_identity(domain: &str) -> Option<NativeSourceIdentity> {
    match domain {
        "mechanical" => Some(NativeSourceIdentity {
            module_digest: generated::MECHANICAL_MODULE_DIGEST,
            package_digest: generated::MECHANICAL_PACKAGE_DIGEST,
        }),
        "building" => Some(NativeSourceIdentity {
            module_digest: generated::BUILDING_MODULE_DIGEST,
            package_digest: generated::BUILDING_PACKAGE_DIGEST,
        }),
        "wing" => Some(NativeSourceIdentity {
            module_digest: generated::WING_MODULE_DIGEST,
            package_digest: generated::WING_PACKAGE_DIGEST,
        }),
        _ => None,
    }
}

/// The native module name for a domain evaluator module.
pub fn native_module_name(domain: &str) -> String {
    format!("kala_domains.{domain}.evaluation")
}

/// The identified implementation of one evaluator bound to a catalog step.
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluatorImplementation {
    step_id: String,
    qualified_name: String,
    module_name: String,
    module_digest: String,
    package_digest: String,
    required_capabilities: Vec<String>,
    digest: String,
}

impl EvaluatorImplementation {
    /// Build an implementation identity from explicit source digests.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        step_id: &str,
        qualified_name: &str,
        module_name: &str,
        module_digest: &str,
        package_digest: &str,
        required_capabilities: &[&str],
    ) -> Result<Self, DomainError> {
        let step_id = require_identifier(step_id, "evaluator implementation step_id")?;
        validate_implementation_name(module_name).map_err(|_| {
            DomainError::Package(
                "an evaluator implementation must declare an importable module".to_owned(),
            )
        })?;
        validate_implementation_name(qualified_name).map_err(|_| {
            DomainError::Package(
                "an evaluator qualified name must be a safe dotted name".to_owned(),
            )
        })?;
        if !qualified_name.starts_with(&format!("{module_name}.")) {
            return Err(DomainError::Package(
                "an evaluator qualified name must name its own module".to_owned(),
            ));
        }
        let module_digest = require_digest(module_digest, "evaluator module digest")?;
        let package_digest = require_digest(package_digest, "evaluator package digest")?;
        let required_capabilities = require_capabilities(
            required_capabilities.iter().copied(),
            "evaluator required capability",
        )?;
        let digest = compute_implementation_digest(
            &step_id,
            qualified_name,
            module_name,
            &module_digest,
            &package_digest,
            &required_capabilities,
        )?;
        Ok(Self {
            step_id,
            qualified_name: qualified_name.to_owned(),
            module_name: module_name.to_owned(),
            module_digest,
            package_digest,
            required_capabilities,
            digest,
        })
    }

    /// Build a shipped native implementation identity from the build-embedded
    /// Rust source digests.
    pub fn native(
        domain: &str,
        step_id: &str,
        type_name: &str,
        required_capabilities: &[&str],
    ) -> Result<Self, DomainError> {
        let identity = native_source_identity(domain).ok_or_else(|| {
            DomainError::Package(format!("{domain} has no native source identity"))
        })?;
        let module_name = native_module_name(domain);
        let qualified_name = format!("{module_name}.{type_name}");
        Self::new(
            step_id,
            &qualified_name,
            &module_name,
            identity.module_digest,
            identity.package_digest,
            required_capabilities,
        )
    }

    /// The bound catalog step id.
    pub fn step_id(&self) -> &str {
        &self.step_id
    }
    /// The stable native qualified name.
    pub fn qualified_name(&self) -> &str {
        &self.qualified_name
    }
    /// The native evaluator module name.
    pub fn module_name(&self) -> &str {
        &self.module_name
    }
    /// SHA-256 of the evaluator module source.
    pub fn module_digest(&self) -> &str {
        &self.module_digest
    }
    /// Digest over the package and shared Rust source.
    pub fn package_digest(&self) -> &str {
        &self.package_digest
    }
    /// Canonical required capabilities.
    pub fn required_capabilities(&self) -> &[String] {
        &self.required_capabilities
    }
    /// The implementation digest.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// The Python `EvaluatorImplementation.as_dict()` declaration shape.
    pub fn as_canonical_value(&self) -> CanonicalValue {
        CanonicalValue::object([
            ("digest", CanonicalValue::text(self.digest.as_str())),
            ("module", CanonicalValue::text(self.module_name.as_str())),
            (
                "module_digest",
                CanonicalValue::text(self.module_digest.as_str()),
            ),
            (
                "package_digest",
                CanonicalValue::text(self.package_digest.as_str()),
            ),
            (
                "qualified_name",
                CanonicalValue::text(self.qualified_name.as_str()),
            ),
            (
                "required_capabilities",
                CanonicalValue::Array(
                    self.required_capabilities
                        .iter()
                        .map(|value| CanonicalValue::text(value.as_str()))
                        .collect(),
                ),
            ),
            ("step_id", CanonicalValue::text(self.step_id.as_str())),
        ])
    }

    /// The canonical digest payload, including the explicit native scheme and
    /// language. Exposed so reports and tests can show the bound identity.
    pub fn digest_payload(&self) -> CanonicalValue {
        CanonicalValue::object([
            (
                "implementation_digest_version",
                CanonicalValue::text(NATIVE_IMPLEMENTATION_DIGEST_VERSION),
            ),
            (
                "implementation_language",
                CanonicalValue::text(NATIVE_IMPLEMENTATION_LANGUAGE),
            ),
            ("module", CanonicalValue::text(self.module_name.as_str())),
            (
                "module_digest",
                CanonicalValue::text(self.module_digest.as_str()),
            ),
            (
                "package_digest",
                CanonicalValue::text(self.package_digest.as_str()),
            ),
            (
                "qualified_name",
                CanonicalValue::text(self.qualified_name.as_str()),
            ),
            (
                "required_capabilities",
                CanonicalValue::Array(
                    self.required_capabilities
                        .iter()
                        .map(|value| CanonicalValue::text(value.as_str()))
                        .collect(),
                ),
            ),
            ("step_id", CanonicalValue::text(self.step_id.as_str())),
        ])
    }
}

fn validate_implementation_name(value: &str) -> Result<(), ()> {
    if value.is_empty() || value.len() > MAX_IMPLEMENTATION_NAME_LENGTH {
        return Err(());
    }
    if value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'<' | b'>' | b'-')
    }) {
        Ok(())
    } else {
        Err(())
    }
}

fn compute_implementation_digest(
    step_id: &str,
    qualified_name: &str,
    module_name: &str,
    module_digest: &str,
    package_digest: &str,
    required_capabilities: &[String],
) -> Result<String, DomainError> {
    canonical_digest(&CanonicalValue::object([
        (
            "implementation_digest_version",
            CanonicalValue::text(NATIVE_IMPLEMENTATION_DIGEST_VERSION),
        ),
        (
            "implementation_language",
            CanonicalValue::text(NATIVE_IMPLEMENTATION_LANGUAGE),
        ),
        ("module", CanonicalValue::text(module_name)),
        ("module_digest", CanonicalValue::text(module_digest)),
        ("package_digest", CanonicalValue::text(package_digest)),
        ("qualified_name", CanonicalValue::text(qualified_name)),
        (
            "required_capabilities",
            CanonicalValue::Array(
                required_capabilities
                    .iter()
                    .map(|value| CanonicalValue::text(value.as_str()))
                    .collect(),
            ),
        ),
        ("step_id", CanonicalValue::text(step_id)),
    ]))
}

/// Historical Python-v2 provenance values from the accepted read-only corpus.
///
/// These identify Python source bytes and are never used as native identities.
pub mod legacy {
    /// Python package digest for `mechanical.bracket`.
    pub const MECHANICAL_PACKAGE_DIGEST: &str =
        "97a2ebade1532e16864d81bc1f42b725c584c14250d1ae1f946eff1d4d5937b5";
    /// Python package digest for `building.storey-envelope`.
    pub const BUILDING_PACKAGE_DIGEST: &str =
        "a6d193634794b7a0f28af54ea451924314d61e803ced03610efdbfd6570f40b7";
    /// Python package digest for `wing.parametric-tapered`.
    pub const WING_PACKAGE_DIGEST: &str =
        "108cd68bf1ec08088f526e5ef0b8f2034199298fe7dc443e7c425e46d2358c71";

    /// Python parameter-schema digests.
    pub const MECHANICAL_SCHEMA_DIGESTS: &[&str] =
        &["9fb3d2b068184cedff55fe47664f3cd98fabe0528e4324bdeb9bfc16de284721"];
    /// Python parameter-schema digests.
    pub const BUILDING_SCHEMA_DIGESTS: &[&str] =
        &["8ba15360e4e181e5a78686a84394039a27e176477c0edcb72197b4c104ad8b45"];
    /// Python parameter-schema digests.
    pub const WING_SCHEMA_DIGESTS: &[&str] =
        &["898306c1388c1449d0049693aa66e90493f1aef7e84acaed160c8f48bc41d15e"];

    /// Python implementation digests keyed by catalog step.
    pub const MECHANICAL_IMPLEMENTATIONS: &[(&str, &str)] = &[
        (
            "mechanical.constraint-evaluation",
            "62281ffb23e7732669de991bc7bac6acf21b05404e8ec2ccbd3bdb68a01a419b",
        ),
        (
            "mechanical.geometry-check",
            "a3c6ed56837677e22faff1206fb8dd5517e2c4b94e9932a84e540b3f91c5bc6b",
        ),
        (
            "mechanical.load-check",
            "f72ecbc5b34f7f18f0723a759954ea400e863ce619c776b8b994b3c8eb9bceba",
        ),
        (
            "mechanical.material-check",
            "acafdfa1334cfc090ae552f247384358f5f95d4acacacb4d709706ee839cdf35",
        ),
    ];
    /// Python implementation digests keyed by catalog step.
    pub const BUILDING_IMPLEMENTATIONS: &[(&str, &str)] = &[
        (
            "building.area-model",
            "17c7b04d7be4f4999e4bdd9ef0180aa3ed6ae5cd4aaf8702772db36e4d1aea74",
        ),
        (
            "building.storey-model",
            "6413241c2e547294b449ae570bfc2bac613ed1c47c36fa471bad69f52889b1ad",
        ),
        (
            "building.thermal-envelope",
            "059e747203ac38c87d629e518ff5de4e686e6166aa42b1f15a91e1ac798f05fa",
        ),
    ];
    /// Python implementation digests keyed by catalog step.
    pub const WING_IMPLEMENTATIONS: &[(&str, &str)] = &[
        (
            "wing.aerodynamic-surrogate",
            "a038013612a991ab6da4caaf8290c61a2cadc40e6be1c2636e0486beb00c7d77",
        ),
        (
            "wing.planform-geometry",
            "0935c2012b7086fd24941ead52d5370b167a9ce86dcc9970f108b13517afadc3",
        ),
        (
            "wing.span-load",
            "c0d4b43c4d5c66da474aa6b11440e22f544d0d3b2bbab9d57f5aa2cf4db7f75d",
        ),
    ];
}

/// The required downstream identity boundary.
pub const PROVENANCE_BOUNDARY: &str = "Native identity binds this crate's domain/shared Rust source bytes and the source trees and manifests of the accepted local dependencies kala-budget, kala-json and kala-contracts. External crates, compiler version, target triple, profile flags and built binaries are outside that source-identity boundary. Newly computed native states and revisions carry the native Rust package digests. Stored Python-v2 revisions remain immutable readable legacy records. Legacy Python-v2 prepared actions, approvals and commit bindings are not valid for native identities: an explicit re-preparation and re-approval is required when the recorded identity differs. This crate performs no database, repository or application migration.";

/// One step's legacy-Python and native-Rust implementation digests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImplementationIdentityChange {
    /// Catalog step id.
    pub step_id: String,
    /// Legacy Python-v2 implementation digest (Python source bytes).
    pub legacy_python_digest: String,
    /// Native Rust implementation digest (Rust source bytes and scheme).
    pub native_rust_digest: String,
}

/// The explicit legacy-Python-v2 versus native-Rust identity record.
///
/// This is a comparison record, not a claim that the identities match. A
/// downstream repository/application must treat a native digest as a new
/// identity and require explicit re-preparation and re-approval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvenanceCompatibilityRecord {
    /// Package identifier.
    pub package_id: String,
    /// Legacy scheme name.
    pub legacy_scheme: &'static str,
    /// Native scheme name.
    pub native_scheme: &'static str,
    /// Legacy Python package digest.
    pub legacy_python_package_digest: &'static str,
    /// Native Rust package digest.
    pub native_rust_package_digest: String,
    /// Legacy parameter-schema digests.
    pub legacy_python_schema_digests: &'static [&'static str],
    /// Native parameter-schema digests.
    pub native_rust_schema_digests: Vec<String>,
    /// Whether every parameter-schema digest is byte-identical.
    pub schema_digests_preserved: bool,
    /// Per-step implementation identity differences.
    pub implementation_identities: Vec<ImplementationIdentityChange>,
    /// Always `false`: Python source digests and Rust source digests identify
    /// different implementations.
    pub implementation_identity_equivalent: bool,
    /// Always `true`: prepared actions bound to legacy identities are not
    /// valid for native identities.
    pub requires_reparation_and_reapproval: bool,
    /// The downstream boundary this record requires.
    pub boundary: &'static str,
}

/// Build the legacy-versus-native provenance record for one domain.
pub fn provenance_compatibility(
    domain: DomainName,
) -> Result<ProvenanceCompatibilityRecord, DomainError> {
    let package = package_for(domain);
    build_compatibility_record(package, domain)
}

fn build_compatibility_record(
    package: &dyn DomainPackageApi,
    domain: DomainName,
) -> Result<ProvenanceCompatibilityRecord, DomainError> {
    let (legacy_package, legacy_schema, legacy_implementations) = match domain {
        DomainName::Mechanical => (
            legacy::MECHANICAL_PACKAGE_DIGEST,
            legacy::MECHANICAL_SCHEMA_DIGESTS,
            legacy::MECHANICAL_IMPLEMENTATIONS,
        ),
        DomainName::Building => (
            legacy::BUILDING_PACKAGE_DIGEST,
            legacy::BUILDING_SCHEMA_DIGESTS,
            legacy::BUILDING_IMPLEMENTATIONS,
        ),
        DomainName::Wing => (
            legacy::WING_PACKAGE_DIGEST,
            legacy::WING_SCHEMA_DIGESTS,
            legacy::WING_IMPLEMENTATIONS,
        ),
    };
    let native_schema: Vec<String> = package.schema_digests().to_vec();
    let schema_digests_preserved = native_schema.len() == legacy_schema.len()
        && native_schema
            .iter()
            .map(String::as_str)
            .eq(legacy_schema.iter().copied());
    let implementations: Vec<ImplementationIdentityChange> = package
        .evaluator_implementations()
        .iter()
        .map(|implementation| {
            let legacy_digest = legacy_implementations
                .iter()
                .find(|(step_id, _)| *step_id == implementation.step_id())
                .map(|(_, digest)| (*digest).to_owned())
                .unwrap_or_default();
            ImplementationIdentityChange {
                step_id: implementation.step_id().to_owned(),
                legacy_python_digest: legacy_digest,
                native_rust_digest: implementation.digest().to_owned(),
            }
        })
        .collect();
    Ok(ProvenanceCompatibilityRecord {
        package_id: package.package_id().to_owned(),
        legacy_scheme: LEGACY_PYTHON_IMPLEMENTATION_DIGEST_VERSION,
        native_scheme: NATIVE_IMPLEMENTATION_DIGEST_VERSION,
        legacy_python_package_digest: legacy_package,
        native_rust_package_digest: package.digest().to_owned(),
        legacy_python_schema_digests: legacy_schema,
        native_rust_schema_digests: native_schema,
        schema_digests_preserved,
        implementation_identities: implementations,
        implementation_identity_equivalent: false,
        requires_reparation_and_reapproval: true,
        boundary: PROVENANCE_BOUNDARY,
    })
}
