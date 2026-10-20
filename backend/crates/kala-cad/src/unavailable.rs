//! Explicit unavailable mode (`--no-default-features`).
//!
//! The public surface stays present so a packaging/capability probe can be
//! compiled without any native SDK, but every geometry operation returns a
//! typed unavailable error and `design_available()` is false. No analytic
//! approximation is substituted for a real B-Rep shape.

use std::collections::BTreeMap;

use kala_contracts::{AnalysisScript, ConstructionScript, EditStep};

use crate::bounds::{Bounds, CheckReport, Measurement, TopologyCounts};
use crate::drawing::{Drawing, Section};
use crate::error::{
    CompileError, DrawingError, MeasurementError, MeshError, StepError, StepImportError,
};
use crate::mesh_data::{MeshData, MeshOptions};
use crate::step_common::StepImportOptions;

/// The coarse retry linear deflection used by the native importer.
pub const COARSER_MESH_DEFLECTION_MM: f64 = 0.5;
/// The coarse retry angular deflection used by the native importer.
pub const COARSER_ANGULAR_DEFLECTION_RAD: f64 = 0.8;
/// The warning the native importer appends after its single coarser retry.
pub const COARSENED_WARNING: &str = "the imported shape exceeded the default tessellation budget; it was retried once with a coarser 0.5 mm deflection";

/// A placeholder shape in a build with no native kernel. It is always null.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shape {
    _private: (),
}

impl Shape {
    /// A null placeholder shape.
    pub fn null() -> Self {
        Self { _private: () }
    }

    /// Unavailable builds never hold a real shape.
    pub fn is_null(&self) -> bool {
        true
    }
}

/// See the native type; the unavailable mode never lowers geometry.
#[derive(Clone, Copy, Debug)]
pub struct CompileOptions<'a> {
    /// An analysis script; refused.
    pub analysis: Option<&'a AnalysisScript>,
    /// Tool edits; refused.
    pub edits: &'a [EditStep],
    /// Requested fillets; refused.
    pub fillets: Option<&'a BTreeMap<String, f64>>,
    /// Wall-clock compile budget.
    pub timeout_seconds: f64,
}

impl Default for CompileOptions<'_> {
    fn default() -> Self {
        Self {
            analysis: None,
            edits: &[],
            fillets: None,
            timeout_seconds: crate::DEFAULT_COMPILE_TIMEOUT_SECONDS,
        }
    }
}

/// See the native type.
#[derive(Debug)]
pub struct CompiledComponent {
    /// Component name.
    pub name: String,
    /// Always a placeholder.
    pub local_shape: Shape,
    /// Always a placeholder.
    pub placed_shape: Shape,
    /// Placement in millimetres.
    pub placement_mm: [f64; 3],
}

/// See the native type.
#[derive(Debug)]
pub struct CompilationResult {
    /// Always empty.
    pub components: Vec<CompiledComponent>,
    /// Always a placeholder.
    pub assembly_shape: Shape,
    /// Always false.
    pub fused: bool,
    /// Always empty.
    pub warnings: Vec<String>,
}

impl CompilationResult {
    /// Return the placeholder assembly shape.
    pub fn shape(&self) -> &Shape {
        &self.assembly_shape
    }

    /// Return a component by name (always `None`).
    pub fn component(&self, _name: &str) -> Option<&CompiledComponent> {
        None
    }
}

fn unavailable_compile() -> CompileError {
    CompileError::unavailable()
}

/// Compile is unavailable in this build.
pub fn compile_script(_script: &ConstructionScript) -> Result<CompilationResult, CompileError> {
    Err(unavailable_compile())
}

/// Compile is unavailable in this build.
pub fn compile_script_with_options(
    _script: &ConstructionScript,
    _options: &CompileOptions<'_>,
) -> Result<CompilationResult, CompileError> {
    Err(unavailable_compile())
}

/// Validate with the frozen loader, then report that the native kernel is
/// unavailable.
pub fn compile_json_str(
    text: &str,
    _options: &CompileOptions<'_>,
) -> Result<CompilationResult, CompileError> {
    kala_contracts::load_construction_json_str(text)
        .map_err(|error| CompileError::contract(error.message()))?;
    Err(unavailable_compile())
}

