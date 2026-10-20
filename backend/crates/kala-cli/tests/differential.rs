//! Live differential: the native `kala-agent` against the repository Python CLI.
//!
//! One script of commands is driven through both implementations, each side in
//! its own temporary data directory and HOME. Every step compares the exit
//! code, stdout and stderr byte for byte after normalizing only the declared
//! opaque values: the two data-directory paths, native identity digests, the
//! wall-clock timestamps, and the digest-derived `commit-...` request ids.
//! At the end the two data directories are compared structurally: the file
//! tree, every table's row count, the journal's event sequence, the revision
//! ids and the approval statuses.
//!
//! Values that a later command needs (`request_id`, `state_digest`,
//! `action_digest`) are read from each side's own output, because the declared
//! native package identity makes them different by design.
//!
//! Python is a test oracle only. The binary never invokes it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use regex::Regex;
use tempfile::TempDir;

const FIXED_TOKEN: &str = "cli-differential-launch-token-0123456789abcdef";
const COLUMNS: [&str; 11] = [
    "0", "-5", "abc", "23", "40", "60", "78", "80", "100", "120", "200",
];
const COMMANDS: [&str; 12] = [
    "init",
    "domains",
    "token",
    "serve",
    "evaluate",
    "run-mechanical",
    "workflow-status",
    "approve",
    "reject",
    "prepare-revision",
    "commit-revision",
    "events",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("crate lives under backend/crates/kala-cli")
        .to_path_buf()
}

fn python_binary() -> PathBuf {
    std::env::var_os("KALA_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join(".venv").join("bin").join("python"))
}

fn rust_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_kala-agent"))
}

fn python_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        Command::new(python_binary())
            .args(["-I", "-c", "import kala_agent.cli.main"])
            .current_dir(repo_root())
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    })
}

#[derive(Clone, Debug)]
struct Captured {
    code: i32,
    stdout: String,
    stderr: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TreeEntry {
    path: String,
    kind: &'static str,
}

struct Side {
    data: PathBuf,
    home: PathBuf,
    path_override: PathBuf,
}

impl Side {
    fn new(root: &Path, name: &str) -> Self {
        let home = root.join(format!("{name}-home"));
        fs::create_dir_all(&home).expect("home");
        Self {
            data: root.join(format!("{name}-data")),
            home,
            path_override: root.join(format!("{name}-override")),
        }
    }
}

struct Harness {
    _root: TempDir,
    python: Side,
    rust: Side,
    failures: Vec<String>,
    steps: usize,
}

impl Harness {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        Self {
            python: Side::new(root.path(), "py"),
            rust: Side::new(root.path(), "rs"),
            _root: root,
            failures: Vec::new(),
            steps: 0,
        }
    }

    fn side(&self, python: bool) -> &Side {
        if python { &self.python } else { &self.rust }
    }

    fn run_on(
        &self,
        python: bool,
        argv: &[String],
        columns: Option<&str>,
        env: &[(&str, &str)],
    ) -> Captured {
        self.run_with(python, argv, columns, env, true)
    }

