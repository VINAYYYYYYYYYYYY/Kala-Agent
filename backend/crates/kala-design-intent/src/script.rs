//! Pydantic-compatible validation of the raw `ConstructionScript` field.
//!
//! The accepted `kala-contracts` crate owns the typed script and constructs it
//! on success, but its error objects target its own loader contract: model
//! validators are reported at the field they touch and tuple-length errors are
//! attached even when the tuple field was missing. `DesignIntent.script` is a
//! Pydantic field, so its product-visible error text follows Pydantic's
//! locations and error order instead. This module validates the JSON payload
//! with the same rules, then hands a finite payload to
//! [`kala_contracts::ConstructionScript`] for the typed value.

use kala_contracts::{ConstructionScript, ContractModel, Text};
use num_traits::ToPrimitive;

use crate::intent::{DesignIssue, DesignIssueLocation};
use crate::json::JsonValue;

const MAX_COMPONENTS: usize = kala_contracts::MAX_COMPONENTS;
const MAX_STEPS: usize = kala_contracts::MAX_STEPS;
const MAX_TOTAL_STEPS: usize = kala_contracts::MAX_TOTAL_STEPS;
const MAX_ASSEMBLY_STEPS: usize = 16;
const MAX_DIMENSION_MM: f64 = 5_000.0;
const MIN_DIMENSION_MM: f64 = 0.001;
const FULL_TURN: f64 = 360.0;
const COMPONENT_OPS: [&str; 5] = ["sketch", "extrude", "revolve", "pocket", "helix"];
const ASSEMBLY_OPS: [&str; 3] = ["place", "fuse", "compound"];
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
const STEP_FIELDS: [&str; 12] = [
    "op", "plane", "profile", "width", "height", "diameter", "length", "pitch", "angle", "x", "y",
    "reversed",
];

fn loc_field(prefix: &[DesignIssueLocation], name: &str) -> Vec<DesignIssueLocation> {
    let mut location = prefix.to_vec();
    location.push(DesignIssueLocation::Field(name.to_owned()));
    location
}

fn loc_index(prefix: &[DesignIssueLocation], index: usize) -> Vec<DesignIssueLocation> {
    let mut location = prefix.to_vec();
    location.push(DesignIssueLocation::Index(index));
    location
}

fn issue(prefix: &[DesignIssueLocation], message: impl Into<String>) -> DesignIssue {
    DesignIssue::from_parts(prefix.to_vec(), message)
}

fn field_issue(
    prefix: &[DesignIssueLocation],
    name: &str,
    message: impl Into<String>,
) -> DesignIssue {
    DesignIssue::from_parts(loc_field(prefix, name), message)
}

fn entry<'a>(entries: &'a [(Text, JsonValue)], name: &str) -> Option<&'a JsonValue> {
    entries
        .iter()
        .find(|(key, _)| key.as_str() == Some(name))
        .map(|(_, value)| value)
}

fn text_value(value: &JsonValue) -> Option<&Text> {
    value.as_text()
}

fn finite_number(value: &JsonValue, label: &str) -> Result<f64, String> {
    match value {
        JsonValue::Integer(number) => number
            .to_f64()
            .filter(|number| number.is_finite())
            .ok_or_else(|| format!("{label} must be a finite number")),
        JsonValue::Float(number) if number.is_finite() => Ok(*number),
        _ => Err(format!("{label} must be a finite number")),
    }
}

fn optional_size(value: &JsonValue, label: &str) -> Result<Option<f64>, String> {
    if matches!(value, JsonValue::Null) {
        return Ok(None);
    }
    let number = finite_number(value, label)?;
    if number < MIN_DIMENSION_MM || number > MAX_DIMENSION_MM {
        return Err(format!("{label} is outside the model bounds"));
    }
    Ok(Some(number))
}

fn coordinate(value: &JsonValue, label: &str) -> Result<f64, String> {
    let number = finite_number(value, label)?;
    if number.abs() > MAX_DIMENSION_MM {
        return Err(format!("{label} is outside the model bounds"));
    }
    Ok(number)
}

