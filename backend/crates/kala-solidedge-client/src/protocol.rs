//! Bridge protocol v1: the versioned wire contract of the Solid Edge bridge.
//!
//! This is the native mirror of `kala_solidedge_bridge.protocol`. The bridge
//! speaks JSON only: a request names typed operations and bounded dimensions,
//! and a response is one of [`Health`], [`BuildAccepted`], [`BuildStatus`] or
//! [`BuildResult`]. Artifact bytes never travel in a request; a result names
//! produced artifacts by filename, format, size and digest.
//!
//! Planes follow the measured convention: `XY` has normal `+Z`, `XZ` has
//! normal `-Y`, and `YZ` has normal `+X`. A cut removes material along the
//! canonical plane normal and has no `reversed` flag; `None` means
//! through-all. A revolve turns around the component-local `Y` axis only.
//!
//! Serialization is byte-compatible with the Python contract:
//!
//! * `to_canonical_value` plus `kala_json::canonical_dumps` reproduces
//!   `json.dumps(payload, sort_keys=True, separators=(",", ":"),
//!   ensure_ascii=True)`, which is the one deterministic digest input;
//! * `to_wire_json` reproduces the field order and escaping of Pydantic's
//!   `model_dump_json()`;
//! * `request_digest` is the lowercase SHA-256 of the canonical request bytes
//!   with `request_id` removed, so a retry of the same semantic request has
//!   the same digest.

use std::collections::BTreeMap;

use kala_contracts::{Text, WireValue};
use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use num_bigint::BigInt;
use num_traits::ToPrimitive as _;

use crate::errors::SolidEdgeClientError;

/// The wire contract version implemented by this crate.
pub const PROTOCOL_VERSION: u8 = 1;
/// Maximum parts in one build request.
pub const MAX_PARTS: usize = 8;
/// Maximum operations in one build request.
pub const MAX_OPERATIONS: usize = 600;
/// Smallest accepted size, in millimetres.
pub const MIN_SIZE_MM: f64 = 0.001;
/// Largest accepted size, in millimetres.
pub const MAX_SIZE_MM: f64 = 5_000.0;
/// Largest accepted absolute offset, in millimetres.
pub const MAX_OFFSET_MM: f64 = 5_000.0;
/// Largest accepted revolve angle, in degrees.
pub const MAX_ANGLE_DEG: f64 = 360.0;
/// Smallest accepted build timeout, in seconds.
pub const MIN_TIMEOUT_SECONDS: f64 = 1.0;
/// Largest accepted build timeout, in seconds.
pub const MAX_TIMEOUT_SECONDS: f64 = 900.0;
/// Largest accepted density, in kilograms per cubic metre.
pub const MAX_DENSITY_KG_M3: f64 = 22_000.0;
/// Maximum trace entries in one response.
pub const MAX_TRACE_STEPS: usize = 600;
/// Maximum artifacts in one response.
pub const MAX_ARTIFACTS: usize = 2 * MAX_PARTS + 2;
/// Maximum warnings in one response.
pub const MAX_WARNINGS: usize = 16;
/// The only axis a revolve may use.
pub const REVOLVE_AXIS: &str = "Y";

const SAFE_ID_PATTERN: &str = "^[A-Za-z0-9][A-Za-z0-9._:-]*$";
const PART_NAME_PATTERN: &str = "^[A-Za-z][A-Za-z0-9_-]*$";
const OP_NAME_PATTERN: &str = "^[a-z][a-z0-9._]*$";
const ARTIFACT_NAME_PATTERN: &str = "^[A-Za-z0-9][A-Za-z0-9._-]*$";
const MEDIA_TYPE_PATTERN: &str = "^[a-z0-9][a-z0-9!#$&^_.+-]*/[a-z0-9][a-z0-9!#$&^_.+-]*$";
const SHA256_PATTERN: &str = "^[0-9a-f]{64}$";
const PLATFORM_PATTERN: &str = "^[a-z][a-z0-9._-]*$";
const CAPABILITY_PATTERN: &str = "^[a-z][a-z0-9._-]*$";

/// A wire document that is not valid protocol v1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolViolation {
    /// Where the document went wrong, in Python `loc` form.
    pub location: String,
    /// The Pydantic-style explanation.
    pub message: String,
}

impl ProtocolViolation {
    fn new(location: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            location: location.into(),
            message: message.into(),
        }
    }

    /// The `the build request is invalid at {location}: {message}` rendering.
    pub fn to_request_message(&self) -> String {
        format!(
            "the build request is invalid at {}: {}",
            self.location, self.message
        )
    }
}

impl std::fmt::Display for ProtocolViolation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "the build request is invalid at {}: {}",
            self.location, self.message
        )
    }
}

impl std::error::Error for ProtocolViolation {}

impl From<ProtocolViolation> for SolidEdgeClientError {
    fn from(violation: ProtocolViolation) -> Self {
        SolidEdgeClientError::compile(violation.to_request_message())
    }
}

/// The measured sketch-plane orientation and its canonical normal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Plane {
    /// `(u, v, 0)`, normal `+Z`.
    XY,
    /// `(u, 0, v)`, normal `-Y`.
    XZ,
    /// `(0, u, v)`, normal `+X`.
    YZ,
}

impl Plane {
    /// The wire value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::XY => "XY",
            Self::XZ => "XZ",
            Self::YZ => "YZ",
        }
    }

    /// Parse an exact wire value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "XY" => Some(Self::XY),
            "XZ" => Some(Self::XZ),
            "YZ" => Some(Self::YZ),
            _ => None,
        }
    }

    /// The values in declaration order, for error messages.
    pub const ALL: [Plane; 3] = [Self::XY, Self::XZ, Self::YZ];
}

impl std::fmt::Display for Plane {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The only states a bridge job may report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JobState {
    /// Accepted, not started.
    Queued,
    /// The worker is building.
    Running,
    /// The build finished.
    Succeeded,
    /// The build failed.
    Failed,
    /// Cancellation was requested and honoured between operations.
    Cancelled,
    /// The bridge cannot prove whether the native call completed.
    Indeterminate,
    /// The bridge restarted before the job was handed to the worker.
    Interrupted,
}

impl JobState {
    /// The wire value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Indeterminate => "indeterminate",
            Self::Interrupted => "interrupted",
        }
    }

    /// Parse an exact wire value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "indeterminate" => Some(Self::Indeterminate),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }

    /// Whether no further state change is possible.
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded
                | Self::Failed
                | Self::Cancelled
                | Self::Indeterminate
                | Self::Interrupted
        )
    }

    /// The values in declaration order, for error messages.
    pub const ALL: [JobState; 7] = [
        Self::Queued,
        Self::Running,
        Self::Succeeded,
        Self::Failed,
        Self::Cancelled,
        Self::Indeterminate,
        Self::Interrupted,
    ];
}

impl std::fmt::Display for JobState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The artifact formats the bridge may publish.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArtifactFormat {
    /// A Solid Edge part document.
    Par,
    /// A Solid Edge assembly document.
    Asm,
    /// A STEP file.
    Step,
}

impl ArtifactFormat {
    /// The wire value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Par => "par",
            Self::Asm => "asm",
            Self::Step => "step",
        }
    }

    /// Parse an exact wire value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "par" => Some(Self::Par),
            "asm" => Some(Self::Asm),
            "step" => Some(Self::Step),
            _ => None,
        }
    }

    /// The values in declaration order, for error messages.
    pub const ALL: [ArtifactFormat; 3] = [Self::Par, Self::Asm, Self::Step];
}

impl std::fmt::Display for ArtifactFormat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Stable bridge error codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ErrorCode {
    /// The request was refused before any native work.
    InvalidRequest,
    /// The operation order is not allowed.
    InvalidSequence,
    /// The operation name is not known.
    UnknownOperation,
    /// The operation is known but not supported here.
    UnsupportedOperation,
    /// Solid Edge, COM, or the calibrated worker is unavailable.
    Unavailable,
    /// The host has not been calibrated.
    CalibrationRequired,
    /// Another build owns the single worker.
    Busy,
    /// The build identity or target conflicts with existing state.
    Conflict,
    /// The supervisor deadline expired; the outcome is unknown.
    TimeoutIndeterminate,
    /// The build was cancelled.
    Cancelled,
    /// An unexpected internal failure.
    InternalError,
}

