//! The CAD provider contract (ADR-0011).
//!
//! Kala orchestrates; a CAD provider builds. Every provider accepts the same
//! [`ConstructionScript`](kala_contracts::ConstructionScript) and returns the
//! same [`ProviderBuild`]: the STEP bytes it exported, its own measured
//! properties, its version, and optional native bytes. The OpenCascade viewer
//! then imports the provider's STEP for display and never re-measures it as
//! the authority.
//!
//! This module is deliberately kernel-free. It imports only the shared
//! construction contract and never links FreeCAD, the FreeCAD adapter or the
//! Solid Edge bridge client distributions into a build. The FreeCAD provider
//! reaches FreeCAD only inside a subprocess; the Solid Edge provider reaches
//! the bridge only through the accepted loopback client.
//!
//! There is no default provider. [`ALLOWED_PROVIDER_IDS`] is the closed set a
//! caller must choose from; an absent or unknown choice is refused rather than
//! silently mapped to the built-in builder.

use std::fmt;

use kala_contracts::ConstructionScript;

/// Maximum characters retained in an availability reason.
pub const MAX_REASON_LENGTH: usize = 512;
/// Maximum characters retained in a provider version.
pub const MAX_VERSION_LENGTH: usize = 64;
/// Maximum characters retained in one warning.
pub const MAX_WARNING_LENGTH: usize = 256;
/// Maximum warnings retained on one build.
pub const MAX_WARNINGS: usize = 16;

/// The fragment of a provider warning that says a cut removed no material.
pub const CUT_NO_MATERIAL: &str = "removed no material";

/// The prefix of a display summary that stands for suppressed warnings.
pub const WARNING_SUMMARY_PREFIX: &str = "… and ";

/// How many individual warnings a bounded display keeps.
///
/// Two entries of [`MAX_WARNINGS`] are reserved for the two possible summary
/// lines, so a build never loses the exact ineffective-cut count.
const MAX_INDIVIDUAL_WARNINGS: usize = MAX_WARNINGS - 2;

/// The summary line that stands for suppressed ineffective-cut warnings.
///
/// The exact count is in the line, so
/// [`ineffective_cut_count`] recovers it however many lines are displayed.
#[must_use]
pub fn cut_warning_summary(count: usize) -> String {
    let noun = if count == 1 { "pocket" } else { "pockets" };
    format!("{WARNING_SUMMARY_PREFIX}{count} more {noun} {CUT_NO_MATERIAL}")
}

/// The summary line that stands for any other suppressed warnings.
#[must_use]
pub fn other_warning_summary(count: usize) -> String {
    let noun = if count == 1 { "warning" } else { "warnings" };
    format!("{WARNING_SUMMARY_PREFIX}{count} more {noun}")
}

/// The number a cut-summary line states, if the line is a cut summary.
#[must_use]
pub fn parse_cut_summary(line: &str) -> Option<usize> {
    let rest = line.strip_prefix(WARNING_SUMMARY_PREFIX)?;
    for suffix in [
        format!(" more pockets {CUT_NO_MATERIAL}"),
        format!(" more pocket {CUT_NO_MATERIAL}"),
    ] {
        if let Some(number) = rest.strip_suffix(&suffix) {
            return number.parse().ok();
        }
    }
    None
}

/// The number an other-warnings summary line states, if it is one.
#[must_use]
pub fn parse_other_summary(line: &str) -> Option<usize> {
    let rest = line.strip_prefix(WARNING_SUMMARY_PREFIX)?;
    for suffix in [" more warnings", " more warning"] {
        if let Some(number) = rest.strip_suffix(suffix) {
            return number.parse().ok();
        }
    }
    None
}

