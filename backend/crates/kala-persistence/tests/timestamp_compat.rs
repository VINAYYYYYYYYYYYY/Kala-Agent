//! Live Python `datetime.fromisoformat` differential for stored timestamps.
//!
//! The persistence contract reads through [Python's native ISO parser], keeps
//! only microseconds, requires an offset and cannot represent a leap second.
//! Every case here is checked against the live product helpers rather than a
//! hand-maintained expected table.
//!
//! [Python's native ISO parser]: https://docs.python.org/3/library/datetime.html#datetime.datetime.fromisoformat

mod support;

use chrono::{DateTime, Utc};
use kala_persistence::{
    Database, EventJournal, PersistenceError, SnapshotStore, timestamp_from_iso, timestamp_to_iso,
};
use support::run_oracle;

fn valid_cases() -> Vec<&'static str> {
    vec![
        // Reviewer matrix.
        "2026-01-01T00:00:00.123456789Z",
        "2026-W01-4T00:00:00+00:00",
        "2026-01-01X00:00:00+00:00",
        "2026-01-01T00:00:00+05:30:15",
        "2026-01-01T00:00:00+05:30:15.123456",
        "2026-01-01T00:00:00,123456Z",
        // Date forms: extended calendar, basic calendar, extended/basic week.
        "2026-01-01T00:00:00+00:00",
        "20260101T000000+0000",
        "2026-W01T00:00:00+00:00",
        "2026W014T000000+0000",
        "2024-02-29T00:00:00Z",
        "2020-W53-7T00:00:00Z",
        // Time forms: hour, minute, second, comma/dot extra fractions.
        "2026-01-01T00Z",
        "2026-01-01T00:00Z",
        "2026-01-01T000000Z",
        "2026-01-01T00:00:00.1234567890123Z",
        "2026-01-01T00:00:00,1234567890123Z",
        "2026-01-01T24:00Z",
        "2026-01-01T24:00:00.000000Z",
        // C-parser leniency around trailing time characters and basic forms.
        // These fields are ignored exactly as `datetime.fromisoformat` does.
        "2026-01-01T000+00:00",
        "2026-01-01T000000000+00:00",
        "2026-01-01T000000000000x+00:00",
        "2026-01-01T00:00:00.5000000x+00:00",
        // Separator and offset spellings, including offset seconds/fractions.
        "2026-01-01 00:00:00+05",
        "2026-01-01🐍00:00:00+00:00",
        "2026-01-01T00:00:00+0530",
        "2026-01-01T00:00:00+05:3015",
        "2026-01-01T00:00:00+053015.5",
        "2026-01-01T00:00:00+00:99",
        "2026-01-01T00:00:00+00:00:99",
        "2026-01-01T00:00:00+00:00:00.5",
        "2026-01-01T00:00:00+23:59:59.999999",
        "2026-01-01T00:00:00-23:59:59.999999",
        "2026-01-01T00:00:00-00:00",
        // Python datetime year boundaries that stay representable in UTC.
        "0001-01-01T00:00:00+00:00",
        "9999-12-31T23:59:59.999999Z",
    ]
}

fn invalid_cases() -> Vec<&'static str> {
    vec![
        // Leap seconds are impossible in Python `datetime`.
        "2026-01-01T00:00:60Z",
        "2026-01-01T00:00:60.5Z",
        // 24:00 is only valid with every other time component zero.
        "2026-01-01T24:00:01Z",
        "2026-01-01T24:01Z",
        "2026-01-01T24:00:00.000001Z",
        // The persistence reader requires an offset.
        "2026-01-01T00:00:00",
        "2026-01-01",
        // Offset domain is strictly (-24h, 24h).
        "2026-01-01T00:00:00+24:00",
        "2026-01-01T00:00:00-24:00",
        "2026-01-01T00:00:00+99:00",
        // Invalid calendar/week dates.
        "2026-13-01T00:00:00Z",
        "2026-02-29T00:00:00Z",
        "2025-W53-1T00:00:00Z",
        "2026-W00-1T00:00:00Z",
        "2026-W01-8T00:00:00Z",
        "0000-01-01T00:00:00Z",
        // Converting the local wall time to UTC overflows Python's year range.
        "0001-01-01T00:00:00+00:01",
        "9999-12-31T23:59:59-00:01",
        // Malformed spellings.
        "2026-01-01t00:00:00z",
        "2026-01-01T00:00:00+00:00Z",
        "2026-1-01T00:00:00Z",
        "2026-01-1T00:00:00Z",
        "2026-01-01T0:00:00Z",
        "2026-01-01T00:00:00+00:00 junk",
        " 2026-01-01T00:00:00+00:00",
        "2026-01-01T00:00:00+00:00 ",
        "2026-01-01T00:00:00+5",
        "2026-01-01T00:00:00+05:3015",
        "2026-01-01T00:00:00+05.5",
        "2026-01-01T00:00:00+00:000",
        "2026-01-01T00:00:00.5x+00:00",
        "not-a-timestamp",
        "",
    ]
}

