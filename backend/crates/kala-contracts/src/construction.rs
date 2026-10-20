//! Construction, analysis, edit and reading scripts.
//!
//! Port of `kala_agent.contracts.construction`. The model writes millimetre
//! operations; every CAD adapter lowers the same validated script. The Python
//! loader's user-visible [`ConstructionError`] message selection is preserved:
//! an unknown operation anywhere wins, otherwise the first validation message
//! is used.

use std::fmt;
use std::str::FromStr;

use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use num_bigint::BigInt;

use crate::error::{ContractError, ContractIssue};
use crate::input::OrderedValue;
use crate::scalar::as_text;
use crate::text::Text;
use crate::wire::{ContractModel, WireValue, key};

/// Maximum components in one construction script.
pub const MAX_COMPONENTS: usize = 8;
/// Maximum steps inside one component.
pub const MAX_STEPS: usize = 12;
/// Maximum absolute coordinate or size in millimetres.
pub const MAX_DIMENSION_MM: f64 = 5_000.0;

const MIN_DIMENSION_MM: f64 = 0.001;
const MAX_ASSEMBLY_STEPS: usize = 16;
const MAX_ANALYSIS_STEPS: usize = 8;
const MAX_EDITS: usize = 4;
const MAX_VERDICT_CHARS: usize = 500;
const MAX_FACE: u32 = 64;
const MAX_FORCE_N: f64 = 1_000_000.0;
const MAX_PRESSURE_MPA: f64 = 5_000.0;
const FULL_TURN: f64 = 360.0;

/// Accepted component operations.
pub const COMPONENT_OPS: [&str; 5] = ["sketch", "extrude", "revolve", "pocket", "helix"];
/// Accepted assembly operations.
pub const ASSEMBLY_OPS: [&str; 3] = ["place", "fuse", "compound"];
/// Accepted analysis operations.
pub const ANALYSIS_OPS: [&str; 6] = ["material", "fix", "force", "pressure", "mesh", "solve"];
/// Accepted tool-edit operations.
pub const EDIT_OPS: [&str; 3] = ["move", "resize", "fillet"];

const PLANES: [&str; 3] = ["XY", "XZ", "YZ"];
const PROFILE_ALIASES: [(&str, &str); 12] = [
    ("box", "rectangle"),
    ("rect", "rectangle"),
    ("rectangle", "rectangle"),
    ("rectangular", "rectangle"),
    ("square", "rectangle"),
    ("circle", "circle"),
    ("circular", "circle"),
    ("cylinder", "circle"),
    ("cylindrical", "circle"),
    ("disc", "circle"),
    ("disk", "circle"),
    ("round", "circle"),
];

/// The decision model's choice after analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RevisionAction {
    /// Accept the analysis.
    Accept,
    /// Apply the proposed tool edits.
    ToolEdit,
    /// Rewrite the construction script.
    ScriptRewrite,
}

impl RevisionAction {
    /// Every accepted value, in Python declaration order.
    pub const ALL: &'static [RevisionAction] = &[Self::Accept, Self::ToolEdit, Self::ScriptRewrite];

    /// The Python string value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::ToolEdit => "tool_edit",
            Self::ScriptRewrite => "script_rewrite",
        }
    }

    /// Parse an exact Python enum value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "accept" => Some(Self::Accept),
            "tool_edit" => Some(Self::ToolEdit),
            "script_rewrite" => Some(Self::ScriptRewrite),
            _ => None,
        }
    }
}

impl fmt::Display for RevisionAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for RevisionAction {
    type Err = ContractError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value).ok_or_else(|| {
            ContractError::new(
                "RevisionAction",
                "Input should be a valid RevisionAction value",
            )
        })
    }
}

/// A construction script refused by the shared protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstructionError {
    message: String,
}

impl ConstructionError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The cleaned first message the Python loader would raise.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ConstructionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ConstructionError {}

// ---------------------------------------------------------------------------
// Ordered model input
// ---------------------------------------------------------------------------

/// Construction models care about object insertion order because Python's
/// `_flatten_profile` before-validator chooses the first profile alias it sees.
/// These scopes name the only places where a `ComponentStep` is validated, so
/// profile hints never walk arbitrary extra/attribute subtrees.
#[derive(Clone, Copy)]
enum ProfileScope {
    None,
    ComponentStep,
    ComponentScript,
    ConstructionScript,
    AnalysisReading,
}

fn construction_from_json_str<T: ContractModel>(
    text: &str,
    scope: ProfileScope,
) -> Result<T, ContractError> {
    let mut input = crate::input::parse_json_str(text)?;
    apply_profile_scope(input.root_mut(), scope);
    let value = input.into_canonical();
    T::from_value(&value)
}

fn construction_from_json_bytes<T: ContractModel>(
    bytes: &[u8],
    scope: ProfileScope,
) -> Result<T, ContractError> {
    let mut input = crate::input::parse_json_bytes(bytes)?;
    apply_profile_scope(input.root_mut(), scope);
    let value = input.into_canonical();
    T::from_value(&value)
}

/// Apply Python's first-alias interpretation to the ordered input.
///
/// Only the fixed construction paths are visited. In particular a huge or
/// deeply nested arbitrary extra field is never walked just to look for a
/// `profile` key.
fn apply_profile_scope(root: &mut OrderedValue, scope: ProfileScope) {
    match scope {
        ProfileScope::None => {}
        ProfileScope::ComponentStep => apply_profile_hint(root),
        ProfileScope::ComponentScript => apply_step_profiles(root),
        ProfileScope::ConstructionScript => {
            let Some(components) =
                object_field_mut(root, "components").and_then(OrderedValue::as_array_mut)
            else {
                return;
            };
            for component in components {
                apply_step_profiles(component);
            }
        }
        ProfileScope::AnalysisReading => {
            if let Some(rewrite) = object_field_mut(root, "rewrite") {
                apply_step_profiles(rewrite);
            }
        }
    }
}

fn apply_step_profiles(script: &mut OrderedValue) {
    let Some(steps) = object_field_mut(script, "steps").and_then(OrderedValue::as_array_mut) else {
        return;
    };
    for step in steps {
        apply_profile_hint(step);
    }
}

fn object_field_mut<'a>(value: &'a mut OrderedValue, name: &str) -> Option<&'a mut OrderedValue> {
    let entries = value.as_object_mut()?;
    entries
        .iter_mut()
        .find(|(key, _)| key_matches(key, name))
        .map(|(_, value)| value)
}

fn key_matches(key: &JsonText, name: &str) -> bool {
    key.to_utf8().is_ok_and(|text| text == name)
}

enum ProfileSelection {
    Alias(String),
    Named,
    None,
}

