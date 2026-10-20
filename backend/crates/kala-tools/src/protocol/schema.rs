//! JSON schema validation.
//!
//! This is the port of the Python module's schema layer: the keyword
//! whitelist, the type, `const`, `enum`, `allOf`, `anyOf` and `oneOf`
//! keywords, and the object, array, string and number validators. Every
//! message and every error precedence is preserved, because a tool schema is
//! part of the security boundary: a schema that is accepted here must mean
//! the same thing in the Python runtime.
//!
//! The admitted keyword set is deliberately small. A keyword outside the
//! supported or annotation set is refused rather than ignored, so a schema
//! can never quietly mean less than it appears to.

use kala_json::{CanonicalObject, CanonicalValue, JsonText};

use crate::protocol::MAX_COLLECTION_ITEMS;
use crate::protocol::MAX_SCHEMA_DEPTH;
use crate::protocol::error::ToolProtocolError;
use crate::protocol::json::canonical_json_dumps;
use crate::protocol::pattern::bounded_pattern;
use crate::protocol::text::{python_repr, python_repr_text};

/// The keywords a schema may declare.
const SUPPORTED_KEYWORDS: [&str; 23] = [
    "additionalProperties",
    "allOf",
    "anyOf",
    "const",
    "enum",
    "exclusiveMaximum",
    "exclusiveMinimum",
    "items",
    "maxItems",
    "maxLength",
    "maxProperties",
    "maximum",
    "minItems",
    "minLength",
    "minProperties",
    "minimum",
    "multipleOf",
    "oneOf",
    "pattern",
    "properties",
    "required",
    "type",
    "uniqueItems",
];

/// The annotation keywords a schema may declare.
const ANNOTATION_KEYWORDS: [&str; 12] = [
    "$comment",
    "$id",
    "$schema",
    "default",
    "deprecated",
    "description",
    "examples",
    "format",
    "readOnly",
    "title",
    "unit",
    "writeOnly",
];

/// The declared JSON type of a value.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DeclaredType {
    Array,
    Boolean,
    Integer,
    Null,
    Number,
    Object,
    String,
}

impl DeclaredType {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "array" => Some(Self::Array),
            "boolean" => Some(Self::Boolean),
            "integer" => Some(Self::Integer),
            "null" => Some(Self::Null),
            "number" => Some(Self::Number),
            "object" => Some(Self::Object),
            "string" => Some(Self::String),
            _ => None,
        }
    }

    fn accepts(self, value: &CanonicalValue) -> bool {
        match self {
            Self::Array => matches!(value, CanonicalValue::Array(_)),
            Self::Boolean => matches!(value, CanonicalValue::Bool(_)),
            Self::Integer => matches!(value, CanonicalValue::Integer(_)),
            Self::Null => matches!(value, CanonicalValue::Null),
            Self::Number => {
                matches!(value, CanonicalValue::Integer(_) | CanonicalValue::Float(_))
            }
            Self::Object => matches!(value, CanonicalValue::Object(_)),
            Self::String => matches!(value, CanonicalValue::Text(_)),
        }
    }
}

/// A numeric bound taken from a schema.
enum NumberBound {
    Integer(num_bigint::BigInt),
    Float(f64),
}

