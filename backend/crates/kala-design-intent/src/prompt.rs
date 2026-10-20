//! The strict local design-prompt parser and its missing-value replies.
//!
//! Port of the design half of `kala_agent.studio.providers.local`. The parser
//! is deliberately narrow: a family keyword must be present, every required
//! value must be stated (or be a documented canonical template default), and
//! the helper never guesses a dimension to fill a gap.

use std::fmt;
use std::sync::LazyLock;

use num_bigint::BigInt;
use regex::Regex;

use crate::format::ascii_digits;
use crate::intent::{
    DEFAULT_BRACKET_DEPTH_MM, DEFAULT_BRACKET_WIDTH_MM, DesignIntent, DesignIntentValidationError,
    HoleCount, HousingCall, LbracketCall, SteppedShaftCall, TemplateCall,
};
use crate::pytext::{normalise_lowered, python_truncate};

/// Largest reply the local helper returns.
pub const MAX_RESPONSE_LENGTH: usize = 1024;

/// ISO 273 medium clearance holes for metric bolts: M5 -> 5.5 mm.
pub const M_SIZE_CLEARANCE: &[(f64, f64)] = &[
    (3.0, 3.4),
    (4.0, 4.5),
    (5.0, 5.5),
    (6.0, 6.6),
    (8.0, 9.0),
    (10.0, 11.0),
    (12.0, 13.5),
    (16.0, 17.5),
    (20.0, 22.0),
];

/// Return the ISO 273 medium clearance diameter for a nominal M size.
pub fn iso_273_medium_clearance(nominal_mm: f64) -> Option<f64> {
    M_SIZE_CLEARANCE
        .iter()
        .find(|(nominal, _)| *nominal == nominal_mm)
        .map(|(_, clearance)| *clearance)
}

const NUMBER: &str = r"\d+(?:\.\d+)?";

static WALL_MM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"({NUMBER})\s*mm\s*(?:thick\s+)?wall")).expect("valid wall pattern")
});
static WALL_LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"wall(?:\s+thickness)?\s*(?:of|is|:|=)?\s*({NUMBER})\s*mm"
    ))
    .expect("valid wall label pattern")
});
static BORE_MM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"({NUMBER})\s*mm\s*(?:centered|centred|center|centre)?\s*bore"
    ))
    .expect("valid bore pattern")
});
static BORE_LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"bore\s*(?:of|is|:|=)?\s*({NUMBER})\s*mm"))
        .expect("valid bore label pattern")
});
static TRIPLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"({NUMBER})\s*[x×]\s*({NUMBER})\s*[x×]\s*({NUMBER})\s*mm"
    ))
    .expect("valid triple pattern")
});
static LEGS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"legs?\s*(?:of|are|:)?\s*({NUMBER})\s*mm\s*(?:and|,)\s*({NUMBER})\s*mm"
    ))
    .expect("valid legs pattern")
});
static WIDTH_MM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"width\s*(?:of|is|:|=)?\s*({NUMBER})\s*mm")).expect("valid width pattern")
});
static WIDTH_AFTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"({NUMBER})\s*mm\s*(?:wide|across)")).expect("valid width-after pattern")
});
static DEPTH_MM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"depth\s*(?:of|is|:|=)?\s*({NUMBER})\s*mm")).expect("valid depth pattern")
});
static DEPTH_AFTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"({NUMBER})\s*mm\s*(?:deep|long)")).expect("valid depth-after pattern")
});
static HEIGHT_MM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"({NUMBER})\s*mm\s*mounting\s+plate"))
        .expect("valid mounting-plate pattern")
});
static HEIGHT_AFTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"({NUMBER})\s*mm\s*(?:tall|high)")).expect("valid height-after pattern")
});
static HEIGHT_LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?:height|tall|high)\s*(?:of|is|:|=)?\s*({NUMBER})\s*mm"
    ))
    .expect("valid height label pattern")
});
static METRIC_HOLES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"(\d+)\s*[x×]\s*m\s*({NUMBER})")).expect("valid metric hole pattern")
});
static EXPLICIT_HOLES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(\d+)\s*[x×]\s*(?:ø|dia(?:meter)?\s*)?({NUMBER})\s*mm\s*(?:corner\s+)?(?:mounting\s+)?(?:clearance\s+)?holes?"
    ))
    .expect("valid explicit hole pattern")
});
static COUNT_ONLY_HOLES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(\d+)\s+(?:corner\s+)?(?:mounting\s+)?holes?").expect("valid hole count pattern")
});
static WALL_AFTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"({NUMBER})\s*mm\s*(?:thick\s+)?walls?"))
        .expect("valid wall-after pattern")
});
static SHAFT_MAIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"({NUMBER})\s*mm\s*(?:diameter|dia)\s*(?:by|and|,|[×x])\s*({NUMBER})\s*mm\s*(?:long|length)"
    ))
    .expect("valid shaft pattern")
});
static SHAFT_MAIN_SHORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"({NUMBER})\s*mm\s*(?:diameter|dia)\s*(?:by|and|,|[×x])\s*({NUMBER})\s*mm"
    ))
    .expect("valid short shaft pattern")
});
static SHAFT_SHOULDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"({NUMBER})\s*mm\s*(?:diameter\s*)?shoulder\s*(?:of\s*)?({NUMBER})\s*mm\s*(?:long|length)"
    ))
    .expect("valid shoulder pattern")
});