fn apply_profile_hint(step: &mut OrderedValue) {
    let Some(entries) = step.as_object_mut() else {
        return;
    };
    let Some((_, profile)) = entries
        .iter_mut()
        .find(|(key, _)| key_matches(key, "profile"))
    else {
        return;
    };
    let Some(profile_entries) = profile.as_object_mut() else {
        return;
    };
    match profile_selection(profile_entries) {
        ProfileSelection::Alias(chosen) => {
            profile_entries.retain(|(name, _)| {
                let rendered = name.to_string_lossy();
                rendered == chosen || !is_alias_key(name)
            });
        }
        ProfileSelection::Named => {
            profile_entries.retain(|(name, _)| !is_alias_key(name));
        }
        ProfileSelection::None => {}
    }
}

fn profile_selection(profile: &[(JsonText, OrderedValue)]) -> ProfileSelection {
    for (name, value) in profile {
        if matches!(value, OrderedValue::Object(_)) && is_alias_key(name) {
            return ProfileSelection::Alias(name.to_string_lossy());
        }
    }
    for name in ["type", "kind", "shape"] {
        let Some((_, value)) = profile.iter().find(|(key, _)| key_matches(key, name)) else {
            continue;
        };
        if !truthy_ordered(value) {
            continue;
        }
        let Some(text) = ordered_text(value) else {
            return ProfileSelection::None;
        };
        if text.python_strip().is_empty() {
            return ProfileSelection::None;
        }
        return ProfileSelection::Named;
    }
    ProfileSelection::None
}

fn is_alias_key(name: &JsonText) -> bool {
    profile_alias(&Text::from_json_text(name.clone())).is_some()
}

fn ordered_text(value: &OrderedValue) -> Option<Text> {
    value
        .as_text()
        .map(|text| Text::from_json_text(text.clone()))
}

fn truthy_ordered(value: &OrderedValue) -> bool {
    match value {
        OrderedValue::Null => false,
        OrderedValue::Bool(value) => *value,
        OrderedValue::Integer(value) => value.sign() != num_bigint::Sign::NoSign,
        OrderedValue::Float(value) => *value != 0.0,
        OrderedValue::Text(value) => !value.is_empty(),
        OrderedValue::Array(values) => !values.is_empty(),
        OrderedValue::Object(values) => !values.is_empty(),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn object<'a>(
    value: &'a CanonicalValue,
    model: &'static str,
) -> Result<&'a CanonicalObject, ContractError> {
    match value {
        CanonicalValue::Object(object) => Ok(object),
        _ => Err(ContractError::new(
            model,
            format!("Input should be a valid dictionary or instance of {model}"),
        )),
    }
}

fn push(issues: &mut Vec<ContractIssue>, field: &str, message: impl Into<String>) {
    issues.push(ContractIssue::new(field, message));
}

fn flatten_nested<T>(
    map: &CanonicalObject,
    field: &str,
    issues: &mut Vec<ContractIssue>,
    parse: impl Fn(&CanonicalValue) -> Result<T, String>,
) -> Option<T> {
    match map.get(&key(field)) {
        Some(value) => match parse(value) {
            Ok(value) => Some(value),
            Err(message) => {
                push(issues, field, message);
                None
            }
        },
        None => {
            push(issues, field, "Field required");
            None
        }
    }
}

fn optional_value<T>(
    map: &CanonicalObject,
    field: &str,
    issues: &mut Vec<ContractIssue>,
    parse: impl Fn(&CanonicalValue) -> Result<T, String>,
) -> Option<T> {
    match map.get(&key(field)) {
        None | Some(CanonicalValue::Null) => None,
        Some(value) => match parse(value) {
            Ok(value) => Some(value),
            Err(message) => {
                push(issues, field, message);
                None
            }
        },
    }
}

fn default_value<T>(
    map: &CanonicalObject,
    field: &str,
    issues: &mut Vec<ContractIssue>,
    default: impl FnOnce() -> T,
    parse: impl Fn(&CanonicalValue) -> Result<T, String>,
) -> T {
    match map.get(&key(field)) {
        None => default(),
        Some(value) => match parse(value) {
            Ok(value) => value,
            Err(message) => {
                push(issues, field, message);
                default()
            }
        },
    }
}

fn array_value<T>(
    map: &CanonicalObject,
    field: &str,
    issues: &mut Vec<ContractIssue>,
    parse: impl Fn(&CanonicalValue) -> Result<T, String>,
) -> Vec<T> {
    match map.get(&key(field)) {
        None => Vec::new(),
        Some(CanonicalValue::Array(values)) => {
            let mut parsed = Vec::with_capacity(values.len());
            for (index, value) in values.iter().enumerate() {
                match parse(value) {
                    Ok(value) => parsed.push(value),
                    Err(message) => push(issues, &format!("{field}[{index}]"), message),
                }
            }
            parsed
        }
        Some(_) => {
            push(issues, field, "Input should be a valid tuple");
            Vec::new()
        }
    }
}

fn length_bounds(
    values: usize,
    min: usize,
    max: usize,
    issues: &mut Vec<ContractIssue>,
    field: &str,
) {
    if values < min {
        push(
            issues,
            field,
            format!(
                "Tuple should have at least {min} {} after validation, not {values}",
                if min == 1 { "item" } else { "items" }
            ),
        );
    }
    if values > max {
        push(
            issues,
            field,
            format!("Tuple should have at most {max} items after validation, not {values}"),
        );
    }
}

fn reject_extra(map: &CanonicalObject, allowed: &[&str], issues: &mut Vec<ContractIssue>) {
    for name in map.keys() {
        let rendered = name.to_string_lossy();
        if !allowed.contains(&rendered.as_str()) {
            push(issues, &rendered, "Extra inputs are not permitted");
        }
    }
}

fn finite_number(value: &CanonicalValue, label: &str) -> Result<f64, String> {
    match value {
        CanonicalValue::Integer(number) => {
            let parsed = num_traits::ToPrimitive::to_f64(number)
                .filter(|number| number.is_finite())
                .ok_or_else(|| format!("{label} must be a finite number"))?;
            Ok(parsed)
        }
        CanonicalValue::Float(number) if number.is_finite() => Ok(*number),
        _ => Err(format!("{label} must be a finite number")),
    }
}

fn optional_size(value: &CanonicalValue, label: &str) -> Result<Option<f64>, String> {
    if matches!(value, CanonicalValue::Null) {
        return Ok(None);
    }
    let number = finite_number(value, label)?;
    if number < MIN_DIMENSION_MM || number > MAX_DIMENSION_MM {
        return Err(format!("{label} is outside the model bounds"));
    }
    Ok(Some(number))
}

fn coordinate(value: &CanonicalValue, label: &str) -> Result<f64, String> {
    let number = finite_number(value, label)?;
    if number.abs() > MAX_DIMENSION_MM {
        return Err(format!("{label} is outside the model bounds"));
    }
    Ok(number)
}