impl ErrorCode {
    /// The wire value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::InvalidSequence => "invalid_sequence",
            Self::UnknownOperation => "unknown_operation",
            Self::UnsupportedOperation => "unsupported_operation",
            Self::Unavailable => "unavailable",
            Self::CalibrationRequired => "calibration_required",
            Self::Busy => "busy",
            Self::Conflict => "conflict",
            Self::TimeoutIndeterminate => "timeout_indeterminate",
            Self::Cancelled => "cancelled",
            Self::InternalError => "internal_error",
        }
    }

    /// Parse an exact wire value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "invalid_request" => Some(Self::InvalidRequest),
            "invalid_sequence" => Some(Self::InvalidSequence),
            "unknown_operation" => Some(Self::UnknownOperation),
            "unsupported_operation" => Some(Self::UnsupportedOperation),
            "unavailable" => Some(Self::Unavailable),
            "calibration_required" => Some(Self::CalibrationRequired),
            "busy" => Some(Self::Busy),
            "conflict" => Some(Self::Conflict),
            "timeout_indeterminate" => Some(Self::TimeoutIndeterminate),
            "cancelled" => Some(Self::Cancelled),
            "internal_error" => Some(Self::InternalError),
            _ => None,
        }
    }

    /// The values in declaration order, for error messages.
    pub const ALL: [ErrorCode; 11] = [
        Self::InvalidRequest,
        Self::InvalidSequence,
        Self::UnknownOperation,
        Self::UnsupportedOperation,
        Self::Unavailable,
        Self::CalibrationRequired,
        Self::Busy,
        Self::Conflict,
        Self::TimeoutIndeterminate,
        Self::Cancelled,
        Self::InternalError,
    ];
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One typed bridge operation.
#[derive(Clone, Debug, PartialEq)]
pub enum BridgeOp {
    /// Start a new part document.
    PartNew {
        /// The part name.
        name: String,
    },
    /// A rectangular profile on a sketch plane.
    SketchRectangle {
        /// The sketch plane.
        plane: Plane,
        /// The width, in millimetres.
        width: f64,
        /// The height, in millimetres.
        height: f64,
        /// The offset along the plane's first axis.
        x: f64,
        /// The offset along the plane's second axis.
        y: f64,
    },
    /// A circular profile on a sketch plane.
    SketchCircle {
        /// The sketch plane.
        plane: Plane,
        /// The diameter, in millimetres.
        diameter: f64,
        /// The offset along the plane's first axis.
        x: f64,
        /// The offset along the plane's second axis.
        y: f64,
    },
    /// Extrude the pending profile along the plane normal.
    SolidExtrude {
        /// The extrusion length, in millimetres.
        length: f64,
        /// Whether the extrusion runs against the canonical normal.
        reversed: bool,
    },
    /// Remove material along the canonical plane normal.
    SolidCut {
        /// The cut depth, or `None` for through-all.
        length: Option<f64>,
    },
    /// Revolve the pending profile around the component-local `Y` axis.
    SolidRevolve {
        /// The swept angle, in degrees.
        angle: f64,
        /// The axis; always `Y` in protocol v1.
        axis: String,
    },
    /// Measure the current part.
    PartMeasure,
    /// Save the current part.
    PartSave,
    /// STEP-export the current part.
    PartExportStep,
    /// Start a new assembly document.
    AssemblyNew {
        /// The assembly name.
        name: String,
    },
    /// Place one finished part in the assembly.
    AssemblyPlace {
        /// The placed part.
        component: String,
        /// The translation along `X`.
        x: f64,
        /// The translation along `Y`.
        y: f64,
        /// The translation along `Z`.
        z: f64,
    },
    /// Save the assembly.
    AssemblySave,
    /// STEP-export the assembly.
    AssemblyExportStep,
}

impl BridgeOp {
    /// The wire operation name.
    pub const fn op_name(&self) -> &'static str {
        match self {
            Self::PartNew { .. } => "part.new",
            Self::SketchRectangle { .. } => "sketch.rectangle",
            Self::SketchCircle { .. } => "sketch.circle",
            Self::SolidExtrude { .. } => "solid.extrude",
            Self::SolidCut { .. } => "solid.cut",
            Self::SolidRevolve { .. } => "solid.revolve",
            Self::PartMeasure => "part.measure",
            Self::PartSave => "part.save",
            Self::PartExportStep => "part.export_step",
            Self::AssemblyNew { .. } => "assembly.new",
            Self::AssemblyPlace { .. } => "assembly.place",
            Self::AssemblySave => "assembly.save",
            Self::AssemblyExportStep => "assembly.export_step",
        }
    }

    /// The canonical JSON value of this operation.
    pub fn to_canonical_value(&self) -> CanonicalValue {
        let mut object = CanonicalObject::new();
        object.insert(key("op"), canonical_text(self.op_name()));
        match self {
            Self::PartNew { name } => {
                object.insert(key("name"), canonical_text(name));
            }
            Self::SketchRectangle {
                plane,
                width,
                height,
                x,
                y,
            } => {
                object.insert(key("plane"), canonical_text(plane.as_str()));
                object.insert(key("width"), canonical_float(*width));
                object.insert(key("height"), canonical_float(*height));
                object.insert(key("x"), canonical_float(*x));
                object.insert(key("y"), canonical_float(*y));
            }
            Self::SketchCircle {
                plane,
                diameter,
                x,
                y,
            } => {
                object.insert(key("plane"), canonical_text(plane.as_str()));
                object.insert(key("diameter"), canonical_float(*diameter));
                object.insert(key("x"), canonical_float(*x));
                object.insert(key("y"), canonical_float(*y));
            }
            Self::SolidExtrude { length, reversed } => {
                object.insert(key("length"), canonical_float(*length));
                object.insert(key("reversed"), CanonicalValue::Bool(*reversed));
            }
            Self::SolidCut { length } => {
                object.insert(
                    key("length"),
                    match length {
                        Some(value) => canonical_float(*value),
                        None => CanonicalValue::Null,
                    },
                );
            }
            Self::SolidRevolve { angle, axis } => {
                object.insert(key("angle"), canonical_float(*angle));
                object.insert(key("axis"), canonical_text(axis));
            }
            Self::PartMeasure | Self::PartSave | Self::PartExportStep => {}
            Self::AssemblyNew { name } => {
                object.insert(key("name"), canonical_text(name));
            }
            Self::AssemblyPlace { component, x, y, z } => {
                object.insert(key("component"), canonical_text(component));
                object.insert(key("x"), canonical_float(*x));
                object.insert(key("y"), canonical_float(*y));
                object.insert(key("z"), canonical_float(*z));
            }
            Self::AssemblySave | Self::AssemblyExportStep => {}
        }
        CanonicalValue::Object(object)
    }

    /// The wire JSON value of this operation, in Pydantic field order.
    pub fn to_wire_value(&self) -> WireValue {
        let mut fields: Vec<(&'static str, WireValue)> = vec![("op", wire_text(self.op_name()))];
        match self {
            Self::PartNew { name } => fields.push(("name", wire_text(name))),
            Self::SketchRectangle {
                plane,
                width,
                height,
                x,
                y,
            } => {
                fields.push(("plane", wire_text(plane.as_str())));
                fields.push(("width", wire_float(*width)));
                fields.push(("height", wire_float(*height)));
                fields.push(("x", wire_float(*x)));
                fields.push(("y", wire_float(*y)));
            }
            Self::SketchCircle {
                plane,
                diameter,
                x,
                y,
            } => {
                fields.push(("plane", wire_text(plane.as_str())));
                fields.push(("diameter", wire_float(*diameter)));
                fields.push(("x", wire_float(*x)));
                fields.push(("y", wire_float(*y)));
            }
            Self::SolidExtrude { length, reversed } => {
                fields.push(("length", wire_float(*length)));
                fields.push(("reversed", WireValue::Bool(*reversed)));
            }
            Self::SolidCut { length } => fields.push((
                "length",
                match length {
                    Some(value) => wire_float(*value),
                    None => WireValue::Null,
                },
            )),
            Self::SolidRevolve { angle, axis } => {
                fields.push(("angle", wire_float(*angle)));
                fields.push(("axis", wire_text(axis)));
            }
            Self::PartMeasure | Self::PartSave | Self::PartExportStep => {}
            Self::AssemblyNew { name } => fields.push(("name", wire_text(name))),
            Self::AssemblyPlace { component, x, y, z } => {
                fields.push(("component", wire_text(component)));
                fields.push(("x", wire_float(*x)));
                fields.push(("y", wire_float(*y)));
                fields.push(("z", wire_float(*z)));
            }
            Self::AssemblySave | Self::AssemblyExportStep => {}
        }
        WireValue::object(fields)
    }
}

/// One bounded, deterministic build request.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildRequest {
    /// The wire contract version; always `1`.
    pub protocol_version: u8,
    /// Transport correlation only. A retry keeps the same id.
    pub request_id: String,
    /// The calibration the bridge reported for this host.
    pub calibration_id: Option<String>,
    /// A bounded human summary.
    pub summary: String,
    /// The ordered procedure.
    pub procedure: Vec<BridgeOp>,
    /// The supervisor deadline, in seconds.
    pub timeout_seconds: f64,
    /// An optional density, in kilograms per cubic metre.
    pub density_kg_m3: Option<f64>,
}

