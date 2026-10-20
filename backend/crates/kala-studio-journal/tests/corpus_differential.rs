//! The randomized deterministic corpus differential.
//!
//! The same deterministic corpus of operations is run twice: once against the
//! native journal and once against the repository's Python journal, in two
//! separate directories. Every response is compared, and the two journal
//! files are compared row by row at the end, so the two implementations must
//! produce the same file, the same chain digests and the same answers.
//!
//! The corpus is seeded, so a failure is reproducible: the seed and the
//! corpus size are constants of this file.

mod support;

use std::path::Path;

use kala_persistence::timestamp_from_iso;
use kala_studio_journal::{AppendOptions, StudioSessionJournal};
use serde_json::{Value, json};
use support::{PythonOracle, base64_encode};
use tempfile::TempDir;

const SEED: u64 = 0x4b41_4c41_2d4a_4f55;

const OPERATION_COUNT: usize = 240;

/// The payload pool. Every entry is a JSON object with no duplicate key, no
/// non-finite number and no deep nesting, which is the domain both
/// implementations accept.
const PAYLOADS: [&str; 18] = [
    r#"{}"#,
    r#"{"text":"hello"}"#,
    r#"{"b":1,"a":2}"#,
    r#"{"z":1,"a":{"n":null,"m":true}}"#,
    r#"{"index":0}"#,
    r#"{"index":1}"#,
    r#"{"nested":{"deep":{"deeper":[1,2,3]}}}"#,
    r#"{"unicode":"café über \u00e9"}"#,
    r#"{"emoji":"\ud83d\ude00 family \ud83c\udf1f"}"#,
    r#"{"big":1180591620717411303424}"#,
    r#"{"negative":-1180591620717411303424}"#,
    r#"{"float":35571.416047}"#,
    r#"{"tiny":1e-05}"#,
    r#"{"big_float":1e+20}"#,
    r#"{"zero":0.0,"neg_zero":-0.0}"#,
    r#"{"array":[1,"two",3.0,null,true,{"six":6}]}"#,
    r#"{"kind":"step","sha256":"0000000000000000000000000000000000000000000000000000000000000000","size_bytes":42}"#,
    r#"{"kind":"step","sha256":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","size_bytes":0}"#,
];

const EVENT_TYPES: [&str; 7] = [
    "design.intent",
    "design.compiled",
    "design.measured",
    "design.checked",
    "artifact.stored",
    "export.completed",
    "chat.turn",
];

const AGGREGATE_IDS: [&str; 3] = ["studio", "design-42", "turn.7"];

/// A deterministic xorshift generator, so the corpus is the same on every run
/// and on every platform.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value >> 12;
        value ^= value << 25;
        value ^= value >> 27;
        self.0 = value;
        value.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

/// One operation of the corpus.
#[derive(Clone, Debug)]
enum Op {
    Append {
        event_type: String,
        payload: String,
        aggregate_id: String,
        timestamp: String,
    },
    Page {
        after: i64,
        limit: i64,
    },
    Get {
        sequence: i64,
    },
    Verify,
    HeadDigest,
    PutArtifact {
        data: Vec<u8>,
    },
    GetArtifact {
        digest: String,
    },
    HasArtifact {
        digest: String,
    },
}

/// Return the ISO-8601 stamp of the given microsecond offset.
///
/// Every third stamp is written with a non-UTC offset, so the corpus covers
/// the conversion to UTC as well as the UTC form.
fn stamp(microseconds: i64) -> String {
    let base = chrono::DateTime::parse_from_rfc3339("2026-10-20T12:00:00Z")
        .expect("a timestamp")
        .with_timezone(&chrono::Utc);
    let value = base + chrono::TimeDelta::microseconds(microseconds);
    match microseconds.rem_euclid(3) {
        0 => value.to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
        1 => (value + chrono::TimeDelta::hours(2))
            .to_rfc3339_opts(chrono::SecondsFormat::Micros, false),
        _ => (value - chrono::TimeDelta::hours(5))
            .to_rfc3339_opts(chrono::SecondsFormat::Micros, false),
    }
}

