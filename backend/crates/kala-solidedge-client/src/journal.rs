//! A durable local journal of build submissions.
//!
//! The journal is the client's own memory of what it sent. It records the
//! normalised script itself, the script digest, the canonical request digest,
//! the normalised bridge target, the request id, the build id, and the last
//! known status. It never records the bearer token, a URL with credentials,
//! or artifact bytes.
//!
//! The record format is the one the Python client writes, byte for byte: one
//! JSON object per line, keys sorted, no whitespace, ASCII escaped. Either
//! client can therefore read the other's records.
//!
//! The request digest covers everything that changes the requested build —
//! calibration, timeout, density, summary, and the ordered procedure — so a
//! retry reuses a request id only when the whole semantic request is
//! identical. The bridge target binds that to one bridge URL and calibration,
//! so a re-probed host does not inherit an old job. A legacy entry written
//! before those fields existed has no exact identity; it is read for history
//! but never reused for a new submission.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use kala_json::{CanonicalObject, CanonicalValue, JsonText};

/// The journal file name.
pub const JOURNAL_FILENAME: &str = "journal.jsonl";
/// The largest number of records kept.
pub const MAX_ENTRIES: usize = 200;
/// The largest accepted digest length.
pub const MAX_DIGEST_CHARS: usize = 64;
/// The largest accepted identifier length.
pub const MAX_ID_CHARS: usize = 64;
/// The largest accepted bridge target length.
pub const MAX_TARGET_CHARS: usize = 512;
/// The largest accepted normalised script length.
pub const MAX_SCRIPT_CHARS: usize = 65_536;

const MAX_STATUS_CHARS: usize = 32;
const MAX_SUMMARY_CHARS: usize = 200;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// One recorded submission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    /// The digest of the normalised script.
    pub script_digest: String,
    /// The transport correlation id.
    pub request_id: String,
    /// The bridge's build id, once one is known.
    pub build_id: Option<String>,
    /// The last known state.
    pub status: String,
    /// A short summary of the request.
    pub summary: String,
    /// The canonical request digest, when known.
    pub request_digest: Option<String>,
    /// The normalised bridge target, when known.
    pub bridge_target: Option<String>,
    /// The normalised script, when recorded.
    pub script: Option<String>,
}

impl JournalEntry {
    /// Build one entry with only the required fields.
    pub fn new(
        script_digest: impl Into<String>,
        request_id: impl Into<String>,
        build_id: Option<String>,
        status: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            script_digest: script_digest.into(),
            request_id: request_id.into(),
            build_id,
            status: status.into(),
            summary: summary.into(),
            request_digest: None,
            bridge_target: None,
            script: None,
        }
    }

    /// The canonical JSON value of this entry.
    pub fn to_canonical_value(&self) -> CanonicalValue {
        let mut object = CanonicalObject::new();
        object.insert(
            JsonText::new("script_digest"),
            CanonicalValue::Text(JsonText::new(&self.script_digest)),
        );
        object.insert(
            JsonText::new("request_id"),
            CanonicalValue::Text(JsonText::new(&self.request_id)),
        );
        object.insert(
            JsonText::new("build_id"),
            optional_text(self.build_id.as_deref()),
        );
        object.insert(
            JsonText::new("status"),
            CanonicalValue::Text(JsonText::new(&self.status)),
        );
        object.insert(
            JsonText::new("summary"),
            CanonicalValue::Text(JsonText::new(&self.summary)),
        );
        object.insert(
            JsonText::new("request_digest"),
            optional_text(self.request_digest.as_deref()),
        );
        object.insert(
            JsonText::new("bridge_target"),
            optional_text(self.bridge_target.as_deref()),
        );
        object.insert(
            JsonText::new("script"),
            optional_text(self.script.as_deref()),
        );
        CanonicalValue::Object(object)
    }

    /// The one line this entry occupies in the journal file.
    pub fn to_json_line(&self) -> String {
        let encoded = kala_json::canonical_dumps(
            &self.to_canonical_value(),
            &kala_contracts::model_json_limits(),
        )
        .expect("a journal record is always canonicalizable");
        format!("{encoded}\n")
    }

    /// Read one entry from a parsed JSON value.
    ///
    /// A malformed record is skipped, exactly like the Python journal: a
    /// truncated line must not hide the records around it.
    pub fn from_canonical_value(value: &CanonicalValue) -> Option<Self> {
        let CanonicalValue::Object(object) = value else {
            return None;
        };
        let script_digest = bounded_text(object, "script_digest", MAX_DIGEST_CHARS)?;
        let request_id = bounded_text(object, "request_id", MAX_ID_CHARS)?;
        let build_id = optional_bounded_text(object, "build_id", MAX_ID_CHARS)?;
        let status = bounded_text(object, "status", MAX_STATUS_CHARS)?;
        let summary = bounded_text(object, "summary", MAX_SUMMARY_CHARS)?;
        let request_digest = optional_bounded_text(object, "request_digest", MAX_DIGEST_CHARS)?;
        let bridge_target = optional_bounded_text(object, "bridge_target", MAX_TARGET_CHARS)?;
        let script = optional_bounded_text(object, "script", MAX_SCRIPT_CHARS)?;
        Some(Self {
            script_digest,
            request_id,
            build_id,
            status,
            summary,
            request_digest,
            bridge_target,
            script,
        })
    }
}

