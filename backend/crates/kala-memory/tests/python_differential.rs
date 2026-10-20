//! Live Python differential tests for the native memory manager.
//!
//! Every scenario in this file is executed twice: once by the repository's
//! Python `kala_agent.kernel.memory` reference implementation (through the
//! test-only oracle) and once by the native crate. The two runs are compared
//! as canonical JSON bytes, so a difference in a status, a record projection,
//! an ordering, an expiry boundary or an error message fails the test.

mod support;

use kala_json::CanonicalValue;
use support::{array, assert_matches_oracle, document_totals, int, object, record_payload, text};

const T0: &str = "2026-01-02T03:04:05Z";
const T1: &str = "2026-01-02T03:05:05Z";
const T2: &str = "2026-01-02T03:06:05Z";
/// The same instant as `T0`, written with a different UTC offset.
const T0_OFFSET: &str = "2026-01-02T04:04:05+01:00";

fn record(
    memory_id: &str,
    content: &str,
    source: &str,
    created_at: &str,
    expires_at: Option<&str>,
) -> CanonicalValue {
    record_payload(memory_id, content, source, None, created_at, expires_at)
}

fn op(name: &str, mut fields: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    fields.push(("op", text(name)));
    object(fields)
}

fn op_add(record: CanonicalValue, durable: bool, promote: bool, now: &str) -> CanonicalValue {
    op(
        "add",
        vec![
            ("record", record),
            ("durable", CanonicalValue::Bool(durable)),
            ("promote", CanonicalValue::Bool(promote)),
            ("now", text(now)),
        ],
    )
}

fn op_try_add(record: CanonicalValue, now: &str) -> CanonicalValue {
    op("try_add", vec![("record", record), ("now", text(now))])
}

fn op_promote_record(
    record: CanonicalValue,
    provenance_id: Option<&str>,
    now: &str,
) -> CanonicalValue {
    op(
        "promote",
        vec![
            (
                "target",
                object(vec![("kind", text("record")), ("record", record)]),
            ),
            (
                "provenance_id",
                provenance_id.map_or(CanonicalValue::Null, text),
            ),
            ("now", text(now)),
        ],
    )
}

fn op_promote_id(memory_id: &str, provenance_id: Option<&str>, now: &str) -> CanonicalValue {
    op(
        "promote",
        vec![
            (
                "target",
                object(vec![("kind", text("id")), ("id", text(memory_id))]),
            ),
            (
                "provenance_id",
                provenance_id.map_or(CanonicalValue::Null, text),
            ),
            ("now", text(now)),
        ],
    )
}

fn op_get(memory_id: &str, now: &str, include_expired: bool) -> CanonicalValue {
    op(
        "get",
        vec![
            ("memory_id", text(memory_id)),
            ("now", text(now)),
            ("include_expired", CanonicalValue::Bool(include_expired)),
        ],
    )
}

fn op_list(
    source: Option<&str>,
    now: &str,
    include_working: bool,
    durable_only: bool,
    limit: i64,
) -> CanonicalValue {
    op(
        "list",
        vec![
            ("source", source.map_or(CanonicalValue::Null, text)),
            ("now", text(now)),
            ("include_working", CanonicalValue::Bool(include_working)),
            ("durable_only", CanonicalValue::Bool(durable_only)),
            ("limit", int(limit)),
        ],
    )
}

fn op_list_active(
    source: Option<&str>,
    now: &str,
    include_working: bool,
    durable_only: bool,
    limit: i64,
) -> CanonicalValue {
    op(
        "list_active",
        vec![
            ("source", source.map_or(CanonicalValue::Null, text)),
            ("now", text(now)),
            ("include_working", CanonicalValue::Bool(include_working)),
            ("durable_only", CanonicalValue::Bool(durable_only)),
            ("limit", int(limit)),
        ],
    )
}

fn op_list_durable(source: Option<&str>, now: &str, limit: i64) -> CanonicalValue {
    op(
        "list_durable",
        vec![
            ("source", source.map_or(CanonicalValue::Null, text)),
            ("now", text(now)),
            ("limit", int(limit)),
        ],
    )
}

fn op_list_working(source: Option<&str>, now: &str, limit: i64) -> CanonicalValue {
    op(
        "list_working",
        vec![
            ("source", source.map_or(CanonicalValue::Null, text)),
            ("now", text(now)),
            ("limit", int(limit)),
        ],
    )
}