/// Return the SHA-256 hex digest of the given bytes.
fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Build the deterministic corpus.
fn build_corpus() -> Vec<Op> {
    let mut rng = Rng(SEED);
    let mut corpus = Vec::with_capacity(OPERATION_COUNT);
    let mut appends = 0_i64;
    for _ in 0..OPERATION_COUNT {
        match rng.below(100) {
            0..=39 => {
                appends += 1;
                corpus.push(Op::Append {
                    event_type: EVENT_TYPES[rng.below(EVENT_TYPES.len())].to_owned(),
                    payload: PAYLOADS[rng.below(PAYLOADS.len())].to_owned(),
                    aggregate_id: AGGREGATE_IDS[rng.below(AGGREGATE_IDS.len())].to_owned(),
                    timestamp: stamp(rng.below(1_000_000) as i64),
                });
            }
            40..=59 => corpus.push(Op::Page {
                after: rng.below(12) as i64,
                limit: 1 + rng.below(600) as i64,
            }),
            60..=74 => corpus.push(Op::Get {
                sequence: 1 + rng.below(12) as i64,
            }),
            75..=84 => corpus.push(Op::Verify),
            85..=89 => corpus.push(Op::HeadDigest),
            90..=94 => {
                let size = rng.below(64);
                let data: Vec<u8> = (0..size).map(|index| (index * 7 + 3) as u8).collect();
                corpus.push(Op::PutArtifact { data });
            }
            95..=97 => {
                let size = rng.below(64);
                let data: Vec<u8> = (0..size).map(|index| (index * 7 + 3) as u8).collect();
                corpus.push(Op::GetArtifact {
                    digest: sha256_hex(&data),
                });
            }
            _ => {
                let size = rng.below(64);
                let data: Vec<u8> = (0..size).map(|index| (index * 7 + 3) as u8).collect();
                corpus.push(Op::HasArtifact {
                    digest: sha256_hex(&data),
                });
            }
        }
    }
    assert!(appends > 0, "the corpus must contain appends");
    corpus
}

/// The request the oracle receives for one operation.
fn oracle_request(op: &Op) -> Value {
    match op {
        Op::Append {
            event_type,
            payload,
            aggregate_id,
            timestamp,
        } => json!({
            "op": "append",
            "event_type": event_type,
            "payload_text": payload,
            "aggregate_id": aggregate_id,
            "occurred_at": timestamp,
        }),
        Op::Page { after, limit } => {
            json!({"op": "page", "after_sequence": after, "limit": limit})
        }
        Op::Get { sequence } => json!({"op": "get", "sequence": sequence}),
        Op::Verify => json!({"op": "verify"}),
        Op::HeadDigest => json!({"op": "head_digest"}),
        Op::PutArtifact { data } => json!({
            "op": "put_artifact",
            "data_base64": base64_encode(data),
        }),
        Op::GetArtifact { digest } => json!({"op": "get_artifact", "digest": digest}),
        Op::HasArtifact { digest } => json!({"op": "has_artifact", "digest": digest}),
    }
}

fn native_event(event: &kala_studio_journal::SessionEvent) -> Value {
    let canonical = event
        .payload
        .as_ref()
        .map(|value| kala_persistence::canonicalize_object_value(value).expect("canonical"));
    json!({
        "sequence": event.sequence,
        "event_type": event.event_type,
        "aggregate_id": event.aggregate_id,
        "payload": canonical,
        "payload_canonical": canonical,
        "payload_json": event.payload_json,
        "digest": event.digest,
        "occurred_at": event.occurred_at,
        "previous_event_digest": event.previous_event_digest,
        "event_digest": event.event_digest,
    })
}

/// Return the coarse failure name the oracle reports.
fn kind_name(kind: kala_studio_journal::ErrorKind) -> &'static str {
    match kind {
        kala_studio_journal::ErrorKind::Type => "type",
        kala_studio_journal::ErrorKind::Value => "value",
        kala_studio_journal::ErrorKind::Integrity => "integrity",
        kala_studio_journal::ErrorKind::NotFound => "not_found",
        kala_studio_journal::ErrorKind::Closed => "closed",
        kala_studio_journal::ErrorKind::Other => "other",
    }
}