fn optional_open_interval(
    value: &CanonicalValue,
    label: &str,
    high: f64,
) -> Result<Option<f64>, String> {
    if matches!(value, CanonicalValue::Null) {
        return Ok(None);
    }
    let number = finite_number(value, label)?;
    if !(0.0 < number && number <= high) {
        return Err(format!("{label} is outside the model bounds"));
    }
    Ok(Some(number))
}

fn known_operation(value: &CanonicalValue, allowed: &[&str]) -> Result<String, String> {
    if let Some(text) = as_text(value) {
        if let Some(value) = text.as_str() {
            if allowed.contains(&value) {
                return Ok(value.to_owned());
            }
            if !value.is_empty() {
                return Err(format!("unknown operation {value}"));
            }
        } else if !text.is_empty() {
            return Err(format!("unknown operation {}", text.to_string_lossy()));
        }
    }
    Err("unknown operation".to_owned())
}

fn component_name_value(value: &CanonicalValue) -> Result<String, String> {
    let text = as_text(value).ok_or_else(|| "Input should be a valid string".to_owned())?;
    let utf8 = text
        .to_utf8()
        .map_err(|_| "a component name must match ^[A-Za-z][A-Za-z0-9_-]{0,39}$".to_owned())?;
    component_name(&utf8)
}

fn component_name(value: &str) -> Result<String, String> {
    if !is_component_name(value) {
        return Err("a component name must match ^[A-Za-z][A-Za-z0-9_-]{0,39}$".to_owned());
    }
    Ok(value.to_owned())
}

fn is_component_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 40 || !bytes[0].is_ascii_alphabetic() {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn nonempty_text(value: &CanonicalValue, label: &str) -> Result<Text, String> {
    let text = as_text(value).ok_or_else(|| format!("{label} must be nonempty text"))?;
    let stripped = text.python_strip();
    if stripped.is_empty() {
        return Err(format!("{label} must be nonempty text"));
    }
    Ok(stripped)
}

fn bounded_text(value: &CanonicalValue, label: &str, limit: usize) -> Result<Text, String> {
    let text = as_text(value).ok_or_else(|| format!("{label} must be nonempty text"))?;
    let stripped = text.python_strip();
    if stripped.is_empty() || stripped.len() > limit {
        return Err(format!(
            "{label} must be nonempty and at most {limit} characters"
        ));
    }
    Ok(stripped)
}

fn profile_value(value: &CanonicalValue) -> Result<Text, String> {
    let text = as_text(value).ok_or_else(|| "profile must be rectangle or circle".to_owned())?;
    let stripped = text.python_strip();
    if stripped.is_empty() {
        return Err("profile must be rectangle or circle".to_owned());
    }
    Ok(profile_alias(&stripped).map_or_else(|| stripped.casefold(), Text::new))
}

fn profile_alias(value: &Text) -> Option<&'static str> {
    PROFILE_ALIASES
        .iter()
        .find(|(name, _)| value.casefold_eq_ascii(name))
        .map(|(_, alias)| *alias)
}

fn profile_is(value: &Option<Text>, expected: &str) -> bool {
    value
        .as_ref()
        .is_some_and(|profile| profile.casefold_eq_ascii(expected))
}

// ---------------------------------------------------------------------------
// ComponentStep
// ---------------------------------------------------------------------------

/// One operation inside a component.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentStep {
    op: String,
    plane: String,
    profile: Option<Text>,
    width: Option<f64>,
    height: Option<f64>,
    diameter: Option<f64>,
    length: Option<f64>,
    pitch: Option<f64>,
    angle: Option<f64>,
    x: f64,
    y: f64,
    reversed: bool,
}

impl ContractModel for ComponentStep {
    const NAME: &'static str = "ComponentStep";

    fn from_json_str(text: &str) -> Result<Self, ContractError> {
        construction_from_json_str(text, ProfileScope::ComponentStep)
    }

    fn from_json_bytes(bytes: &[u8]) -> Result<Self, ContractError> {
        construction_from_json_bytes(bytes, ProfileScope::ComponentStep)
    }

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let flattened = flatten_profile(value);
        let value = &flattened;
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let op = flatten_nested(map, "op", &mut issues, |value| {
            known_operation(value, &COMPONENT_OPS)
        });
        let plane = default_value(
            map,
            "plane",
            &mut issues,
            || "XY".to_owned(),
            |value| {
                let text =
                    as_text(value).ok_or_else(|| "Input should be a valid string".to_owned())?;
                let utf8 = text
                    .to_utf8()
                    .map_err(|_| "plane must be XY, XZ, or YZ".to_owned())?;
                if PLANES.contains(&utf8.as_str()) {
                    Ok(utf8)
                } else {
                    Err("plane must be XY, XZ, or YZ".to_owned())
                }
            },
        );
        let profile = optional_value(map, "profile", &mut issues, profile_value);
        let width = optional_value(map, "width", &mut issues, |value| {
            optional_size(value, "width")
        })
        .flatten();
        let height = optional_value(map, "height", &mut issues, |value| {
            optional_size(value, "height")
        })
        .flatten();
        let diameter = optional_value(map, "diameter", &mut issues, |value| {
            optional_size(value, "diameter")
        })
        .flatten();
        let length = optional_value(map, "length", &mut issues, |value| {
            optional_size(value, "length")
        })
        .flatten();
        let pitch = optional_value(map, "pitch", &mut issues, |value| {
            optional_size(value, "pitch")
        })
        .flatten();
        let angle = optional_value(map, "angle", &mut issues, |value| {
            optional_size(value, "angle")
        })
        .flatten();
        let x = default_value(
            map,
            "x",
            &mut issues,
            || 0.0,
            |value| coordinate(value, "x"),
        );
        let y = default_value(
            map,
            "y",
            &mut issues,
            || 0.0,
            |value| coordinate(value, "y"),
        );
        let reversed = default_value(
            map,
            "reversed",
            &mut issues,
            || false,
            |value| match value {
                CanonicalValue::Bool(value) => Ok(*value),
                _ => Err("Input should be a valid boolean".to_owned()),
            },
        );
        reject_extra(
            map,
            &[
                "op", "plane", "profile", "width", "height", "diameter", "length", "pitch",
                "angle", "x", "y", "reversed",
            ],
            &mut issues,
        );
        let mut angle = angle;
        if issues.is_empty() {
            let op = op.as_deref().expect("validated");
            let mut profile = profile;
            match op {
                "sketch" => {
                    normalize_profile(&mut profile, diameter, width, height);
                    if let Err(message) = check_sketch(&profile, width, height, diameter) {
                        push(&mut issues, "profile", message);
                    }
                }
                "extrude" => {
                    if length.is_none() {
                        push(&mut issues, "length", "extrude requires a length");
                    }
                }
                "revolve" => {
                    if angle.is_none() {
                        angle = Some(FULL_TURN);
                    } else if angle.is_some_and(|value| value > FULL_TURN) {
                        push(&mut issues, "angle", "revolve angle must be in (0, 360]");
                    }
                }
                "helix" => {
                    if diameter.is_none() {
                        push(&mut issues, "diameter", "helix requires a diameter");
                    }
                    if pitch.is_none() {
                        push(&mut issues, "pitch", "helix requires a pitch");
                    }
                    if length.is_none() {
                        push(&mut issues, "length", "helix requires a length");
                    }
                }
                _ => {}
            }
            if reversed && op != "extrude" {
                push(
                    &mut issues,
                    "reversed",
                    "reversed is only meaningful on extrude",
                );
            }
            if !issues.is_empty() {
                return Err(ContractError::from_issues(Self::NAME, issues));
            }
            return Ok(Self {
                op: op.to_owned(),
                plane,
                profile,
                width,
                height,
                diameter,
                length,
                pitch,
                angle,
                x,
                y,
                reversed,
            });
        }
        Err(ContractError::from_issues(Self::NAME, issues))
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("op", WireValue::Text(Text::new(&self.op))),
            ("plane", WireValue::Text(Text::new(&self.plane))),
            (
                "profile",
                self.profile
                    .as_ref()
                    .map_or(WireValue::Null, |value| WireValue::Text(value.clone())),
            ),
            ("width", opt_f64(self.width)),
            ("height", opt_f64(self.height)),
            ("diameter", opt_f64(self.diameter)),
            ("length", opt_f64(self.length)),
            ("pitch", opt_f64(self.pitch)),
            ("angle", opt_f64(self.angle)),
            ("x", WireValue::Float(self.x)),
            ("y", WireValue::Float(self.y)),
            ("reversed", WireValue::Bool(self.reversed)),
        ])
    }
}