/// An append-only, bounded JSONL journal with atomic rewrites.
#[derive(Clone, Debug)]
pub struct Journal {
    path: PathBuf,
}

impl Journal {
    /// Open the journal at a path. The file is not created yet.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The journal file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the recorded entries, newest last. Malformed lines are skipped.
    pub fn read(&self) -> Vec<JournalEntry> {
        let Ok(text) = fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(value) =
                kala_json::parse_strict(line.as_bytes(), &kala_contracts::model_json_limits())
            else {
                continue;
            };
            if let Some(entry) = JournalEntry::from_canonical_value(&value) {
                entries.push(entry);
            }
        }
        entries
    }

    /// Return the most recent entry for a semantic script digest.
    pub fn find_by_digest(&self, script_digest: &str) -> Option<JournalEntry> {
        self.read()
            .into_iter()
            .rev()
            .find(|entry| entry.script_digest == script_digest)
    }

    /// Return the newest entry with the exact request identity, if any.
    pub fn find_exact(
        &self,
        script_digest: &str,
        request_digest: &str,
        bridge_target: &str,
    ) -> Option<JournalEntry> {
        self.read().into_iter().rev().find(|entry| {
            entry.script_digest == script_digest
                && entry.request_digest.as_deref() == Some(request_digest)
                && entry.bridge_target.as_deref() == Some(bridge_target)
        })
    }

    /// Return the entry for one request id.
    pub fn find_by_request(&self, request_id: &str) -> Option<JournalEntry> {
        self.read()
            .into_iter()
            .rev()
            .find(|entry| entry.request_id == request_id)
    }

    /// Append one entry and compact to the bounded size.
    pub fn append(&self, entry: &JournalEntry) -> std::io::Result<()> {
        let mut entries = self.read();
        entries.push(entry.clone());
        let start = entries.len().saturating_sub(MAX_ENTRIES);
        self.rewrite(&entries[start..])
    }

    /// Replace the newest entry for `request_id` with updated fields.
    ///
    /// A request id that is not in the journal is left alone: the journal
    /// never invents a record.
    pub fn update(
        &self,
        request_id: &str,
        build_id: Option<&str>,
        status: &str,
    ) -> std::io::Result<()> {
        let mut entries = self.read();
        let Some(index) = entries
            .iter()
            .rposition(|entry| entry.request_id == request_id)
        else {
            return Ok(());
        };
        let previous = entries[index].clone();
        entries[index] = JournalEntry {
            script_digest: previous.script_digest,
            request_id: request_id.to_owned(),
            build_id: build_id.map(str::to_owned).or(previous.build_id),
            status: status.to_owned(),
            summary: previous.summary,
            request_digest: previous.request_digest,
            bridge_target: previous.bridge_target,
            script: previous.script,
        };
        let start = entries.len().saturating_sub(MAX_ENTRIES);
        self.rewrite(&entries[start..])
    }

    fn rewrite(&self, entries: &[JournalEntry]) -> std::io::Result<()> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let payload: String = entries
            .iter()
            .map(JournalEntry::to_json_line)
            .collect::<Vec<_>>()
            .join("");
        let temporary = temporary_path(parent, &self.path);
        let outcome = (|| -> std::io::Result<()> {
            let mut handle = fs::File::create(&temporary)?;
            handle.write_all(payload.as_bytes())?;
            handle.flush()?;
            handle.sync_all()?;
            Ok(())
        })();
        if let Err(error) = outcome {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        match fs::rename(&temporary, &self.path) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                Err(error)
            }
        }
    }
}