/// The template family a prompt asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesignFamily {
    /// L-bracket.
    LBracket,
    /// Stepped shaft.
    SteppedShaft,
    /// Rectangular housing envelope.
    HousingEnvelope,
}

impl DesignFamily {
    /// The Python family id.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LBracket => "l_bracket",
            Self::SteppedShaft => "stepped_shaft",
            Self::HousingEnvelope => "housing_envelope",
        }
    }

    /// The readable family label used in replies.
    pub const fn label(self) -> &'static str {
        match self {
            Self::LBracket => "L-bracket",
            Self::SteppedShaft => "stepped shaft",
            Self::HousingEnvelope => "rectangular housing envelope",
        }
    }
}

impl fmt::Display for DesignFamily {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The strict design reading of one local-helper prompt.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignPrompt {
    family: DesignFamily,
    intent: Option<DesignIntent>,
    missing: Vec<String>,
    notes: Vec<String>,
}

impl DesignPrompt {
    /// The template family.
    pub fn family(&self) -> DesignFamily {
        self.family
    }

    /// The parsed intent, present only when every required value was stated.
    pub fn intent(&self) -> Option<&DesignIntent> {
        self.intent.as_ref()
    }

    /// The values the operator still has to state.
    pub fn missing(&self) -> &[String] {
        &self.missing
    }

    /// The parser notes (defaults, ISO 273 mappings).
    pub fn notes(&self) -> &[String] {
        &self.notes
    }
}

/// A parsed prompt whose declared values do not satisfy the intent contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignPromptError {
    error: DesignIntentValidationError,
}

impl DesignPromptError {
    /// The underlying validation issues.
    pub fn issues(&self) -> &[crate::DesignIssue] {
        self.error.issues()
    }
}

impl fmt::Display for DesignPromptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.error.message())
    }
}

impl std::error::Error for DesignPromptError {}

/// Return the template family a prompt asks for, or `None`.
///
/// The match is deliberately narrow: a family keyword must be present, not
/// merely a loose noun. A cylindrical housing is not the rectangular
/// `housing_envelope` template, so it stays an analytic reply.
/// Python `str.isalnum() || '_'`, the character class behind `\b`/`\w`.
fn python_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn boundary_before(text: &str, index: usize) -> bool {
    text[..index]
        .chars()
        .next_back()
        .is_none_or(|character| !python_word_character(character))
}

fn boundary_after(text: &str, index: usize) -> bool {
    text[index..]
        .chars()
        .next()
        .is_none_or(|character| !python_word_character(character))
}

fn bounded_at(text: &str, start: usize, end: usize) -> bool {
    boundary_before(text, start) && boundary_after(text, end)
}

/// Python `re.search(r"\b<needle>\b", text)`.
fn contains_bounded(text: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(offset) = text[from..].find(needle) {
        let start = from + offset;
        let end = start + needle.len();
        if bounded_at(text, start, end) {
            return true;
        }
        from = start + 1;
        if from >= text.len() {
            break;
        }
    }
    false
}