impl BuildRequest {
    /// Build and validate one request.
    ///
    /// Fields are validated in the contract's declaration order, so the first
    /// reported violation is the same one Pydantic would report.
    pub fn new(
        request_id: impl Into<String>,
        calibration_id: Option<String>,
        summary: impl Into<String>,
        procedure: Vec<BridgeOp>,
        timeout_seconds: f64,
        density_kg_m3: Option<f64>,
    ) -> Result<Self, ProtocolViolation> {
        let request_id = request_id.into();
        let summary = summary.into();
        if request_id.is_empty() {
            return Err(ProtocolViolation::new(
                "request_id",
                "String should have at least 1 character",
            ));
        }
        if request_id.chars().count() > 64 {
            return Err(ProtocolViolation::new(
                "request_id",
                "String should have at most 64 characters",
            ));
        }
        if !matches_safe_id(&request_id) {
            return Err(ProtocolViolation::new(
                "request_id",
                format!("String should match pattern '{SAFE_ID_PATTERN}'"),
            ));
        }
        if let Some(calibration) = calibration_id.as_ref() {
            if calibration.is_empty() {
                return Err(ProtocolViolation::new(
                    "calibration_id",
                    "String should have at least 1 character",
                ));
            }
            if calibration.chars().count() > 128 {
                return Err(ProtocolViolation::new(
                    "calibration_id",
                    "String should have at most 128 characters",
                ));
            }
            if !matches_safe_id(calibration) {
                return Err(ProtocolViolation::new(
                    "calibration_id",
                    format!("String should match pattern '{SAFE_ID_PATTERN}'"),
                ));
            }
        }
        if summary.chars().count() > 500 {
            return Err(ProtocolViolation::new(
                "summary",
                "String should have at most 500 characters",
            ));
        }
        if procedure.is_empty() {
            return Err(ProtocolViolation::new(
                "procedure",
                "Tuple should have at least 1 item after validation, not 0",
            ));
        }
        if procedure.len() > MAX_OPERATIONS {
            return Err(ProtocolViolation::new(
                "procedure",
                format!(
                    "Tuple should have at most {MAX_OPERATIONS} items after validation, not {}",
                    procedure.len()
                ),
            ));
        }
        for (index, operation) in procedure.iter().enumerate() {
            validate_operation(operation, &format!("procedure.{index}"))?;
        }
        if !(timeout_seconds.is_finite()
            && (MIN_TIMEOUT_SECONDS..=MAX_TIMEOUT_SECONDS).contains(&timeout_seconds))
        {
            return Err(ProtocolViolation::new(
                "timeout_seconds",
                "Value error, timeout_seconds must be between 1 and 900 seconds",
            ));
        }
        if let Some(density) = density_kg_m3 {
            if !(density.is_finite() && density > 0.0 && density <= MAX_DENSITY_KG_M3) {
                return Err(ProtocolViolation::new(
                    "density_kg_m3",
                    "Value error, density_kg_m3 must be greater than 0 and at most 22000.0",
                ));
            }
        }
        validate_procedure(&procedure)?;
        Ok(Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            calibration_id,
            summary,
            procedure,
            timeout_seconds,
            density_kg_m3,
        })
    }

    /// The canonical JSON value of the whole request.
    pub fn to_canonical_value(&self) -> CanonicalValue {
        let mut object = CanonicalObject::new();
        object.insert(
            key("protocol_version"),
            CanonicalValue::Integer(BigInt::from(self.protocol_version)),
        );
        object.insert(key("request_id"), canonical_text(&self.request_id));
        object.insert(
            key("calibration_id"),
            match &self.calibration_id {
                Some(value) => canonical_text(value),
                None => CanonicalValue::Null,
            },
        );
        object.insert(key("summary"), canonical_text(&self.summary));
        object.insert(
            key("procedure"),
            CanonicalValue::Array(
                self.procedure
                    .iter()
                    .map(BridgeOp::to_canonical_value)
                    .collect(),
            ),
        );
        object.insert(
            key("timeout_seconds"),
            canonical_float(self.timeout_seconds),
        );
        object.insert(
            key("density_kg_m3"),
            match self.density_kg_m3 {
                Some(value) => canonical_float(value),
                None => CanonicalValue::Null,
            },
        );
        CanonicalValue::Object(object)
    }

    /// The canonical JSON value without `request_id`.
    ///
    /// `request_id` is transport correlation only, so a retry of the same
    /// semantic request must have the same digest.
    pub fn to_canonical_value_without_request_id(&self) -> CanonicalValue {
        let mut object = CanonicalObject::new();
        object.insert(
            key("protocol_version"),
            CanonicalValue::Integer(BigInt::from(self.protocol_version)),
        );
        object.insert(
            key("calibration_id"),
            match &self.calibration_id {
                Some(value) => canonical_text(value),
                None => CanonicalValue::Null,
            },
        );
        object.insert(key("summary"), canonical_text(&self.summary));
        object.insert(
            key("procedure"),
            CanonicalValue::Array(
                self.procedure
                    .iter()
                    .map(BridgeOp::to_canonical_value)
                    .collect(),
            ),
        );
        object.insert(
            key("timeout_seconds"),
            canonical_float(self.timeout_seconds),
        );
        object.insert(
            key("density_kg_m3"),
            match self.density_kg_m3 {
                Some(value) => canonical_float(value),
                None => CanonicalValue::Null,
            },
        );
        CanonicalValue::Object(object)
    }

    /// The deterministic digest input: UTF-8, sorted keys, no whitespace.
    pub fn canonical_request_bytes(&self) -> Result<Vec<u8>, ProtocolViolation> {
        let value = self.to_canonical_value_without_request_id();
        kala_json::canonical_dumps(&value, &kala_contracts::model_json_limits())
            .map(String::into_bytes)
            .map_err(|error| ProtocolViolation::new("request", format!("Value error, {error}")))
    }

    /// The lowercase SHA-256 hex digest of the canonical request bytes.
    pub fn request_digest(&self) -> Result<String, ProtocolViolation> {
        let bytes = self.canonical_request_bytes()?;
        Ok(sha256_hex(&bytes))
    }

    /// The wire JSON body, in Pydantic field order.
    pub fn to_wire_json(&self) -> Result<String, ProtocolViolation> {
        let mut fields: Vec<(&'static str, WireValue)> = vec![
            (
                "protocol_version",
                WireValue::Integer(serde_integer(self.protocol_version)),
            ),
            ("request_id", wire_text(&self.request_id)),
            (
                "calibration_id",
                match &self.calibration_id {
                    Some(value) => wire_text(value),
                    None => WireValue::Null,
                },
            ),
            ("summary", wire_text(&self.summary)),
            (
                "procedure",
                WireValue::array(
                    self.procedure
                        .iter()
                        .map(|operation| operation.to_wire_value()),
                ),
            ),
            ("timeout_seconds", wire_float(self.timeout_seconds)),
            (
                "density_kg_m3",
                match self.density_kg_m3 {
                    Some(value) => wire_float(value),
                    None => WireValue::Null,
                },
            ),
        ];
        fields.shrink_to_fit();
        WireValue::object(fields)
            .encode(false)
            .map_err(|error| ProtocolViolation::new("request", error.to_string()))
    }

    /// Parse and validate one request from wire bytes.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, ProtocolViolation> {
        let value = kala_json::parse_strict(bytes, &kala_contracts::model_json_limits())
            .map_err(|error| ProtocolViolation::new("request", error.to_string()))?;
        Self::from_canonical_value(&value)
    }

    /// Parse and validate one request from a canonical value.
    pub fn from_canonical_value(value: &CanonicalValue) -> Result<Self, ProtocolViolation> {
        let object = expect_object(value, "request")?;
        reject_extra_fields(
            object,
            &[
                "protocol_version",
                "request_id",
                "calibration_id",
                "summary",
                "procedure",
                "timeout_seconds",
                "density_kg_m3",
            ],
            "request",
        )?;
        let protocol_version = match optional_field(object, "protocol_version") {
            Some(value) => {
                let number = expect_integer(value, "protocol_version")?;
                if number != i64::from(PROTOCOL_VERSION) {
                    return Err(ProtocolViolation::new(
                        "protocol_version",
                        "Value error, protocol_version must be the integer 1",
                    ));
                }
                PROTOCOL_VERSION
            }
            None => PROTOCOL_VERSION,
        };
        let request_id = expect_text(
            required_field(object, "request_id", "request")?,
            "request_id",
        )?;
        let calibration_id = match optional_field(object, "calibration_id") {
            Some(CanonicalValue::Null) | None => None,
            Some(value) => Some(expect_text(value, "calibration_id")?),
        };
        let summary = match optional_field(object, "summary") {
            Some(value) => expect_text(value, "summary")?,
            None => String::new(),
        };
        let procedure = match optional_field(object, "procedure") {
            Some(value) => {
                let CanonicalValue::Array(items) = value else {
                    return Err(ProtocolViolation::new(
                        "procedure",
                        "Input should be a valid list",
                    ));
                };
                let mut operations = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    operations.push(parse_operation(item, &format!("procedure.{index}"))?);
                }
                operations
            }
            None => {
                return Err(ProtocolViolation::new("procedure", "Field required"));
            }
        };
        let timeout_seconds = match optional_field(object, "timeout_seconds") {
            Some(value) => expect_number(value, "timeout_seconds")?,
            None => 120.0,
        };
        let density_kg_m3 = match optional_field(object, "density_kg_m3") {
            Some(CanonicalValue::Null) | None => None,
            Some(value) => Some(expect_number(value, "density_kg_m3")?),
        };
        let request = Self::new(
            request_id,
            calibration_id,
            summary,
            procedure,
            timeout_seconds,
            density_kg_m3,
        )?;
        Ok(Self {
            protocol_version,
            ..request
        })
    }
}

