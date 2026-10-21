//! Exact `ConstructionScript` compiler.
//!
//! This is a port of `src/kala_agent/studio/design/compile_ocp.py`. The
//! semantics are pinned by `docs/construction-semantics.md`:
//!
//! * `XY` maps `(u, v)` to `(u, v, 0)` with normal `+Z`;
//! * `XZ` maps `(u, v)` to `(u, 0, v)` with normal `-Y`;
//! * `YZ` maps `(u, v)` to `(0, u, v)` with normal `+X`;
//! * an extrusion follows the plane normal, or the opposite direction when
//!   `reversed` is set;
//! * a pocket always cuts along the canonical normal, independent of whether
//!   the preceding extrusion was reversed;
//! * a circle `revolve` is deliberately lowered to an extrusion whose length is
//!   the supplied length or the circle diameter.
//!
//! Everything outside the v1 subset is refused before any geometry is built.
//! A step is never skipped and an invalid shape is never healed into a
//! different accepted result.

#![allow(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use kala_contracts::{
    AnalysisScript, AssemblyStep, ComponentScript, ComponentStep, ConstructionScript, EditStep,
};

use crate::error::{CompileError, KernelError};
use crate::native::ffi::{self, FfiError};
use crate::native::measure;
use crate::native::shape::Shape;

use crate::{
    DEFAULT_COMPILE_TIMEOUT_SECONDS, SUPPORTED_ASSEMBLY_OPS, SUPPORTED_COMPONENT_OPS,
    THROUGH_ALL_MARGIN_MM, UNSUPPORTED_PREFIX,
};

const ZERO_MM: f64 = 1e-9;
const ZERO_VOLUME_MM3: f64 = 1e-9;

/// Optional inputs whose only v1 behaviour is an explicit refusal, plus the
/// wall-clock budget. These exist so a caller can prove `analysis`, `edits` and
/// `fillets` are refused rather than silently dropped.
#[derive(Clone, Copy, Debug)]
pub struct CompileOptions<'a> {
    /// An analysis script; any step is refused by name.
    pub analysis: Option<&'a AnalysisScript>,
    /// Tool edits; the first edit is refused by name.
    pub edits: &'a [EditStep],
    /// Requested fillets; any non-empty map is refused as `fillet`.
    pub fillets: Option<&'a BTreeMap<String, f64>>,
    /// Wall-clock compile budget in seconds.
    pub timeout_seconds: f64,
}

impl Default for CompileOptions<'_> {
    fn default() -> Self {
        Self {
            analysis: None,
            edits: &[],
            fillets: None,
            timeout_seconds: DEFAULT_COMPILE_TIMEOUT_SECONDS,
        }
    }
}

/// One component's local and placed exact shape.
#[derive(Debug)]
pub struct CompiledComponent {
    /// The component name from the script.
    pub name: String,
    /// The shape in component-local coordinates.
    pub local_shape: Shape,
    /// The same shape translated by the `place` step (or the local shape when
    /// the placement is zero).
    pub placed_shape: Shape,
    /// The placement translation in millimetres.
    pub placement_mm: [f64; 3],
}

/// The components and the single placed-or-fused assembly shape.
#[derive(Debug)]
pub struct CompilationResult {
    /// Components in script order.
    pub components: Vec<CompiledComponent>,
    /// The shape a caller measures or exports.
    pub assembly_shape: Shape,
    /// Whether at least one `fuse` step participated.
    pub fused: bool,
    /// Non-fatal, honest warnings (a pocket that removed no material).
    pub warnings: Vec<String>,
}

impl CompilationResult {
    /// Return the assembly shape (the shape a caller measures or exports).
    pub fn shape(&self) -> &Shape {
        &self.assembly_shape
    }

    /// Return a compiled component by name.
    pub fn component(&self, name: &str) -> Option<&CompiledComponent> {
        self.components
            .iter()
            .find(|component| component.name == name)
    }
}

/// Compile `script` with the default five-second budget.
pub fn compile_script(script: &ConstructionScript) -> Result<CompilationResult, CompileError> {
    compile_script_with_options(script, &CompileOptions::default())
}