#[test]
fn reader_matches_live_python_for_representative_domain() {
    let mut sources: Vec<String> = valid_cases()
        .into_iter()
        .chain(invalid_cases())
        .map(str::to_owned)
        .collect();
    sources.sort();
    sources.dedup();
    let request = serde_json::to_vec(&sources).expect("serialize oracle request");
    let results = run_oracle(&["timestamps"], &request);
    let results = results.as_array().expect("oracle result array");
    assert_eq!(results.len(), sources.len());

    for (source, result) in sources.iter().zip(results) {
        let python_ok = result["ok"].as_bool().expect("ok flag");
        let rust = timestamp_from_iso(source);
        if python_ok {
            let expected = result["output"].as_str().expect("oracle output");
            let parsed = rust.unwrap_or_else(|error| {
                panic!("Rust rejected a Python-accepted timestamp {source:?}: {error:?}")
            });
            assert_eq!(
                timestamp_to_iso(&parsed).expect("format parsed timestamp"),
                expected,
                "UTC microsecond value differs for {source:?}"
            );
        } else {
            assert!(
                rust.is_err(),
                "Rust accepted a Python-rejected timestamp {source:?}"
            );
        }
    }
}

#[test]
fn generated_form_matrix_matches_live_python() {
    let dates = [
        "2026-01-01",
        "20260101",
        "2026-W01-4",
        "2026W014",
        "2026-W01",
        "2024-02-29",
        "9999-12-31",
        "2025-W53-1",
        "0000-01-01",
    ];
    let separators = ["T", " ", "X", "🐍"];
    let times = [
        "00",
        "00:00",
        "00:00:00",
        "000000",
        "24:00",
        "23:59:59",
        "00:00:00.123456789",
        "00:00:00,5",
        "00:00:60",
        "24:00:01",
        "24:00:00.000001",
        "00:000",
        "000",
        "0112345",
        "00:00:00.5x",
    ];
    let offsets = [
        "+00:00",
        "Z",
        "+05:30",
        "-05:30",
        "+05:30:15",
        "+053015.5",
        "+00:99",
        "+23:59:59.999999",
        "-23:59:59.999999",
        "+24:00",
        "+5",
        "",
        "+00:000",
        "+00:00:99",
        "+00:00:00.5",
        "-00:00:00.000001",
        "+00000",
    ];
    let mut sources = Vec::new();
    for date in dates {
        for separator in separators {
            for time in times {
                for offset in offsets {
                    sources.push(format!("{date}{separator}{time}{offset}"));
                }
            }
        }
    }
    let request = serde_json::to_vec(&sources).expect("serialize oracle request");
    let results = run_oracle(&["timestamps"], &request);
    let results = results.as_array().expect("oracle result array");
    assert_eq!(results.len(), sources.len());
    let mut mismatches = Vec::new();
    for (source, result) in sources.iter().zip(results) {
        let python_ok = result["ok"].as_bool().expect("ok flag");
        let rust = timestamp_from_iso(source);
        if python_ok {
            match rust {
                Ok(parsed) => {
                    let expected = result["output"].as_str().expect("oracle output");
                    let actual = timestamp_to_iso(&parsed).expect("format");
                    if actual != expected {
                        mismatches.push(format!("{source:?}: value {actual:?} != {expected:?}"));
                    }
                }
                Err(error) => mismatches.push(format!(
                    "{source:?}: Rust rejected Python-accepted value: {error:?}"
                )),
            }
        } else if rust.is_ok() {
            mismatches.push(format!("{source:?}: Rust accepted a Python-rejected value"));
        }
        if mismatches.len() >= 20 {
            break;
        }
    }
    assert!(
        mismatches.is_empty(),
        "timestamp mismatches:\n{}",
        mismatches.join("\n")
    );
}

#[test]
fn writer_uses_python_precision_and_refuses_leap_seconds() {
    let parsed = timestamp_from_iso("2026-01-01T00:00:00.123456789Z").expect("parse");
    assert_eq!(parsed.timestamp_subsec_nanos(), 123_456_000);
    assert_eq!(
        timestamp_to_iso(&parsed).expect("format"),
        "2026-01-01T00:00:00.123456Z"
    );

    let leap = DateTime::parse_from_rfc3339("2026-01-01T00:00:60+00:00")
        .expect("chrono leap")
        .with_timezone(&Utc);
    assert!(matches!(
        timestamp_to_iso(&leap),
        Err(PersistenceError::TimestampLeapSecond)
    ));

    let database = Database::in_memory().expect("open");
    let journal = EventJournal::new(&database).expect("journal");
    assert!(matches!(
        journal.append_with(
            "step.started",
            "step-1",
            r#"{}"#,
            &kala_persistence::AppendOptions::default().with_occurred_at(leap),
        ),
        Err(PersistenceError::TimestampLeapSecond)
    ));
    assert!(matches!(
        SnapshotStore::new(&database).save("snapshot-1", "workflow-1", r#"{}"#, Some(leap)),
        Err(PersistenceError::TimestampLeapSecond)
    ));
    assert_eq!(journal.verify_chain().expect("empty verify").event_count, 0);
}