/// The bridge's honest health document.
#[derive(Clone, Debug, PartialEq)]
pub struct Health {
    /// The wire contract version the bridge speaks.
    pub protocol_version: u8,
    /// The bridge runtime version.
    pub bridge_version: String,
    /// The host platform, for example `win32`.
    pub platform: String,
    /// Whether a native build executor exists at all.
    pub supported: bool,
    /// Whether Solid Edge can be reached in this session.
    pub available: bool,
    /// Whether another build owns the worker.
    pub busy: bool,
    /// The reported Solid Edge version, when known.
    pub se_version: Option<String>,
    /// The current calibration id, when the host is calibrated.
    pub calibration_id: Option<String>,
    /// Bounded capability names. A name is never a readiness claim.
    pub capabilities: Vec<String>,
    /// A bounded detail.
    pub detail: String,
}

impl Health {
    /// Parse one health document.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, ProtocolViolation> {
        let value = kala_json::parse_strict(bytes, &kala_contracts::model_json_limits())
            .map_err(|error| ProtocolViolation::new("document", error.to_string()))?;
        let object = expect_object(&value, "document")?;
        reject_extra_fields(
            object,
            &[
                "protocol_version",
                "bridge_version",
                "platform",
                "supported",
                "available",
                "busy",
                "se_version",
                "calibration_id",
                "capabilities",
                "detail",
            ],
            "document",
        )?;
        let protocol_version = match optional_field(object, "protocol_version") {
            Some(value) => {
                let number = expect_integer(value, "protocol_version")?;
                if number != i64::from(PROTOCOL_VERSION) {
                    return Err(ProtocolViolation::new(
                        "protocol_version",
                        "Value error, protocol_version must be the integer 1",
                    ));
                }
                PROTOCOL_VERSION
            }
            None => PROTOCOL_VERSION,
        };
        Ok(Self {
            protocol_version,
            bridge_version: {
                let version = expect_text(
                    required_field(object, "bridge_version", "document")?,
                    "bridge_version",
                )?;
                validate_bounded_text(&version, "bridge_version", 1, 64, None)?;
                version
            },
            platform: {
                let platform =
                    expect_text(required_field(object, "platform", "document")?, "platform")?;
                validate_bounded_text(&platform, "platform", 1, 32, Some(Pattern::Platform))?;
                platform
            },
            supported: expect_bool(
                required_field(object, "supported", "document")?,
                "supported",
            )?,
            available: expect_bool(
                required_field(object, "available", "document")?,
                "available",
            )?,
            busy: expect_bool(required_field(object, "busy", "document")?, "busy")?,
            se_version: match optional_field(object, "se_version") {
                Some(value) => {
                    let version = expect_text(value, "se_version")?;
                    validate_bounded_text(&version, "se_version", 1, 64, None)?;
                    Some(version)
                }
                None => None,
            },
            calibration_id: match optional_field(object, "calibration_id") {
                Some(value) => {
                    let calibration = expect_text(value, "calibration_id")?;
                    validate_bounded_text(
                        &calibration,
                        "calibration_id",
                        1,
                        128,
                        Some(Pattern::SafeId),
                    )?;
                    Some(calibration)
                }
                None => None,
            },
            capabilities: match optional_field(object, "capabilities") {
                Some(value) => {
                    let CanonicalValue::Array(items) = value else {
                        return Err(ProtocolViolation::new(
                            "capabilities",
                            "Input should be a valid list",
                        ));
                    };
                    if items.len() > 32 {
                        return Err(ProtocolViolation::new(
                            "capabilities",
                            "List should have at most 32 items after validation, not ".to_owned()
                                + &items.len().to_string(),
                        ));
                    }
                    let mut names = Vec::with_capacity(items.len());
                    for (index, item) in items.iter().enumerate() {
                        let location = format!("capabilities.{index}");
                        let name = expect_text(item, &location)?;
                        if !matches_capability(&name) {
                            return Err(ProtocolViolation::new(
                                location,
                                format!("String should match pattern '{CAPABILITY_PATTERN}'"),
                            ));
                        }
                        names.push(name);
                    }
                    names
                }
                None => Vec::new(),
            },
            detail: match optional_field(object, "detail") {
                Some(value) => {
                    let detail = expect_text(value, "detail")?;
                    validate_bounded_text(&detail, "detail", 0, 500, None)?;
                    detail
                }
                None => String::new(),
            },
        })
    }
}

/// The bridge's acceptance of one build.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildAccepted {
    /// The bridge's build id.
    pub build_id: String,
    /// The state the job starts in.
    pub state: JobState,
}

impl BuildAccepted {
    /// Parse one acceptance document.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, ProtocolViolation> {
        let value = kala_json::parse_strict(bytes, &kala_contracts::model_json_limits())
            .map_err(|error| ProtocolViolation::new("document", error.to_string()))?;
        let object = expect_object(&value, "document")?;
        reject_extra_fields(object, &["build_id", "state"], "document")?;
        Ok(Self {
            build_id: expect_text(required_field(object, "build_id", "document")?, "build_id")?,
            state: match optional_field(object, "state") {
                Some(value) => expect_job_state(value, "state")?,
                None => JobState::Queued,
            },
        })
    }
}

/// The status of one build.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildStatus {
    /// The bridge's build id.
    pub build_id: String,
    /// The current state.
    pub state: JobState,
    /// How many operations have been attempted.
    pub step_index: i64,
    /// How many operations the procedure has.
    pub total: i64,
    /// The bounded operation trace.
    pub trace: Vec<StepTrace>,
    /// The result, once the job reached a terminal state.
    pub result: Option<BuildResult>,
    /// A bounded message.
    pub message: Option<String>,
}

impl BuildStatus {
    /// Parse one status document.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, ProtocolViolation> {
        let value = kala_json::parse_strict(bytes, &kala_contracts::model_json_limits())
            .map_err(|error| ProtocolViolation::new("document", error.to_string()))?;
        let object = expect_object(&value, "document")?;
        reject_extra_fields(
            object,
            &[
                "build_id",
                "state",
                "step_index",
                "total",
                "trace",
                "result",
                "message",
            ],
            "document",
        )?;
        Ok(Self {
            build_id: expect_text(required_field(object, "build_id", "document")?, "build_id")?,
            state: expect_job_state(required_field(object, "state", "document")?, "state")?,
            step_index: expect_integer(
                required_field(object, "step_index", "document")?,
                "step_index",
            )?,
            total: expect_integer(required_field(object, "total", "document")?, "total")?,
            trace: match optional_field(object, "trace") {
                Some(value) => parse_trace(value, "trace")?,
                None => Vec::new(),
            },
            result: match optional_field(object, "result") {
                Some(value) => Some(parse_build_result(value, "result")?),
                None => None,
            },
            message: match optional_field(object, "message") {
                Some(value) => Some(expect_text(value, "message")?),
                None => None,
            },
        })
    }
}

/// The measured result of one build.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildResult {
    /// The bridge's build id.
    pub build_id: String,
    /// The terminal state.
    pub state: JobState,
    /// The per-part measurements.
    pub parts: Vec<PartResult>,
    /// The assembly bounds, when an assembly was built.
    pub assembly_bounds: Option<RangeBox>,
    /// The produced artifacts.
    pub artifacts: Vec<Artifact>,
    /// The bounded operation trace.
    pub trace: Vec<StepTrace>,
    /// The bounded warnings.
    pub warnings: Vec<String>,
    /// The typed failure, when the build failed.
    pub error: Option<ErrorResponse>,
}

/// One part's measured metrics.
#[derive(Clone, Debug, PartialEq)]
pub struct PartResult {
    /// The part name.
    pub name: String,
    /// Whether the native build produced a valid solid.
    pub valid: bool,
    /// The measured volume, in cubic millimetres.
    pub volume_mm3: f64,
    /// The measured surface area, in square millimetres.
    pub surface_area_mm2: f64,
    /// The part-local bounds.
    pub range_box: RangeBox,
    /// The mass, when a density was supplied.
    pub mass_kg: Option<f64>,
}

/// An axis-aligned range in part-local coordinates, in millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RangeBox {
    /// The lower corner.
    pub min_mm: [f64; 3],
    /// The upper corner.
    pub max_mm: [f64; 3],
}

/// One traced operation.
#[derive(Clone, Debug, PartialEq)]
pub struct StepTrace {
    /// The operation index.
    pub seq: i64,
    /// The operation name.
    pub op: String,
    /// The bounded status.
    pub status: String,
    /// The bounded message.
    pub message: String,
    /// The duration, in milliseconds.
    pub duration_ms: f64,
}

/// One produced file, named by metadata only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifact {
    /// The file name.
    pub name: String,
    /// The format.
    pub format: ArtifactFormat,
    /// The media type.
    pub media_type: String,
    /// The size in bytes.
    pub size_bytes: u64,
    /// The lowercase SHA-256 hex digest.
    pub sha256: String,
}

/// A typed bridge failure.
#[derive(Clone, Debug, PartialEq)]
pub struct ErrorResponse {
    /// The stable code.
    pub code: ErrorCode,
    /// The bounded message.
    pub message: String,
}

