//! The `DesignIntent` contract, its JSON Schema and fenced-JSON parsing.
//!
//! Port of `kala_agent.studio.design.intent`. Exactly one of `template` and
//! `script` must be present; ambiguity is refused instead of guessed. Field
//! order, alias choices, strip/length constraints, discriminated-union tag
//! behaviour and the product-visible validation text all follow the live
//! Pydantic model.

use std::fmt;

use kala_contracts::{ConstructionScript, ContractError, ContractModel, Text, WireValue};
use kala_json::{CanonicalValue, Limits, canonical_dumps};
use num_bigint::BigInt;
use num_traits::ToPrimitive;

use crate::format::parse_python_float;
use crate::json::{JsonValue, parse_python_json};
use crate::limit::MAX_DIMENSION_MM as STUDIO_MAX_DIMENSION_MM;
use crate::pytext::{python_first_line, python_strip, python_truncate};

/// Maximum tolerated tolerance strings on one intent.
pub const MAX_TOLERANCES: usize = 16;
/// Maximum material-string length.
pub const MAX_MATERIAL_LENGTH: usize = 64;
/// Maximum label length.
pub const MAX_LABEL_LENGTH: usize = 120;
/// Maximum tolerance-string length.
pub const MAX_TOLERANCE_LENGTH: usize = 64;
/// Maximum shaft segments in one intent call.
pub const MAX_SEGMENTS: usize = 8;
/// Canonical L-bracket plate width used by the strict prompt parser.
pub const DEFAULT_BRACKET_WIDTH_MM: f64 = 60.0;
/// Canonical L-bracket plate depth used by the strict prompt parser.
pub const DEFAULT_BRACKET_DEPTH_MM: f64 = 45.0;

/// The instruction a remote provider prompt carries.
pub const DESIGN_INTENT_INSTRUCTION: &str = r#"You are Kala Studio's design planner. When the user asks you to design, size, or draft a part, answer with a short prose explanation and exactly one fenced ```json block containing a DesignIntent object that validates against the JSON Schema below. All dimensions are millimetres.

Use a template call when one of the templates (l_bracket, stepped_shaft, housing_envelope) matches the request, and only when every value it needs is stated. When no template matches, author a ConstructionScript from the dimensions the user stated, using only this supported subset: sketch a rectangle (width, height) or circle (diameter) on XY, XZ or YZ at an optional x/y offset; extrude it by length (reversed stays false); pocket on the same plane, omitting length for through-all or giving length for a finite depth; revolve a rectangle on XY or YZ by angle about the local Y axis (default 360); place each component once with x/y/z; fuse two or more placed components. A component is one solid: each sketch must be consumed by an extrude, revolve or pocket before the next operation, and the component must end with a solid.

Sketch coordinates are in the sketch plane: a rectangle's x/y is its lower-left corner, so it spans x to x+width and y to y+height; a circle's x/y is its centre. Every pocket must cut into material that its own component already has at that step; a cut where the component has no material removes nothing, and the build is rejected. A design may use up to 500 steps in total across up to 64 components, and one component may use all of them. Each solid body is one component: do not split a body into components just to save steps.

Never invent a dimension the user did not state: no guessed thickness, diameter, spacing, radius or angle. You may compute coordinates from stated positions, such as a hole centre given as an edge distance. Never state a volume, surface area, or envelope as a result; the geometry kernel measures those. Include material, tolerances or a label only when the user stated them.

If a value needed to build the part is missing, ask for it in prose and do not emit a JSON block.

Worked examples (each is a complete DesignIntent; do not copy one unless it matches the request):

```json
{"script":{"summary":"80 x 40 x 8 mm plate with four 6 mm through holes","components":[{"name":"plate","steps":[{"op":"sketch","plane":"XY","profile":"rectangle","width":80,"height":40},{"op":"extrude","length":8},{"op":"sketch","plane":"XY","profile":"circle","diameter":6,"x":8,"y":8},{"op":"pocket"},{"op":"sketch","plane":"XY","profile":"circle","diameter":6,"x":72,"y":8},{"op":"pocket"},{"op":"sketch","plane":"XY","profile":"circle","diameter":6,"x":72,"y":32},{"op":"pocket"},{"op":"sketch","plane":"XY","profile":"circle","diameter":6,"x":8,"y":32},{"op":"pocket"}]}]}}
```

```json
{"script":{"summary":"stepped 30/20 mm round post, 40 and 25 mm long","components":[{"name":"post","steps":[{"op":"sketch","plane":"XY","profile":"rectangle","width":15,"height":40},{"op":"revolve","angle":360},{"op":"sketch","plane":"XY","profile":"rectangle","width":10,"height":25,"y":40},{"op":"revolve","angle":360}]}]}}
```

```json
{"script":{"summary":"40 x 30 x 5 mm base with a 12 mm boss placed on top","components":[{"name":"base","steps":[{"op":"sketch","plane":"XY","profile":"rectangle","width":40,"height":30},{"op":"extrude","length":5}]},{"name":"boss","steps":[{"op":"sketch","plane":"XY","profile":"circle","diameter":12},{"op":"extrude","length":10}]}],"assembly":[{"op":"place","component":"base"},{"op":"place","component":"boss","x":20,"y":15,"z":5}]}}
```"#;

static DESIGN_INTENT_SCHEMA_TEXT: &str = include_str!("../assets/design_intent_schema.json");

/// The exported schema as readable, bounded JSON for a prompt.
pub fn design_intent_schema_text() -> &'static str {
    DESIGN_INTENT_SCHEMA_TEXT
}

/// The exported JSON Schema as a value.
pub fn design_intent_json_schema() -> CanonicalValue {
    kala_json::strict_json_loads(DESIGN_INTENT_SCHEMA_TEXT.as_bytes())
        .expect("the checked-in design intent schema is valid JSON")
}

