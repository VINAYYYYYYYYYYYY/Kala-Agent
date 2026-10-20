//! Unit, edge and security tests derived from the Python design suites.
//!
//! These do not need Python. The live differential lives in
//! `tests/python_differential.rs`.

use kala_design_intent::{
    Bounds, DesignFamily, DesignIntent, DesignIntentInputError, DesignIntentParseError, HoleCount,
    M_SIZE_CLEARANCE, MAX_LABEL_LENGTH, MAX_MATERIAL_LENGTH, MAX_RESPONSE_LENGTH, MAX_SEGMENTS,
    MAX_TOLERANCE_LENGTH, MAX_TOLERANCES, TemplateCall, design_intent_json_schema,
    design_intent_schema_text, design_prompt_reply, detect_design_family, format_g,
    housing_envelope, iso_273_medium_clearance, l_bracket, parse_design_intent_text,
    parse_design_prompt, stepped_shaft, strip_design_json_block,
};
use kala_design_intent::{TemplateShaftSegment, design_intent_schema_text as schema_text};

fn adr_bracket() -> kala_design_intent::TemplateResult {
    l_bracket(60.0, 45.0, 40.0, 8.0, Some(10.0), Some(5.5), 4, None).expect("ADR bracket")
}

#[test]
fn l_bracket_matches_the_adr_declared_values() {
    let result = adr_bracket();
    assert_eq!(result.label(), "L-bracket 60 x 45 x 40");
    assert_eq!(result.envelope().min_mm(), [-30.0, 0.0, 0.0]);
    assert_eq!(result.envelope().max_mm(), [30.0, 45.0, 40.0]);
    assert_eq!(
        result.tolerances(),
        [
            "bore diameter 10 mm (declared)",
            "4 x hole diameter 5.5 mm (declared)",
            "ISO 2768-m (declared)"
        ]
    );
    let script = result.script();
    assert_eq!(script.summary().as_str(), Some("L-bracket 60 x 45 x 40 mm"));
    assert_eq!(script.components().len(), 2);
    assert_eq!(script.components()[0].name(), "base");
    assert_eq!(script.components()[0].steps().len(), 10);
    assert_eq!(script.components()[1].name(), "upright");
    assert_eq!(script.components()[1].steps().len(), 4);
    assert_eq!(script.assembly().len(), 2);
    assert!(
        result
            .canonical_script_json()
            .unwrap()
            .contains("\"summary\":\"L-bracket 60 x 45 x 40 mm\"")
    );
}

#[test]
fn l_bracket_without_holes_is_a_plain_angle() {
    let result = l_bracket(60.0, 45.0, 40.0, 8.0, None, None, 0, None).expect("plain bracket");
    assert_eq!(result.script().components()[0].steps().len(), 2);
    assert_eq!(result.script().components()[1].steps().len(), 2);
    assert_eq!(result.script().assembly().len(), 2);
    assert_eq!(result.tolerances(), ["ISO 2768-m (declared)"]);
}

#[test]
fn l_bracket_two_holes_are_centred_in_depth() {
    let result = l_bracket(60.0, 45.0, 40.0, 8.0, None, Some(5.5), 2, None).expect("two holes");
    let steps = result.script().components()[0].steps();
    assert_eq!(steps.len(), 6);
    assert_eq!(steps[2].x(), -21.75);
    assert_eq!(steps[2].y(), 26.5);
    assert_eq!(steps[4].x(), 21.75);
    assert_eq!(steps[4].y(), 26.5);
}

#[test]
fn l_bracket_refuses_every_infeasible_test_case() {
    type InfeasibleCase = (
        [f64; 4],
        Option<f64>,
        Option<f64>,
        i64,
        Option<f64>,
        &'static str,
    );
    let cases: &[InfeasibleCase] = &[
        (
            [12.0, 45.0, 40.0, 8.0],
            None,
            Some(5.5),
            4,
            None,
            "two edge distances do not fit across the bracket width",
        ),
        (
            [60.0, 45.0, 20.0, 8.0],
            Some(12.0),
            None,
            0,
            None,
            "the bore must fit inside the upright leg (height - wall)",
        ),
        (
            [60.0, 45.0, 40.0, 8.0],
            None,
            Some(8.0),
            4,
            None,
            "the base holes must be smaller than the wall thickness",
        ),
        (
            [60.0, 45.0, 40.0, 8.0],
            None,
            Some(5.5),
            3,
            None,
            "hole_count must be 0, 2, or 4",
        ),
        (
            [60.0, 45.0, 8.0, 8.0],
            None,
            None,
            0,
            None,
            "height must be greater than the wall thickness",
        ),
        (
            [60.0, 8.0, 40.0, 8.0],
            None,
            None,
            0,
            None,
            "depth must be greater than the wall thickness",
        ),
        (
            [60.0, 45.0, 40.0, 8.0],
            None,
            Some(5.5),
            4,
            Some(2.0),
            "the hole edge distance must exceed the hole radius",
        ),
    ];
    for (dims, bore, hole, count, edge, expected) in cases {
        let error = l_bracket(
            dims[0], dims[1], dims[2], dims[3], *bore, *hole, *count, *edge,
        )
        .expect_err(expected);
        assert_eq!(error.message(), *expected);
    }
}

