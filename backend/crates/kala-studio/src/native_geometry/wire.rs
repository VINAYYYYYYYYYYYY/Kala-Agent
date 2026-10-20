//! Wire-value helpers for the native geometry backend.
//!
//! Two renderings of the same construction script are needed and they are
//! *not* the same bytes:
//!
//! * the **canonical** form feeds the design identity: it is
//!   `model_dump(mode="json", exclude_none=True)` rendered with sorted keys,
//!   compact separators and ASCII escaping, exactly the Python
//!   `canonical_script_json`;
//! * the **response** form is the Pydantic model dump in field order, and
//!   Pydantic renders its floats with pydantic-core's text (the Rust
//!   `serde_json`/`ryu` form), not with CPython's `repr`.
//!
//! The helpers here produce both from the accepted `kala-contracts` wire
//! value, so neither rendering is re-implemented.

use kala_contracts::{ConstructionScript, ContractModel, WireValue};
use kala_json::{CanonicalValue, Limits, canonical_dumps};
use serde_json::{Map, Number, Value};

/// Drop the `null` entries an `exclude_none=True` model dump removes.
///
/// The Python dump removes a `None` field from its parent object. A `null`
/// inside an *array* is data, not an absent field, so it is kept.
pub fn prune_nulls(value: &WireValue) -> WireValue {
    match value {
        WireValue::Null => WireValue::Null,
        WireValue::Bool(flag) => WireValue::Bool(*flag),
        WireValue::Integer(number) => WireValue::Integer(number.clone()),
        WireValue::Float(number) => WireValue::Float(*number),
        WireValue::Text(text) => WireValue::Text(text.clone()),
        WireValue::Array(values) => WireValue::Array(values.iter().map(prune_nulls).collect()),
        WireValue::Object(fields) => WireValue::Object(
            fields
                .iter()
                .filter(|(_, entry)| !matches!(entry, WireValue::Null))
                .map(|(name, entry)| (name.clone(), prune_nulls(entry)))
                .collect(),
        ),
    }
}

/// The `model_dump(mode="json", exclude_none=True)` shape of a script.
#[must_use]
pub fn script_dump(script: &ConstructionScript) -> WireValue {
    prune_nulls(&script.to_wire_value())
}

/// The canonical design-identity bytes of a script.
///
/// This is the exact `canonical_script_json` of the Python service: the
/// pruned model dump, sorted keys, compact separators, ASCII escaping.
pub fn canonical_script_json(script: &ConstructionScript) -> Result<String, String> {
    let canonical = script_dump(script)
        .to_canonical_value()
        .map_err(|error| error.to_string())?;
    canonical_dumps(&canonical, &Limits::default()).map_err(|error| error.to_string())
}

/// Render a wire value the way Pydantic renders a `response_model` body.
///
/// Pydantic's JSON serializer writes floats with pydantic-core's text, which
/// is the Rust `serde_json`/`ryu` form. `serde_json` is configured with
/// `preserve_order`, so the object order of the model dump is kept.
#[must_use]
pub fn to_response_value(value: &WireValue) -> Value {
    match value {
        WireValue::Null => Value::Null,
        WireValue::Bool(flag) => Value::Bool(*flag),
        WireValue::Integer(number) => integer_value(&number.to_string()),
        WireValue::Float(number) => float_value(*number),
        WireValue::Text(text) => Value::String(text.to_string_lossy()),
        WireValue::Array(values) => Value::Array(values.iter().map(to_response_value).collect()),
        WireValue::Object(fields) => {
            let mut map = Map::new();
            for (name, entry) in fields {
                map.insert(name.to_string_lossy(), to_response_value(entry));
            }
            Value::Object(map)
        }
    }
}

/// Render a canonical journal value the way Pydantic renders a payload dict.
#[must_use]
pub fn canonical_to_response_value(value: &CanonicalValue) -> Value {
    match value {
        CanonicalValue::Null => Value::Null,
        CanonicalValue::Bool(flag) => Value::Bool(*flag),
        CanonicalValue::Integer(number) => integer_value(&number.to_string()),
        CanonicalValue::Float(number) => float_value(*number),
        CanonicalValue::Text(text) => Value::String(text.to_string_lossy()),
        CanonicalValue::Array(values) => {
            Value::Array(values.iter().map(canonical_to_response_value).collect())
        }
        CanonicalValue::Object(entries) => {
            let mut map = Map::new();
            for (name, entry) in entries {
                map.insert(name.to_string_lossy(), canonical_to_response_value(entry));
            }
            Value::Object(map)
        }
    }
}

/// A finite float as a JSON number, or `null` for a non-finite value.
#[must_use]
pub fn float_value(number: f64) -> Value {
    Number::from_f64(number).map_or(Value::Null, Value::Number)
}

/// An arbitrary-precision integer as a JSON number.
///
/// `serde_json` is configured with `arbitrary_precision`, so the exact
/// decimal text of a big integer survives the round trip.
fn integer_value(text: &str) -> Value {
    serde_json::from_str::<Value>(text).unwrap_or(Value::Null)
}