// ---------------------------------------------------------------------------
// Validation issues
// ---------------------------------------------------------------------------

/// One element of a Pydantic-style error location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesignIssueLocation {
    /// A model field or dictionary key.
    Field(String),
    /// An array index.
    Index(usize),
}

impl fmt::Display for DesignIssueLocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Field(name) => formatter.write_str(name),
            Self::Index(index) => write!(formatter, "{index}"),
        }
    }
}

/// One validation failure at a location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignIssue {
    location: Vec<DesignIssueLocation>,
    message: String,
}

impl DesignIssue {
    fn new(location: Vec<DesignIssueLocation>, message: impl Into<String>) -> Self {
        Self {
            location,
            message: message.into(),
        }
    }

    pub(crate) fn from_parts(
        location: Vec<DesignIssueLocation>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(location, message)
    }

    /// The failing path.
    pub fn location(&self) -> &[DesignIssueLocation] {
        &self.location
    }

    /// The cleaned message (Pydantic's `Value error, ` prefix removed).
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Every validation failure for one design intent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignIntentValidationError {
    issues: Vec<DesignIssue>,
}

impl DesignIntentValidationError {
    fn new(issues: Vec<DesignIssue>) -> Self {
        Self { issues }
    }

    /// The collected issues, in Pydantic order.
    pub fn issues(&self) -> &[DesignIssue] {
        &self.issues
    }

    /// The product-visible text (`intent._validation_text`).
    pub fn message(&self) -> String {
        let mut parts = Vec::new();
        for issue in self.issues.iter().take(6) {
            let location = if issue.location.is_empty() {
                "intent".to_owned()
            } else {
                issue
                    .location
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(".")
            };
            let first_line = python_first_line(&issue.message);
            let mut message = python_strip(first_line).to_owned();
            if let Some(stripped) = message.strip_prefix("Value error, ") {
                message = stripped.to_owned();
            }
            parts.push(format!("{location}: {}", python_truncate(&message, 160)));
        }
        if parts.is_empty() {
            "the design intent was refused".to_owned()
        } else {
            parts.join("; ")
        }
    }
}

impl fmt::Display for DesignIntentValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for DesignIntentValidationError {}

/// Failure to read an intent from JSON text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesignIntentInputError {
    /// CPython `JSONDecodeError.msg`.
    Json(String),
    /// The parsed payload did not validate.
    Validation(DesignIntentValidationError),
}

impl fmt::Display for DesignIntentInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(message) => {
                write!(formatter, "the design JSON is not valid JSON: {message}")
            }
            Self::Validation(error) => formatter.write_str(&error.message()),
        }
    }
}

impl std::error::Error for DesignIntentInputError {}

/// The model reply did not contain a valid `DesignIntent`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignIntentParseError {
    message: String,
}

impl DesignIntentParseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The exact message Python's `DesignIntentParseError` carries.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for DesignIntentParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DesignIntentParseError {}

// ---------------------------------------------------------------------------
// Intent models
// ---------------------------------------------------------------------------

/// The accepted `hole_count` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoleCount {
    /// No base holes.
    Zero,
    /// Two base holes.
    Two,
    /// Four base holes.
    Four,
}

impl HoleCount {
    /// The integer value.
    pub const fn value(self) -> u8 {
        match self {
            Self::Zero => 0,
            Self::Two => 2,
            Self::Four => 4,
        }
    }

    /// Parse one of the three accepted values.
    pub const fn from_value(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Zero),
            2 => Some(Self::Two),
            4 => Some(Self::Four),
            _ => None,
        }
    }
}

impl fmt::Display for HoleCount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.value())
    }
}

/// A call to [`crate::templates::l_bracket`].
#[derive(Clone, Debug, PartialEq)]
pub struct LbracketCall {
    width: f64,
    depth: f64,
    height: f64,
    wall: f64,
    bore_d: Option<f64>,
    hole_d: Option<f64>,
    hole_count: HoleCount,
    edge: Option<f64>,
}

impl LbracketCall {
    /// Bracket width in millimetres.
    pub fn width(&self) -> f64 {
        self.width
    }
    /// Bracket depth in millimetres.
    pub fn depth(&self) -> f64 {
        self.depth
    }
    /// Bracket height in millimetres.
    pub fn height(&self) -> f64 {
        self.height
    }
    /// Wall thickness in millimetres.
    pub fn wall(&self) -> f64 {
        self.wall
    }
    /// Optional bore diameter.
    pub fn bore_d(&self) -> Option<f64> {
        self.bore_d
    }
    /// Optional base-hole diameter.
    pub fn hole_d(&self) -> Option<f64> {
        self.hole_d
    }
    /// Base-hole count.
    pub fn hole_count(&self) -> HoleCount {
        self.hole_count
    }
    /// Optional hole edge distance.
    pub fn edge(&self) -> Option<f64> {
        self.edge
    }

    /// Build a call from already parsed prompt values, using the field order
    /// and messages of the Pydantic model.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parsed(
        width: f64,
        depth: f64,
        height: f64,
        wall: f64,
        bore_d: Option<f64>,
        hole_d: Option<f64>,
        hole_count: HoleCount,
        edge: Option<f64>,
    ) -> Result<Self, DesignIntentValidationError> {
        let mut issues = Vec::new();
        for (name, value) in [
            ("width", width),
            ("depth", depth),
            ("height", height),
            ("wall", wall),
        ] {
            if let Err(message) = dimension_float_value(value) {
                issues.push(DesignIssue::new(
                    vec![DesignIssueLocation::Field(name.to_owned())],
                    message,
                ));
            }
        }
        for (name, value) in [("bore_d", bore_d), ("hole_d", hole_d), ("edge", edge)] {
            if let Some(value) = value {
                if let Err(message) = dimension_float_value(value) {
                    issues.push(DesignIssue::new(
                        vec![DesignIssueLocation::Field(name.to_owned())],
                        message,
                    ));
                }
            }
        }
        if !issues.is_empty() {
            return Err(DesignIntentValidationError::new(issues));
        }
        Ok(Self {
            width,
            depth,
            height,
            wall,
            bore_d,
            hole_d,
            hole_count,
            edge,
        })
    }
}

