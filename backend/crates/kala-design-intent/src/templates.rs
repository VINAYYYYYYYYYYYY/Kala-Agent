//! Deterministic part templates for Studio design turns.
//!
//! A template is a pure function: a bounded set of declared millimetre values
//! in, one [`ConstructionScript`] plus the declared envelope, declared
//! tolerance strings and a label out. Templates never declare a volume or a
//! surface area; those come from the kernel after the script compiles.
//!
//! The constructions follow the v1 compiler's same-plane pocket rule. The
//! L-bracket upright is therefore an `XZ` pad extruded to `y in [-T, 0]`,
//! bored through along `Y`, and placed at `y = +T` before it is fused with the
//! `XY` base plate. The housing is a floor plus four walls so the opening is on
//! `+Z`.
//!
//! Every template validates its own parameter ranges and geometric feasibility
//! before it builds a script, and returns [`TemplateError`] with the readable
//! Python message when the declared values cannot produce the part.

use std::fmt;

use kala_contracts::{ConstructionScript, ContractError, ContractModel};
use kala_json::{CanonicalObject, CanonicalValue, JsonText, Limits, canonical_dumps};

use crate::format::format_g;
use crate::intent::prune_nulls;
use crate::limit::{MAX_DIMENSION_MM, validate_finite_positive};

/// Default edge distance for base holes, in millimetres.
pub const DEFAULT_HOLE_EDGE_MM: f64 = 6.0;
/// Maximum shaft segments (the construction component budget).
pub const MAX_SEGMENTS: usize = kala_contracts::MAX_COMPONENTS;

/// The declared values cannot produce a feasible template part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateError {
    message: String,
}

impl TemplateError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The Python `TemplateError` message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for TemplateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TemplateError {}

/// An axis-aligned box in millimetres, measured or declared.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    min_mm: [f64; 3],
    max_mm: [f64; 3],
}

/// Bounds that are not a finite, ordered box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundsError {
    message: String,
}

impl BoundsError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The Python `Bounds` validation message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for BoundsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BoundsError {}

impl Bounds {
    /// Validate and build declared or measured bounds.
    ///
    /// The Python dataclass also rejects tuples that are not exactly three
    /// values; Rust's `[f64; 3]` makes that case unrepresentable.
    pub fn new(min_mm: [f64; 3], max_mm: [f64; 3]) -> Result<Self, BoundsError> {
        for value in min_mm.iter().chain(max_mm.iter()) {
            if !value.is_finite() {
                return Err(BoundsError::new("bounds must be finite numbers"));
            }
        }
        for axis in 0..3 {
            if min_mm[axis] > max_mm[axis] {
                return Err(BoundsError::new("bounds minimum exceeds maximum"));
            }
        }
        Ok(Self { min_mm, max_mm })
    }

    /// The minimum corner.
    pub fn min_mm(&self) -> [f64; 3] {
        self.min_mm
    }

    /// The maximum corner.
    pub fn max_mm(&self) -> [f64; 3] {
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

/// One diameter/length pair of a stepped shaft, in millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShaftSegment {
    /// Segment diameter in millimetres.
    pub diameter_mm: f64,
    /// Segment length in millimetres.
    pub length_mm: f64,
}

impl From<(f64, f64)> for ShaftSegment {
    fn from((diameter_mm, length_mm): (f64, f64)) -> Self {
        Self {
            diameter_mm,
            length_mm,
        }
    }
}

/// What a template declares: a script, an envelope, tolerances, a label.
#[derive(Clone, Debug, PartialEq)]
pub struct TemplateResult {
    script: ConstructionScript,
    envelope: Bounds,
    tolerances: Vec<String>,
    label: String,
}

impl TemplateResult {
    /// The construction script.
    pub fn script(&self) -> &ConstructionScript {
        &self.script
    }

    /// The declared axis-aligned envelope.
    pub fn envelope(&self) -> &Bounds {
        &self.envelope
    }

    /// The declared tolerance strings, in template order.
    pub fn tolerances(&self) -> &[String] {
        &self.tolerances
    }

