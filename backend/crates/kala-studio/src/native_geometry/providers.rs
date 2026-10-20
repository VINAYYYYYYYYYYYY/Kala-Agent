//! The CAD provider registry of the native design service (ADR-0011).
//!
//! The closed provider set is the Python one: `opencascade` (the opt-in
//! built-in builder), `freecad` (a real out-of-process `FreeCADCmd` build) and
//! `solidedge` (the Windows ADR-0008 loopback bridge). There is no default
//! provider and no automatic fallback.
//!
//! The `opencascade` provider is the native one: it compiles the script with
//! the real OpenCascade adapter (`kala-cad`), measures the exact shape with
//! the kernel and exports the provider's own STEP bytes. The two external
//! providers are the reviewed connections of `kala-cad-providers`; they reach
//! FreeCAD only inside a subprocess and the bridge only over loopback.
//!
//! Every provider reports the provider's *own* numbers. OpenCascade is never
//! the measuring authority for an external model.

use std::collections::BTreeMap;

use kala_cad_providers::{
    ALLOWED_PROVIDER_IDS, Availability, CadProvider, CadProviderBuildError, CadProviderError,
    CadProviderUnavailableError, FreeCadProvider, FreeCadProviderOptions, MeasuredBounds,
    MeasuredProperties, ProviderBuild, ProviderStatus, SolidEdgeProvider, SolidEdgeProviderOptions,
};
use kala_contracts::ConstructionScript;

/// The provider id of the opt-in built-in builder.
pub const PROVIDER_OPENCASCADE: &str = "opencascade";
/// The provider id of the out-of-process FreeCAD builder.
pub const PROVIDER_FREECAD: &str = "freecad";
/// The provider id of the Windows bridge builder.
pub const PROVIDER_SOLIDEDGE: &str = "solidedge";

/// The human display name of the opt-in built-in builder.
pub const OPENCASCADE_DISPLAY_NAME: &str = "OpenCascade (opt-in)";
/// The interaction mode of the opt-in built-in builder.
pub const OPENCASCADE_MODE: &str = "opt-in";

/// The refusal reason an unavailable OpenCascade build reports.
///
/// The Python provider names the optional Python package it imports. The
/// native build links the OpenCascade SDK itself, so the reason names the
/// kernel instead. The *shape* of the refusal (a provider that cannot build
/// reports why) is unchanged.
pub const OPENCASCADE_UNAVAILABLE_REASON: &str = "the native OpenCascade kernel is not linked in this build; rebuild with the \
     `native` feature and a configured OCCT SDK to enable the opt-in OpenCascade provider";

/// The opt-in built-in provider: the real OpenCascade kernel.
#[derive(Clone, Debug)]
pub struct NativeOpenCascadeProvider {
    max_step_bytes: u64,
}

impl NativeOpenCascadeProvider {
    /// Build the provider over a STEP byte ceiling.
    #[must_use]
    pub fn new(max_step_bytes: u64) -> Self {
        Self { max_step_bytes }
    }
}