/// One diameter/length pair of a stepped shaft.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaftSegment {
    diameter: f64,
    length: f64,
}

impl ShaftSegment {
    /// Segment diameter in millimetres.
    pub fn diameter(&self) -> f64 {
        self.diameter
    }
    /// Segment length in millimetres.
    pub fn length(&self) -> f64 {
        self.length
    }
}

/// A call to [`crate::templates::stepped_shaft`].
#[derive(Clone, Debug, PartialEq)]
pub struct SteppedShaftCall {
    segments: Vec<ShaftSegment>,
}

impl SteppedShaftCall {
    /// The shaft segments in order.
    pub fn segments(&self) -> &[ShaftSegment] {
        &self.segments
    }

    /// Build a call from already parsed prompt values.
    ///
    /// Python constructs one `ShaftSegment` at a time, so the first segment
    /// with an invalid field decides the error; each segment reports its
    /// diameter before its length.
    pub(crate) fn from_parsed(values: &[(f64, f64)]) -> Result<Self, DesignIntentValidationError> {
        let mut segments = Vec::with_capacity(values.len());
        for (diameter, length) in values {
            let mut issues = Vec::new();
            if let Err(message) = dimension_float_value(*diameter) {
                issues.push(DesignIssue::new(
                    vec![DesignIssueLocation::Field("diameter".to_owned())],
                    message,
                ));
            }
            if let Err(message) = dimension_float_value(*length) {
                issues.push(DesignIssue::new(
                    vec![DesignIssueLocation::Field("length".to_owned())],
                    message,
                ));
            }
            if !issues.is_empty() {
                return Err(DesignIntentValidationError::new(issues));
            }
            segments.push(ShaftSegment {
                diameter: *diameter,
                length: *length,
            });
        }
        if segments.is_empty() {
            return Err(DesignIntentValidationError::new(vec![DesignIssue::new(
                vec![DesignIssueLocation::Field("segments".to_owned())],
                "Tuple should have at least 1 item after validation, not 0",
            )]));
        }
        if segments.len() > MAX_SEGMENTS {
            return Err(DesignIntentValidationError::new(vec![DesignIssue::new(
                vec![DesignIssueLocation::Field("segments".to_owned())],
                format!(
                    "Tuple should have at most {MAX_SEGMENTS} items after validation, not {}",
                    segments.len()
                ),
            )]));
        }
        Ok(Self { segments })
    }
}

/// A call to [`crate::templates::housing_envelope`].
#[derive(Clone, Debug, PartialEq)]
pub struct HousingCall {
    width: f64,
    depth: f64,
    height: f64,
    wall: f64,
}

impl HousingCall {
    /// Housing width in millimetres.
    pub fn width(&self) -> f64 {
        self.width
    }
    /// Housing depth in millimetres.
    pub fn depth(&self) -> f64 {
        self.depth
    }
    /// Housing height in millimetres.
    pub fn height(&self) -> f64 {
        self.height
    }
    /// Wall and floor thickness in millimetres.
    pub fn wall(&self) -> f64 {
        self.wall
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parsed(
        width: f64,
        depth: f64,
        height: f64,
        wall: f64,
    ) -> Result<Self, DesignIntentValidationError> {
        let mut issues = Vec::new();
        for (name, value) in [
            ("width", width),
            ("depth", depth),
            ("height", height),
            ("wall", wall),
        ] {
            if let Err(message) = dimension_float_value(value) {
                issues.push(DesignIssue::new(
                    vec![DesignIssueLocation::Field(name.to_owned())],
                    message,
                ));
            }
        }
        if !issues.is_empty() {
            return Err(DesignIntentValidationError::new(issues));
        }
        Ok(Self {
            width,
            depth,
            height,
            wall,
        })
    }
}

/// A template call: exactly one of the three families.
#[derive(Clone, Debug, PartialEq)]
pub enum TemplateCall {
    /// L-bracket.
    Lbracket(LbracketCall),
    /// Stepped shaft.
    SteppedShaft(SteppedShaftCall),
    /// Housing envelope.
    Housing(HousingCall),
}

impl TemplateCall {
    /// The discriminated-union tag.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Lbracket(_) => "l_bracket",
            Self::SteppedShaft(_) => "stepped_shaft",
            Self::Housing(_) => "housing_envelope",
        }
    }

    fn to_wire_value(&self) -> WireValue {
        match self {
            Self::Lbracket(call) => call.to_wire_value(),
            Self::SteppedShaft(call) => call.to_wire_value(),
            Self::Housing(call) => call.to_wire_value(),
        }
    }
}

impl LbracketCall {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("template", WireValue::Text(Text::new("l_bracket"))),
            ("width", WireValue::Float(self.width)),
            ("depth", WireValue::Float(self.depth)),
            ("height", WireValue::Float(self.height)),
            ("wall", WireValue::Float(self.wall)),
            ("bore_d", optional_float(self.bore_d)),
            ("hole_d", optional_float(self.hole_d)),
            (
                "hole_count",
                WireValue::Integer(BigInt::from(self.hole_count.value())),
            ),
            ("edge", optional_float(self.edge)),
        ])
    }
}