    /// The declared label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The canonical script bytes used for design identity.
    ///
    /// Matches `kala_agent.studio.design.service.canonical_script_json`:
    /// `model_dump(mode="json", exclude_none=True)`, sorted keys, compact
    /// separators and ASCII escaping.
    pub fn canonical_script_json(&self) -> Result<String, ContractError> {
        let value = prune_nulls(self.script.to_wire_value()).to_canonical_value()?;
        canonical_dumps(&value, &Limits::default())
            .map_err(|error| ContractError::new("ConstructionScript", error.to_string()))
    }
}

fn dimension(name: &str, value: f64) -> Result<f64, TemplateError> {
    validate_finite_positive(name, value).map_err(TemplateError::new)
}

fn number(value: f64) -> CanonicalValue {
    CanonicalValue::float(value).expect("templates only declare finite values")
}

fn text(value: &str) -> CanonicalValue {
    CanonicalValue::Text(JsonText::new(value))
}

fn object(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    let mut object = CanonicalObject::new();
    for (key, value) in entries {
        object.insert(JsonText::new(key), value);
    }
    CanonicalValue::Object(object)
}

fn array(values: Vec<CanonicalValue>) -> CanonicalValue {
    CanonicalValue::Array(values)
}

fn sketch_rectangle(plane: &str, width: f64, height: f64, x: f64, y: f64) -> CanonicalValue {
    object(vec![
        ("op", text("sketch")),
        ("plane", text(plane)),
        ("profile", text("rectangle")),
        ("width", number(width)),
        ("height", number(height)),
        ("x", number(x)),
        ("y", number(y)),
    ])
}

fn sketch_circle(plane: &str, diameter: f64, x: f64, y: f64) -> CanonicalValue {
    object(vec![
        ("op", text("sketch")),
        ("plane", text(plane)),
        ("profile", text("circle")),
        ("diameter", number(diameter)),
        ("x", number(x)),
        ("y", number(y)),
    ])
}

fn extrude(length: f64) -> CanonicalValue {
    object(vec![("op", text("extrude")), ("length", number(length))])
}

fn pocket() -> CanonicalValue {
    object(vec![("op", text("pocket"))])
}

fn component(name: &str, steps: Vec<CanonicalValue>) -> CanonicalValue {
    object(vec![("name", text(name)), ("steps", array(steps))])
}

fn place(component: &str, x: f64, y: f64, z: f64) -> CanonicalValue {
    object(vec![
        ("op", text("place")),
        ("component", text(component)),
        ("x", number(x)),
        ("y", number(y)),
        ("z", number(z)),
    ])
}

fn fuse(names: &[&str]) -> CanonicalValue {
    object(vec![
        ("op", text("fuse")),
        (
            "components",
            array(names.iter().map(|name| text(name)).collect()),
        ),
    ])
}

fn build_script(
    summary: String,
    components: Vec<CanonicalValue>,
    assembly: Vec<CanonicalValue>,
) -> Result<ConstructionScript, TemplateError> {
    let value = object(vec![
        ("summary", text(&summary)),
        ("components", array(components)),
        ("assembly", array(assembly)),
    ]);
    ConstructionScript::from_value(&value).map_err(|error| {
        TemplateError::new(format!(
            "the generated template script was refused: {error}"
        ))
    })
}

/// Build the ADR-0009 L-bracket.
///
/// The base plate spans `x in [-W/2, W/2]`, `y in [0, D]`, `z in [0, T]`. The
/// upright spans `x in [-W/2, W/2]`, `y in [0, T]`, `z in [0, H]`. The optional
/// bore runs along Y through the upright at `(x=0, z=T+(H-T)/2)`. Base holes
/// run along Z at `x = +/- (W/2 - edge)` and `y in {T+edge, D-edge}` for four
/// holes or `y = (T+D)/2` for two.
#[allow(clippy::too_many_arguments)]
pub fn l_bracket(
    width: f64,
    depth: f64,
    height: f64,
    wall: f64,
    bore_d: Option<f64>,
    hole_d: Option<f64>,
    hole_count: i64,
    edge: Option<f64>,
) -> Result<TemplateResult, TemplateError> {
    let w = dimension("width", width)?;
    let d = dimension("depth", depth)?;
    let h = dimension("height", height)?;
    let t = dimension("wall", wall)?;
    if h <= t {
        return Err(TemplateError::new(
            "height must be greater than the wall thickness",
        ));
    }
    if d <= t {
        return Err(TemplateError::new(
            "depth must be greater than the wall thickness",
        ));
    }
    if !matches!(hole_count, 0 | 2 | 4) {
        return Err(TemplateError::new("hole_count must be 0, 2, or 4"));
    }

    let mut bore: Option<f64> = None;
    if let Some(value) = bore_d {
        let measured = dimension("bore_d", value)?;
        if measured >= w {
            return Err(TemplateError::new(
                "the bore must be narrower than the bracket width",
            ));
        }
        if measured >= h - t {
            return Err(TemplateError::new(
                "the bore must fit inside the upright leg (height - wall)",
            ));
        }
        bore = Some(measured);
    }

    let mut holes: Option<f64> = None;
    let mut hole_edge = 0.0;
    if hole_count != 0 {
        let Some(value) = hole_d else {
            return Err(TemplateError::new(
                "hole_d is required when hole_count is not 0",
            ));
        };
        let measured = dimension("hole_d", value)?;
        if measured >= t {
            return Err(TemplateError::new(
                "the base holes must be smaller than the wall thickness",
            ));
        }
        hole_edge = match edge {
            None => (1.5 * measured).max(DEFAULT_HOLE_EDGE_MM),
            Some(value) => dimension("edge", value)?,
        };
        if hole_edge <= measured / 2.0 {
            return Err(TemplateError::new(
                "the hole edge distance must exceed the hole radius",
            ));
        }
        if 2.0 * hole_edge >= w {
            return Err(TemplateError::new(
                "two edge distances do not fit across the bracket width",
            ));
        }
        if hole_edge + measured / 2.0 >= w / 2.0 {
            return Err(TemplateError::new(
                "the hole edge distance leaves too little material at the edge",
            ));
        }
        if hole_count == 4 {
            if d - t <= 2.0 * hole_edge {
                return Err(TemplateError::new(
                    "the base plate is too shallow for four holes at this edge distance",
                ));
            }
        } else if d - t <= measured {
            return Err(TemplateError::new(
                "the base plate is too shallow for two centred holes",
            ));
        }
        holes = Some(measured);
    }

    let mut base_steps = vec![sketch_rectangle("XY", w, d, -w / 2.0, 0.0), extrude(t)];
    if let Some(diameter) = holes {
        let rows: Vec<f64> = if hole_count == 4 {
            vec![t + hole_edge, d - hole_edge]
        } else {
            vec![(t + d) / 2.0]
        };
        for row in rows {
            for column in [-(w / 2.0 - hole_edge), w / 2.0 - hole_edge] {
                base_steps.push(sketch_circle("XY", diameter, column, row));
                base_steps.push(pocket());
            }
        }
    }

    let mut upright_steps = vec![sketch_rectangle("XZ", w, h, -w / 2.0, 0.0), extrude(t)];
    if let Some(diameter) = bore {
        upright_steps.push(sketch_circle("XZ", diameter, 0.0, (h + t) / 2.0));
        upright_steps.push(pocket());
    }

    let summary = format!(
        "L-bracket {} x {} x {} mm",
        format_g(w),
        format_g(d),
        format_g(h)
    );
    let script = build_script(
        summary,
        vec![
            component("base", base_steps),
            component("upright", upright_steps),
        ],
        vec![place("upright", 0.0, t, 0.0), fuse(&["base", "upright"])],
    )?;

    let mut tolerances = Vec::new();
    if let Some(diameter) = bore {
        tolerances.push(format!(
            "bore diameter {} mm (declared)",
            format_g(diameter)
        ));
    }
    if let Some(diameter) = holes {
        tolerances.push(format!(
            "{hole_count} x hole diameter {} mm (declared)",
            format_g(diameter)
        ));
    }
    tolerances.push("ISO 2768-m (declared)".to_owned());
    let envelope = Bounds {
        min_mm: [-w / 2.0, 0.0, 0.0],
        max_mm: [w / 2.0, d, h],
    };
    Ok(TemplateResult {
        script,
        envelope,
        tolerances,
        label: format!(
            "L-bracket {} x {} x {}",
            format_g(w),
            format_g(d),
            format_g(h)
        ),
    })
}

/// Build a coaxial stepped shaft from `(diameter, length)` segments.
///
/// Each segment is a circle extruded along +Z, placed at the cumulative length
/// of the segments before it, and fused into one solid.
pub fn stepped_shaft(segments: &[ShaftSegment]) -> Result<TemplateResult, TemplateError> {
    if segments.is_empty() {
        return Err(TemplateError::new(
            "a stepped shaft requires at least one segment",
        ));
    }
    if segments.len() > MAX_SEGMENTS {
        return Err(TemplateError::new(format!(
            "a stepped shaft supports at most {MAX_SEGMENTS} segments"
        )));
    }

    let mut measured = Vec::with_capacity(segments.len());
    let mut total_length = 0.0;
    for (index, segment) in segments.iter().enumerate() {
        let position = index + 1;
        let diameter = dimension(&format!("segment {position} diameter"), segment.diameter_mm)?;
        let length = dimension(&format!("segment {position} length"), segment.length_mm)?;
        measured.push((diameter, length));
        total_length += length;
        if total_length > MAX_DIMENSION_MM {
            return Err(TemplateError::new(
                "the stepped shaft length exceeds the studio dimension limit",
            ));
        }
    }

    let mut components = Vec::with_capacity(measured.len());
    let mut assembly = Vec::with_capacity(measured.len() + 1);
    let mut offset = 0.0;
    for (index, (diameter, length)) in measured.iter().enumerate() {
        let name = format!("segment_{}", index + 1);
        components.push(component(
            &name,
            vec![sketch_circle("XY", *diameter, 0.0, 0.0), extrude(*length)],
        ));
        assembly.push(place(&name, 0.0, 0.0, offset));
        offset += *length;
    }
    if measured.len() > 1 {
        let names_owned: Vec<String> = (1..=measured.len())
            .map(|index| format!("segment_{index}"))
            .collect();
        let names: Vec<&str> = names_owned.iter().map(String::as_str).collect();
        assembly.push(fuse(&names));
    }

    let max_diameter = measured
        .iter()
        .map(|(diameter, _)| *diameter)
        .fold(f64::NEG_INFINITY, f64::max);
    let summary = format!(
        "Stepped shaft {} segments, {} mm long",
        measured.len(),
        format_g(total_length)
    );
    let script = build_script(summary, components, assembly)?;

    let mut tolerances: Vec<String> = measured
        .iter()
        .enumerate()
        .map(|(index, (diameter, _))| {
            format!(
                "segment {} diameter {} mm (declared)",
                index + 1,
                format_g(*diameter)
            )
        })
        .collect();
    tolerances.push("ISO 2768-m (declared)".to_owned());
    let envelope = Bounds {
        min_mm: [-max_diameter / 2.0, -max_diameter / 2.0, 0.0],
        max_mm: [max_diameter / 2.0, max_diameter / 2.0, total_length],
    };
    Ok(TemplateResult {
        script,
        envelope,
        tolerances,
        label: format!("Stepped shaft {} segments", measured.len()),
    })
}

/// Build a rectangular housing open on +Z with floor and walls `wall` thick.
pub fn housing_envelope(
    width: f64,
    depth: f64,
    height: f64,
    wall: f64,
) -> Result<TemplateResult, TemplateError> {
    let w = dimension("width", width)?;
    let d = dimension("depth", depth)?;
    let h = dimension("height", height)?;
    let t = dimension("wall", wall)?;
    if 2.0 * t >= w {
        return Err(TemplateError::new(
            "the wall thickness leaves no interior across the width",
        ));
    }
    if 2.0 * t >= d {
        return Err(TemplateError::new(
            "the wall thickness leaves no interior across the depth",
        ));
    }
    if t >= h {
        return Err(TemplateError::new(
            "the wall thickness must be less than the housing height",
        ));
    }
    let inner_width = w - 2.0 * t;
    let components = vec![
        component(
            "floor",
            vec![sketch_rectangle("XY", w, d, -w / 2.0, -d / 2.0), extrude(t)],
        ),
        component(
            "wall_left",
            vec![sketch_rectangle("XY", t, d, -w / 2.0, -d / 2.0), extrude(h)],
        ),
        component(
            "wall_right",
            vec![
                sketch_rectangle("XY", t, d, w / 2.0 - t, -d / 2.0),
                extrude(h),
            ],
        ),
        component(
            "wall_front",
            vec![
                sketch_rectangle("XY", inner_width, t, -w / 2.0 + t, -d / 2.0),
                extrude(h),
            ],
        ),
        component(
            "wall_back",
            vec![
                sketch_rectangle("XY", inner_width, t, -w / 2.0 + t, d / 2.0 - t),
                extrude(h),
            ],
        ),
    ];
    let summary = format!(
        "Housing envelope {} x {} x {} mm",
        format_g(w),
        format_g(d),
        format_g(h)
    );
    let script = build_script(
        summary,
        components,
        vec![fuse(&[
            "floor",
            "wall_left",
            "wall_right",
            "wall_front",
            "wall_back",
        ])],
    )?;
    let envelope = Bounds {
        min_mm: [-w / 2.0, -d / 2.0, 0.0],
        max_mm: [w / 2.0, d / 2.0, h],
    };
    Ok(TemplateResult {
        script,
        envelope,
        tolerances: vec![
            format!("walls and floor {} mm (declared)", format_g(t)),
            "ISO 2768-m (declared)".to_owned(),
        ],
        label: format!(
            "Housing {} x {} x {}",
            format_g(w),
            format_g(d),
            format_g(h)
        ),
    })
}
