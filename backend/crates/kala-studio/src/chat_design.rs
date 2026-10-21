//! Chat with designs (ADR-0010, ADR-0011).
//!
//! A chat turn may carry a design. The front door never parses a design
//! intent itself and never compiles geometry: it asks the worker, which owns
//! the strict parser, the compiler and the journal.
//!
//! * **Local provider:** the worker parses the prompt with `intent/parse`.
//!   A prompt that is not a design request keeps the ordinary analytic
//!   reply; a design prompt gets the worker's strict reading, and the design
//!   reply text is built from the parsed family, the missing values and the
//!   parser notes.
//! * **Remote providers:** the front door adds the DesignIntent JSON Schema
//!   (fetched from `intent/schema`) and the instruction to the prompt. It
//!   extracts the one design object from a fenced or bare JSON reply and
//!   validates it with `intent/validate`. One repair round-trip is allowed,
//!   with the exact validation error and the failing field's schema excerpt.
//!   A reply with no design object is a question about missing values: it is
//!   not repaired, and the chat text says plainly that no part was built.
//!
//! **The validation repair sees the same world as the first call.** Every
//! model call is stateless, so the repair call is a fresh call: without the
//! instruction, the full schema and the JSON block the model itself sent, the
//! repair re-authors the design blind, in a different shape, and loses the
//! values it had. The repair prompt therefore carries the same instruction
//! and the same full schema the first call carried, the model's own previous
//! JSON (bounded), the exact validation error and the failing field's schema
//! excerpt. When the repair still fails, the honest text names **both**
//! errors: what went wrong the first time and what went wrong the second.
//!
//! **The build-feedback repair.** A reply that validates can still build into
//! the wrong part: a cut that removed no material, a failed envelope check, or
//! a builder that refuses the script as unsupported. The turn then spends its
//! one repair call on the *build* instead of the validation, and hands the
//! model the concrete facts: the failed check rows, the provider warnings, or
//! the refusal reason. The whole turn never spends more than one repair call,
//! so the worst case stays at two model calls.
//!
//! A repair call that itself fails is not a turn failure: the first design was
//! built and journalled already, so the turn keeps it, says that the repair
//! attempt failed, and answers 200. Every model call is stateless, so the
//! repair prompt carries the same design instruction the first call carried.
//!
//! ADR-0011 is binding on the provider choice: `opencascade` is never
//! selected automatically. When the request carries no `cad_provider`, the
//! turn asks the operator which provider to use and compiles nothing. A
//! selected provider is passed to the worker unchanged: the worker decides
//! whether it can build on this host, the front door relays its refusal in
//! the source's own wording, and no other provider is ever tried.

use std::future::Future;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::geometry_backend::{BackendError, GeometryBackend};
use crate::providers::{ChatOutcome, ProviderFailure};

/// The instruction the remote provider prompt carries.
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

/// The provider that builds a design with the worker's own compiler.
pub const CAD_PROVIDER_OPENCASCADE: &str = "opencascade";

/// The aggregate the Studio journal stores its events under.
pub const JOURNAL_AGGREGATE_ID: &str = "studio";

/// The event type the front door appends for a chat turn.
pub const CHAT_TURN_EVENT_TYPE: &str = "chat.turn";

/// The largest number of missing values the journal payload carries.
pub const MAX_JOURNAL_MISSING: usize = 16;

/// The largest length of one journaled missing value.
pub const MAX_JOURNAL_MISSING_LENGTH: usize = 64;

/// The design families the strict parser names, and their readable labels.
const FAMILY_LABELS: &[(&str, &str)] = &[
    ("l_bracket", "L-bracket"),
    ("stepped_shaft", "stepped shaft"),
    ("housing_envelope", "rectangular housing envelope"),
];

/// The result of the design hook of one chat turn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignTurn {
    /// The reply text the turn produces.
    pub text: String,
    /// The design result the worker returned.
    pub design: Option<Value>,
    /// The design identifier, when a design was compiled.
    pub design_id: Option<String>,
    /// Whether the turn asked for a design.
    pub design_requested: bool,
    /// The values the operator still has to state.
    pub missing: Vec<String>,
    /// The model that answered the turn.
    pub model: Option<String>,
}

/// Why a design turn could not be completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesignTurnError {
    /// The worker could not be reached.
    Backend(BackendError),
    /// The provider failed while the remote design prompt was answered.
    Provider(ProviderFailure),
}

impl std::fmt::Display for DesignTurnError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Backend(error) => write!(formatter, "{error}"),
            Self::Provider(failure) => write!(formatter, "{failure:?}"),
        }
    }
}

impl std::error::Error for DesignTurnError {}

/// The strict reading of one prompt, as the worker reports it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedIntent {
    /// The template family, when the prompt asks for a design.
    pub family: Option<String>,
    /// The complete intent, when every required value was stated.
    pub intent: Option<Value>,
    /// The values the operator still has to state.
    pub missing: Vec<String>,
    /// The parser's notes.
    pub notes: Vec<String>,
}

impl ParsedIntent {
    /// Whether the prompt asked for a design at all.
    #[must_use]
    pub fn is_design_request(&self) -> bool {
        self.family.is_some()
    }
}