impl SteppedShaftCall {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("template", WireValue::Text(Text::new("stepped_shaft"))),
            (
                "segments",
                WireValue::array(self.segments.iter().map(|segment| {
                    WireValue::object([
                        ("diameter", WireValue::Float(segment.diameter)),
                        ("length", WireValue::Float(segment.length)),
                    ])
                })),
            ),
        ])
    }
}

impl HousingCall {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("template", WireValue::Text(Text::new("housing_envelope"))),
            ("width", WireValue::Float(self.width)),
            ("depth", WireValue::Float(self.depth)),
            ("height", WireValue::Float(self.height)),
            ("wall", WireValue::Float(self.wall)),
        ])
    }
}

/// A template call or a raw script plus declared, non-measured metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignIntent {
    template: Option<TemplateCall>,
    script: Option<ConstructionScript>,
    material: Option<Text>,
    tolerances: Vec<Text>,
    label: Option<Text>,
}

impl DesignIntent {
    /// The template call, when present.
    pub fn template(&self) -> Option<&TemplateCall> {
        self.template.as_ref()
    }
    /// The raw construction script, when present.
    pub fn script(&self) -> Option<&ConstructionScript> {
        self.script.as_ref()
    }
    /// The declared material.
    pub fn material(&self) -> Option<&Text> {
        self.material.as_ref()
    }
    /// The declared tolerance strings.
    pub fn tolerances(&self) -> &[Text] {
        &self.tolerances
    }
    /// The declared label.
    pub fn label(&self) -> Option<&Text> {
        self.label.as_ref()
    }

    pub(crate) fn from_template(
        call: TemplateCall,
        material: Option<&str>,
    ) -> Result<Self, DesignIntentValidationError> {
        Ok(Self {
            template: Some(call),
            script: None,
            material: material.map(Text::new),
            tolerances: Vec::new(),
            label: None,
        })
    }

    /// Parse and validate a JSON payload.
    pub fn from_json_str(text: &str) -> Result<Self, DesignIntentInputError> {
        let value = parse_python_json(text).map_err(DesignIntentInputError::Json)?;
        validate_design_intent(&value).map_err(DesignIntentInputError::Validation)
    }

    /// Parse and validate UTF-8 JSON bytes.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, DesignIntentInputError> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| DesignIntentInputError::Json("payload is not valid UTF-8".to_owned()))?;
        Self::from_json_str(text)
    }

    /// Validate a strict canonical value.
    pub fn from_canonical_value(
        value: &CanonicalValue,
    ) -> Result<Self, DesignIntentValidationError> {
        validate_design_intent(&crate::json::from_canonical(value))
    }

    /// The Python `model_dump(mode="json")` shape, in field order.
    pub fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            (
                "template",
                self.template
                    .as_ref()
                    .map_or(WireValue::Null, TemplateCall::to_wire_value),
            ),
            (
                "script",
                self.script
                    .as_ref()
                    .map_or(WireValue::Null, ContractModel::to_wire_value),
            ),
            (
                "material",
                self.material
                    .as_ref()
                    .map_or(WireValue::Null, |value| WireValue::Text(value.clone())),
            ),
            (
                "tolerances",
                WireValue::array(
                    self.tolerances
                        .iter()
                        .map(|value| WireValue::Text(value.clone())),
                ),
            ),
            (
                "label",
                self.label
                    .as_ref()
                    .map_or(WireValue::Null, |value| WireValue::Text(value.clone())),
            ),
        ])
    }

    /// The `model_dump(mode="json", exclude_none=True)` shape.
    pub fn to_wire_value_excluding_none(&self) -> WireValue {
        prune_nulls(self.to_wire_value())
    }

    /// Wire bytes (`model_dump(mode="json", exclude_none=False)`).
    pub fn to_wire_json(&self) -> Result<String, ContractError> {
        self.to_wire_value().encode(false)
    }

    /// ASCII-escaped wire bytes; lone surrogates survive as `\udxxx`.
    pub fn to_wire_json_ascii(&self, exclude_none: bool) -> String {
        let wire = if exclude_none {
            self.to_wire_value_excluding_none()
        } else {
            self.to_wire_value()
        };
        wire.encode(true).expect("ASCII escaping always succeeds")
    }

    /// Wire bytes with `None` fields removed.
    pub fn to_wire_json_excluding_none(&self) -> Result<String, ContractError> {
        self.to_wire_value_excluding_none().encode(false)
    }

    /// Canonical, sorted, ASCII-escaped JSON bytes.
    pub fn to_canonical_json(&self, exclude_none: bool) -> Result<String, ContractError> {
        let wire = if exclude_none {
            self.to_wire_value_excluding_none()
        } else {
            self.to_wire_value()
        };
        let value = wire.to_canonical_value()?;
        canonical_dumps(&value, &Limits::default())
            .map_err(|error| ContractError::new("DesignIntent", error.to_string()))
    }
}

fn optional_float(value: Option<f64>) -> WireValue {
    value.map_or(WireValue::Null, WireValue::Float)
}

