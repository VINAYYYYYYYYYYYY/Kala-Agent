//! The shape of the append cost.
//!
//! The journal caches the verified head, so an append reads one row instead of
//! walking the chain. The counters make that observable: a steady stream of
//! appends must not walk the chain again, whatever the length of the history.

use std::time::Instant;

use kala_studio_journal::StudioSessionJournal;
use tempfile::TempDir;

fn open(directory: &std::path::Path) -> StudioSessionJournal {
    StudioSessionJournal::open(directory).expect("open the journal")
}

#[test]
fn a_long_history_costs_no_more_per_append_than_a_short_one() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());

    // A long history.
    for index in 0..5_000 {
        journal
            .append_json("chat.turn", &format!(r#"{{"index":{index}}}"#))
            .expect("append");
    }

    let before = journal.diagnostics();
    let started = Instant::now();
    for index in 0..1_000 {
        journal
            .append_json("chat.turn", &format!(r#"{{"index":{index}}}"#))
            .expect("append");
    }
    let elapsed = started.elapsed();
    let after = journal.diagnostics();

    // The shape: no walk per append.
    assert_eq!(
        after.chain_walks, before.chain_walks,
        "an append must not walk the chain"
    );
    assert_eq!(after.fast_path_appends, before.fast_path_appends + 1_000);
    assert_eq!(after.fallback_appends, before.fallback_appends);

    // A sanity bound, not the shape itself: the shape is proved by the
    // counters above. An O(n) append would need minutes for this loop.
    assert!(
        elapsed.as_secs() < 30,
        "1,000 appends on a 6,000-event journal took {elapsed:?}"
    );
    println!(
        "1,000 appends on a 6,000-event journal took {elapsed:?} \
         ({:.1} us per append)",
        elapsed.as_micros() as f64 / 1_000.0
    );

    let verification = journal.verify().expect("verify");
    assert!(verification.ok);
    assert_eq!(verification.event_count, 6_000);
}

#[test]
fn the_first_append_after_an_open_pays_for_one_walk() {
    let directory = TempDir::new().expect("temp dir");
    let journal = open(directory.path());

    let after_open = journal.diagnostics();
    assert_eq!(
        after_open.chain_walks, 1,
        "opening the journal walks the chain exactly once"
    );

    journal
        .append_json("chat.turn", r#"{"index":0}"#)
        .expect("append");
    let after_append = journal.diagnostics();
    assert_eq!(
        after_append.chain_walks, 1,
        "the first append uses the cached head"
    );
    assert_eq!(after_append.fast_path_appends, 1);
    assert_eq!(after_append.fallback_appends, 0);
}

#[test]
fn a_second_instance_on_the_same_file_pays_for_one_walk() {
    let directory = TempDir::new().expect("temp dir");
    let first = open(directory.path());
    first
        .append_json("chat.turn", r#"{"text":"one"}"#)
        .expect("append");

    let second = open(directory.path());
    second
        .append_json("chat.turn", r#"{"text":"two"}"#)
        .expect("append");

    let before = first.diagnostics();
    first
        .append_json("chat.turn", r#"{"text":"three"}"#)
        .expect("append");
    let after = first.diagnostics();

    assert_eq!(
        after.chain_walks,
        before.chain_walks + 1,
        "an append that finds a foreign head walks the chain once"
    );
    assert_eq!(after.fallback_appends, before.fallback_appends + 1);
    assert_eq!(after.fast_path_appends, before.fast_path_appends);
}