/// Compile `script` with explicit optional refusals and a wall-clock budget.
pub fn compile_script_with_options(
    script: &ConstructionScript,
    options: &CompileOptions<'_>,
) -> Result<CompilationResult, CompileError> {
    if !options.timeout_seconds.is_finite() || options.timeout_seconds < 0.0 {
        return Err(CompileError::invariant(
            "the compile budget must be a non-negative finite number",
        ));
    }
    let started = Instant::now();
    check_budget(started, options.timeout_seconds)?;
    refuse_unsupported(script, options)?;

    let placements = placements(script.assembly())?;
    let mut compiled = Vec::with_capacity(script.components().len());
    let mut warnings = Vec::new();
    for component in script.components() {
        check_budget(started, options.timeout_seconds)?;
        let (local_shape, component_warnings) =
            build_component(component, started, options.timeout_seconds)?;
        require_publishable(&local_shape, &format!("component '{}'", component.name()))?;
        let placement = placements
            .get(component.name())
            .copied()
            .unwrap_or((0.0, 0.0, 0.0));
        let placed_shape = if is_zero_translation(placement) {
            local_shape
                .try_clone_shape()
                .map_err(|error| CompileError::kernel(error.class_name()))?
        } else {
            let placed = translated(&local_shape, placement)?;
            require_publishable(&placed, &format!("placed component '{}'", component.name()))?;
            placed
        };
        compiled.push(CompiledComponent {
            name: component.name().to_owned(),
            local_shape,
            placed_shape,
            placement_mm: [placement.0, placement.1, placement.2],
        });
        warnings.extend(component_warnings);
    }

    check_budget(started, options.timeout_seconds)?;
    let (assembly_shape, fused) = assemble(script, &compiled)?;
    require_publishable(&assembly_shape, "assembly")?;
    check_budget(started, options.timeout_seconds)?;

    Ok(CompilationResult {
        components: compiled,
        assembly_shape,
        fused,
        warnings,
    })
}

/// Validate
/// [`kala_contracts::load_construction_json_str`] and compile the result.
pub fn compile_json_str(
    text: &str,
    options: &CompileOptions<'_>,
) -> Result<CompilationResult, CompileError> {
    let script = kala_contracts::load_construction_json_str(text)
        .map_err(|error| CompileError::contract(error.message()))?;
    compile_script_with_options(&script, options)
}

/// Validate construction JSON bytes and compile the result.
pub fn compile_json_bytes(
    bytes: &[u8],
    options: &CompileOptions<'_>,
) -> Result<CompilationResult, CompileError> {
    let script = kala_contracts::load_construction_json_bytes(bytes)
        .map_err(|error| CompileError::contract(error.message()))?;
    compile_script_with_options(&script, options)
}

// --------------------------------------------------------------------- refusals

fn refuse_unsupported(
    script: &ConstructionScript,
    options: &CompileOptions<'_>,
) -> Result<(), CompileError> {
    if let Some(analysis) = options.analysis {
        if let Some(step) = analysis.steps().first() {
            return Err(CompileError::unsupported(step.op()));
        }
    }
    if let Some(edit) = options.edits.first() {
        return Err(CompileError::unsupported(edit.op()));
    }
    if options.fillets.is_some_and(|fillets| !fillets.is_empty()) {
        return Err(CompileError::unsupported("fillet"));
    }
    for component in script.components() {
        for step in component.steps() {
            if !SUPPORTED_COMPONENT_OPS.contains(&step.op()) {
                return Err(CompileError::unsupported(step.op()));
            }
        }
    }
    for step in script.assembly() {
        if !SUPPORTED_ASSEMBLY_OPS.contains(&step.op()) {
            return Err(CompileError::unsupported(step.op()));
        }
    }
    Ok(())
}