/// Bound a warning display without losing the ineffective-cut count.
///
/// Up to [`MAX_INDIVIDUAL_WARNINGS`] warnings are shown unchanged. The rest
/// are summarised in at most two lines: one carrying the exact number of
/// suppressed cuts, one carrying the number of suppressed other warnings. The
/// whole display therefore never exceeds [`MAX_WARNINGS`] entries, and
/// [`ineffective_cut_count`] over it is exact for any number of cuts. A
/// summary line in the input is folded into the new summary, so bounding an
/// already bounded display keeps both its shape and its exact count.
#[must_use]
pub fn bound_warnings<I>(warnings: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut display: Vec<String> = Vec::new();
    let mut suppressed_cuts = 0usize;
    let mut suppressed_other = 0usize;
    for warning in warnings {
        let text = truncate_chars(&warning, MAX_WARNING_LENGTH);
        if let Some(count) = parse_cut_summary(&text) {
            suppressed_cuts += count;
            continue;
        }
        if let Some(count) = parse_other_summary(&text) {
            suppressed_other += count;
            continue;
        }
        let is_cut = text.contains(CUT_NO_MATERIAL);
        if display.len() < MAX_INDIVIDUAL_WARNINGS {
            display.push(text);
        } else if is_cut {
            suppressed_cuts += 1;
        } else {
            suppressed_other += 1;
        }
    }
    if suppressed_cuts > 0 {
        display.push(cut_warning_summary(suppressed_cuts));
    }
    if suppressed_other > 0 {
        display.push(other_warning_summary(suppressed_other));
    }
    display
}

/// The exact number of ineffective-cut warnings in a bounded display.
///
/// Every individual cut warning counts once and a summary line contributes
/// the number it states, so the count does not depend on how many lines are
/// displayed.
#[must_use]
pub fn ineffective_cut_count(warnings: &[String]) -> usize {
    let mut total = 0usize;
    for warning in warnings {
        if let Some(count) = parse_cut_summary(warning) {
            total += count;
            continue;
        }
        if warning.contains(CUT_NO_MATERIAL) {
            total += 1;
        }
    }
    total
}

/// The CAD providers ADR-0011 defines. This set is closed.
///
/// `opencascade` is named because it is part of the closed provider set and the
/// API publishes it, but it is **not implemented here**: the OpenCascade
/// compiler and viewer stay in the geometry worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CadProviderId {
    /// Kala's opt-in OpenCascade builder (not part of this crate).
    OpenCascade,
    /// A real `FreeCADCmd` subprocess.
    FreeCad,
    /// Solid Edge through the Windows loopback bridge.
    SolidEdge,
}

impl CadProviderId {
    /// Every accepted provider id, in Python declaration order.
    pub const ALL: [CadProviderId; 3] = [
        CadProviderId::OpenCascade,
        CadProviderId::FreeCad,
        CadProviderId::SolidEdge,
    ];

    /// The Python string value.
    pub const fn as_str(self) -> &'static str {
        match self {
            CadProviderId::OpenCascade => "opencascade",
            CadProviderId::FreeCad => "freecad",
            CadProviderId::SolidEdge => "solidedge",
        }
    }

    /// Parse an exact Python enum value.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
    }
}

impl fmt::Display for CadProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The closed provider ids a caller must choose from.
pub const ALLOWED_PROVIDER_IDS: [&str; 3] = ["opencascade", "freecad", "solidedge"];

/// One provider's probe result, never assumed.
///
/// `available` is the only field a caller may act on. `version` and `reason`
/// are diagnostic provenance; a missing version is never filled with a guess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Availability {
    available: bool,
    version: Option<String>,
    reason: Option<String>,
}

impl Availability {
    /// Build a probe result, truncating `version` and `reason` like Python.
    pub fn new(
        available: bool,
        version: Option<impl Into<String>>,
        reason: Option<impl Into<String>>,
    ) -> Self {
        Self {
            available,
            version: version.map(|value| truncate_chars(&value.into(), MAX_VERSION_LENGTH)),
            reason: reason.map(|value| truncate_chars(&value.into(), MAX_REASON_LENGTH)),
        }
    }

    /// Whether this provider can build on this host right now.
    pub const fn is_available(&self) -> bool {
        self.available
    }

    /// The reported version, when the provider reported one.
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// The diagnostic reason a probe failed.
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

/// An axis-aligned box the provider measured, in millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeasuredBounds {
    min_mm: [f64; 3],
    max_mm: [f64; 3],
}

