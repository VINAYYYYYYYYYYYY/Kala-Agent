# kala-studio-journal

The native Rust equivalent of the Python module
`kala_agent.studio.session_journal`: the Studio's own hash-chained session
journal and its content-addressed artifact store.

The file it writes is **byte-compatible** with the file the Python module
writes. The same `studio_events` schema, the same canonical payload JSON, the
same length-prefixed SHA-256 chain digests and the same
`artifacts/objects/<ab>/<digest>` layout. A journal written by either
implementation can be opened, verified and extended by the other, and the two
files compare equal row by row.

## Public API

- `StudioSessionJournal` — `open`, `open_with_config`, `append`,
  `append_with`, `append_json`, `append_json_with`, `page`, `get`, `verify`,
  `head_digest`, `put_artifact`, `get_artifact`, `get_artifact_text`,
  `has_artifact`, `close`, `diagnostics`, and the `data_dir`,
  `database_path`, `artifact_root`, `max_payload_bytes` and `max_page_size`
  accessors.
- `SessionEvent` — one journalled event: `sequence`, `event_type`,
  `aggregate_id`, `payload`, `payload_json`, `digest`, `occurred_at`,
  `previous_event_digest`, `event_digest`, plus `timestamp()`.
- `JournalVerification` — `ok`, `event_count`, `head_digest`,
  `first_bad_sequence`.
- `JournalEventType` — the seven event types ADR-0009 §5 names.
- `JournalConfig`, `AppendOptions`, `JournalDiagnostics`, `StudioJournalError`
  and `ErrorKind`.
- The constants `JOURNAL_FILE_NAME`, `ARTIFACT_DIRECTORY_NAME`,
  `DEFAULT_AGGREGATE_ID`, `DEFAULT_MAX_PAYLOAD_BYTES`,
  `DEFAULT_MAX_ARTIFACT_BYTES`, `DEFAULT_MAX_PAGE_SIZE` and
  `DEFAULT_BUSY_TIMEOUT_MS`.

## What is reused

- The chain digest is the core chain digest: `kala_json::chain_digest` over
  the same eight length-prefixed parts, so a Studio chain digest is
  reproducible with the core function without any Studio-specific knowledge.
- The artifact store is `kala_artifacts::ContentAddressedArtifactStore`.
- The canonical JSON form is `kala_persistence::canonicalize_json_object`,
  the very function the core journal uses.

## The append cost

The chain is walked once, when the journal is opened, and the verified head is
cached from then on. An append re-reads only the newest stored row, so a long
session costs no more per event than a short one.
`StudioSessionJournal::diagnostics()` exposes the counters that make that
observable (`chain_walks`, `fast_path_appends`, `fallback_appends`).

The detection guarantee that follows is narrow and deliberate: an edit of the
head is caught by the next append, an edit in the middle of the chain is
caught by the next `verify()` or the next open. A broken chain is never
extended.

## Tests

From this directory:

```bash
cargo test --locked
cargo test --locked --release
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo fmt --check
```

The differential tests spawn the repository's Python as an oracle
(`/home/v9i/Projects/Kala-Agent/.venv/bin/python` with `PYTHONPATH=src`, or
`$KALA_PYTHON`). Python is a test oracle only; the library contains no
Python, no subprocess and no fallback implementation.