fn op_detect_conflicts(record: CanonicalValue, now: &str) -> CanonicalValue {
    op(
        "detect_conflicts",
        vec![("record", record), ("now", text(now))],
    )
}

fn op_discard_working(memory_id: &str) -> CanonicalValue {
    op("discard_working", vec![("memory_id", text(memory_id))])
}

fn op_delete(memory_id: &str) -> CanonicalValue {
    op("delete", vec![("memory_id", text(memory_id))])
}

fn op_repository_put(record: CanonicalValue) -> CanonicalValue {
    op("repository_put", vec![("record", record)])
}

fn scenario(ops: Vec<CanonicalValue>) -> CanonicalValue {
    object(vec![("ops", array(ops))])
}

fn scenario_with(
    max_working_records: i64,
    require_provenance: bool,
    ops: Vec<CanonicalValue>,
) -> CanonicalValue {
    object(vec![
        ("max_working_records", int(max_working_records)),
        (
            "require_provenance_on_promotion",
            CanonicalValue::Bool(require_provenance),
        ),
        ("ops", array(ops)),
    ])
}

fn batch(scenarios: Vec<CanonicalValue>) -> CanonicalValue {
    object(vec![("scenarios", array(scenarios))])
}

#[test]
fn recording_querying_and_promoting_match_python() {
    let document = batch(vec![scenario(vec![
        op_try_add(record("m-1", "alpha", "source-a", T0, None), T0),
        op_try_add(record("m-1", "alpha", "source-a", T0, None), T0),
        op_try_add(record("m-2", "beta", "source-a", T0, None), T0),
        op_try_add(record("m-2", "alpha", "source-a", T0, None), T0),
        op_try_add(record("m-3", "alpha", "source-b", T0, None), T0),
        op_promote_id("m-1", None, T0),
        op_list_durable(None, T0, 1000),
        op_try_add(record("m-4", "alpha", "source-a", T0, None), T0),
        op_try_add(record("m-4", "gamma", "source-a", T0, None), T0),
        op_get("m-1", T0, false),
        op_delete("m-1"),
        op_get("m-1", T0, false),
        op_list(None, T0, true, false, 1000),
    ])]);
    assert_matches_oracle(&document);
}

#[test]
fn expiry_boundaries_match_python() {
    let document = batch(vec![scenario(vec![
        op_add(
            record("m-1", "alpha", "source-a", T0, Some(T1)),
            false,
            false,
            T0,
        ),
        op_get("m-1", T0, false),
        op_get("m-1", T1, false),
        op_get("m-1", T1, true),
        op_get("m-1", T2, false),
        op_list(None, T1, true, false, 1000),
        op_list_working(None, T1, 1000),
        op_try_add(record("m-2", "beta", "source-a", T1, None), T1),
        op_try_add(record("m-2", "beta", "source-a", T1, None), T2),
    ])]);
    assert_matches_oracle(&document);
}

#[test]
fn listing_sources_limits_and_ordering_match_python() {
    let document = batch(vec![scenario(vec![
        op_add(
            record("m-2", "second", "source-2", T0, None),
            false,
            false,
            T0,
        ),
        op_add(
            record("m-1", "first", "source-1", T0, None),
            false,
            false,
            T0,
        ),
        op_add(
            record("m-3", "third", "source-1", T1, None),
            false,
            false,
            T0,
        ),
        op_list(None, T0, true, false, 1000),
        op_list(Some("source-1"), T0, true, false, 1000),
        op_list(None, T0, true, false, 1),
        op_list(None, T0, true, false, 2),
        // The Python `list` accepts `include_working` and never reads it, so
        // a false value still lists the working records.
        op_list(None, T0, false, false, 1000),
        op_list_active(None, T0, true, false, 1000),
        op_list_active(None, T0, false, true, 1000),
        op_list_active(None, T0, false, false, 1000),
        op_list_working(None, T0, 1000),
        op_list_working(Some("source-1"), T0, 1000),
        op_list_working(None, T0, 0),
        op_list_working(None, T0, -1),
        op_list_working(None, T0, -5),
    ])]);
    assert_matches_oracle(&document);
}