pub(crate) fn prune_nulls(value: WireValue) -> WireValue {
    match value {
        WireValue::Object(fields) => WireValue::Object(
            fields
                .into_iter()
                .filter(|(_, value)| !matches!(value, WireValue::Null))
                .map(|(name, value)| (name, prune_nulls(value)))
                .collect(),
        ),
        WireValue::Array(values) => WireValue::Array(values.into_iter().map(prune_nulls).collect()),
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

fn dimension_float_value(value: f64) -> Result<f64, String> {
    if !value.is_finite() {
        return Err("Input should be a finite number".to_owned());
    }
    if value <= 0.0 {
        return Err("Input should be greater than 0".to_owned());
    }
    if value > STUDIO_MAX_DIMENSION_MM {
        return Err("Input should be less than or equal to 2000".to_owned());
    }
    Ok(value)
}

/// Pydantic's lax float conversion for a JSON value.
fn float_value(value: &JsonValue) -> Result<f64, String> {
    match value {
        JsonValue::Bool(value) => Ok(if *value { 1.0 } else { 0.0 }),
        JsonValue::Integer(value) => {
            let converted = value.to_f64().filter(|number| number.is_finite());
            converted.ok_or_else(|| "Input should be a valid number".to_owned())
        }
        JsonValue::Float(value) => {
            if value.is_finite() {
                Ok(*value)
            } else {
                Err("Input should be a finite number".to_owned())
            }
        }
        JsonValue::Text(value) => {
            let Some(text) = value.as_str() else {
                return Err(
                    "Input should be a valid number, unable to parse string as a number".to_owned(),
                );
            };
            match parse_python_float(text) {
                Some(number) if number.is_finite() => Ok(number),
                Some(_) => Err("Input should be a finite number".to_owned()),
                None => Err(
                    "Input should be a valid number, unable to parse string as a number".to_owned(),
                ),
            }
        }
        _ => Err("Input should be a valid number".to_owned()),
    }
}

fn dimension_float(value: &JsonValue) -> Result<f64, String> {
    dimension_float_value(float_value(value)?)
}

fn optional_dimension(value: &JsonValue) -> Result<Option<f64>, String> {
    if matches!(value, JsonValue::Null) {
        return Ok(None);
    }
    dimension_float(value).map(Some)
}

fn string_value(value: &JsonValue, minimum: usize, maximum: usize) -> Result<Text, String> {
    let Some(text) = value.as_text() else {
        return Err("Input should be a valid string".to_owned());
    };
    if text.has_surrogates() {
        return Err(
            "Input should be a valid string, unable to parse raw data as a unicode string"
                .to_owned(),
        );
    }
    let stripped = text.pydantic_strip();
    if stripped.len() < minimum {
        return Err(format!(
            "String should have at least {minimum} character{}",
            if minimum == 1 { "" } else { "s" }
        ));
    }
    if stripped.len() > maximum {
        return Err(format!("String should have at most {maximum} characters"));
    }
    Ok(stripped)
}

fn hole_count_value(value: &JsonValue) -> Result<HoleCount, String> {
    const MESSAGE: &str = "Input should be 0, 2 or 4";
    let parsed = match value {
        JsonValue::Bool(false) => Some(0u8),
        JsonValue::Bool(true) => None,
        JsonValue::Integer(value) => value.to_u8(),
        JsonValue::Float(value) => {
            if value.is_finite() && value.fract() == 0.0 && *value >= 0.0 && *value <= 255.0 {
                Some(*value as u8)
            } else {
                None
            }
        }
        _ => None,
    };
    parsed
        .and_then(HoleCount::from_value)
        .ok_or_else(|| MESSAGE.to_owned())
}

struct Reader<'a> {
    entries: &'a [(Text, JsonValue)],
    consumed: Vec<usize>,
}

impl<'a> Reader<'a> {
    fn new(entries: &'a [(Text, JsonValue)]) -> Self {
        Self {
            entries,
            consumed: Vec::new(),
        }
    }

    /// Take the first present alias in alias order, marking it consumed.
    fn take(&mut self, aliases: &[&str]) -> Option<&'a JsonValue> {
        for alias in aliases {
            if let Some((index, (_, value))) = self
                .entries
                .iter()
                .enumerate()
                .find(|(_, (key, _))| key.as_str() == Some(*alias))
            {
                if !self.consumed.contains(&index) {
                    self.consumed.push(index);
                }
                return Some(value);
            }
        }
        None
    }

    fn extras(&self, prefix: &[DesignIssueLocation]) -> Vec<DesignIssue> {
        let mut issues = Vec::new();
        for (index, (key, _)) in self.entries.iter().enumerate() {
            if !self.consumed.contains(&index) {
                let mut location = prefix.to_vec();
                location.push(DesignIssueLocation::Field(key.to_string_lossy()));
                issues.push(DesignIssue::new(location, "Extra inputs are not permitted"));
            }
        }
        issues
    }
}

fn with_field(prefix: &[DesignIssueLocation], name: &str) -> Vec<DesignIssueLocation> {
    let mut location = prefix.to_vec();
    location.push(DesignIssueLocation::Field(name.to_owned()));
    location
}

fn missing(prefix: &[DesignIssueLocation], name: &str) -> DesignIssue {
    DesignIssue::new(with_field(prefix, name), "Field required")
}

