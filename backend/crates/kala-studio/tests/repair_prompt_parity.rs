//! Cross-language byte comparison of the validation-repair prompt.
//!
//! The Rust product and the Python reference build the same prompt for the
//! same prompt, error and previous reply. This test asks the live Python
//! reference for its prompt and compares the bytes. The Python reference is
//! the oracle; the production runtime never invokes it.
//!
//! When no Python interpreter is available the test prints a loud skip: a
//! skip is never counted as byte-compare evidence.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use kala_studio::chat_design::{build_repair_prompt, repair_prompt};

#[derive(Debug)]
struct Case {
    prompt: String,
    error: String,
    previous: String,
    build_prompt: String,
    build_feedback: String,
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// The Python interpreter the oracle runs under.
///
/// `KALA_PYTHON` wins, exactly like the other live Python differentials in
/// this workspace.
fn python_executable() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("KALA_PYTHON") {
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    let venv = repository_root().join(".venv/bin/python");
    if venv.is_file() {
        return Some(venv);
    }
    None
}

/// Ask the Python reference for the prompts, or `None` when it cannot run.
fn oracle(payload: &serde_json::Value) -> Option<serde_json::Value> {
    let Some(python) = python_executable() else {
        eprintln!(
            "SKIPPED LOUDLY: no Python interpreter is available on this host. The \
             cross-language byte comparison of the validation-repair prompt did NOT run."
        );
        return None;
    };
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/repair_prompt_oracle.py");
    let mut child = match Command::new(&python)
        .arg("-I")
        .arg(&script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            eprintln!(
                "SKIPPED LOUDLY: the Python interpreter {python:?} could not be started \
                 ({error}). The cross-language byte comparison of the validation-repair \
                 prompt did NOT run."
            );
            return None;
        }
    };
    let input = serde_json::to_vec(payload).expect("oracle payload");
    child
        .stdin
        .as_mut()
        .expect("oracle stdin")
        .write_all(&input)
        .expect("write oracle input");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("run Python oracle");
    assert!(
        output.status.success(),
        "the Python oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(serde_json::from_slice(&output.stdout).expect("the oracle prints one JSON document"))
}

fn cases() -> Vec<Case> {
    let feedback = "the script you sent:\n{\"components\":[]}\n\nthe build reported:\n- the check 'cuts_effective' (expected 1, measured 0)\n- warning: component 'plate': pocket at step index 3 on XY removed no material";
    vec![
        Case {
            prompt: "aluminium mounting plate 80 x 40 x 8 mm with four 6 mm through holes 8 mm from the edges".to_owned(),
            error: "script.components.0.steps.0.op: Input tag 'operation' found using 'op' does not match any of the expected tags: 'sketch', 'extrude', 'pocket', 'revolve'".to_owned(),
            previous: "Here is the flange.\n\n```json\n{\"script\": {\"summary\": \"flange\", \"components\": [{\"name\": \"flange\", \"steps\": [{\"operation\": \"extrude\", \"length\": 8}]}]}}\n```".to_owned(),
            build_prompt: "a plate 80 x 40 x 8 mm".to_owned(),
            build_feedback: feedback.to_owned(),
        },
        Case {
            prompt: "a stepped round post 30/20 mm".to_owned(),
            error: "template.l_bracket.width: Input should be greater than 0".to_owned(),
            previous: "```json\n{\"template\": {\"template\": \"l_bracket\", \"width\": -3, \"depth\": 45, \"height\": 40, \"wall\": 8}}\n```".to_owned(),
            build_prompt: "a plate 80 x 40 x 8 mm".to_owned(),
            build_feedback: "the script you sent:\n{}\n\nthe builder refused the script:\ndesign_rejected: the design could not be compiled or measured (the profile is not supported)".to_owned(),
        },
        Case {
            prompt: "a 40 x 30 x 5 mm base with a 12 mm boss on top".to_owned(),
            error: "script: Field required".to_owned(),
            previous: "The base and the boss:\n{\"script\": {\"summary\": \"base\", \"components\": []}}".to_owned(),
            build_prompt: "a plate 80 x 40 x 8 mm".to_owned(),
            build_feedback: feedback.to_owned(),
        },
        Case {
            prompt: "a plate 80 x 40 x 8 mm".to_owned(),
            error: "script.components: Field required".to_owned(),
            previous: "```json\n{\"script\": {\n```".to_owned(),
            build_prompt: "a plate 80 x 40 x 8 mm".to_owned(),
            build_feedback: feedback.to_owned(),
        },
        Case {
            prompt: "a plate 80 x 40 x 8 mm".to_owned(),
            error: "a design intent must carry exactly one of template or script".to_owned(),
            previous: "```json\n{\"template\": {\"template\": \"housing_envelope\", \"width\": 40, \"depth\": 30, \"height\": 25, \"wall\": 4}}\n```".to_owned(),
            build_prompt: "a plate 80 x 40 x 8 mm".to_owned(),
            build_feedback: feedback.to_owned(),
        },
        Case {
            prompt: "a plate 80 x 40 x 8 mm".to_owned(),
            error: "script.components: Field required".to_owned(),
            previous: format!(
                "```json\n{{\"script\": {{\"summary\": \"{}\"}}\n```",
                "x".repeat(kala_studio::chat_design::MAX_REPAIR_PREVIOUS_CHARS + 5_000)
            ),
            build_prompt: "a plate 80 x 40 x 8 mm".to_owned(),
            build_feedback: feedback.to_owned(),
        },
    ]
}

#[test]
fn the_validation_repair_prompt_is_byte_identical_in_both_languages() {
    let cases = cases();
    let payload = serde_json::json!({
        "cases": cases
            .iter()
            .map(|case| serde_json::json!({
                "prompt": case.prompt,
                "error": case.error,
                "previous": case.previous,
                "build_prompt": case.build_prompt,
                "build_feedback": case.build_feedback,
            }))
            .collect::<Vec<_>>(),
    });
    let Some(output) = oracle(&payload) else {
        return;
    };
    let schema = output["schema_text"]
        .as_str()
        .expect("the oracle prints the schema text");
    let prompts = output["prompts"]
        .as_array()
        .expect("the oracle prints one prompt per case");
    assert_eq!(prompts.len(), cases.len());
    let mut compared = 0usize;
    for (case, python) in cases.iter().zip(prompts) {
        let python = python.as_str().expect("a prompt is text");
        let rust = repair_prompt(&case.prompt, &case.error, schema, &case.previous);
        assert_eq!(
            rust, python,
            "the Rust and the Python validation-repair prompts differ for the case \
             {case:?}"
        );
        compared += rust.len();
    }
    assert!(compared > 0, "the comparison ran on real bytes");
    eprintln!(
        "the validation-repair prompt is byte-identical in both languages: {} cases, \
         {compared} bytes compared",
        cases.len()
    );
}

#[test]
fn the_build_repair_prompt_is_still_byte_identical_in_both_languages() {
    let cases = cases();
    let payload = serde_json::json!({
        "cases": cases
            .iter()
            .map(|case| serde_json::json!({
                "prompt": case.prompt,
                "error": case.error,
                "previous": case.previous,
                "build_prompt": case.build_prompt,
                "build_feedback": case.build_feedback,
            }))
            .collect::<Vec<_>>(),
    });
    let Some(output) = oracle(&payload) else {
        return;
    };
    let schema = output["schema_text"]
        .as_str()
        .expect("the oracle prints the schema text");
    let prompts = output["build_prompts"]
        .as_array()
        .expect("the oracle prints one build prompt per case");
    assert_eq!(prompts.len(), cases.len());
    let mut compared = 0usize;
    for (case, python) in cases.iter().zip(prompts) {
        let python = python.as_str().expect("a prompt is text");
        let rust = build_repair_prompt(&case.build_prompt, &case.build_feedback, schema);
        assert_eq!(
            rust, python,
            "the Rust and the Python build-repair prompts differ for the case {case:?}"
        );
        compared += rust.len();
    }
    assert!(compared > 0, "the comparison ran on real bytes");
    eprintln!(
        "the build-repair prompt is byte-identical in both languages: {} cases, {compared} \
         bytes compared",
        cases.len()
    );
}

/// The 6 000+ character design reply the display test uses: 70 Ø3 holes.
fn long_design_reply() -> String {
    let mut steps = vec![
        serde_json::json!({"op": "sketch", "plane": "XY", "profile": "rectangle", "width": 200, "height": 200}),
        serde_json::json!({"op": "extrude", "length": 5}),
    ];
    for index in 0..70 {
        steps.push(serde_json::json!({
            "op": "sketch",
            "plane": "XY",
            "profile": "circle",
            "diameter": 3,
            "x": 20 + 4 * (index % 10),
            "y": 20 + 4 * (index / 10),
        }));
        steps.push(serde_json::json!({"op": "pocket"}));
    }
    let payload = serde_json::json!({
        "script": {
            "summary": "200 x 200 x 5 mm plate with 70 through holes",
            "components": [{"name": "plate", "steps": steps}],
        }
    });
    format!(
        "Here is the plate.\n\n```json\n{}\n```",
        serde_json::to_string(&payload).expect("payload")
    )
}

/// The displayed text and the parsed design agree across the two languages.
///
/// The reply is longer than the 4 096-character display bound, so the test
/// also proves the design is read from the full reply, not from the display.
#[test]
fn the_long_reply_display_and_design_are_byte_identical_in_both_languages() {
    let reply = long_design_reply();
    assert!(reply.chars().count() > 6_000, "{}", reply.chars().count());

    let payload = serde_json::json!({ "cases": [], "display_reply": reply });
    let Some(document) = oracle(&payload) else {
        return;
    };

    let expected_display: String = kala_design_intent::strip_design_json_block(&reply)
        .chars()
        .take(4_096)
        .collect();
    assert_eq!(
        document["display_text"].as_str().expect("display text"),
        expected_display,
        "the displayed reply differs between Rust and Python"
    );

    let intent =
        kala_design_intent::parse_design_intent_text(&reply).expect("the long design parses");
    let steps = intent.script().expect("a script").components()[0]
        .steps()
        .len();
    assert_eq!(steps, 142);
    assert_eq!(
        document["display_steps"].as_u64(),
        Some(steps as u64),
        "the parsed design differs between Rust and Python"
    );
}