/// Validate a value against a JSON schema.
///
/// `path` is the JSON path reported in a [`ToolProtocolError::Schema`]
/// failure and `depth` is the schema nesting depth, exactly like the Python
/// keyword arguments.
pub fn validate_against_schema(
    value: &CanonicalValue,
    schema: &CanonicalValue,
    path: &str,
    depth: usize,
) -> Result<(), ToolProtocolError> {
    let CanonicalValue::Object(schema) = schema else {
        return Err(ToolProtocolError::schema("schema must be an object", path));
    };
    if depth > MAX_SCHEMA_DEPTH {
        return Err(ToolProtocolError::schema(
            "schema nesting exceeds the supported depth",
            path,
        ));
    }
    schema_keywords(schema, path)?;
    let declared = get_present(schema, "type");
    if let Some(declared) = declared {
        let names = declared_type_names(declared);
        let mut types = Vec::with_capacity(names.len());
        for name in &names {
            let Some(text) = name.text() else {
                return Err(ToolProtocolError::schema(
                    format!("unsupported schema type: {}", name.repr()),
                    path,
                ));
            };
            let utf8 = text.to_string_lossy();
            let Some(kind) = DeclaredType::parse(&utf8) else {
                return Err(ToolProtocolError::schema(
                    format!("unsupported schema type: {}", name.repr()),
                    path,
                ));
            };
            types.push(kind);
        }
        if !types.iter().any(|kind| kind.accepts(value)) {
            let joined = names
                .iter()
                .map(|name| match name.text() {
                    Some(text) => text.to_string_lossy(),
                    None => name.repr(),
                })
                .collect::<Vec<_>>()
                .join("|");
            return Err(ToolProtocolError::schema(
                format!("value does not match the declared type {joined}"),
                path,
            ));
        }
    }
    if let Some(expected) = schema.get(&JsonText::new("const")) {
        if !json_equal(value, expected) {
            return Err(ToolProtocolError::schema(
                "value does not match the declared const",
                path,
            ));
        }
    }
    if let Some(options) = schema.get(&JsonText::new("enum")) {
        let CanonicalValue::Array(options) = options else {
            return Err(ToolProtocolError::schema(
                "enum must be a nonempty array",
                path,
            ));
        };
        if options.is_empty() {
            return Err(ToolProtocolError::schema(
                "enum must be a nonempty array",
                path,
            ));
        }
        if !options.iter().any(|option| json_equal(value, option)) {
            return Err(ToolProtocolError::schema(
                "value is not a permitted enum member",
                path,
            ));
        }
    }
    if let Some(branches) = get_present(schema, "allOf") {
        let CanonicalValue::Array(branches) = branches else {
            return Err(ToolProtocolError::schema(
                "allOf must be a nonempty array",
                path,
            ));
        };
        if branches.is_empty() {
            return Err(ToolProtocolError::schema(
                "allOf must be a nonempty array",
                path,
            ));
        }
        for branch in branches {
            if !schema_branch(value, branch, path, depth)? {
                return Err(ToolProtocolError::schema(
                    "value does not match a required subschema",
                    path,
                ));
            }
        }
    }
    for keyword in ["anyOf", "oneOf"] {
        let Some(branches) = get_present(schema, keyword) else {
            continue;
        };
        let CanonicalValue::Array(branches) = branches else {
            return Err(ToolProtocolError::schema(
                format!("{keyword} must be a nonempty array"),
                path,
            ));
        };
        if branches.is_empty() {
            return Err(ToolProtocolError::schema(
                format!("{keyword} must be a nonempty array"),
                path,
            ));
        }
        let mut matched = 0usize;
        for branch in branches {
            if schema_branch(value, branch, path, depth)? {
                matched += 1;
            }
        }
        if matched == 0 {
            return Err(ToolProtocolError::schema(
                format!("value does not match any {keyword} subschema"),
                path,
            ));
        }
        if keyword == "oneOf" && matched > 1 {
            return Err(ToolProtocolError::schema(
                "value matches more than one oneOf subschema",
                path,
            ));
        }
    }
    match value {
        CanonicalValue::Object(entries) => validate_object(entries, schema, path),
        CanonicalValue::Array(items) => validate_array(items, schema, path),
        CanonicalValue::Text(text) => validate_string(text, schema, path),
        CanonicalValue::Integer(number) => validate_number(bigint_to_f64(number), schema, path),
        CanonicalValue::Float(number) => validate_number(*number, schema, path),
        CanonicalValue::Bool(_) | CanonicalValue::Null => Ok(()),
    }
}