fn push_field_result<T>(
    issues: &mut Vec<DesignIssue>,
    prefix: &[DesignIssueLocation],
    name: &str,
    result: Result<T, String>,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(message) => {
            issues.push(DesignIssue::new(with_field(prefix, name), message));
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Template call validation
// ---------------------------------------------------------------------------

fn validate_lbracket(
    entries: &[(Text, JsonValue)],
    prefix: &[DesignIssueLocation],
) -> (Option<LbracketCall>, Vec<DesignIssue>) {
    let mut reader = Reader::new(entries);
    let mut issues = Vec::new();
    reader.take(&["template"]);

    let width = match reader.take(&["width", "W"]) {
        Some(value) => push_field_result(&mut issues, prefix, "width", dimension_float(value)),
        None => {
            issues.push(missing(prefix, "width"));
            None
        }
    };
    let depth = match reader.take(&["depth", "D"]) {
        Some(value) => push_field_result(&mut issues, prefix, "depth", dimension_float(value)),
        None => {
            issues.push(missing(prefix, "depth"));
            None
        }
    };
    let height = match reader.take(&["height", "H"]) {
        Some(value) => push_field_result(&mut issues, prefix, "height", dimension_float(value)),
        None => {
            issues.push(missing(prefix, "height"));
            None
        }
    };
    let wall = match reader.take(&["wall", "T"]) {
        Some(value) => push_field_result(&mut issues, prefix, "wall", dimension_float(value)),
        None => {
            issues.push(missing(prefix, "wall"));
            None
        }
    };
    let bore_d = match reader.take(&["bore_d", "B"]) {
        Some(value) => push_field_result(&mut issues, prefix, "bore_d", optional_dimension(value)),
        None => Some(None),
    };
    let hole_d = match reader.take(&["hole_d", "h"]) {
        Some(value) => push_field_result(&mut issues, prefix, "hole_d", optional_dimension(value)),
        None => Some(None),
    };
    let hole_count = match reader.take(&["hole_count", "holes", "holeCount"]) {
        Some(value) => {
            push_field_result(&mut issues, prefix, "hole_count", hole_count_value(value))
        }
        None => Some(HoleCount::Zero),
    };
    let edge = match reader.take(&["edge", "e"]) {
        Some(value) => push_field_result(&mut issues, prefix, "edge", optional_dimension(value)),
        None => Some(None),
    };
    issues.extend(reader.extras(prefix));

    if !issues.is_empty() {
        return (None, issues);
    }
    (
        Some(LbracketCall {
            width: width.expect("validated"),
            depth: depth.expect("validated"),
            height: height.expect("validated"),
            wall: wall.expect("validated"),
            bore_d: bore_d.expect("validated"),
            hole_d: hole_d.expect("validated"),
            hole_count: hole_count.expect("validated"),
            edge: edge.expect("validated"),
        }),
        issues,
    )
}

fn validate_stepped_shaft(
    entries: &[(Text, JsonValue)],
    prefix: &[DesignIssueLocation],
) -> (Option<SteppedShaftCall>, Vec<DesignIssue>) {
    let mut reader = Reader::new(entries);
    let mut issues = Vec::new();
    reader.take(&["template"]);

    let mut segments = Vec::new();
    match reader.take(&["segments"]) {
        None => issues.push(missing(prefix, "segments")),
        Some(JsonValue::Array(values)) => {
            for (index, item) in values.iter().enumerate() {
                let mut location = with_field(prefix, "segments");
                location.push(DesignIssueLocation::Index(index));
                let Some(fields) = item.entries() else {
                    issues.push(DesignIssue::new(
                        location,
                        "Input should be a valid dictionary or instance of ShaftSegment",
                    ));
                    continue;
                };
                let mut inner = Reader::new(fields);
                let mut inner_issues = Vec::new();
                let diameter = match inner.take(&["diameter"]) {
                    Some(value) => push_field_result(
                        &mut inner_issues,
                        &location,
                        "diameter",
                        dimension_float(value),
                    ),
                    None => {
                        inner_issues.push(missing(&location, "diameter"));
                        None
                    }
                };
                let length = match inner.take(&["length"]) {
                    Some(value) => push_field_result(
                        &mut inner_issues,
                        &location,
                        "length",
                        dimension_float(value),
                    ),
                    None => {
                        inner_issues.push(missing(&location, "length"));
                        None
                    }
                };
                inner_issues.extend(inner.extras(&location));
                if inner_issues.is_empty() {
                    segments.push(ShaftSegment {
                        diameter: diameter.expect("validated"),
                        length: length.expect("validated"),
                    });
                } else {
                    issues.extend(inner_issues);
                }
            }
            let location = with_field(prefix, "segments");
            if segments.len() > MAX_SEGMENTS {
                issues.push(DesignIssue::new(
                    location.clone(),
                    format!(
                        "Tuple should have at most {MAX_SEGMENTS} items after validation, not {}",
                        segments.len()
                    ),
                ));
            } else if segments.is_empty() {
                issues.push(DesignIssue::new(
                    location,
                    "Tuple should have at least 1 item after validation, not 0",
                ));
            }
        }
        Some(_) => issues.push(DesignIssue::new(
            with_field(prefix, "segments"),
            "Input should be a valid tuple",
        )),
    }
    issues.extend(reader.extras(prefix));

    if issues.is_empty() {
        (Some(SteppedShaftCall { segments }), Vec::new())
    } else {
        (None, issues)
    }
}

fn validate_housing(
    entries: &[(Text, JsonValue)],
    prefix: &[DesignIssueLocation],
) -> (Option<HousingCall>, Vec<DesignIssue>) {
    let mut reader = Reader::new(entries);
    let mut issues = Vec::new();
    reader.take(&["template"]);
    let width = match reader.take(&["width", "W"]) {
        Some(value) => push_field_result(&mut issues, prefix, "width", dimension_float(value)),
        None => {
            issues.push(missing(prefix, "width"));
            None
        }
    };
    let depth = match reader.take(&["depth", "D"]) {
        Some(value) => push_field_result(&mut issues, prefix, "depth", dimension_float(value)),
        None => {
            issues.push(missing(prefix, "depth"));
            None
        }
    };
    let height = match reader.take(&["height", "H"]) {
        Some(value) => push_field_result(&mut issues, prefix, "height", dimension_float(value)),
        None => {
            issues.push(missing(prefix, "height"));
            None
        }
    };
    let wall = match reader.take(&["wall", "T"]) {
        Some(value) => push_field_result(&mut issues, prefix, "wall", dimension_float(value)),
        None => {
            issues.push(missing(prefix, "wall"));
            None
        }
    };
    issues.extend(reader.extras(prefix));
    if issues.is_empty() {
        (
            Some(HousingCall {
                width: width.expect("validated"),
                depth: depth.expect("validated"),
                height: height.expect("validated"),
                wall: wall.expect("validated"),
            }),
            Vec::new(),
        )
    } else {
        (None, issues)
    }
}

fn validate_template(
    value: &JsonValue,
    prefix: &[DesignIssueLocation],
) -> (Option<TemplateCall>, Vec<DesignIssue>) {
    let Some(entries) = value.entries() else {
        return (
            None,
            vec![DesignIssue::new(
                prefix.to_vec(),
                "Input should be a valid dictionary or object to extract fields from",
            )],
        );
    };
    let Some(tag_value) = value.get("template") else {
        return (
            None,
            vec![DesignIssue::new(
                prefix.to_vec(),
                "Unable to extract tag using discriminator 'template'",
            )],
        );
    };
    let tag = tag_value.python_str();
    let mut call_prefix = prefix.to_vec();
    call_prefix.push(DesignIssueLocation::Field(tag.clone()));
    match tag.as_str() {
        "l_bracket" => {
            let (call, issues) = validate_lbracket(entries, &call_prefix);
            (call.map(TemplateCall::Lbracket), issues)
        }
        "stepped_shaft" => {
            let (call, issues) = validate_stepped_shaft(entries, &call_prefix);
            (call.map(TemplateCall::SteppedShaft), issues)
        }
        "housing_envelope" => {
            let (call, issues) = validate_housing(entries, &call_prefix);
            (call.map(TemplateCall::Housing), issues)
        }
        _ => (
            None,
            vec![DesignIssue::new(
                prefix.to_vec(),
                format!(
                    "Input tag '{tag}' found using 'template' does not match any of the expected tags: 'l_bracket', 'stepped_shaft', 'housing_envelope'"
                ),
            )],
        ),
    }
}

// ---------------------------------------------------------------------------
// DesignIntent validation
// ---------------------------------------------------------------------------

fn validate_design_intent(value: &JsonValue) -> Result<DesignIntent, DesignIntentValidationError> {
    let Some(entries) = value.entries() else {
        return Err(DesignIntentValidationError::new(vec![DesignIssue::new(
            Vec::new(),
            "Input should be a valid dictionary or instance of DesignIntent",
        )]));
    };
    let mut issues = Vec::new();

    let template = match value.get("template") {
        None | Some(JsonValue::Null) => None,
        Some(item) => {
            let (call, template_issues) =
                validate_template(item, &[DesignIssueLocation::Field("template".to_owned())]);
            issues.extend(template_issues);
            call
        }
    };

    let script = match value.get("script") {
        None | Some(JsonValue::Null) => None,
        Some(item) => {
            let (script, script_issues) = crate::script::validate_script(
                item,
                &[DesignIssueLocation::Field("script".to_owned())],
            );
            issues.extend(script_issues);
            script
        }
    };

    let material = match value.get("material") {
        None | Some(JsonValue::Null) => None,
        Some(item) => push_field_result(
            &mut issues,
            &[],
            "material",
            string_value(item, 1, MAX_MATERIAL_LENGTH),
        ),
    };

    let mut tolerances = Vec::new();
    match value.get("tolerances") {
        None => {}
        Some(JsonValue::Null)
        | Some(JsonValue::Bool(_))
        | Some(JsonValue::Integer(_))
        | Some(JsonValue::Float(_))
        | Some(JsonValue::Text(_)) => issues.push(DesignIssue::new(
            with_field(&[], "tolerances"),
            "Input should be a valid tuple",
        )),
        Some(JsonValue::Object(_)) => issues.push(DesignIssue::new(
            with_field(&[], "tolerances"),
            "Input should be a valid tuple",
        )),
        Some(JsonValue::Array(values)) => {
            for (index, item) in values.iter().enumerate() {
                let location = vec![
                    DesignIssueLocation::Field("tolerances".to_owned()),
                    DesignIssueLocation::Index(index),
                ];
                match string_value(item, 1, MAX_TOLERANCE_LENGTH) {
                    Ok(text) => tolerances.push(text),
                    Err(message) => issues.push(DesignIssue::new(location, message)),
                }
            }
            if tolerances.len() > MAX_TOLERANCES {
                issues.push(DesignIssue::new(
                    with_field(&[], "tolerances"),
                    format!(
                        "Tuple should have at most {MAX_TOLERANCES} items after validation, not {}",
                        tolerances.len()
                    ),
                ));
            }
        }
    }

    let label = match value.get("label") {
        None | Some(JsonValue::Null) => None,
        Some(item) => push_field_result(
            &mut issues,
            &[],
            "label",
            string_value(item, 1, MAX_LABEL_LENGTH),
        ),
    };

    for (key, _) in entries {
        let name = key.to_string_lossy();
        if !matches!(
            name.as_str(),
            "template" | "script" | "material" | "tolerances" | "label"
        ) {
            issues.push(DesignIssue::new(
                vec![DesignIssueLocation::Field(name)],
                "Extra inputs are not permitted",
            ));
        }
    }

    if issues.is_empty() && (template.is_none() == script.is_none()) {
        issues.push(DesignIssue::new(
            Vec::new(),
            "a design intent must carry exactly one of template or script",
        ));
    }

    if !issues.is_empty() {
        return Err(DesignIntentValidationError::new(issues));
    }

    Ok(DesignIntent {
        template,
        script,
        material,
        tolerances,
        label,
    })
}

// ---------------------------------------------------------------------------
// Reply parsing
// ---------------------------------------------------------------------------

/// One fenced block: the whole fence and the content inside it.
struct FenceSpan {
    outer_start: usize,
    outer_end: usize,
    content_start: usize,
    content_end: usize,
}

/// Return the fenced blocks of a reply, with an unclosed fence running to the
/// end. The language tag is skipped, so an unlabelled fence behaves like a
/// ```json fence.
fn fence_spans(text: &str) -> Vec<FenceSpan> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0;
    while let Some(offset) = text[index..].find("```") {
        let opening = index + offset;
        let mut cursor = opening + 3;
        while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t') {
            cursor += 1;
        }
        while cursor < bytes.len()
            && (bytes[cursor].is_ascii_alphanumeric() || matches!(bytes[cursor], b'_' | b'-'))
        {
            cursor += 1;
        }
        while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t') {
            cursor += 1;
        }
        if text[cursor..].starts_with("\r\n") {
            cursor += 2;
        } else if bytes.get(cursor) == Some(&b'\n') {
            cursor += 1;
        }
        match text[cursor..].find("```") {
            Some(close_offset) => {
                let closing = cursor + close_offset;
                spans.push(FenceSpan {
                    outer_start: opening,
                    outer_end: closing + 3,
                    content_start: cursor,
                    content_end: closing,
                });
                index = closing + 3;
            }
            None => {
                spans.push(FenceSpan {
                    outer_start: opening,
                    outer_end: text.len(),
                    content_start: cursor,
                    content_end: text.len(),
                });
                break;
            }
        }
    }
    spans
}