impl ComponentStep {
    pub fn op(&self) -> &str {
        &self.op
    }
    pub fn plane(&self) -> &str {
        &self.plane
    }
    pub fn profile(&self) -> Option<&Text> {
        self.profile.as_ref()
    }
    pub fn width(&self) -> Option<f64> {
        self.width
    }
    pub fn height(&self) -> Option<f64> {
        self.height
    }
    pub fn diameter(&self) -> Option<f64> {
        self.diameter
    }
    pub fn length(&self) -> Option<f64> {
        self.length
    }
    pub fn pitch(&self) -> Option<f64> {
        self.pitch
    }
    pub fn angle(&self) -> Option<f64> {
        self.angle
    }
    pub fn x(&self) -> f64 {
        self.x
    }
    pub fn y(&self) -> f64 {
        self.y
    }
    pub fn reversed(&self) -> bool {
        self.reversed
    }
}

fn flatten_profile(value: &CanonicalValue) -> CanonicalValue {
    let CanonicalValue::Object(map) = value else {
        return value.clone();
    };
    let Some(CanonicalValue::Object(profile)) = map.get(&key("profile")) else {
        return value.clone();
    };
    let mut flat = map.clone();
    let mut shape: Option<&str> = None;
    let mut body: Option<&CanonicalObject> = None;
    for (name, item) in profile {
        let CanonicalValue::Object(item) = item else {
            continue;
        };
        let name_text = Text::from_json_text(name.clone());
        if let Some(alias) = profile_alias(&name_text) {
            shape = Some(alias);
            body = Some(item);
            break;
        }
    }
    if shape.is_none() {
        let named = ["type", "kind", "shape"]
            .iter()
            .filter_map(|name| profile.get(&key(name)))
            .find(|value| truthy(value));
        if let Some(named) = named {
            if let Some(text) = as_text(named) {
                let stripped = text.python_strip();
                if !stripped.is_empty() {
                    if let Some(alias) = profile_alias(&stripped) {
                        shape = Some(alias);
                    } else {
                        let folded = stripped.casefold();
                        flat.insert(
                            key("profile"),
                            CanonicalValue::Text(folded.as_json_text().clone()),
                        );
                        shape = Some("");
                    }
                    body = Some(profile);
                }
            }
        }
    }
    if let Some(shape) = shape {
        if !shape.is_empty() {
            flat.insert(key("profile"), CanonicalValue::text(shape));
        }
    }
    if let Some(body) = body {
        for size in ["width", "height", "diameter", "length", "pitch"] {
            if let Some(item) = body.get(&key(size)) {
                if flat
                    .get(&key(size))
                    .is_none_or(|value| matches!(value, CanonicalValue::Null))
                {
                    flat.insert(key(size), item.clone());
                }
            }
        }
    }
    for coordinate in ["x", "y"] {
        if let Some(item) = profile.get(&key(coordinate)) {
            flat.entry(key(coordinate)).or_insert_with(|| item.clone());
        }
    }
    CanonicalValue::Object(flat)
}

fn truthy(value: &CanonicalValue) -> bool {
    match value {
        CanonicalValue::Null => false,
        CanonicalValue::Bool(value) => *value,
        CanonicalValue::Integer(value) => value.sign() != num_bigint::Sign::NoSign,
        CanonicalValue::Float(value) => *value != 0.0,
        CanonicalValue::Text(value) => !value.is_empty(),
        CanonicalValue::Array(values) => !values.is_empty(),
        CanonicalValue::Object(values) => !values.is_empty(),
    }
}

fn normalize_profile(
    profile: &mut Option<Text>,
    diameter: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
) {
    if profile_is(profile, "rectangle") || profile_is(profile, "circle") {
        return;
    }
    if diameter.is_some() && (width.is_none() || height.is_none()) {
        *profile = Some(Text::new("circle"));
        return;
    }
    if width.is_some() && height.is_some() {
        *profile = Some(Text::new("rectangle"));
    }
}

fn check_sketch(
    profile: &Option<Text>,
    width: Option<f64>,
    height: Option<f64>,
    diameter: Option<f64>,
) -> Result<(), &'static str> {
    if profile_is(profile, "rectangle") {
        if width.is_none() || height.is_none() {
            return Err("a rectangle sketch requires width and height");
        }
        return Ok(());
    }
    if profile_is(profile, "circle") {
        if diameter.is_none() {
            return Err("a circle sketch requires a diameter");
        }
        return Ok(());
    }
    Err("profile must be rectangle or circle")
}

// ---------------------------------------------------------------------------
// ComponentScript
// ---------------------------------------------------------------------------

/// The operations that build one named solid.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentScript {
    name: String,
    steps: Vec<ComponentStep>,
}

