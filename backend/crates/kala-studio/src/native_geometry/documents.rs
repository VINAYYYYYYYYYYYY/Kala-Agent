//! The response documents of the native geometry backend.
//!
//! Every success body is a Pydantic `response_model` output, so the object
//! order is the model field order and the float text is pydantic-core's. The
//! documents are therefore built as `serde_json` values (which is configured
//! with `preserve_order` and writes floats with the same `ryu` text) and
//! serialised with `serde_json::to_vec`.
//!
//! The refusal bodies are the flat Studio envelope, in the two orderings the
//! Python source uses: the Pydantic-model order for framework validation
//! errors and the sorted order for route refusals.

use kala_cad_providers::ProviderStatus;
use kala_studio_journal::{JournalVerification, SessionEvent};
use serde_json::{Map, Number, Value};

use crate::native_geometry::service::{
    CheckRow, CheckValue, DesignOutcome, ImportedView, truncate_chars,
};
use crate::native_geometry::wire::{canonical_to_response_value, to_response_value};

/// The largest label the design document carries.
pub const MAX_LABEL_LENGTH: usize = 120;

/// The complete, provider-measured document of one design.
#[must_use]
pub fn design_document(outcome: &DesignOutcome) -> Value {
    let mut map = Map::new();
    map.insert(
        "design_id".to_owned(),
        Value::String(outcome.design_id.clone()),
    );
    map.insert(
        "label".to_owned(),
        Value::String(truncate_chars(&outcome.label, MAX_LABEL_LENGTH)),
    );
    map.insert(
        "provider".to_owned(),
        Value::String(outcome.provider.clone()),
    );
    map.insert(
        "provider_version".to_owned(),
        optional_text(outcome.provider_version.as_deref()),
    );
    map.insert(
        "measured_by".to_owned(),
        Value::String(outcome.measured_by.clone()),
    );
    map.insert("script".to_owned(), to_response_value(&outcome.script));
    map.insert("measured".to_owned(), measured_document(outcome));
    map.insert(
        "checks".to_owned(),
        Value::Array(outcome.checks.iter().map(check_document).collect()),
    );
    map.insert("verified".to_owned(), Value::Bool(outcome.verified));
    map.insert("view".to_owned(), view_document(outcome));
    map.insert("artifacts".to_owned(), artifacts_document(outcome));
    map.insert(
        "journal_head".to_owned(),
        optional_text(outcome.journal_head.as_deref()),
    );
    map.insert(
        "material".to_owned(),
        optional_text(outcome.material.as_deref()),
    );
    map.insert(
        "tolerances".to_owned(),
        Value::Array(
            outcome
                .tolerances
                .iter()
                .map(|value| Value::String(value.clone()))
                .collect(),
        ),
    );
    map.insert(
        "declared_envelope".to_owned(),
        match outcome.declared_envelope {
            Some(bounds) => bounds_document(bounds.min_mm, bounds.max_mm),
            None => Value::Null,
        },
    );
    map.insert("cached".to_owned(), Value::Bool(outcome.cached));
    map.insert(
        "warnings".to_owned(),
        Value::Array(
            outcome
                .warnings
                .iter()
                .map(|value| Value::String(value.clone()))
                .collect(),
        ),
    );
    map.insert(
        "component_count".to_owned(),
        Value::Number(Number::from(outcome.component_count)),
    );
    map.insert("fused".to_owned(), Value::Bool(outcome.fused));
    Value::Object(map)
}

