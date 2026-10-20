//! Focused R2 regressions for permissive model JSON and full uniqueness.
//!
//! These tests exercise the same entry points HTTP/repository consumers use
//! (`from_json_str`, `from_json_bytes`, the construction loaders) rather than
//! building a `CanonicalValue` first, because the ordered input path is the one
//! under test.

use kala_contracts::{
    CanonicalJsonObject, ConstructionScript, ContractModel, DesignSpec, EvaluationResult, PlanStep,
    Text, ToolCall, ToolResult, load_construction_json_bytes, load_construction_json_str,
    load_reading_json_bytes, load_reading_json_str,
};

const AT: &str = "2026-01-02T03:04:05Z";

fn spec(constraints: &str) -> String {
    format!(r#"{{"id":"spec","name":"probe","summary":"probe","constraints":{constraints}}}"#)
}

fn evaluation(findings: &str) -> String {
    format!(
        r#"{{"evaluation_id":"e","subject_id":"s","passed":true,"findings":{findings},"evaluated_at":"{AT}"}}"#
    )
}

fn texts(values: &[Text]) -> Vec<String> {
    values.iter().map(|value| value.to_string_lossy()).collect()
}

#[test]
fn nonadjacent_and_normalized_duplicates_are_refused() {
    for constraints in [r#"["a","b","a"]"#, r#"[" a ","b","a"]"#, r#"["a","a","b"]"#] {
        let error = DesignSpec::from_json_str(&spec(constraints)).expect_err(constraints);
        assert_eq!(
            error.message(),
            "collection values must be unique",
            "{constraints}"
        );
    }
    for findings in [r#"["x","y","x"]"#, r#"[" x ","y","x"]"#, r#"["x","x","y"]"#] {
        let error = EvaluationResult::from_json_str(&evaluation(findings)).expect_err(findings);
        assert_eq!(
            error.message(),
            "collection values must be unique",
            "{findings}"
        );
    }

    let spec = DesignSpec::from_json_str(&spec(r#"["b","a","c"]"#)).expect("ordered constraints");
    assert_eq!(texts(spec.constraints()), ["b", "a", "c"]);
    let result =
        EvaluationResult::from_json_str(&evaluation(r#"["b","a","c"]"#)).expect("ordered findings");
    assert_eq!(texts(result.findings()), ["b", "a", "c"]);

    let artifacts = format!(
        r#"{{"evaluation_id":"e","subject_id":"s","passed":true,"artifact_ids":["a","b","a"],"evaluated_at":"{AT}"}}"#
    );
    assert_eq!(
        EvaluationResult::from_json_str(&artifacts)
            .expect_err("nonadjacent artifact duplicate")
            .message(),
        "collection values must be unique"
    );
}

#[test]
fn private_number_marker_objects_are_plain_user_data() {
    let cases = [
        (
            r#"{"$serde_json::private::Number":"42"}"#,
            r#"{"$serde_json::private::Number":"42"}"#,
        ),
        (
            r#"{"$serde_json::private::Number":42}"#,
            r#"{"$serde_json::private::Number":42}"#,
        ),
        (
            r#"{"\u0024serde_json::private::Number":"42","sibling":1}"#,
            r#"{"$serde_json::private::Number":"42","sibling":1}"#,
        ),
        (
            r#"{"outer":{"$serde_json::private::Number":"7"},"items":[{"$serde_json::private::Number":8,"x":true}]}"#,
            r#"{"items":[{"$serde_json::private::Number":8,"x":true}],"outer":{"$serde_json::private::Number":"7"}}"#,
        ),
    ];
    for (attributes, expected) in cases {
        let source = format!(
            r#"{{"id":"spec","name":"probe","summary":"probe","attributes_json":{attributes}}}"#
        );
        let spec = DesignSpec::from_json_str(&source).expect(attributes);
        assert_eq!(spec.attributes_json().as_str(), expected, "{attributes}");
    }

    let plan = PlanStep::from_json_str(
        r#"{"step_id":"s","catalog_step_id":"c","intent":"i","parameters_json":{"$serde_json::private::Number":"42"}}"#,
    )
    .expect("plan marker object");
    assert_eq!(
        plan.parameters_json().as_str(),
        r#"{"$serde_json::private::Number":"42"}"#
    );

    let call = ToolCall::from_json_str(&format!(
        r#"{{"call_id":"c","tool_id":"t","tool_version":"1","side_effect_level":"read","timeout_seconds":1,"issued_at":"{AT}","arguments_json":{{"$serde_json::private::Number":42}}}}"#
    ))
    .expect("tool-call marker object");
    assert_eq!(
        call.arguments_json().as_str(),
        r#"{"$serde_json::private::Number":42}"#
    );

    let result = ToolResult::from_json_str(
        r#"{"call_id":"c","tool_id":"t","status":"succeeded","result_json":{"nested":{"$serde_json::private::Number":"42"}}}"#,
    )
    .expect("tool-result marker object");
    assert_eq!(
        result.result_json().expect("result").as_str(),
        r#"{"nested":{"$serde_json::private::Number":"42"}}"#
    );

    let string_source = r#"{"id":"spec","name":"probe","summary":"probe","attributes_json":"{\"$serde_json::private::Number\":\"42\",\"sibling\":1}"}"#;
    let spec = DesignSpec::from_json_str(string_source).expect("marker in canonical string");
    assert_eq!(
        spec.attributes_json().as_str(),
        r#"{"$serde_json::private::Number":"42","sibling":1}"#
    );
}

#[test]
fn duplicate_surrogate_and_arbitrary_integer_input_coexist() {
    let source = r#"{"id":"spec","id":"last","name":"probe","summary":"probe","attributes_json":{"text":"\ud800","big":1180591620717411303424,"$serde_json::private::Number":"42"}}"#;
    let spec = DesignSpec::from_json_str(source).expect("str path");
    let bytes = DesignSpec::from_json_bytes(source.as_bytes()).expect("bytes path");
    assert_eq!(spec, bytes);
    assert_eq!(spec.id(), "last");
    assert!(
        spec.attributes_json()
            .as_str()
            .contains(r#""text":"\ud800""#)
    );
    assert!(
        spec.attributes_json()
            .as_str()
            .contains("1180591620717411303424")
    );
    assert!(
        spec.attributes_json()
            .as_str()
            .contains(r#""$serde_json::private::Number":"42""#)
    );
    assert!(spec.to_wire_json_ascii().contains(r#"\\ud800"#));
    assert!(spec.to_wire_json().is_ok());
    assert!(spec.canonical_digest().is_ok());

    // Escaped key spelling is the same code-point key, and the last value
    // wins while the first position is retained.
    let escaped = r#"{"i\u0064":"first","id":"last","name":"probe","summary":"probe","attributes_json":{"text":"a\ud800b"}}"#;
    assert_eq!(
        DesignSpec::from_json_str(escaped).expect("escaped id").id(),
        "last"
    );
}

#[test]
fn persisted_canonical_string_duplicates_stay_strict() {
    let source =
        r#"{"id":"spec","name":"probe","summary":"probe","attributes_json":"{\"a\":1,\"a\":2}"}"#;
    assert!(DesignSpec::from_json_str(source).is_err());
    assert!(CanonicalJsonObject::parse(r#"{"a":1,"a":2}"#).is_err());

    // An ordinary parsed object has already collapsed duplicates before the
    // CanonicalJsonObject validator receives it, exactly like Python.
    let collapsed =
        r#"{"id":"spec","name":"probe","summary":"probe","attributes_json":{"a":1,"a":2}}"#;
    let spec = DesignSpec::from_json_str(collapsed).expect("collapsed duplicate object");
    assert_eq!(spec.attributes_json().as_str(), r#"{"a":2}"#);
}

#[test]
fn duplicate_surrogate_and_profile_order_work_through_all_constructors() {
    let rectangle_first = r#"{"summary":"first","summary":"last\ud800","components":[{"name":"B","steps":[{"op":"sketch","profile":{"rectangle":{"width":10,"height":4},"circle":{"diameter":8}}},{"op":"extrude","length":2}]}]}"#;
    let circle_first = r#"{"summary":"first","summary":"last","components":[{"name":"B","steps":[{"op":"sketch","profile":{"circle":{"diameter":8},"rectangle":{"width":10,"height":4}}},{"op":"extrude","length":2}]}]}"#;

    for script in [
        ConstructionScript::from_json_str(rectangle_first).expect("str"),
        ConstructionScript::from_json_bytes(rectangle_first.as_bytes()).expect("bytes"),
        load_construction_json_str(rectangle_first).expect("loader str"),
        load_construction_json_bytes(rectangle_first.as_bytes()).expect("loader bytes"),
    ] {
        assert!(script.summary().code_points().ends_with(&[0xD800]));
        let sketch = &script.components()[0].steps()[0];
        assert_eq!(sketch.profile().and_then(Text::as_str), Some("rectangle"));
        assert_eq!(sketch.width(), Some(10.0));
    }
    let circle = load_construction_json_bytes(circle_first.as_bytes()).expect("circle loader");
    assert_eq!(
        circle.components()[0].steps()[0]
            .profile()
            .and_then(Text::as_str),
        Some("circle")
    );

    let escaped_alias = r#"{"op":"bogus","op":"sketch","profile":{"rec\u0074angle":{"width":10,"height":4},"circle":{"diameter":8}}}"#;
    let step = kala_contracts::ComponentStep::from_json_str(escaped_alias).expect("escaped alias");
    assert_eq!(step.profile().and_then(Text::as_str), Some("rectangle"));

    let reading = r#"{"verdict":"first","verdict":"last\ud800","rewrite":{"name":"B","steps":[{"op":"sketch","profile":{"circle":{"diameter":8},"rectangle":{"width":10,"height":4}}},{"op":"extrude","length":2}]}}"#;
    for reading in [
        load_reading_json_str(reading).expect("reading str"),
        load_reading_json_bytes(reading.as_bytes()).expect("reading bytes"),
    ] {
        assert!(reading.verdict().code_points().ends_with(&[0xD800]));
        assert_eq!(
            reading.rewrite().expect("rewrite").steps()[0]
                .profile()
                .and_then(Text::as_str),
            Some("circle")
        );
    }
}

#[test]
fn deep_raw_input_is_stack_safe_and_profile_hints_stay_bounded() {
    const DEPTH: usize = 50_000;
    let mut source =
        String::from(r#"{"id":"spec","name":"probe","summary":"probe","attributes_json":"#);
    source.push_str(&r#"{"a":"#.repeat(DEPTH));
    source.push_str(r#"{"dup":1,"dup":2,"text":"\ud800"}"#);
    source.push_str(&"}".repeat(DEPTH));
    source.push('}');
    let spec = DesignSpec::from_json_str(&source).expect("deep attributes");
    let attributes = spec.attributes_json().as_str();
    assert!(attributes.contains(r#""dup":2"#));
    assert!(attributes.contains(r#""text":"\ud800""#));

    // A deeply nested body inside an unused profile alias is not walked by
    // the profile hint, and a chosen alias may itself carry an ignored nested
    // extra subtree.
    let mut step =
        String::from(r#"{"op":"sketch","profile":{"rectangle":{"width":1,"height":1,"ignored":"#);
    step.push_str(&r#"{"a":"#.repeat(DEPTH));
    step.push('1');
    step.push_str(&"}".repeat(DEPTH));
    step.push_str(r#"},"circle":{"diameter":8}}}"#);
    let parsed = kala_contracts::ComponentStep::from_json_str(&step).expect("deep profile body");
    assert_eq!(parsed.profile().and_then(Text::as_str), Some("rectangle"));
    assert_eq!(parsed.width(), Some(1.0));

    // A huge arbitrary extra subtree must not be walked to find `profile`.
    let mut step = String::from(
        r#"{"op":"sketch","profile":{"rectangle":{"width":1,"height":1},"circle":{"diameter":2}},"extra":"#,
    );
    step.push_str(&r#"{"a":"#.repeat(DEPTH));
    step.push('1');
    step.push_str(&"}".repeat(DEPTH));
    step.push('}');
    let error = kala_contracts::ComponentStep::from_json_str(&step)
        .expect_err("extra field must still be refused");
    assert_eq!(error.message(), "Extra inputs are not permitted");
}