impl MeasuredBounds {
    /// Validate and build a measured box.
    ///
    /// The Python model also refuses arrays that are not exactly three long;
    /// fixed-size arrays make that state unrepresentable here.
    pub fn new(min_mm: [f64; 3], max_mm: [f64; 3]) -> Result<Self, CadValueError> {
        for value in min_mm.iter().chain(max_mm.iter()) {
            if !value.is_finite() {
                return Err(CadValueError::new("provider bounds must be finite numbers"));
            }
        }
        for axis in 0..3 {
            if min_mm[axis] > max_mm[axis] {
                return Err(CadValueError::new(
                    "provider bounds minimum exceeds maximum",
                ));
            }
        }
        Ok(Self { min_mm, max_mm })
    }

    /// The minimum corner along X, Y and Z.
    pub const fn min_mm(&self) -> [f64; 3] {
        self.min_mm
    }

    /// The maximum corner along X, Y and Z.
    pub const fn max_mm(&self) -> [f64; 3] {
        self.max_mm
    }

    /// The box size along X, Y and Z.
    pub fn size_mm(&self) -> [f64; 3] {
        [
            self.max_mm[0] - self.min_mm[0],
            self.max_mm[1] - self.min_mm[1],
            self.max_mm[2] - self.min_mm[2],
        ]
    }
}

/// The provider's own measured numbers.
///
/// ADR-0011 names volume, area, bounds and mass. `centroid_mm` is carried
/// because the OpenCascade provider reports it; a provider that does not
/// report a property leaves it `None` and the UI shows `—`. No value is ever
/// substituted by OpenCascade.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeasuredProperties {
    volume_mm3: Option<f64>,
    area_mm2: Option<f64>,
    bounds: Option<MeasuredBounds>,
    mass_g: Option<f64>,
    centroid_mm: Option<[f64; 3]>,
}

impl MeasuredProperties {
    /// Validate and build the provider's measured properties.
    ///
    /// Fields are checked in Python declaration order: volume, area, mass,
    /// then the centroid.
    pub fn new(
        volume_mm3: Option<f64>,
        area_mm2: Option<f64>,
        bounds: Option<MeasuredBounds>,
        mass_g: Option<f64>,
        centroid_mm: Option<[f64; 3]>,
    ) -> Result<Self, CadValueError> {
        for (name, value) in [
            ("volume_mm3", volume_mm3),
            ("area_mm2", area_mm2),
            ("mass_g", mass_g),
        ] {
            if value.is_some_and(|number| !number.is_finite()) {
                return Err(CadValueError::new(format!(
                    "provider {name} must be a finite number"
                )));
            }
        }
        if let Some(centroid) = centroid_mm {
            if !centroid.iter().all(|value| value.is_finite()) {
                return Err(CadValueError::new(
                    "provider centroid_mm must be three finite numbers",
                ));
            }
        }
        Ok(Self {
            volume_mm3,
            area_mm2,
            bounds,
            mass_g,
            centroid_mm,
        })
    }

    /// The measured volume in cubic millimetres.
    pub const fn volume_mm3(&self) -> Option<f64> {
        self.volume_mm3
    }

    /// The measured surface area in square millimetres.
    pub const fn area_mm2(&self) -> Option<f64> {
        self.area_mm2
    }

    /// The measured bounding box.
    pub const fn bounds(&self) -> Option<MeasuredBounds> {
        self.bounds
    }

    /// The measured mass in grams.
    pub const fn mass_g(&self) -> Option<f64> {
        self.mass_g
    }

    /// The measured centroid in millimetres.
    pub const fn centroid_mm(&self) -> Option<[f64; 3]> {
        self.centroid_mm
    }
}

/// One provider build: the exported STEP and the provider's own facts.
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderBuild {
    step_bytes: Vec<u8>,
    measured: MeasuredProperties,
    provider_version: Option<String>,
    warnings: Vec<String>,
    native_bytes: Option<Vec<u8>>,
    native_kind: Option<String>,
    valid: Option<bool>,
    solid_count: Option<i64>,
}