/// The provider-measured numbers of one design.
fn measured_document(outcome: &DesignOutcome) -> Value {
    let measured = &outcome.measured;
    let mut map = Map::new();
    map.insert(
        "volume_mm3".to_owned(),
        optional_float(measured.volume_mm3()),
    );
    map.insert(
        "surface_area_mm2".to_owned(),
        optional_float(measured.area_mm2()),
    );
    map.insert(
        "centroid_mm".to_owned(),
        match measured.centroid_mm() {
            Some(centroid) => float_array(centroid),
            None => Value::Null,
        },
    );
    map.insert(
        "bounds".to_owned(),
        match measured.bounds() {
            Some(bounds) => bounds_document(bounds.min_mm(), bounds.max_mm()),
            None => Value::Null,
        },
    );
    map.insert(
        "size_mm".to_owned(),
        match measured.bounds() {
            Some(bounds) => float_array(bounds.size_mm()),
            None => Value::Null,
        },
    );
    map.insert(
        "valid".to_owned(),
        match outcome.valid {
            Some(value) => Value::Bool(value),
            None => Value::Null,
        },
    );
    map.insert(
        "solid_count".to_owned(),
        match outcome.solid_count {
            Some(count) => Value::Number(Number::from(count)),
            None => Value::Null,
        },
    );
    map.insert("mass_g".to_owned(), optional_float(measured.mass_g()));
    Value::Object(map)
}

/// One declared-versus-measured check row.
fn check_document(check: &CheckRow) -> Value {
    let mut map = Map::new();
    map.insert("id".to_owned(), Value::String(check.id.clone()));
    map.insert("label".to_owned(), Value::String(check.label.clone()));
    map.insert("expected".to_owned(), check_value(check.expected));
    map.insert(
        "measured".to_owned(),
        match check.measured {
            Some(value) => check_value(value),
            None => Value::Null,
        },
    );
    map.insert("tolerance".to_owned(), float(check.tolerance));
    map.insert("unit".to_owned(), Value::String(check.unit.clone()));
    map.insert("passed".to_owned(), Value::Bool(check.passed));
    map.insert(
        "status".to_owned(),
        Value::String(check.status().to_owned()),
    );
    Value::Object(map)
}

/// One check value.
fn check_value(value: CheckValue) -> Value {
    match value {
        CheckValue::Bool(flag) => Value::Bool(flag),
        CheckValue::Float(number) => float(number),
        CheckValue::Pair(low, high) => Value::Array(vec![float(low), float(high)]),
    }
}

/// The display view imported from the provider's STEP.
fn view_document(outcome: &DesignOutcome) -> Value {
    let mut map = Map::new();
    map.insert("positions".to_owned(), float_list(&outcome.view.positions));
    map.insert("normals".to_owned(), float_list(&outcome.view.normals));
    map.insert("indices".to_owned(), index_list(&outcome.view.indices));
    map.insert(
        "vertex_count".to_owned(),
        Value::Number(Number::from(outcome.view.vertex_count())),
    );
    map.insert(
        "triangle_count".to_owned(),
        Value::Number(Number::from(outcome.view.triangle_count())),
    );
    map.insert(
        "deflection_mm".to_owned(),
        float(outcome.view.deflection_mm),
    );
    map.insert("engine".to_owned(), Value::String("ocp".to_owned()));
    map.insert(
        "bounds".to_owned(),
        bounds_document(outcome.view_bounds.min_mm, outcome.view_bounds.max_mm),
    );
    map.insert(
        "solids".to_owned(),
        Value::Number(Number::from(outcome.view_solids)),
    );
    map.insert(
        "warnings".to_owned(),
        Value::Array(
            outcome
                .view_warnings
                .iter()
                .map(|value| Value::String(value.clone()))
                .collect(),
        ),
    );
    Value::Object(map)
}

/// The content digests stored for one design.
fn artifacts_document(outcome: &DesignOutcome) -> Value {
    let mut map = Map::new();
    map.insert(
        "script_sha256".to_owned(),
        Value::String(outcome.script_sha256.clone()),
    );
    map.insert(
        "step_sha256".to_owned(),
        optional_text(outcome.step_sha256.as_deref()),
    );
    map.insert(
        "native_sha256".to_owned(),
        optional_text(outcome.native_sha256.as_deref()),
    );
    map.insert(
        "native_kind".to_owned(),
        optional_text(outcome.native_kind.as_deref()),
    );
    Value::Object(map)
}