/// Check that a schema is a closed object schema.
///
/// This is the admission check a tool registry runs before a schema is
/// accepted: the schema must declare an object type, forbid additional
/// properties and declare a nonempty properties map whose members are
/// themselves valid.
pub fn check_object_schema(schema: &CanonicalValue, label: &str) -> Result<(), ToolProtocolError> {
    let CanonicalValue::Object(schema) = schema else {
        return Err(ToolProtocolError::schema(
            format!("{label} must be a JSON object"),
            "$",
        ));
    };
    let declared = schema.get(&JsonText::new("type"));
    let is_object =
        matches!(declared, Some(CanonicalValue::Text(text)) if text.to_string_lossy() == "object");
    if !is_object {
        return Err(ToolProtocolError::schema(
            format!("{label} must declare an object type"),
            "$",
        ));
    }
    let forbids_extras = matches!(
        schema.get(&JsonText::new("additionalProperties")),
        Some(CanonicalValue::Bool(false))
    );
    if !forbids_extras {
        return Err(ToolProtocolError::schema(
            format!("{label} must forbid additional properties"),
            "$",
        ));
    }
    let properties = schema.get(&JsonText::new("properties"));
    let Some(CanonicalValue::Object(properties)) = properties else {
        return Err(ToolProtocolError::schema(
            format!("{label} must declare a nonempty properties map"),
            "$",
        ));
    };
    if properties.is_empty() {
        return Err(ToolProtocolError::schema(
            format!("{label} must declare a nonempty properties map"),
            "$",
        ));
    }
    for (key, subschema) in properties {
        let CanonicalValue::Object(subschema) = subschema else {
            return Err(ToolProtocolError::schema(
                format!("{label} properties must map names to schemas"),
                "$",
            ));
        };
        let path = format!("{label}.{}", key.to_string_lossy());
        schema_keywords(subschema, &path)?;
        pattern_rejects(subschema, &path)?;
    }
    let required = schema.get(&JsonText::new("required"));
    if let Some(required) = required {
        let CanonicalValue::Array(required) = required else {
            return Err(ToolProtocolError::schema(
                format!("{label} required entries must be an array"),
                "$",
            ));
        };
        for name in required {
            let Some(text) = name.as_text() else {
                return Err(ToolProtocolError::schema(
                    format!("{label} requires an undeclared property"),
                    "$",
                ));
            };
            let key = JsonText::new(&text.to_string_lossy());
            if !properties.contains_key(&key) {
                return Err(ToolProtocolError::schema(
                    format!("{label} requires an undeclared property"),
                    "$",
                ));
            }
        }
    }
    schema_keywords(schema, label)?;
    pattern_rejects(schema, label)?;
    Ok(())
}

/// Refuse a schema keyword outside the supported and annotation sets.
fn schema_keywords(schema: &CanonicalObject, path: &str) -> Result<(), ToolProtocolError> {
    for keyword in schema.keys() {
        let name = keyword.to_string_lossy();
        if SUPPORTED_KEYWORDS.contains(&name.as_str())
            || ANNOTATION_KEYWORDS.contains(&name.as_str())
        {
            continue;
        }
        return Err(ToolProtocolError::schema(
            format!("unsupported schema keyword: {name}"),
            path,
        ));
    }
    Ok(())
}

/// A declared type name.
///
/// The Python implementation reads the `type` keyword with `dict.get`, which
/// yields `None` for a missing key *and* for a `null` value, and treats a
/// sequence of names as a tuple of members.
enum DeclaredName<'a> {
    Value(&'a CanonicalValue),
    Key(&'a JsonText),
}

impl<'a> DeclaredName<'a> {
    fn text(&self) -> Option<&'a JsonText> {
        match self {
            Self::Value(value) => value.as_text(),
            Self::Key(key) => Some(key),
        }
    }

    fn repr(&self) -> String {
        match self {
            Self::Value(value) => python_repr(value),
            Self::Key(key) => python_repr_text(key),
        }
    }
}