impl ErrorResponse {
    /// Parse one error envelope.
    ///
    /// An unknown code is never accepted on the wire, so a document with one
    /// is not an error envelope.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, ProtocolViolation> {
        let value = kala_json::parse_strict(bytes, &kala_contracts::model_json_limits())
            .map_err(|error| ProtocolViolation::new("document", error.to_string()))?;
        let object = expect_object(&value, "document")?;
        reject_extra_fields(object, &["code", "message"], "document")?;
        let code = expect_text(required_field(object, "code", "document")?, "code")?;
        let Some(parsed) = ErrorCode::parse(&code) else {
            return Err(ProtocolViolation::new(
                "code",
                "Input should be 'invalid_request', 'invalid_sequence', 'unknown_operation', \
                 'unsupported_operation', 'unavailable', 'calibration_required', 'busy', \
                 'conflict', 'timeout_indeterminate', 'cancelled' or 'internal_error'",
            ));
        };
        let message = expect_text(required_field(object, "message", "document")?, "message")?;
        if message.is_empty() {
            return Err(ProtocolViolation::new(
                "message",
                "String should have at least 1 character",
            ));
        }
        if message.chars().count() > 500 {
            return Err(ProtocolViolation::new(
                "message",
                "String should have at most 500 characters",
            ));
        }
        Ok(Self {
            code: parsed,
            message,
        })
    }
}

/// The SHA-256 hex digest of some bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

// ---------------------------------------------------------------------------
// Procedure validation
// ---------------------------------------------------------------------------

/// Reject any procedure that would compile into the wrong feature order.
///
/// Part selection is implicit: the `part.new` that most recently succeeded is
/// the current part. A part may carry several additive features: the first
/// extrude or revolve creates its solid, and a later profile plus feature
/// extends the solid that already exists.
pub fn validate_procedure(procedure: &[BridgeOp]) -> Result<(), ProtocolViolation> {
    let mut created: BTreeMap<&str, bool> = BTreeMap::new();
    let mut current: Option<&str> = None;
    let mut profile_pending = false;
    let mut solid_built = false;
    let mut assembly_started = false;
    let mut placed: Vec<&str> = Vec::new();

    for (index, operation) in procedure.iter().enumerate() {
        let where_ = format!("operation {index} ({})", operation.op_name());
        if assembly_started
            && !matches!(
                operation,
                BridgeOp::AssemblyPlace { .. }
                    | BridgeOp::AssemblySave
                    | BridgeOp::AssemblyExportStep
            )
        {
            return Err(model_violation(format!(
                "{where_} cannot follow assembly.new"
            )));
        }
        match operation {
            BridgeOp::PartNew { name } => {
                if profile_pending {
                    return Err(model_violation(format!(
                        "{where_}: finish the pending profile before a new part"
                    )));
                }
                if let Some(part) = current {
                    if !solid_built {
                        return Err(model_violation(format!(
                            "{where_}: part {part:?} has no solid yet"
                        )));
                    }
                }
                if created.contains_key(name.as_str()) {
                    return Err(model_violation(format!(
                        "{where_}: duplicate part name {name:?}"
                    )));
                }
                if created.len() >= MAX_PARTS {
                    return Err(model_violation(format!(
                        "{where_}: at most {MAX_PARTS} parts are allowed"
                    )));
                }
                created.insert(name.as_str(), false);
                current = Some(name.as_str());
                profile_pending = false;
                solid_built = false;
            }
            BridgeOp::SketchRectangle { .. } | BridgeOp::SketchCircle { .. } => {
                if current.is_none() {
                    return Err(model_violation(format!(
                        "{where_}: a sketch requires a current part"
                    )));
                }
                if profile_pending {
                    return Err(model_violation(format!(
                        "{where_}: consume the pending profile before another sketch"
                    )));
                }
                profile_pending = true;
            }
            BridgeOp::SolidExtrude { .. } | BridgeOp::SolidRevolve { .. } => {
                if current.is_none() {
                    return Err(model_violation(format!(
                        "{where_}: a solid requires a current part"
                    )));
                }
                if !profile_pending {
                    return Err(model_violation(format!(
                        "{where_}: a solid requires a pending profile"
                    )));
                }
                profile_pending = false;
                solid_built = true;
                if let Some(part) = current {
                    created.insert(part, true);
                }
            }
            BridgeOp::SolidCut { .. } => {
                if current.is_none() || !solid_built {
                    return Err(model_violation(format!(
                        "{where_}: a cut requires an existing solid"
                    )));
                }
                if !profile_pending {
                    return Err(model_violation(format!(
                        "{where_}: a cut requires a pending profile"
                    )));
                }
                profile_pending = false;
            }
            BridgeOp::PartMeasure | BridgeOp::PartSave | BridgeOp::PartExportStep => {
                if current.is_none() || !solid_built {
                    return Err(model_violation(format!(
                        "{where_}: the current part has no solid"
                    )));
                }
                if profile_pending {
                    return Err(model_violation(format!(
                        "{where_}: consume the pending profile before {}",
                        operation.op_name()
                    )));
                }
            }
            BridgeOp::AssemblyNew { .. } => {
                if assembly_started {
                    return Err(model_violation(format!(
                        "{where_}: only one assembly may be created"
                    )));
                }
                if current.is_none() || !solid_built {
                    return Err(model_violation(format!(
                        "{where_}: an assembly requires at least one finished part"
                    )));
                }
                assembly_started = true;
            }
            BridgeOp::AssemblyPlace { component, .. } => {
                if !assembly_started {
                    return Err(model_violation(format!(
                        "{where_}: assembly.place requires assembly.new first"
                    )));
                }
                match created.get(component.as_str()) {
                    Some(true) => {}
                    _ => {
                        return Err(model_violation(format!(
                            "{where_}: {component:?} is not a finished part"
                        )));
                    }
                }
                if placed.contains(&component.as_str()) {
                    return Err(model_violation(format!(
                        "{where_}: {component:?} is already placed"
                    )));
                }
                placed.push(component.as_str());
            }
            BridgeOp::AssemblySave | BridgeOp::AssemblyExportStep => {
                if !assembly_started {
                    return Err(model_violation(format!(
                        "{where_}: the assembly has not been created"
                    )));
                }
                if placed.is_empty() {
                    return Err(model_violation(format!(
                        "{where_}: place at least one component before saving"
                    )));
                }
            }
        }
    }

    if current.is_none() {
        return Err(model_violation("the procedure must create a part"));
    }
    if profile_pending {
        return Err(model_violation(
            "the procedure ends with an unconsumed profile",
        ));
    }
    if !solid_built {
        return Err(model_violation(
            "the procedure ends with a part that has no solid",
        ));
    }
    Ok(())
}

fn model_violation(message: impl Into<String>) -> ProtocolViolation {
    let message: String = message.into();
    ProtocolViolation::new("request", format!("Value error, {message}"))
}

// ---------------------------------------------------------------------------
// Operation parsing and validation
// ---------------------------------------------------------------------------