fn known_operation(value: &JsonValue, allowed: &[&str]) -> Result<String, String> {
    let Some(text) = text_value(value) else {
        return Err("unknown operation".to_owned());
    };
    let rendered = text.to_string_lossy();
    if allowed.contains(&rendered.as_str()) {
        return Ok(rendered);
    }
    if rendered.is_empty() {
        Err("unknown operation".to_owned())
    } else {
        Err(format!("unknown operation {rendered}"))
    }
}

fn component_name(text: &Text) -> Result<String, String> {
    let Some(value) = text.as_str() else {
        return Err("a component name must match ^[A-Za-z][A-Za-z0-9_-]{0,39}$".to_owned());
    };
    let bytes = value.as_bytes();
    let valid = !bytes.is_empty()
        && bytes.len() <= 40
        && bytes[0].is_ascii_alphabetic()
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    if valid {
        Ok(value.to_owned())
    } else {
        Err("a component name must match ^[A-Za-z][A-Za-z0-9_-]{0,39}$".to_owned())
    }
}

fn profile_value(value: &JsonValue) -> Result<String, String> {
    let Some(text) = text_value(value) else {
        return Err("profile must be rectangle or circle".to_owned());
    };
    let stripped = text.python_strip();
    if stripped.is_empty() {
        return Err("profile must be rectangle or circle".to_owned());
    }
    let lowered = stripped.casefold();
    let lowered = lowered.as_str().unwrap_or("");
    if let Some((_, alias)) = PROFILE_ALIASES.iter().find(|(name, _)| lowered == *name) {
        return Ok((*alias).to_owned());
    }
    Ok(lowered.to_owned())
}

fn is_profile(value: &str, expected: &str) -> bool {
    value == expected
}

fn truthy(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(value) => *value,
        JsonValue::Integer(value) => value != &num_bigint::BigInt::from(0),
        JsonValue::Float(value) => *value != 0.0,
        JsonValue::Text(value) => !value.is_empty(),
        JsonValue::Array(values) => !values.is_empty(),
        JsonValue::Object(entries) => !entries.is_empty(),
    }
}

/// Python `_flatten_profile`: lift a nested profile object onto the step.
fn flatten_profile(value: &JsonValue) -> JsonValue {
    let Some(entries) = value.entries() else {
        return value.clone();
    };
    let Some(profile) = entry(entries, "profile") else {
        return value.clone();
    };
    let Some(profile_entries) = profile.entries() else {
        return value.clone();
    };
    let mut flat = entries.to_vec();
    let mut shape: Option<String> = None;
    let mut body: Option<&[(Text, JsonValue)]> = None;
    for (key, item) in profile_entries {
        let folded = key.casefold();
        let folded = folded.as_str().unwrap_or("");
        if let Some((_, alias)) = PROFILE_ALIASES.iter().find(|(alias, _)| folded == *alias) {
            if item.entries().is_some() {
                shape = Some((*alias).to_owned());
                body = item.entries();
                break;
            }
        }
    }
    if shape.is_none() {
        let mut named = None;
        for key in ["type", "kind", "shape"] {
            if let Some(value) = entry(profile_entries, key) {
                if truthy(value) {
                    named = Some(value);
                    break;
                }
            }
        }
        if let Some(JsonValue::Text(named)) = named {
            let stripped = named.python_strip();
            if !stripped.is_empty() {
                let folded = stripped.casefold();
                let folded = folded.as_str().unwrap_or("");
                let alias = PROFILE_ALIASES
                    .iter()
                    .find(|(alias, _)| folded == *alias)
                    .map(|(_, canonical)| *canonical);
                shape = Some(alias.unwrap_or(folded).to_owned());
                body = Some(profile_entries);
            }
        }
    }
    if let Some(shape) = shape {
        if let Some((_, existing)) = flat
            .iter_mut()
            .find(|(key, _)| key.as_str() == Some("profile"))
        {
            *existing = JsonValue::Text(Text::new(&shape));
        } else {
            flat.push((Text::new("profile"), JsonValue::Text(Text::new(&shape))));
        }
    }
    if let Some(body) = body {
        for size in ["width", "height", "diameter", "length", "pitch"] {
            if let Some(value) = entry(body, size) {
                let target = flat
                    .iter()
                    .find(|(key, _)| key.as_str() == Some(size))
                    .map(|(_, value)| value);
                if target.is_none() || matches!(target, Some(JsonValue::Null)) {
                    if let Some((_, existing)) =
                        flat.iter_mut().find(|(key, _)| key.as_str() == Some(size))
                    {
                        *existing = value.clone();
                    } else {
                        flat.push((Text::new(size), value.clone()));
                    }
                }
            }
        }
    }
    for coordinate_name in ["x", "y"] {
        if let Some(value) = entry(profile_entries, coordinate_name) {
            if !flat
                .iter()
                .any(|(key, _)| key.as_str() == Some(coordinate_name))
            {
                flat.push((Text::new(coordinate_name), value.clone()));
            }
        }
    }
    JsonValue::Object(flat)
}