/// The declared type names of a `type` keyword.
fn declared_type_names(declared: &CanonicalValue) -> Vec<DeclaredName<'_>> {
    match declared {
        CanonicalValue::Array(items) => items.iter().map(DeclaredName::Value).collect(),
        CanonicalValue::Object(entries) => entries.keys().map(DeclaredName::Key).collect(),
        other => vec![DeclaredName::Value(other)],
    }
}

/// Read a schema member the way the Python implementation reads it.
///
/// `dict.get` returns `None` for a missing key and for a `null` value, and the
/// Python module uses that reading for the `type`, `allOf`, `anyOf`, `oneOf`,
/// `items` and `pattern` keywords.
fn get_present<'a>(schema: &'a CanonicalObject, key: &str) -> Option<&'a CanonicalValue> {
    match schema.get(&JsonText::new(key)) {
        Some(CanonicalValue::Null) => None,
        other => other,
    }
}

/// Validate a value against one branch of `allOf`, `anyOf` or `oneOf`.
///
/// A branch that fails is reported as "not matched" rather than as an error;
/// only a branch that is not an object is an error.
fn schema_branch(
    value: &CanonicalValue,
    branch: &CanonicalValue,
    path: &str,
    depth: usize,
) -> Result<bool, ToolProtocolError> {
    if !matches!(branch, CanonicalValue::Object(_)) {
        return Err(ToolProtocolError::schema(
            "schema branches must be objects",
            path,
        ));
    }
    Ok(validate_against_schema(value, branch, path, depth + 1).is_ok())
}

/// Validate an object value.
fn validate_object(
    value: &CanonicalObject,
    schema: &CanonicalObject,
    path: &str,
) -> Result<(), ToolProtocolError> {
    let properties = schema.get(&JsonText::new("properties"));
    let properties = match properties {
        None => &CanonicalObject::new(),
        Some(CanonicalValue::Object(properties)) => properties,
        Some(_) => {
            return Err(ToolProtocolError::schema(
                "properties must be an object",
                path,
            ));
        }
    };
    for subschema in properties.values() {
        if !matches!(subschema, CanonicalValue::Object(_)) {
            return Err(ToolProtocolError::schema(
                "properties must map names to schemas",
                path,
            ));
        }
    }
    let required: &[CanonicalValue] = match schema.get(&JsonText::new("required")) {
        None => &[],
        Some(CanonicalValue::Array(items)) => items.as_slice(),
        Some(_) => {
            return Err(ToolProtocolError::schema("required must be an array", path));
        }
    };
    let mut normalized_required: Vec<&CanonicalValue> = Vec::with_capacity(required.len());
    for name in required {
        if !matches!(name, CanonicalValue::Text(_)) {
            return Err(ToolProtocolError::schema(
                "required entries must be strings",
                path,
            ));
        }
        if normalized_required.contains(&name) {
            return Err(ToolProtocolError::schema(
                "required entries must be unique",
                path,
            ));
        }
        normalized_required.push(name);
        if !properties.is_empty() {
            let key = JsonText::new(&name.as_text().expect("text").to_string_lossy());
            if !properties.contains_key(&key) {
                return Err(ToolProtocolError::schema(
                    format!("required property {} is not declared", python_repr(name)),
                    path,
                ));
            }
        }
    }
    let additional = schema.get(&JsonText::new("additionalProperties"));
    let additional = match additional {
        None => AdditionalProperties::Allowed,
        Some(CanonicalValue::Bool(value)) => {
            if *value {
                AdditionalProperties::Allowed
            } else {
                AdditionalProperties::Forbidden
            }
        }
        Some(CanonicalValue::Object(schema)) => AdditionalProperties::Schema(schema.clone()),
        Some(_) => {
            return Err(ToolProtocolError::schema(
                "additionalProperties must be a boolean or schema",
                path,
            ));
        }
    };
    for name in &normalized_required {
        let key = JsonText::new(&name.as_text().expect("text").to_string_lossy());
        if !value.contains_key(&key) {
            return Err(ToolProtocolError::schema(
                format!("required property {} is missing", python_repr(name)),
                path,
            ));
        }
    }
    if let Some(minimum) = schema_int(schema, "minProperties", path)? {
        if (value.len() as i128) < minimum {
            return Err(ToolProtocolError::schema(
                "object has too few properties",
                path,
            ));
        }
    }
    if let Some(maximum) = schema_int(schema, "maxProperties", path)? {
        if (value.len() as i128) > maximum {
            return Err(ToolProtocolError::schema(
                "object has too many properties",
                path,
            ));
        }
    }
    let mut extras: Vec<String> = Vec::new();
    for (key, item) in value {
        let name = key.to_string_lossy();
        let subschema = properties.get(key);
        if let Some(subschema) = subschema {
            validate_against_schema(item, subschema, &format!("{path}.{name}"), 0)?;
            continue;
        }
        match &additional {
            AdditionalProperties::Forbidden => extras.push(name),
            AdditionalProperties::Allowed => {}
            AdditionalProperties::Schema(schema) => {
                validate_against_schema(
                    item,
                    &CanonicalValue::Object(schema.clone()),
                    &format!("{path}.{name}"),
                    0,
                )?;
            }
        }
    }
    if !extras.is_empty() {
        extras.sort();
        return Err(ToolProtocolError::schema(
            format!("unpermitted properties: {}", extras.join(", ")),
            path,
        ));
    }
    Ok(())
}