/// Read the strict reading of a prompt from the worker.
///
/// # Errors
///
/// Returns the backend failure when the worker cannot answer.
pub async fn parse_intent(
    backend: &dyn GeometryBackend,
    prompt: &str,
) -> Result<ParsedIntent, BackendError> {
    let mut body = Map::new();
    body.insert("prompt".to_owned(), Value::String(prompt.to_owned()));
    let response = backend
        .intent_parse(
            &serde_json::to_vec(&Value::Object(body)).unwrap_or_default(),
            Some("application/json"),
        )
        .await?;
    let parsed = response.json().ok_or_else(|| {
        BackendError::Transport("the worker returned an unreadable intent".to_owned())
    })?;
    Ok(ParsedIntent {
        family: parsed
            .get("family")
            .and_then(Value::as_str)
            .map(str::to_owned),
        intent: parsed
            .get("intent")
            .cloned()
            .filter(|value| !value.is_null()),
        missing: string_list(parsed.get("missing")),
        notes: string_list(parsed.get("notes")),
    })
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The reply the local helper gives for a design prompt.
///
/// The text is the worker's strict reading rendered in the source's own
/// wording: the family, the missing values and the notes. The front door
/// never guesses a dimension and never states a measurement.
#[must_use]
pub fn design_reply_text(parsed: &ParsedIntent) -> String {
    let family = parsed.family.clone().unwrap_or_default();
    let label = FAMILY_LABELS
        .iter()
        .find(|(name, _)| *name == family)
        .map(|(_, label)| (*label).to_owned())
        .unwrap_or_else(|| family.replace('_', " "));
    let article = if label
        .chars()
        .next()
        .is_some_and(|first| "aeiou".contains(first.to_ascii_lowercase()))
    {
        "an"
    } else {
        "a"
    };
    let notes = parsed.notes.join(" ");
    match &parsed.intent {
        None => {
            let mut notes = notes;
            if !notes.is_empty() {
                notes.insert(0, ' ');
            }
            let need = if parsed.missing.is_empty() {
                "a complete set of declared dimensions".to_owned()
            } else {
                parsed.missing.join(", ")
            };
            format!(
                "Local helper. I can draft {article} {label}, but I need these declared values: {need}.{notes} No design was compiled."
            )
        }
        Some(_) => {
            let mut notes = notes;
            if !notes.is_empty() {
                notes.push(' ');
            }
            format!(
                "Local helper. Design intent parsed from declared values. {notes}Compiling and measuring with OpenCASCADE."
            )
        }
    }
}

/// The remote provider prompt that asks for a DesignIntent.
#[must_use]
pub fn design_prompt(prompt: &str, schema_text: &str) -> String {
    format!(
        "{DESIGN_INTENT_INSTRUCTION}\n\nDesignIntent JSON Schema:\n{schema_text}\n\nUser request:\n{prompt}"
    )
}

/// The largest previous JSON block the validation-repair prompt carries.
///
/// Sized from the measured maximum design: a 500-step canonical script is
/// 90 497 characters with worst-case values (81 366 for a realistic 100-hole
/// plate), and a pretty-printed maximum is 171 546. 200 000 holds every one of
/// those whole, so a repair always sees its own complete JSON, while the reply
/// itself stays bounded by the 1 MB transport limit.
pub const MAX_REPAIR_PREVIOUS_CHARS: usize = 200_000;

/// The largest validation error the repair prompt and the honest text carry.
pub const MAX_REPAIR_ERROR_CHARS: usize = 512;

/// The single repair prompt the remote provider is allowed.
///
/// Every model call is stateless. The repair call therefore carries the same
/// [`DESIGN_INTENT_INSTRUCTION`] and the same full DesignIntent schema the
/// first call carried, the JSON block the model itself sent (bounded), the
/// exact validation error, and a bounded schema excerpt for the field that
/// failed. Without its own previous JSON the repair re-authors the design
/// blind: it invents a different shape and loses the values it had.
#[must_use]
pub fn repair_prompt(prompt: &str, error: &str, schema_text: &str, previous: &str) -> String {
    let error = bounded_error(error);
    let location = error_location(&error);
    let schema_section = design_schema_excerpt(schema_text, &error)
        .map(|excerpt| {
            let location = if location.is_empty() {
                "the failing field"
            } else {
                location.as_str()
            };
            format!("\n\nSchema for {location}:\n{excerpt}")
        })
        .unwrap_or_default();
    let previous_section = previous_design_json_section(previous);
    format!(
        "{DESIGN_INTENT_INSTRUCTION}\n\nDesignIntent JSON Schema:\n{schema_text}{previous_section}\n\nYour previous reply could not be validated as a DesignIntent: {error}{schema_section}\n\nReturn exactly one fenced ```json block containing a corrected DesignIntent for the same request. Keep every value the user stated and do not invent a value the user did not state. If a required value is missing, reply in prose with the missing values instead.\n\nUser request:\n{prompt}"
    )
}

/// One validation error, bounded to [`MAX_REPAIR_ERROR_CHARS`].
fn bounded_error(error: &str) -> String {
    error.chars().take(MAX_REPAIR_ERROR_CHARS).collect()
}

/// The section the validation-repair prompt carries about the model's own
/// previous reply.
///
/// The model must see the JSON it sent: without it the repair re-authors the
/// design blind. The block is extracted from the reply and bounded. A reply
/// whose JSON is not a balanced object still shows its own text, so the model
/// sees exactly what it wrote.
fn previous_design_json_section(previous: &str) -> String {
    let (header, body) = match extract_design_json_block(previous) {
        Some(block) => ("The JSON block you sent:", block),
        None => ("Your previous reply:", previous.trim().to_owned()),
    };
    let body: String = body.chars().take(MAX_REPAIR_PREVIOUS_CHARS).collect();
    if body.is_empty() {
        return String::new();
    }
    format!("\n\n{header}\n{body}")
}

/// The first balanced JSON object of one model reply.
///
/// The block is taken from the first fenced block that carries an object, and
/// from the text outside the fences when no fence carries one. A bare object
/// in prose is found the same way.
#[must_use]
pub fn extract_design_json_block(text: &str) -> Option<String> {
    let fences = fence_spans(text);
    for fence in &fences {
        let content = &text[fence.content_start..fence.content_end];
        if let Some((start, end)) = first_object_span(content) {
            return Some(content[start..end].to_owned());
        }
    }
    let outside = outside_fences(text, &fences);
    let (start, end) = first_object_span(&outside)?;
    Some(outside[start..end].to_owned())
}

/// One fenced block of a model reply.
struct FenceSpan {
    /// The offset of the opening fence.
    outer_start: usize,
    /// The offset after the closing fence.
    outer_end: usize,
    /// The offset of the first character of the block's content.
    content_start: usize,
    /// The offset after the last character of the block's content.
    content_end: usize,
}

/// The fenced blocks of one model reply.
///
/// The language tag is skipped, so an unlabelled fence and a ```json fence
/// behave the same. An unclosed fence runs to the end of the reply.
fn fence_spans(text: &str) -> Vec<FenceSpan> {
    let bytes = text.as_bytes();
    let mut spans: Vec<FenceSpan> = Vec::new();
    let mut index = 0usize;
    while let Some(opening) = find_bytes(bytes, b"```", index) {
        let mut cursor = opening + 3;
        while cursor < bytes.len() && (bytes[cursor] == b' ' || bytes[cursor] == b'\t') {
            cursor += 1;
        }
        while cursor < bytes.len()
            && (bytes[cursor].is_ascii_alphanumeric()
                || bytes[cursor] == b'_'
                || bytes[cursor] == b'-')
        {
            cursor += 1;
        }
        while cursor < bytes.len() && (bytes[cursor] == b' ' || bytes[cursor] == b'\t') {
            cursor += 1;
        }
        if bytes[cursor..].starts_with(b"\r\n") {
            cursor += 2;
        } else if cursor < bytes.len() && bytes[cursor] == b'\n' {
            cursor += 1;
        }
        let Some(closing) = find_bytes(bytes, b"```", cursor) else {
            spans.push(FenceSpan {
                outer_start: opening,
                outer_end: bytes.len(),
                content_start: cursor,
                content_end: bytes.len(),
            });
            break;
        };
        spans.push(FenceSpan {
            outer_start: opening,
            outer_end: closing + 3,
            content_start: cursor,
            content_end: closing,
        });
        index = closing + 3;
    }
    spans
}

/// The offset of the first occurrence of `needle` at or after `from`.
fn find_bytes(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= haystack.len() || needle.is_empty() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

/// The reply with every fenced block removed.
fn outside_fences(text: &str, fences: &[FenceSpan]) -> String {
    let mut outside = String::new();
    let mut last = 0usize;
    for fence in fences {
        outside.push_str(&text[last..fence.outer_start]);
        last = fence.outer_end;
    }
    outside.push_str(&text[last..]);
    outside
}

/// The span of the first balanced `{...}` object, strings respected.
fn first_object_span(text: &str) -> Option<(usize, usize)> {
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, character) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => {
                if depth > 0 {
                    in_string = true;
                }
            }
            '{' => {
                if depth == 0 {
                    start = index;
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    return Some((start, index + character.len_utf8()));
                }
            }
            _ => {}
        }
    }
    None
}

/// The largest compact script the build-repair prompt carries.
///
/// A 500-step script is 90 497 characters in canonical JSON with worst-case
/// values and 81 366 with realistic ones; 200 000 holds either whole, so the
/// build repair never reads a script cut mid-object.
pub const MAX_BUILD_REPAIR_SCRIPT_CHARS: usize = 200_000;

/// The largest build feedback the build-repair prompt carries.
pub const MAX_BUILD_REPAIR_FEEDBACK_CHARS: usize = 2_000;

/// The largest schema excerpt the build-repair prompt carries.
pub const MAX_BUILD_REPAIR_SCHEMA_CHARS: usize = 700;

/// The largest length of the build-repair prompt without the script.
///
/// The script is bounded separately by [`MAX_BUILD_REPAIR_SCRIPT_CHARS`], and
/// the user's own request is appended unchanged and is not part of this bound,
/// exactly like in the first design prompt.
pub const MAX_BUILD_REPAIR_PROMPT_CHARS: usize = 9_000;

/// The concrete facts one failed build hands back to the model.
///
/// The feedback is what the model could not see: the part it asked for was
/// built, but the build said the part is not the part. Every value here comes
/// from the worker's own report, never from a guess.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuildFeedback {
    /// The compact JSON of the script the model sent.
    pub script: String,
    /// The failed check rows, already rendered.
    pub failed_checks: Vec<String>,
    /// The provider warnings of the build.
    pub warnings: Vec<String>,
    /// The builder refusal, when the builder refused the script.
    pub refusal: Option<String>,
}

impl BuildFeedback {
    /// The bounded feedback body the repair prompt carries.
    #[must_use]
    pub fn text(&self) -> String {
        let mut body = String::from("the script you sent:\n");
        body.push_str(&self.script);
        body.push('\n');
        if let Some(refusal) = &self.refusal {
            body.push_str("\nthe builder refused the script:\n");
            body.push_str(refusal);
            body.push('\n');
        }
        if !self.failed_checks.is_empty() || !self.warnings.is_empty() {
            body.push_str("\nthe build reported:\n");
            for check in &self.failed_checks {
                body.push_str("- ");
                body.push_str(check);
                body.push('\n');
            }
            for warning in &self.warnings {
                body.push_str("- warning: ");
                body.push_str(warning);
                body.push('\n');
            }
        }
        body.trim_end().to_owned()
    }

    /// The one short sentence that says why the first attempt was rejected.
    #[must_use]
    pub fn reason(&self) -> String {
        if let Some(refusal) = &self.refusal {
            return format!("the builder refused the script: {refusal}");
        }
        let checks = self.failed_checks.join(" and ");
        let warning = self
            .warnings
            .first()
            .map(|warning| format!("; the provider warned: {warning}"))
            .unwrap_or_default();
        if checks.is_empty() {
            if warning.is_empty() {
                return "the build did not produce a verified part".to_owned();
            }
            return format!("the build was not verified{warning}");
        }
        format!("the build failed {checks}{warning}")
    }
}