#[derive(Clone, Debug)]
struct StepData {
    op: String,
    plane: String,
}

fn validate_step(
    value: &JsonValue,
    location: &[DesignIssueLocation],
) -> Result<StepData, Vec<DesignIssue>> {
    let flattened = flatten_profile(value);
    let Some(entries) = flattened.entries() else {
        return Err(vec![issue(
            location,
            "Input should be a valid dictionary or instance of ComponentStep",
        )]);
    };
    let mut issues = Vec::new();
    let mut op: Option<String> = None;
    let mut plane: Option<String> = None;
    let mut profile: Option<String> = None;
    let mut width = None;
    let mut height = None;
    let mut diameter = None;
    let mut length = None;
    let mut pitch = None;
    let mut angle = None;
    let mut x = Some(0.0);
    let mut y = Some(0.0);
    let mut reversed = Some(false);

    match entry(entries, "op") {
        None => issues.push(field_issue(location, "op", "Field required")),
        Some(value) => match known_operation(value, &COMPONENT_OPS) {
            Ok(value) => op = Some(value),
            Err(message) => issues.push(field_issue(location, "op", message)),
        },
    }

    match entry(entries, "plane") {
        None => {}
        Some(value) => match text_value(value) {
            None => issues.push(field_issue(
                location,
                "plane",
                "Input should be a valid string",
            )),
            Some(text) => match text.as_str() {
                Some(value) if PLANES.contains(&value) => plane = Some(value.to_owned()),
                _ => issues.push(field_issue(
                    location,
                    "plane",
                    "plane must be XY, XZ, or YZ",
                )),
            },
        },
    }

    match entry(entries, "profile") {
        None | Some(JsonValue::Null) => {}
        Some(value) => match profile_value(value) {
            Ok(value) => profile = Some(value),
            Err(message) => issues.push(field_issue(location, "profile", message)),
        },
    }

    for (name, slot) in [
        ("width", &mut width),
        ("height", &mut height),
        ("diameter", &mut diameter),
        ("length", &mut length),
        ("pitch", &mut pitch),
        ("angle", &mut angle),
    ] {
        if let Some(value) = entry(entries, name) {
            match optional_size(value, name) {
                Ok(value) => *slot = value,
                Err(message) => issues.push(field_issue(location, name, message)),
            }
        }
    }

    for (name, slot) in [("x", &mut x), ("y", &mut y)] {
        match entry(entries, name) {
            None => {}
            Some(value) => match coordinate(value, name) {
                Ok(value) => *slot = Some(value),
                Err(message) => issues.push(field_issue(location, name, message)),
            },
        }
    }

    match entry(entries, "reversed") {
        None => {}
        Some(JsonValue::Bool(value)) => reversed = Some(*value),
        Some(_) => issues.push(field_issue(
            location,
            "reversed",
            "Input should be a valid boolean",
        )),
    }

    for (key, _) in entries {
        let name = key.to_string_lossy();
        if !STEP_FIELDS.contains(&name.as_str()) {
            issues.push(field_issue(
                location,
                &name,
                "Extra inputs are not permitted",
            ));
        }
    }

    let Some(op_value) = op.as_deref() else {
        return Err(issues);
    };
    if !issues.is_empty() {
        return Err(issues);
    }
    match op_value {
        "sketch" => {
            if !profile
                .as_deref()
                .is_some_and(|value| is_profile(value, "rectangle") || is_profile(value, "circle"))
            {
                if diameter.is_some() && (width.is_none() || height.is_none()) {
                    profile = Some("circle".to_owned());
                } else if width.is_some() && height.is_some() {
                    profile = Some("rectangle".to_owned());
                }
            }
            match profile.as_deref() {
                Some("rectangle") => {
                    if width.is_none() || height.is_none() {
                        issues.push(issue(
                            location,
                            "a rectangle sketch requires width and height",
                        ));
                    }
                }
                Some("circle") => {
                    if diameter.is_none() {
                        issues.push(issue(location, "a circle sketch requires a diameter"));
                    }
                }
                _ => issues.push(issue(location, "profile must be rectangle or circle")),
            }
        }
        "extrude" => {
            if length.is_none() {
                issues.push(issue(location, "extrude requires a length"));
            }
        }
        "revolve" => {
            if let Some(value) = angle {
                if value > FULL_TURN {
                    issues.push(issue(location, "revolve angle must be in (0, 360]"));
                }
            }
        }
        "helix" => {
            if diameter.is_none() {
                issues.push(issue(location, "helix requires a diameter"));
            } else if pitch.is_none() {
                issues.push(issue(location, "helix requires a pitch"));
            } else if length.is_none() {
                issues.push(issue(location, "helix requires a length"));
            }
        }
        _ => {}
    }
    if issues.is_empty() && reversed == Some(true) && op_value != "extrude" {
        issues.push(issue(location, "reversed is only meaningful on extrude"));
    }
    if issues.is_empty() {
        Ok(StepData {
            op: op_value.to_owned(),
            plane: plane.unwrap_or_else(|| "XY".to_owned()),
        })
    } else {
        Err(issues)
    }
}