impl ProviderBuild {
    /// Validate one provider build.
    ///
    /// The checks and messages match the Python `ProviderBuild` model. The
    /// step bytes must be present and non-empty; a native artifact kind, when
    /// present, must be non-empty; warnings keep a bounded display whose last
    /// lines carry the exact number of suppressed warnings, each truncated to
    /// 256 characters; a solid count must be non-negative.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        step_bytes: Vec<u8>,
        measured: MeasuredProperties,
        provider_version: Option<String>,
        warnings: Vec<String>,
        native_bytes: Option<Vec<u8>>,
        native_kind: Option<String>,
        valid: Option<bool>,
        solid_count: Option<i64>,
    ) -> Result<Self, CadValueError> {
        if step_bytes.is_empty() {
            return Err(CadValueError::new(
                "a provider build must return STEP bytes",
            ));
        }
        if native_kind.as_deref().is_some_and(str::is_empty) {
            return Err(CadValueError::new(
                "a native artifact kind must not be empty",
            ));
        }
        if solid_count.is_some_and(|count| count < 0) {
            return Err(CadValueError::new("a solid count must be non-negative"));
        }
        let warnings = bound_warnings(warnings);
        Ok(Self {
            step_bytes,
            measured,
            provider_version: provider_version
                .map(|version| truncate_chars(&version, MAX_VERSION_LENGTH)),
            warnings,
            native_bytes,
            native_kind,
            valid,
            solid_count,
        })
    }

    /// The exported STEP bytes.
    pub fn step_bytes(&self) -> &[u8] {
        &self.step_bytes
    }

    /// The provider's measured numbers.
    pub const fn measured(&self) -> &MeasuredProperties {
        &self.measured
    }

    /// The provider version, when reported.
    pub fn provider_version(&self) -> Option<&str> {
        self.provider_version.as_deref()
    }

    /// Bounded warnings the provider reported.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The provider's native artifact bytes, when it produced one.
    pub fn native_bytes(&self) -> Option<&[u8]> {
        self.native_bytes.as_deref()
    }

    /// The native artifact kind, for example `fcstd`.
    pub fn native_kind(&self) -> Option<&str> {
        self.native_kind.as_deref()
    }

    /// The provider's B-Rep validity statement.
    pub const fn valid(&self) -> Option<bool> {
        self.valid
    }

    /// The provider's solid count.
    pub const fn solid_count(&self) -> Option<i64> {
        self.solid_count
    }
}

/// A probe result plus the static identity the API publishes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderStatus {
    id: String,
    name: String,
    available: bool,
    version: Option<String>,
    reason: Option<String>,
    measured_by: String,
    mode: String,
}

impl ProviderStatus {
    /// Build a status row from a probe and the provider's static identity.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        availability: &Availability,
        measured_by: impl Into<String>,
        mode: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            available: availability.is_available(),
            version: availability.version().map(str::to_owned),
            reason: availability.reason().map(str::to_owned),
            measured_by: measured_by.into(),
            mode: mode.into(),
        }
    }

    /// The provider id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the provider can build right now.
    pub const fn available(&self) -> bool {
        self.available
    }

    /// The reported version.
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// The probe's reason, when unavailable.
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    /// Who measured the numbers.
    pub fn measured_by(&self) -> &str {
        &self.measured_by
    }

    /// The provider interaction mode.
    pub fn mode(&self) -> &str {
        &self.mode
    }
}

/// A value refused by a provider contract model (`ValueError` in Python).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct CadValueError {
    message: String,
}

impl CadValueError {
    /// Create a contract-model refusal.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The refusal message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// A provider configuration refusal (`ValueError` from the Python constructor).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct CadProviderConfigError {
    message: String,
}

impl CadProviderConfigError {
    /// Create a configuration refusal.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The refusal message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// The selected provider cannot build on this host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CadProviderUnavailableError {
    provider: String,
    message: String,
    reason: String,
    available: Vec<String>,
}