fn matches_l_bracket(text: &str) -> bool {
    let mut from = 0;
    while let Some(offset) = text[from..].find("bracket") {
        let start = from + offset;
        let mut end = start + "bracket".len();
        if text[end..].starts_with('s') {
            end += 1;
        }
        // First alternative: l[\s-]?brackets?
        if start >= 1 {
            let before = text[..start].chars().next_back().unwrap_or(' ');
            let l_index = if before == 'l' {
                Some(start - 1)
            } else if before == ' ' || before == '-' {
                text[..start - 1]
                    .chars()
                    .next_back()
                    .filter(|character| *character == 'l')
                    .map(|_| start - 2)
            } else {
                None
            };
            if let Some(l_index) = l_index {
                if bounded_at(text, l_index, end) {
                    return true;
                }
            }
        }
        // Second alternative: brackets?
        if bounded_at(text, start, end) {
            return true;
        }
        from = start + 1;
        if from >= text.len() {
            break;
        }
    }
    false
}

fn matches_stepped_shaft(text: &str) -> bool {
    let mut from = 0;
    while let Some(offset) = text[from..].find("stepped") {
        let start = from + offset;
        let after = start + "stepped".len();
        let rest = &text[after..];
        let separator = rest
            .chars()
            .next()
            .filter(|character| *character == ' ' || *character == '-')
            .map_or(0, char::len_utf8);
        let shaft_start = after + separator;
        if text[shaft_start..].starts_with("shaft") {
            let mut end = shaft_start + "shaft".len();
            if text[end..].starts_with('s') {
                end += 1;
            }
            if bounded_at(text, start, end) {
                return true;
            }
        }
        from = start + 1;
        if from >= text.len() {
            break;
        }
    }
    false
}

/// Return the template family a prompt asks for, or `None`.
///
/// The match is deliberately narrow: a family keyword must be present, not
/// merely a loose noun. A cylindrical housing is not the rectangular
/// `housing_envelope` template, so it stays an analytic reply. Word boundaries
/// use Python's `str.isalnum()` class rather than the regex crate's broader
/// `\w`, which also counts combining marks.
pub fn detect_design_family(prompt: &str) -> Option<DesignFamily> {
    let lowered = normalise_lowered(prompt);
    if lowered.is_empty() {
        return None;
    }
    if matches_l_bracket(&lowered) {
        return Some(DesignFamily::LBracket);
    }
    if matches_stepped_shaft(&lowered) {
        return Some(DesignFamily::SteppedShaft);
    }
    if (contains_bounded(&lowered, "housing") || contains_bounded(&lowered, "enclosure"))
        && !(contains_bounded(&lowered, "cylindrical")
            || contains_bounded(&lowered, "round")
            || contains_bounded(&lowered, "circular"))
    {
        return Some(DesignFamily::HousingEnvelope);
    }
    None
}

fn mm_value(pattern: &Regex, text: &str) -> Option<f64> {
    pattern
        .captures(text)
        .and_then(|captures| captures.get(1))
        .and_then(|value| crate::format::parse_python_float(value.as_str()))
}

fn tuple_pair(pattern: &Regex, text: &str) -> Option<(f64, f64)> {
    let captures = pattern.captures(text)?;
    let first = crate::format::parse_python_float(captures.get(1)?.as_str())?;
    let second = crate::format::parse_python_float(captures.get(2)?.as_str())?;
    Some((first, second))
}

fn tuple_triple(pattern: &Regex, text: &str) -> Option<(f64, f64, f64)> {
    let captures = pattern.captures(text)?;
    let first = crate::format::parse_python_float(captures.get(1)?.as_str())?;
    let second = crate::format::parse_python_float(captures.get(2)?.as_str())?;
    let third = crate::format::parse_python_float(captures.get(3)?.as_str())?;
    Some((first, second, third))
}