fn tuple_bounds(
    issues: &mut Vec<DesignIssue>,
    location: &[DesignIssueLocation],
    count: usize,
    minimum: usize,
    maximum: usize,
) {
    if count < minimum {
        issues.push(issue(
            location,
            format!(
                "Tuple should have at least {minimum} item{} after validation, not {count}",
                if minimum == 1 { "" } else { "s" }
            ),
        ));
    }
    if count > maximum {
        issues.push(issue(
            location,
            format!("Tuple should have at most {maximum} items after validation, not {count}"),
        ));
    }
}

fn sequence_error(steps: &[StepData]) -> Option<&'static str> {
    let mut pending = false;
    let mut solid = false;
    for step in steps {
        match step.op.as_str() {
            "sketch" => {
                if pending {
                    return Some("a sketch must be consumed before another sketch");
                }
                pending = true;
            }
            "helix" => {
                if !solid {
                    return Some("a helix must follow a solid");
                }
                if pending {
                    return Some("a sketch must be consumed before a helix");
                }
            }
            "pocket" => {
                if !solid {
                    return Some("a pocket must follow a solid");
                }
                if !pending {
                    return Some("a pocket must follow a sketch");
                }
                pending = false;
            }
            "extrude" => {
                if !pending {
                    return Some("extrude must follow a sketch");
                }
                pending = false;
                solid = true;
            }
            "revolve" => {
                if !pending {
                    return Some("revolve must follow a sketch");
                }
                pending = false;
                solid = true;
            }
            _ => {}
        }
    }
    if !solid {
        return Some("a component must produce a solid");
    }
    if pending {
        return Some("a sketch must be consumed");
    }
    None
}

