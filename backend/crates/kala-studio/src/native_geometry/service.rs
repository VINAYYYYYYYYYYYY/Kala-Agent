//! The native design service: the port of the Python `DesignService`.
//!
//! The service owns the whole honest pipeline: resolve a [`DesignIntent`]
//! into a construction script and a declared envelope, hash the canonical
//! script plus the selected CAD provider, ask that provider to build and
//! measure the model, store its STEP and native artifacts, and import the
//! provider's STEP with the view-only importer for the display mesh. It never
//! touches the core `KalaApplication` database.
//!
//! Three invariants matter more than throughput, and they are the Python
//! ones:
//!
//! * **A provider is required.** There is no default and no automatic
//!   OpenCascade fallback.
//! * **The view is the provider's export.** The display mesh comes from
//!   importing the exact STEP bytes the provider returned; OpenCascade is
//!   never the measuring authority for an external model.
//! * **No partial artifact on refusal.** A provider refusal writes
//!   `provider.build.failed` plus `design.rejected` and stores no artifact.
//!
//! Compiled views live in a bounded LRU (32). On a cold miss the service
//! reloads the stored STEP artifact and the provider numbers recorded in the
//! journal, so a drawing, a section or an export does not rerun an external
//! CAD application.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use kala_cad::{Bounds, MeshData, StepImportOptions};
use kala_cad_providers::{
    CadProviderError, CadProviderUnavailableError, MeasuredProperties, ProviderBuild,
    ProviderStatus,
};
use kala_contracts::{ConstructionScript, WireValue};
use kala_design_intent::{DesignIntent, TemplateCall, TemplateResult};
use kala_json::{CanonicalValue, JsonText};
use kala_studio_journal::{AppendOptions, JournalVerification, SessionEvent, StudioSessionJournal};
use sha2::{Digest, Sha256};

use crate::native_geometry::providers::{PROVIDER_OPENCASCADE, ProviderRegistry};
use crate::native_geometry::wire::{canonical_script_json, script_dump};

/// The bounded number of compiled designs the service keeps in memory.
pub const DEFAULT_CACHE_SIZE: usize = 32;
/// The largest number of warnings one design reports.
pub const MAX_WARNINGS: usize = 16;
/// The largest length of one warning.
pub const MAX_WARNING_LENGTH: usize = 256;
/// The envelope tolerance in millimetres.
pub const ENVELOPE_TOLERANCE_MM: f64 = 0.01;
/// The exact single-solid tolerance.
pub const SINGLE_SOLID_TOLERANCE: f64 = 0.0;
/// The exact solid-count tolerance.
pub const SOLID_COUNT_TOLERANCE: f64 = 0.0;
/// The exact B-Rep validity tolerance.
pub const BREP_VALID_TOLERANCE: f64 = 0.0;

/// The event type of a provider build request.
pub const PROVIDER_BUILD_REQUESTED: &str = "provider.build.requested";
/// The event type of a completed provider build.
pub const PROVIDER_BUILD_COMPLETED: &str = "provider.build.completed";
/// The event type of a failed provider build.
pub const PROVIDER_BUILD_FAILED: &str = "provider.build.failed";
/// The event type of a view-only import.
pub const VIEW_IMPORTED: &str = "view.imported";
/// The event type of a refused design.
pub const DESIGN_REJECTED: &str = "design.rejected";
/// The event type of a recorded design intent.
pub const DESIGN_INTENT: &str = "design.intent";
/// The event type of a stored artifact.
pub const ARTIFACT_STORED: &str = "artifact.stored";
/// The event type of a completed design compile.
pub const DESIGN_COMPILED: &str = "design.compiled";
/// The event type of a completed measurement.
pub const DESIGN_MEASURED: &str = "design.measured";
/// The event type of a completed check run.
pub const DESIGN_CHECKED: &str = "design.checked";
/// The event type of a completed export.
pub const EXPORT_COMPLETED: &str = "export.completed";

/// One declared-versus-measured check value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CheckValue {
    /// A boolean expectation or measurement.
    Bool(bool),
    /// A scalar expectation or measurement.
    Float(f64),
    /// An inclusive `(min, max)` pair.
    Pair(f64, f64),
}

/// One check row.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckRow {
    /// The stable row id.
    pub id: String,
    /// The human label.
    pub label: String,
    /// The declared value.
    pub expected: CheckValue,
    /// The measured value, or `None` when the provider reported none.
    pub measured: Option<CheckValue>,
    /// The allowed difference.
    pub tolerance: f64,
    /// The unit of the row.
    pub unit: String,
    /// Whether the row passed.
    pub passed: bool,
}

impl CheckRow {
    /// The honest status of the row.
    #[must_use]
    pub fn status(&self) -> &'static str {
        match self.measured {
            None => "not_reported",
            Some(_) if self.passed => "passed",
            Some(_) => "failed",
        }
    }
}

/// The plain-data result of a view-only STEP import.
///
/// The imported exact shape is deliberately dropped: it is a kernel handle
/// that cannot cross a thread boundary, and the design pipeline never needs
/// it again.
#[derive(Clone, Debug)]
pub struct ImportedView {
    /// The display triangulation.
    pub mesh: MeshData,
    /// The exact imported envelope.
    pub bounds: Bounds,
    /// The number of imported solids.
    pub solids: usize,
    /// The display warnings.
    pub warnings: Vec<String>,
}

/// Every provider-reported fact about one design, plus the display view.
#[derive(Clone, Debug)]
pub struct DesignOutcome {
    /// The design identity.
    pub design_id: String,
    /// The human label.
    pub label: String,
    /// The provider that built the design.
    pub provider: String,
    /// The provider version, when reported.
    pub provider_version: Option<String>,
    /// Who measured the numbers.
    pub measured_by: String,
    /// The construction script, in model field order.
    pub script: WireValue,
    /// The provider's own measured numbers.
    pub measured: MeasuredProperties,
    /// The provider's B-Rep validity statement.
    pub valid: Option<bool>,
    /// The provider's solid count.
    pub solid_count: Option<i64>,
    /// The honesty checks.
    pub checks: Vec<CheckRow>,
    /// Whether every check passed.
    pub verified: bool,
    /// The display mesh imported from the provider's STEP.
    pub view: MeshData,
    /// The imported envelope.
    pub view_bounds: Bounds,
    /// The number of imported solids.
    pub view_solids: usize,
    /// The display warnings.
    pub view_warnings: Vec<String>,
    /// The digest of the canonical script.
    pub script_sha256: String,
    /// The digest of the stored provider STEP.
    pub step_sha256: Option<String>,
    /// The digest of the stored native artifact.
    pub native_sha256: Option<String>,
    /// The kind of the stored native artifact.
    pub native_kind: Option<String>,
    /// The chain digest of the last event of the turn.
    pub journal_head: Option<String>,
    /// The declared tolerances.
    pub tolerances: Vec<String>,
    /// The declared material.
    pub material: Option<String>,
    /// The declared envelope.
    pub declared_envelope: Option<Bounds>,
    /// Whether the design came from the cache or the journal.
    pub cached: bool,
    /// The bounded warnings of the turn.
    pub warnings: Vec<String>,
    /// The number of script components.
    pub component_count: usize,
    /// Whether the script fuses its components.
    pub fused: bool,
}

/// One exported design file and the digest it was stored under.
#[derive(Clone, Debug)]
pub struct ExportOutcome {
    /// The design identity.
    pub design_id: String,
    /// The export format.
    pub format: String,
    /// The download name.
    pub filename: String,
    /// The digest of the exported bytes.
    pub sha256: String,
    /// The exported bytes.
    pub data: Vec<u8>,
}

/// Why a design turn could not be completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesignError {
    /// The intent or the selected provider could not build the design.
    Rejected(String),
    /// No stored design exists for the requested id.
    NotFound(String),
    /// The native geometry kernel is not available in this build.
    Unavailable(String),
    /// The selected provider cannot build on this host.
    ProviderUnavailable(CadProviderUnavailableError),
    /// The request itself is invalid.
    Invalid(String),
    /// The kernel could not produce the drawing or the section.
    Internal(String),
    /// A view-only STEP import refusal.
    ///
    /// The code and the message are the Python exception class's own
    /// attributes; the detail is the instance message.
    StepImport {
        /// The error code the Python class declares.
        code: &'static str,
        /// The class-level message the Python class declares.
        message: &'static str,
        /// The specific refusal text.
        detail: String,
    },
    /// The journal could not record the turn.
    Journal(String),
}