fn detect_material(lowered: &str) -> Option<String> {
    for (keyword, material) in [
        ("stainless steel", "stainless steel"),
        ("aluminium", "aluminium"),
        ("aluminum", "aluminium"),
        ("steel", "steel"),
        ("brass", "brass"),
        ("titanium", "titanium"),
        ("nylon", "nylon"),
        ("plastic", "plastic"),
    ] {
        if contains_bounded(lowered, keyword) {
            return Some(material.to_owned());
        }
    }
    None
}

/// Python applies `\b` after `holes?`; the regex crate cannot express Python's
/// `str.isalnum()` boundary without look-around, so filter matches.
fn count_only_captures(text: &str) -> Option<regex::Captures<'_>> {
    let mut from = 0;
    while let Some(captures) = COUNT_ONLY_HOLES.captures_at(text, from) {
        let matched = captures.get(0).expect("whole match");
        if boundary_after(text, matched.end()) {
            return Some(captures);
        }
        from = matched.start() + 1;
        if from >= text.len() {
            break;
        }
    }
    None
}

fn is_hole_count(count: &BigInt) -> bool {
    count == &BigInt::from(0) || count == &BigInt::from(2) || count == &BigInt::from(4)
}

fn hole_count_of(count: &BigInt) -> HoleCount {
    if count == &BigInt::from(2) {
        HoleCount::Two
    } else if count == &BigInt::from(4) {
        HoleCount::Four
    } else {
        HoleCount::Zero
    }
}

/// Strictly parse one local prompt into a template call or a missing list.
pub fn parse_design_prompt(prompt: &str) -> Result<Option<DesignPrompt>, DesignPromptError> {
    let Some(family) = detect_design_family(prompt) else {
        return Ok(None);
    };
    let lowered = normalise_lowered(prompt);
    match family {
        DesignFamily::LBracket => parse_lbracket(&lowered),
        DesignFamily::SteppedShaft => parse_stepped_shaft(&lowered),
        DesignFamily::HousingEnvelope => parse_housing(&lowered),
    }
}