/// The view-only import document of one STEP file.
#[must_use]
pub fn import_document(view: &ImportedView, step_sha256: &str) -> Value {
    let mut inner = Map::new();
    inner.insert("positions".to_owned(), float_list(&view.mesh.positions));
    inner.insert("normals".to_owned(), float_list(&view.mesh.normals));
    inner.insert("indices".to_owned(), index_list(&view.mesh.indices));
    inner.insert(
        "bounds".to_owned(),
        bounds_document(view.bounds.min_mm, view.bounds.max_mm),
    );
    let mut map = Map::new();
    map.insert("view".to_owned(), Value::Object(inner));
    map.insert(
        "step_sha256".to_owned(),
        Value::String(step_sha256.to_owned()),
    );
    map.insert(
        "solids".to_owned(),
        Value::Number(Number::from(view.solids)),
    );
    map.insert(
        "warnings".to_owned(),
        Value::Array(
            view.warnings
                .iter()
                .map(|value| Value::String(value.clone()))
                .collect(),
        ),
    );
    Value::Object(map)
}

/// One bounded page of Studio journal events.
#[must_use]
pub fn journal_page_document(
    events: &[SessionEvent],
    next_after: i64,
    has_more: bool,
    limit: i64,
) -> Value {
    let mut map = Map::new();
    map.insert(
        "events".to_owned(),
        Value::Array(events.iter().map(journal_event_document).collect()),
    );
    map.insert(
        "next_after".to_owned(),
        Value::Number(Number::from(next_after)),
    );
    map.insert("has_more".to_owned(), Value::Bool(has_more));
    map.insert("limit".to_owned(), Value::Number(Number::from(limit)));
    Value::Object(map)
}

/// One stored journal event.
#[must_use]
pub fn journal_event_document(event: &SessionEvent) -> Value {
    let mut map = Map::new();
    map.insert(
        "sequence".to_owned(),
        Value::Number(Number::from(event.sequence)),
    );
    map.insert(
        "event_type".to_owned(),
        Value::String(event.event_type.clone()),
    );
    map.insert(
        "aggregate_id".to_owned(),
        Value::String(event.aggregate_id.clone()),
    );
    map.insert(
        "payload".to_owned(),
        match event.payload.as_ref() {
            Some(payload) => canonical_to_response_value(payload),
            None => Value::Null,
        },
    );
    map.insert("digest".to_owned(), Value::String(event.digest.clone()));
    map.insert(
        "occurred_at".to_owned(),
        Value::String(event.occurred_at.clone()),
    );
    map.insert(
        "previous_event_digest".to_owned(),
        Value::String(event.previous_event_digest.clone()),
    );
    map.insert(
        "event_digest".to_owned(),
        Value::String(event.event_digest.clone()),
    );
    Value::Object(map)
}

/// The hash-chain verdict of the Studio journal.
#[must_use]
pub fn journal_verify_document(verification: &JournalVerification) -> Value {
    let mut map = Map::new();
    map.insert("ok".to_owned(), Value::Bool(verification.ok));
    map.insert(
        "event_count".to_owned(),
        Value::Number(Number::from(verification.event_count)),
    );
    map.insert(
        "head_digest".to_owned(),
        optional_text(verification.head_digest.as_deref()),
    );
    map.insert(
        "first_bad_sequence".to_owned(),
        match verification.first_bad_sequence {
            Some(sequence) => Value::Number(Number::from(sequence)),
            None => Value::Null,
        },
    );
    Value::Object(map)
}

/// The live availability of every CAD provider.
#[must_use]
pub fn providers_cad_document(statuses: &[ProviderStatus]) -> Value {
    let mut map = Map::new();
    map.insert(
        "providers".to_owned(),
        Value::Array(statuses.iter().map(provider_status_document).collect()),
    );
    Value::Object(map)
}

