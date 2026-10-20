//! Studio design templates, the `DesignIntent` contract and the strict local
//! design-prompt parser, as pure logic.
//!
//! This crate is a native port of three Python modules:
//!
//! * `kala_agent.studio.design.templates` — the deterministic L-bracket,
//!   stepped-shaft and housing-envelope part templates, including every
//!   feasibility check and message;
//! * `kala_agent.studio.design.intent` — the `DesignIntent` Pydantic models,
//!   the exported JSON Schema, fenced-JSON extraction and validation errors;
//! * the design-prompt half of `kala_agent.studio.providers.local` —
//!   [`detect_design_family`], [`parse_design_prompt`], the
//!   ISO 273 medium clearance table and the missing-value replies.
//!
//! It owns no geometry and never imports OpenCascade. A template returns a
//! [`kala_contracts::ConstructionScript`] plus a declared envelope, declared
//! tolerance strings and a label; volume and surface area remain kernel
//! measurements.
//!
//! Python is used only by the test-time differential oracle. Production code
//! never spawns Python.
//!
//! # Compatibility notes
//!
//! * The JSON Schema text is a byte-exact artifact generated from the live
//!   Pydantic model (`assets/design_intent_schema.json`); the differential test
//!   re-generates it from Python on every run. Exporting the checked-in bytes
//!   is the only way this crate can guarantee byte identity without embedding a
//!   second Pydantic schema generator.
//! * Text is handled with Python code-point semantics through
//!   [`kala_contracts::Text`], so Pydantic whitespace stripping and `len()`
//!   limits match.
//! * JSON reply parsing follows CPython's `json.loads` error vocabulary,
//!   including `NaN`/`Infinity` (which then fail `allow_inf_nan=False`), but
//!   imposes a bounded 512-level nesting depth instead of CPython's recursion
//!   limit.
//! * Rust's typed template arguments replace Python's dynamic `bool`/`float`
//!   coercion. `hole_count` is `i64` and keeps the Python range message.

#![forbid(unsafe_code)]

mod format;
mod intent;
mod json;
mod limit;
mod prompt;
mod pytext;
mod script;
mod templates;

pub use format::{format_g, format_g_with};
pub use intent::{
    DEFAULT_BRACKET_DEPTH_MM, DEFAULT_BRACKET_WIDTH_MM, DESIGN_INTENT_INSTRUCTION, DesignIntent,
    DesignIntentInputError, DesignIntentParseError, DesignIntentValidationError, DesignIssue,
    DesignIssueLocation, HoleCount, HousingCall, LbracketCall, ShaftSegment, SteppedShaftCall,
    TemplateCall, design_intent_json_schema, design_intent_schema_text, parse_design_intent_text,
    strip_design_json_block,
};
pub use limit::{MAX_DIMENSION_MM, MIN_DIMENSION_MM};
pub use prompt::{
    DesignFamily, DesignPrompt, DesignPromptError, M_SIZE_CLEARANCE, MAX_RESPONSE_LENGTH,
    design_prompt_reply, detect_design_family, iso_273_medium_clearance, parse_design_prompt,
};
/// The templates' own `ShaftSegment` (diameter/length in millimetres), named
/// apart from the intent model's [`ShaftSegment`].
pub use templates::ShaftSegment as TemplateShaftSegment;
pub use templates::{
    Bounds, BoundsError, DEFAULT_HOLE_EDGE_MM, TemplateError, TemplateResult, housing_envelope,
    l_bracket, stepped_shaft,
};

/// Maximum components in one construction script, re-exported from the
/// accepted construction contract.
pub const MAX_COMPONENTS: usize = kala_contracts::MAX_COMPONENTS;

/// Maximum tolerances on one design intent (`intent.MAX_TOLERANCES`).
pub const MAX_TOLERANCES: usize = intent::MAX_TOLERANCES;
/// Maximum material-string length (`intent.MAX_MATERIAL_LENGTH`).
pub const MAX_MATERIAL_LENGTH: usize = intent::MAX_MATERIAL_LENGTH;
/// Maximum label length (`intent.MAX_LABEL_LENGTH`).
pub const MAX_LABEL_LENGTH: usize = intent::MAX_LABEL_LENGTH;
/// Maximum tolerance-string length (`intent.MAX_TOLERANCE_LENGTH`).
pub const MAX_TOLERANCE_LENGTH: usize = intent::MAX_TOLERANCE_LENGTH;
/// Maximum shaft segments in one intent call (`intent.MAX_SEGMENTS`).
pub const MAX_SEGMENTS: usize = intent::MAX_SEGMENTS;