fn assert_event_matches(python: &Value, native: &Value, context: &str) {
    for field in [
        "sequence",
        "event_type",
        "aggregate_id",
        "payload_json",
        "digest",
        "occurred_at",
        "previous_event_digest",
        "event_digest",
        "payload_canonical",
    ] {
        assert_eq!(
            python[field], native[field],
            "{context}: the field {field} differs\npython={python}\nnative={native}"
        );
    }
}

#[test]
fn the_corpus_produces_the_same_answers_and_the_same_file() {
    let corpus = build_corpus();
    assert_eq!(corpus.len(), OPERATION_COUNT);

    let directory = TempDir::new().expect("temp dir");
    let native_dir = directory.path().join("native");
    let python_dir = directory.path().join("python");

    let mut oracle = PythonOracle::spawn();
    oracle.ok(json!({"op": "open", "data_dir": python_dir.to_str().unwrap()}));
    let journal = StudioSessionJournal::open(&native_dir).expect("open the native journal");

    for (index, op) in corpus.iter().enumerate() {
        let context = format!("operation {index}: {op:?}");
        let python_response = oracle.request(oracle_request(op));
        let python_ok = python_response["ok"].as_bool().expect("a flag");
        let python = &python_response["result"];
        let python_error = &python_response["error"];
        let native: Value = match op {
            Op::Append {
                event_type,
                payload,
                aggregate_id,
                timestamp,
            } => {
                let event = journal
                    .append_json_with(
                        event_type,
                        payload,
                        &AppendOptions::default()
                            .with_aggregate_id(aggregate_id)
                            .with_occurred_at(timestamp_from_iso(timestamp).expect("a timestamp")),
                    )
                    .expect("native append");
                native_event(&event)
            }
            Op::Page { after, limit } => json!(
                journal
                    .page(*after, *limit)
                    .expect("native page")
                    .iter()
                    .map(native_event)
                    .collect::<Vec<_>>()
            ),
            Op::Get { sequence } => json!(
                journal
                    .get(*sequence)
                    .expect("native get")
                    .as_ref()
                    .map(native_event)
            ),
            Op::Verify => {
                let verification = journal.verify().expect("native verify");
                json!({
                    "ok": verification.ok,
                    "event_count": verification.event_count,
                    "head_digest": verification.head_digest,
                    "first_bad_sequence": verification.first_bad_sequence,
                })
            }
            Op::HeadDigest => json!(journal.head_digest().expect("native head")),
            Op::PutArtifact { data } => json!(journal.put_artifact(data).expect("native put")),
            Op::GetArtifact { digest } => {
                // The artifact may not have been stored yet; both
                // implementations must agree on that.
                match journal.get_artifact(digest) {
                    Ok(data) => {
                        assert!(python_ok, "{context}: the oracle refused the read");
                        assert_eq!(python["base64"], json!(base64_encode(&data)), "{context}");
                    }
                    Err(error) => {
                        assert!(!python_ok, "{context}: the oracle read a missing artifact");
                        assert_eq!(
                            python_error["kind"],
                            json!(kind_name(error.kind())),
                            "{context}: the failure kinds differ"
                        );
                    }
                }
                continue;
            }
            Op::HasArtifact { digest } => {
                json!(journal.has_artifact(digest).expect("native has"))
            }
        };

        assert!(python_ok, "{context}: the oracle refused the request");
        match op {
            Op::Append { .. } => assert_event_matches(python, &native, &context),
            Op::Page { .. } => {
                let python_events = python.as_array().expect("a page");
                let native_events = native.as_array().expect("a page");
                assert_eq!(python_events.len(), native_events.len(), "{context}");
                for (python_event, native_event) in python_events.iter().zip(native_events) {
                    assert_event_matches(python_event, native_event, &context);
                }
            }
            Op::Get { .. } => match (python.is_null(), native.is_null()) {
                (true, true) => {}
                (false, false) => assert_event_matches(python, &native, &context),
                _ => panic!("{context}: the get answers differ\npython={python}\nnative={native}"),
            },
            _ => assert_eq!(*python, native, "{context}: the answers differ"),
        }
    }

    // --- the two files are the same file -----------------------------------
    let python_rows = oracle.ok(json!({"op": "rows"}));
    let python_rows = python_rows.as_array().expect("rows");
    let native_rows = native_rows(&journal);
    assert_eq!(
        python_rows.len(),
        native_rows.len(),
        "the two journals must hold the same number of events"
    );
    for (python, native) in python_rows.iter().zip(&native_rows) {
        assert_eq!(python, native, "the stored rows must be identical");
    }

    // --- and the artifact trees hold the same objects ----------------------
    let python_artifacts = artifact_digests(&python_dir);
    let native_artifacts = artifact_digests(&native_dir);
    assert_eq!(python_artifacts, native_artifacts);

    // --- and the two files have the same schema ---------------------------
    let python_schema = oracle.ok(json!({"op": "schema"}));
    let native_schema = native_schema(journal.database_path());
    assert_eq!(
        python_schema, native_schema,
        "the two implementations must write the same schema"
    );
}