/// How a schema treats properties it does not declare.
enum AdditionalProperties {
    Allowed,
    Forbidden,
    Schema(CanonicalObject),
}

/// Validate an array value.
fn validate_array(
    value: &[CanonicalValue],
    schema: &CanonicalObject,
    path: &str,
) -> Result<(), ToolProtocolError> {
    if value.len() > MAX_COLLECTION_ITEMS {
        return Err(ToolProtocolError::schema(
            "array exceeds the bounded item count",
            path,
        ));
    }
    if let Some(minimum) = schema_int(schema, "minItems", path)? {
        if (value.len() as i128) < minimum {
            return Err(ToolProtocolError::schema("array has too few items", path));
        }
    }
    if let Some(maximum) = schema_int(schema, "maxItems", path)? {
        if (value.len() as i128) > maximum {
            return Err(ToolProtocolError::schema("array has too many items", path));
        }
    }
    if let Some(items) = get_present(schema, "items") {
        let CanonicalValue::Object(items) = items else {
            return Err(ToolProtocolError::schema("items must be a schema", path));
        };
        for (index, item) in value.iter().enumerate() {
            validate_against_schema(
                item,
                &CanonicalValue::Object(items.clone()),
                &format!("{path}[{index}]"),
                0,
            )?;
        }
    }
    let unique = matches!(
        schema.get(&JsonText::new("uniqueItems")),
        Some(CanonicalValue::Bool(true))
    );
    if unique {
        let mut seen: Vec<String> = Vec::with_capacity(value.len());
        for item in value {
            let fingerprint = canonical_json_dumps(item, crate::protocol::MAX_JSON_BYTES)?;
            if seen.contains(&fingerprint) {
                return Err(ToolProtocolError::schema(
                    "array items must be unique",
                    path,
                ));
            }
            seen.push(fingerprint);
        }
    }
    Ok(())
}

/// Validate a string value.
fn validate_string(
    value: &JsonText,
    schema: &CanonicalObject,
    path: &str,
) -> Result<(), ToolProtocolError> {
    let minimum_length = schema_int(schema, "minLength", path)?;
    let maximum_length = schema_int(schema, "maxLength", path)?;
    if let Some(minimum) = minimum_length {
        if (value.len() as i128) < minimum {
            return Err(ToolProtocolError::schema(
                "string is shorter than the permitted length",
                path,
            ));
        }
    }
    if let Some(maximum) = maximum_length {
        if (value.len() as i128) > maximum {
            return Err(ToolProtocolError::schema(
                "string is longer than the permitted length",
                path,
            ));
        }
    }
    let Some(pattern) = get_present(schema, "pattern") else {
        return Ok(());
    };
    let CanonicalValue::Text(pattern) = pattern else {
        return Err(ToolProtocolError::schema("pattern must be a string", path));
    };
    let compiled = bounded_pattern(pattern.code_points(), maximum_length, path)?;
    if compiled.search(value.code_points()).is_none() {
        return Err(ToolProtocolError::schema(
            "string does not match the declared pattern",
            path,
        ));
    }
    Ok(())
}