/// One CAD provider row.
fn provider_status_document(status: &ProviderStatus) -> Value {
    let mut map = Map::new();
    map.insert("id".to_owned(), Value::String(status.id().to_owned()));
    map.insert("name".to_owned(), Value::String(status.name().to_owned()));
    map.insert("available".to_owned(), Value::Bool(status.available()));
    map.insert("version".to_owned(), optional_text(status.version()));
    map.insert("reason".to_owned(), optional_text(status.reason()));
    map.insert(
        "measured_by".to_owned(),
        Value::String(status.measured_by().to_owned()),
    );
    map.insert("mode".to_owned(), Value::String(status.mode().to_owned()));
    Value::Object(map)
}

/// The strict reading of one prompt.
#[must_use]
pub fn intent_parse_document(
    family: Option<&str>,
    intent: Option<Value>,
    missing: &[String],
    notes: &[String],
) -> Value {
    let mut map = Map::new();
    map.insert("family".to_owned(), optional_text(family));
    map.insert("intent".to_owned(), intent.unwrap_or(Value::Null));
    map.insert(
        "missing".to_owned(),
        Value::Array(
            missing
                .iter()
                .map(|value| Value::String(value.clone()))
                .collect(),
        ),
    );
    map.insert(
        "notes".to_owned(),
        Value::Array(
            notes
                .iter()
                .map(|value| Value::String(value.clone()))
                .collect(),
        ),
    );
    Value::Object(map)
}

/// The validated intent of one model reply.
#[must_use]
pub fn intent_validate_document(intent: Value) -> Value {
    let mut map = Map::new();
    map.insert("intent".to_owned(), intent);
    Value::Object(map)
}

/// An axis-aligned box in millimetres.
fn bounds_document(min_mm: [f64; 3], max_mm: [f64; 3]) -> Value {
    let mut map = Map::new();
    map.insert("min".to_owned(), float_array(min_mm));
    map.insert("max".to_owned(), float_array(max_mm));
    Value::Object(map)
}

fn float(value: f64) -> Value {
    Number::from_f64(value).map_or(Value::Null, Value::Number)
}

fn optional_float(value: Option<f64>) -> Value {
    match value {
        Some(number) => float(number),
        None => Value::Null,
    }
}

fn float_array(values: [f64; 3]) -> Value {
    Value::Array(values.iter().copied().map(float).collect())
}

fn float_list(values: &[f64]) -> Value {
    Value::Array(values.iter().copied().map(float).collect())
}

fn index_list(values: &[u32]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|value| Value::Number(Number::from(*value)))
            .collect(),
    )
}

fn optional_text(value: Option<&str>) -> Value {
    match value {
        Some(text) => Value::String(text.to_owned()),
        None => Value::Null,
    }
}

/// The hidden-line drawing of one stored design.
#[must_use]
pub fn drawing_document(design_id: &str, drawing: &kala_cad::Drawing) -> Value {
    let mut map = Map::new();
    map.insert("design_id".to_owned(), Value::String(design_id.to_owned()));
    map.insert(
        "views".to_owned(),
        Value::Array(
            drawing
                .views
                .iter()
                .map(|view| {
                    let mut row = Map::new();
                    row.insert(
                        "name".to_owned(),
                        Value::String(view.name.as_str().to_owned()),
                    );
                    row.insert(
                        "visible".to_owned(),
                        Value::Array(view.visible.iter().map(polyline_document).collect()),
                    );
                    row.insert(
                        "hidden".to_owned(),
                        Value::Array(view.hidden.iter().map(polyline_document).collect()),
                    );
                    row.insert("truncated".to_owned(), Value::Bool(view.truncated));
                    row.insert(
                        "point_count".to_owned(),
                        Value::Number(Number::from(view.point_count())),
                    );
                    row.insert(
                        "polyline_count".to_owned(),
                        Value::Number(Number::from(view.polyline_count())),
                    );
                    Value::Object(row)
                })
                .collect(),
        ),
    );
    map.insert(
        "chord_tolerance_mm".to_owned(),
        float(drawing.chord_tolerance_mm),
    );
    map.insert(
        "max_points".to_owned(),
        Value::Number(Number::from(drawing.max_points)),
    );
    map.insert(
        "point_count".to_owned(),
        Value::Number(Number::from(drawing.total_point_count())),
    );
    Value::Object(map)
}