#[test]
fn l_bracket_refuses_non_finite_and_out_of_range_values() {
    assert_eq!(
        l_bracket(f64::NAN, 45.0, 40.0, 8.0, None, None, 0, None)
            .unwrap_err()
            .message(),
        "width must be a finite number"
    );
    assert_eq!(
        l_bracket(f64::INFINITY, 45.0, 40.0, 8.0, None, None, 0, None)
            .unwrap_err()
            .message(),
        "width must be a finite number"
    );
    assert_eq!(
        l_bracket(0.0, 45.0, 40.0, 8.0, None, None, 0, None)
            .unwrap_err()
            .message(),
        "width must be between 0.1 and 2000.0 millimetres"
    );
    assert_eq!(
        l_bracket(2000.1, 45.0, 40.0, 8.0, None, None, 0, None)
            .unwrap_err()
            .message(),
        "width must be between 0.1 and 2000.0 millimetres"
    );
    assert_eq!(
        l_bracket(60.0, 45.0, 40.0, 8.0, None, None, 0, None)
            .unwrap()
            .label(),
        "L-bracket 60 x 45 x 40"
    );
}

#[test]
fn stepped_shaft_declares_envelope_and_tolerances() {
    let result = stepped_shaft(&[
        TemplateShaftSegment {
            diameter_mm: 25.0,
            length_mm: 120.0,
        },
        TemplateShaftSegment {
            diameter_mm: 15.0,
            length_mm: 30.0,
        },
    ])
    .expect("stepped shaft");
    assert_eq!(result.label(), "Stepped shaft 2 segments");
    assert_eq!(result.envelope().min_mm(), [-12.5, -12.5, 0.0]);
    assert_eq!(result.envelope().max_mm(), [12.5, 12.5, 150.0]);
    assert_eq!(
        result.tolerances(),
        [
            "segment 1 diameter 25 mm (declared)",
            "segment 2 diameter 15 mm (declared)",
            "ISO 2768-m (declared)"
        ]
    );
    assert_eq!(result.script().components().len(), 2);
    assert_eq!(result.script().assembly().len(), 3);
    assert_eq!(
        result.script().components()[1].steps()[0].diameter(),
        Some(15.0)
    );
}

#[test]
fn stepped_shaft_refuses_empty_oversized_and_too_long() {
    assert_eq!(
        stepped_shaft(&[]).unwrap_err().message(),
        "a stepped shaft requires at least one segment"
    );
    let nine = vec![
        TemplateShaftSegment {
            diameter_mm: 10.0,
            length_mm: 10.0,
        };
        9
    ];
    assert_eq!(
        stepped_shaft(&nine).unwrap_err().message(),
        "a stepped shaft supports at most 8 segments"
    );
    let long = vec![
        TemplateShaftSegment {
            diameter_mm: 10.0,
            length_mm: 2000.0,
        };
        2
    ];
    assert_eq!(
        stepped_shaft(&long).unwrap_err().message(),
        "the stepped shaft length exceeds the studio dimension limit"
    );
    assert_eq!(
        stepped_shaft(&[TemplateShaftSegment {
            diameter_mm: 0.0,
            length_mm: 10.0,
        }])
        .unwrap_err()
        .message(),
        "segment 1 diameter must be between 0.1 and 2000.0 millimetres"
    );
    assert_eq!(
        stepped_shaft(&[TemplateShaftSegment {
            diameter_mm: 10.0,
            length_mm: 0.0,
        }])
        .unwrap_err()
        .message(),
        "segment 1 length must be between 0.1 and 2000.0 millimetres"
    );
}

#[test]
fn housing_envelope_declares_floor_walls_and_envelope() {
    let result = housing_envelope(80.0, 60.0, 40.0, 5.0).expect("housing");
    assert_eq!(result.label(), "Housing 80 x 60 x 40");
    assert_eq!(result.envelope().min_mm(), [-40.0, -30.0, 0.0]);
    assert_eq!(result.envelope().max_mm(), [40.0, 30.0, 40.0]);
    assert_eq!(
        result.tolerances(),
        ["walls and floor 5 mm (declared)", "ISO 2768-m (declared)"]
    );
    let names: Vec<&str> = result
        .script()
        .components()
        .iter()
        .map(|component| component.name())
        .collect();
    assert_eq!(
        names,
        [
            "floor",
            "wall_left",
            "wall_right",
            "wall_front",
            "wall_back"
        ]
    );
    assert_eq!(
        result.script().components()[0].steps()[0].width(),
        Some(80.0)
    );
    assert_eq!(
        result.script().components()[3].steps()[0].width(),
        Some(70.0)
    );
}