fn placements(
    assembly: &[AssemblyStep],
) -> Result<BTreeMap<String, (f64, f64, f64)>, CompileError> {
    let mut placements = BTreeMap::new();
    for step in assembly {
        if step.op() != "place" {
            continue;
        }
        let Some(name) = step.component() else {
            return Err(CompileError::invariant(
                "a place operation must name a component",
            ));
        };
        if placements.contains_key(name) {
            return Err(CompileError::invariant(format!(
                "component '{name}' is placed more than once"
            )));
        }
        placements.insert(name.to_owned(), (step.x(), step.y(), step.z()));
    }
    Ok(placements)
}

// --------------------------------------------------------------------- components

fn build_component(
    component: &ComponentScript,
    started: Instant,
    timeout_seconds: f64,
) -> Result<(Shape, Vec<String>), CompileError> {
    let mut shape: Option<Shape> = None;
    let mut pending: Option<&ComponentStep> = None;
    let mut last_solid_plane: Option<&str> = None;
    let mut warnings = Vec::new();

    for (index, step) in component.steps().iter().enumerate() {
        check_budget(started, timeout_seconds)?;
        match step.op() {
            "sketch" => {
                pending = Some(step);
            }
            "extrude" | "revolve" => {
                let Some(sketch) = pending else {
                    return Err(CompileError::invariant(format!(
                        "{} in component '{}' has no sketch",
                        step.op(),
                        component.name()
                    )));
                };
                let solid = additive_solid(step, sketch, component.name())?;
                shape = Some(match shape {
                    None => solid,
                    Some(current) => fuse(&current, &solid, component.name())?,
                });
                last_solid_plane = Some(sketch.plane());
                pending = None;
            }
            "pocket" => {
                let Some(sketch) = pending else {
                    return Err(CompileError::invariant(format!(
                        "pocket in component '{}' has no preceding solid and sketch",
                        component.name()
                    )));
                };
                let Some(current) = shape.as_ref() else {
                    return Err(CompileError::invariant(format!(
                        "pocket in component '{}' has no preceding solid and sketch",
                        component.name()
                    )));
                };
                if Some(sketch.plane()) != last_solid_plane {
                    return Err(CompileError::unsupported_detail(format!(
                        "{UNSUPPORTED_PREFIX}pocket on {} over a {} solid",
                        sketch.plane(),
                        last_solid_plane.unwrap_or("missing")
                    )));
                }
                let (result, removed) = cut(current, sketch, step, component.name())?;
                if !removed {
                    warnings.push(format!(
                        "component '{}': pocket at step index {} on {} removed no material",
                        component.name(),
                        index,
                        sketch.plane()
                    ));
                }
                shape = Some(result);
                pending = None;
            }
            other => return Err(CompileError::unsupported(other)),
        }
    }

    let Some(shape) = shape else {
        return Err(CompileError::invariant(format!(
            "component '{}' produced no solid",
            component.name()
        )));
    };
    if pending.is_some() {
        return Err(CompileError::invariant(format!(
            "component '{}' ends with an unused sketch",
            component.name()
        )));
    }
    Ok((shape, warnings))
}

fn additive_solid(
    step: &ComponentStep,
    sketch: &ComponentStep,
    component_name: &str,
) -> Result<Shape, CompileError> {
    if profile_is(sketch, "circle") && step.op() == "revolve" {
        // The measured FreeCAD lowering: a circle revolve is an extrusion whose
        // length is the supplied length or the circle diameter.
        let length = step.length().or(sketch.diameter());
        let Some(length) = length else {
            return Err(CompileError::invariant(format!(
                "component '{component_name}' has a circle revolve without a size"
            )));
        };
        let face = profile_face(sketch)?;
        return prism(&face, plane_normal(sketch.plane())?, length);
    }

    let face = profile_face(sketch)?;
    if step.op() == "extrude" {
        let Some(length) = step.length() else {
            return Err(CompileError::invariant(format!(
                "component '{component_name}' extrudes without a length"
            )));
        };
        let mut direction = plane_normal(sketch.plane())?;
        if step.reversed() {
            direction = negated(direction);
        }
        return prism(&face, direction, length);
    }

    if sketch.plane() == "XZ" {
        return Err(CompileError::unsupported_detail(format!(
            "{UNSUPPORTED_PREFIX}revolve of a rectangle on XZ \
             (the component-local Y axis is not the profile axis)"
        )));
    }
    let angle = step.angle().unwrap_or(360.0);
    revolve(&face, angle.to_radians(), component_name)
}