impl ContractModel for ComponentScript {
    const NAME: &'static str = "ComponentScript";

    fn from_json_str(text: &str) -> Result<Self, ContractError> {
        construction_from_json_str(text, ProfileScope::ComponentScript)
    }

    fn from_json_bytes(bytes: &[u8]) -> Result<Self, ContractError> {
        construction_from_json_bytes(bytes, ProfileScope::ComponentScript)
    }

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let name = flatten_nested(map, "name", &mut issues, component_name_value);
        let steps = model_array_required::<ComponentStep>(map, "steps", &mut issues);
        length_bounds(steps.len(), 1, MAX_STEPS, &mut issues, "steps");
        reject_extra(map, &["name", "steps"], &mut issues);
        if issues.is_empty() {
            if let Err(message) = check_component_sequence(&steps) {
                push(&mut issues, "steps", message);
            }
        }
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            name: name.expect("validated"),
            steps,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("name", WireValue::Text(Text::new(&self.name))),
            (
                "steps",
                WireValue::Array(
                    self.steps
                        .iter()
                        .map(ContractModel::to_wire_value)
                        .collect(),
                ),
            ),
        ])
    }
}

impl ComponentScript {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn steps(&self) -> &[ComponentStep] {
        &self.steps
    }
}

fn model_array_required<T: ContractModel>(
    map: &CanonicalObject,
    field: &str,
    issues: &mut Vec<ContractIssue>,
) -> Vec<T> {
    match map.get(&key(field)) {
        None => {
            push(issues, field, "Field required");
            Vec::new()
        }
        Some(CanonicalValue::Array(values)) => {
            let mut parsed = Vec::with_capacity(values.len());
            for (index, value) in values.iter().enumerate() {
                match T::from_value(value) {
                    Ok(value) => parsed.push(value),
                    Err(error) => {
                        for issue in error.issues() {
                            issues.push(issue.clone().under(&format!("{field}[{index}]")));
                        }
                    }
                }
            }
            parsed
        }
        Some(_) => {
            push(issues, field, "Input should be a valid tuple");
            Vec::new()
        }
    }
}

fn check_component_sequence(steps: &[ComponentStep]) -> Result<(), &'static str> {
    let mut pending = false;
    let mut solid = false;
    for step in steps {
        match step.op() {
            "sketch" => {
                if pending {
                    return Err("a sketch must be consumed before another sketch");
                }
                pending = true;
            }
            "helix" => {
                if !solid {
                    return Err("a helix must follow a solid");
                }
                if pending {
                    return Err("a sketch must be consumed before a helix");
                }
            }
            "pocket" => {
                if !solid {
                    return Err("a pocket must follow a solid");
                }
                if !pending {
                    return Err("a pocket must follow a sketch");
                }
                pending = false;
            }
            "extrude" | "revolve" => {
                if !pending {
                    return Err(if step.op() == "extrude" {
                        "extrude must follow a sketch"
                    } else {
                        "revolve must follow a sketch"
                    });
                }
                pending = false;
                solid = true;
            }
            _ => return Err("unknown operation"),
        }
    }
    if !solid {
        return Err("a component must produce a solid");
    }
    if pending {
        return Err("a sketch must be consumed");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// AssemblyStep
// ---------------------------------------------------------------------------

/// Place a component, or fuse or compound several of them.
#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyStep {
    op: String,
    component: Option<String>,
    components: Vec<String>,
    x: f64,
    y: f64,
    z: f64,
}

impl ContractModel for AssemblyStep {
    const NAME: &'static str = "AssemblyStep";

    fn from_json_str(text: &str) -> Result<Self, ContractError> {
        construction_from_json_str(text, ProfileScope::None)
    }

    fn from_json_bytes(bytes: &[u8]) -> Result<Self, ContractError> {
        construction_from_json_bytes(bytes, ProfileScope::None)
    }

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let op = flatten_nested(map, "op", &mut issues, |value| {
            known_operation(value, &ASSEMBLY_OPS)
        });
        let component = optional_value(map, "component", &mut issues, component_name_value);
        let components = array_value(map, "components", &mut issues, component_name_value);
        let x = default_value(
            map,
            "x",
            &mut issues,
            || 0.0,
            |value| coordinate(value, "x"),
        );
        let y = default_value(
            map,
            "y",
            &mut issues,
            || 0.0,
            |value| coordinate(value, "y"),
        );
        let z = default_value(
            map,
            "z",
            &mut issues,
            || 0.0,
            |value| coordinate(value, "z"),
        );
        reject_extra(
            map,
            &["op", "component", "components", "x", "y", "z"],
            &mut issues,
        );
        if issues.is_empty() {
            match op.as_deref() {
                Some("place") if component.is_none() => {
                    push(&mut issues, "", "place requires a component");
                }
                Some("place") => {}
                Some(other) if components.len() < 2 => {
                    push(
                        &mut issues,
                        "",
                        format!("{other} requires at least two components"),
                    );
                }
                _ => {}
            }
        }
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            op: op.expect("validated"),
            component,
            components,
            x,
            y,
            z,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("op", WireValue::Text(Text::new(&self.op))),
            (
                "component",
                self.component
                    .as_ref()
                    .map_or(WireValue::Null, |value| WireValue::Text(Text::new(value))),
            ),
            (
                "components",
                WireValue::Array(
                    self.components
                        .iter()
                        .map(|value| WireValue::Text(Text::new(value)))
                        .collect(),
                ),
            ),
            ("x", WireValue::Float(self.x)),
            ("y", WireValue::Float(self.y)),
            ("z", WireValue::Float(self.z)),
        ])
    }
}

impl AssemblyStep {
    pub fn op(&self) -> &str {
        &self.op
    }
    pub fn component(&self) -> Option<&str> {
        self.component.as_deref()
    }
    pub fn components(&self) -> &[String] {
        &self.components
    }
    pub fn x(&self) -> f64 {
        self.x
    }
    pub fn y(&self) -> f64 {
        self.y
    }
    pub fn z(&self) -> f64 {
        self.z
    }
}

// ---------------------------------------------------------------------------
// ConstructionScript
// ---------------------------------------------------------------------------

/// Components and the assembly steps that join them.
#[derive(Clone, Debug, PartialEq)]
pub struct ConstructionScript {
    summary: Text,
    components: Vec<ComponentScript>,
    assembly: Vec<AssemblyStep>,
}

impl ContractModel for ConstructionScript {
    const NAME: &'static str = "ConstructionScript";

    fn from_json_str(text: &str) -> Result<Self, ContractError> {
        construction_from_json_str(text, ProfileScope::ConstructionScript)
    }