/// The first pocket whose sketch plane cannot cut the solid it follows.
///
/// Mirrors `kala_agent.studio.design.intent._check_pocket_planes`: a pending
/// sketch whose plane differs from the sketch that produced the current
/// solid is refused with the OCCT compiler's wording. The pocket's own
/// `plane` field is not checked, because the compiler never reads it and the
/// shipped L-bracket template's canonical wire form states `XY` on its XZ
/// upright pocket.
fn pocket_plane_error(components: &[ComponentData]) -> Option<String> {
    for component in components {
        let mut pending: Option<&StepData> = None;
        let mut solid_plane: Option<&str> = None;
        for (index, step) in component.steps.iter().enumerate() {
            match step.op.as_str() {
                "sketch" => pending = Some(step),
                "extrude" | "revolve" => {
                    if let Some(sketch) = pending {
                        solid_plane = Some(sketch.plane.as_str());
                    }
                    pending = None;
                }
                "pocket" => {
                    let (Some(sketch), Some(solid)) = (pending, solid_plane) else {
                        continue;
                    };
                    if sketch.plane != solid {
                        return Some(format!(
                            "component '{}': pocket at step index {index} on {} over a {solid} solid",
                            component.name, sketch.plane
                        ));
                    }
                    pending = None;
                }
                _ => {}
            }
        }
    }
    None
}

#[derive(Clone, Debug)]
struct ComponentData {
    name: String,
    steps: Vec<StepData>,
}

fn validate_component(
    value: &JsonValue,
    location: &[DesignIssueLocation],
) -> Result<ComponentData, Vec<DesignIssue>> {
    let Some(entries) = value.entries() else {
        return Err(vec![issue(
            location,
            "Input should be a valid dictionary or instance of ComponentScript",
        )]);
    };
    let mut issues = Vec::new();
    let mut name = None;
    match entry(entries, "name") {
        None => issues.push(field_issue(location, "name", "Field required")),
        Some(value) => match text_value(value) {
            None => issues.push(field_issue(
                location,
                "name",
                "Input should be a valid string",
            )),
            Some(text) => match component_name(text) {
                Ok(value) => name = Some(value),
                Err(message) => issues.push(field_issue(location, "name", message)),
            },
        },
    }

    let mut steps = Vec::new();
    let steps_location = loc_field(location, "steps");
    match entry(entries, "steps") {
        None => issues.push(field_issue(location, "steps", "Field required")),
        Some(JsonValue::Array(values)) => {
            let mut valid = Vec::new();
            for (index, item) in values.iter().enumerate() {
                let item_location = loc_index(&steps_location, index);
                if item.entries().is_none() {
                    issues.push(issue(
                        &item_location,
                        "Input should be a valid dictionary or instance of ComponentStep",
                    ));
                    continue;
                }
                match validate_step(item, &item_location) {
                    Ok(data) => valid.push(data),
                    Err(item_issues) => issues.extend(item_issues),
                }
            }
            steps = valid;
            tuple_bounds(&mut issues, &steps_location, steps.len(), 1, MAX_STEPS);
        }
        Some(_) => issues.push(issue(&steps_location, "Input should be a valid tuple")),
    }

    for (key, _) in entries {
        let field = key.to_string_lossy();
        if !matches!(field.as_str(), "name" | "steps") {
            issues.push(field_issue(
                location,
                &field,
                "Extra inputs are not permitted",
            ));
        }
    }

    if !issues.is_empty() {
        return Err(issues);
    }

    if let Some(message) = sequence_error(&steps) {
        issues.push(issue(location, message));
    }
    if issues.is_empty() {
        Ok(ComponentData {
            name: name.expect("validated"),
            steps,
        })
    } else {
        Err(issues)
    }
}

#[derive(Clone, Debug)]
struct AssemblyData {
    component: Option<String>,
    components: Vec<String>,
}