fn cut(
    shape: &Shape,
    sketch: &ComponentStep,
    cut: &ComponentStep,
    component_name: &str,
) -> Result<(Shape, bool), CompileError> {
    let depth = cut_depth(shape, sketch, cut)?;
    let cutter = prism(&profile_face(sketch)?, plane_normal(sketch.plane())?, depth)?;
    let before = measure::volume(shape).map_err(wrap_kernel)?;
    let result = boolean_cut(shape, &cutter, component_name)?;
    let after = measure::volume(&result).map_err(wrap_kernel)?;
    if after <= ZERO_VOLUME_MM3 {
        return Err(CompileError::invariant(format!(
            "the cut in component '{component_name}' removed the whole solid"
        )));
    }
    let removed = before - after > (ZERO_VOLUME_MM3).max(before * 1e-12);
    require_publishable(&result, &format!("cut component '{component_name}'"))?;
    Ok((result, removed))
}

fn cut_depth(
    shape: &Shape,
    sketch: &ComponentStep,
    cut: &ComponentStep,
) -> Result<f64, CompileError> {
    if let Some(length) = cut.length() {
        return Ok(length);
    }
    let normal = plane_normal(sketch.plane())?;
    let Some(bounds) = measure::optimal_box(shape).map_err(wrap_kernel)? else {
        return Err(CompileError::invariant(
            "the pocket cannot measure the current solid",
        ));
    };
    let minimum = bounds.min_mm;
    let maximum = bounds.max_mm;
    let corners = [
        (minimum[0], minimum[1], minimum[2]),
        (minimum[0], minimum[1], maximum[2]),
        (minimum[0], maximum[1], minimum[2]),
        (minimum[0], maximum[1], maximum[2]),
        (maximum[0], minimum[1], minimum[2]),
        (maximum[0], minimum[1], maximum[2]),
        (maximum[0], maximum[1], minimum[2]),
        (maximum[0], maximum[1], maximum[2]),
    ];
    let highest = corners
        .iter()
        .map(|(x, y, z)| x * normal[0] + y * normal[1] + z * normal[2])
        .fold(f64::NEG_INFINITY, f64::max);
    // The sketch plane passes through the component-local origin. Cutting a
    // little past the body is safe; cutting short leaves a membrane.
    Ok(highest.max(0.0) + THROUGH_ALL_MARGIN_MM)
}

// --------------------------------------------------------------------- assembly

fn assemble(
    script: &ConstructionScript,
    compiled: &[CompiledComponent],
) -> Result<(Shape, bool), CompileError> {
    let placed: BTreeMap<&str, &Shape> = compiled
        .iter()
        .map(|component| (component.name.as_str(), &component.placed_shape))
        .collect();
    let ordered: Vec<&str> = compiled
        .iter()
        .map(|component| component.name.as_str())
        .collect();
    let mut fuse_names: BTreeSet<&str> = BTreeSet::new();
    for step in script.assembly() {
        if step.op() == "fuse" {
            fuse_names.extend(step.components().iter().map(String::as_str));
        }
    }
    if fuse_names.is_empty() {
        let shapes: Vec<&Shape> = ordered.iter().map(|name| placed[name]).collect();
        return Ok((compound(&shapes)?, false));
    }

    let fused_names: Vec<&str> = ordered
        .iter()
        .copied()
        .filter(|name| fuse_names.contains(name))
        .collect();
    let Some(first) = fused_names.first().copied() else {
        return Err(CompileError::invariant("a fuse step named no components"));
    };
    let mut fused_shape = placed[first]
        .try_clone_shape()
        .map_err(|error| CompileError::kernel(error.class_name()))?;
    for name in fused_names.iter().skip(1) {
        fused_shape = fuse(&fused_shape, placed[name], "assembly")?;
    }
    let remaining: Vec<&Shape> = ordered
        .iter()
        .filter(|name| !fuse_names.contains(*name))
        .map(|name| placed[name])
        .collect();
    if remaining.is_empty() {
        return Ok((fused_shape, true));
    }
    let mut shapes = Vec::with_capacity(remaining.len() + 1);
    shapes.push(&fused_shape);
    shapes.extend(remaining);
    Ok((compound(&shapes)?, true))
}