    fn from_json_bytes(bytes: &[u8]) -> Result<Self, ContractError> {
        construction_from_json_bytes(bytes, ProfileScope::ConstructionScript)
    }

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let summary = flatten_nested(map, "summary", &mut issues, |value| {
            nonempty_text(value, "summary")
        });
        let components = model_array_required::<ComponentScript>(map, "components", &mut issues);
        length_bounds(
            components.len(),
            1,
            MAX_COMPONENTS,
            &mut issues,
            "components",
        );
        let assembly = model_array::<AssemblyStep>(map, "assembly", &mut issues);
        length_bounds(
            assembly.len(),
            0,
            MAX_ASSEMBLY_STEPS,
            &mut issues,
            "assembly",
        );
        reject_extra(map, &["summary", "components", "assembly"], &mut issues);
        if issues.is_empty() {
            let mut names = Vec::with_capacity(components.len());
            for component in &components {
                if names.contains(&component.name()) {
                    push(&mut issues, "components", "component names must be unique");
                    break;
                }
                names.push(component.name());
            }
            for step in &assembly {
                for name in step
                    .component()
                    .into_iter()
                    .chain(step.components().iter().map(String::as_str))
                {
                    if !names.contains(&name) {
                        push(
                            &mut issues,
                            "assembly",
                            format!("assembly names a missing component {name}"),
                        );
                        break;
                    }
                }
            }
        }
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            summary: summary.expect("validated"),
            components,
            assembly,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("summary", WireValue::Text(self.summary.clone())),
            (
                "components",
                WireValue::Array(
                    self.components
                        .iter()
                        .map(ContractModel::to_wire_value)
                        .collect(),
                ),
            ),
            (
                "assembly",
                WireValue::Array(
                    self.assembly
                        .iter()
                        .map(ContractModel::to_wire_value)
                        .collect(),
                ),
            ),
        ])
    }
}

impl ConstructionScript {
    pub fn summary(&self) -> &Text {
        &self.summary
    }
    pub fn components(&self) -> &[ComponentScript] {
        &self.components
    }
    pub fn assembly(&self) -> &[AssemblyStep] {
        &self.assembly
    }
}

fn model_array<T: ContractModel>(
    map: &CanonicalObject,
    field: &str,
    issues: &mut Vec<ContractIssue>,
) -> Vec<T> {
    match map.get(&key(field)) {
        None => Vec::new(),
        Some(CanonicalValue::Array(values)) => {
            let mut parsed = Vec::with_capacity(values.len());
            for (index, value) in values.iter().enumerate() {
                match T::from_value(value) {
                    Ok(value) => parsed.push(value),
                    Err(error) => {
                        for issue in error.issues() {
                            issues.push(issue.clone().under(&format!("{field}[{index}]")));
                        }
                    }
                }
            }
            parsed
        }
        Some(_) => {
            push(issues, field, "Input should be a valid tuple");
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------
// Analysis
// ---------------------------------------------------------------------------

/// One analysis operation.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalysisStep {
    op: String,
    name: Option<Text>,
    face: Option<u32>,
    force_n: Option<f64>,
    pressure_mpa: Option<f64>,
}

impl ContractModel for AnalysisStep {
    const NAME: &'static str = "AnalysisStep";

    fn from_json_str(text: &str) -> Result<Self, ContractError> {
        construction_from_json_str(text, ProfileScope::None)
    }

    fn from_json_bytes(bytes: &[u8]) -> Result<Self, ContractError> {
        construction_from_json_bytes(bytes, ProfileScope::None)
    }

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let op = flatten_nested(map, "op", &mut issues, |value| {
            known_operation(value, &ANALYSIS_OPS)
        });
        let name = optional_value(map, "name", &mut issues, |value| {
            as_text(value).ok_or_else(|| "Input should be a valid string".to_owned())
        });
        let face = optional_value(map, "face", &mut issues, |value| match value {
            CanonicalValue::Integer(number) => {
                let face = num_traits::ToPrimitive::to_u32(number)
                    .ok_or_else(|| "face must be an integer from 1 to 64".to_owned())?;
                if (1..=MAX_FACE).contains(&face) {
                    Ok(face)
                } else {
                    Err("face must be an integer from 1 to 64".to_owned())
                }
            }
            _ => Err("face must be an integer from 1 to 64".to_owned()),
        });
        let force_n = optional_value(map, "force_n", &mut issues, |value| {
            optional_open_interval(value, "force_n", MAX_FORCE_N)
        })
        .flatten();
        let pressure_mpa = optional_value(map, "pressure_mpa", &mut issues, |value| {
            optional_open_interval(value, "pressure_mpa", MAX_PRESSURE_MPA)
        })
        .flatten();
        reject_extra(
            map,
            &["op", "name", "face", "force_n", "pressure_mpa"],
            &mut issues,
        );
        if issues.is_empty() {
            match op.as_deref() {
                Some("force") if force_n.is_none() => {
                    push(&mut issues, "force_n", "force requires force_n");
                }
                Some("pressure") if pressure_mpa.is_none() => {
                    push(
                        &mut issues,
                        "pressure_mpa",
                        "pressure requires pressure_mpa",
                    );
                }
                _ => {}
            }
        }
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            op: op.expect("validated"),
            name,
            face,
            force_n,
            pressure_mpa,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("op", WireValue::Text(Text::new(&self.op))),
            (
                "name",
                self.name
                    .as_ref()
                    .map_or(WireValue::Null, |value| WireValue::Text(value.clone())),
            ),
            (
                "face",
                self.face.map_or(WireValue::Null, |value| {
                    WireValue::Integer(BigInt::from(value))
                }),
            ),
            ("force_n", opt_f64(self.force_n)),
            ("pressure_mpa", opt_f64(self.pressure_mpa)),
        ])
    }
}

impl AnalysisStep {
    pub fn op(&self) -> &str {
        &self.op
    }
    pub fn name(&self) -> Option<&Text> {
        self.name.as_ref()
    }
    pub fn face(&self) -> Option<u32> {
        self.face
    }
    pub fn force_n(&self) -> Option<f64> {
        self.force_n
    }
    pub fn pressure_mpa(&self) -> Option<f64> {
        self.pressure_mpa
    }
}

/// The analysis operations, in the order a solver expects.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalysisScript {
    steps: Vec<AnalysisStep>,
}

impl ContractModel for AnalysisScript {
    const NAME: &'static str = "AnalysisScript";

    fn from_json_str(text: &str) -> Result<Self, ContractError> {
        construction_from_json_str(text, ProfileScope::None)
    }