/// Validate a number value.
fn validate_number(
    value: f64,
    schema: &CanonicalObject,
    path: &str,
) -> Result<(), ToolProtocolError> {
    let minimum = schema_float(schema, "minimum", path)?;
    let maximum = schema_float(schema, "maximum", path)?;
    let exclusive_minimum = schema_float(schema, "exclusiveMinimum", path)?;
    let exclusive_maximum = schema_float(schema, "exclusiveMaximum", path)?;
    let multiple_of = schema_float(schema, "multipleOf", path)?;
    if let Some(minimum) = minimum {
        if value < minimum {
            return Err(ToolProtocolError::schema(
                "number is below the permitted minimum",
                path,
            ));
        }
    }
    if let Some(maximum) = maximum {
        if value > maximum {
            return Err(ToolProtocolError::schema(
                "number is above the permitted maximum",
                path,
            ));
        }
    }
    if let Some(exclusive_minimum) = exclusive_minimum {
        if value <= exclusive_minimum {
            return Err(ToolProtocolError::schema(
                "number is not above the exclusive minimum",
                path,
            ));
        }
    }
    if let Some(exclusive_maximum) = exclusive_maximum {
        if value >= exclusive_maximum {
            return Err(ToolProtocolError::schema(
                "number is not below the exclusive maximum",
                path,
            ));
        }
    }
    if let Some(multiple_of) = multiple_of {
        if multiple_of <= 0.0 {
            return Err(ToolProtocolError::schema(
                "multipleOf must be positive",
                path,
            ));
        }
        let remainder = python_remainder(value, multiple_of);
        if !is_close_to_zero(remainder) {
            return Err(ToolProtocolError::schema(
                "number is not a multiple of the declared value",
                path,
            ));
        }
    }
    Ok(())
}

/// Refuse a pattern that is not provably bounded at schema registration.
fn pattern_rejects(schema: &CanonicalObject, path: &str) -> Result<(), ToolProtocolError> {
    let Some(pattern) = get_present(schema, "pattern") else {
        return Ok(());
    };
    let CanonicalValue::Text(pattern) = pattern else {
        return Err(ToolProtocolError::schema("pattern must be a string", path));
    };
    let maximum_length = schema_int(schema, "maxLength", path)?;
    bounded_pattern(pattern.code_points(), maximum_length, path)?;
    Ok(())
}

/// The integral bound of a schema keyword.
fn schema_int(
    schema: &CanonicalObject,
    keyword: &str,
    path: &str,
) -> Result<Option<i128>, ToolProtocolError> {
    match schema_number(schema, keyword, path)? {
        None => Ok(None),
        Some(NumberBound::Integer(number)) => Ok(Some(bigint_to_i128(&number))),
        Some(NumberBound::Float(number)) => {
            if number.fract() != 0.0 {
                return Err(ToolProtocolError::schema(
                    format!("{keyword} must be an integer"),
                    path,
                ));
            }
            Ok(Some(number as i128))
        }
    }
}

/// The floating point bound of a schema keyword.
fn schema_float(
    schema: &CanonicalObject,
    keyword: &str,
    path: &str,
) -> Result<Option<f64>, ToolProtocolError> {
    match schema_number(schema, keyword, path)? {
        None => Ok(None),
        Some(NumberBound::Integer(number)) => Ok(Some(bigint_to_f64(&number))),
        Some(NumberBound::Float(number)) => Ok(Some(number)),
    }
}