/// One drawn polyline of a view.
fn polyline_document(polyline: &kala_cad::Polyline) -> Value {
    let mut map = Map::new();
    map.insert(
        "points".to_owned(),
        Value::Array(
            polyline
                .points
                .iter()
                .map(|point| Value::Array(vec![float(point.u), float(point.v)]))
                .collect(),
        ),
    );
    map.insert("closed".to_owned(), Value::Bool(polyline.closed));
    map.insert(
        "point_count".to_owned(),
        Value::Number(Number::from(polyline.point_count())),
    );
    Value::Object(map)
}

/// The closed loops of one section cut.
#[must_use]
pub fn section_document(design_id: &str, section: &kala_cad::Section) -> Value {
    let mut map = Map::new();
    map.insert("design_id".to_owned(), Value::String(design_id.to_owned()));
    map.insert(
        "plane".to_owned(),
        Value::String(section.plane.as_str().to_owned()),
    );
    map.insert("offset_mm".to_owned(), float(section.offset_mm));
    map.insert(
        "loops".to_owned(),
        Value::Array(section.loops.iter().map(section_loop_document).collect()),
    );
    map.insert("area_mm2".to_owned(), float(section.area_mm2));
    map.insert("area_exact".to_owned(), Value::Bool(section.area_exact));
    map.insert(
        "chord_tolerance_mm".to_owned(),
        float(section.chord_tolerance_mm),
    );
    map.insert("truncated".to_owned(), Value::Bool(section.truncated));
    map.insert(
        "outer_loop_count".to_owned(),
        Value::Number(Number::from(section.outer_loop_count())),
    );
    map.insert(
        "inner_loop_count".to_owned(),
        Value::Number(Number::from(section.inner_loop_count())),
    );
    map.insert(
        "point_count".to_owned(),
        Value::Number(Number::from(section.point_count())),
    );
    Value::Object(map)
}

/// One closed loop of a section cut.
fn section_loop_document(loop_: &kala_cad::SectionLoop) -> Value {
    let mut map = Map::new();
    map.insert(
        "points".to_owned(),
        Value::Array(
            loop_
                .points
                .iter()
                .map(|point| Value::Array(vec![float(point.u), float(point.v)]))
                .collect(),
        ),
    );
    map.insert("area_mm2".to_owned(), float(loop_.area_mm2));
    map.insert("area_exact".to_owned(), Value::Bool(loop_.area_exact));
    map.insert("outer".to_owned(), Value::Bool(loop_.outer));
    let mut bounds_map = Map::new();
    match loop_.bounds() {
        Some(bounds) => {
            bounds_map.insert("min_u".to_owned(), float(bounds.min_u));
            bounds_map.insert("min_v".to_owned(), float(bounds.min_v));
            bounds_map.insert("max_u".to_owned(), float(bounds.max_u));
            bounds_map.insert("max_v".to_owned(), float(bounds.max_v));
            bounds_map.insert("width".to_owned(), float(bounds.width()));
            bounds_map.insert("height".to_owned(), float(bounds.height()));
            bounds_map.insert("center_u".to_owned(), float(bounds.center_u()));
            bounds_map.insert("center_v".to_owned(), float(bounds.center_v()));
        }
        None => {
            for name in [
                "min_u", "min_v", "max_u", "max_v", "width", "height", "center_u", "center_v",
            ] {
                bounds_map.insert(name.to_owned(), float(0.0));
            }
        }
    }
    map.insert("bounds".to_owned(), Value::Object(bounds_map));
    map.insert(
        "centroid".to_owned(),
        match loop_.centroid() {
            Some(centroid) => Value::Array(vec![float(centroid.u), float(centroid.v)]),
            None => Value::Array(vec![float(0.0), float(0.0)]),
        },
    );
    map.insert(
        "point_count".to_owned(),
        Value::Number(Number::from(loop_.point_count())),
    );
    Value::Object(map)
}