    fn from_json_bytes(bytes: &[u8]) -> Result<Self, ContractError> {
        construction_from_json_bytes(bytes, ProfileScope::None)
    }

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let steps = model_array_required::<AnalysisStep>(map, "steps", &mut issues);
        length_bounds(steps.len(), 1, MAX_ANALYSIS_STEPS, &mut issues, "steps");
        reject_extra(map, &["steps"], &mut issues);
        if issues.is_empty() {
            if let Err(message) = check_analysis_order(&steps) {
                push(&mut issues, "steps", message);
            }
        }
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self { steps })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([(
            "steps",
            WireValue::Array(
                self.steps
                    .iter()
                    .map(ContractModel::to_wire_value)
                    .collect(),
            ),
        )])
    }
}

impl AnalysisScript {
    pub fn steps(&self) -> &[AnalysisStep] {
        &self.steps
    }
}

fn check_analysis_order(steps: &[AnalysisStep]) -> Result<(), &'static str> {
    fn rank(op: &str) -> Option<u8> {
        match op {
            "material" => Some(1),
            "fix" => Some(2),
            "force" | "pressure" => Some(3),
            "mesh" => Some(4),
            "solve" => Some(5),
            _ => None,
        }
    }
    let mut current = 0;
    let mut solved = false;
    for step in steps {
        let Some(step_rank) = rank(step.op()) else {
            return Err("unknown operation");
        };
        if step_rank < current {
            return Err("analysis steps are out of order");
        }
        current = step_rank;
        solved = solved || step.op() == "solve";
    }
    if !solved {
        return Err("an analysis script must include solve");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// EditStep / AnalysisReading
// ---------------------------------------------------------------------------

/// A small change applied with tool calls.
#[derive(Clone, Debug, PartialEq)]
pub struct EditStep {
    op: String,
    component: String,
    x: f64,
    y: f64,
    z: f64,
    length: Option<f64>,
    diameter: Option<f64>,
    radius: Option<f64>,
}

impl ContractModel for EditStep {
    const NAME: &'static str = "EditStep";

    fn from_json_str(text: &str) -> Result<Self, ContractError> {
        construction_from_json_str(text, ProfileScope::None)
    }

    fn from_json_bytes(bytes: &[u8]) -> Result<Self, ContractError> {
        construction_from_json_bytes(bytes, ProfileScope::None)
    }

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let op = flatten_nested(map, "op", &mut issues, |value| {
            known_operation(value, &EDIT_OPS)
        });
        let component = flatten_nested(map, "component", &mut issues, component_name_value);
        let x = default_value(
            map,
            "x",
            &mut issues,
            || 0.0,
            |value| coordinate(value, "x"),
        );
        let y = default_value(
            map,
            "y",
            &mut issues,
            || 0.0,
            |value| coordinate(value, "y"),
        );
        let z = default_value(
            map,
            "z",
            &mut issues,
            || 0.0,
            |value| coordinate(value, "z"),
        );
        let length = optional_value(map, "length", &mut issues, |value| {
            optional_size(value, "length")
        })
        .flatten();
        let diameter = optional_value(map, "diameter", &mut issues, |value| {
            optional_size(value, "diameter")
        })
        .flatten();
        let radius = optional_value(map, "radius", &mut issues, |value| {
            optional_size(value, "radius")
        })
        .flatten();
        reject_extra(
            map,
            &[
                "op",
                "component",
                "x",
                "y",
                "z",
                "length",
                "diameter",
                "radius",
            ],
            &mut issues,
        );
        if issues.is_empty() {
            match op.as_deref() {
                Some("resize") if length.is_none() && diameter.is_none() => {
                    push(
                        &mut issues,
                        "length",
                        "resize requires a length or a diameter",
                    );
                }
                Some("fillet") if radius.is_none() => {
                    push(&mut issues, "radius", "fillet requires a radius");
                }
                _ => {}
            }
        }
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            op: op.expect("validated"),
            component: component.expect("validated"),
            x,
            y,
            z,
            length,
            diameter,
            radius,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("op", WireValue::Text(Text::new(&self.op))),
            ("component", WireValue::Text(Text::new(&self.component))),
            ("x", WireValue::Float(self.x)),
            ("y", WireValue::Float(self.y)),
            ("z", WireValue::Float(self.z)),
            ("length", opt_f64(self.length)),
            ("diameter", opt_f64(self.diameter)),
            ("radius", opt_f64(self.radius)),
        ])
    }
}

impl EditStep {
    pub fn op(&self) -> &str {
        &self.op
    }
    pub fn component(&self) -> &str {
        &self.component
    }
    pub fn x(&self) -> f64 {
        self.x
    }
    pub fn y(&self) -> f64 {
        self.y
    }
    pub fn z(&self) -> f64 {
        self.z
    }
    pub fn length(&self) -> Option<f64> {
        self.length
    }
    pub fn diameter(&self) -> Option<f64> {
        self.diameter
    }
    pub fn radius(&self) -> Option<f64> {
        self.radius
    }
}

/// What the design model says about the numbers, and the change it proposes.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalysisReading {
    verdict: Text,
    tool_edit: Vec<EditStep>,
    rewrite: Option<ComponentScript>,
}

impl ContractModel for AnalysisReading {
    const NAME: &'static str = "AnalysisReading";

    fn from_json_str(text: &str) -> Result<Self, ContractError> {
        construction_from_json_str(text, ProfileScope::AnalysisReading)
    }

    fn from_json_bytes(bytes: &[u8]) -> Result<Self, ContractError> {
        construction_from_json_bytes(bytes, ProfileScope::AnalysisReading)
    }

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let verdict = flatten_nested(map, "verdict", &mut issues, |value| {
            bounded_text(value, "verdict", MAX_VERDICT_CHARS)
        });
        let tool_edit = model_array::<EditStep>(map, "tool_edit", &mut issues);
        length_bounds(tool_edit.len(), 0, MAX_EDITS, &mut issues, "tool_edit");
        let rewrite = optional_model::<ComponentScript>(map, "rewrite", &mut issues);
        reject_extra(map, &["verdict", "tool_edit", "rewrite"], &mut issues);
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            verdict: verdict.expect("validated"),
            tool_edit,
            rewrite,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("verdict", WireValue::Text(self.verdict.clone())),
            (
                "tool_edit",
                WireValue::Array(
                    self.tool_edit
                        .iter()
                        .map(ContractModel::to_wire_value)
                        .collect(),
                ),
            ),
            (
                "rewrite",
                self.rewrite
                    .as_ref()
                    .map_or(WireValue::Null, |value| value.to_wire_value()),
            ),
        ])
    }
}

impl AnalysisReading {
    pub fn verdict(&self) -> &Text {
        &self.verdict
    }
    pub fn tool_edit(&self) -> &[EditStep] {
        &self.tool_edit
    }
    pub fn rewrite(&self) -> Option<&ComponentScript> {
        self.rewrite.as_ref()
    }
}