fn validate_assembly(
    value: &JsonValue,
    location: &[DesignIssueLocation],
) -> Result<AssemblyData, Vec<DesignIssue>> {
    let Some(entries) = value.entries() else {
        return Err(vec![issue(
            location,
            "Input should be a valid dictionary or instance of AssemblyStep",
        )]);
    };
    let mut issues = Vec::new();
    let mut op = None;
    let mut component = None;
    let mut components = Vec::new();
    let mut x = 0.0;
    let mut y = 0.0;
    let mut z = 0.0;

    match entry(entries, "op") {
        None => issues.push(field_issue(location, "op", "Field required")),
        Some(value) => match known_operation(value, &ASSEMBLY_OPS) {
            Ok(value) => op = Some(value),
            Err(message) => issues.push(field_issue(location, "op", message)),
        },
    }

    match entry(entries, "component") {
        None | Some(JsonValue::Null) => {}
        Some(value) => match text_value(value) {
            None => issues.push(field_issue(
                location,
                "component",
                "Input should be a valid string",
            )),
            Some(text) => match component_name(text) {
                Ok(value) => component = Some(value),
                Err(message) => issues.push(field_issue(location, "component", message)),
            },
        },
    }

    match entry(entries, "components") {
        None => {}
        Some(JsonValue::Array(values)) => {
            for (index, item) in values.iter().enumerate() {
                let item_location = loc_index(&loc_field(location, "components"), index);
                match text_value(item) {
                    None => issues.push(issue(&item_location, "Input should be a valid string")),
                    Some(text) => match component_name(text) {
                        Ok(value) => components.push(value),
                        Err(message) => issues.push(issue(&item_location, message)),
                    },
                }
            }
        }
        Some(_) => issues.push(field_issue(
            location,
            "components",
            "Input should be a valid tuple",
        )),
    }

    for (name, slot) in [("x", &mut x), ("y", &mut y), ("z", &mut z)] {
        match entry(entries, name) {
            None => {}
            Some(value) => match coordinate(value, name) {
                Ok(value) => *slot = value,
                Err(message) => issues.push(field_issue(location, name, message)),
            },
        }
    }

    for (key, _) in entries {
        let field = key.to_string_lossy();
        if !matches!(
            field.as_str(),
            "op" | "component" | "components" | "x" | "y" | "z"
        ) {
            issues.push(field_issue(
                location,
                &field,
                "Extra inputs are not permitted",
            ));
        }
    }

    let _ = (x, y, z);
    let Some(op_value) = op.as_deref() else {
        return Err(issues);
    };
    if !issues.is_empty() {
        return Err(issues);
    }
    if op_value == "place" {
        if component.is_none() {
            issues.push(issue(location, "place requires a component"));
        }
    } else if components.len() < 2 {
        issues.push(issue(
            location,
            format!("{op_value} requires at least two components"),
        ));
    }
    if issues.is_empty() {
        Ok(AssemblyData {
            component,
            components,
        })
    } else {
        Err(issues)
    }
}

