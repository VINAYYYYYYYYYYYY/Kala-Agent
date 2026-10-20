//! A test-only helper that builds the jail and reports what it can see.
//!
//! The helper is used by the differential test to compare the jail that the
//! Python implementation builds with the jail that this crate builds. It is
//! not a product entry point and the library never requires it to exist.

use kala_sandbox::{
    JsonValue, child_pre_exec_or_exit, isolate_process, posix_text, probe_entry, start_recording,
    take_recorded_calls, verify_controls,
};

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() < 3 {
        eprintln!("the probe payload is malformed");
        std::process::exit(2);
    }
    let (spec, canary) = match probe_entry(&arguments[1], &arguments[2]) {
        Ok(entry) => entry,
        Err(error) => {
            eprintln!("{}", error.detail());
            std::process::exit(2);
        }
    };
    if std::env::var_os("KALA_SANDBOX_PRE_EXEC").is_some() {
        // The pre-exec hook builds the jail in this process and terminates
        // the process when the jail cannot be established.
        child_pre_exec_or_exit(Some(&spec));
        println!(
            "{}",
            JsonValue::Object(vec![(
                "pre_exec".to_owned(),
                JsonValue::Text("established".to_owned()),
            )])
            .dump()
        );
        return;
    }
    start_recording();
    let outcome = isolate_process(&spec);
    let calls = take_recorded_calls();
    let mut document: Vec<(String, JsonValue)> = Vec::new();
    match outcome {
        Ok(established) => {
            let (verified, failed) = verify_controls(&spec, &canary, &posix_text(&spec.workspace));
            document.push(("established".to_owned(), text_list(&established)));
            document.push(("verified".to_owned(), text_list(&verified)));
            document.push(("failed".to_owned(), text_list(&failed)));
            document.push(("root_entries".to_owned(), root_entries()));
            document.push((
                "pid_is_one".to_owned(),
                JsonValue::Bool(std::process::id() == 1),
            ));
        }
        Err(error) => {
            document.push((
                "error".to_owned(),
                JsonValue::Text(error.detail().to_owned()),
            ));
        }
    }
    document.push((
        "calls".to_owned(),
        JsonValue::Array(
            calls
                .into_iter()
                .map(|(operation, arguments)| {
                    JsonValue::Array(vec![
                        JsonValue::Text(operation),
                        JsonValue::Array(
                            arguments
                                .into_iter()
                                .map(|argument| match argument {
                                    serde_json::Value::String(text) => JsonValue::Text(text),
                                    serde_json::Value::Null => JsonValue::Null,
                                    serde_json::Value::Number(number) => {
                                        JsonValue::Integer(number.as_i64().unwrap_or_default())
                                    }
                                    other => JsonValue::Text(other.to_string()),
                                })
                                .collect(),
                        ),
                    ])
                })
                .collect(),
        ),
    ));
    println!("{}", JsonValue::Object(document).dump());
}

fn text_list(values: &[String]) -> JsonValue {
    JsonValue::Array(
        values
            .iter()
            .map(|value| JsonValue::Text(value.clone()))
            .collect(),
    )
}

fn root_entries() -> JsonValue {
    let mut entries: Vec<String> = Vec::new();
    if let Ok(directory) = std::fs::read_dir("/") {
        for entry in directory.flatten() {
            entries.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    entries.sort();
    text_list(&entries)
}