/// Validate with the frozen loader, then report that the native kernel is
/// unavailable.
pub fn compile_json_bytes(
    bytes: &[u8],
    _options: &CompileOptions<'_>,
) -> Result<CompilationResult, CompileError> {
    kala_contracts::load_construction_json_bytes(bytes)
        .map_err(|error| CompileError::contract(error.message()))?;
    Err(unavailable_compile())
}

/// Measurement is unavailable in this build.
pub fn measure_shape(_shape: &Shape) -> Result<Measurement, MeasurementError> {
    Err(MeasurementError::new(
        "the native OpenCascade kernel is unavailable in this build",
    ))
}

/// Measurement is unavailable in this build.
pub fn measure_and_check(
    _shape: &Shape,
    _expected_envelope: Option<&crate::Bounds>,
    _expect_single_solid: bool,
) -> Result<(Measurement, CheckReport), MeasurementError> {
    Err(MeasurementError::new(
        "the native OpenCascade kernel is unavailable in this build",
    ))
}

/// Topology measurement is unavailable in this build.
pub fn topology(_shape: &Shape) -> Result<TopologyCounts, MeasurementError> {
    Err(MeasurementError::new(
        "the native OpenCascade kernel is unavailable in this build",
    ))
}

/// Tessellation is unavailable in this build.
pub fn tessellate(_shape: &Shape, _options: MeshOptions) -> Result<MeshData, MeshError> {
    Err(MeshError::new(
        "the native OpenCascade kernel is unavailable in this build",
    ))
}

/// STEP export is unavailable in this build.
pub fn step_bytes(_shape: &Shape) -> Result<Vec<u8>, StepError> {
    Err(StepError::new(
        "the native OpenCascade kernel is unavailable in this build",
    ))
}

/// STEP import is unavailable in this build.
pub fn step_shape(_data: &[u8]) -> Result<Shape, StepError> {
    Err(StepError::new(
        "the native OpenCascade kernel is unavailable in this build",
    ))
}

/// STEP export with options is unavailable in this build.
pub fn step_bytes_with_options(
    _shape: &Shape,
    _options: crate::StepWriteOptions,
) -> Result<Vec<u8>, StepError> {
    Err(StepError::new(
        "the native OpenCascade kernel is unavailable in this build",
    ))
}

/// See the native type; the unavailable mode never produces a view.
#[derive(Debug)]
pub struct StepImportOutcome {
    /// Always empty.
    pub mesh: MeshData,
    /// Always a zero box; never measured.
    pub bounds: Bounds,
    /// Always zero.
    pub solids: usize,
    /// Always empty.
    pub warnings: Vec<String>,
    /// Always a placeholder.
    pub shape: Shape,
    /// Always false.
    pub coarsened: bool,
}

/// View-only STEP import is unavailable in this build.
pub fn import_step_view(
    _data: &[u8],
    _options: StepImportOptions,
) -> Result<StepImportOutcome, StepImportError> {
    Err(StepImportError::new(
        crate::StepImportErrorKind::Malformed,
        "the native OpenCascade kernel is unavailable in this build",
    ))
}

/// HLR drawing is unavailable in this build.
pub fn hlr_views(_shape: &Shape) -> Result<Drawing, DrawingError> {
    Err(DrawingError::unavailable())
}

/// HLR drawing is unavailable in this build.
pub fn hlr_views_with_options(
    _shape: &Shape,
    _views: &[&str],
    _chord_tolerance_mm: f64,
    _max_points: usize,
) -> Result<Drawing, DrawingError> {
    Err(DrawingError::unavailable())
}

/// Section cuts are unavailable in this build.
pub fn section(_shape: &Shape) -> Result<Section, DrawingError> {
    Err(DrawingError::unavailable())
}

/// Section cuts are unavailable in this build.
pub fn section_with_options(
    _shape: &Shape,
    _plane: &str,
    _offset_mm: Option<f64>,
    _chord_tolerance_mm: f64,
) -> Result<Section, DrawingError> {
    Err(DrawingError::unavailable())
}

/// Default section offsets need the kernel's exact bounds.
pub fn default_section_offset(_shape: &Shape) -> Result<f64, DrawingError> {
    Err(DrawingError::unavailable())
}