/// The build-repair prompt the remote provider is allowed.
///
/// Every model call is stateless, so the repair is a fresh call and the model
/// no longer sees the supported subset, the plane rule or the worked examples
/// it read the first time. The prompt therefore carries the same
/// [`DESIGN_INTENT_INSTRUCTION`] the first call carried, unchanged, followed
/// by the user's original request, the script the model sent (compact JSON,
/// bounded) and the concrete build feedback: the failed check rows, the
/// provider warnings, or the builder's refusal. The model is told to fix the
/// script so that every cut removes material and the part matches the request,
/// and to keep every value the user stated.
#[must_use]
pub fn build_repair_prompt(prompt: &str, feedback: &str, schema_text: &str) -> String {
    let feedback: String = feedback
        .chars()
        .take(MAX_BUILD_REPAIR_FEEDBACK_CHARS)
        .collect();
    let schema: String = build_repair_schema_excerpt(schema_text);
    format!(
        "{DESIGN_INTENT_INSTRUCTION}\n\nYour previous reply was built, but the build was rejected: {feedback}\n\nReturn exactly one fenced ```json block containing a corrected DesignIntent for the same request. Fix the script so that every cut removes material and the part matches the request, and keep every dimension the user stated: do not invent a value the user did not state and do not change a value the user did state. If a required value is missing, reply in prose with the missing values instead.\n\nSchema for script:\n{schema}\n\nUser request:\n{prompt}"
    )
}

/// The bounded schema excerpt the build-repair prompt carries.
///
/// The whole DesignIntent schema is about 11 KB; the repair only needs the
/// shape of the script, so the excerpt is the `script` property of the root
/// schema, and the root summary when that property cannot be read.
fn build_repair_schema_excerpt(schema_text: &str) -> String {
    let excerpt = || -> Option<String> {
        let schema: Value = serde_json::from_str(schema_text).ok()?;
        let node = schema.get("properties")?.get("script")?;
        let text = serde_json::to_string_pretty(node).ok()?;
        Some(text.chars().take(MAX_BUILD_REPAIR_SCHEMA_CHARS).collect())
    };
    excerpt().unwrap_or_else(|| {
        let summary =
            root_schema_summary(&serde_json::from_str(schema_text).unwrap_or(Value::Null));
        serde_json::to_string_pretty(&summary)
            .unwrap_or_default()
            .chars()
            .take(MAX_BUILD_REPAIR_SCHEMA_CHARS)
            .collect()
    })
}

/// Render one check value the way the repair prompt states it.
fn render_check_value(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "not reported".to_owned(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(Value::Number(number)) => match number.as_f64() {
            // A whole count reads better without its decimal tail, and the
            // value is the same number either way.
            Some(value) if value.fract() == 0.0 && value.abs() < 1e15 => {
                format!("{}", value as i64)
            }
            _ => number.to_string(),
        },
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => {
            let rendered = items
                .iter()
                .map(|item| render_check_value(Some(item)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{rendered}]")
        }
        Some(other) => other.to_string(),
    }
}

/// The compact JSON of the script the model sent.
fn compact_script(intent: &Value, design: Option<&Value>) -> String {
    let script = design
        .and_then(|design| design.get("script"))
        .filter(|value| !value.is_null())
        .or_else(|| intent.get("script"))
        .unwrap_or(intent);
    let text = serde_json::to_string(script).unwrap_or_default();
    text.chars().take(MAX_BUILD_REPAIR_SCRIPT_CHARS).collect()
}

/// The feedback one failed build produces.
///
/// A design that built and verified has no feedback: nothing is wrong. A
/// design that built and failed a check reports the failed rows and the
/// provider warnings. A builder refusal reports the refusal itself.
#[must_use]
pub fn build_feedback(intent: &Value, build: &BuildOutcome) -> Option<BuildFeedback> {
    match build {
        BuildOutcome::NoProvider => None,
        BuildOutcome::Built(design) => {
            if design.get("verified").and_then(Value::as_bool) == Some(true) {
                return None;
            }
            let failed_checks = design
                .get("checks")
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter(|row| row.get("passed").and_then(Value::as_bool) != Some(true))
                        .map(|row| {
                            let id = row.get("id").and_then(Value::as_str).unwrap_or_default();
                            let expected = render_check_value(row.get("expected"));
                            let measured = render_check_value(row.get("measured"));
                            format!("the check '{id}' (expected {expected}, measured {measured})")
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let warnings = design
                .get("warnings")
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if failed_checks.is_empty() && warnings.is_empty() {
                return None;
            }
            Some(BuildFeedback {
                script: compact_script(intent, Some(design)),
                failed_checks,
                warnings,
                refusal: None,
            })
        }
        BuildOutcome::Refused(failure) => {
            if !failure.is_script_refusal() {
                return None;
            }
            Some(BuildFeedback {
                script: compact_script(intent, None),
                failed_checks: Vec::new(),
                warnings: Vec::new(),
                refusal: Some(failure.summary()),
            })
        }
    }
}

/// The prose of a reply that asked for values instead of carrying a design.
const NO_DESIGN_PROSE: &str = "Design intent received.";

/// The honest text for a turn that produced no design.
fn no_build_text(prose: &str, reason: &str) -> String {
    if prose == NO_DESIGN_PROSE {
        format!("No part was built: {reason}")
    } else {
        format!("{prose} No part was built: {reason}")
    }
}

/// The reason a turn that spent its repair on the validation still built
/// nothing.
///
/// **Both** errors are named: the first reply's own error and the repair's
/// error. A user who reads only the last line never learns what went wrong
/// the first time, and the first failure is the one that explains why the
/// repair was needed at all. Each error is bounded.
fn validation_repair_failed_reason(first_error: &str, second_error: &str) -> String {
    format!(
        "the first reply was not a valid design ({}), and the reply after one repair attempt was not a valid design either ({}).",
        bounded_error(first_error),
        bounded_error(second_error)
    )
}

/// Whether a reply carries any object that could be a design.
fn has_design_candidate(text: &str) -> bool {
    text.contains('{')
}

/// The first field path an error names, when it is a dotted identifier path.
fn error_location(error: &str) -> String {
    let head = error.split(": ").next().unwrap_or_default().trim();
    if head.is_empty()
        || !head.split('.').all(|token| {
            !token.is_empty()
                && token
                    .chars()
                    .all(|character| character.is_alphanumeric() || character == '_')
        })
    {
        return String::new();
    }
    head.to_owned()
}

/// A bounded schema excerpt for the field an error names.
fn design_schema_excerpt(schema_text: &str, error: &str) -> Option<String> {
    let schema: Value = serde_json::from_str(schema_text).ok()?;
    let location = error_location(error);
    let tokens: Vec<&str> = location
        .split('.')
        .filter(|token| !token.is_empty())
        .collect();
    let (node, progressed) = walk_schema(&schema, &schema, &tokens);
    let excerpt = if progressed {
        node.clone()
    } else {
        root_schema_summary(&schema)
    };
    let text = serde_json::to_string_pretty(&excerpt).ok()?;
    Some(text.chars().take(700).collect())
}

fn root_schema_summary(schema: &Value) -> Value {
    let required = schema
        .get("required")
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    let mut names: Vec<String> = schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| properties.keys().cloned().collect())
        .unwrap_or_default();
    names.sort();
    let properties = Value::Object(
        names
            .into_iter()
            .map(|name| (name, Value::Object(Map::new())))
            .collect(),
    );
    // The key order is the sorted order, and the property names are sorted
    // too: the Python reference renders the same summary with
    // `sort_keys=True`, so both prompts carry the same bytes.
    serde_json::json!({
        "properties": properties,
        "required": required,
        "type": "object",
    })
}

fn walk_schema<'a>(schema: &'a Value, start: &'a Value, tokens: &[&str]) -> (&'a Value, bool) {
    let mut node = start;
    let mut progressed = false;
    let mut index = 0;
    while index < tokens.len() {
        node = resolve_schema(schema, node);
        let token = tokens[index];
        if let Some(next) = node
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get(token))
        {
            node = next;
            progressed = true;
            index += 1;
            continue;
        }
        if token.chars().all(|character| character.is_ascii_digit()) {
            if let Some(items) = node.get("items") {
                node = items;
                progressed = true;
                index += 1;
                continue;
            }
        }
        if let Some(pointer) = discriminator_mapping(node)
            .and_then(|mapping| mapping.get(token))
            .and_then(Value::as_str)
        {
            if let Some(target) = schema_pointer(schema, pointer) {
                node = target;
                progressed = true;
                index += 1;
                continue;
            }
        }
        if let Some(branch) = select_schema_branch(schema, node, token) {
            node = branch;
            continue;
        }
        index += 1;
    }
    (resolve_schema(schema, node), progressed)
}

fn select_schema_branch<'a>(schema: &'a Value, node: &'a Value, token: &str) -> Option<&'a Value> {
    for key in ["anyOf", "oneOf"] {
        let Some(branches) = node.get(key).and_then(Value::as_array) else {
            continue;
        };
        for branch in branches {
            let resolved = resolve_schema(schema, branch);
            if resolved
                .get("properties")
                .and_then(Value::as_object)
                .is_some_and(|properties| properties.contains_key(token))
            {
                return Some(branch);
            }
            if token.chars().all(|character| character.is_ascii_digit())
                && resolved.get("items").is_some()
            {
                return Some(branch);
            }
            if discriminator_mapping(resolved).is_some_and(|mapping| mapping.contains_key(token)) {
                return Some(branch);
            }
        }
    }
    None
}