/// Return the text outside every fence.
fn outside_fences(text: &str) -> String {
    let mut outside = String::with_capacity(text.len());
    let mut last = 0;
    for fence in fence_spans(text) {
        outside.push_str(&text[last..fence.outer_start]);
        last = fence.outer_end;
    }
    outside.push_str(&text[last..]);
    outside
}

/// Return the spans of balanced `{...}` objects, string escapes respected.
fn object_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, byte) in text.bytes().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' if depth > 0 => in_string = true,
            b'{' => {
                if depth == 0 {
                    start = index;
                }
                depth += 1;
            }
            b'}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    spans.push((start, index + 1));
                }
            }
            _ => {}
        }
    }
    spans
}

/// Parse every candidate object, keeping the first CPython error text.
fn parse_object_spans(text: &str, spans: &[(usize, usize)]) -> (Vec<JsonValue>, Option<String>) {
    let mut values = Vec::new();
    let mut first_error = None;
    for (start, end) in spans {
        match parse_python_json(&text[*start..*end]) {
            Ok(value) => values.push(value),
            Err(message) => {
                if first_error.is_none() {
                    first_error = Some(message);
                }
            }
        }
    }
    (values, first_error)
}

/// Return the one design object, refusing conflicting candidates.
fn single_payload(values: Vec<JsonValue>) -> Result<JsonValue, DesignIntentParseError> {
    let first = &values[0];
    let first_canonical = first.to_canonical();
    for value in &values[1..] {
        let equal = match (&first_canonical, value.to_canonical()) {
            (Some(left), Some(right)) => left == &right,
            _ => first == value,
        };
        if !equal {
            return Err(DesignIntentParseError::new(
                "the model reply contained more than one conflicting JSON block",
            ));
        }
    }
    Ok(values
        .into_iter()
        .next()
        .expect("the candidates are not empty"))
}