fn parse_lbracket(lowered: &str) -> Result<Option<DesignPrompt>, DesignPromptError> {
    let mut notes: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();

    let mut wall = mm_value(&WALL_MM, lowered);
    if wall.is_none() {
        wall = mm_value(&WALL_LABEL, lowered);
    }
    let mut bore = mm_value(&BORE_MM, lowered);
    if bore.is_none() {
        bore = mm_value(&BORE_LABEL, lowered);
    }

    let mut width: Option<f64> = None;
    let mut depth: Option<f64> = None;
    let mut height: Option<f64> = None;
    if let Some((w, d, h)) = tuple_triple(&TRIPLE, lowered) {
        width = Some(w);
        depth = Some(d);
        height = Some(h);
    }
    if let Some((w, d)) = tuple_pair(&LEGS, lowered) {
        width = Some(w);
        depth = Some(d);
    }
    if width.is_none() {
        width = mm_value(&WIDTH_MM, lowered);
    }
    if width.is_none() {
        width = mm_value(&WIDTH_AFTER, lowered);
    }
    if depth.is_none() {
        depth = mm_value(&DEPTH_MM, lowered);
    }
    if depth.is_none() {
        depth = mm_value(&DEPTH_AFTER, lowered);
    }
    if height.is_none() {
        height = mm_value(&HEIGHT_MM, lowered);
    }
    if height.is_none() {
        height = mm_value(&HEIGHT_AFTER, lowered);
    }
    if height.is_none() {
        height = mm_value(&HEIGHT_LABEL, lowered);
    }

    let mut hole_count = BigInt::from(0);
    let mut hole_d: Option<f64> = None;
    let metric = METRIC_HOLES.captures(lowered);
    let explicit = EXPLICIT_HOLES.captures(lowered);
    let count_only = count_only_captures(lowered);
    if let Some(captures) = metric {
        let raw_count = captures.get(1).map(|value| value.as_str()).unwrap_or("0");
        hole_count = ascii_digits(raw_count)
            .and_then(|digits| BigInt::parse_bytes(digits.as_bytes(), 10))
            .unwrap_or_else(|| BigInt::from(0));
        let nominal = captures
            .get(2)
            .and_then(|value| crate::format::parse_python_float(value.as_str()))
            .unwrap_or(f64::NAN);
        match iso_273_medium_clearance(nominal) {
            None => missing.push(format!(
                "hole diameter (no ISO 273 medium value for M{})",
                crate::format_g(nominal)
            )),
            Some(clearance) => {
                hole_d = Some(clearance);
                notes.push(format!(
                    "{hole_count}x M{} mapped to {} mm clearance holes (ISO 273 medium).",
                    crate::format_g(nominal),
                    crate::format_g(clearance)
                ));
            }
        }
    } else if let Some(captures) = explicit {
        hole_count = captures
            .get(1)
            .and_then(|value| ascii_digits(value.as_str()))
            .and_then(|digits| BigInt::parse_bytes(digits.as_bytes(), 10))
            .unwrap_or_else(|| BigInt::from(0));
        hole_d = captures
            .get(2)
            .and_then(|value| crate::format::parse_python_float(value.as_str()));
    } else if let Some(captures) = count_only {
        hole_count = captures
            .get(1)
            .and_then(|value| ascii_digits(value.as_str()))
            .and_then(|digits| BigInt::parse_bytes(digits.as_bytes(), 10))
            .unwrap_or_else(|| BigInt::from(0));
        missing.push("hole diameter (mm) or an M-size".to_owned());
    }
    if !is_hole_count(&hole_count) {
        missing.push("2 or 4 holes".to_owned());
        hole_count = BigInt::from(0);
    }
    if hole_count != BigInt::from(0) && hole_d.is_none() && missing.is_empty() {
        missing.push("hole diameter (mm) or an M-size".to_owned());
    }

    if width.is_none()
        && depth.is_none()
        && height.is_some()
        && (wall.is_some() || bore.is_some() || hole_count != BigInt::from(0))
    {
        width = Some(DEFAULT_BRACKET_WIDTH_MM);
        depth = Some(DEFAULT_BRACKET_DEPTH_MM);
        notes.push(format!(
            "Plate width {} mm and depth {} mm are canonical template defaults; state them to override.",
            crate::format_g(DEFAULT_BRACKET_WIDTH_MM),
            crate::format_g(DEFAULT_BRACKET_DEPTH_MM)
        ));
    }

    if width.is_none() {
        missing.push("width (mm)".to_owned());
    }
    if depth.is_none() {
        missing.push("depth (mm)".to_owned());
    }
    if height.is_none() {
        missing.push("height (mm)".to_owned());
    }
    if wall.is_none() {
        missing.push("wall thickness (mm)".to_owned());
    }
    if !missing.is_empty()
        || width.is_none()
        || depth.is_none()
        || height.is_none()
        || wall.is_none()
    {
        return Ok(Some(DesignPrompt {
            family: DesignFamily::LBracket,
            intent: None,
            missing: deduplicate(missing),
            notes,
        }));
    }

    let call = LbracketCall::from_parsed(
        width.expect("present"),
        depth.expect("present"),
        height.expect("present"),
        wall.expect("present"),
        bore,
        hole_d,
        hole_count_of(&hole_count),
        None,
    )
    .map_err(|error| DesignPromptError { error })?;
    let intent = DesignIntent::from_template(
        TemplateCall::Lbracket(call),
        detect_material(lowered).as_deref(),
    )
    .map_err(|error| DesignPromptError { error })?;
    Ok(Some(DesignPrompt {
        family: DesignFamily::LBracket,
        intent: Some(intent),
        missing: Vec::new(),
        notes,
    }))
}