#[test]
fn working_memory_eviction_and_capacity_match_python() {
    let document = batch(vec![
        scenario_with(
            2,
            false,
            vec![
                op_add(
                    record("m-1", "first", "source-1", T0, Some(T1)),
                    false,
                    false,
                    T0,
                ),
                op_add(
                    record("m-2", "second", "source-2", T0, None),
                    false,
                    false,
                    T0,
                ),
                op_try_add(record("m-3", "third", "source-3", T0, None), T1),
                op_list(None, T1, true, false, 1000),
            ],
        ),
        scenario_with(
            1,
            false,
            vec![
                op_add(
                    record("m-1", "first", "source-1", T0, None),
                    false,
                    false,
                    T0,
                ),
                op_try_add(record("m-2", "second", "source-2", T0, None), T0),
                op_list(None, T0, true, false, 1000),
            ],
        ),
        scenario_with(0, false, vec![op_list(None, T0, true, false, 1000)]),
    ]);
    assert_matches_oracle(&document);
}

#[test]
fn promotion_rules_match_python() {
    let document = batch(vec![
        scenario(vec![
            op_add(
                record("m-1", "alpha", "source-a", T0, None),
                false,
                false,
                T0,
            ),
            op_promote_id("m-1", None, T0),
            op_promote_id("m-1", Some("p-42"), T0),
            op_get("m-1", T0, false),
            op_promote_id("absent", None, T0),
            op_promote_record(
                record("m-2", "beta", "source-b", T0, None),
                Some("../bad"),
                T0,
            ),
            // The promotion of a record that is already in working memory
            // uses the stored working record, not the offered one.
            op_add(
                record("m-3", "gamma", "source-c", T0, None),
                false,
                false,
                T0,
            ),
            op_promote_record(record("m-3", "delta", "source-d", T0, None), None, T0),
            op_get("m-3", T0, false),
        ]),
        scenario_with(
            4,
            true,
            vec![
                op_promote_record(record("m-1", "alpha", "source-a", T0, None), None, T0),
                op_add(
                    record("m-2", "alpha", "source-a", T0, None),
                    true,
                    false,
                    T0,
                ),
            ],
        ),
    ]);
    assert_matches_oracle(&document);
}

#[test]
fn conflict_detection_and_discarding_match_python() {
    let document = batch(vec![scenario(vec![
        op_add(
            record("m-1", "first", "source-1", T0, None),
            false,
            false,
            T0,
        ),
        op_repository_put(record("m-2", "second", "source-1", T1, None)),
        op_detect_conflicts(record("m-3", "third", "source-1", T0, None), T0),
        op_detect_conflicts(record("m-4", "other", "source-2", T0, None), T0),
        op_discard_working("m-1"),
        op_discard_working("m-1"),
        op_discard_working("absent"),
        op_list(None, T0, true, false, 1000),
    ])]);
    assert_matches_oracle(&document);
}

#[test]
fn invalid_arguments_match_python() {
    let document = batch(vec![scenario(vec![
        op_get("", T0, false),
        op_get("   ", T0, false),
        op_get("../escape", T0, false),
        op_list(Some("../escape"), T0, true, false, 1000),
        op_list(None, T0, true, false, 0),
        op_list(None, T0, true, false, -1),
        op_delete("../escape"),
        op_discard_working("../escape"),
    ])]);
    assert_matches_oracle(&document);
}

#[test]
fn offsets_and_instant_equality_match_python() {
    let document = batch(vec![scenario(vec![
        op_try_add(record("m-1", "alpha", "source-a", T0, None), T0_OFFSET),
        op_try_add(record("m-2", "beta", "source-a", T0, None), T0_OFFSET),
        op_get("m-1", T0_OFFSET, false),
        op_list(None, T0, true, false, 1000),
    ])]);
    assert_matches_oracle(&document);
}

/// A deterministic pseudo-random generator, so the corpus is reproducible
/// without a dependency.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

const IDS: [&str; 6] = ["m-0", "m-1", "m-2", "m-3", "m-4", "m-5"];
const SOURCES: [&str; 3] = ["source-1", "source-2", "source-3"];
const CONTENTS: [&str; 4] = ["alpha", "beta", "gamma", "delta"];
const NOWS: [&str; 4] = [T0, T1, T2, T0_OFFSET];

fn random_record(rng: &mut Rng) -> CanonicalValue {
    let memory_id = *rng.pick(&IDS);
    let content = *rng.pick(&CONTENTS);
    let source = *rng.pick(&SOURCES);
    let created_at = *rng.pick(&NOWS);
    let expires_at = if rng.below(3) == 0 { Some(T2) } else { None };
    record(memory_id, content, source, created_at, expires_at)
}