    fn run_with(
        &self,
        python: bool,
        argv: &[String],
        columns: Option<&str>,
        env: &[(&str, &str)],
        include_data_dir: bool,
    ) -> Captured {
        let side = self.side(python);
        let mut command = if python {
            let mut command = Command::new(python_binary());
            command.args(["-I", "-m", "kala_agent.cli"]);
            command
        } else {
            Command::new(rust_binary())
        };
        command
            .args(argv)
            .current_dir(repo_root())
            .env_clear()
            .env("HOME", &side.home)
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("LC_ALL", "C.UTF-8")
            .env("LANG", "C.UTF-8")
            .env("TZ", "UTC");
        if include_data_dir
            && !argv.iter().any(|argument| argument == "--data-dir")
            && !argv.iter().any(|argument| argument.starts_with("--data-d"))
        {
            command.env("KALA_AGENT_DATA_DIR", &side.data);
        }
        command
            .env("KALA_AGENT_MODEL_PROVIDER", "local")
            .env("KALA_AGENT_MODEL_ID", "local");
        if let Some(columns) = columns {
            command.env("COLUMNS", columns);
        }
        for (name, value) in env {
            command.env(name, value);
        }
        let output = command.output().expect("spawn the CLI");
        Captured {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    fn step(
        &mut self,
        label: &str,
        argv: &[String],
        columns: Option<&str>,
        env: &[(&str, &str)],
    ) -> (Captured, Captured) {
        self.step_with(label, argv, argv, columns, env)
    }

    fn step_with(
        &mut self,
        label: &str,
        python_argv: &[String],
        rust_argv: &[String],
        columns: Option<&str>,
        env: &[(&str, &str)],
    ) -> (Captured, Captured) {
        let python = self.run_on(true, python_argv, columns, env);
        let rust = self.run_on(false, rust_argv, columns, env);
        self.steps += 1;
        let python_normalized = {
            let side = &self.python;
            normalize(&python, side)
        };
        let rust_normalized = {
            let side = &self.rust;
            normalize(&rust, side)
        };
        if python.code != rust.code {
            self.failures.push(format!(
                "{label}: exit code differs (python {}, rust {})\npython argv: {python_argv:?}\nrust argv: {rust_argv:?}\n",
                python.code, rust.code
            ));
        }
        if python_normalized != rust_normalized {
            self.failures.push(format!(
                "{label}: stdout/stderr differ\npython argv: {python_argv:?}\nrust argv: {rust_argv:?}\n--- python ---\n{python_normalized}\n--- rust ---\n{rust_normalized}\n"
            ));
        }
        (python, rust)
    }

    fn step_default_home(&mut self, label: &str, argv: &[String]) -> (Captured, Captured) {
        let python = self.run_with(true, argv, None, &[], false);
        let rust = self.run_with(false, argv, None, &[], false);
        self.steps += 1;
        let python_normalized = {
            let side = &self.python;
            normalize(&python, side)
        };
        let rust_normalized = {
            let side = &self.rust;
            normalize(&rust, side)
        };
        if python.code != rust.code || python_normalized != rust_normalized {
            self.failures.push(format!(
                "{label}: HOME default data directory differs\npython: {python_normalized}\nrust: {rust_normalized}\n"
            ));
        }
        (python, rust)
    }

    fn run_pair(&mut self, label: &str, argv: &[&str]) {
        let argv: Vec<String> = argv.iter().map(|value| (*value).to_owned()).collect();
        let _ = self.step(label, &argv, None, &[]);
    }

    fn field(&mut self, captured: &Captured, label: &str, path: &[&str]) -> String {
        let value: serde_json::Value =
            serde_json::from_str(&captured.stdout).unwrap_or_else(|error| {
                panic!("{label}: stdout is not JSON ({error}): {}", captured.stdout)
            });
        let mut current = &value;
        for key in path {
            current = current.get(*key).unwrap_or_else(|| {
                panic!("{label}: missing field {path:?} in {}", captured.stdout)
            });
        }
        current
            .as_str()
            .unwrap_or_else(|| panic!("{label}: field {path:?} is not a string"))
            .to_owned()
    }

    fn finish(self) -> Result<usize, String> {
        if self.failures.is_empty() {
            Ok(self.steps)
        } else {
            Err(format!(
                "{} of {} steps differ:\n\n{}",
                self.failures.len(),
                self.steps,
                self.failures.join("\n")
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Normalization
// ---------------------------------------------------------------------------

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("test regex")
}

fn normalize(captured: &Captured, side: &Side) -> String {
    let digest = regex(r"\b[0-9a-f]{64}\b");
    let timestamp = regex(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|\+00:00)");
    let request_id = regex(r"commit-[A-Za-z0-9._:-]+-[0-9a-f]{16}-[0-9a-f]{16}");
    let data = side.data.display().to_string();
    let home = side.home.display().to_string();
    let over = side.path_override.display().to_string();
    let render = |text: &str| -> String {
        let text = text.replace(&over, "<override>");
        let text = text.replace(&data, "<data>");
        let text = text.replace(&home, "<home>");
        let text = digest.replace_all(&text, "<digest>").into_owned();
        let text = timestamp.replace_all(&text, "<timestamp>").into_owned();
        request_id.replace_all(&text, "<request-id>").into_owned()
    };
    format!(
        "exit: {}\n--stdout--\n{}\n--stderr--\n{}\n",
        captured.code,
        render(&captured.stdout),
        render(&captured.stderr)
    )
}

// ---------------------------------------------------------------------------
// Data-directory state
// ---------------------------------------------------------------------------

fn digest_segment() -> Regex {
    regex(r"^[0-9a-f]{64}$")
}

fn tree(root: &Path) -> Vec<TreeEntry> {
    let mut entries = Vec::new();
    if !root.exists() {
        return entries;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let read = fs::read_dir(&directory).expect("read_dir");
        for entry in read {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            let relative = path.strip_prefix(root).expect("under root");
            let mut parts: Vec<String> = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect();
            let object_prefix = parts.first().is_some_and(|part| part == "artifacts")
                && parts.get(1).is_some_and(|part| part == "objects");
            for (index, part) in parts.iter_mut().enumerate() {
                if digest_segment().is_match(part) {
                    *part = "<digest>".to_owned();
                } else if object_prefix && index == 2 && part.len() == 2 {
                    *part = "<prefix>".to_owned();
                }
            }
            let rendered = parts.join("/");
            if path.is_dir() {
                entries.push(TreeEntry {
                    path: rendered,
                    kind: "dir",
                });
                stack.push(path);
            } else {
                entries.push(TreeEntry {
                    path: rendered,
                    kind: "file",
                });
            }
        }
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries.dedup();
    entries
}

fn normalize_state_text(value: &str) -> String {
    let request_id = regex(r"commit-[A-Za-z0-9._:-]+-[0-9a-f]{16}-[0-9a-f]{16}");
    let digest = regex(r"\b[0-9a-f]{64}\b");
    let value = request_id.replace_all(value, "<request-id>").into_owned();
    digest.replace_all(&value, "<digest>").into_owned()
}

fn sqlite_state(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()?;
    let mut out = String::new();
    let tables: Vec<String> = {
        let mut statement = connection
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .ok()?;
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .ok()?
            .collect::<Result<Vec<_>, _>>()
            .ok()?
    };
    for table in tables {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .ok()?;
        out.push_str(&format!("{table}: {count}\n"));
        match table.as_str() {
            "events" => {
                let mut statement = connection
                    .prepare(
                        "SELECT sequence, event_type, aggregate_id, workflow_id, run_id \
                         FROM events ORDER BY sequence",
                    )
                    .ok()?;
                let rows = statement
                    .query_map([], |row| {
                        Ok(format!(
                            "{}|{}|{}|{}|{}",
                            row.get::<_, i64>(0)?,
                            normalize_state_text(&row.get::<_, String>(1)?),
                            normalize_state_text(&row.get::<_, String>(2)?),
                            normalize_state_text(
                                &row.get::<_, Option<String>>(3)?.unwrap_or_default()
                            ),
                            normalize_state_text(
                                &row.get::<_, Option<String>>(4)?.unwrap_or_default()
                            ),
                        ))
                    })
                    .ok()?
                    .collect::<Result<Vec<_>, _>>()
                    .ok()?;
                for row in rows {
                    out.push_str(&format!("  event {row}\n"));
                }
            }
            "design_revisions" => {
                let mut statement = connection
                    .prepare(
                        "SELECT revision_id, design_id, sequence FROM design_revisions \
                         ORDER BY revision_id",
                    )
                    .ok()?;
                let rows = statement
                    .query_map([], |row| {
                        Ok(format!(
                            "{}|{}|{}",
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    })
                    .ok()?
                    .collect::<Result<Vec<_>, _>>()
                    .ok()?;
                for row in rows {
                    out.push_str(&format!("  revision {row}\n"));
                }
            }
            "approval_requests" => {
                let mut statement = connection
                    .prepare(
                        "SELECT status, workflow_id FROM approval_requests \
                         ORDER BY status, workflow_id",
                    )
                    .ok()?;
                let rows = statement
                    .query_map([], |row| {
                        Ok(format!(
                            "{}|{}",
                            normalize_state_text(&row.get::<_, String>(0)?),
                            normalize_state_text(
                                &row.get::<_, Option<String>>(1)?.unwrap_or_default()
                            ),
                        ))
                    })
                    .ok()?
                    .collect::<Result<Vec<_>, _>>()
                    .ok()?;
                for row in rows {
                    out.push_str(&format!("  approval {row}\n"));
                }
            }
            "design_specs" => {
                let mut statement = connection
                    .prepare("SELECT id FROM design_specs ORDER BY id")
                    .ok()?;
                let rows = statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .ok()?
                    .collect::<Result<Vec<_>, _>>()
                    .ok()?;
                for row in rows {
                    out.push_str(&format!("  spec {row}\n"));
                }
            }
            "schema_migrations" => {
                let mut statement = connection
                    .prepare("SELECT version, name FROM schema_migrations ORDER BY version")
                    .ok()?;
                let rows = statement
                    .query_map([], |row| {
                        Ok(format!(
                            "{}|{}",
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                        ))
                    })
                    .ok()?
                    .collect::<Result<Vec<_>, _>>()
                    .ok()?;
                for row in rows {
                    out.push_str(&format!("  migration {row}\n"));
                }
            }
            _ => {}
        }
    }
    Some(out)
}

fn compare_state(python_root: &Path, rust_root: &Path) -> Result<(), String> {
    let python_tree = tree(python_root);
    let rust_tree = tree(rust_root);
    if python_tree != rust_tree {
        let missing: Vec<_> = python_tree
            .iter()
            .filter(|entry| !rust_tree.contains(entry))
            .collect();
        let extra: Vec<_> = rust_tree
            .iter()
            .filter(|entry| !python_tree.contains(entry))
            .collect();
        return Err(format!(
            "data-directory trees differ\nmissing from rust: {missing:?}\nextra in rust: {extra:?}"
        ));
    }
    for name in ["kala.sqlite3", "journal.sqlite3"] {
        match (
            sqlite_state(&python_root.join(name)),
            sqlite_state(&rust_root.join(name)),
        ) {
            (None, None) => {}
            (Some(python), Some(rust)) => {
                if python != rust {
                    return Err(format!(
                        "{name} state differs\n--- python ---\n{python}\n--- rust ---\n{rust}\n"
                    ));
                }
            }
            (python, rust) => {
                return Err(format!(
                    "{name} existence differs (python {}, rust {})",
                    python.is_some(),
                    rust.is_some()
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The corpus
// ---------------------------------------------------------------------------

#[test]
fn help_text_matches_for_every_command_and_width() {
    if !python_available() {
        eprintln!(
            "skipping the live differential: {} cannot import kala_agent",
            python_binary().display()
        );
        return;
    }
    let mut harness = Harness::new();
    let mut argv: Vec<String> = Vec::new();
    for columns in COLUMNS {
        harness.step("root help", &["--help".to_owned()], Some(columns), &[]);
        for command in COMMANDS {
            argv.clear();
            argv.push(command.to_owned());
            argv.push("--help".to_owned());
            let (_, _) = harness.step(command, &argv, Some(columns), &[]);
        }
    }
    let steps = harness.steps;
    harness
        .finish()
        .unwrap_or_else(|error| panic!("help differential failed:\n{error}"));
    eprintln!("{steps} live help-document steps matched");
}

#[test]
fn command_line_behaviour_matches_the_python_cli() {
    if !python_available() {
        eprintln!(
            "skipping the live differential: {} cannot import kala_agent",
            python_binary().display()
        );
        return;
    }
    let mut harness = Harness::new();

    // Data-directory lifecycle.
    harness.run_pair("init creates", &["init"]);
    harness.run_pair("init verifies", &["init"]);
    harness.run_pair("domains", &["domains"]);
    harness.run_pair("token before a launch", &["token"]);

    // A stored launch token is the one secret the CLI prints, and only for
    // `token`. Write the same valid token on both sides.
    for side in [&harness.python, &harness.rust] {
        fs::write(side.data.join("api-token"), FIXED_TOKEN).expect("token file");
    }
    harness.run_pair("token after a launch", &["token"]);

    // Evaluations.
    harness.run_pair(
        "evaluate mechanical",
        &["evaluate", "--domain", "mechanical"],
    );
    harness.run_pair("evaluate building", &["evaluate", "--domain", "building"]);
    harness.run_pair("evaluate wing", &["evaluate", "--domain", "wing"]);
    harness.run_pair(
        "evaluate failing constraints",
        &[
            "evaluate",
            "--domain",
            "mechanical",
            "--param",
            "thickness_mm=0.5",
            "--param",
            "minimum_thickness_mm=2.0",
        ],
    );
    harness.run_pair(
        "evaluate with parameters json",
        &[
            "evaluate",
            "--domain",
            "wing",
            "--parameters-json",
            "{\"span_m\": 8.0}",
            "--design-id",
            "wing-8",
        ],
    );
    harness.run_pair(
        "evaluate with duplicate keys",
        &[
            "evaluate",
            "--domain",
            "mechanical",
            "--parameters-json",
            "{\"thickness_mm\": 2.5, \"thickness_mm\": 1.5}",
        ],
    );
    harness.run_pair(
        "evaluate with null",
        &[
            "evaluate",
            "--domain",
            "mechanical",
            "--parameters-json",
            "{\"thickness_mm\": null}",
        ],
    );
    harness.run_pair(
        "evaluate with NaN",
        &[
            "evaluate",
            "--domain",
            "mechanical",
            "--param",
            "thickness_mm=NaN",
        ],
    );
    harness.run_pair(
        "evaluate with a boolean",
        &[
            "evaluate",
            "--domain",
            "mechanical",
            "--param",
            "thickness_mm=true",
        ],
    );
    harness.run_pair(
        "evaluate with an array",
        &[
            "evaluate",
            "--domain",
            "mechanical",
            "--param",
            "thickness_mm=[2.5]",
        ],
    );
    harness.run_pair(
        "evaluate with a bad override",
        &["evaluate", "--domain", "mechanical", "--param", "broken"],
    );
    harness.run_pair(
        "evaluate with truncated json",
        &[
            "evaluate",
            "--domain",
            "mechanical",
            "--parameters-json",
            "{",
        ],
    );
    harness.run_pair(
        "evaluate with a non-object",
        &[
            "evaluate",
            "--domain",
            "mechanical",
            "--parameters-json",
            "[1]",
        ],
    );
    harness.run_pair(
        "evaluate unknown domain",
        &["evaluate", "--domain", "not-a-domain"],
    );
    harness.run_pair(
        "evaluate unsafe design id",
        &[
            "evaluate",
            "--domain",
            "mechanical",
            "--design-id",
            "../escape",
        ],
    );
    harness.run_pair("evaluate empty domain", &["evaluate", "--domain", ""]);

    // The mechanical workflow, approval and rejection.
    harness.run_pair(
        "run mechanical",
        &["run-mechanical", "--design-id", "cli-bracket"],
    );
    harness.run_pair(
        "run mechanical again",
        &["run-mechanical", "--design-id", "cli-bracket"],
    );
    harness.run_pair(
        "workflow status",
        &["workflow-status", "--workflow-id", "workflow-cli-bracket"],
    );
    harness.run_pair(
        "approve checkpoint",
        &[
            "approve",
            "--workflow-id",
            "workflow-cli-bracket",
            "--actor",
            "engineer-a",
        ],
    );
    harness.run_pair(
        "approve a finished workflow",
        &["approve", "--workflow-id", "workflow-cli-bracket"],
    );
    harness.run_pair(
        "run rejected workflow",
        &["run-mechanical", "--design-id", "cli-rejected"],
    );
    harness.run_pair(
        "reject checkpoint",
        &[
            "reject",
            "--workflow-id",
            "workflow-cli-rejected",
            "--reason",
            "operator declined",
        ],
    );
    harness.run_pair(
        "rejected workflow status",
        &["workflow-status", "--workflow-id", "workflow-cli-rejected"],
    );
    harness.run_pair(
        "unknown workflow status",
        &["workflow-status", "--workflow-id", "workflow-absent"],
    );

    // Journal pages and filters.
    harness.run_pair("events page", &["events", "--limit", "3"]);
    harness.run_pair(
        "events after sequence",
        &[
            "events",
            "--after-sequence",
            "3",
            "--workflow-id",
            "workflow-cli-bracket",
        ],
    );
    harness.run_pair(
        "events by type",
        &["events", "--event-type", "domain.evaluated", "--limit", "2"],
    );
    harness.run_pair(
        "events by aggregate",
        &[
            "events",
            "--aggregate-id",
            "bracket-reference",
            "--limit",
            "2",
        ],
    );
    harness.run_pair("events zero limit", &["events", "--limit", "0"]);
    harness.run_pair(
        "events negative sequence",
        &["events", "--after-sequence", "-1", "--limit", "1"],
    );
    harness.run_pair("events full page", &["events", "--limit", "1000"]);

    // The two-phase revision commit. The identities differ by design, so each
    // side's own digests and request id feed its own commit; the outputs still
    // compare because only the opaque identities are normalized.
    let prepare_args = [
        "prepare-revision".to_owned(),
        "--design-id".to_owned(),
        "cli-commit".to_owned(),
        "--domain".to_owned(),
        "mechanical".to_owned(),
        "--summary".to_owned(),
        "reviewed configuration".to_owned(),
    ];
    let (python_prepared, rust_prepared) =
        harness.step("prepare revision", &prepare_args, None, &[]);
    harness.run_pair("events after prepare", &["events", "--limit", "5"]);
    let python_request = harness.field(
        &python_prepared,
        "prepare revision",
        &["data", "request_id"],
    );
    let python_state = harness.field(
        &python_prepared,
        "prepare revision",
        &["data", "state_digest"],
    );
    let python_action = harness.field(
        &python_prepared,
        "prepare revision",
        &["data", "action_digest"],
    );
    let rust_request = harness.field(&rust_prepared, "prepare revision", &["data", "request_id"]);
    let rust_state = harness.field(
        &rust_prepared,
        "prepare revision",
        &["data", "state_digest"],
    );
    let rust_action = harness.field(
        &rust_prepared,
        "prepare revision",
        &["data", "action_digest"],
    );
    let commit_args = |request: &str, state: &str, action: &str| -> Vec<String> {
        vec![
            "commit-revision".to_owned(),
            "--request-id".to_owned(),
            request.to_owned(),
            "--state-digest".to_owned(),
            state.to_owned(),
            "--action-digest".to_owned(),
            action.to_owned(),
            "--actor".to_owned(),
            "cli-reviewer".to_owned(),
        ]
    };
    let _ = harness.step_with(
        "commit revision",
        &commit_args(&python_request, &python_state, &python_action),
        &commit_args(&rust_request, &rust_state, &rust_action),
        None,
        &[],
    );
    harness.run_pair(
        "commit again",
        &["commit-revision", "--request-id", "commit-absent"],
    );

    // Stale prepared commit: the head moved after the first preparation.
    let stale_args = [
        "prepare-revision".to_owned(),
        "--design-id".to_owned(),
        "cli-stale".to_owned(),
        "--domain".to_owned(),
        "mechanical".to_owned(),
    ];
    let (python_stale, rust_stale) = harness.step("prepare stale", &stale_args, None, &[]);
    let moved_args = [
        "prepare-revision".to_owned(),
        "--design-id".to_owned(),
        "cli-stale".to_owned(),
        "--domain".to_owned(),
        "mechanical".to_owned(),
        "--summary".to_owned(),
        "moved on".to_owned(),
    ];
    let (python_moved, rust_moved) = harness.step("prepare moved", &moved_args, None, &[]);
    let python_moved_request =
        harness.field(&python_moved, "prepare moved", &["data", "request_id"]);
    let rust_moved_request = harness.field(&rust_moved, "prepare moved", &["data", "request_id"]);
    let _ = harness.step_with(
        "commit moved",
        &[
            "commit-revision".to_owned(),
            "--request-id".to_owned(),
            python_moved_request,
        ],
        &[
            "commit-revision".to_owned(),
            "--request-id".to_owned(),
            rust_moved_request,
        ],
        None,
        &[],
    );
    let python_stale_request =
        harness.field(&python_stale, "prepare stale", &["data", "request_id"]);
    let rust_stale_request = harness.field(&rust_stale, "prepare stale", &["data", "request_id"]);
    let _ = harness.step_with(
        "commit stale",
        &[
            "commit-revision".to_owned(),
            "--request-id".to_owned(),
            python_stale_request,
        ],
        &[
            "commit-revision".to_owned(),
            "--request-id".to_owned(),
            rust_stale_request,
        ],
        None,
        &[],
    );

    // A rejected revision commit.
    let declined_args = [
        "prepare-revision".to_owned(),
        "--design-id".to_owned(),
        "cli-declined".to_owned(),
        "--domain".to_owned(),
        "mechanical".to_owned(),
    ];
    let (python_declined, rust_declined) =
        harness.step("prepare declined", &declined_args, None, &[]);
    let python_declined_request = harness.field(
        &python_declined,
        "prepare declined",
        &["data", "request_id"],
    );
    let rust_declined_request =
        harness.field(&rust_declined, "prepare declined", &["data", "request_id"]);
    let reject_args = |request: &str| -> Vec<String> {
        vec![
            "commit-revision".to_owned(),
            "--request-id".to_owned(),
            request.to_owned(),
            "--decision".to_owned(),
            "reject".to_owned(),
            "--reason".to_owned(),
            "declined locally".to_owned(),
        ]
    };
    let _ = harness.step_with(
        "reject revision commit",
        &reject_args(&python_declined_request),
        &reject_args(&rust_declined_request),
        None,
        &[],
    );
    harness.run_pair(
        "declined workflow status",
        &["workflow-status", "--workflow-id", "cli-declined"],
    );

    // Application-level validation.
    harness.run_pair(
        "prepare unsafe design",
        &[
            "prepare-revision",
            "--design-id",
            "../escape",
            "--domain",
            "mechanical",
        ],
    );
    harness.run_pair(
        "prepare unknown domain",
        &[
            "prepare-revision",
            "--design-id",
            "cli-bad",
            "--domain",
            "absent",
        ],
    );
    harness.run_pair(
        "commit unknown request",
        &["commit-revision", "--request-id", "commit-absent"],
    );
    harness.run_pair(
        "commit bad digest",
        &[
            "commit-revision",
            "--request-id",
            "commit-x",
            "--state-digest",
            "nope",
        ],
    );
    harness.run_pair(
        "commit bad action digest",
        &[
            "commit-revision",
            "--request-id",
            "commit-x",
            "--action-digest",
            "ZZZ",
        ],
    );
    harness.run_pair(
        "actor empty",
        &["run-mechanical", "--design-id", "x", "--actor", ""],
    );
    harness.run_pair(
        "actor too long",
        &[
            "run-mechanical",
            "--design-id",
            "x",
            "--actor",
            &"a".repeat(65),
        ],
    );
    harness.run_pair(
        "reason too long",
        &[
            "run-mechanical",
            "--design-id",
            "x",
            "--reason",
            &"r".repeat(513),
        ],
    );
    harness.run_pair(
        "reserved workflow id",
        &[
            "run-mechanical",
            "--design-id",
            "x",
            "--workflow-id",
            "kernel-state:policy",
        ],
    );
    harness.run_pair(
        "reserved event filter",
        &["events", "--workflow-id", "kernel-state:policy"],
    );
    harness.run_pair(
        "unsafe workflow id",
        &["workflow-status", "--workflow-id", "../../../escape"],
    );

    // Long messages are truncated to 512 code points by the error writer.
    let long_domain = "d".repeat(2_000);
    harness.run_pair(
        "long unknown domain",
        &["evaluate", "--domain", &long_domain],
    );
    // Parameter name trimming and empty names.
    harness.run_pair(
        "parameter name trimming",
        &[
            "evaluate",
            "--domain",
            "mechanical",
            "--param",
            "  thickness_mm  = 2.5 ",
        ],
    );
    harness.run_pair(
        "empty parameter name",
        &["evaluate", "--domain", "mechanical", "--param", "  =1"],
    );
    harness.run_pair(
        "parameter with two separators",
        &["evaluate", "--domain", "mechanical", "--param", "a=b=c"],
    );
    // An override wins over the JSON document, and a later override wins over
    // an earlier one.
    harness.run_pair(
        "parameters json then override",
        &[
            "evaluate",
            "--domain",
            "mechanical",
            "--parameters-json",
            "{\"thickness_mm\": 1.5}",
            "--param",
            "thickness_mm=2.5",
            "--param",
            "minimum_thickness_mm=2.0",
        ],
    );
    // A declared endpoint is reported as declared; a credential-shaped value
    // is never printed (the security suite asserts the value is absent).
    let _ = harness.step(
        "declared endpoint and secret",
        &["init".to_owned()],
        None,
        &[
            ("KALA_AGENT_MODEL_ENDPOINT", "http://127.0.0.1:11434/v1"),
            (
                "KALA_AGENT_API_KEY",
                "sk-differential-secret-must-not-be-printed",
            ),
        ],
    );

    // A document nested past CPython's recursion limit is a local failure with
    // the same class name on both sides.
    let deep = format!("{}0{}", "[".repeat(60_000), "]".repeat(60_000));
    harness.run_pair(
        "deep parameter document",
        &[
            "evaluate",
            "--domain",
            "mechanical",
            "--param",
            &format!("thickness_mm={deep}"),
        ],
    );
    let deep_object = format!(
        "{{\"thickness_mm\":{}0{}}}",
        "[".repeat(60_000),
        "]".repeat(60_000)
    );
    harness.run_pair(
        "deep parameters json",
        &[
            "evaluate",
            "--domain",
            "mechanical",
            "--parameters-json",
            &deep_object,
        ],
    );
    // Non-ASCII values count their escaped JSON bytes in the size check.
    let non_ascii = "\u{00e9}".repeat(3_000);
    harness.run_pair(
        "non ascii parameter value",
        &[
            "evaluate",
            "--domain",
            "mechanical",
            "--param",
            &format!("value={non_ascii}"),
        ],
    );
    // The data directory may not be a filesystem root.
    harness.run_pair("root data dir", &["--data-dir", "/", "init"]);

    // The launch validation shared with the Python command. Every one of
    // these fails before the token and the socket; the live launch contract
    // is in `serve_differential.rs` and `serve_live.rs`.
    harness.run_pair("serve wildcard host", &["serve", "--host", "0.0.0.0"]);
    harness.run_pair("serve ipv6 wildcard", &["serve", "--host", "::"]);
    harness.run_pair("serve routable host", &["serve", "--host", "192.168.1.10"]);
    harness.run_pair("serve blank host", &["serve", "--host", "  "]);
    harness.run_pair("serve port zero", &["serve", "--port", "0"]);
    harness.run_pair("serve port too high", &["serve", "--port", "65536"]);
    harness.run_pair("serve port negative", &["serve", "--port", "-1"]);
    harness.run_pair("serve ambiguous host", &["serve", "--h", "127.0.0.1"]);

    // argparse surface.
    harness.run_pair("no command", &[]);
    harness.run_pair("unknown command", &["not-a-command"]);
    harness.run_pair("unknown flag", &["init", "--unknown-flag"]);
    harness.run_pair(
        "extra positional",
        &["evaluate", "--domain", "mechanical", "extra"],
    );
    harness.run_pair("expected value", &["evaluate", "--domain"]);
    harness.run_pair("missing required", &["evaluate"]);
    harness.run_pair("missing several required", &["prepare-revision"]);
    harness.run_pair("invalid int", &["events", "--limit", "not-a-number"]);
    harness.run_pair(
        "invalid choice",
        &["commit-revision", "--request-id", "x", "--decision", "nope"],
    );
    harness.run_pair("ambiguous option", &["evaluate", "--d", "mechanical"]);
    harness.run_pair("unknown root option", &["--bogus", "init"]);
    harness.run_pair("unknown root short option", &["-x", "init"]);
    harness.run_pair(
        "root option after command",
        &["init", "--data-dir", "/tmp/x"],
    );
    harness.run_pair(
        "negative value",
        &["evaluate", "--domain", "mechanical", "--param", "-1"],
    );
    harness.run_pair(
        "option-like value",
        &["evaluate", "--domain", "mechanical", "--param", "-x=1"],
    );
    harness.run_pair(
        "option separator",
        &["evaluate", "--domain", "mechanical", "--", "--x"],
    );
    harness.run_pair("help abbreviation", &["--hel"]);
    harness.run_pair("short help suffix", &["init", "-hx"]);
    harness.run_pair("help explicit argument", &["--help=x"]);
    harness.run_pair("integer underscore", &["events", "--limit", "1_0"]);
    harness.run_pair("integer plus", &["events", "--limit", "+5"]);
    harness.run_pair("integer empty", &["events", "--limit="]);
    harness.run_pair("int option abbreviation", &["events", "--lim", "2"]);
    harness.run_pair("option abbreviation", &["evaluate", "--dom", "mechanical"]);
    harness.run_pair(
        "explicit option value",
        &["evaluate", "--domain=mechanical"],
    );
    harness.run_pair(
        "repeated option",
        &["events", "--limit", "5", "--limit", "6"],
    );
    harness.run_pair("terminator before command", &["--", "init"]);
    harness.run_pair("terminator then flag", &["--", "--help"]);

    // The root `--data-dir` override, including abbreviation and precedence.
    let python_override = harness.python.path_override.display().to_string();
    let rust_override = harness.rust.path_override.display().to_string();
    let _ = harness.step_with(
        "data-dir override",
        &[
            "--data-dir".to_owned(),
            python_override.clone(),
            "init".to_owned(),
        ],
        &[
            "--data-dir".to_owned(),
            rust_override.clone(),
            "init".to_owned(),
        ],
        None,
        &[],
    );
    assert!(harness.python.path_override.exists());
    assert!(harness.rust.path_override.exists());
    let _ = harness.step_with(
        "data-dir override abbreviation",
        &["--data-d".to_owned(), python_override, "domains".to_owned()],
        &["--data-d".to_owned(), rust_override, "domains".to_owned()],
        None,
        &[],
    );

    // Configuration failures, in a separate pair of data directories.
    let mut config = Harness::new();
    let _ = config.step(
        "relative data dir",
        &["init".to_owned()],
        None,
        &[("KALA_AGENT_DATA_DIR", "relative/path")],
    );
    let _ = config.step(
        "worker zero",
        &["init".to_owned()],
        None,
        &[("KALA_AGENT_WORKER_COUNT", "0")],
    );
    let _ = config.step(
        "worker high",
        &["init".to_owned()],
        None,
        &[("KALA_AGENT_WORKER_COUNT", "9")],
    );
    let _ = config.step(
        "worker not an integer",
        &["init".to_owned()],
        None,
        &[("KALA_AGENT_WORKER_COUNT", "abc")],
    );
    let _ = config.step(
        "negative timeout",
        &["init".to_owned()],
        None,
        &[("KALA_AGENT_REQUEST_TIMEOUT_SECONDS", "-1")],
    );
    let _ = config.step(
        "timeout not a number",
        &["init".to_owned()],
        None,
        &[("KALA_AGENT_REQUEST_TIMEOUT_SECONDS", "abc")],
    );
    let _ = config.step(
        "bad provider",
        &["init".to_owned()],
        None,
        &[("KALA_AGENT_MODEL_PROVIDER", "bad provider")],
    );
    let _ = config.step(
        "non-http endpoint",
        &["init".to_owned()],
        None,
        &[("KALA_AGENT_MODEL_ENDPOINT", "ftp://example.com")],
    );
    let _ = config.step(
        "credentialed endpoint",
        &["init".to_owned()],
        None,
        &[("KALA_AGENT_MODEL_ENDPOINT", "http://user:pass@example.com")],
    );
    let _ = config.step(
        "two invalid fields",
        &["init".to_owned()],
        None,
        &[
            ("KALA_AGENT_DATA_DIR", "relative"),
            ("KALA_AGENT_WORKER_COUNT", "0"),
        ],
    );
    fs::write(&config.python.data, b"not a directory").expect("file data dir");
    fs::write(&config.rust.data, b"not a directory").expect("file data dir");
    let _ = config.step("data dir is a file", &["init".to_owned()], None, &[]);

    // The default data directory comes from HOME when the environment names
    // none. Both HOMEs are temporary.
    let _ = harness.step_default_home("default home data dir", &["init".to_owned()]);
    if let Err(error) = compare_state(
        &harness.python.home.join(".local/share/kala-agent"),
        &harness.rust.home.join(".local/share/kala-agent"),
    ) {
        harness
            .failures
            .push(format!("HOME default data directory: {error}"));
    }

    // Compare the final data-directory state of the main sequence.
    if let Err(error) = compare_state(&harness.python.data, &harness.rust.data) {
        harness
            .failures
            .push(format!("main data directory: {error}"));
    }
    if let Err(error) = compare_state(&harness.python.path_override, &harness.rust.path_override) {
        harness
            .failures
            .push(format!("override data directory: {error}"));
    }

    let steps = harness.steps + config.steps;
    let mut failures = harness.failures;
    failures.extend(config.failures);
    if failures.is_empty() {
        eprintln!("{steps} live differential steps matched");
    } else {
        panic!(
            "{} of {steps} differential steps differ:\n\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}