impl CadProvider for NativeOpenCascadeProvider {
    fn name(&self) -> &'static str {
        PROVIDER_OPENCASCADE
    }

    fn display_name(&self) -> &'static str {
        OPENCASCADE_DISPLAY_NAME
    }

    fn measured_by(&self) -> &'static str {
        OPENCASCADE_DISPLAY_NAME
    }

    fn mode(&self) -> &'static str {
        OPENCASCADE_MODE
    }

    fn probe(&mut self) -> Availability {
        let capability = kala_cad::probe_capability();
        Availability::new(
            capability.available,
            Some(capability.kernel),
            capability.reason.or_else(|| {
                if capability.available {
                    None
                } else {
                    Some(OPENCASCADE_UNAVAILABLE_REASON.to_owned())
                }
            }),
        )
    }

    fn build(&mut self, script: &ConstructionScript) -> Result<ProviderBuild, CadProviderError> {
        let compiled = kala_cad::compile_script(script).map_err(|error| {
            CadProviderError::Build(CadProviderBuildError::new(format!(
                "opencascade refused the script: {error}"
            )))
        })?;
        let measurement = kala_cad::measure_shape(&compiled.assembly_shape).map_err(|error| {
            CadProviderError::Build(CadProviderBuildError::new(format!(
                "OpenCascade could not measure the compiled shape: {}",
                error_kind_name(&error.to_string())
            )))
        })?;
        let step = kala_cad::step_bytes(&compiled.assembly_shape).map_err(|error| {
            CadProviderError::Build(CadProviderBuildError::new(step_writer_message(
                error.message(),
            )))
        })?;
        if step.len() as u64 > self.max_step_bytes {
            return Err(CadProviderError::Build(CadProviderBuildError::new(
                format!(
                    "the OpenCascade STEP export is larger than the configured STEP import \
                 limit of {} bytes",
                    self.max_step_bytes
                ),
            )));
        }
        let bounds = MeasuredBounds::new(measurement.bounds.min_mm, measurement.bounds.max_mm)
            .map_err(|error| {
                CadProviderError::Build(CadProviderBuildError::new(error.message().to_owned()))
            })?;
        let measured = MeasuredProperties::new(
            Some(measurement.volume_mm3),
            Some(measurement.surface_area_mm2),
            Some(bounds),
            None,
            Some(measurement.centroid_mm),
        )
        .map_err(|error| {
            CadProviderError::Build(CadProviderBuildError::new(error.message().to_owned()))
        })?;
        ProviderBuild::new(
            step,
            measured,
            Some(self.probe().version().unwrap_or("unknown").to_owned()),
            compiled.warnings,
            None,
            None,
            Some(measurement.valid),
            Some(i64::try_from(measurement.solid_count).unwrap_or(i64::MAX)),
        )
        .map_err(|error| {
            CadProviderError::Build(CadProviderBuildError::new(error.message().to_owned()))
        })
    }
}

/// The name of the Python exception class a native failure corresponds to.
///
/// The Python provider reports `type(error).__name__`; the native kernel
/// reports a bounded message, and the only failure class the measurement
/// path raises is `MeasurementError`.
fn error_kind_name(_message: &str) -> String {
    "MeasurementError".to_owned()
}

/// The Python wording of a STEP writer failure.
fn step_writer_message(message: &str) -> String {
    if message == "the STEP writer refused the shape" {
        "the OpenCascade STEP writer refused the compiled shape".to_owned()
    } else if message == "the STEP writer could not select millimetre units" {
        "the OpenCascade STEP writer failed: Standard_Failure".to_owned()
    } else if let Some(rest) = message.strip_prefix("the geometry kernel could not export STEP: ") {
        format!("the OpenCascade STEP writer failed: {rest}")
    } else {
        message.to_owned()
    }
}

/// One configured provider.
pub enum ProviderSlot {
    OpenCascade(NativeOpenCascadeProvider),
    FreeCad(FreeCadProvider),
    SolidEdge(SolidEdgeProvider),
}

impl ProviderSlot {
    fn provider(&mut self) -> &mut dyn CadProvider {
        match self {
            Self::OpenCascade(provider) => provider,
            Self::FreeCad(provider) => provider,
            Self::SolidEdge(provider) => provider,
        }
    }

    /// Probe the provider on this host.
    pub fn probe(&mut self) -> Availability {
        self.provider().probe()
    }
}

/// The closed provider registry of one Studio data directory.
pub struct ProviderRegistry {
    opencascade: ProviderSlot,
    freecad: ProviderSlot,
    solidedge: ProviderSlot,
}