fn random_scenario(rng: &mut Rng) -> CanonicalValue {
    let operation_count = 4 + rng.below(12) as usize;
    let mut operations = Vec::with_capacity(operation_count);
    for _ in 0..operation_count {
        let now = *rng.pick(&NOWS);
        let operation = match rng.below(11) {
            0 => op_add(
                random_record(rng),
                rng.below(2) == 0,
                rng.below(3) == 0,
                now,
            ),
            1 => op_try_add(random_record(rng), now),
            2 => {
                if rng.below(2) == 0 {
                    op_promote_record(random_record(rng), None, now)
                } else {
                    op_promote_id(rng.pick(&IDS), None, now)
                }
            }
            3 => op_get(rng.pick(&IDS), now, rng.below(2) == 0),
            4 => op_list(
                if rng.below(3) == 0 {
                    None
                } else {
                    Some(rng.pick(&SOURCES))
                },
                now,
                rng.below(2) == 0,
                rng.below(3) == 0,
                1 + rng.below(4) as i64,
            ),
            5 => op_list_active(
                if rng.below(3) == 0 {
                    None
                } else {
                    Some(rng.pick(&SOURCES))
                },
                now,
                rng.below(2) == 0,
                rng.below(3) == 0,
                1 + rng.below(4) as i64,
            ),
            6 => op_list_durable(
                if rng.below(3) == 0 {
                    None
                } else {
                    Some(rng.pick(&SOURCES))
                },
                now,
                1 + rng.below(4) as i64,
            ),
            7 => op_list_working(
                if rng.below(3) == 0 {
                    None
                } else {
                    Some(rng.pick(&SOURCES))
                },
                now,
                rng.below(4) as i64 - 1,
            ),
            8 => op_detect_conflicts(random_record(rng), now),
            9 => op_discard_working(rng.pick(&IDS)),
            _ => op_delete(rng.pick(&IDS)),
        };
        operations.push(operation);
    }
    let max_working_records = 1 + rng.below(6) as i64;
    scenario_with(max_working_records, rng.below(4) == 0, operations)
}

#[test]
fn randomized_corpus_matches_python() {
    let mut rng = Rng::new(0x5eed_1234_abcd_0001);
    let scenarios: Vec<CanonicalValue> = (0..64).map(|_| random_scenario(&mut rng)).collect();
    let document = batch(scenarios);
    let (scenario_count, operation_count) = document_totals(&document);
    assert_eq!(scenario_count, 64);
    assert!(
        operation_count >= 500,
        "corpus too small: {operation_count}"
    );
    assert_matches_oracle(&document);
}

#[test]
fn durable_scan_limit_matches_python() {
    // The Python manager reads durable records through the repository's
    // default `limit=1_000`, so the thousand-and-first record of a source is
    // outside the window it compares against.
    let mut operations = Vec::new();
    for index in 0..1_001 {
        operations.push(op_repository_put(record(
            &format!("d-{index:04}"),
            &format!("content-{index:04}"),
            "source-1",
            T0,
            None,
        )));
    }
    operations.push(op_try_add(
        record("probe", "content-1000", "source-1", T0, None),
        T0,
    ));
    operations.push(op_try_add(
        record("probe-2", "content-0000", "source-1", T0, None),
        T0,
    ));
    operations.push(op_list_durable(Some("source-1"), T0, 2));

    // The discriminating case: a thousand durable records of the source share
    // one content and the thousand-and-first holds another. The candidate
    // matches the visible window, so it is a duplicate; the record outside
    // the window never takes part in the comparison.
    let mut windowed = Vec::new();
    for index in 0..1_000 {
        windowed.push(op_repository_put(record(
            &format!("w-{index:04}"),
            "shared",
            "source-2",
            T0,
            None,
        )));
    }
    windowed.push(op_repository_put(record(
        "w-1000", "other", "source-2", T0, None,
    )));
    windowed.push(op_try_add(
        record("probe-3", "shared", "source-2", T0, None),
        T0,
    ));
    windowed.push(op_try_add(
        record("probe-4", "other", "source-2", T0, None),
        T0,
    ));

    assert_matches_oracle(&batch(vec![scenario(operations), scenario(windowed)]));
}
