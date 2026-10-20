//! Construction script validation independent of the Python oracle.

use kala_contracts::{
    ConstructionScript, ContractModel, Text, load_analysis, load_construction,
    load_construction_json_str, load_reading, model_json_limits,
};
use kala_json::CanonicalValue;

fn raw(source: &str) -> CanonicalValue {
    kala_json::parse_strict(source.as_bytes(), &model_json_limits()).expect("test JSON")
}

fn construction_error(source: &str) -> String {
    load_construction(&raw(source))
        .expect_err(source)
        .message()
        .to_owned()
}

fn analysis_error(source: &str) -> String {
    load_analysis(&raw(source))
        .expect_err(source)
        .message()
        .to_owned()
}

fn reading_error(source: &str) -> String {
    load_reading(&raw(source))
        .expect_err(source)
        .message()
        .to_owned()
}

#[test]
fn loads_nested_profiles_and_sequence_rules() {
    let script = load_construction(&raw(
        r#"{"summary":"A screw.","components":[{"name":"Screw","steps":[{"op":"sketch","profile":{"circle":{"diameter":20},"x":1,"y":2}},{"op":"extrude","length":100},{"op":"helix","diameter":20,"pitch":5,"length":100}]}],"assembly":[{"op":"place","component":"Screw","z":5}]}"#,
    ))
    .expect("script");
    let sketch = &script.components()[0].steps()[0];
    assert_eq!(sketch.profile().and_then(Text::as_str), Some("circle"));
    assert_eq!(sketch.diameter(), Some(20.0));
    assert_eq!(sketch.x(), 1.0);
    assert_eq!(script.assembly()[0].z(), 5.0);
}