#[test]
fn housing_refuses_a_closed_or_empty_interior() {
    let cases = [
        (
            [10.0, 60.0, 40.0, 5.0],
            "the wall thickness leaves no interior across the width",
        ),
        (
            [80.0, 10.0, 40.0, 5.0],
            "the wall thickness leaves no interior across the depth",
        ),
        (
            [80.0, 60.0, 5.0, 5.0],
            "the wall thickness must be less than the housing height",
        ),
    ];
    for (values, expected) in cases {
        let error =
            housing_envelope(values[0], values[1], values[2], values[3]).expect_err(expected);
        assert_eq!(error.message(), expected);
    }
}

#[test]
fn templates_are_deterministic() {
    assert_eq!(adr_bracket(), adr_bracket());
    assert_eq!(
        adr_bracket().canonical_script_json().unwrap(),
        adr_bracket().canonical_script_json().unwrap()
    );
}

#[test]
fn bounds_validate_finite_and_ordered_corners() {
    assert_eq!(
        Bounds::new([0.0, 0.0, f64::NAN], [1.0, 1.0, 1.0])
            .unwrap_err()
            .message(),
        "bounds must be finite numbers"
    );
    assert_eq!(
        Bounds::new([0.0, 0.0, 2.0], [1.0, 1.0, 1.0])
            .unwrap_err()
            .message(),
        "bounds minimum exceeds maximum"
    );
    let bounds = Bounds::new([-1.0, -2.0, 0.0], [3.0, 4.0, 5.0]).unwrap();
    assert_eq!(bounds.size_mm(), [4.0, 6.0, 5.0]);
}

#[test]
fn general_number_format_matches_python() {
    let cases = [
        (0.0, "0"),
        (-0.0, "-0"),
        (60.0, "60"),
        (5.5, "5.5"),
        (1_000_000.0, "1e+06"),
        (0.000_01, "1e-05"),
        (1.0 / 3.0, "0.333333"),
        (1_234.567_890_123, "1234.57"),
    ];
    for (value, expected) in cases {
        assert_eq!(format_g(value), expected);
    }
}

// ---------------------------------------------------------------------------
// JSON Schema
// ---------------------------------------------------------------------------

#[test]
fn schema_is_a_valid_bounded_schema() {
    let text = design_intent_schema_text();
    assert_eq!(text.len(), 11_291);
    let schema = design_intent_json_schema();
    let kala_json::CanonicalValue::Object(fields) = &schema else {
        panic!("schema is not an object");
    };
    assert!(fields.contains_key(&kala_json::JsonText::new("$defs")));
    assert!(fields.contains_key(&kala_json::JsonText::new("properties")));
    assert!(text.contains("\"title\": \"DesignIntent\""));
    assert!(text.contains("l_bracket"));
    assert!(text.contains("stepped_shaft"));
    assert!(text.contains("housing_envelope"));
    assert!(text.contains("ConstructionScript"));
    assert_eq!(schema_text(), text);
    assert!(kala_design_intent::DESIGN_INTENT_INSTRUCTION.contains("JSON Schema"));
    assert!(kala_design_intent::DESIGN_INTENT_INSTRUCTION.contains("Never invent a dimension"));
}

// ---------------------------------------------------------------------------
// DesignIntent
// ---------------------------------------------------------------------------

fn bracket_payload() -> &'static str {
    r#"{"template":{"template":"l_bracket","width":60,"depth":45,"height":40,"wall":8,"bore_d":10,"hole_d":5.5,"hole_count":4},"material":"aluminium","tolerances":["ISO 2768-m"],"label":"Mounting bracket"}"#
}

fn box_script() -> &'static str {
    r#"{"summary":"20 x 30 x 40 box","components":[{"name":"box","steps":[{"op":"sketch","profile":"rectangle","plane":"XY","width":20,"height":30,"x":-10,"y":-15},{"op":"extrude","length":40}]}]}"#
}

