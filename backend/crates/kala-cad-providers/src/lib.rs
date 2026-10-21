//! Native CAD provider connections for the Kala Agent backend (ADR-0011).
//!
//! This crate ports the **provider connections** of
//! `kala_agent.studio.design.providers`: the provider contract, the real
//! `FreeCADCmd` subprocess provider, and the Solid Edge loopback-bridge
//! provider. It computes no geometry. The numbers a build returns are the
//! provider's own numbers, parsed from the same envelope the Python provider
//! reads; OpenCascade is neither imported nor consulted here.
//!
//! The two connections are deliberately different in kind:
//!
//! * [`FreeCadProvider`] runs FreeCAD out of process. The process, its argv,
//!   its environment, its working directory and its wall deadline are all
//!   explicit, and supervision is delegated to the accepted `kala-process`
//!   crate. The driver script is embedded byte for byte from the Python
//!   provider.
//! * [`SolidEdgeProvider`] is a probe and an honest refusal. It resolves the
//!   bridge URL and token file like the Python provider and asks the
//!   authenticated `/v1/health` route through the accepted
//!   `kala-solidedge-client`. A reachable bridge still refuses a build,
//!   because this milestone has no verified native executor.
//!
//! There is no `opencascade` provider here, and there is no default
//! provider: [`ALLOWED_PROVIDER_IDS`] is the closed set a caller chooses from.
//!
//! # Evidence boundary
//!
//! The FreeCAD differential runs against a real FreeCAD 1.1.4 on the
//! development host. The Solid Edge probe differential runs against the live
//! Python provider on Linux, and the bridge protocol is exercised against an
//! in-process mock bridge. **Nothing here is evidence about Solid Edge, a
//! Windows host, or a native Solid Edge build.**

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod base;
mod freecad;
mod solidedge;

pub use base::{
    ALLOWED_PROVIDER_IDS, Availability, CUT_NO_MATERIAL, CadProvider, CadProviderBuildError,
    CadProviderConfigError, CadProviderError, CadProviderId, CadProviderUnavailableError,
    CadValueError, MAX_REASON_LENGTH, MAX_VERSION_LENGTH, MAX_WARNING_LENGTH, MAX_WARNINGS,
    MeasuredBounds, MeasuredProperties, ProviderBuild, ProviderStatus, WARNING_SUMMARY_PREFIX,
    bound_warnings, cut_warning_summary, ineffective_cut_count, other_warning_summary,
    parse_cut_summary, parse_other_summary,
};
pub use freecad::{
    BundledPythonSelection, DEFAULT_MAX_STEP_BYTES, DEFAULT_PROBE_CACHE_SECONDS,
    DEFAULT_TIMEOUT_SECONDS, DRIVER_FILENAME, DRIVER_SOURCE, FREECAD_EXECUTABLE_NAMES,
    FreeCadProvider, FreeCadProviderOptions, MAX_RESPONSE_BYTES, MAX_STEP_ENV_VAR,
    MAX_STEP_IMPORT_BYTES, MAX_TIMEOUT_SECONDS, MIN_ARTIFACT_BYTES_ENV_VAR, MIN_STEP_IMPORT_BYTES,
    MIN_TIMEOUT_SECONDS, NATIVE_MAX_BYTES, OUTPUT_ENV_VAR, PYTHON_PROBE_FILENAME, REQUEST_ENV_VAR,
    REQUEST_FILENAME, RESPONSE_FILENAME, SITE_PACKAGES_PREFIX, STEP_IMPORT_MAX_BYTES_ENV_VAR,
    SUPPORTED_FREECAD_PYTHON_ABIS, SYS_PATH_ENV_VAR, UNAVAILABLE_REASON,
    VERSION_PROBE_TIMEOUT_SECONDS, WORKSPACE_ENV_VAR, bundled_python_abis, canonical_request_json,
    resolve_step_import_max_bytes, select_bundled_python_abi, validate_step_import_max_bytes,
};
pub use solidedge::{
    BRIDGE_URL_ENV_VAR, DATA_DIR_PARTS, DEFAULT_BRIDGE_URL,
    DEFAULT_PROBE_CACHE_SECONDS as DEFAULT_SOLIDEDGE_PROBE_CACHE_SECONDS,
    DEFAULT_TIMEOUT_SECONDS as DEFAULT_SOLIDEDGE_TIMEOUT_SECONDS, HEALTH_PATH, MAX_HEALTH_BYTES,
    MAX_TOKEN_LENGTH, MIN_TOKEN_LENGTH, NOT_REACHABLE_MESSAGE, SolidEdgeProvider,
    SolidEdgeProviderOptions, TOKEN_FILE_ENV_VAR, TOKEN_FILENAME,
    UNAVAILABLE_REASON as SOLIDEDGE_UNAVAILABLE_REASON,
};