fn parse_operation(value: &CanonicalValue, location: &str) -> Result<BridgeOp, ProtocolViolation> {
    let object = expect_object(value, location)?;
    let op = expect_text(
        required_field(object, "op", location)?,
        &format!("{location}.op"),
    )?;
    let operation = match op.as_str() {
        "part.new" => {
            reject_extra_fields(object, &["op", "name"], location)?;
            BridgeOp::PartNew {
                name: expect_text(
                    required_field(object, "name", location)?,
                    &format!("{location}.name"),
                )?,
            }
        }
        "sketch.rectangle" => {
            reject_extra_fields(
                object,
                &["op", "plane", "width", "height", "x", "y"],
                location,
            )?;
            BridgeOp::SketchRectangle {
                plane: expect_plane(
                    required_field(object, "plane", location)?,
                    &format!("{location}.plane"),
                )?,
                width: expect_number(
                    required_field(object, "width", location)?,
                    &format!("{location}.width"),
                )?,
                height: expect_number(
                    required_field(object, "height", location)?,
                    &format!("{location}.height"),
                )?,
                x: optional_number(object, "x", location, 0.0)?,
                y: optional_number(object, "y", location, 0.0)?,
            }
        }
        "sketch.circle" => {
            reject_extra_fields(object, &["op", "plane", "diameter", "x", "y"], location)?;
            BridgeOp::SketchCircle {
                plane: expect_plane(
                    required_field(object, "plane", location)?,
                    &format!("{location}.plane"),
                )?,
                diameter: expect_number(
                    required_field(object, "diameter", location)?,
                    &format!("{location}.diameter"),
                )?,
                x: optional_number(object, "x", location, 0.0)?,
                y: optional_number(object, "y", location, 0.0)?,
            }
        }
        "solid.extrude" => {
            reject_extra_fields(object, &["op", "length", "reversed"], location)?;
            BridgeOp::SolidExtrude {
                length: expect_number(
                    required_field(object, "length", location)?,
                    &format!("{location}.length"),
                )?,
                reversed: match optional_field(object, "reversed") {
                    Some(value) => expect_bool(value, &format!("{location}.reversed"))?,
                    None => false,
                },
            }
        }
        "solid.cut" => {
            reject_extra_fields(object, &["op", "length"], location)?;
            BridgeOp::SolidCut {
                length: match optional_field(object, "length") {
                    Some(CanonicalValue::Null) | None => None,
                    Some(value) => Some(expect_number(value, &format!("{location}.length"))?),
                },
            }
        }
        "solid.revolve" => {
            reject_extra_fields(object, &["op", "angle", "axis"], location)?;
            let axis = match optional_field(object, "axis") {
                Some(value) => expect_text(value, &format!("{location}.axis"))?,
                None => REVOLVE_AXIS.to_owned(),
            };
            if axis != REVOLVE_AXIS {
                return Err(ProtocolViolation::new(
                    format!("{location}.axis"),
                    "Input should be 'Y'",
                ));
            }
            BridgeOp::SolidRevolve {
                angle: match optional_field(object, "angle") {
                    Some(value) => expect_number(value, &format!("{location}.angle"))?,
                    None => MAX_ANGLE_DEG,
                },
                axis,
            }
        }
        "part.measure" => {
            reject_extra_fields(object, &["op"], location)?;
            BridgeOp::PartMeasure
        }
        "part.save" => {
            reject_extra_fields(object, &["op"], location)?;
            BridgeOp::PartSave
        }
        "part.export_step" => {
            reject_extra_fields(object, &["op"], location)?;
            BridgeOp::PartExportStep
        }
        "assembly.new" => {
            reject_extra_fields(object, &["op", "name"], location)?;
            BridgeOp::AssemblyNew {
                name: expect_text(
                    required_field(object, "name", location)?,
                    &format!("{location}.name"),
                )?,
            }
        }
        "assembly.place" => {
            reject_extra_fields(object, &["op", "component", "x", "y", "z"], location)?;
            BridgeOp::AssemblyPlace {
                component: expect_text(
                    required_field(object, "component", location)?,
                    &format!("{location}.component"),
                )?,
                x: optional_number(object, "x", location, 0.0)?,
                y: optional_number(object, "y", location, 0.0)?,
                z: optional_number(object, "z", location, 0.0)?,
            }
        }
        "assembly.save" => {
            reject_extra_fields(object, &["op"], location)?;
            BridgeOp::AssemblySave
        }
        "assembly.export_step" => {
            reject_extra_fields(object, &["op"], location)?;
            BridgeOp::AssemblyExportStep
        }
        other => {
            return Err(ProtocolViolation::new(
                format!("{location}.op"),
                format!("Input should be {other:?} or a known operation"),
            ));
        }
    };
    validate_operation(&operation, location)?;
    Ok(operation)
}

fn validate_operation(operation: &BridgeOp, location: &str) -> Result<(), ProtocolViolation> {
    match operation {
        BridgeOp::PartNew { name } | BridgeOp::AssemblyNew { name } => {
            if name.is_empty() {
                return Err(ProtocolViolation::new(
                    format!("{location}.name"),
                    "String should have at least 1 character",
                ));
            }
            if name.chars().count() > 40 {
                return Err(ProtocolViolation::new(
                    format!("{location}.name"),
                    "String should have at most 40 characters",
                ));
            }
            if !matches_part_name(name) {
                return Err(ProtocolViolation::new(
                    format!("{location}.name"),
                    format!("String should match pattern '{PART_NAME_PATTERN}'"),
                ));
            }
        }
        BridgeOp::SketchRectangle { width, height, .. } => {
            validate_size(*width, &format!("{location}.width"))?;
            validate_size(*height, &format!("{location}.height"))?;
        }
        BridgeOp::SketchCircle { diameter, .. } => {
            validate_size(*diameter, &format!("{location}.diameter"))?;
        }
        BridgeOp::SolidExtrude { length, .. } => {
            validate_size(*length, &format!("{location}.length"))?;
        }
        BridgeOp::SolidCut { length } => {
            if let Some(length) = length {
                validate_size(*length, &format!("{location}.length"))?;
            }
        }
        BridgeOp::SolidRevolve { angle, .. } => {
            if !(angle.is_finite() && *angle > 0.0 && *angle <= MAX_ANGLE_DEG) {
                return Err(ProtocolViolation::new(
                    format!("{location}.angle"),
                    "Value error, angle must be greater than 0 and at most 360.0 degrees",
                ));
            }
        }
        BridgeOp::AssemblyPlace { component, .. } => {
            if component.is_empty() {
                return Err(ProtocolViolation::new(
                    format!("{location}.component"),
                    "String should have at least 1 character",
                ));
            }
            if component.chars().count() > 40 {
                return Err(ProtocolViolation::new(
                    format!("{location}.component"),
                    "String should have at most 40 characters",
                ));
            }
            if !matches_part_name(component) {
                return Err(ProtocolViolation::new(
                    format!("{location}.component"),
                    format!("String should match pattern '{PART_NAME_PATTERN}'"),
                ));
            }
        }
        BridgeOp::PartMeasure
        | BridgeOp::PartSave
        | BridgeOp::PartExportStep
        | BridgeOp::AssemblySave
        | BridgeOp::AssemblyExportStep => {}
    }
    if let BridgeOp::SketchRectangle { x, y, .. } | BridgeOp::SketchCircle { x, y, .. } = operation
    {
        validate_offset(*x, &format!("{location}.x"))?;
        validate_offset(*y, &format!("{location}.y"))?;
    }
    if let BridgeOp::AssemblyPlace { x, y, z, .. } = operation {
        validate_offset(*x, &format!("{location}.x"))?;
        validate_offset(*y, &format!("{location}.y"))?;
        validate_offset(*z, &format!("{location}.z"))?;
    }
    Ok(())
}

fn validate_size(value: f64, location: &str) -> Result<(), ProtocolViolation> {
    if !(value.is_finite() && (MIN_SIZE_MM..=MAX_SIZE_MM).contains(&value)) {
        return Err(ProtocolViolation::new(
            location,
            "Value error, size must be between 0.001 and 5000.0 mm",
        ));
    }
    Ok(())
}