fn optional_model<T: ContractModel>(
    map: &CanonicalObject,
    field: &str,
    issues: &mut Vec<ContractIssue>,
) -> Option<T> {
    match map.get(&key(field)) {
        None | Some(CanonicalValue::Null) => None,
        Some(value) => match T::from_value(value) {
            Ok(value) => Some(value),
            Err(error) => {
                for issue in error.issues() {
                    issues.push(issue.clone().under(field));
                }
                None
            }
        },
    }
}

fn opt_f64(value: Option<f64>) -> WireValue {
    value.map_or(WireValue::Null, WireValue::Float)
}

// ---------------------------------------------------------------------------
// Python loader entry points
// ---------------------------------------------------------------------------

/// Validate a construction object, preserving the Python loader error text.
///
/// A `CanonicalValue` has no object insertion metadata, so a nested profile
/// with multiple alias keys uses deterministic canonical order. JSON/HTTP
/// consumers should use [`load_construction_json_str`] or
/// [`load_construction_json_bytes`], which preserve input order.
pub fn load_construction(raw: &CanonicalValue) -> Result<ConstructionScript, ConstructionError> {
    load(raw, ConstructionScript::from_value)
}

/// Validate construction JSON text with Pydantic-compatible duplicate keys and
/// insertion-order profile aliases.
pub fn load_construction_json_str(text: &str) -> Result<ConstructionScript, ConstructionError> {
    load_json_str(
        text,
        ProfileScope::ConstructionScript,
        ConstructionScript::from_value,
    )
}

/// Validate construction UTF-8 JSON bytes with ordered profile aliases.
pub fn load_construction_json_bytes(bytes: &[u8]) -> Result<ConstructionScript, ConstructionError> {
    load_json_bytes(
        bytes,
        ProfileScope::ConstructionScript,
        ConstructionScript::from_value,
    )
}

/// Validate an analysis object, preserving the Python loader error text.
pub fn load_analysis(raw: &CanonicalValue) -> Result<AnalysisScript, ConstructionError> {
    load(raw, AnalysisScript::from_value)
}

/// Validate analysis JSON text.
pub fn load_analysis_json_str(text: &str) -> Result<AnalysisScript, ConstructionError> {
    load_json_str(text, ProfileScope::None, AnalysisScript::from_value)
}

/// Validate analysis UTF-8 JSON bytes.
pub fn load_analysis_json_bytes(bytes: &[u8]) -> Result<AnalysisScript, ConstructionError> {
    load_json_bytes(bytes, ProfileScope::None, AnalysisScript::from_value)
}

/// Validate a reading object, preserving the Python loader error text.
pub fn load_reading(raw: &CanonicalValue) -> Result<AnalysisReading, ConstructionError> {
    load(raw, AnalysisReading::from_value)
}

/// Validate reading JSON text, including ordered rewrite profiles.
pub fn load_reading_json_str(text: &str) -> Result<AnalysisReading, ConstructionError> {
    load_json_str(
        text,
        ProfileScope::AnalysisReading,
        AnalysisReading::from_value,
    )
}

/// Validate reading UTF-8 JSON bytes, including ordered rewrite profiles.
pub fn load_reading_json_bytes(bytes: &[u8]) -> Result<AnalysisReading, ConstructionError> {
    load_json_bytes(
        bytes,
        ProfileScope::AnalysisReading,
        AnalysisReading::from_value,
    )
}

fn load<T>(
    raw: &CanonicalValue,
    parse: impl Fn(&CanonicalValue) -> Result<T, ContractError>,
) -> Result<T, ConstructionError> {
    if !matches!(raw, CanonicalValue::Object(_)) {
        return Err(ConstructionError::new("the model did not return an object"));
    }
    parse(raw).map_err(|error| ConstructionError::new(error.construction_message()))
}

fn load_json_str<T>(
    text: &str,
    scope: ProfileScope,
    parse: impl Fn(&CanonicalValue) -> Result<T, ContractError>,
) -> Result<T, ConstructionError> {
    let mut input = crate::input::parse_json_str(text)
        .map_err(|error| ConstructionError::new(error.construction_message()))?;
    apply_profile_scope(input.root_mut(), scope);
    load_json_input(input, parse)
}

fn load_json_bytes<T>(
    bytes: &[u8],
    scope: ProfileScope,
    parse: impl Fn(&CanonicalValue) -> Result<T, ContractError>,
) -> Result<T, ConstructionError> {
    let mut input = crate::input::parse_json_bytes(bytes)
        .map_err(|error| ConstructionError::new(error.construction_message()))?;
    apply_profile_scope(input.root_mut(), scope);
    load_json_input(input, parse)
}

fn load_json_input<T>(
    input: crate::input::JsonInput,
    parse: impl Fn(&CanonicalValue) -> Result<T, ContractError>,
) -> Result<T, ConstructionError> {
    if !input.is_object() {
        return Err(ConstructionError::new("the model did not return an object"));
    }
    let value = input.into_canonical();
    parse(&value).map_err(|error| ConstructionError::new(error.construction_message()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_construction(source: &str) -> ConstructionScript {
        let value = kala_json::parse_strict(source.as_bytes(), &crate::model_json_limits())
            .expect("valid JSON");
        load_construction(&value).expect("valid construction")
    }

    #[test]
    fn nested_profiles_and_defaults_match_python() {
        let script = parse_construction(
            r#"{"summary":" hi ","components":[{"name":"Body","steps":[{"op":"sketch","profile":{"rectangle":{"width":10,"height":20}}},{"op":"extrude","length":5}]}],"assembly":[{"op":"place","component":"Body"}]}"#,
        );
        assert_eq!(script.summary().as_str(), Some("hi"));
        let sketch = &script.components()[0].steps()[0];
        assert_eq!(sketch.profile().and_then(Text::as_str), Some("rectangle"));
        assert_eq!(sketch.width(), Some(10.0));
        assert_eq!(sketch.x(), 0.0);
    }

    #[test]
    fn inference_and_error_messages_match_python() {
        let script = parse_construction(
            r#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","diameter":3},{"op":"extrude","length":2}]}]}"#,
        );
        assert_eq!(
            script.components()[0].steps()[0]
                .profile()
                .and_then(Text::as_str),
            Some("circle")
        );
        let value = kala_json::parse_strict(
            br#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"bogus"}]}]}"#,
            &crate::model_json_limits(),
        )
        .unwrap();
        assert_eq!(
            load_construction(&value).unwrap_err().message(),
            "unknown operation bogus"
        );
    }

    #[test]
    fn sequenced_component_rules_are_enforced() {
        let value = kala_json::parse_strict(
            br#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"pocket"}]}]}"#,
            &crate::model_json_limits(),
        )
        .unwrap();
        assert_eq!(
            load_construction(&value).unwrap_err().message(),
            "a pocket must follow a solid"
        );
    }
}