#[test]
fn template_intent_values_and_dump_order() {
    let intent = DesignIntent::from_json_str(bracket_payload()).expect("valid intent");
    let TemplateCall::Lbracket(call) = intent.template().expect("template") else {
        panic!("not an L-bracket");
    };
    assert_eq!(call.depth(), 45.0);
    assert_eq!(call.height(), 40.0);
    assert_eq!(call.wall(), 8.0);
    assert_eq!(call.bore_d(), Some(10.0));
    assert_eq!(call.hole_d(), Some(5.5));
    assert_eq!(call.hole_count(), HoleCount::Four);
    assert_eq!(call.edge(), None);
    assert_eq!(intent.material().unwrap().as_str(), Some("aluminium"));
    assert_eq!(intent.tolerances()[0].as_str(), Some("ISO 2768-m"));
    assert_eq!(intent.label().unwrap().as_str(), Some("Mounting bracket"));
    assert_eq!(
        intent.to_wire_json_excluding_none().unwrap(),
        r#"{"template":{"template":"l_bracket","width":60.0,"depth":45.0,"height":40.0,"wall":8.0,"bore_d":10.0,"hole_d":5.5,"hole_count":4},"material":"aluminium","tolerances":["ISO 2768-m"],"label":"Mounting bracket"}"#
    );
    assert!(intent.to_wire_json().unwrap().contains("\"script\":null"));
    let canonical = intent.to_canonical_json(true).unwrap();
    assert!(canonical.starts_with("{\"label\":\"Mounting bracket\",\"material\":\"aluminium\""));
}