/// Read the stored schema the way the oracle does.
fn native_schema(path: &Path) -> Value {
    use rusqlite::Connection;
    let connection = Connection::open(path).expect("open the journal");
    let mut statement = connection
        .prepare("SELECT type, name, sql FROM sqlite_master ORDER BY type, name")
        .expect("prepare");
    let mut rows = statement.query([]).expect("query");
    let mut out = Vec::new();
    while let Some(row) = rows.next().expect("next") {
        out.push(json!([
            row.get::<_, String>("type").expect("type"),
            row.get::<_, String>("name").expect("name"),
            row.get::<_, Option<String>>("sql").expect("sql"),
        ]));
    }
    Value::Array(out)
}

fn native_rows(journal: &StudioSessionJournal) -> Vec<Value> {
    use rusqlite::Connection;
    let connection = Connection::open(journal.database_path()).expect("open the journal");
    let mut statement = connection
        .prepare(
            "SELECT sequence, event_type, aggregate_id, workflow_id, run_id, payload_json, \
             digest, occurred_at, previous_event_digest, event_digest \
             FROM studio_events ORDER BY sequence ASC",
        )
        .expect("prepare");
    let mut rows = statement.query([]).expect("query");
    let mut out = Vec::new();
    while let Some(row) = rows.next().expect("next") {
        out.push(json!({
            "sequence": row.get::<_, i64>("sequence").expect("sequence"),
            "event_type": row.get::<_, String>("event_type").expect("event_type"),
            "aggregate_id": row.get::<_, String>("aggregate_id").expect("aggregate_id"),
            "workflow_id": row.get::<_, Option<String>>("workflow_id").expect("workflow_id"),
            "run_id": row.get::<_, Option<String>>("run_id").expect("run_id"),
            "payload_json": row.get::<_, String>("payload_json").expect("payload_json"),
            "digest": row.get::<_, String>("digest").expect("digest"),
            "occurred_at": row.get::<_, String>("occurred_at").expect("occurred_at"),
            "previous_event_digest": row
                .get::<_, String>("previous_event_digest")
                .expect("previous_event_digest"),
            "event_digest": row.get::<_, String>("event_digest").expect("event_digest"),
        }));
    }
    out
}

/// Return the digests of every stored artifact, sorted.
fn artifact_digests(data_dir: &Path) -> Vec<String> {
    let objects = data_dir.join("artifacts").join("objects");
    let mut digests = Vec::new();
    if let Ok(buckets) = std::fs::read_dir(&objects) {
        for bucket in buckets.flatten() {
            if let Ok(objects) = std::fs::read_dir(bucket.path()) {
                for object in objects.flatten() {
                    digests.push(object.file_name().to_string_lossy().into_owned());
                }
            }
        }
    }
    digests.sort();
    digests
}