/// Return the one design object in a reply, or a parse error.
fn extract_design_payload(text: &str) -> Result<JsonValue, DesignIntentParseError> {
    let fences = fence_spans(text);
    if !fences.is_empty() {
        let mut spans = Vec::new();
        for fence in &fences {
            let content = &text[fence.content_start..fence.content_end];
            spans.extend(
                object_spans(content)
                    .into_iter()
                    .map(|(start, end)| (fence.content_start + start, fence.content_start + end)),
            );
        }
        let (values, first_error) = parse_object_spans(text, &spans);
        if !values.is_empty() {
            return single_payload(values);
        }
        if fences
            .iter()
            .any(|fence| text[fence.content_start..fence.content_end].contains('{'))
        {
            return Err(DesignIntentParseError::new(format!(
                "the design JSON is not valid JSON: {}",
                first_error.unwrap_or_else(|| "Expecting value".to_owned())
            )));
        }
        return Err(DesignIntentParseError::new(
            "the model reply did not contain a fenced json block",
        ));
    }

    let outside = outside_fences(text);
    let spans = object_spans(&outside);
    let (values, first_error) = parse_object_spans(&outside, &spans);
    if !values.is_empty() {
        return single_payload(values);
    }
    if outside.contains('{') {
        return Err(DesignIntentParseError::new(format!(
            "the design JSON is not valid JSON: {}",
            first_error.unwrap_or_else(|| "Expecting value".to_owned())
        )));
    }
    Err(DesignIntentParseError::new(
        "the model reply did not contain a fenced json block",
    ))
}

/// Parse and validate the JSON design block in a model reply.
///
/// The failure text is exactly the message Python's
/// `DesignIntentParseError` carries.
pub fn parse_design_intent_text(text: &str) -> Result<DesignIntent, DesignIntentParseError> {
    if python_strip(text).is_empty() {
        return Err(DesignIntentParseError::new("the model reply was empty"));
    }
    let payload = extract_design_payload(text)?;
    validate_design_intent(&payload).map_err(|error| DesignIntentParseError::new(error.message()))
}

/// Return the prose of a reply with its design JSON removed.
pub fn strip_design_json_block(text: &str) -> String {
    let mut without_fences = outside_fences(text);
    let spans = object_spans(&without_fences);
    for (start, end) in spans.iter().rev() {
        if parse_python_json(&without_fences[*start..*end]).is_ok() {
            without_fences.replace_range(*start..*end, "");
        }
    }
    let stripped = python_strip(&without_fences);
    if stripped.is_empty() {
        "Design intent received.".to_owned()
    } else {
        stripped.to_owned()
    }
}