impl std::fmt::Display for DesignError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(message)
            | Self::NotFound(message)
            | Self::Unavailable(message)
            | Self::Invalid(message)
            | Self::Internal(message)
            | Self::Journal(message) => formatter.write_str(message),
            Self::StepImport { detail, .. } => formatter.write_str(detail),
            Self::ProviderUnavailable(error) => formatter.write_str(error.message()),
        }
    }
}

impl std::error::Error for DesignError {}

/// The resolved intent of one design turn.
struct ResolvedIntent {
    script: ConstructionScript,
    envelope: Option<Bounds>,
    tolerances: Vec<String>,
    material: Option<String>,
    label: String,
    template_name: Option<String>,
}

/// The journal records needed to reload one completed design.
struct StoredDesign {
    intent: CanonicalValue,
    build: Option<CanonicalValue>,
}

/// The bounded LRU of compiled designs.
struct DesignCache {
    entries: Vec<(String, DesignOutcome)>,
    capacity: usize,
}

impl DesignCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::new(),
            capacity: capacity.max(1),
        }
    }

    fn get(&mut self, design_id: &str) -> Option<DesignOutcome> {
        let index = self.entries.iter().position(|(key, _)| key == design_id)?;
        let entry = self.entries.remove(index);
        let outcome = entry.1.clone();
        self.entries.push(entry);
        Some(outcome)
    }

    fn put(&mut self, outcome: DesignOutcome) {
        let design_id = outcome.design_id.clone();
        self.entries.retain(|(key, _)| key != &design_id);
        self.entries.push((design_id, outcome));
        while self.entries.len() > self.capacity {
            self.entries.remove(0);
        }
    }
}

/// The Studio design service over one Studio data directory.
pub struct NativeDesignService {
    journal: StudioSessionJournal,
    providers: Mutex<ProviderRegistry>,
    cache: Mutex<DesignCache>,
    geometry: Mutex<()>,
    max_step_bytes: u64,
    data_dir: PathBuf,
}