// --------------------------------------------------------------------- geometry

fn profile_face(sketch: &ComponentStep) -> Result<Shape, CompileError> {
    if profile_is(sketch, "rectangle") {
        let (Some(width), Some(height)) = (sketch.width(), sketch.height()) else {
            return Err(CompileError::invariant(
                "a rectangle sketch requires width and height",
            ));
        };
        let u0 = sketch.x();
        let v0 = sketch.y();
        let u1 = u0 + width;
        let v1 = v0 + height;
        let points = [
            to_world(sketch.plane(), u0, v0)?,
            to_world(sketch.plane(), u1, v0)?,
            to_world(sketch.plane(), u1, v1)?,
            to_world(sketch.plane(), u0, v1)?,
        ];
        return polygon_face(&points);
    }

    if profile_is(sketch, "circle") {
        let Some(diameter) = sketch.diameter() else {
            return Err(CompileError::invariant(
                "a circle sketch requires a diameter",
            ));
        };
        let centre = to_world(sketch.plane(), sketch.x(), sketch.y())?;
        let normal = plane_normal(sketch.plane())?;
        return circle_face(centre, normal, diameter / 2.0);
    }

    let profile = sketch
        .profile()
        .map_or_else(|| "None".to_owned(), |text| format!("'{text}'"));
    Err(CompileError::unsupported_detail(format!(
        "{UNSUPPORTED_PREFIX}sketch profile {profile}"
    )))
}

fn profile_is(step: &ComponentStep, expected: &str) -> bool {
    step.profile()
        .is_some_and(|profile| profile.casefold_eq_ascii(expected))
}

fn to_world(plane: &str, u: f64, v: f64) -> Result<[f64; 3], CompileError> {
    match plane {
        "XY" => Ok([u, v, 0.0]),
        "XZ" => Ok([u, 0.0, v]),
        "YZ" => Ok([0.0, u, v]),
        other => Err(CompileError::invariant(format!(
            "unknown sketch plane '{other}'"
        ))),
    }
}

fn plane_normal(plane: &str) -> Result<[f64; 3], CompileError> {
    match plane {
        "XY" => Ok([0.0, 0.0, 1.0]),
        "XZ" => Ok([0.0, -1.0, 0.0]),
        "YZ" => Ok([1.0, 0.0, 0.0]),
        other => Err(CompileError::invariant(format!(
            "unknown sketch plane '{other}'"
        ))),
    }
}

fn negated(vector: [f64; 3]) -> [f64; 3] {
    [-vector[0], -vector[1], -vector[2]]
}

fn is_zero_translation(translation: (f64, f64, f64)) -> bool {
    [translation.0, translation.1, translation.2]
        .iter()
        .all(|value| value.abs() <= ZERO_MM)
}

// --------------------------------------------------------------------- FFI wrappers

fn wrap_kernel(error: KernelError) -> CompileError {
    CompileError::kernel(error.class_name())
}

fn status_error(status: i32, error: &FfiError) -> CompileError {
    CompileError::kernel(&ffi::exception_class(status, error))
}

fn owned_shape(
    raw: *mut std::ffi::c_void,
    status: i32,
    error: &FfiError,
) -> Result<Shape, CompileError> {
    if status != ffi::OK || raw.is_null() {
        return Err(status_error(status, error));
    }
    Shape::from_owned_raw(raw).map_err(|failure| CompileError::kernel(failure.class_name()))
}