impl CadProviderUnavailableError {
    /// Build an honest provider refusal.
    ///
    /// `reason` falls back to the message, and the stored reason is truncated
    /// to 512 characters exactly like the Python exception. The display
    /// message itself is not truncated there.
    pub fn new(
        provider: impl Into<String>,
        message: impl Into<String>,
        reason: Option<impl Into<String>>,
        available: impl IntoIterator<Item = String>,
    ) -> Self {
        let provider = provider.into();
        let message = message.into();
        let reason = reason
            .map(Into::into)
            .filter(|value: &String| !value.is_empty())
            .unwrap_or_else(|| message.clone());
        Self {
            provider,
            message,
            reason: truncate_chars(&reason, MAX_REASON_LENGTH),
            available: available.into_iter().collect(),
        }
    }

    /// The provider id that refused.
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// The user-visible message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The bounded diagnostic reason.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Providers that were available, when the caller supplied them.
    pub fn available(&self) -> &[String] {
        &self.available
    }
}

impl fmt::Display for CadProviderUnavailableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CadProviderUnavailableError {}

/// The provider was available but refused or failed the build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CadProviderBuildError {
    message: String,
    op: Option<String>,
}

impl CadProviderBuildError {
    /// Build a refusal; the message is truncated to 512 characters.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: truncate_chars(&message.into(), MAX_REASON_LENGTH),
            op: None,
        }
    }

    /// Build a refusal that names the failing assembly operation.
    pub fn with_op(message: impl Into<String>, op: impl Into<String>) -> Self {
        Self {
            message: truncate_chars(&message.into(), MAX_REASON_LENGTH),
            op: Some(op.into()),
        }
    }

    /// The bounded refusal message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The failing assembly operation, when known.
    pub fn op(&self) -> Option<&str> {
        self.op.as_deref()
    }
}

impl fmt::Display for CadProviderBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CadProviderBuildError {}

/// A provider refusal: unavailable, a failed build, or a cancelled build.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CadProviderError {
    /// The selected provider cannot build on this host.
    #[error(transparent)]
    Unavailable(#[from] CadProviderUnavailableError),
    /// The provider was available but refused or failed the build.
    #[error(transparent)]
    Build(#[from] CadProviderBuildError),
    /// The caller cancelled the build and the provider stopped it.
    #[error("{0}")]
    Cancelled(String),
}

impl CadProviderError {
    /// The user-visible message.
    pub fn message(&self) -> &str {
        match self {
            Self::Unavailable(error) => error.message(),
            Self::Build(error) => error.message(),
            Self::Cancelled(message) => message,
        }
    }

    /// The failing assembly operation, when the provider named one.
    pub fn op(&self) -> Option<&str> {
        match self {
            Self::Unavailable(_) | Self::Cancelled(_) => None,
            Self::Build(error) => error.op(),
        }
    }

    /// The unavailable error, when this is one.
    pub const fn unavailable(&self) -> Option<&CadProviderUnavailableError> {
        match self {
            Self::Unavailable(error) => Some(error),
            Self::Build(_) | Self::Cancelled(_) => None,
        }
    }

    /// The build error, when this is one.
    pub const fn build_error(&self) -> Option<&CadProviderBuildError> {
        match self {
            Self::Unavailable(_) | Self::Cancelled(_) => None,
            Self::Build(error) => Some(error),
        }
    }
}

/// The interface every CAD provider implements (ADR-0011).
///
/// Python's protocol also accepts the `DesignIntent`; both provider
/// implementations in this crate delete it without reading it, so the Rust
/// connection takes the already-resolved script and nothing else.
pub trait CadProvider {
    /// The closed provider id.
    fn name(&self) -> &'static str;
    /// The human display name.
    fn display_name(&self) -> &'static str;
    /// Who measured the numbers.
    fn measured_by(&self) -> &'static str;
    /// The provider interaction mode.
    fn mode(&self) -> &'static str;

    /// Return whether this provider can build on this host right now.
    fn probe(&mut self) -> Availability;

    /// Build `script`, export STEP, and report the provider's own facts.
    fn build(&mut self, script: &ConstructionScript) -> Result<ProviderBuild, CadProviderError>;

    /// Build `script`, stopping at the next safe point when `cancel` is set.
    ///
    /// The default is the uninterruptible [`Self::build`]: an in-process
    /// kernel cannot be stopped in the middle of an operation, so its caller
    /// cancels at the next phase boundary instead. An out-of-process provider
    /// (FreeCAD) overrides this and kills its own process group.
    fn build_cancellable(
        &mut self,
        script: &ConstructionScript,
        cancel: &kala_process::CancelToken,
    ) -> Result<ProviderBuild, CadProviderError> {
        let _ = cancel;
        self.build(script)
    }

    /// A probe result plus the static identity the API publishes.
    fn status(&mut self) -> ProviderStatus {
        let availability = self.probe();
        ProviderStatus::new(
            self.name(),
            self.display_name(),
            &availability,
            self.measured_by(),
            self.mode(),
        )
    }
}

/// Snapshot the process environment, skipping names or values that are not
/// valid UTF-8.
pub(crate) fn snapshot_environment() -> std::collections::BTreeMap<String, String> {
    std::env::vars_os()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}

/// Truncate to a number of Unicode scalar values, like Python `str[:n]`.
pub(crate) fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    value.chars().take(limit).collect()
}