fn validate_offset(value: f64, location: &str) -> Result<(), ProtocolViolation> {
    if !(value.is_finite() && value.abs() <= MAX_OFFSET_MM) {
        return Err(ProtocolViolation::new(
            location,
            "Value error, offset must be between -5000.0 and 5000.0 mm",
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Response parsing helpers
// ---------------------------------------------------------------------------

fn parse_trace(
    value: &CanonicalValue,
    location: &str,
) -> Result<Vec<StepTrace>, ProtocolViolation> {
    let CanonicalValue::Array(items) = value else {
        return Err(ProtocolViolation::new(
            location,
            "Input should be a valid list",
        ));
    };
    if items.len() > MAX_TRACE_STEPS {
        return Err(ProtocolViolation::new(
            location,
            format!(
                "List should have at most {MAX_TRACE_STEPS} items after validation, not {}",
                items.len()
            ),
        ));
    }
    let mut trace = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let where_ = format!("{location}.{index}");
        let object = expect_object(item, &where_)?;
        reject_extra_fields(
            object,
            &["seq", "op", "status", "message", "duration_ms"],
            &where_,
        )?;
        trace.push(StepTrace {
            seq: expect_integer(
                required_field(object, "seq", &where_)?,
                &format!("{where_}.seq"),
            )?,
            op: {
                let op = expect_text(
                    required_field(object, "op", &where_)?,
                    &format!("{where_}.op"),
                )?;
                validate_bounded_text(&op, &format!("{where_}.op"), 1, 64, Some(Pattern::OpName))?;
                op
            },
            status: {
                let status = expect_text(
                    required_field(object, "status", &where_)?,
                    &format!("{where_}.status"),
                )?;
                validate_bounded_text(
                    &status,
                    &format!("{where_}.status"),
                    1,
                    32,
                    Some(Pattern::Platform),
                )?;
                status
            },
            message: match optional_field(object, "message") {
                Some(value) => expect_text(value, &format!("{where_}.message"))?,
                None => String::new(),
            },
            duration_ms: expect_number(
                required_field(object, "duration_ms", &where_)?,
                &format!("{where_}.duration_ms"),
            )?,
        });
    }
    Ok(trace)
}

fn parse_build_result(
    value: &CanonicalValue,
    location: &str,
) -> Result<BuildResult, ProtocolViolation> {
    let object = expect_object(value, location)?;
    reject_extra_fields(
        object,
        &[
            "build_id",
            "state",
            "parts",
            "assembly_bounds",
            "artifacts",
            "trace",
            "warnings",
            "error",
        ],
        location,
    )?;
    Ok(BuildResult {
        build_id: expect_text(
            required_field(object, "build_id", location)?,
            &format!("{location}.build_id"),
        )?,
        state: expect_job_state(
            required_field(object, "state", location)?,
            &format!("{location}.state"),
        )?,
        parts: match optional_field(object, "parts") {
            Some(value) => parse_parts(value, &format!("{location}.parts"))?,
            None => Vec::new(),
        },
        assembly_bounds: match optional_field(object, "assembly_bounds") {
            Some(value) => Some(parse_range_box(
                value,
                &format!("{location}.assembly_bounds"),
            )?),
            None => None,
        },
        artifacts: match optional_field(object, "artifacts") {
            Some(value) => parse_artifacts(value, &format!("{location}.artifacts"))?,
            None => Vec::new(),
        },
        trace: match optional_field(object, "trace") {
            Some(value) => parse_trace(value, &format!("{location}.trace"))?,
            None => Vec::new(),
        },
        warnings: match optional_field(object, "warnings") {
            Some(value) => {
                let CanonicalValue::Array(items) = value else {
                    return Err(ProtocolViolation::new(
                        format!("{location}.warnings"),
                        "Input should be a valid list",
                    ));
                };
                if items.len() > MAX_WARNINGS {
                    return Err(ProtocolViolation::new(
                        format!("{location}.warnings"),
                        format!(
                            "List should have at most {MAX_WARNINGS} items after validation, not {}",
                            items.len()
                        ),
                    ));
                }
                let mut warnings = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    warnings.push(expect_text(item, &format!("{location}.warnings.{index}"))?);
                }
                warnings
            }
            None => Vec::new(),
        },
        error: match optional_field(object, "error") {
            Some(value) => {
                let where_ = format!("{location}.error");
                let object = expect_object(value, &where_)?;
                reject_extra_fields(object, &["code", "message"], &where_)?;
                let code = expect_text(
                    required_field(object, "code", &where_)?,
                    &format!("{where_}.code"),
                )?;
                let Some(parsed) = ErrorCode::parse(&code) else {
                    return Err(ProtocolViolation::new(
                        format!("{where_}.code"),
                        "Input should be a known error code",
                    ));
                };
                Some(ErrorResponse {
                    code: parsed,
                    message: expect_text(
                        required_field(object, "message", &where_)?,
                        &format!("{where_}.message"),
                    )?,
                })
            }
            None => None,
        },
    })
}

fn parse_parts(
    value: &CanonicalValue,
    location: &str,
) -> Result<Vec<PartResult>, ProtocolViolation> {
    let CanonicalValue::Array(items) = value else {
        return Err(ProtocolViolation::new(
            location,
            "Input should be a valid list",
        ));
    };
    if items.len() > MAX_PARTS {
        return Err(ProtocolViolation::new(
            location,
            format!(
                "List should have at most {MAX_PARTS} items after validation, not {}",
                items.len()
            ),
        ));
    }
    let mut parts = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let where_ = format!("{location}.{index}");
        let object = expect_object(item, &where_)?;
        reject_extra_fields(
            object,
            &[
                "name",
                "valid",
                "volume_mm3",
                "surface_area_mm2",
                "range_box",
                "mass_kg",
            ],
            &where_,
        )?;
        parts.push(PartResult {
            name: expect_text(
                required_field(object, "name", &where_)?,
                &format!("{where_}.name"),
            )?,
            valid: expect_bool(
                required_field(object, "valid", &where_)?,
                &format!("{where_}.valid"),
            )?,
            volume_mm3: expect_number(
                required_field(object, "volume_mm3", &where_)?,
                &format!("{where_}.volume_mm3"),
            )?,
            surface_area_mm2: expect_number(
                required_field(object, "surface_area_mm2", &where_)?,
                &format!("{where_}.surface_area_mm2"),
            )?,
            range_box: parse_range_box(
                required_field(object, "range_box", &where_)?,
                &format!("{where_}.range_box"),
            )?,
            mass_kg: match optional_field(object, "mass_kg") {
                Some(value) => Some(expect_number(value, &format!("{where_}.mass_kg"))?),
                None => None,
            },
        });
    }
    Ok(parts)
}

fn parse_range_box(value: &CanonicalValue, location: &str) -> Result<RangeBox, ProtocolViolation> {
    let object = expect_object(value, location)?;
    reject_extra_fields(object, &["min_mm", "max_mm"], location)?;
    let min_mm = parse_triple(
        required_field(object, "min_mm", location)?,
        &format!("{location}.min_mm"),
    )?;
    let max_mm = parse_triple(
        required_field(object, "max_mm", location)?,
        &format!("{location}.max_mm"),
    )?;
    if min_mm
        .iter()
        .zip(max_mm.iter())
        .any(|(low, high)| low > high)
    {
        return Err(ProtocolViolation::new(
            location,
            "Value error, min_mm must be less than or equal to max_mm on every axis",
        ));
    }
    Ok(RangeBox { min_mm, max_mm })
}

fn parse_triple(value: &CanonicalValue, location: &str) -> Result<[f64; 3], ProtocolViolation> {
    let CanonicalValue::Array(items) = value else {
        return Err(ProtocolViolation::new(
            location,
            "Input should be a valid list",
        ));
    };
    if items.len() != 3 {
        return Err(ProtocolViolation::new(
            location,
            format!(
                "List should have 3 items after validation, not {}",
                items.len()
            ),
        ));
    }
    let mut triple = [0.0; 3];
    for (index, item) in items.iter().enumerate() {
        triple[index] = expect_number(item, &format!("{location}.{index}"))?;
    }
    Ok(triple)
}

fn parse_artifacts(
    value: &CanonicalValue,
    location: &str,
) -> Result<Vec<Artifact>, ProtocolViolation> {
    let CanonicalValue::Array(items) = value else {
        return Err(ProtocolViolation::new(
            location,
            "Input should be a valid list",
        ));
    };
    if items.len() > MAX_ARTIFACTS {
        return Err(ProtocolViolation::new(
            location,
            format!(
                "List should have at most {MAX_ARTIFACTS} items after validation, not {}",
                items.len()
            ),
        ));
    }
    let mut artifacts = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let where_ = format!("{location}.{index}");
        let object = expect_object(item, &where_)?;
        reject_extra_fields(
            object,
            &["name", "format", "media_type", "size_bytes", "sha256"],
            &where_,
        )?;
        let name = expect_text(
            required_field(object, "name", &where_)?,
            &format!("{where_}.name"),
        )?;
        if name.is_empty() {
            return Err(ProtocolViolation::new(
                format!("{where_}.name"),
                "String should have at least 1 character",
            ));
        }
        if name.chars().count() > 128 {
            return Err(ProtocolViolation::new(
                format!("{where_}.name"),
                "String should have at most 128 characters",
            ));
        }
        if !matches_artifact_name(&name) {
            return Err(ProtocolViolation::new(
                format!("{where_}.name"),
                format!("String should match pattern '{ARTIFACT_NAME_PATTERN}'"),
            ));
        }
        let format_text = expect_text(
            required_field(object, "format", &where_)?,
            &format!("{where_}.format"),
        )?;
        let Some(format) = ArtifactFormat::parse(&format_text) else {
            return Err(ProtocolViolation::new(
                format!("{where_}.format"),
                "Input should be 'par', 'asm' or 'step'",
            ));
        };
        let media_type = expect_text(
            required_field(object, "media_type", &where_)?,
            &format!("{where_}.media_type"),
        )?;
        if matches_media_type(&media_type).is_none() {
            return Err(ProtocolViolation::new(
                format!("{where_}.media_type"),
                format!("String should match pattern '{MEDIA_TYPE_PATTERN}'"),
            ));
        }
        let size_bytes = expect_integer(
            required_field(object, "size_bytes", &where_)?,
            &format!("{where_}.size_bytes"),
        )?;
        if size_bytes < 0 {
            return Err(ProtocolViolation::new(
                format!("{where_}.size_bytes"),
                "Input should be greater than or equal to 0",
            ));
        }
        let sha256 = expect_text(
            required_field(object, "sha256", &where_)?,
            &format!("{where_}.sha256"),
        )?;
        if !matches_sha256(&sha256) {
            return Err(ProtocolViolation::new(
                format!("{where_}.sha256"),
                format!("String should match pattern '{SHA256_PATTERN}'"),
            ));
        }
        artifacts.push(Artifact {
            name,
            format,
            media_type,
            size_bytes: size_bytes as u64,
            sha256,
        });
    }
    Ok(artifacts)
}

// ---------------------------------------------------------------------------
// Canonical value helpers
// ---------------------------------------------------------------------------

fn key(name: &str) -> JsonText {
    JsonText::new(name)
}

fn canonical_text(value: &str) -> CanonicalValue {
    CanonicalValue::Text(JsonText::new(value))
}

fn canonical_float(value: f64) -> CanonicalValue {
    CanonicalValue::float(value).unwrap_or(CanonicalValue::Null)
}

fn wire_text(value: &str) -> WireValue {
    WireValue::Text(Text::new(value))
}

fn wire_float(value: f64) -> WireValue {
    WireValue::Float(value)
}

fn serde_integer(value: u8) -> num_bigint::BigInt {
    BigInt::from(value)
}

// ---------------------------------------------------------------------------
// Field helpers
// ---------------------------------------------------------------------------

/// One of the contract's string patterns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pattern {
    /// `^[A-Za-z0-9][A-Za-z0-9._:-]*$`
    SafeId,
    /// `^[a-z][a-z0-9._-]*$`
    Platform,
    /// `^[a-z][a-z0-9._]*$`
    OpName,
}

impl Pattern {
    /// The pattern text used in the Pydantic-style message.
    const fn text(self) -> &'static str {
        match self {
            Self::SafeId => SAFE_ID_PATTERN,
            Self::Platform => PLATFORM_PATTERN,
            Self::OpName => OP_NAME_PATTERN,
        }
    }

    /// Whether the value matches.
    fn matches(self, value: &str) -> bool {
        match self {
            Self::SafeId => matches_safe_id(value),
            Self::Platform => matches_platform(value),
            Self::OpName => matches_op_name(value),
        }
    }
}