/// Validate the `script` field and construct the typed script on success.
pub(crate) fn validate_script(
    value: &JsonValue,
    prefix: &[DesignIssueLocation],
) -> (Option<ConstructionScript>, Vec<DesignIssue>) {
    let Some(entries) = value.entries() else {
        return (
            None,
            vec![issue(
                prefix,
                "Input should be a valid dictionary or instance of ConstructionScript",
            )],
        );
    };
    let mut issues = Vec::new();
    match entry(entries, "summary") {
        None => issues.push(field_issue(prefix, "summary", "Field required")),
        Some(value) => match text_value(value) {
            Some(text) => {
                let stripped = text.python_strip();
                if stripped.is_empty() {
                    issues.push(field_issue(
                        prefix,
                        "summary",
                        "summary must be nonempty text",
                    ));
                }
            }
            None => issues.push(field_issue(
                prefix,
                "summary",
                "summary must be nonempty text",
            )),
        },
    }

    let mut components = Vec::new();
    let components_location = loc_field(prefix, "components");
    match entry(entries, "components") {
        None => issues.push(field_issue(prefix, "components", "Field required")),
        Some(JsonValue::Array(values)) => {
            let mut valid = Vec::new();
            for (index, item) in values.iter().enumerate() {
                let item_location = loc_index(&components_location, index);
                if item.entries().is_none() {
                    issues.push(issue(
                        &item_location,
                        "Input should be a valid dictionary or instance of ComponentScript",
                    ));
                    continue;
                }
                match validate_component(item, &item_location) {
                    Ok(data) => valid.push(data),
                    Err(item_issues) => issues.extend(item_issues),
                }
            }
            components = valid;
            tuple_bounds(
                &mut issues,
                &components_location,
                components.len(),
                1,
                MAX_COMPONENTS,
            );
        }
        Some(_) => issues.push(issue(&components_location, "Input should be a valid tuple")),
    }

    let mut assembly = Vec::new();
    let assembly_location = loc_field(prefix, "assembly");
    match entry(entries, "assembly") {
        None => {}
        Some(JsonValue::Array(values)) => {
            let mut valid = Vec::new();
            for (index, item) in values.iter().enumerate() {
                let item_location = loc_index(&assembly_location, index);
                if item.entries().is_none() {
                    issues.push(issue(
                        &item_location,
                        "Input should be a valid dictionary or instance of AssemblyStep",
                    ));
                    continue;
                }
                match validate_assembly(item, &item_location) {
                    Ok(data) => valid.push(data),
                    Err(item_issues) => issues.extend(item_issues),
                }
            }
            assembly = valid;
            if assembly.len() > MAX_ASSEMBLY_STEPS {
                issues.push(issue(
                    &assembly_location,
                    format!(
                        "Tuple should have at most {MAX_ASSEMBLY_STEPS} items after validation, not {}",
                        assembly.len()
                    ),
                ));
            }
        }
        Some(_) => issues.push(issue(&assembly_location, "Input should be a valid tuple")),
    }

    for (key, _) in entries {
        let field = key.to_string_lossy();
        if !matches!(field.as_str(), "summary" | "components" | "assembly") {
            issues.push(field_issue(
                prefix,
                &field,
                "Extra inputs are not permitted",
            ));
        }
    }

    if !issues.is_empty() {
        return (None, issues);
    }

    let names: Vec<&str> = components
        .iter()
        .map(|component| component.name.as_str())
        .collect();
    let unique: std::collections::BTreeSet<&str> = names.iter().copied().collect();
    if unique.len() != names.len() {
        return (None, vec![issue(prefix, "component names must be unique")]);
    }
    for step in &assembly {
        let mut referenced: Vec<&str> = Vec::new();
        if let Some(name) = step.component.as_deref() {
            referenced.push(name);
        }
        referenced.extend(step.components.iter().map(String::as_str));
        for name in referenced {
            if !unique.contains(name) {
                return (
                    None,
                    vec![issue(
                        prefix,
                        format!("assembly names a missing component {name}"),
                    )],
                );
            }
        }
    }

    let total = components
        .iter()
        .map(|component| component.steps.len())
        .sum::<usize>()
        + assembly.len();
    if total > MAX_TOTAL_STEPS {
        return (
            None,
            vec![issue(
                prefix,
                format!(
                    "a script may use at most {MAX_TOTAL_STEPS} steps in total; this one uses {total}"
                ),
            )],
        );
    }

    if let Some(message) = pocket_plane_error(&components) {
        return (None, vec![issue(prefix, message)]);
    }

    let Some(canonical) = value.to_canonical() else {
        return (
            None,
            vec![issue(
                prefix,
                "Input should be a valid dictionary or instance of ConstructionScript",
            )],
        );
    };
    match ConstructionScript::from_value(&canonical) {
        Ok(script) => (Some(script), Vec::new()),
        Err(error) => (
            None,
            vec![issue(
                prefix,
                format!("the construction contract refused a validated script: {error}"),
            )],
        ),
    }
}