fn temporary_path(parent: &Path, path: &Path) -> PathBuf {
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "journal".to_owned());
    parent.join(format!(".{name}.{}.{counter}.tmp", std::process::id()))
}

fn optional_text(value: Option<&str>) -> CanonicalValue {
    match value {
        Some(text) => CanonicalValue::Text(JsonText::new(text)),
        None => CanonicalValue::Null,
    }
}

fn bounded_text(object: &CanonicalObject, name: &str, limit: usize) -> Option<String> {
    let value = object.get(&JsonText::new(name))?;
    let CanonicalValue::Text(text) = value else {
        return None;
    };
    let text = text.to_utf8().ok()?;
    if text.chars().count() > limit {
        return None;
    }
    Some(text)
}

fn optional_bounded_text(
    object: &CanonicalObject,
    name: &str,
    limit: usize,
) -> Option<Option<String>> {
    match object.get(&JsonText::new(name)) {
        Some(CanonicalValue::Null) | None => Some(None),
        Some(_) => bounded_text(object, name, limit).map(Some),
    }
}

#[cfg(test)]
mod tests {
    use super::{Journal, JournalEntry, MAX_ENTRIES};

    fn entry(request_id: &str, status: &str) -> JournalEntry {
        JournalEntry::new("a".repeat(64), request_id, None, status, "summary")
    }

    #[test]
    fn a_missing_journal_reads_as_empty() {
        let journal = Journal::new("/nonexistent/dir/journal.jsonl");
        assert!(journal.read().is_empty());
    }

    #[test]
    fn records_round_trip_through_the_file() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("journal.jsonl");
        let journal = Journal::new(&path);
        journal.append(&entry("req-1", "pending")).expect("append");
        journal
            .update("req-1", Some("build-1"), "succeeded")
            .expect("update");
        let entries = journal.read();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].build_id.as_deref(), Some("build-1"));
        assert_eq!(entries[0].status, "succeeded");
    }

    #[test]
    fn the_record_is_one_compact_sorted_line() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("journal.jsonl");
        let journal = Journal::new(&path);
        journal.append(&entry("req-1", "pending")).expect("append");
        let text = std::fs::read_to_string(&path).expect("read");
        assert_eq!(text.lines().count(), 1);
        assert!(text.starts_with(
            "{\"bridge_target\":null,\"build_id\":null,\"request_digest\":null,\"request_id\""
        ));
        assert!(text.ends_with("}\n"));
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("journal.jsonl");
        std::fs::write(&path, "not json\n{\"script_digest\":1}\n").expect("write");
        let journal = Journal::new(&path);
        assert!(journal.read().is_empty());
    }

    #[test]
    fn the_journal_is_bounded() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("journal.jsonl");
        let journal = Journal::new(&path);
        for index in 0..(MAX_ENTRIES + 10) {
            journal
                .append(&entry(&format!("req-{index}"), "pending"))
                .expect("append");
        }
        let entries = journal.read();
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert_eq!(entries[0].request_id, "req-10");
    }

    #[test]
    fn an_update_for_an_unknown_request_does_nothing() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("journal.jsonl");
        let journal = Journal::new(&path);
        journal.append(&entry("req-1", "pending")).expect("append");
        journal
            .update("other", Some("build-1"), "succeeded")
            .expect("update");
        let entries = journal.read();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "pending");
    }
}