/// Validate a bounded string with an optional pattern.
fn validate_bounded_text(
    value: &str,
    location: &str,
    min_length: usize,
    max_length: usize,
    pattern: Option<Pattern>,
) -> Result<(), ProtocolViolation> {
    let length = value.chars().count();
    if length < min_length {
        return Err(ProtocolViolation::new(
            location,
            format!("String should have at least {min_length} character"),
        ));
    }
    if length > max_length {
        return Err(ProtocolViolation::new(
            location,
            format!("String should have at most {max_length} characters"),
        ));
    }
    if let Some(pattern) = pattern {
        if !pattern.matches(value) {
            return Err(ProtocolViolation::new(
                location,
                format!("String should match pattern '{}'", pattern.text()),
            ));
        }
    }
    Ok(())
}

fn expect_object<'a>(
    value: &'a CanonicalValue,
    location: &str,
) -> Result<&'a CanonicalObject, ProtocolViolation> {
    match value {
        CanonicalValue::Object(object) => Ok(object),
        _ => Err(ProtocolViolation::new(
            location,
            "Input should be a valid dictionary",
        )),
    }
}

fn reject_extra_fields(
    object: &CanonicalObject,
    allowed: &[&str],
    location: &str,
) -> Result<(), ProtocolViolation> {
    for name in object.keys() {
        let Some(name) = name.to_utf8().ok() else {
            return Err(ProtocolViolation::new(
                location,
                "Extra inputs are not permitted",
            ));
        };
        if !allowed.contains(&name.as_str()) {
            return Err(ProtocolViolation::new(
                format!("{location}.{name}"),
                "Extra inputs are not permitted",
            ));
        }
    }
    Ok(())
}

fn optional_field<'a>(object: &'a CanonicalObject, name: &str) -> Option<&'a CanonicalValue> {
    object.get(&key(name))
}

fn required_field<'a>(
    object: &'a CanonicalObject,
    name: &str,
    location: &str,
) -> Result<&'a CanonicalValue, ProtocolViolation> {
    optional_field(object, name)
        .ok_or_else(|| ProtocolViolation::new(format!("{location}.{name}"), "Field required"))
}

fn expect_text(value: &CanonicalValue, location: &str) -> Result<String, ProtocolViolation> {
    match value {
        CanonicalValue::Text(text) => text
            .to_utf8()
            .map_err(|_| ProtocolViolation::new(location, "Input should be a valid string")),
        _ => Err(ProtocolViolation::new(
            location,
            "Input should be a valid string",
        )),
    }
}

fn expect_bool(value: &CanonicalValue, location: &str) -> Result<bool, ProtocolViolation> {
    match value {
        CanonicalValue::Bool(flag) => Ok(*flag),
        _ => Err(ProtocolViolation::new(
            location,
            "Input should be a valid boolean",
        )),
    }
}

fn expect_integer(value: &CanonicalValue, location: &str) -> Result<i64, ProtocolViolation> {
    match value {
        CanonicalValue::Integer(number) => number
            .to_i64()
            .ok_or_else(|| ProtocolViolation::new(location, "Input should be a valid integer")),
        _ => Err(ProtocolViolation::new(
            location,
            "Input should be a valid integer",
        )),
    }
}

fn expect_number(value: &CanonicalValue, location: &str) -> Result<f64, ProtocolViolation> {
    match value {
        CanonicalValue::Float(number) => Ok(*number),
        CanonicalValue::Integer(number) => number
            .to_f64()
            .ok_or_else(|| ProtocolViolation::new(location, "Input should be a valid number")),
        _ => Err(ProtocolViolation::new(
            location,
            "Input should be a valid number",
        )),
    }
}

/// Read an optional number field, defaulting when it is absent.
fn optional_number(
    object: &CanonicalObject,
    name: &str,
    location: &str,
    default: f64,
) -> Result<f64, ProtocolViolation> {
    match optional_field(object, name) {
        None => Ok(default),
        Some(CanonicalValue::Null) => Err(ProtocolViolation::new(
            format!("{location}.{name}"),
            "Input should be a valid number",
        )),
        Some(value) => expect_number(value, &format!("{location}.{name}")),
    }
}

fn expect_plane(value: &CanonicalValue, location: &str) -> Result<Plane, ProtocolViolation> {
    let text = expect_text(value, location)?;
    Plane::parse(&text)
        .ok_or_else(|| ProtocolViolation::new(location, "Input should be 'XY', 'XZ' or 'YZ'"))
}

fn expect_job_state(value: &CanonicalValue, location: &str) -> Result<JobState, ProtocolViolation> {
    let text = expect_text(value, location)?;
    JobState::parse(&text).ok_or_else(|| {
        ProtocolViolation::new(
            location,
            "Input should be 'queued', 'running', 'succeeded', 'failed', 'cancelled', \
             'indeterminate' or 'interrupted'",
        )
    })
}

// ---------------------------------------------------------------------------
// Pattern helpers
// ---------------------------------------------------------------------------

fn matches_safe_id(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    characters.all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-')
    })
}

fn matches_part_name(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    characters.all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn matches_artifact_name(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
}

fn matches_capability(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    characters.all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '.' | '_' | '-')
    })
}

fn matches_platform(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    characters.all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '.' | '_' | '-')
    })
}

fn matches_op_name(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    characters.all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '.' | '_')
    })
}

fn matches_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn matches_media_type(value: &str) -> Option<()> {
    let (kind, subtype) = value.split_once('/')?;
    let kind_ok = kind
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && kind.chars().all(is_media_character);
    let subtype_ok = subtype
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && subtype.chars().all(is_media_character);
    if kind_ok && subtype_ok {
        Some(())
    } else {
        None
    }
}

fn is_media_character(character: char) -> bool {
    character.is_ascii_lowercase()
        || character.is_ascii_digit()
        || matches!(
            character,
            '!' | '#' | '$' | '&' | '^' | '_' | '.' | '+' | '-'
        )
}

#[cfg(test)]
mod tests {
    use super::{
        BridgeOp, BuildRequest, ErrorCode, Health, JobState, Plane, ProtocolViolation, sha256_hex,
        validate_procedure,
    };

    fn plate() -> Vec<BridgeOp> {
        vec![
            BridgeOp::PartNew {
                name: "plate".to_owned(),
            },
            BridgeOp::SketchRectangle {
                plane: Plane::XY,
                width: 20.0,
                height: 20.0,
                x: 0.0,
                y: 0.0,
            },
            BridgeOp::SolidExtrude {
                length: 10.0,
                reversed: false,
            },
        ]
    }

    #[test]
    fn a_request_round_trips_through_the_wire() {
        let request = BuildRequest::new(
            "req-1",
            Some("cal-1".to_owned()),
            "one part",
            plate(),
            120.0,
            None,
        )
        .expect("valid request");
        let body = request.to_wire_json().expect("wire body");
        let parsed = BuildRequest::from_json_bytes(body.as_bytes()).expect("round trip");
        assert_eq!(request, parsed);
    }

    #[test]
    fn a_bad_request_id_is_reported_with_the_python_message() {
        let error =
            BuildRequest::new("not safe!", None, "", plate(), 120.0, None).expect_err("refused");
        assert_eq!(error.location, "request_id");
        assert_eq!(
            error.message,
            "String should match pattern '^[A-Za-z0-9][A-Za-z0-9._:-]*$'"
        );
    }

    #[test]
    fn a_procedure_without_a_solid_is_refused() {
        let error = validate_procedure(&[BridgeOp::PartNew {
            name: "a".to_owned(),
        }])
        .expect_err("refused");
        assert_eq!(error.location, "request");
        assert_eq!(
            error.message,
            "Value error, the procedure ends with a part that has no solid"
        );
    }

    #[test]
    fn a_version_mismatch_is_refused() {
        let body = br#"{"protocol_version":2,"bridge_version":"0.1.0","platform":"win32",
            "supported":true,"available":true,"busy":false}"#;
        let error = Health::from_json_bytes(body).expect_err("refused");
        assert_eq!(error.location, "protocol_version");
    }

    #[test]
    fn the_digest_is_the_sha256_of_the_canonical_bytes() {
        let request = BuildRequest::new("req-1", None, "", plate(), 120.0, None).expect("request");
        let bytes = request.canonical_request_bytes().expect("bytes");
        assert_eq!(
            request.request_digest().expect("digest"),
            sha256_hex(&bytes)
        );
    }

    #[test]
    fn job_states_and_error_codes_round_trip() {
        for state in JobState::ALL {
            assert_eq!(JobState::parse(state.as_str()), Some(state));
        }
        for code in ErrorCode::ALL {
            assert_eq!(ErrorCode::parse(code.as_str()), Some(code));
        }
    }

    #[test]
    fn a_violation_renders_the_python_request_message() {
        let violation = ProtocolViolation::new("request_id", "Field required");
        assert_eq!(
            violation.to_request_message(),
            "the build request is invalid at request_id: Field required"
        );
    }
}