fn polygon_face(points: &[[f64; 3]; 4]) -> Result<Shape, CompileError> {
    let mut flat = [0.0_f64; 12];
    for (index, point) in points.iter().enumerate() {
        flat[index * 3] = point[0];
        flat[index * 3 + 1] = point[1];
        flat[index * 3 + 2] = point[2];
    }
    let mut raw = std::ptr::null_mut();
    let mut error = FfiError::default();
    // SAFETY: the point array is 12 valid doubles; the out-pointers are valid.
    let status = unsafe { ffi::kala_polygon_face(flat.as_ptr(), 4, &mut raw, &mut error) };
    match status {
        ffi::OK => owned_shape(raw, status, &error),
        ffi::NOT_DONE => Err(CompileError::invariant(
            "the geometry kernel could not close the rectangle",
        )),
        ffi::NULL_RESULT => Err(CompileError::invariant(
            "the geometry kernel could not build the rectangle face",
        )),
        status => Err(status_error(status, &error)),
    }
}

fn circle_face(centre: [f64; 3], normal: [f64; 3], radius: f64) -> Result<Shape, CompileError> {
    let mut raw = std::ptr::null_mut();
    let mut error = FfiError::default();
    // SAFETY: all scalar arguments are finite; the out-pointers are valid.
    let status = unsafe {
        ffi::kala_circle_face(
            centre[0], centre[1], centre[2], normal[0], normal[1], normal[2], radius, &mut raw,
            &mut error,
        )
    };
    match status {
        ffi::OK => owned_shape(raw, status, &error),
        ffi::NOT_DONE => Err(CompileError::invariant(
            "the geometry kernel could not build the circle edge",
        )),
        ffi::NULL_RESULT => Err(CompileError::invariant(
            "the geometry kernel could not build the circle wire",
        )),
        ffi::NO_TRIANGLES => Err(CompileError::invariant(
            "the geometry kernel could not build the circle face",
        )),
        status => Err(status_error(status, &error)),
    }
}

fn prism(face: &Shape, direction: [f64; 3], length: f64) -> Result<Shape, CompileError> {
    let mut raw = std::ptr::null_mut();
    let mut error = FfiError::default();
    // SAFETY: face is live; the vector is finite; the out-pointers are valid.
    let status = unsafe {
        ffi::kala_prism(
            face.as_ptr(),
            direction[0] * length,
            direction[1] * length,
            direction[2] * length,
            &mut raw,
            &mut error,
        )
    };
    match status {
        ffi::OK => owned_shape(raw, status, &error),
        ffi::NOT_DONE => Err(CompileError::invariant(
            "the geometry kernel could not extrude the profile",
        )),
        ffi::NULL_RESULT => Err(CompileError::invariant(
            "the geometry kernel returned an empty extrusion",
        )),
        status => Err(status_error(status, &error)),
    }
}

fn revolve(face: &Shape, radians: f64, component_name: &str) -> Result<Shape, CompileError> {
    let mut raw = std::ptr::null_mut();
    let mut error = FfiError::default();
    // SAFETY: face is live; the angle is finite; the out-pointers are valid.
    let status = unsafe { ffi::kala_revolve_y(face.as_ptr(), radians, &mut raw, &mut error) };
    match status {
        ffi::OK => {
            if raw.is_null() {
                // The source does not check a revolved null shape here; the
                // publishability check names the component instead.
                Shape::null().map_err(|failure| CompileError::kernel(failure.class_name()))
            } else {
                Shape::from_owned_raw(raw)
                    .map_err(|failure| CompileError::kernel(failure.class_name()))
            }
        }
        ffi::NOT_DONE => Err(CompileError::invariant(format!(
            "the geometry kernel could not revolve component '{component_name}'"
        ))),
        status => Err(status_error(status, &error)),
    }
}