impl NativeDesignService {
    /// Open the service over one data directory.
    ///
    /// # Errors
    ///
    /// Returns a message when the journal or the data directory cannot be
    /// opened.
    pub fn new(
        data_dir: &Path,
        environ: BTreeMap<String, String>,
        max_step_bytes: u64,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(data_dir)
            .map_err(|_| "the studio data directory could not be created".to_owned())?;
        let artifact_limit = std::cmp::max(
            kala_studio_journal::DEFAULT_MAX_ARTIFACT_BYTES,
            max_step_bytes,
        );
        let journal = StudioSessionJournal::open_with_config(
            data_dir,
            kala_studio_journal::JournalConfig {
                max_artifact_bytes: artifact_limit,
                ..kala_studio_journal::JournalConfig::default()
            },
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            journal,
            providers: Mutex::new(ProviderRegistry::new(environ, max_step_bytes)),
            cache: Mutex::new(DesignCache::new(DEFAULT_CACHE_SIZE)),
            geometry: Mutex::new(()),
            max_step_bytes,
            data_dir: data_dir.to_path_buf(),
        })
    }

    /// The Studio data directory this service writes to.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The STEP import/export ceiling this service enforces.
    #[must_use]
    pub fn max_step_bytes(&self) -> u64 {
        self.max_step_bytes
    }

    /// The journal this service writes.
    #[must_use]
    pub fn journal(&self) -> &StudioSessionJournal {
        &self.journal
    }

    /// Probe every CAD provider and return the honest status rows.
    #[must_use]
    pub fn probe_providers(&self) -> Vec<ProviderStatus> {
        self.providers
            .lock()
            .map(|mut registry| registry.probe_all())
            .unwrap_or_default()
    }

    /// The ids of the providers that can build right now.
    #[must_use]
    pub fn available_provider_ids(&self) -> Vec<String> {
        self.providers
            .lock()
            .map(|mut registry| registry.available_provider_ids())
            .unwrap_or_default()
    }

    /// Build, measure, import and journal one design with one provider.
    ///
    /// # Errors
    ///
    /// Returns the honest refusal: an unknown provider, an unavailable
    /// provider, a build refusal or a journal failure.
    pub fn design(
        &self,
        intent: &DesignIntent,
        provider: &str,
    ) -> Result<DesignOutcome, DesignError> {
        self.validate_provider(provider)?;
        let _guard = self.geometry.lock().map_err(|_| {
            DesignError::Journal("the design geometry lock was poisoned".to_owned())
        })?;
        self.design_locked(intent, provider)
    }

    fn validate_provider(&self, provider: &str) -> Result<(), DesignError> {
        let allowed = kala_cad_providers::ALLOWED_PROVIDER_IDS.join(", ");
        if provider.trim().is_empty() {
            return Err(DesignError::Rejected(format!(
                "a CAD provider is required; choose one of: {allowed}"
            )));
        }
        if !kala_cad_providers::ALLOWED_PROVIDER_IDS.contains(&provider) {
            return Err(DesignError::Rejected(format!(
                "unknown CAD provider '{provider}'; choose one of: {allowed}"
            )));
        }
        if !self
            .providers
            .lock()
            .map(|registry| registry.is_configured(provider))
            .unwrap_or(false)
        {
            return Err(DesignError::Rejected(format!(
                "the CAD provider '{provider}' is not configured; choose one of: {allowed}"
            )));
        }
        Ok(())
    }

    fn design_locked(
        &self,
        intent: &DesignIntent,
        provider_name: &str,
    ) -> Result<DesignOutcome, DesignError> {
        let resolved = self.resolve_intent(intent)?;
        let script_bytes =
            canonical_script_json(&resolved.script).map_err(DesignError::Rejected)?;
        let script_sha256 = sha256_hex(script_bytes.as_bytes());
        let design_id = design_id_for(&script_bytes, provider_name);
        let measured_by = self
            .providers
            .lock()
            .map(|registry| registry.measured_by(provider_name))
            .unwrap_or_else(|_| provider_name.to_owned());

        let cached_in_memory = self
            .cache
            .lock()
            .ok()
            .and_then(|mut cache| cache.get(&design_id))
            .is_some();
        let stored = if cached_in_memory {
            None
        } else {
            self.find_journal_design(&design_id)
        };
        let cached_on_disk = stored
            .as_ref()
            .is_some_and(|record| self.stored_design_is_available(record));
        let cached = cached_in_memory || cached_on_disk;

        let intent_event = self.append(
            DESIGN_INTENT,
            self.intent_payload(&design_id, &resolved, provider_name, &script_sha256, cached),
            &design_id,
        )?;

        if cached {
            let mut outcome = self.load_locked(&design_id, stored)?;
            outcome.cached = true;
            outcome.journal_head = Some(intent_event.event_digest);
            return Ok(outcome);
        }

        let mut providers = self
            .providers
            .lock()
            .map_err(|_| DesignError::Journal("the provider registry was poisoned".to_owned()))?;
        let availability = providers
            .slot_for(provider_name)
            .map(|slot| slot.probe())
            .ok_or_else(|| {
                DesignError::Rejected(format!(
                    "the CAD provider '{provider_name}' is not configured; choose one of: {}",
                    kala_cad_providers::ALLOWED_PROVIDER_IDS.join(", ")
                ))
            })?;
        if !availability.is_available() {
            let reason = availability
                .reason()
                .unwrap_or("the provider reported no reason")
                .to_owned();
            self.provider_failed(&design_id, provider_name, "provider_unavailable", &reason)?;
            self.reject(&design_id, &reason)?;
            return Err(DesignError::ProviderUnavailable(
                CadProviderUnavailableError::new(
                    provider_name,
                    format!("the {provider_name} CAD provider is not available"),
                    Some(reason),
                    providers.available_provider_ids(),
                ),
            ));
        }

        self.append(
            PROVIDER_BUILD_REQUESTED,
            canonical_object(vec![
                ("design_id", canonical_text(&design_id)),
                ("provider", canonical_text(provider_name)),
                (
                    "provider_version",
                    match availability.version() {
                        Some(version) => canonical_text(version),
                        None => CanonicalValue::Null,
                    },
                ),
                ("script_sha256", canonical_text(&script_sha256)),
            ]),
            &design_id,
        )?;

        let build = match providers.build(provider_name, &resolved.script) {
            Ok(build) => build,
            Err(CadProviderError::Unavailable(error)) => {
                self.provider_failed(
                    &design_id,
                    provider_name,
                    "provider_unavailable",
                    error.reason(),
                )?;
                self.reject(&design_id, error.message())?;
                return Err(DesignError::ProviderUnavailable(error));
            }
            Err(CadProviderError::Build(error)) => {
                let reason = error.message().to_owned();
                self.provider_failed(&design_id, provider_name, "build_refused", &reason)?;
                self.reject(&design_id, &reason)?;
                return Err(DesignError::Rejected(reason));
            }
        };

        self.validate_build(&design_id, provider_name, &build)?;
        let imported = self.import_view(&design_id, provider_name, &build)?;

        let step_sha256 = sha256_hex(build.step_bytes());
        let native_kind = build.native_kind().map(str::to_owned);
        let native_sha256 = build.native_bytes().map(sha256_hex);
        let checks = build_provider_checks(
            build.measured(),
            build.valid(),
            build.solid_count(),
            expected_solid_count(&resolved.script),
            resolved.envelope.as_ref(),
        );
        let verified = !checks.is_empty() && checks.iter().all(|check| check.passed);

        self.append(
            PROVIDER_BUILD_COMPLETED,
            canonical_object(vec![
                ("design_id", canonical_text(&design_id)),
                ("provider", canonical_text(provider_name)),
                (
                    "provider_version",
                    match build.provider_version() {
                        Some(version) => canonical_text(version),
                        None => CanonicalValue::Null,
                    },
                ),
                ("measured_by", canonical_text(&measured_by)),
                ("script_sha256", canonical_text(&script_sha256)),
                ("step_sha256", canonical_text(&step_sha256)),
                (
                    "native_sha256",
                    match &native_sha256 {
                        Some(digest) => canonical_text(digest),
                        None => CanonicalValue::Null,
                    },
                ),
                (
                    "native_kind",
                    match &native_kind {
                        Some(kind) => canonical_text(kind),
                        None => CanonicalValue::Null,
                    },
                ),
                ("measured", measured_payload(build.measured())),
                (
                    "valid",
                    match build.valid() {
                        Some(value) => CanonicalValue::Bool(value),
                        None => CanonicalValue::Null,
                    },
                ),
                (
                    "solid_count",
                    match build.solid_count() {
                        Some(count) => CanonicalValue::integer(count),
                        None => CanonicalValue::Null,
                    },
                ),
                ("warnings", canonical_array(build.warnings())),
            ]),
            &design_id,
        )?;

        let stored_script = self.put_artifact(script_bytes.as_bytes())?;
        if stored_script != script_sha256 {
            return Err(DesignError::Rejected(
                "the stored script artifact digest did not match the script digest".to_owned(),
            ));
        }
        self.append(
            ARTIFACT_STORED,
            canonical_object(vec![
                ("design_id", canonical_text(&design_id)),
                ("provider", canonical_text(provider_name)),
                ("kind", canonical_text("script")),
                ("sha256", canonical_text(&stored_script)),
                ("size_bytes", CanonicalValue::integer(script_bytes.len())),
            ]),
            &design_id,
        )?;

        let stored_step = self.put_artifact(build.step_bytes())?;
        if stored_step != step_sha256 {
            return Err(DesignError::Rejected(
                "the stored STEP artifact digest did not match the provider export".to_owned(),
            ));
        }
        self.append(
            ARTIFACT_STORED,
            canonical_object(vec![
                ("design_id", canonical_text(&design_id)),
                ("provider", canonical_text(provider_name)),
                ("kind", canonical_text("step")),
                ("sha256", canonical_text(&stored_step)),
                (
                    "size_bytes",
                    CanonicalValue::integer(build.step_bytes().len()),
                ),
                (
                    "source",
                    canonical_text(&format!("provider:{provider_name}")),
                ),
            ]),
            &design_id,
        )?;

        if let Some(native_bytes) = build.native_bytes() {
            let stored_native = self.put_artifact(native_bytes)?;
            if Some(&stored_native) != native_sha256.as_ref() {
                return Err(DesignError::Rejected(
                    "the stored native artifact digest did not match the provider export"
                        .to_owned(),
                ));
            }
            self.append(
                ARTIFACT_STORED,
                canonical_object(vec![
                    ("design_id", canonical_text(&design_id)),
                    ("provider", canonical_text(provider_name)),
                    (
                        "kind",
                        canonical_text(native_kind.as_deref().unwrap_or("native")),
                    ),
                    ("sha256", canonical_text(&stored_native)),
                    ("size_bytes", CanonicalValue::integer(native_bytes.len())),
                    (
                        "source",
                        canonical_text(&format!("provider:{provider_name}")),
                    ),
                ]),
                &design_id,
            )?;
        }

        self.append(
            VIEW_IMPORTED,
            canonical_object(vec![
                ("design_id", canonical_text(&design_id)),
                ("provider", canonical_text(provider_name)),
                ("step_sha256", canonical_text(&stored_step)),
                ("viewer", canonical_text("opencascade")),
                ("role", canonical_text("view-only")),
                ("solids", CanonicalValue::integer(imported.solids)),
                (
                    "vertices",
                    CanonicalValue::integer(imported.mesh.vertex_count()),
                ),
                (
                    "triangles",
                    CanonicalValue::integer(imported.mesh.triangle_count()),
                ),
                (
                    "bounds",
                    canonical_object(vec![
                        ("min", canonical_float_array(imported.bounds.min_mm)),
                        ("max", canonical_float_array(imported.bounds.max_mm)),
                    ]),
                ),
                ("warnings", canonical_array(&imported.warnings)),
            ]),
            &design_id,
        )?;

        if provider_name == PROVIDER_OPENCASCADE {
            self.append(
                DESIGN_COMPILED,
                canonical_object(vec![
                    ("design_id", canonical_text(&design_id)),
                    ("provider", canonical_text(provider_name)),
                    (
                        "components",
                        CanonicalValue::integer(resolved.script.components().len()),
                    ),
                    (
                        "fused",
                        CanonicalValue::Bool(script_fused(&resolved.script)),
                    ),
                    ("warnings", canonical_array(build.warnings())),
                    ("envelope", envelope_payload(resolved.envelope.as_ref())),
                    ("tolerances", canonical_array(&resolved.tolerances)),
                    (
                        "material",
                        match &resolved.material {
                            Some(material) => canonical_text(material),
                            None => CanonicalValue::Null,
                        },
                    ),
                    ("label", canonical_text(&resolved.label)),
                ]),
                &design_id,
            )?;
        }

        self.append(
            DESIGN_MEASURED,
            canonical_object(vec![
                ("design_id", canonical_text(&design_id)),
                ("provider", canonical_text(provider_name)),
                ("measured_by", canonical_text(&measured_by)),
                (
                    "volume_mm3",
                    measured_optional_float(build.measured().volume_mm3()),
                ),
                (
                    "surface_area_mm2",
                    measured_optional_float(build.measured().area_mm2()),
                ),
                (
                    "centroid_mm",
                    match build.measured().centroid_mm() {
                        Some(centroid) => canonical_float_array(centroid),
                        None => CanonicalValue::Null,
                    },
                ),
                (
                    "bounds",
                    match build.measured().bounds() {
                        Some(bounds) => canonical_object(vec![
                            ("min", canonical_float_array(bounds.min_mm())),
                            ("max", canonical_float_array(bounds.max_mm())),
                        ]),
                        None => CanonicalValue::Null,
                    },
                ),
                ("mass_g", measured_optional_float(build.measured().mass_g())),
                (
                    "valid",
                    match build.valid() {
                        Some(value) => CanonicalValue::Bool(value),
                        None => CanonicalValue::Null,
                    },
                ),
                (
                    "solid_count",
                    match build.solid_count() {
                        Some(count) => CanonicalValue::integer(count),
                        None => CanonicalValue::Null,
                    },
                ),
            ]),
            &design_id,
        )?;

        let checked_event = self.append(
            DESIGN_CHECKED,
            canonical_object(vec![
                ("design_id", canonical_text(&design_id)),
                ("provider", canonical_text(provider_name)),
                ("verified", CanonicalValue::Bool(verified)),
                (
                    "checks",
                    CanonicalValue::Array(
                        checks
                            .iter()
                            .map(|check| {
                                canonical_object(vec![
                                    ("id", canonical_text(&check.id)),
                                    ("label", canonical_text(&check.label)),
                                    ("expected", check_value_payload(&check.expected)),
                                    (
                                        "measured",
                                        match check.measured {
                                            Some(value) => check_value_payload(&value),
                                            None => CanonicalValue::Null,
                                        },
                                    ),
                                    ("tolerance", canonical_float(check.tolerance)),
                                    ("unit", canonical_text(&check.unit)),
                                    ("passed", CanonicalValue::Bool(check.passed)),
                                    ("status", canonical_text(check.status())),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ]),
            &design_id,
        )?;

        let outcome = self.make_outcome(
            &design_id,
            &resolved,
            provider_name,
            build.provider_version().map(str::to_owned),
            &measured_by,
            *build.measured(),
            build.valid(),
            build.solid_count(),
            checks,
            verified,
            imported.mesh,
            imported.bounds,
            imported.solids,
            imported.warnings,
            Some(stored_step),
            native_sha256,
            native_kind,
            Some(checked_event.event_digest),
            build.warnings().to_vec(),
            false,
            script_sha256,
        );
        if let Ok(mut cache) = self.cache.lock() {
            cache.put(outcome.clone());
        }
        Ok(outcome)
    }

    /// Load a stored design from the cache or from the journal.
    ///
    /// # Errors
    ///
    /// Returns a refusal when no design is stored under the id, when the
    /// stored design is incomplete, or when the stored artifacts cannot be
    /// read back.
    pub fn load(&self, design_id: &str) -> Result<DesignOutcome, DesignError> {
        validate_design_id(design_id)?;
        let _guard = self.geometry.lock().map_err(|_| {
            DesignError::Journal("the design geometry lock was poisoned".to_owned())
        })?;
        self.load_locked(design_id, None)
    }

    fn load_locked(
        &self,
        design_id: &str,
        stored: Option<StoredDesign>,
    ) -> Result<DesignOutcome, DesignError> {
        if let Ok(mut cache) = self.cache.lock() {
            if let Some(mut outcome) = cache.get(design_id) {
                outcome.cached = true;
                return Ok(outcome);
            }
        }
        let record = match stored {
            Some(record) => record,
            None => self.find_journal_design(design_id).ok_or_else(|| {
                DesignError::NotFound("no design is stored under that id".to_owned())
            })?,
        };
        let Some(build) = record.build else {
            return Err(DesignError::NotFound(
                "the design has no completed provider build".to_owned(),
            ));
        };
        let provider_name = object_get(&build, "provider")
            .and_then(|value| value.as_text())
            .map(|text| text.to_string_lossy())
            .ok_or_else(|| {
                DesignError::Rejected(
                    "the stored design does not name a known CAD provider".to_owned(),
                )
            })?;
        if !kala_cad_providers::ALLOWED_PROVIDER_IDS.contains(&provider_name.as_str()) {
            return Err(DesignError::Rejected(
                "the stored design does not name a known CAD provider".to_owned(),
            ));
        }
        let script_sha256 = object_get(&build, "script_sha256")
            .and_then(|value| value.as_text())
            .map(|text| text.to_string_lossy())
            .ok_or_else(|| {
                DesignError::Rejected("the stored design has no script artifact".to_owned())
            })?;
        if !self.has_artifact(&script_sha256) {
            return Err(DesignError::Rejected(
                "the stored design has no script artifact".to_owned(),
            ));
        }
        let script_text = self
            .journal
            .get_artifact_text(&script_sha256)
            .map_err(|_| {
                DesignError::Rejected("the stored script artifact is missing".to_owned())
            })?;
        let script = kala_contracts::load_construction_json_str(&script_text).map_err(|error| {
            DesignError::Rejected(format!(
                "the stored script artifact is not a valid construction script: {error}"
            ))
        })?;
        let script_bytes = canonical_script_json(&script).map_err(DesignError::Rejected)?;
        if design_id_for(&script_bytes, &provider_name) != design_id {
            return Err(DesignError::Rejected(
                "the stored script artifact does not hash to its design id".to_owned(),
            ));
        }

        let step_sha256 = object_get(&build, "step_sha256")
            .and_then(|value| value.as_text())
            .map(|text| text.to_string_lossy())
            .ok_or_else(|| {
                DesignError::Rejected("the stored design has no STEP artifact".to_owned())
            })?;
        if !self.has_artifact(&step_sha256) {
            return Err(DesignError::Rejected(
                "the stored design has no STEP artifact".to_owned(),
            ));
        }
        let step_bytes = self.journal.get_artifact(&step_sha256).map_err(|_| {
            DesignError::Rejected("the stored STEP artifact could not be imported".to_owned())
        })?;
        let imported = self.import_stored_view(&step_bytes)?;

        let envelope = envelope_from_payload(object_get(&record.intent, "envelope"));
        let tolerances = string_tuple(object_get(&record.intent, "tolerances"));
        let material = object_get(&record.intent, "material")
            .and_then(|value| value.as_text())
            .map(|text| text.to_string_lossy());
        let label = object_get(&record.intent, "label")
            .and_then(|value| value.as_text())
            .filter(|text| !text.is_empty())
            .map(|text| text.to_string_lossy())
            .unwrap_or_else(|| script.summary().to_string_lossy());
        let resolved = ResolvedIntent {
            script,
            envelope,
            tolerances,
            material,
            label,
            template_name: None,
        };
        let measured = measured_from_payload(object_get(&build, "measured"));
        let valid = match object_get(&build, "valid") {
            Some(CanonicalValue::Bool(value)) => Some(*value),
            _ => None,
        };
        let solid_count = match object_get(&build, "solid_count") {
            Some(CanonicalValue::Integer(value)) => value
                .to_string()
                .parse::<i64>()
                .ok()
                .filter(|count| *count >= 0),
            _ => None,
        };
        let checks = build_provider_checks(
            &measured,
            valid,
            solid_count,
            expected_solid_count(&resolved.script),
            resolved.envelope.as_ref(),
        );
        let verified = !checks.is_empty() && checks.iter().all(|check| check.passed);
        let provider_version = object_get(&build, "provider_version")
            .and_then(|value| value.as_text())
            .map(|text| text.to_string_lossy());
        let measured_by = object_get(&build, "measured_by")
            .and_then(|value| value.as_text())
            .filter(|text| !text.is_empty())
            .map(|text| text.to_string_lossy())
            .unwrap_or_else(|| provider_name.clone());
        let native_kind = object_get(&build, "native_kind")
            .and_then(|value| value.as_text())
            .map(|text| text.to_string_lossy());
        let native_sha256 = object_get(&build, "native_sha256")
            .and_then(|value| value.as_text())
            .map(|text| text.to_string_lossy());
        let outcome = self.make_outcome(
            design_id,
            &resolved,
            &provider_name,
            provider_version,
            &measured_by,
            measured,
            valid,
            solid_count,
            checks,
            verified,
            imported.mesh,
            imported.bounds,
            imported.solids,
            imported.warnings,
            Some(step_sha256),
            native_sha256,
            native_kind,
            None,
            string_tuple(object_get(&build, "warnings")),
            true,
            sha256_hex(script_bytes.as_bytes()),
        );
        if let Ok(mut cache) = self.cache.lock() {
            cache.put(outcome.clone());
        }
        Ok(outcome)
    }

    /// Export the provider's stored STEP, or a display STL, for a design.
    ///
    /// # Errors
    ///
    /// Returns a refusal when the format is unknown, when the design has no
    /// stored STEP artifact or when the stored artifact is missing.
    pub fn export(&self, design_id: &str, format: &str) -> Result<ExportOutcome, DesignError> {
        let extension = format.trim().to_ascii_lowercase();
        if extension != "step" && extension != "stl" {
            return Err(DesignError::Invalid(
                "the export format must be step or stl".to_owned(),
            ));
        }
        let _guard = self.geometry.lock().map_err(|_| {
            DesignError::Journal("the design geometry lock was poisoned".to_owned())
        })?;
        let outcome = self.load_locked(design_id, None)?;
        let data = if extension == "step" {
            let Some(digest) = outcome.step_sha256.as_ref() else {
                return Err(DesignError::Invalid(
                    "the design has no stored provider STEP artifact".to_owned(),
                ));
            };
            self.journal.get_artifact(digest).map_err(|_| {
                DesignError::Invalid("the stored provider STEP artifact is missing".to_owned())
            })?
        } else {
            kala_cad::stl_bytes(&outcome.view, kala_cad::STL_HEADER_FILL)
                .map_err(|error| DesignError::Rejected(error.to_string()))?
        };
        let digest = sha256_hex(&data);
        if extension == "step" && Some(&digest) != outcome.step_sha256.as_ref() {
            return Err(DesignError::Invalid(
                "the stored STEP artifact does not match its digest".to_owned(),
            ));
        }
        let stored = self.put_artifact(&data)?;
        if stored != digest {
            return Err(DesignError::Invalid(
                "the exported artifact digest did not match its bytes".to_owned(),
            ));
        }
        let filename = format!("kala-studio-{}.{extension}", &design_id[..12]);
        self.append(
            ARTIFACT_STORED,
            canonical_object(vec![
                ("design_id", canonical_text(design_id)),
                ("provider", canonical_text(&outcome.provider)),
                ("kind", canonical_text(&extension)),
                ("sha256", canonical_text(&digest)),
                ("size_bytes", CanonicalValue::integer(data.len())),
            ]),
            design_id,
        )?;
        self.append(
            EXPORT_COMPLETED,
            canonical_object(vec![
                ("design_id", canonical_text(design_id)),
                ("provider", canonical_text(&outcome.provider)),
                ("format", canonical_text(&extension)),
                ("sha256", canonical_text(&digest)),
                ("size_bytes", CanonicalValue::integer(data.len())),
                ("filename", canonical_text(&filename)),
            ]),
            design_id,
        )?;
        Ok(ExportOutcome {
            design_id: design_id.to_owned(),
            format: extension,
            filename,
            sha256: digest,
            data,
        })
    }

    /// The view-only import of externally built STEP bytes.
    ///
    /// The original bytes are stored in the session artifact store by content
    /// digest and two journal events record the result: `artifact.stored` for
    /// the STEP blob and `view.imported` for the derived display view.
    ///
    /// # Errors
    ///
    /// Returns the typed import refusal: an empty file, a file above the
    /// configured ceiling, a malformed file or a file the mesh budget cannot
    /// tessellate.
    pub fn import_and_store(&self, bytes: &[u8]) -> Result<ImportedView, DesignError> {
        if bytes.is_empty() {
            return Err(DesignError::StepImport {
                code: "invalid_step",
                message: "the STEP file is malformed or carries no transferable shape",
                detail: "the STEP file is empty".to_owned(),
            });
        }
        if bytes.len() as u64 > self.max_step_bytes {
            return Err(DesignError::StepImport {
                code: "payload_too_large",
                message: "the STEP file exceeds the configured size limit",
                detail: format!(
                    "the STEP file must not exceed {} bytes",
                    self.max_step_bytes
                ),
            });
        }
        if !bytes.starts_with(b"ISO-10303-21") {
            return Err(DesignError::StepImport {
                code: "invalid_step",
                message: "the STEP file is malformed or carries no transferable shape",
                detail: "the file does not start with an ISO-10303-21 header".to_owned(),
            });
        }
        let view = self.import_bytes(bytes).map_err(|error| {
            let (code, message) = match error.code() {
                "payload_too_large" => (
                    "payload_too_large",
                    "the STEP file exceeds the configured size limit",
                ),
                "step_too_complex" => (
                    "step_too_complex",
                    "the imported STEP shape exceeds the mesh limits",
                ),
                _ => (
                    "invalid_step",
                    "the STEP file is malformed or carries no transferable shape",
                ),
            };
            DesignError::StepImport {
                code,
                message,
                detail: error.message().to_owned(),
            }
        })?;
        let digest = sha256_hex(bytes);
        let stored = self.put_artifact(bytes)?;
        if stored != digest {
            return Err(DesignError::Journal(
                "the stored STEP digest did not match its bytes".to_owned(),
            ));
        }
        self.append(
            ARTIFACT_STORED,
            canonical_object(vec![
                ("kind", canonical_text("step")),
                ("sha256", canonical_text(&stored)),
                ("size_bytes", CanonicalValue::integer(bytes.len())),
                ("source", canonical_text("step_import")),
            ]),
            &stored,
        )?;
        self.append(
            VIEW_IMPORTED,
            canonical_object(vec![
                ("step_sha256", canonical_text(&stored)),
                ("viewer", canonical_text("opencascade")),
                ("role", canonical_text("view-only")),
                ("solids", CanonicalValue::integer(view.solids)),
                (
                    "vertices",
                    CanonicalValue::integer(view.mesh.vertex_count()),
                ),
                (
                    "triangles",
                    CanonicalValue::integer(view.mesh.triangle_count()),
                ),
                (
                    "bounds",
                    canonical_object(vec![
                        ("min", canonical_float_array(view.bounds.min_mm)),
                        ("max", canonical_float_array(view.bounds.max_mm)),
                    ]),
                ),
                ("warnings", canonical_array(&view.warnings)),
            ]),
            &stored,
        )?;
        Ok(view)
    }

    /// The hidden-line views of a stored design's imported shape.
    ///
    /// The shape is the provider's own STEP artifact, imported again for the
    /// drawing: the view is always the geometry the provider exported.
    ///
    /// # Errors
    ///
    /// Returns a refusal when no design is stored under the id, when the
    /// stored STEP artifact cannot be read back, or when the kernel cannot
    /// draw the shape.
    pub fn drawing(
        &self,
        design_id: &str,
        views: &[&str],
    ) -> Result<kala_cad::Drawing, DesignError> {
        let _guard = self.geometry.lock().map_err(|_| {
            DesignError::Journal("the design geometry lock was poisoned".to_owned())
        })?;
        let shape = self.imported_shape(design_id)?;
        kala_cad::hlr_views_with_options(
            &shape,
            views,
            kala_cad::DEFAULT_CHORD_TOLERANCE_MM,
            kala_cad::MAX_POINTS_PER_VIEW,
        )
        .map_err(drawing_error)
    }

    /// One section cut of a stored design's imported shape.
    ///
    /// # Errors
    ///
    /// Returns a refusal when no design is stored under the id, when the
    /// stored STEP artifact cannot be read back, or when the kernel cannot cut
    /// the shape.
    pub fn section(
        &self,
        design_id: &str,
        plane: &str,
        offset_mm: Option<f64>,
    ) -> Result<kala_cad::Section, DesignError> {
        let _guard = self.geometry.lock().map_err(|_| {
            DesignError::Journal("the design geometry lock was poisoned".to_owned())
        })?;
        let shape = self.imported_shape(design_id)?;
        kala_cad::section_with_options(
            &shape,
            plane,
            offset_mm,
            kala_cad::DEFAULT_SECTION_CHORD_TOLERANCE_MM,
        )
        .map_err(drawing_error)
    }

    /// The exact shape of a stored design's provider STEP artifact.
    fn imported_shape(&self, design_id: &str) -> Result<kala_cad::Shape, DesignError> {
        validate_design_id(design_id)?;
        let record = self
            .find_journal_design(design_id)
            .ok_or_else(|| DesignError::NotFound("no design is stored under that id".to_owned()))?;
        let Some(build) = record.build else {
            return Err(DesignError::NotFound(
                "the design has no completed provider build".to_owned(),
            ));
        };
        let step_sha256 = object_get(&build, "step_sha256")
            .and_then(|value| value.as_text())
            .map(|text| text.to_string_lossy())
            .ok_or_else(|| {
                DesignError::Rejected("the stored design has no STEP artifact".to_owned())
            })?;
        if !self.has_artifact(&step_sha256) {
            return Err(DesignError::Rejected(
                "the stored design has no STEP artifact".to_owned(),
            ));
        }
        let bytes = self.journal.get_artifact(&step_sha256).map_err(|_| {
            DesignError::Rejected("the stored STEP artifact could not be imported".to_owned())
        })?;
        if bytes.len() as u64 > self.max_step_bytes {
            return Err(DesignError::StepImport {
                code: "payload_too_large",
                message: "the STEP file exceeds the configured size limit",
                detail: format!(
                    "the STEP file must not exceed {} bytes",
                    self.max_step_bytes
                ),
            });
        }
        kala_cad::step_shape(&bytes).map_err(|_| {
            DesignError::Rejected(
                "the stored STEP artifact could not be imported: StepImportError".to_owned(),
            )
        })
    }

    /// One bounded page of Studio journal events.
    ///
    /// # Errors
    ///
    /// Returns the journal's own refusal for an invalid cursor or limit.
    pub fn journal_page(&self, after: i64, limit: i64) -> Result<Vec<SessionEvent>, DesignError> {
        self.journal
            .page(after, limit)
            .map_err(|error| DesignError::Journal(error.to_string()))
    }

    /// The chain verification verdict.
    ///
    /// # Errors
    ///
    /// Returns the journal's own refusal when the chain cannot be walked.
    pub fn verify(&self) -> Result<JournalVerification, DesignError> {
        self.journal
            .verify()
            .map_err(|error| DesignError::Journal(error.to_string()))
    }

    // ---------------------------------------------------------------- internals

    fn resolve_intent(&self, intent: &DesignIntent) -> Result<ResolvedIntent, DesignError> {
        let tolerances: Vec<String> = intent
            .tolerances()
            .iter()
            .map(|value| value.to_string_lossy())
            .collect();
        let material = intent.material().map(|value| value.to_string_lossy());
        let Some(call) = intent.template() else {
            let Some(script) = intent.script() else {
                return Err(DesignError::Rejected(
                    "the design intent carries neither a template nor a script".to_owned(),
                ));
            };
            let label = intent
                .label()
                .map(|value| value.to_string_lossy())
                .unwrap_or_else(|| script.summary().to_string_lossy());
            return Ok(ResolvedIntent {
                script: script.clone(),
                envelope: None,
                tolerances,
                material,
                label,
                template_name: None,
            });
        };
        let result: TemplateResult = match call {
            TemplateCall::Lbracket(call) => kala_design_intent::l_bracket(
                call.width(),
                call.depth(),
                call.height(),
                call.wall(),
                call.bore_d(),
                call.hole_d(),
                i64::from(call.hole_count().value()),
                call.edge(),
            ),
            TemplateCall::SteppedShaft(call) => kala_design_intent::stepped_shaft(
                &call
                    .segments()
                    .iter()
                    .map(|segment| kala_design_intent::TemplateShaftSegment {
                        diameter_mm: segment.diameter(),
                        length_mm: segment.length(),
                    })
                    .collect::<Vec<_>>(),
            ),
            TemplateCall::Housing(call) => kala_design_intent::housing_envelope(
                call.width(),
                call.depth(),
                call.height(),
                call.wall(),
            ),
        }
        .map_err(|error| DesignError::Rejected(error.message().to_owned()))?;
        let envelope = Bounds::new(result.envelope().min_mm(), result.envelope().max_mm())
            .map_err(|error| DesignError::Rejected(error.message().to_owned()))?;
        let mut resolved_tolerances = tolerances;
        if resolved_tolerances.is_empty() {
            resolved_tolerances = result.tolerances().to_vec();
        }
        let label = intent
            .label()
            .map(|value| value.to_string_lossy())
            .unwrap_or_else(|| result.label().to_owned());
        Ok(ResolvedIntent {
            script: result.script().clone(),
            envelope: Some(envelope),
            tolerances: resolved_tolerances,
            material,
            label,
            template_name: Some(call.tag().to_owned()),
        })
    }

    fn validate_build(
        &self,
        design_id: &str,
        provider_name: &str,
        build: &ProviderBuild,
    ) -> Result<(), DesignError> {
        if build.step_bytes().len() as u64 > self.max_step_bytes {
            let reason = format!(
                "the {provider_name} STEP export is larger than the configured {} byte STEP \
                 import limit",
                self.max_step_bytes
            );
            self.provider_failed(design_id, provider_name, "step_too_large", &reason)?;
            self.reject(design_id, &reason)?;
            return Err(DesignError::Rejected(reason));
        }
        if !build.step_bytes().starts_with(b"ISO-10303-21") {
            let reason = format!("the {provider_name} export is not an ISO-10303-21 STEP file");
            self.provider_failed(design_id, provider_name, "invalid_step_export", &reason)?;
            self.reject(design_id, &reason)?;
            return Err(DesignError::Rejected(reason));
        }
        let artifact_limit = std::cmp::max(
            kala_studio_journal::DEFAULT_MAX_ARTIFACT_BYTES,
            self.max_step_bytes,
        );
        if let Some(native_bytes) = build.native_bytes() {
            if native_bytes.len() as u64 > artifact_limit {
                let reason = format!(
                    "the {provider_name} native export is larger than the configured \
                     {artifact_limit} byte artifact limit"
                );
                self.provider_failed(design_id, provider_name, "native_too_large", &reason)?;
                self.reject(design_id, &reason)?;
                return Err(DesignError::Rejected(reason));
            }
        }
        Ok(())
    }

    fn import_view(
        &self,
        design_id: &str,
        provider_name: &str,
        build: &ProviderBuild,
    ) -> Result<ImportedView, DesignError> {
        match self.import_bytes(build.step_bytes()) {
            Ok(view) => Ok(view),
            Err(error) => {
                let reason = format!(
                    "the {provider_name} STEP export could not be imported for display: {error}"
                );
                self.provider_failed(design_id, provider_name, error.code(), &reason)?;
                self.reject(design_id, &reason)?;
                Err(DesignError::Rejected(reason))
            }
        }
    }

    fn import_stored_view(&self, bytes: &[u8]) -> Result<ImportedView, DesignError> {
        self.import_bytes(bytes).map_err(|error| {
            DesignError::Rejected(format!(
                "the stored STEP artifact could not be imported: {}",
                error.message()
            ))
        })
    }

    fn import_bytes(&self, bytes: &[u8]) -> Result<ImportedView, kala_cad::StepImportError> {
        let outcome = kala_cad::import_step_view(
            bytes,
            StepImportOptions {
                max_step_bytes: self.max_step_bytes,
                ..StepImportOptions::default()
            },
        )?;
        Ok(ImportedView {
            mesh: outcome.mesh,
            bounds: outcome.bounds,
            solids: outcome.solids,
            warnings: outcome.warnings,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn make_outcome(
        &self,
        design_id: &str,
        resolved: &ResolvedIntent,
        provider: &str,
        provider_version: Option<String>,
        measured_by: &str,
        measured: MeasuredProperties,
        valid: Option<bool>,
        solid_count: Option<i64>,
        checks: Vec<CheckRow>,
        verified: bool,
        view: MeshData,
        view_bounds: Bounds,
        view_solids: usize,
        view_warnings: Vec<String>,
        step_sha256: Option<String>,
        native_sha256: Option<String>,
        native_kind: Option<String>,
        journal_head: Option<String>,
        warnings: Vec<String>,
        cached: bool,
        script_sha256: String,
    ) -> DesignOutcome {
        let mut combined = warnings.clone();
        combined.extend(view_warnings.iter().cloned());
        DesignOutcome {
            design_id: design_id.to_owned(),
            label: resolved.label.clone(),
            provider: provider.to_owned(),
            provider_version,
            measured_by: measured_by.to_owned(),
            script: script_dump(&resolved.script),
            measured,
            valid,
            solid_count,
            checks,
            verified,
            view,
            view_bounds,
            view_solids,
            view_warnings,
            script_sha256,
            step_sha256,
            native_sha256,
            native_kind,
            journal_head,
            tolerances: resolved.tolerances.clone(),
            material: resolved.material.clone(),
            declared_envelope: resolved.envelope,
            cached,
            warnings: bounded_warnings(&combined),
            component_count: resolved.script.components().len(),
            fused: script_fused(&resolved.script),
        }
    }

    fn intent_payload(
        &self,
        design_id: &str,
        resolved: &ResolvedIntent,
        provider: &str,
        script_sha256: &str,
        cache_hit: bool,
    ) -> CanonicalValue {
        canonical_object(vec![
            ("design_id", canonical_text(design_id)),
            ("provider", canonical_text(provider)),
            ("script_sha256", canonical_text(script_sha256)),
            (
                "template",
                match &resolved.template_name {
                    Some(name) => canonical_text(name),
                    None => CanonicalValue::Null,
                },
            ),
            ("label", canonical_text(&resolved.label)),
            (
                "material",
                match &resolved.material {
                    Some(material) => canonical_text(material),
                    None => CanonicalValue::Null,
                },
            ),
            ("tolerances", canonical_array(&resolved.tolerances)),
            ("envelope", envelope_payload(resolved.envelope.as_ref())),
            ("cache_hit", CanonicalValue::Bool(cache_hit)),
        ])
    }

    fn reject(&self, design_id: &str, reason: &str) -> Result<(), DesignError> {
        self.append(
            DESIGN_REJECTED,
            canonical_object(vec![
                ("design_id", canonical_text(design_id)),
                ("reason", canonical_text(&truncate_chars(reason, 512))),
            ]),
            design_id,
        )
        .map(|_| ())
    }

    fn provider_failed(
        &self,
        design_id: &str,
        provider: &str,
        code: &str,
        reason: &str,
    ) -> Result<(), DesignError> {
        self.append(
            PROVIDER_BUILD_FAILED,
            canonical_object(vec![
                ("design_id", canonical_text(design_id)),
                ("provider", canonical_text(provider)),
                ("code", canonical_text(&truncate_chars(code, 64))),
                ("reason", canonical_text(&truncate_chars(reason, 512))),
            ]),
            design_id,
        )
        .map(|_| ())
    }

    fn append(
        &self,
        event_type: &str,
        payload: CanonicalValue,
        aggregate_id: &str,
    ) -> Result<SessionEvent, DesignError> {
        self.journal
            .append_with(
                event_type,
                &payload,
                &AppendOptions::default().with_aggregate_id(aggregate_id.to_owned()),
            )
            .map_err(|error| DesignError::Journal(error.to_string()))
    }

    fn put_artifact(&self, data: &[u8]) -> Result<String, DesignError> {
        self.journal
            .put_artifact(data)
            .map_err(|error| DesignError::Journal(error.to_string()))
    }

    fn has_artifact(&self, digest: &str) -> bool {
        self.journal.has_artifact(digest).unwrap_or(false)
    }

    fn find_journal_design(&self, design_id: &str) -> Option<StoredDesign> {
        let mut intent: Option<CanonicalValue> = None;
        let mut build: Option<CanonicalValue> = None;
        let mut after = 0_i64;
        let page_size = self.journal.max_page_size();
        loop {
            let events = match self.journal.page(after, page_size) {
                Ok(events) => events,
                Err(_) => break,
            };
            if events.is_empty() {
                break;
            }
            for event in &events {
                let Some(payload) = event.payload.as_ref() else {
                    continue;
                };
                let matches = object_get(payload, "design_id")
                    .and_then(|value| value.as_text())
                    .is_some_and(|text| text.to_string_lossy() == design_id);
                if !matches {
                    continue;
                }
                if event.event_type == DESIGN_INTENT {
                    intent = Some(payload.clone());
                } else if event.event_type == PROVIDER_BUILD_COMPLETED {
                    build = Some(payload.clone());
                }
            }
            after = events[events.len() - 1].sequence;
        }
        match (intent, build) {
            (None, None) => None,
            (None, Some(build)) => Some(StoredDesign {
                intent: CanonicalValue::Object(Default::default()),
                build: Some(build),
            }),
            (intent, build) => Some(StoredDesign {
                intent: intent.unwrap_or_else(|| CanonicalValue::Object(Default::default())),
                build,
            }),
        }
    }

    fn stored_design_is_available(&self, stored: &StoredDesign) -> bool {
        let Some(build) = stored.build.as_ref() else {
            return false;
        };
        let script_sha256 = object_get(build, "script_sha256")
            .and_then(|value| value.as_text())
            .map(|text| text.to_string_lossy());
        let step_sha256 = object_get(build, "step_sha256")
            .and_then(|value| value.as_text())
            .map(|text| text.to_string_lossy());
        match (script_sha256, step_sha256) {
            (Some(script), Some(step)) => self.has_artifact(&script) && self.has_artifact(&step),
            _ => false,
        }
    }
}

// ------------------------------------------------------------------ free helpers

/// The design identity: `sha256(canonical script JSON || 0x00 || provider)`.
#[must_use]
pub fn design_id_for(script_bytes: &str, provider: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(script_bytes.as_bytes());
    hasher.update([0_u8]);
    hasher.update(provider.as_bytes());
    hex(&hasher.finalize())
}

/// The SHA-256 of some bytes, in lowercase hexadecimal.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// The number of solids the script declares its result to have.
#[must_use]
pub fn expected_solid_count(script: &ConstructionScript) -> i64 {
    let mut consumed: Vec<String> = Vec::new();
    let mut count: i64 = 0;
    for step in script.assembly() {
        if step.op() == "fuse" {
            let members: Vec<String> = step
                .components()
                .iter()
                .filter(|name| !consumed.contains(name))
                .cloned()
                .collect();
            if !members.is_empty() {
                count += 1;
                consumed.extend(members);
            }
        } else if step.op() == "compound" {
            let members: Vec<String> = step
                .components()
                .iter()
                .filter(|name| !consumed.contains(name))
                .cloned()
                .collect();
            if !members.is_empty() {
                count += i64::try_from(members.len()).unwrap_or(i64::MAX);
                consumed.extend(members);
            }
        }
    }
    for step in script.assembly() {
        if step.op() != "place" {
            continue;
        }
        let Some(component) = step.component() else {
            continue;
        };
        if consumed.iter().any(|item| item == component) {
            continue;
        }
        count += 1;
        consumed.push(component.to_owned());
    }
    count.max(1)
}

/// Whether the script fuses its components.
#[must_use]
pub fn script_fused(script: &ConstructionScript) -> bool {
    script.assembly().iter().any(|step| step.op() == "fuse")
}

/// The honesty checks built from the provider's own facts.
#[must_use]
pub fn build_provider_checks(
    measured: &MeasuredProperties,
    valid: Option<bool>,
    solid_count: Option<i64>,
    expected_solids: i64,
    expected_envelope: Option<&Bounds>,
) -> Vec<CheckRow> {
    let mut checks = vec![CheckRow {
        id: "brep_valid".to_owned(),
        label: "B-Rep valid".to_owned(),
        expected: CheckValue::Bool(true),
        measured: valid.map(CheckValue::Bool),
        tolerance: BREP_VALID_TOLERANCE,
        unit: "bool".to_owned(),
        passed: valid == Some(true),
    }];
    let measured_solids = solid_count.map(|count| count as f64);
    if expected_solids == 1 {
        checks.push(CheckRow {
            id: "single_solid".to_owned(),
            label: "Single solid".to_owned(),
            expected: CheckValue::Float(1.0),
            measured: measured_solids.map(CheckValue::Float),
            tolerance: SINGLE_SOLID_TOLERANCE,
            unit: "solid".to_owned(),
            passed: solid_count == Some(1),
        });
    } else {
        checks.push(CheckRow {
            id: "solid_count".to_owned(),
            label: "Solid count".to_owned(),
            expected: CheckValue::Float(expected_solids as f64),
            measured: measured_solids.map(CheckValue::Float),
            tolerance: SOLID_COUNT_TOLERANCE,
            unit: "solid".to_owned(),
            passed: solid_count == Some(expected_solids),
        });
    }
    if let Some(envelope) = expected_envelope {
        for (axis, name) in ["x", "y", "z"].into_iter().enumerate() {
            let expected = (envelope.min_mm[axis], envelope.max_mm[axis]);
            let Some(bounds) = measured.bounds() else {
                checks.push(CheckRow {
                    id: format!("envelope_{name}"),
                    label: format!("Envelope {}", name.to_uppercase()),
                    expected: CheckValue::Pair(expected.0, expected.1),
                    measured: None,
                    tolerance: ENVELOPE_TOLERANCE_MM,
                    unit: "mm".to_owned(),
                    passed: false,
                });
                continue;
            };
            let measured_pair = (bounds.min_mm()[axis], bounds.max_mm()[axis]);
            let passed = (expected.0 - measured_pair.0).abs() <= ENVELOPE_TOLERANCE_MM
                && (expected.1 - measured_pair.1).abs() <= ENVELOPE_TOLERANCE_MM;
            checks.push(CheckRow {
                id: format!("envelope_{name}"),
                label: format!("Envelope {}", name.to_uppercase()),
                expected: CheckValue::Pair(expected.0, expected.1),
                measured: Some(CheckValue::Pair(measured_pair.0, measured_pair.1)),
                tolerance: ENVELOPE_TOLERANCE_MM,
                unit: "mm".to_owned(),
                passed,
            });
        }
    }
    checks
}

/// The refusal of a drawing or a section the kernel could not produce.
///
/// The Python module distinguishes a bad argument (`ValueError`, a 422) from a
/// shape or kernel failure (`DrawingError`, a `RuntimeError`, a 500).
fn drawing_error(error: kala_cad::DrawingError) -> DesignError {
    match error.kind() {
        kala_cad::DrawingErrorKind::Argument => DesignError::Invalid(error.message().to_owned()),
        _ => DesignError::Internal(error.message().to_owned()),
    }
}

/// The design id of a stored design must be a 64-character lowercase digest.
pub fn validate_design_id(design_id: &str) -> Result<(), DesignError> {
    if design_id.len() != 64
        || !design_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(DesignError::Invalid(
            "the design id must be a 64-character lowercase hex digest".to_owned(),
        ));
    }
    Ok(())
}

/// The measured payload of a provider build.
fn measured_payload(measured: &MeasuredProperties) -> CanonicalValue {
    canonical_object(vec![
        ("volume_mm3", measured_optional_float(measured.volume_mm3())),
        (
            "surface_area_mm2",
            measured_optional_float(measured.area_mm2()),
        ),
        (
            "centroid_mm",
            match measured.centroid_mm() {
                Some(centroid) => canonical_float_array(centroid),
                None => CanonicalValue::Null,
            },
        ),
        (
            "bounds",
            match measured.bounds() {
                Some(bounds) => canonical_object(vec![
                    ("min", canonical_float_array(bounds.min_mm())),
                    ("max", canonical_float_array(bounds.max_mm())),
                ]),
                None => CanonicalValue::Null,
            },
        ),
        ("mass_g", measured_optional_float(measured.mass_g())),
    ])
}

/// The measured payload read back from the journal.
///
/// The written key wins and the legacy key stays a fallback, exactly like the
/// Python service: a design restored from an older journal keeps its area.
fn measured_from_payload(value: Option<&CanonicalValue>) -> MeasuredProperties {
    let Some(CanonicalValue::Object(entries)) = value else {
        return MeasuredProperties::default();
    };
    let volume = finite_number(map_get(entries, "volume_mm3"));
    let area = finite_number(map_get(entries, "surface_area_mm2"))
        .or_else(|| finite_number(map_get(entries, "area_mm2")));
    let bounds = measured_bounds_from_payload(map_get(entries, "bounds"));
    let mass = finite_number(map_get(entries, "mass_g"));
    let centroid = centroid_from_payload(map_get(entries, "centroid_mm"));
    MeasuredProperties::new(volume, area, bounds, mass, centroid).unwrap_or_default()
}

fn finite_number(value: Option<&CanonicalValue>) -> Option<f64> {
    match value {
        Some(CanonicalValue::Float(number)) => Some(*number),
        Some(CanonicalValue::Integer(number)) => number
            .to_string()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite()),
        _ => None,
    }
}

fn centroid_from_payload(value: Option<&CanonicalValue>) -> Option<[f64; 3]> {
    let Some(CanonicalValue::Array(values)) = value else {
        return None;
    };
    if values.len() != 3 {
        return None;
    }
    let mut coordinates = [0.0; 3];
    for (index, item) in values.iter().enumerate() {
        coordinates[index] = finite_number(Some(item))?;
    }
    Some(coordinates)
}

fn measured_bounds_from_payload(
    value: Option<&CanonicalValue>,
) -> Option<kala_cad_providers::MeasuredBounds> {
    let Some(CanonicalValue::Object(entries)) = value else {
        return None;
    };
    let minimum = match map_get(entries, "min") {
        Some(CanonicalValue::Array(values)) if values.len() == 3 => values,
        _ => return None,
    };
    let maximum = match map_get(entries, "max") {
        Some(CanonicalValue::Array(values)) if values.len() == 3 => values,
        _ => return None,
    };
    let mut low = [0.0; 3];
    let mut high = [0.0; 3];
    for index in 0..3 {
        low[index] = finite_number(Some(&minimum[index]))?;
        high[index] = finite_number(Some(&maximum[index]))?;
    }
    kala_cad_providers::MeasuredBounds::new(low, high).ok()
}

/// The declared envelope read back from the journal.
fn envelope_from_payload(value: Option<&CanonicalValue>) -> Option<Bounds> {
    let Some(CanonicalValue::Object(entries)) = value else {
        return None;
    };
    let minimum = match map_get(entries, "min") {
        Some(CanonicalValue::Array(values)) if values.len() == 3 => values,
        _ => return None,
    };
    let maximum = match map_get(entries, "max") {
        Some(CanonicalValue::Array(values)) if values.len() == 3 => values,
        _ => return None,
    };
    let mut low = [0.0; 3];
    let mut high = [0.0; 3];
    for index in 0..3 {
        low[index] = finite_number(Some(&minimum[index]))?;
        high[index] = finite_number(Some(&maximum[index]))?;
    }
    Bounds::new(low, high).ok()
}

/// The strings of a journal payload list.
fn string_tuple(value: Option<&CanonicalValue>) -> Vec<String> {
    let Some(CanonicalValue::Array(values)) = value else {
        return Vec::new();
    };
    values
        .iter()
        .filter_map(|item| item.as_text().map(|text| text.to_string_lossy()))
        .collect()
}

/// The bounded, de-duplicated warnings of one design.
fn bounded_warnings(warnings: &[String]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut bounded: Vec<String> = Vec::new();
    for warning in warnings {
        let text = warning.trim();
        if text.is_empty() || seen.iter().any(|item| item == text) {
            continue;
        }
        seen.push(text.to_owned());
        bounded.push(truncate_chars(text, MAX_WARNING_LENGTH));
        if bounded.len() >= MAX_WARNINGS {
            break;
        }
    }
    bounded
}

/// Truncate to a number of Unicode scalar values, like Python `str[:n]`.
pub fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

/// One field of a canonical object.
fn object_get<'a>(value: &'a CanonicalValue, key: &str) -> Option<&'a CanonicalValue> {
    let CanonicalValue::Object(entries) = value else {
        return None;
    };
    map_get(entries, key)
}

/// One field of a canonical object map.
fn map_get<'a>(entries: &'a kala_json::CanonicalObject, key: &str) -> Option<&'a CanonicalValue> {
    entries.get(&JsonText::new(key))
}

// -------------------------------------------------------------- payload helpers

fn canonical_text(value: &str) -> CanonicalValue {
    CanonicalValue::text(value)
}

fn canonical_float(value: f64) -> CanonicalValue {
    CanonicalValue::float(value).unwrap_or(CanonicalValue::Null)
}

fn canonical_float_array(values: [f64; 3]) -> CanonicalValue {
    CanonicalValue::Array(
        values
            .iter()
            .map(|value| canonical_float(*value))
            .collect::<Vec<_>>(),
    )
}

fn measured_optional_float(value: Option<f64>) -> CanonicalValue {
    match value {
        Some(number) => canonical_float(number),
        None => CanonicalValue::Null,
    }
}

fn canonical_array(values: &[String]) -> CanonicalValue {
    CanonicalValue::Array(
        values
            .iter()
            .map(|value| canonical_text(value))
            .collect::<Vec<_>>(),
    )
}

fn canonical_object(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    CanonicalValue::Object(
        entries
            .into_iter()
            .map(|(name, value)| (JsonText::new(name), value))
            .collect(),
    )
}

fn envelope_payload(bounds: Option<&Bounds>) -> CanonicalValue {
    match bounds {
        Some(bounds) => canonical_object(vec![
            ("min", canonical_float_array(bounds.min_mm)),
            ("max", canonical_float_array(bounds.max_mm)),
        ]),
        None => CanonicalValue::Null,
    }
}

fn check_value_payload(value: &CheckValue) -> CanonicalValue {
    match value {
        CheckValue::Bool(flag) => CanonicalValue::Bool(*flag),
        CheckValue::Float(number) => canonical_float(*number),
        CheckValue::Pair(low, high) => {
            CanonicalValue::Array(vec![canonical_float(*low), canonical_float(*high)])
        }
    }
}