impl ProviderRegistry {
    /// Build the registry from the Studio environment view.
    ///
    /// A provider that cannot be constructed is still registered: it reports
    /// itself unavailable, exactly like the Python registry does when a
    /// provider raises during its own probe.
    #[must_use]
    pub fn new(environ: BTreeMap<String, String>, max_step_bytes: u64) -> Self {
        let freecad = FreeCadProvider::new(FreeCadProviderOptions {
            max_step_bytes: usize::try_from(max_step_bytes).ok(),
            environ: Some(environ.clone()),
            ..FreeCadProviderOptions::default()
        })
        .map(ProviderSlot::FreeCad)
        .unwrap_or_else(|_| {
            ProviderSlot::FreeCad(
                FreeCadProvider::new(FreeCadProviderOptions {
                    max_step_bytes: Some(64 * 1024 * 1024),
                    environ: Some(environ.clone()),
                    ..FreeCadProviderOptions::default()
                })
                .unwrap_or_else(|_| unreachable!("the default FreeCAD options are valid")),
            )
        });
        let solidedge = SolidEdgeProvider::new(SolidEdgeProviderOptions {
            environ: Some(environ),
            ..SolidEdgeProviderOptions::default()
        })
        .map(ProviderSlot::SolidEdge)
        .unwrap_or_else(|_| {
            ProviderSlot::SolidEdge(
                SolidEdgeProvider::new(SolidEdgeProviderOptions::default())
                    .unwrap_or_else(|_| unreachable!("the default Solid Edge options are valid")),
            )
        });
        Self {
            opencascade: ProviderSlot::OpenCascade(NativeOpenCascadeProvider::new(max_step_bytes)),
            freecad,
            solidedge,
        }
    }

    /// The provider slot of one provider id.
    pub fn slot_for(&mut self, provider: &str) -> Option<&mut ProviderSlot> {
        match provider {
            PROVIDER_OPENCASCADE => Some(&mut self.opencascade),
            PROVIDER_FREECAD => Some(&mut self.freecad),
            PROVIDER_SOLIDEDGE => Some(&mut self.solidedge),
            _ => None,
        }
    }

    /// Who measured the numbers of one provider.
    ///
    /// The Python service reads `provider.measured_by`, which is the display
    /// name of the provider and not its id.
    #[must_use]
    pub fn measured_by(&self, provider: &str) -> String {
        match provider {
            PROVIDER_OPENCASCADE => OPENCASCADE_DISPLAY_NAME.to_owned(),
            PROVIDER_FREECAD => "FreeCAD".to_owned(),
            PROVIDER_SOLIDEDGE => "Solid Edge".to_owned(),
            other => other.to_owned(),
        }
    }

    /// The closed set of provider ids, in the Python declaration order.
    #[must_use]
    pub fn provider_ids(&self) -> [&'static str; 3] {
        [PROVIDER_OPENCASCADE, PROVIDER_FREECAD, PROVIDER_SOLIDEDGE]
    }

    /// Whether the named provider is configured.
    #[must_use]
    pub fn is_configured(&self, provider: &str) -> bool {
        self.provider_ids().contains(&provider)
    }

    /// Probe every provider and return the honest status rows.
    ///
    /// The order is the Python declaration order, and a provider that is not
    /// registered is reported unavailable instead of being skipped.
    pub fn probe_all(&mut self) -> Vec<ProviderStatus> {
        let mut statuses = Vec::with_capacity(ALLOWED_PROVIDER_IDS.len());
        for id in ALLOWED_PROVIDER_IDS {
            let Some(slot) = self.slot_for(id) else {
                statuses.push(ProviderStatus::new(
                    id,
                    id,
                    &Availability::new(
                        false,
                        None::<String>,
                        Some("the provider is not configured in this Studio build"),
                    ),
                    id,
                    "unknown",
                ));
                continue;
            };
            let provider = slot.provider();
            let availability = provider.probe();
            statuses.push(ProviderStatus::new(
                provider.name(),
                provider.display_name(),
                &availability,
                provider.measured_by(),
                provider.mode(),
            ));
        }
        statuses
    }

    /// The ids of the providers that can build right now.
    #[must_use]
    pub fn available_provider_ids(&mut self) -> Vec<String> {
        self.probe_all()
            .into_iter()
            .filter(|status| status.available())
            .map(|status| status.id().to_owned())
            .collect()
    }

    /// Build one design with the named provider.
    ///
    /// # Errors
    ///
    /// Returns the provider's own refusal: an unavailable provider, a build
    /// refusal, or a provider error.
    pub fn build(
        &mut self,
        provider: &str,
        script: &ConstructionScript,
    ) -> Result<ProviderBuild, CadProviderError> {
        let slot = self.slot_for(provider).ok_or_else(|| {
            CadProviderError::Unavailable(CadProviderUnavailableError::new(
                provider,
                format!("the {provider} CAD provider is not available"),
                Some("the provider is not configured in this Studio build"),
                Vec::<String>::new(),
            ))
        })?;
        slot.provider().build(script)
    }
}