fn boolean_cut(left: &Shape, right: &Shape, component_name: &str) -> Result<Shape, CompileError> {
    let mut raw = std::ptr::null_mut();
    let mut error = FfiError::default();
    // SAFETY: both shapes are live; the out-pointers are valid.
    let status =
        unsafe { ffi::kala_boolean_cut(left.as_ptr(), right.as_ptr(), &mut raw, &mut error) };
    match status {
        ffi::OK => owned_shape(raw, status, &error),
        ffi::NOT_DONE => Err(CompileError::invariant(format!(
            "the geometry kernel could not cut component '{component_name}'"
        ))),
        ffi::NULL_RESULT => Err(CompileError::invariant(format!(
            "the geometry kernel produced an empty cut in component '{component_name}'"
        ))),
        status => Err(status_error(status, &error)),
    }
}

fn fuse(left: &Shape, right: &Shape, label: &str) -> Result<Shape, CompileError> {
    let mut raw = std::ptr::null_mut();
    let mut error = FfiError::default();
    // SAFETY: both shapes are live; the out-pointers are valid.
    let status =
        unsafe { ffi::kala_boolean_fuse(left.as_ptr(), right.as_ptr(), &mut raw, &mut error) };
    match status {
        ffi::OK => owned_shape(raw, status, &error),
        ffi::NOT_DONE => Err(CompileError::invariant(format!(
            "the geometry kernel could not fuse {label}"
        ))),
        ffi::NULL_RESULT => Err(CompileError::invariant(format!(
            "the geometry kernel produced an empty fuse for {label}"
        ))),
        status => Err(status_error(status, &error)),
    }
}

fn compound(shapes: &[&Shape]) -> Result<Shape, CompileError> {
    if let [only] = shapes {
        return only
            .try_clone_shape()
            .map_err(|error| CompileError::kernel(error.class_name()));
    }
    let pointers: Vec<*const std::ffi::c_void> =
        shapes.iter().map(|shape| shape.as_ptr()).collect();
    let mut raw = std::ptr::null_mut();
    let mut error = FfiError::default();
    // SAFETY: every pointer in the slice refers to a live shape for the call;
    // the out-pointers are valid.
    let status =
        unsafe { ffi::kala_compound(pointers.as_ptr(), pointers.len(), &mut raw, &mut error) };
    if status != ffi::OK {
        return Err(status_error(status, &error));
    }
    Shape::from_owned_raw(raw).map_err(|failure| CompileError::kernel(failure.class_name()))
}

fn translated(shape: &Shape, translation: (f64, f64, f64)) -> Result<Shape, CompileError> {
    let mut raw = std::ptr::null_mut();
    let mut error = FfiError::default();
    // SAFETY: shape is live; the translation is finite; out-pointers are valid.
    let status = unsafe {
        ffi::kala_translate(
            shape.as_ptr(),
            translation.0,
            translation.1,
            translation.2,
            &mut raw,
            &mut error,
        )
    };
    match status {
        ffi::OK => owned_shape(raw, status, &error),
        ffi::NOT_DONE => Err(CompileError::invariant(
            "the geometry kernel could not place a component",
        )),
        status => Err(status_error(status, &error)),
    }
}

// --------------------------------------------------------------------- checks

fn require_publishable(shape: &Shape, label: &str) -> Result<(), CompileError> {
    if shape.is_null() {
        return Err(CompileError::invariant(format!("{label} is a null shape")));
    }
    let state = measure::publish_state(shape).map_err(wrap_kernel)?;
    if state.is_null {
        return Err(CompileError::invariant(format!("{label} is a null shape")));
    }
    if state.solid_count < 1 {
        return Err(CompileError::invariant(format!(
            "{label} contains no solid"
        )));
    }
    if state.volume <= ZERO_VOLUME_MM3 {
        return Err(CompileError::invariant(format!("{label} has zero volume")));
    }
    if !state.valid {
        return Err(CompileError::invariant(format!(
            "{label} failed the B-Rep validity check"
        )));
    }
    Ok(())
}

fn check_budget(started: Instant, timeout_seconds: f64) -> Result<(), CompileError> {
    if started.elapsed().as_secs_f64() > timeout_seconds {
        return Err(CompileError::budget(timeout_seconds));
    }
    Ok(())
}