#[cfg(test)]
mod warning_tests {
    use super::{
        MAX_WARNINGS, bound_warnings, ineffective_cut_count, parse_cut_summary, parse_other_summary,
    };

    fn cut(index: usize) -> String {
        format!("component 'plate': pocket at step index {index} on XY removed no material")
    }

    #[test]
    fn five_hundred_cut_warnings_keep_an_exact_count() {
        let warnings: Vec<String> = (0..500).map(cut).collect();
        let display = bound_warnings(warnings);
        assert_eq!(display.len(), MAX_WARNINGS - 1);
        assert_eq!(ineffective_cut_count(&display), 500);
        assert_eq!(
            display[MAX_WARNINGS - 2],
            "… and 486 more pockets removed no material"
        );
        assert_eq!(parse_cut_summary(&display[MAX_WARNINGS - 2]), Some(486));
        assert_eq!(parse_other_summary(&display[MAX_WARNINGS - 2]), None);
    }

    #[test]
    fn a_cut_summary_and_an_other_summary_both_fit() {
        let mut warnings: Vec<String> = (0..20).map(cut).collect();
        warnings.extend((0..5).map(|index| format!("warning number {index}")));
        let display = bound_warnings(warnings);
        assert_eq!(display.len(), MAX_WARNINGS);
        assert_eq!(ineffective_cut_count(&display), 20);
        assert_eq!(
            display[MAX_WARNINGS - 2],
            "… and 6 more pockets removed no material"
        );
        assert_eq!(display[MAX_WARNINGS - 1], "… and 5 more warnings");
    }

    #[test]
    fn bounding_an_already_bounded_display_is_idempotent() {
        let warnings: Vec<String> = (0..37).map(cut).collect();
        let once = bound_warnings(warnings);
        let twice = bound_warnings(once.clone());
        assert_eq!(once, twice);
        assert_eq!(ineffective_cut_count(&twice), 37);
    }

    #[test]
    fn a_single_suppressed_cut_uses_the_singular_summary() {
        let warnings: Vec<String> = (0..15).map(cut).collect();
        let display = bound_warnings(warnings);
        assert_eq!(display.len(), 15);
        assert_eq!(display[14], "… and 1 more pocket removed no material");
        assert_eq!(ineffective_cut_count(&display), 15);
    }

    #[test]
    fn a_short_display_is_unchanged_and_exact() {
        let warnings = vec![cut(1), "a plain warning".to_owned(), cut(3)];
        let display = bound_warnings(warnings);
        assert_eq!(display.len(), 3);
        assert_eq!(ineffective_cut_count(&display), 2);
    }
}