#[test]
fn script_intent_round_trips() {
    let payload = format!(r#"{{"script":{},"material":"  steel  "}}"#, box_script());
    let intent = DesignIntent::from_json_str(&payload).expect("valid script intent");
    assert!(intent.template().is_none());
    assert_eq!(
        intent.script().unwrap().summary().as_str(),
        Some("20 x 30 x 40 box")
    );
    assert_eq!(intent.material().unwrap().as_str(), Some("steel"));
    assert!(intent.tolerances().is_empty());
}

#[test]
fn intent_requires_exactly_one_source_and_refuses_unknown_fields() {
    assert_eq!(
        DesignIntent::from_json_str("{}").unwrap_err().to_string(),
        "intent: a design intent must carry exactly one of template or script"
    );
    let both = format!(
        r#"{{"template":{{"template":"l_bracket","width":60,"depth":45,"height":40,"wall":8}},"script":{}}}"#,
        box_script()
    );
    assert_eq!(
        DesignIntent::from_json_str(&both).unwrap_err().to_string(),
        "intent: a design intent must carry exactly one of template or script"
    );
    let extra = r#"{"template":{"template":"l_bracket","width":60,"depth":45,"height":40,"wall":8},"extra":true}"#;
    assert_eq!(
        DesignIntent::from_json_str(extra).unwrap_err().to_string(),
        "extra: Extra inputs are not permitted"
    );
}

#[test]
fn intent_aliases_and_bounds_match_the_python_tests() {
    let intent = DesignIntent::from_json_str(
        r#"{"template":{"template":"l_bracket","W":60,"D":45,"H":40,"T":8,"B":10,"h":5.5,"holes":4,"e":8.25}}"#,
    )
    .expect("aliases");
    let TemplateCall::Lbracket(call) = intent.template().unwrap() else {
        panic!();
    };
    assert_eq!(call.width(), 60.0);
    assert_eq!(call.edge(), Some(8.25));
    assert_eq!(call.hole_count(), HoleCount::Four);

    let long_material = format!(
        r#"{{"template":{{"template":"l_bracket","width":60,"depth":45,"height":40,"wall":8}},"material":"{}"}}"#,
        "x".repeat(MAX_MATERIAL_LENGTH + 1)
    );
    assert_eq!(
        DesignIntent::from_json_str(&long_material)
            .unwrap_err()
            .to_string(),
        format!("material: String should have at most {MAX_MATERIAL_LENGTH} characters")
    );
    let long_tolerance = format!(
        r#"{{"template":{{"template":"l_bracket","width":60,"depth":45,"height":40,"wall":8}},"tolerances":["{}"]}}"#,
        "x".repeat(MAX_TOLERANCE_LENGTH + 1)
    );
    assert!(
        DesignIntent::from_json_str(&long_tolerance)
            .unwrap_err()
            .to_string()
            .starts_with("tolerances.0: String should have at most")
    );
    let too_many = format!(
        r#"{{"template":{{"template":"l_bracket","width":60,"depth":45,"height":40,"wall":8}},"tolerances":[{}]}}"#,
        vec!["\"ok\""; MAX_TOLERANCES + 1].join(",")
    );
    assert_eq!(
        DesignIntent::from_json_str(&too_many)
            .unwrap_err()
            .to_string(),
        format!(
            "tolerances: Tuple should have at most {MAX_TOLERANCES} items after validation, not {}",
            MAX_TOLERANCES + 1
        )
    );
    let long_label = format!(
        r#"{{"template":{{"template":"l_bracket","width":60,"depth":45,"height":40,"wall":8}},"label":"{}"}}"#,
        "x".repeat(MAX_LABEL_LENGTH + 1)
    );
    assert!(
        DesignIntent::from_json_str(&long_label)
            .unwrap_err()
            .to_string()
            .contains("String should have at most 120 characters")
    );
}

#[test]
fn stepped_shaft_segments_are_bounded() {
    assert!(
        DesignIntent::from_json_str(r#"{"template":{"template":"stepped_shaft","segments":[]}}"#)
            .unwrap_err()
            .to_string()
            .contains("Tuple should have at least 1 item")
    );
    let nine = format!(
        r#"{{"template":{{"template":"stepped_shaft","segments":[{}]}}}}"#,
        ["{\"diameter\":10,\"length\":10}"; MAX_SEGMENTS + 1].join(",")
    );
    assert!(
        DesignIntent::from_json_str(&nine)
            .unwrap_err()
            .to_string()
            .contains("Tuple should have at most 8 items")
    );
}

#[test]
fn intent_refuses_unknown_template_and_wrong_types() {
    assert_eq!(
        DesignIntent::from_json_str(r#"{"template":{"template":"flange","width":10}}"#)
            .unwrap_err()
            .to_string(),
        "template: Input tag 'flange' found using 'template' does not match any of the expected tags: 'l_bracket', 'stepped_shaft', 'housing_envelope'"
    );
    assert_eq!(
        DesignIntent::from_json_str(r#"{"template":{"width":10}}"#)
            .unwrap_err()
            .to_string(),
        "template: Unable to extract tag using discriminator 'template'"
    );
    assert_eq!(
        DesignIntent::from_json_str(r#"{"template":1}"#)
            .unwrap_err()
            .to_string(),
        "template: Input should be a valid dictionary or object to extract fields from"
    );
    assert_eq!(
        DesignIntent::from_json_str(
            r#"{"template":{"template":"l_bracket","width":0,"depth":45,"height":40,"wall":8}}"#
        )
        .unwrap_err()
        .to_string(),
        "template.l_bracket.width: Input should be greater than 0"
    );
    assert_eq!(
        DesignIntent::from_json_str(
            r#"{"template":{"template":"l_bracket","width":2001,"depth":45,"height":40,"wall":8}}"#
        )
        .unwrap_err()
        .to_string(),
        "template.l_bracket.width: Input should be less than or equal to 2000"
    );
    assert_eq!(
        DesignIntent::from_json_str(
            r#"{"template":{"template":"l_bracket","width":60,"depth":45,"height":40}}"#
        )
        .unwrap_err()
        .to_string(),
        "template.l_bracket.wall: Field required"
    );
}

// ---------------------------------------------------------------------------
// Reply parsing
// ---------------------------------------------------------------------------

#[test]
fn reply_parsing_accepts_fenced_and_bare_objects() {
    let fenced = format!(
        "Here is the bracket.\n\n```json\n{}\n```\nThe kernel will measure it.",
        bracket_payload()
    );
    let intent = parse_design_intent_text(&fenced).expect("fenced");
    assert!(matches!(intent.template(), Some(TemplateCall::Lbracket(_))));
    parse_design_intent_text(bracket_payload()).expect("bare");
    parse_design_intent_text(&format!("```JSON\n{}\n```", bracket_payload())).expect("upper fence");
    assert_eq!(
        parse_design_intent_text(&format!("``` json\n{}\n```", bracket_payload()))
            .unwrap_err()
            .to_string(),
        "the model reply did not contain a fenced json block"
    );
}

#[test]
fn reply_parsing_refuses_empty_unfenced_and_malformed() {
    assert_eq!(
        parse_design_intent_text("   \n\t ")
            .unwrap_err()
            .to_string(),
        "the model reply was empty"
    );
    assert_eq!(
        parse_design_intent_text("I need the width and depth first.")
            .unwrap_err()
            .to_string(),
        "the model reply did not contain a fenced json block"
    );
    assert_eq!(
        parse_design_intent_text("```json\n{not json}\n```")
            .unwrap_err()
            .to_string(),
        "the design JSON is not valid JSON: Expecting property name enclosed in double quotes"
    );
    assert_eq!(
        parse_design_intent_text("```json\n{\"a\":1} trailing\n```")
            .unwrap_err()
            .to_string(),
        "the model reply did not contain a fenced json block"
    );
    assert_eq!(
        parse_design_intent_text("{\"a\":1} trailing")
            .unwrap_err()
            .to_string(),
        "the design JSON is not valid JSON: Extra data"
    );
    assert!(
        parse_design_intent_text("```json\n{\"template\":{\"template\":\"flange\"}}\n```")
            .unwrap_err()
            .to_string()
            .contains("does not match any of the expected tags")
    );
}

#[test]
fn invalid_intent_parse_names_the_field() {
    let payload =
        r#"{"template":{"template":"l_bracket","width":-3,"depth":45,"height":40,"wall":8}}"#;
    let error: DesignIntentParseError =
        parse_design_intent_text(&format!("```json\n{payload}\n```")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "template.l_bracket.width: Input should be greater than 0"
    );
}

#[test]
fn strip_design_json_block_keeps_prose() {
    let text = format!(
        "A 60 mm plate with a 10 mm bore.\n\n```json\n{}\n```",
        bracket_payload()
    );
    assert_eq!(
        strip_design_json_block(&text),
        "A 60 mm plate with a 10 mm bore."
    );
    assert_eq!(
        strip_design_json_block("```json\n{}\n```"),
        "Design intent received."
    );
}

#[test]
fn python_json_value_errors_are_reported_as_input_errors() {
    let error = DesignIntent::from_json_str("{").unwrap_err();
    assert!(matches!(error, DesignIntentInputError::Json(_)));
    assert_eq!(
        error.to_string(),
        "the design JSON is not valid JSON: Expecting property name enclosed in double quotes"
    );
    let error = DesignIntent::from_json_str("{}").unwrap_err();
    assert!(matches!(error, DesignIntentInputError::Validation(_)));
}

// ---------------------------------------------------------------------------
// Local design-prompt parser
// ---------------------------------------------------------------------------

#[test]
fn detects_the_three_families_and_rejects_a_cylindrical_housing() {
    assert_eq!(
        detect_design_family("make me a bracket"),
        Some(DesignFamily::LBracket)
    );
    assert_eq!(
        detect_design_family("SIZE AN L-BRACKET"),
        Some(DesignFamily::LBracket)
    );
    assert_eq!(
        detect_design_family("stepped shaft spacer"),
        Some(DesignFamily::SteppedShaft)
    );
    assert_eq!(
        detect_design_family("rectangular enclosure"),
        Some(DesignFamily::HousingEnvelope)
    );
    assert_eq!(
        detect_design_family("cylindrical housing"),
        None,
        "a cylindrical housing is not the rectangular template"
    );
    assert_eq!(detect_design_family("round enclosure"), None);
    assert_eq!(detect_design_family("Design a flange"), None);
    assert_eq!(detect_design_family(""), None);
}

#[test]
fn parses_the_l_bracket_starter_and_declares_its_values() {
    let prompt = "Size an L-bracket for a 40 mm mounting plate: legs of 60 mm and 45 mm, 8 mm wall thickness. Give me the bounding envelope and the nominal volume I should expect before I model it in CAD.";
    let parsed = parse_design_prompt(prompt).unwrap().expect("design prompt");
    assert_eq!(parsed.family(), DesignFamily::LBracket);
    assert!(parsed.missing().is_empty());
    assert!(parsed.notes().is_empty());
    let intent = parsed.intent().expect("complete intent");
    let Some(TemplateCall::Lbracket(call)) = intent.template() else {
        panic!("not an L-bracket");
    };
    assert_eq!(
        (call.width(), call.depth(), call.height(), call.wall()),
        (60.0, 45.0, 40.0, 8.0)
    );
    assert_eq!(call.bore_d(), None);
    assert_eq!(call.hole_count(), HoleCount::Zero);
    assert_eq!(
        design_prompt_reply(&parsed),
        "Local helper. Design intent parsed from declared values. Compiling and measuring with OpenCASCADE."
    );
}

#[test]
fn parses_the_stepped_shaft_starter() {
    let prompt = "Size a stepped shaft spacer: 25 mm diameter by 120 mm long, with a 15 mm diameter shoulder 30 mm long. Give me the nominal volume, surface area, and envelope for a quick mass and packaging estimate.";
    let parsed = parse_design_prompt(prompt).unwrap().expect("design prompt");
    assert_eq!(parsed.family(), DesignFamily::SteppedShaft);
    let Some(TemplateCall::SteppedShaft(call)) = parsed.intent().unwrap().template() else {
        panic!("not a stepped shaft");
    };
    assert_eq!(call.segments().len(), 2);
    assert_eq!(call.segments()[0].diameter(), 25.0);
    assert_eq!(call.segments()[1].length(), 30.0);
}

#[test]
fn missing_value_reply_is_exact_and_bounded() {
    let parsed = parse_design_prompt("make me a bracket")
        .unwrap()
        .expect("design prompt");
    assert_eq!(
        parsed.missing(),
        [
            "width (mm)",
            "depth (mm)",
            "height (mm)",
            "wall thickness (mm)"
        ]
    );
    assert_eq!(
        design_prompt_reply(&parsed),
        "Local helper. I can draft a L-bracket, but I need these declared values: width (mm), depth (mm), height (mm), wall thickness (mm). No design was compiled."
    );
    let long = "bracket ".repeat(500);
    let parsed = parse_design_prompt(&long).unwrap().expect("design prompt");
    assert!(design_prompt_reply(&parsed).chars().count() <= MAX_RESPONSE_LENGTH);
}

#[test]
fn canonical_defaults_are_declared_in_a_note() {
    let parsed = parse_design_prompt("40 mm tall bracket with 8 mm wall")
        .unwrap()
        .expect("design prompt");
    let intent = parsed.intent().expect("defaults fill the plate");
    let Some(TemplateCall::Lbracket(call)) = intent.template() else {
        panic!();
    };
    assert_eq!(call.width(), 60.0);
    assert_eq!(call.depth(), 45.0);
    assert_eq!(
        parsed.notes(),
        [
            "Plate width 60 mm and depth 45 mm are canonical template defaults; state them to override."
        ]
    );
}

#[test]
fn metric_hole_sizes_map_through_iso_273_medium() {
    assert_eq!(iso_273_medium_clearance(5.0), Some(5.5));
    assert_eq!(iso_273_medium_clearance(6.0), Some(6.6));
    assert_eq!(iso_273_medium_clearance(20.0), Some(22.0));
    assert_eq!(iso_273_medium_clearance(7.0), None);
    assert_eq!(M_SIZE_CLEARANCE.len(), 9);
    let parsed = parse_design_prompt("bracket with 4 x M6 clearance holes, 60x45x40 mm, 8 mm wall")
        .unwrap()
        .expect("design prompt");
    let Some(TemplateCall::Lbracket(call)) = parsed.intent().unwrap().template() else {
        panic!();
    };
    assert_eq!(call.hole_d(), Some(6.6));
    assert_eq!(
        parsed.notes(),
        ["4x M6 mapped to 6.6 mm clearance holes (ISO 273 medium)."]
    );
    let parsed = parse_design_prompt("bracket with 4 x M7 holes, 60x45x40 mm, 8 mm wall")
        .unwrap()
        .expect("design prompt");
    assert!(parsed.intent().is_none());
    assert_eq!(
        parsed.missing(),
        ["hole diameter (no ISO 273 medium value for M7)"]
    );
}

#[test]
fn parser_reports_missing_values_in_python_order() {
    let parsed = parse_design_prompt("stepped shaft")
        .unwrap()
        .expect("design prompt");
    assert_eq!(parsed.missing(), ["segment diameters and lengths (mm)"]);
    let parsed = parse_design_prompt("housing 80x60x40 mm")
        .unwrap()
        .expect("design prompt");
    assert_eq!(parsed.missing(), ["wall thickness (mm)"]);
    let parsed = parse_design_prompt("bracket with 4 holes, 60x45x40 mm, 8 mm wall")
        .unwrap()
        .expect("design prompt");
    assert_eq!(parsed.missing(), ["hole diameter (mm) or an M-size"]);
    let parsed = parse_design_prompt("bracket with 3 holes, 60x45x40 mm, 8 mm wall")
        .unwrap()
        .expect("design prompt");
    assert_eq!(
        parsed.missing(),
        ["hole diameter (mm) or an M-size", "2 or 4 holes"]
    );
}

#[test]
fn parser_raises_the_intent_validation_text_for_out_of_range_values() {
    let error =
        parse_design_prompt("L-bracket 9999 x 45 x 40 mm, 8 mm wall").expect_err("out of range");
    assert_eq!(
        error.to_string(),
        "width: Input should be less than or equal to 2000"
    );
    let error = parse_design_prompt("L-bracket 60 x 45 x 40 mm, 0 mm wall").expect_err("zero wall");
    assert_eq!(error.to_string(), "wall: Input should be greater than 0");
    let error = parse_design_prompt("stepped shaft 0 mm diameter by 120 mm long")
        .expect_err("zero diameter");
    assert_eq!(
        error.to_string(),
        "diameter: Input should be greater than 0"
    );
}

#[test]
fn parser_returns_none_for_analytic_prompts() {
    for prompt in [
        "Design a small clamp. It must fit a 12 mm shaft.",
        "I am quoting a cylindrical housing, 80 mm outer diameter and 60 mm tall.",
        "Explain the OpenCascade STEP workflow please.",
        "Size a box 20x30x40 mm mounting plate.",
        "",
    ] {
        assert_eq!(parse_design_prompt(prompt).unwrap(), None, "{prompt:?}");
    }
}

#[test]
fn material_detection_uses_the_first_keyword() {
    let parsed = parse_design_prompt("stainless steel bracket 60x45x40 mm 8 mm wall")
        .unwrap()
        .expect("design prompt");
    assert_eq!(
        parsed.intent().unwrap().material().unwrap().as_str(),
        Some("stainless steel")
    );
    let parsed = parse_design_prompt("aluminum bracket 60x45x40 mm 8 mm wall")
        .unwrap()
        .expect("design prompt");
    assert_eq!(
        parsed.intent().unwrap().material().unwrap().as_str(),
        Some("aluminium")
    );
}

#[test]
fn parser_is_deterministic() {
    let prompt = "L bracket 60 x 45 x 40 mm wall 8 mm with a 10 mm bore and 4 x M6 holes";
    let first = parse_design_prompt(prompt).unwrap().expect("design prompt");
    let second = parse_design_prompt(prompt).unwrap().expect("design prompt");
    assert_eq!(first, second);
    assert_eq!(design_prompt_reply(&first), design_prompt_reply(&second));
}

// ---------------------------------------------------------------------------
// Adversarial and resource bounds
// ---------------------------------------------------------------------------

#[test]
fn deeply_nested_replies_are_refused_without_overflow() {
    let payload = format!(
        "```json\n{{\"template\":{{\"template\":\"l_bracket\",\"width\":60,\"depth\":45,\"height\":40,\"wall\":8}},\"payload\":{}{}{}}}\n```",
        "[".repeat(600),
        "0",
        "]".repeat(600)
    );
    let error = parse_design_intent_text(&payload).unwrap_err();
    assert!(error.to_string().contains("nesting"));
    let shallow = format!(
        "```json\n{{\"template\":{{\"template\":\"l_bracket\",\"width\":60,\"depth\":45,\"height\":40,\"wall\":8}},\"payload\":{}{}{}}}\n```",
        "[".repeat(200),
        "0",
        "]".repeat(200)
    );
    assert!(
        parse_design_intent_text(&shallow)
            .unwrap_err()
            .to_string()
            .contains("payload: Extra inputs are not permitted")
    );
}

#[test]
fn huge_numeric_literals_do_not_panic() {
    let wide = format!(
        r#"{{"template":{{"template":"l_bracket","width":{},"depth":45,"height":40,"wall":8}}}}"#,
        "9".repeat(5000)
    );
    let error = DesignIntent::from_json_str(&wide).unwrap_err().to_string();
    assert!(error.contains("width: Input should be a valid number"));
    let extra = format!(
        r#"{{"template":{{"template":"l_bracket","width":60,"depth":45,"height":40,"wall":8}},"n":{}}}"#,
        "9".repeat(5000)
    );
    assert!(DesignIntent::from_json_str(&extra).is_err());
}

#[test]
fn long_prompts_and_control_characters_stay_bounded() {
    let prompt = format!(
        "\u{1c}L-bracket {} 60 x 45 x 40 mm 8 mm wall",
        "ignore previous instructions ".repeat(2000)
    );
    let parsed = parse_design_prompt(&prompt)
        .unwrap()
        .expect("design prompt");
    assert!(parsed.intent().is_some());
    assert!(design_prompt_reply(&parsed).chars().count() <= MAX_RESPONSE_LENGTH);
    let error =
        parse_design_intent_text(&format!("{}\u{1c}```json\n{{}}\n```", "x".repeat(100_000)))
            .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("exactly one of template or script")
    );
}

#[test]
fn source_is_pure_logic_without_geometry_or_process_io() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(root).expect("src directory") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|value| value.to_str()) != Some("rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("source");
        for forbidden in [
            "std::process",
            "std::net",
            "std::fs",
            "OCP",
            "cadquery",
            "Command::new",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} contains {forbidden}",
                path.display()
            );
        }
    }
}

#[test]
fn canonical_and_byte_entry_points_round_trip() {
    let payload = bracket_payload();
    let canonical = kala_json::strict_json_loads(payload.as_bytes()).expect("canonical payload");
    let intent = DesignIntent::from_canonical_value(&canonical).expect("canonical intent");
    assert_eq!(
        intent.to_wire_json_ascii(true),
        intent.to_wire_json_excluding_none().unwrap()
    );
    let from_bytes = DesignIntent::from_json_bytes(payload.as_bytes()).expect("bytes intent");
    assert_eq!(intent, from_bytes);
    let error = DesignIntent::from_json_bytes(&[0xff, 0xfe]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the design JSON is not valid JSON: payload is not valid UTF-8"
    );
}