#[test]
fn construction_error_messages_match_the_python_loader() {
    let cases = [
        (
            r#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"bogus"}]}]}"#,
            "unknown operation bogus",
        ),
        (
            r#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"pocket"}]}]}"#,
            "a pocket must follow a solid",
        ),
        (
            r#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","profile":"rectangle","width":1}]}]}"#,
            "a rectangle sketch requires width and height",
        ),
        (
            r#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","diameter":1},{"op":"extrude"}]}]}"#,
            "extrude requires a length",
        ),
        (
            r#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","width":1,"height":1},{"op":"revolve","angle":361}]}]}"#,
            "revolve angle must be in (0, 360]",
        ),
        (
            r#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","width":1,"height":1},{"op":"helix","diameter":1,"pitch":1}]}]}"#,
            "helix requires a length",
        ),
        (
            r#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"extrude","length":1,"reversed":true}]}]}"#,
            "extrude must follow a sketch",
        ),
        (
            r#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","width":1,"height":1},{"op":"extrude","length":1,"reversed":true}]},{"name":"B","steps":[{"op":"sketch","width":1,"height":1},{"op":"extrude","length":1}]}]}"#,
            "component names must be unique",
        ),
        (
            r#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","width":1,"height":1},{"op":"extrude","length":1}]}],"assembly":[{"op":"place","component":"X"}]}"#,
            "assembly names a missing component X",
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(construction_error(source), expected, "{source}");
    }
}

#[test]
fn non_objects_keep_the_loader_message() {
    for source in ["[]", "1", "\"x\"", "null"] {
        assert_eq!(
            construction_error(source),
            "the model did not return an object"
        );
    }
}

#[test]
fn analysis_and_reading_rules_match() {
    let analysis = load_analysis(&raw(
        r#"{"steps":[{"op":"material","name":"steel"},{"op":"fix","face":1},{"op":"force","force_n":10},{"op":"mesh"},{"op":"solve"}]}"#,
    ))
    .expect("analysis");
    assert_eq!(analysis.steps().len(), 5);
    assert_eq!(
        analysis_error(r#"{"steps":[{"op":"solve"},{"op":"material"}]}"#),
        "analysis steps are out of order"
    );
    assert_eq!(
        analysis_error(r#"{"steps":[{"op":"material"}]}"#),
        "an analysis script must include solve"
    );
    assert_eq!(
        analysis_error(r#"{"steps":[{"op":"force"}]}"#),
        "force requires force_n"
    );
    assert_eq!(
        analysis_error(r#"{"steps":[{"op":"fix","face":0}]}"#),
        "face must be an integer from 1 to 64"
    );
    assert_eq!(
        reading_error(r#"{"verdict":"ok","tool_edit":[{"op":"resize","component":"B"}]}"#),
        "resize requires a length or a diameter"
    );
    assert_eq!(
        reading_error(r#"{"verdict":""}"#),
        "verdict must be nonempty and at most 500 characters"
    );
}

#[test]
fn default_revolve_angle_and_literal_wire_shape() {
    let source = r#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","width":2,"height":2},{"op":"revolve"}]}]}"#;
    let script = ConstructionScript::from_json_str(source).expect("script");
    assert_eq!(script.components()[0].steps()[1].angle(), Some(360.0));
    assert_eq!(
        script.to_wire_json_ascii(),
        r#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","plane":"XY","profile":"rectangle","width":2.0,"height":2.0,"diameter":null,"length":null,"pitch":null,"angle":null,"x":0.0,"y":0.0,"reversed":false},{"op":"revolve","plane":"XY","profile":null,"width":null,"height":null,"diameter":null,"length":null,"pitch":null,"angle":360.0,"x":0.0,"y":0.0,"reversed":false}]}],"assembly":[]}"#
    );
}

#[test]
fn ordered_profile_aliases_match_python_insertion_order() {
    let rectangle_first = r#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","profile":{"rectangle":{"width":10,"height":4},"circle":{"diameter":8}}},{"op":"extrude","length":2}]}]}"#;
    let circle_first = r#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","profile":{"circle":{"diameter":8},"rectangle":{"width":10,"height":4}}},{"op":"extrude","length":2}]}]}"#;
    let rectangle = ConstructionScript::from_json_str(rectangle_first).expect("rectangle");
    let circle = ConstructionScript::from_json_str(circle_first).expect("circle");
    assert_eq!(
        rectangle.components()[0].steps()[0]
            .profile()
            .and_then(Text::as_str),
        Some("rectangle")
    );
    assert_eq!(rectangle.components()[0].steps()[0].width(), Some(10.0));
    assert_eq!(
        circle.components()[0].steps()[0]
            .profile()
            .and_then(Text::as_str),
        Some("circle")
    );
    assert_eq!(circle.components()[0].steps()[0].diameter(), Some(8.0));

    let loaded = load_construction_json_str(rectangle_first).expect("loader");
    assert_eq!(
        loaded.components()[0].steps()[0]
            .profile()
            .and_then(Text::as_str),
        Some("rectangle")
    );
}

#[test]
fn model_json_duplicate_keys_are_last_wins() {
    let source = r#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","width":1,"height":2,"width":3},{"op":"extrude","length":4}]}]}"#;
    let script = ConstructionScript::from_json_str(source).expect("script");
    assert_eq!(script.components()[0].steps()[0].width(), Some(3.0));
}

#[test]
fn ordered_profiles_are_used_by_string_and_byte_entry_points() {
    let source = r#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","profile":{"rectangle":{"width":10,"height":4},"circle":{"diameter":8}}},{"op":"extrude","length":2}]}]}"#;
    let from_str = ConstructionScript::from_json_str(source).expect("str");
    let from_bytes = ConstructionScript::from_json_bytes(source.as_bytes()).expect("bytes");
    assert_eq!(
        from_str.to_wire_json_ascii(),
        from_bytes.to_wire_json_ascii()
    );
    let loader = kala_contracts::load_construction_json_bytes(source.as_bytes()).expect("loader");
    assert_eq!(loader.to_wire_json_ascii(), from_str.to_wire_json_ascii());
}

#[test]
fn duplicate_profile_keys_and_nested_aliases_are_last_wins() {
    let source = r#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","profile":{"rectangle":{"width":1,"height":1},"circle":{"diameter":2}},"profile":{"circle":{"diameter":3}}},{"op":"extrude","length":2}]}]}"#;
    let script = ConstructionScript::from_json_str(source).expect("script");
    assert_eq!(
        script.components()[0].steps()[0]
            .profile()
            .and_then(Text::as_str),
        Some("circle")
    );
    assert_eq!(script.components()[0].steps()[0].diameter(), Some(3.0));
}

#[test]
fn duplicate_alias_keys_within_one_profile_are_last_wins() {
    let source = r#"{"summary":"hi","components":[{"name":"B","steps":[{"op":"sketch","profile":{"rectangle":{"width":1,"height":1},"rectangle":{"width":5,"height":6}}},{"op":"extrude","length":2}]}]}"#;
    let script = ConstructionScript::from_json_str(source).expect("script");
    let sketch = &script.components()[0].steps()[0];
    assert_eq!(sketch.profile().and_then(Text::as_str), Some("rectangle"));
    assert_eq!(sketch.width(), Some(5.0));
    assert_eq!(sketch.height(), Some(6.0));
}