/// The raw numeric bound of a schema keyword.
fn schema_number(
    schema: &CanonicalObject,
    keyword: &str,
    path: &str,
) -> Result<Option<NumberBound>, ToolProtocolError> {
    let Some(value) = schema.get(&JsonText::new(keyword)) else {
        return Ok(None);
    };
    match value {
        CanonicalValue::Bool(_) => Err(ToolProtocolError::schema(
            format!("{keyword} must be a number"),
            path,
        )),
        CanonicalValue::Integer(number) => Ok(Some(NumberBound::Integer(number.clone()))),
        CanonicalValue::Float(number) => {
            if !number.is_finite() {
                return Err(ToolProtocolError::schema(
                    format!("{keyword} must be finite"),
                    path,
                ));
            }
            Ok(Some(NumberBound::Float(*number)))
        }
        _ => Err(ToolProtocolError::schema(
            format!("{keyword} must be a number"),
            path,
        )),
    }
}

/// Convert an arbitrary precision integer to a saturated `i128`.
fn bigint_to_i128(number: &num_bigint::BigInt) -> i128 {
    use num_traits::ToPrimitive;
    number.to_i128().unwrap_or_else(|| {
        if number.sign() == num_bigint::Sign::Minus {
            i128::MIN
        } else {
            i128::MAX
        }
    })
}

/// Convert an arbitrary precision integer to a saturated `f64`.
fn bigint_to_f64(number: &num_bigint::BigInt) -> f64 {
    use num_traits::ToPrimitive;
    number.to_f64().unwrap_or_else(|| {
        if number.sign() == num_bigint::Sign::Minus {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        }
    })
}

/// CPython's floating point remainder.
///
/// CPython's `%` takes the sign of the divisor, unlike the truncated
/// remainder of Rust's `%` operator.
fn python_remainder(value: f64, divisor: f64) -> f64 {
    let remainder = value % divisor;
    if remainder != 0.0 && (remainder < 0.0) != (divisor < 0.0) {
        remainder + divisor
    } else {
        remainder
    }
}

/// Whether a value is close to zero with CPython's `math.isclose` defaults.
fn is_close_to_zero(value: f64) -> bool {
    let difference = value.abs();
    difference <= 1e-9 * f64::max(1.0, difference)
}

/// Compare two JSON values with CPython's equality rules.
///
/// Booleans are never equal to a number, and an integer equals a float only
/// when the float is exactly that integer.
pub(crate) fn json_equal(left: &CanonicalValue, right: &CanonicalValue) -> bool {
    match (left, right) {
        (CanonicalValue::Bool(left), CanonicalValue::Bool(right)) => left == right,
        (CanonicalValue::Bool(_), _) | (_, CanonicalValue::Bool(_)) => false,
        (CanonicalValue::Object(left), CanonicalValue::Object(right)) => {
            left.len() == right.len()
                && left.iter().zip(right.iter()).all(
                    |((left_key, left_value), (right_key, right_value))| {
                        left_key == right_key && json_equal(left_value, right_value)
                    },
                )
        }
        (CanonicalValue::Array(left), CanonicalValue::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right.iter())
                    .all(|(left, right)| json_equal(left, right))
        }
        (CanonicalValue::Integer(left), CanonicalValue::Integer(right)) => left == right,
        (CanonicalValue::Float(left), CanonicalValue::Float(right)) => left == right,
        (CanonicalValue::Integer(left), CanonicalValue::Float(right)) => {
            float_equals_integer(*right, left)
        }
        (CanonicalValue::Float(left), CanonicalValue::Integer(right)) => {
            float_equals_integer(*left, right)
        }
        (CanonicalValue::Text(left), CanonicalValue::Text(right)) => left == right,
        (CanonicalValue::Null, CanonicalValue::Null) => true,
        _ => false,
    }
}

/// Whether a float is exactly the given integer.
fn float_equals_integer(value: f64, integer: &num_bigint::BigInt) -> bool {
    use num_traits::FromPrimitive;
    num_bigint::BigInt::from_f64(value).is_some_and(|value| value == *integer)
}