fn parse_stepped_shaft(lowered: &str) -> Result<Option<DesignPrompt>, DesignPromptError> {
    let notes: Vec<String> = Vec::new();
    let mut segments: Vec<(f64, f64)> = Vec::new();
    if let Some(pair) = tuple_pair(&SHAFT_MAIN, lowered) {
        segments.push(pair);
    } else if let Some(pair) = tuple_pair(&SHAFT_MAIN_SHORT, lowered) {
        segments.push(pair);
    }
    if let Some(pair) = tuple_pair(&SHAFT_SHOULDER, lowered) {
        segments.push(pair);
    }
    if segments.is_empty() {
        return Ok(Some(DesignPrompt {
            family: DesignFamily::SteppedShaft,
            intent: None,
            missing: vec!["segment diameters and lengths (mm)".to_owned()],
            notes,
        }));
    }
    let call =
        SteppedShaftCall::from_parsed(&segments).map_err(|error| DesignPromptError { error })?;
    let intent = DesignIntent::from_template(
        TemplateCall::SteppedShaft(call),
        detect_material(lowered).as_deref(),
    )
    .map_err(|error| DesignPromptError { error })?;
    Ok(Some(DesignPrompt {
        family: DesignFamily::SteppedShaft,
        intent: Some(intent),
        missing: Vec::new(),
        notes,
    }))
}

fn parse_housing(lowered: &str) -> Result<Option<DesignPrompt>, DesignPromptError> {
    let notes: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    let mut width: Option<f64> = None;
    let mut depth: Option<f64> = None;
    let mut height: Option<f64> = None;
    if let Some((w, d, h)) = tuple_triple(&TRIPLE, lowered) {
        width = Some(w);
        depth = Some(d);
        height = Some(h);
    }
    if width.is_none() {
        width = mm_value(&WIDTH_MM, lowered);
    }
    if depth.is_none() {
        depth = mm_value(&DEPTH_MM, lowered);
    }
    if height.is_none() {
        height = mm_value(&HEIGHT_LABEL, lowered);
    }
    let mut wall = mm_value(&WALL_LABEL, lowered);
    if wall.is_none() {
        wall = mm_value(&WALL_AFTER, lowered);
    }

    if width.is_none() {
        missing.push("width (mm)".to_owned());
    }
    if depth.is_none() {
        missing.push("depth (mm)".to_owned());
    }
    if height.is_none() {
        missing.push("height (mm)".to_owned());
    }
    if wall.is_none() {
        missing.push("wall thickness (mm)".to_owned());
    }
    if !missing.is_empty()
        || width.is_none()
        || depth.is_none()
        || height.is_none()
        || wall.is_none()
    {
        return Ok(Some(DesignPrompt {
            family: DesignFamily::HousingEnvelope,
            intent: None,
            missing,
            notes,
        }));
    }
    let call = HousingCall::from_parsed(
        width.expect("present"),
        depth.expect("present"),
        height.expect("present"),
        wall.expect("present"),
    )
    .map_err(|error| DesignPromptError { error })?;
    let intent = DesignIntent::from_template(
        TemplateCall::Housing(call),
        detect_material(lowered).as_deref(),
    )
    .map_err(|error| DesignPromptError { error })?;
    Ok(Some(DesignPrompt {
        family: DesignFamily::HousingEnvelope,
        intent: Some(intent),
        missing: Vec::new(),
        notes,
    }))
}

fn deduplicate(values: Vec<String>) -> Vec<String> {
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for value in values {
        if !seen.contains(&value) {
            seen.push(value.clone());
            out.push(value);
        }
    }
    out
}

/// The local helper's reply for one strict design reading.
pub fn design_prompt_reply(parsed: &DesignPrompt) -> String {
    if parsed.intent.is_none() {
        let family = parsed.family.label();
        let article = if family
            .chars()
            .next()
            .is_some_and(|first| "aeiou".contains(first.to_ascii_lowercase()))
        {
            "an"
        } else {
            "a"
        };
        let mut notes = parsed.notes.join(" ");
        if !notes.is_empty() {
            notes = format!(" {notes}");
        }
        let need = if parsed.missing.is_empty() {
            "a complete set of declared dimensions".to_owned()
        } else {
            parsed.missing.join(", ")
        };
        let reply = format!(
            "Local helper. I can draft {article} {family}, but I need these declared values: {need}.{notes} No design was compiled."
        );
        return python_truncate(&reply, MAX_RESPONSE_LENGTH);
    }
    let mut notes = parsed.notes.join(" ");
    if !notes.is_empty() {
        notes.push(' ');
    }
    let reply = format!(
        "Local helper. Design intent parsed from declared values. {notes}Compiling and measuring with OpenCASCADE."
    );
    python_truncate(&reply, MAX_RESPONSE_LENGTH)
}