fn resolve_schema<'a>(schema: &'a Value, node: &'a Value) -> &'a Value {
    let mut current = node;
    for _ in 0..16 {
        let Some(pointer) = current.get("$ref").and_then(Value::as_str) else {
            break;
        };
        let Some(target) = schema_pointer(schema, pointer) else {
            break;
        };
        current = target;
    }
    current
}

fn schema_pointer<'a>(schema: &'a Value, pointer: &str) -> Option<&'a Value> {
    schema.pointer(pointer.strip_prefix('#').unwrap_or(pointer))
}

fn discriminator_mapping(node: &Value) -> Option<&Map<String, Value>> {
    if let Some(mapping) = node
        .get("discriminator")
        .and_then(|discriminator| discriminator.get("mapping"))
        .and_then(Value::as_object)
    {
        return Some(mapping);
    }
    for key in ["anyOf", "oneOf"] {
        let Some(branches) = node.get(key).and_then(Value::as_array) else {
            continue;
        };
        for branch in branches {
            if let Some(mapping) = discriminator_mapping(branch) {
                return Some(mapping);
            }
        }
    }
    None
}

/// The prose of a reply with its fenced design block removed.
#[must_use]
pub fn strip_design_json_block(text: &str) -> String {
    let stripped = strip_blocks(text);
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        "Design intent received.".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Remove every fenced design block from a reply.
fn strip_blocks(text: &str) -> String {
    static PATTERN: &str = r"(?is)```(?:json)?\s*(\{.*\})\s*```";
    match regex::Regex::new(PATTERN) {
        Ok(pattern) => pattern.replace_all(text, "").into_owned(),
        Err(_) => text.to_owned(),
    }
}

/// The measured summary the turn appends after a design was compiled.
///
/// The provider that measured the model is named, and a provider that
/// reported no number is never given a substitute.
#[must_use]
pub fn measured_summary(design: &Value) -> String {
    let label = design
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let verified = design
        .get("verified")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let verdict = if verified {
        "B-Rep verified"
    } else {
        "B-Rep NOT verified"
    };
    let measured_by = design
        .get("measured_by")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let design_id = design
        .get("design_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let measured = design.get("measured");
    let volume = measured
        .and_then(|value| value.get("volume_mm3"))
        .and_then(Value::as_f64);
    let area = measured
        .and_then(|value| value.get("surface_area_mm2"))
        .and_then(Value::as_f64);
    let size = measured
        .and_then(|value| value.get("size_mm"))
        .and_then(Value::as_array)
        .filter(|values| values.len() >= 3)
        .map(|values| {
            values
                .iter()
                .map(|value| value.as_f64().unwrap_or(0.0))
                .collect::<Vec<_>>()
        });
    match (volume, area, size) {
        (Some(volume), Some(area), Some(size)) => format!(
            "{label}: {verdict}; measured by {measured_by}. Measured volume {volume:.3} mm^3, surface area {area:.3} mm^2, envelope {:.3} x {:.3} x {:.3} mm. design_id {design_id}.",
            size[0], size[1], size[2],
        ),
        _ => format!(
            "{label}: {verdict}; measured by {measured_by}. The provider did not report volume, surface area, or envelope; no number is substituted. design_id {design_id}."
        ),
    }
}

/// Join the reply parts the way the source joins them.
#[must_use]
pub fn join_text(parts: &[&str]) -> String {
    let joined = parts
        .iter()
        .filter_map(|part| {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    joined.chars().take(4_096).collect()
}

/// One CAD provider row, as the front door can prove it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CadProviderStatus {
    /// The provider identifier.
    pub id: String,
    /// The provider's role: `opt-in` for the built-in builder, `external`
    /// for a provider that builds outside the studio.
    pub mode: String,
    /// Whether this front door can build with the provider right now.
    pub available: bool,
}

/// The CAD providers the studio can offer.
///
/// The listing is best effort: an unreachable worker must not turn a chat
/// turn into a failure. The worker's own `providers/cad` route is the
/// authority when it answers; otherwise the front door reports the
/// ADR-0011 provider set and the availability it can prove itself, which is
/// the built-in OpenCascade builder.
pub async fn available_cad_providers(backend: &dyn GeometryBackend) -> Vec<CadProviderStatus> {
    if let Ok(response) = backend.providers_cad().await {
        if response.is_success() {
            if let Some(parsed) = response.json() {
                let entries = parsed
                    .get("providers")
                    .and_then(Value::as_array)
                    .or_else(|| parsed.as_array())
                    .cloned()
                    .unwrap_or_default();
                let rows: Vec<CadProviderStatus> = entries
                    .iter()
                    .filter_map(|entry| match entry {
                        Value::String(text) => Some(CadProviderStatus {
                            id: text.clone(),
                            mode: "external".to_owned(),
                            available: false,
                        }),
                        Value::Object(map) => {
                            let id = map.get("id").and_then(Value::as_str)?.to_owned();
                            let mode = map
                                .get("mode")
                                .and_then(Value::as_str)
                                .unwrap_or("external")
                                .to_owned();
                            let available = map
                                .get("available")
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            Some(CadProviderStatus {
                                id,
                                mode,
                                available,
                            })
                        }
                        _ => None,
                    })
                    .collect();
                if !rows.is_empty() {
                    return rows;
                }
            }
        }
    }
    let opencascade = backend.health().await.design_available;
    vec![
        CadProviderStatus {
            id: CAD_PROVIDER_OPENCASCADE.to_owned(),
            mode: "opt-in".to_owned(),
            available: opencascade,
        },
        CadProviderStatus {
            id: "freecad".to_owned(),
            mode: "external".to_owned(),
            available: false,
        },
        CadProviderStatus {
            id: "solidedge".to_owned(),
            mode: "external".to_owned(),
            available: false,
        },
    ]
}

/// The message that asks the operator which CAD provider to use.
#[must_use]
pub fn provider_choice_message(providers: &[CadProviderStatus]) -> String {
    let options = providers
        .iter()
        .map(|provider| format!("{} ({})", provider.id, provider.mode))
        .collect::<Vec<_>>()
        .join(", ");
    let available = providers
        .iter()
        .filter(|provider| provider.available)
        .map(|provider| provider.id.clone())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Choose a CAD provider before I build this design: {options}. Available on this host: {}. The studio never selects a provider automatically. No design was built.",
        if available.is_empty() {
            "none"
        } else {
            &available
        }
    )
}

/// One refused design request, as the worker answered it.
///
/// The worker answers a `provider_unavailable` design request with its error
/// envelope: the message, then `reason: …` and `available: …` details. The
/// source's chat route renders the same facts, so the front door relays the
/// worker's answer instead of inventing a refusal of its own. Nothing is
/// substituted and no other provider is tried.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignFailure {
    /// The worker's HTTP status.
    pub status: u16,
    /// The worker's bounded error code.
    pub code: String,
    /// The worker's bounded error message.
    pub message: String,
    /// The worker's bounded error details.
    pub details: Vec<String>,
}

impl DesignFailure {
    /// Read one worker error envelope.
    ///
    /// A body that is not the Studio error document keeps the status and the
    /// front door's own fallback code and message, exactly like the previous
    /// refusal text did.
    #[must_use]
    pub fn from_response(response: &crate::geometry_backend::WorkerResponse) -> Self {
        let parsed = response.json();
        let text = |key: &str| {
            parsed
                .as_ref()
                .and_then(|value| value.get(key))
                .and_then(Value::as_str)
        };
        Self {
            status: response.status,
            code: text("code").unwrap_or("design_unavailable").to_owned(),
            message: text("message")
                .unwrap_or("the design could not be built")
                .to_owned(),
            details: parsed
                .as_ref()
                .and_then(|value| value.get("details"))
                .and_then(Value::as_array)
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// Whether the worker refused because the selected provider cannot build
    /// on this host (the source's `provider_unavailable`, 503).
    #[must_use]
    pub fn is_provider_unavailable(&self) -> bool {
        self.status == 503 && self.code == "provider_unavailable"
    }

    /// The code/message/detail form used when the turn reports a failure that
    /// is not the source's CAD provider refusal.
    #[must_use]
    pub fn summary(&self) -> String {
        let detail = self.details.first().map(String::as_str).unwrap_or_default();
        if detail.is_empty() {
            format!("{}: {}", self.code, self.message)
        } else {
            format!("{}: {} ({})", self.code, self.message, detail)
        }
    }

    /// The chat reply for this refusal, in the source's own wording.
    ///
    /// A CAD provider refusal uses the source's `_provider_unavailable_chat_message`:
    /// the requested provider, the worker's reason, the providers that can
    /// build here and `No design was built.`
    #[must_use]
    pub fn chat_message(&self, provider: &str) -> String {
        if self.is_provider_unavailable() {
            let reason = self
                .detail_value("reason: ")
                .unwrap_or(self.message.as_str());
            let available = self.detail_value("available: ").unwrap_or("none");
            return format!(
                "The {provider} CAD provider is not available: {reason}. Available: {available}. No design was built."
            );
        }
        format!("The design could not be built: {}.", self.summary())
    }

    fn detail_value(&self, prefix: &str) -> Option<&str> {
        self.details
            .iter()
            .find_map(|detail| detail.strip_prefix(prefix))
    }

    /// Whether the refusal is about the model's own script.
    ///
    /// A provider that cannot build on this host, and a worker whose kernel is
    /// missing, are not the model's fault: the turn never spends a repair call
    /// on them. A compile or validation refusal is.
    #[must_use]
    pub fn is_script_refusal(&self) -> bool {
        !self.is_provider_unavailable()
            && self.code != "design_unavailable"
            && self.code != "design_not_found"
    }
}

/// What one build produced.
///
/// The turn needs the outcome, not only the text: a design that built and
/// failed a check, and a builder that refused the script, are the two facts
/// that earn the build-feedback repair.
#[derive(Clone, Debug, PartialEq)]
pub enum BuildOutcome {
    /// No CAD provider was selected: the turn asked the operator to choose one.
    NoProvider,
    /// The design was built. The value is the worker's design document.
    Built(Value),
    /// The builder refused the script.
    Refused(DesignFailure),
}

/// Compile one intent with the worker.
///
/// The ADR-0011 provider is part of the design request and is passed through
/// unchanged. The worker decides whether the provider can build on this host:
/// a provider it refuses comes back as a [`DesignFailure`] and the front door
/// never retries with another provider.
///
/// # Errors
///
/// Returns the backend failure when the worker cannot answer.
pub async fn compile_design(
    backend: &dyn GeometryBackend,
    intent: &Value,
    cad_provider: &str,
) -> Result<Result<Value, DesignFailure>, BackendError> {
    let mut body = match intent {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    // The operator's selection is authoritative: it always replaces anything
    // an intent might carry under the same name.
    body.insert(
        "provider".to_owned(),
        Value::String(cad_provider.to_owned()),
    );
    let payload = serde_json::to_vec(&Value::Object(body)).unwrap_or_default();
    let response = backend.design(&payload, Some("application/json")).await?;
    if response.is_success() {
        return Ok(Ok(response.json().unwrap_or(Value::Null)));
    }
    Ok(Err(DesignFailure::from_response(&response)))
}

/// Append the `chat.turn` event for one turn.
///
/// The payload never carries the prompt text, only its digest and length,
/// exactly like the source. `failure` is `Some` only for a design turn whose
/// provider call failed: it carries the error envelope's code and no reply
/// text. The failure key is absent on a completed turn, so the successful
/// payload stays byte-identical.
///
/// # Errors
///
/// Returns the backend failure when the worker cannot answer.
#[allow(clippy::too_many_arguments)]
pub async fn append_chat_turn(
    backend: &dyn GeometryBackend,
    provider: &str,
    model: Option<&str>,
    offline: bool,
    prompt: &str,
    design_id: Option<&str>,
    design_requested: bool,
    cad_provider: Option<&str>,
    missing: &[String],
    failure: Option<&str>,
) -> Result<(), BackendError> {
    let digest = Sha256::digest(prompt.as_bytes());
    let mut payload = Map::new();
    payload.insert("provider".to_owned(), Value::String(provider.to_owned()));
    payload.insert(
        "model".to_owned(),
        model
            .map(|value| Value::String(value.to_owned()))
            .unwrap_or(Value::Null),
    );
    payload.insert("offline".to_owned(), Value::Bool(offline));
    payload.insert(
        "prompt_sha256".to_owned(),
        Value::String(digest.iter().map(|byte| format!("{byte:02x}")).collect()),
    );
    payload.insert(
        "prompt_chars".to_owned(),
        Value::Number(prompt.chars().count().into()),
    );
    payload.insert(
        "design_id".to_owned(),
        design_id
            .map(|value| Value::String(value.to_owned()))
            .unwrap_or(Value::Null),
    );
    payload.insert("design_requested".to_owned(), Value::Bool(design_requested));
    payload.insert(
        "cad_provider".to_owned(),
        cad_provider
            .map(|value| Value::String(value.to_owned()))
            .unwrap_or(Value::Null),
    );
    payload.insert(
        "missing".to_owned(),
        Value::Array(
            missing
                .iter()
                .take(MAX_JOURNAL_MISSING)
                .map(|item| Value::String(item.chars().take(MAX_JOURNAL_MISSING_LENGTH).collect()))
                .collect(),
        ),
    );
    if let Some(failure) = failure {
        payload.insert("failure".to_owned(), Value::String(failure.to_owned()));
    }
    let mut request = Map::new();
    request.insert(
        "event_type".to_owned(),
        Value::String(CHAT_TURN_EVENT_TYPE.to_owned()),
    );
    request.insert("payload".to_owned(), Value::Object(payload));
    request.insert(
        "aggregate_id".to_owned(),
        Value::String(JOURNAL_AGGREGATE_ID.to_owned()),
    );
    let encoded = serde_json::to_vec(&Value::Object(request)).unwrap_or_default();
    backend
        .journal_append(&encoded, Some("application/json"))
        .await?;
    Ok(())
}

/// Run the local design turn.
///
/// The local helper's answer for a design prompt is the strict reading: the
/// family, the values that are missing and the parser notes. A complete
/// intent is compiled when the operator selected a CAD provider.
///
/// # Errors
///
/// Returns the backend failure when the worker cannot answer.
pub async fn local_design_turn(
    backend: &dyn GeometryBackend,
    parsed: &ParsedIntent,
    cad_provider: Option<&str>,
) -> DesignTurn {
    let mut turn = DesignTurn {
        text: design_reply_text(parsed),
        design: None,
        design_id: None,
        design_requested: true,
        missing: parsed.missing.clone(),
        model: None,
    };
    if let Some(intent) = parsed.intent.clone() {
        let _ = finish_design(backend, &intent, cad_provider, &mut turn).await;
    }
    turn
}

/// Run the remote design flow: schema, prompt, validate, one repair, design.
///
/// The turn spends at most ONE repair call in total. The validation repair
/// comes first: a reply that is not a valid DesignIntent is repaired with the
/// exact validation error. When that repair was not needed, the build may
/// spend it instead: a design that built unverified, or a builder that
/// refused the script, is repaired with the concrete build feedback. The worst
/// case is therefore two model calls.
///
/// # Errors
///
/// Returns the provider or backend failure that stopped the turn.
pub async fn remote_design_turn<F, Fut>(
    backend: &dyn GeometryBackend,
    prompt: &str,
    cad_provider: Option<&str>,
    mut call: F,
) -> Result<DesignTurn, DesignTurnError>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<ChatOutcome, ProviderFailure>>,
{
    let schema = backend
        .intent_schema()
        .await
        .map_err(DesignTurnError::Backend)?;
    let schema_text = String::from_utf8(schema.body).unwrap_or_default();
    let first = call(design_prompt(prompt, &schema_text))
        .await
        .map_err(DesignTurnError::Provider)?;
    if !has_design_candidate(&first.reply) {
        // A question about the values the model still needs is a legitimate
        // design answer: never spend a repair turn or invent the values.
        let prose = strip_design_json_block(&first.reply);
        return Ok(DesignTurn {
            text: no_build_text(&prose, "the reply did not include a design."),
            design: None,
            design_id: None,
            design_requested: true,
            missing: Vec::new(),
            model: Some(first.model.clone()),
        });
    }
    let mut reply_text = strip_design_json_block(&first.reply);
    // The one repair call of this turn. Once it is spent, the build may not
    // spend another one.
    let mut repair_spent = false;
    let intent = match validate_reply(backend, &first.reply).await {
        Ok(intent) => Some(intent),
        Err(first_error) => {
            repair_spent = true;
            // The repair call is stateless: it carries the model's own previous
            // JSON, so it corrects its own answer instead of re-authoring one.
            let repair = call(repair_prompt(
                prompt,
                &first_error,
                &schema_text,
                &first.reply,
            ))
            .await
            .map_err(DesignTurnError::Provider)?;
            reply_text = strip_design_json_block(&repair.reply);
            if !has_design_candidate(&repair.reply) {
                return Ok(DesignTurn {
                    text: no_build_text(
                        &reply_text,
                        "the reply asks for values instead of including a design.",
                    ),
                    design: None,
                    design_id: None,
                    design_requested: true,
                    missing: Vec::new(),
                    model: Some(repair.model.clone()),
                });
            }
            match validate_reply(backend, &repair.reply).await {
                Ok(intent) => Some(intent),
                Err(second_error) => {
                    let reason = validation_repair_failed_reason(&first_error, &second_error);
                    return Ok(DesignTurn {
                        text: no_build_text(&reply_text, &reason),
                        design: None,
                        design_id: None,
                        design_requested: true,
                        missing: Vec::new(),
                        model: Some(repair.model.clone()),
                    });
                }
            }
        }
    };
    let mut turn = DesignTurn {
        text: reply_text,
        design: None,
        design_id: None,
        design_requested: true,
        missing: Vec::new(),
        model: Some(first.model.clone()),
    };
    if let Some(intent) = intent {
        let build = finish_design(backend, &intent, cad_provider, &mut turn).await?;
        if !repair_spent {
            if let Some(feedback) = build_feedback(&intent, &build) {
                let context = BuildRepair {
                    prompt,
                    cad_provider,
                    schema_text: &schema_text,
                    feedback: &feedback,
                    first: &build,
                };
                repair_build(backend, &context, &mut turn, &mut call).await?;
            }
        }
    }
    Ok(turn)
}

/// The context one build repair needs.
struct BuildRepair<'a> {
    /// The user's original request.
    prompt: &'a str,
    /// The selected CAD provider.
    cad_provider: Option<&'a str>,
    /// The DesignIntent schema text.
    schema_text: &'a str,
    /// The concrete build feedback.
    feedback: &'a BuildFeedback,
    /// What the first attempt produced.
    first: &'a BuildOutcome,
}

/// Spend the turn's one repair call on the build, not on the validation.
///
/// The reply is validated and built again. A second design that verifies is
/// the turn's design and the text says plainly that the first attempt was
/// rejected and why. A second attempt that does not build, or that builds
/// unverified again, leaves the best honest state in place: the latest built
/// design with `verified: false` visible, or the "No part was built" text when
/// nothing built at all.
///
/// A repair CALL that fails is not a turn failure. The first design was built
/// and journalled already, so a provider that times out on the repair call
/// must not throw the first build away: the turn keeps the first design, says
/// that the repair attempt failed, and answers 200.
async fn repair_build<F, Fut>(
    backend: &dyn GeometryBackend,
    context: &BuildRepair<'_>,
    turn: &mut DesignTurn,
    call: &mut F,
) -> Result<(), DesignTurnError>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<ChatOutcome, ProviderFailure>>,
{
    let rejection = format!(
        "The first attempt was rejected: {}.",
        context.feedback.reason()
    );
    let repair = match call(build_repair_prompt(
        context.prompt,
        &context.feedback.text(),
        context.schema_text,
    ))
    .await
    {
        Ok(repair) => repair,
        Err(failure) => {
            // The first build stands: it was already compiled and journalled.
            let reason = format!("the repair attempt failed: {}.", failure.envelope_code());
            keep_first_build(turn, &rejection, context.first, &reason);
            return Ok(());
        }
    };
    let prose = strip_design_json_block(&repair.reply);
    let mut second = DesignTurn {
        text: prose.clone(),
        design: None,
        design_id: None,
        design_requested: true,
        missing: Vec::new(),
        model: Some(repair.model.clone()),
    };
    if !has_design_candidate(&repair.reply) {
        // The repaired reply asks for values instead of a design: the honest
        // state of the first attempt stays, and the turn says why.
        keep_first_build(
            turn,
            &rejection,
            context.first,
            "the repair attempt did not produce a buildable part.",
        );
        return Ok(());
    }
    let intent = match validate_reply(backend, &repair.reply).await {
        Ok(intent) => intent,
        Err(error) => {
            let reason = format!(
                "the reply after the build repair was not a valid design ({}).",
                error.chars().take(512).collect::<String>()
            );
            keep_first_build(turn, &rejection, context.first, &reason);
            return Ok(());
        }
    };
    match finish_design(backend, &intent, context.cad_provider, &mut second).await? {
        BuildOutcome::Built(design) => {
            // `second.text` already carries the second attempt's summary, and
            // that summary says "B-Rep NOT verified" when the second design is
            // still not verified: the turn never claims more than it has.
            let design_id = design
                .get("design_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            turn.text = join_text(&[&rejection, &second.text]);
            turn.design = Some(design);
            turn.design_id = design_id;
            turn.model = Some(repair.model.clone());
            Ok(())
        }
        BuildOutcome::Refused(failure) => {
            let reason = format!("the repair attempt was refused: {}", failure.summary());
            keep_first_build(turn, &rejection, context.first, &reason);
            Ok(())
        }
        BuildOutcome::NoProvider => Ok(()),
    }
}

/// Keep the honest state of the first attempt when the repair did not build.
///
/// A part that did build stays the turn's design, with its own "NOT verified"
/// verdict. When nothing built at all, the turn says so in the house wording.
fn keep_first_build(turn: &mut DesignTurn, rejection: &str, first: &BuildOutcome, reason: &str) {
    match first {
        BuildOutcome::Built(_) => {
            turn.text = join_text(&[rejection, &turn.text, reason]);
        }
        _ => {
            turn.text = join_text(&[rejection, &format!("No part was built: {reason}")]);
        }
    }
}

/// Validate one model reply with the worker.
async fn validate_reply(backend: &dyn GeometryBackend, text: &str) -> Result<Value, String> {
    let mut body = Map::new();
    body.insert("text".to_owned(), Value::String(text.to_owned()));
    let payload = serde_json::to_vec(&Value::Object(body)).unwrap_or_default();
    let response = backend
        .intent_validate(&payload, Some("application/json"))
        .await
        .map_err(|error| error.to_string())?;
    if !response.is_success() {
        let parsed = response.json();
        let detail = parsed
            .as_ref()
            .and_then(|value| value.get("details"))
            .and_then(Value::as_array)
            .and_then(|entries| entries.first())
            .and_then(Value::as_str)
            .unwrap_or("the model reply did not contain a valid DesignIntent");
        return Err(detail.to_owned());
    }
    let parsed = response
        .json()
        .ok_or_else(|| "the worker returned an unreadable intent".to_owned())?;
    parsed
        .get("intent")
        .cloned()
        .ok_or_else(|| "the worker returned no intent".to_owned())
}

/// Compile the design and record it on the turn.
///
/// The selected ADR-0011 provider is passed to the worker unchanged, and the
/// worker alone decides whether it can build on this host. A refusal is
/// relayed in the source's own chat wording; the front door never refuses a
/// provider itself and never falls back to another one.
///
/// The outcome is returned as well as recorded: the turn needs to know
/// whether the part built, and whether it verified, before it can decide to
/// spend its repair call on the build.
async fn finish_design(
    backend: &dyn GeometryBackend,
    intent: &Value,
    cad_provider: Option<&str>,
    turn: &mut DesignTurn,
) -> Result<BuildOutcome, DesignTurnError> {
    let Some(cad_provider) = cad_provider else {
        let providers = available_cad_providers(backend).await;
        turn.text = join_text(&[&turn.text, &provider_choice_message(&providers)]);
        return Ok(BuildOutcome::NoProvider);
    };
    let outcome = compile_design(backend, intent, cad_provider)
        .await
        .map_err(DesignTurnError::Backend)?;
    match outcome {
        Ok(design) => {
            let design_id = design
                .get("design_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let summary = measured_summary(&design);
            turn.text = join_text(&[&turn.text, &summary]);
            turn.design = Some(design.clone());
            turn.design_id = design_id;
            Ok(BuildOutcome::Built(design))
        }
        Err(failure) => {
            turn.text = join_text(&[&turn.text, &failure.chat_message(cad_provider)]);
            Ok(BuildOutcome::Refused(failure))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_missing_value_reply_names_the_family_and_the_values() {
        let parsed = ParsedIntent {
            family: Some("l_bracket".to_owned()),
            intent: None,
            missing: vec!["width (mm)".to_owned(), "depth (mm)".to_owned()],
            notes: Vec::new(),
        };
        assert_eq!(
            design_reply_text(&parsed),
            "Local helper. I can draft a L-bracket, but I need these declared values: width (mm), depth (mm). No design was compiled."
        );
    }

    #[test]
    fn the_complete_intent_reply_mentions_the_compiler() {
        let parsed = ParsedIntent {
            family: Some("stepped_shaft".to_owned()),
            intent: Some(Value::Object(Map::new())),
            missing: Vec::new(),
            notes: vec!["a note".to_owned()],
        };
        assert_eq!(
            design_reply_text(&parsed),
            "Local helper. Design intent parsed from declared values. a note Compiling and measuring with OpenCASCADE."
        );
    }

    #[test]
    fn the_fenced_block_is_stripped_from_the_prose() {
        let reply = "Here you go.\n```json\n{\"template\": null}\n```\nDone.";
        assert_eq!(strip_design_json_block(reply), "Here you go.\n\nDone.");
        assert_eq!(strip_design_json_block(""), "Design intent received.");
    }

    #[test]
    fn the_measured_summary_matches_the_source_shape() {
        let design = serde_json::json!({
            "design_id": "abc",
            "label": "Bracket",
            "verified": true,
            "measured_by": "OpenCascade (opt-in)",
            "measured": {
                "volume_mm3": 35571.416047,
                "surface_area_mm2": 1234.5,
                "size_mm": [60.0, 45.0, 40.0]
            }
        });
        assert_eq!(
            measured_summary(&design),
            "Bracket: B-Rep verified; measured by OpenCascade (opt-in). Measured volume 35571.416 mm^3, surface area 1234.500 mm^2, envelope 60.000 x 45.000 x 40.000 mm. design_id abc."
        );
    }

    #[test]
    fn a_provider_without_numbers_never_gets_a_substitute() {
        let design = serde_json::json!({
            "design_id": "abc",
            "label": "Bracket",
            "verified": false,
            "measured_by": "FreeCAD",
            "measured": {
                "volume_mm3": null,
                "surface_area_mm2": null,
                "size_mm": []
            }
        });
        assert_eq!(
            measured_summary(&design),
            "Bracket: B-Rep NOT verified; measured by FreeCAD. The provider did not report volume, surface area, or envelope; no number is substituted. design_id abc."
        );
    }

    #[test]
    fn the_instruction_matches_the_contract_crate() {
        #[cfg(feature = "native-geometry")]
        assert_eq!(
            DESIGN_INTENT_INSTRUCTION,
            kala_design_intent::DESIGN_INTENT_INSTRUCTION
        );
        assert!(
            DESIGN_INTENT_INSTRUCTION
                .contains("When no template matches, author a ConstructionScript")
        );
        assert!(DESIGN_INTENT_INSTRUCTION.contains("a rectangle's x/y is its lower-left corner"));
        assert!(DESIGN_INTENT_INSTRUCTION.contains("a circle's x/y is its centre"));
        assert!(
            DESIGN_INTENT_INSTRUCTION
                .contains("Every pocket must cut into material that its own component already has")
        );
        assert!(DESIGN_INTENT_INSTRUCTION.len() < 6_000);
    }

    #[test]
    fn the_repair_prompt_carries_the_error_and_the_failing_field_schema() {
        let schema = r##"{"properties":{"script":{"anyOf":[{"$ref":"#/$defs/ConstructionScript"},{"type":"null"}]}},"$defs":{"ConstructionScript":{"type":"object","properties":{"summary":{"type":"string"},"components":{"type":"array","items":{"type":"string"}}}}}}"##;
        let text = repair_prompt(
            "plate 80 mm",
            "script.components: Field required",
            schema,
            "```json\n{\"script\": {}}\n```",
        );
        assert!(text.contains("script.components: Field required"));
        assert!(text.contains("Schema for script.components"));
        assert!(text.contains("\"items\""));
        assert!(text.contains("Keep every value the user stated"));
        let nested = repair_prompt(
            "plate 80 mm",
            "script.components.0: Input should be a valid string",
            schema,
            "```json\n{\"script\": {}}\n```",
        );
        assert!(nested.contains("Schema for script.components.0"));
        assert!(nested.contains("\"type\": \"string\""));
        let root = repair_prompt(
            "plate 80 mm",
            "a design intent must carry exactly one",
            schema,
            "```json\n{\"script\": {}}\n```",
        );
        assert!(root.contains("the failing field"));
        assert!(root.contains("\"properties\""));
    }

    #[test]
    fn the_repair_prompt_carries_the_instruction_the_schema_and_the_previous_json() {
        // Every model call is stateless. Without the instruction, the full
        // schema and its own previous JSON the repair re-authors the design
        // blind: it invents a different shape and loses the stated values.
        let schema = r##"{"type":"object","required":["script"],"properties":{"script":{"type":"object"}}}"##;
        let previous =
            "Here is the part.\n\n```json\n{\"script\": {\"operation\": \"plate\"}}\n```";
        let text = repair_prompt(
            "aluminium mounting plate 80 x 40 x 8 mm",
            "script: Field required",
            schema,
            previous,
        );
        assert!(
            text.starts_with(DESIGN_INTENT_INSTRUCTION),
            "the repair prompt opens with the same instruction the first call carried: {text}"
        );
        assert!(
            text.contains("When no template matches, author a ConstructionScript"),
            "{text}"
        );
        assert!(text.contains("Worked examples"), "{text}");
        assert!(
            text.contains(&format!("DesignIntent JSON Schema:\n{schema}")),
            "the repair prompt carries the FULL schema, not only an excerpt: {text}"
        );
        assert!(
            text.contains("The JSON block you sent:\n{\"script\": {\"operation\": \"plate\"}}"),
            "the repair prompt carries the JSON the model itself sent: {text}"
        );
        assert!(text.contains("script: Field required"), "{text}");
        assert!(text.contains("Keep every value the user stated"), "{text}");
        assert!(
            text.contains("If a required value is missing, reply in prose"),
            "{text}"
        );
        assert!(
            text.contains("aluminium mounting plate 80 x 40 x 8 mm"),
            "{text}"
        );
        // The order of the prompt is the order of the facts the model needs.
        let instruction = text.find(DESIGN_INTENT_INSTRUCTION).expect("instruction");
        let schema_at = text.find("DesignIntent JSON Schema:").expect("schema");
        let previous_at = text.find("The JSON block you sent:").expect("previous");
        let error_at = text.find("could not be validated").expect("error");
        let request_at = text.find("User request:").expect("request");
        assert!(instruction < schema_at, "{text}");
        assert!(schema_at < previous_at, "{text}");
        assert!(previous_at < error_at, "{text}");
        assert!(error_at < request_at, "{text}");
    }

    #[test]
    fn the_repair_prompt_bounds_the_previous_json() {
        let schema = r##"{"type":"object"}"##;
        let huge = format!(
            "```json\n{{\"script\": \"{}\"}}\n```",
            "x".repeat(MAX_REPAIR_PREVIOUS_CHARS + 5_000)
        );
        let text = repair_prompt("plate 80 mm", "script: Field required", schema, &huge);
        assert!(text.contains("The JSON block you sent:"), "{text}");
        let carried = text
            .split_once("The JSON block you sent:\n")
            .expect("the previous JSON section")
            .1
            .split_once("\n\n")
            .expect("the section ends")
            .0;
        assert!(
            carried.chars().count() <= MAX_REPAIR_PREVIOUS_CHARS,
            "the previous JSON is bounded: {} characters",
            carried.chars().count()
        );
    }

    #[test]
    fn the_previous_json_block_is_extracted_from_the_reply() {
        assert_eq!(
            extract_design_json_block("prose\n```json\n{\"script\": {}}\n```"),
            Some("{\"script\": {}}".to_owned())
        );
        assert_eq!(
            extract_design_json_block("prose {\"script\": {}} more prose"),
            Some("{\"script\": {}}".to_owned())
        );
        assert_eq!(
            extract_design_json_block("```json\n{\"a\": {\"b\": 1}}\n```"),
            Some("{\"a\": {\"b\": 1}}".to_owned())
        );
        assert_eq!(extract_design_json_block("no json here"), None::<String>);
        // A reply whose JSON is not a balanced object still shows its own
        // text, so the model sees exactly what it wrote.
        let text = repair_prompt(
            "plate 80 mm",
            "script: Field required",
            r##"{"type":"object"}"##,
            "```json\n{\"script\": {\n```",
        );
        assert!(text.contains("Your previous reply:"), "{text}");
        assert!(text.contains("{\"script\": {"), "{text}");
    }

    #[test]
    fn the_honest_text_names_both_validation_errors() {
        let reason = validation_repair_failed_reason(
            "script: Field required",
            "template: Input should be a valid dictionary",
        );
        assert_eq!(
            reason,
            "the first reply was not a valid design (script: Field required), and the reply after one repair attempt was not a valid design either (template: Input should be a valid dictionary)."
        );
        // Each error is bounded.
        let long = "z".repeat(MAX_REPAIR_ERROR_CHARS + 100);
        let bounded = validation_repair_failed_reason(&long, &long);
        assert_eq!(
            bounded
                .chars()
                .filter(|character| *character == 'z')
                .count(),
            2 * MAX_REPAIR_ERROR_CHARS,
            "each of the two errors is bounded: {bounded}"
        );
    }

    #[test]
    fn a_reply_without_a_candidate_is_not_repaired() {
        assert!(!has_design_candidate("What is the plate thickness?"));
        assert!(has_design_candidate("```json\n{}\n```"));
        assert!(has_design_candidate("Here it is: {\"script\": null}"));
        assert_eq!(
            no_build_text(
                "Design intent received.",
                "the reply did not include a design."
            ),
            "No part was built: the reply did not include a design."
        );
        assert_eq!(
            no_build_text("What thickness?", "the reply did not include a design."),
            "What thickness? No part was built: the reply did not include a design."
        );
    }

    #[test]
    fn the_journal_payload_never_carries_the_prompt() {
        let digest = Sha256::digest("hello".as_bytes());
        let expected: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(expected.len(), 64);
    }

    fn refusal_response(status: u16, body: Value) -> crate::geometry_backend::WorkerResponse {
        crate::geometry_backend::WorkerResponse {
            status,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body: serde_json::to_vec(&body).unwrap(),
        }
    }

    #[test]
    fn a_provider_refusal_is_relayed_in_the_source_wording() {
        // The exact envelope the worker's public design route answers with for
        // Solid Edge on Linux: the message, the reason and the availability
        // list. The source's chat route renders the same three facts.
        let response = refusal_response(
            503,
            serde_json::json!({
                "code": "provider_unavailable",
                "message": "the solidedge CAD provider is not available",
                "details": [
                    "the solidedge CAD provider is not available",
                    "reason: Solid Edge build requires the Windows bridge (not reachable); this host is linux, so the ADR-0008 bridge cannot run here",
                    "available: opencascade, freecad"
                ]
            }),
        );
        let failure = DesignFailure::from_response(&response);
        assert!(failure.is_provider_unavailable());
        assert_eq!(
            failure.chat_message("solidedge"),
            "The solidedge CAD provider is not available: Solid Edge build requires the Windows bridge (not reachable); this host is linux, so the ADR-0008 bridge cannot run here. Available: opencascade, freecad. No design was built."
        );
    }

    #[test]
    fn a_refusal_without_a_reason_detail_falls_back_to_the_message() {
        let response = refusal_response(
            503,
            serde_json::json!({
                "code": "provider_unavailable",
                "message": "the freecad CAD provider is not available",
                "details": [
                    "the freecad CAD provider is not available",
                    "available: none"
                ]
            }),
        );
        assert_eq!(
            DesignFailure::from_response(&response).chat_message("freecad"),
            "The freecad CAD provider is not available: the freecad CAD provider is not available. Available: none. No design was built."
        );
    }

    #[test]
    fn another_refusal_keeps_the_front_door_summary() {
        let response = refusal_response(
            422,
            serde_json::json!({
                "code": "design_rejected",
                "message": "the design could not be compiled or measured",
                "details": ["the profile is not supported"]
            }),
        );
        let failure = DesignFailure::from_response(&response);
        assert!(!failure.is_provider_unavailable());
        assert_eq!(
            failure.summary(),
            "design_rejected: the design could not be compiled or measured (the profile is not supported)"
        );
        assert_eq!(
            failure.chat_message("freecad"),
            "The design could not be built: design_rejected: the design could not be compiled or measured (the profile is not supported)."
        );
    }

    #[test]
    fn an_unreadable_refusal_keeps_the_status_and_the_fallback_code() {
        let response = crate::geometry_backend::WorkerResponse {
            status: 502,
            headers: Vec::new(),
            body: b"not json".to_vec(),
        };
        let failure = DesignFailure::from_response(&response);
        assert_eq!(failure.status, 502);
        assert_eq!(failure.code, "design_unavailable");
        assert_eq!(failure.message, "the design could not be built");
        assert!(failure.details.is_empty());
    }

    #[test]
    fn only_a_refusal_about_the_script_earns_a_build_repair() {
        let refused = |status: u16, code: &str| DesignFailure {
            status,
            code: code.to_owned(),
            message: "the design could not be built".to_owned(),
            details: vec!["the profile is not supported".to_owned()],
        };
        assert!(refused(422, "design_rejected").is_script_refusal());
        assert!(refused(422, "invalid_request").is_script_refusal());
        assert!(!refused(503, "provider_unavailable").is_script_refusal());
        assert!(!refused(503, "design_unavailable").is_script_refusal());
        assert!(!refused(404, "design_not_found").is_script_refusal());
    }

    #[test]
    fn the_build_repair_prompt_carries_the_script_and_the_feedback() {
        let schema = r##"{"type":"object","properties":{"script":{"type":"object","properties":{"summary":{"type":"string"}}}}}"##;
        let feedback = BuildFeedback {
            script: r#"{"components":[]}"#.to_owned(),
            failed_checks: vec!["the check 'cuts_effective' (expected 1, measured 0)".to_owned()],
            warnings: vec![
                "component 'plate': pocket at step index 3 on XY removed no material".to_owned(),
            ],
            refusal: None,
        };
        let text = build_repair_prompt("plate 80 x 40 x 8", &feedback.text(), schema);
        assert!(text.contains("the build was rejected"), "{text}");
        assert!(text.contains("cuts_effective"), "{text}");
        assert!(text.contains("removed no material"), "{text}");
        assert!(text.contains(r#"{"components":[]}"#), "{text}");
        assert!(text.contains("every cut removes material"), "{text}");
        assert!(text.contains("plate 80 x 40 x 8"), "{text}");
        assert!(text.contains("Schema for script"), "{text}");
        let fixed = text.len() - "plate 80 x 40 x 8".len();
        assert!(
            fixed < MAX_BUILD_REPAIR_PROMPT_CHARS,
            "the whole prompt stays bounded: {fixed} characters"
        );
    }

    #[test]
    fn the_build_repair_prompt_still_carries_the_design_instruction() {
        // Every model call is stateless: without the instruction the repair
        // call no longer sees the supported subset, the plane rule or the
        // worked examples.
        let schema = r##"{"type":"object","properties":{"script":{"type":"object"}}}"##;
        let text = build_repair_prompt(
            "plate 80 x 40 x 8",
            "the check 'cuts_effective' failed",
            schema,
        );
        assert!(
            text.starts_with(DESIGN_INTENT_INSTRUCTION),
            "the repair prompt opens with the same instruction the first call carried"
        );
        assert!(
            text.contains("When no template matches, author a ConstructionScript"),
            "{text}"
        );
        assert!(
            text.contains("Never invent a dimension the user did not state"),
            "{text}"
        );
        assert!(text.contains("Worked examples"), "{text}");
        // The rest of the prompt is unchanged.
        assert!(
            text.contains("Your previous reply was built, but the build was rejected"),
            "{text}"
        );
        assert!(text.contains("Schema for script"), "{text}");
    }

    #[test]
    fn a_failed_repair_call_names_the_envelope_code() {
        assert_eq!(
            ProviderFailure::Timeout("slow".to_owned()).envelope_code(),
            "provider_timeout"
        );
        assert_eq!(
            ProviderFailure::Network("down".to_owned()).envelope_code(),
            "provider_network_error"
        );
        assert_eq!(
            ProviderFailure::Protocol("garbage".to_owned()).envelope_code(),
            "provider_protocol_error"
        );
        assert_eq!(
            ProviderFailure::Unavailable("off".to_owned()).envelope_code(),
            "provider_unavailable"
        );
        assert_eq!(
            ProviderFailure::Subprocess("crashed".to_owned()).envelope_code(),
            "provider_subprocess_error"
        );
        assert_eq!(
            ProviderFailure::configuration("no key").envelope_code(),
            "provider_not_configured"
        );
    }

    #[test]
    fn the_build_feedback_reports_the_failed_rows_and_the_warnings() {
        let intent = serde_json::json!({
            "script": {"summary": "plate", "components": []}
        });
        let design = serde_json::json!({
            "verified": false,
            "script": {"summary": "plate", "components": []},
            "checks": [
                {"id": "brep_valid", "passed": true, "expected": true, "measured": true},
                {"id": "cuts_effective", "passed": false, "expected": 1.0, "measured": 0.0}
            ],
            "warnings": ["component 'plate': pocket at step index 3 on XY removed no material"]
        });
        let feedback = build_feedback(&intent, &BuildOutcome::Built(design))
            .expect("an unverified design earns a build repair");
        assert_eq!(
            feedback.failed_checks,
            vec!["the check 'cuts_effective' (expected 1, measured 0)"]
        );
        assert_eq!(
            feedback.warnings,
            vec!["component 'plate': pocket at step index 3 on XY removed no material"]
        );
        assert!(
            feedback.reason().contains("cuts_effective"),
            "{}",
            feedback.reason()
        );
        assert!(
            feedback.reason().contains("removed no material"),
            "{}",
            feedback.reason()
        );
    }

    #[test]
    fn a_verified_design_and_a_foreign_refusal_earn_no_build_repair() {
        let intent = serde_json::json!({"script": {"components": []}});
        let verified = serde_json::json!({
            "verified": true,
            "checks": [{"id": "brep_valid", "passed": true}],
            "warnings": []
        });
        assert!(build_feedback(&intent, &BuildOutcome::Built(verified)).is_none());
        assert!(build_feedback(&intent, &BuildOutcome::NoProvider).is_none());
        let unavailable = DesignFailure {
            status: 503,
            code: "provider_unavailable".to_owned(),
            message: "the solidedge CAD provider is not available".to_owned(),
            details: vec!["reason: no bridge".to_owned()],
        };
        assert!(build_feedback(&intent, &BuildOutcome::Refused(unavailable)).is_none());
    }

    #[test]
    fn a_refused_script_carries_the_refusal_and_the_script() {
        let intent = serde_json::json!({
            "script": {"summary": "plate", "components": []}
        });
        let refused = DesignFailure {
            status: 422,
            code: "design_rejected".to_owned(),
            message: "the design could not be compiled or measured".to_owned(),
            details: vec!["unsupported by the Studio OCP compiler v1: pocket".to_owned()],
        };
        let feedback = build_feedback(&intent, &BuildOutcome::Refused(refused))
            .expect("a refusal about the script earns a build repair");
        assert_eq!(
            feedback.refusal.as_deref(),
            Some(
                "design_rejected: the design could not be compiled or measured \
                 (unsupported by the Studio OCP compiler v1: pocket)"
            )
        );
        assert!(
            feedback.script.contains("\"summary\":\"plate\""),
            "{}",
            feedback.script
        );
        assert!(
            feedback
                .reason()
                .contains("unsupported by the Studio OCP compiler"),
            "{}",
            feedback.reason()
        );
    }
}
