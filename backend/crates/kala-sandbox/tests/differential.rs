//! Test-only differential against the live Python source.
//!
//! The Rust crate never invokes Python at product time. This test uses the
//! legacy implementation as an oracle so the payload bytes, the refusal
//! messages, the error precedence and the jail plan are checked against
//! actual source rather than against a hand-written expectation.
//!
//! The corpus is deterministic: a fixed-seed generator produces the valid and
//! hostile cases, so a failure is reproducible.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kala_sandbox::{
    InterpreterLayout, IsolationProfile, SandboxProbe, SandboxSpec, SandboxVerdict,
    build_sandbox_spec_with, child_resource_paths, exit_code_from_status, judge_sandbox,
    platform_name, probe_entry, resolve_system_bindings_with, unshadowed_scratch_paths,
};
use serde_json::{Value, json};

/// The deterministic generator of the corpus.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 11
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[(self.next() as usize) % items.len()]
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repository root")
}

fn python_executable() -> PathBuf {
    let repository = repository_root();
    let candidate = repository.join(".venv/bin/python");
    if candidate.is_file() {
        candidate
    } else {
        PathBuf::from("python3")
    }
}

/// Build the interpreter layout from the facts the oracle reports.
fn layout_from(value: &Value) -> InterpreterLayout {
    let optional = |name: &str| -> Option<PathBuf> {
        value[name]
            .as_str()
            .filter(|text| !text.is_empty())
            .map(PathBuf::from)
    };
    InterpreterLayout {
        executable: PathBuf::from(value["executable"].as_str().expect("executable")),
        prefix: PathBuf::from(value["prefix"].as_str().expect("prefix")),
        base_prefix: PathBuf::from(value["base_prefix"].as_str().expect("base prefix")),
        exec_prefix: PathBuf::from(value["exec_prefix"].as_str().expect("exec prefix")),
        base_exec_prefix: PathBuf::from(
            value["base_exec_prefix"]
                .as_str()
                .expect("base exec prefix"),
        ),
        module_directory: PathBuf::from(
            value["module_directory"]
                .as_str()
                .expect("module directory"),
        ),
        stdlib: optional("stdlib"),
        platstdlib: optional("platstdlib"),
        purelib: optional("purelib"),
        platlib: optional("platlib"),
    }
}

/// Run the oracle with the given cases and return its answers.
fn run_oracle(cases: &[Value]) -> Vec<Value> {
    let repository = repository_root();
    let script = repository.join("backend/crates/kala-sandbox/tests/python_oracle.py");
    let mut child = Command::new(python_executable())
        .arg("-I")
        .arg(&script)
        .current_dir(&repository)
        .env("PYTHONPATH", repository.join("src"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the python oracle starts");
    let request = json!({ "cases": cases });
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(request.to_string().as_bytes())
        .expect("the request is written");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("the oracle completes");
    assert!(
        output.status.success(),
        "the python oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: Value = serde_json::from_slice(&output.stdout).expect("the oracle emits JSON");
    document["results"]
        .as_array()
        .expect("the oracle emits an array")
        .clone()
}

fn ok(value: Value) -> Value {
    json!({ "ok": true, "value": value })
}

fn failed(kind: &str, message: &str) -> Value {
    json!({ "ok": false, "error": kind, "message": message })
}

fn controls_of(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

const ALL: [&str; 12] = [
    "chroot_workspace_only",
    "device_nodes_minimal",
    "ipc_namespace",
    "mount_namespace",
    "network_isolation",
    "pid_namespace",
    "procfs",
    "readonly_system_bindings",
    "scratch_tmpfs",
    "user_namespace",
    "uts_namespace",
    "workspace_read_write",
];

/// The index of the interpreter-layout case in the corpus.
const LAYOUT_CASE: usize = 1;

const REQUIRED: [&str; 6] = [
    "chroot_workspace_only",
    "mount_namespace",
    "network_isolation",
    "readonly_system_bindings",
    "user_namespace",
    "workspace_read_write",
];

/// The expected answer of the Rust port for one case.
fn rust_answer(case: &Value, oracle: &[Value]) -> Value {
    match case["kind"].as_str().expect("kind") {
        "platform_name" => ok(json!(platform_name())),
        "interpreter_layout" => ok(oracle[LAYOUT_CASE]["value"].clone()),
        "child_resource_paths" => {
            let layout = layout_from(&oracle[LAYOUT_CASE]["value"]);
            let paths = child_resource_paths(&layout);
            ok(json!(paths))
        }
        "resolve_system_bindings" => {
            let layout = layout_from(&oracle[LAYOUT_CASE]["value"]);
            let child = child_resource_paths(&layout);
            let roots: Vec<PathBuf> = case["import_roots"]
                .as_array()
                .expect("import roots")
                .iter()
                .map(|item| PathBuf::from(item.as_str().expect("path")))
                .collect();
            ok(json!(resolve_system_bindings_with(&roots, &child)))
        }
        "build_spec" => {
            let controls = controls_of(
                &case["controls"]
                    .as_array()
                    .expect("controls")
                    .iter()
                    .map(|item| item.as_str().expect("control"))
                    .collect::<Vec<_>>(),
            );
            let root = PathBuf::from(case["root"].as_str().expect("root"));
            let workspace = PathBuf::from(case["workspace"].as_str().expect("workspace"));
            let roots: Vec<PathBuf> = case["import_roots"]
                .as_array()
                .expect("import roots")
                .iter()
                .map(|item| PathBuf::from(item.as_str().expect("path")))
                .collect();
            let scratch_bytes = case["scratch_bytes"].as_i64().expect("scratch bytes");
            let layout = layout_from(&oracle[LAYOUT_CASE]["value"]);
            let child = child_resource_paths(&layout);
            match build_sandbox_spec_with(
                &controls,
                &root,
                &workspace,
                &roots,
                scratch_bytes,
                &child,
            ) {
                Ok(spec) => ok(json!(spec.to_json())),
                Err(error) => failed("SandboxSetupError", error.detail()),
            }
        }
        "spec_from_payload" => match SandboxSpec::from_payload(&case["payload"]) {
            Ok(spec) => ok(json!(spec.to_json())),
            Err(error) => failed("SandboxSetupError", error.detail()),
        },
        "probe_payload" => {
            let payload = &case["probe"];
            let probe = SandboxProbe {
                available: payload["available"].as_bool().expect("available"),
                platform: payload["platform"].as_str().expect("platform").to_owned(),
                controls: payload["controls"]
                    .as_array()
                    .expect("controls")
                    .iter()
                    .map(|item| item.as_str().expect("control").to_owned())
                    .collect(),
                detail: payload["detail"].as_str().expect("detail").to_owned(),
                elapsed_ms: payload["elapsed_ms"].as_f64().expect("elapsed"),
            };
            ok(json!(probe.to_json()))
        }
        "verdict_payload" => {
            let payload = &case["verdict"];
            let verdict = SandboxVerdict {
                allowed: payload["allowed"].as_bool().expect("allowed"),
                reason: payload["reason"].as_str().expect("reason").to_owned(),
                controls: payload["controls"]
                    .as_array()
                    .expect("controls")
                    .iter()
                    .map(|item| item.as_str().expect("control").to_owned())
                    .collect(),
            };
            ok(json!(verdict.to_json()))
        }
        "judge" => {
            let profile = match case["profile"].as_str().expect("profile") {
                "portable" => IsolationProfile::Portable,
                "strict_linux" => IsolationProfile::StrictLinux,
                other => panic!("unknown profile {other}"),
            };
            let probe = case["probe"].as_object().map(|payload| SandboxProbe {
                available: payload["available"].as_bool().expect("available"),
                platform: payload["platform"].as_str().expect("platform").to_owned(),
                controls: payload["controls"]
                    .as_array()
                    .expect("controls")
                    .iter()
                    .map(|item| item.as_str().expect("control").to_owned())
                    .collect(),
                detail: payload["detail"].as_str().expect("detail").to_owned(),
                elapsed_ms: payload["elapsed_ms"].as_f64().expect("elapsed"),
            });
            ok(json!(judge_sandbox(profile, probe.as_ref()).to_json()))
        }
        "unshadowed" => {
            let spec = SandboxSpec::from_payload(&case["payload"]).expect("a valid payload");
            ok(json!(unshadowed_scratch_paths(&spec)))
        }
        "os_strerror" => {
            let mut reasons = serde_json::Map::new();
            for code in case["codes"].as_array().expect("codes") {
                let code = code.as_i64().expect("code") as i32;
                let error = std::io::Error::from_raw_os_error(code);
                reasons.insert(code.to_string(), json!(kala_sandbox::os_reason(&error)));
            }
            ok(Value::Object(reasons))
        }
        "exit_code" => ok(json!(exit_code_from_status(
            case["status"].as_i64().expect("status") as i32
        ))),
        "probe_entry" => match probe_entry(
            case["spec_payload"].as_str().expect("spec payload"),
            case["canary_payload"].as_str().expect("canary payload"),
        ) {
            Ok((spec, canary)) => ok(json!({ "spec": spec.to_json(), "canary": canary })),
            Err(error) => failed("SandboxProbeError", error.detail()),
        },
        "run_probe" => {
            let spec_payload = case["spec_payload"].as_str().expect("spec payload");
            let canary_payload = case["canary_payload"].as_str().expect("canary payload");
            let mut stdout = Vec::new();
            let exit = run_probe_captured(spec_payload, canary_payload, &mut stdout);
            ok(json!({ "exit": exit, "stdout": String::from_utf8_lossy(&stdout) }))
        }
        other => panic!("unknown case {other}"),
    }
}

/// Run the probe entry point in a child process and capture its output.
fn run_probe_captured(spec_payload: &str, canary_payload: &str, stdout: &mut Vec<u8>) -> i32 {
    // The probe changes the namespace of the calling process, so it runs in a
    // dedicated helper process.
    let helper = PathBuf::from(env!("CARGO_BIN_EXE_kala-sandbox-probe"));
    let child = Command::new(helper)
        .arg(spec_payload)
        .arg(canary_payload)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the probe helper starts");
    let output = child
        .wait_with_output()
        .expect("the probe helper completes");
    stdout.extend_from_slice(&output.stdout);
    output.status.code().unwrap_or(-1)
}

#[test]
fn live_python_source_matches_the_ported_contract() {
    let repository = repository_root();
    let workspace = repository.join("backend/crates/kala-sandbox");
    let import_roots = [repository.join("src")];
    let mut cases: Vec<Value> = Vec::new();

    cases.push(json!({ "kind": "platform_name" }));
    cases.push(json!({ "kind": "interpreter_layout" }));
    cases.push(json!({ "kind": "child_resource_paths" }));
    cases.push(json!({
        "kind": "resolve_system_bindings",
        "import_roots": import_roots.iter().map(|root| root.to_string_lossy()).collect::<Vec<_>>(),
    }));

    // A valid specification and the refusals around it.
    cases.push(json!({
        "kind": "build_spec",
        "controls": ALL,
        "root": "/tmp/kala-root",
        "workspace": "/tmp/kala-workspace",
        "import_roots": import_roots.iter().map(|root| root.to_string_lossy()).collect::<Vec<_>>(),
        "scratch_bytes": 1 << 20,
    }));
    for missing in REQUIRED {
        let remaining: Vec<&str> = REQUIRED
            .iter()
            .copied()
            .filter(|control| *control != missing)
            .collect();
        cases.push(json!({
            "kind": "build_spec",
            "controls": remaining,
            "root": "/tmp/kala-root",
            "workspace": "/tmp/kala-workspace",
            "import_roots": [],
            "scratch_bytes": 1 << 20,
        }));
    }
    cases.push(json!({
        "kind": "build_spec",
        "controls": ALL,
        "root": "relative/root",
        "workspace": "/tmp/kala-workspace",
        "import_roots": [],
        "scratch_bytes": 1 << 20,
    }));
    cases.push(json!({
        "kind": "build_spec",
        "controls": ALL,
        "root": "/tmp/kala-root",
        "workspace": "relative/workspace",
        "import_roots": [],
        "scratch_bytes": 1 << 20,
    }));
    cases.push(json!({
        "kind": "build_spec",
        "controls": ALL,
        "root": "/tmp/kala-root",
        "workspace": "/tmp/kala-workspace",
        "import_roots": [],
        "scratch_bytes": 0,
    }));
    cases.push(json!({
        "kind": "build_spec",
        "controls": ALL,
        "root": "/tmp/kala-root",
        "workspace": "/tmp/kala-workspace",
        "import_roots": [],
        "scratch_bytes": -5,
    }));

    // The specification payload, valid and hostile.
    let valid_spec = json!({
        "controls": ALL,
        "device_bindings": ["/dev/null"],
        "readonly_bindings": ["/usr"],
        "root": "/tmp/kala-root",
        "scratch_bytes": 1 << 20,
        "scratch_paths": ["/dev/shm", "/tmp"],
        "writable_bindings": ["/tmp/kala-workspace"],
        "workspace": "/tmp/kala-workspace",
    });
    cases.push(json!({ "kind": "spec_from_payload", "payload": valid_spec }));
    let mut hostile = valid_spec.clone();
    hostile.as_object_mut().expect("object").remove("root");
    cases.push(json!({ "kind": "spec_from_payload", "payload": hostile }));
    let mut hostile = valid_spec.clone();
    hostile["root"] = json!(7);
    cases.push(json!({ "kind": "spec_from_payload", "payload": hostile }));
    let mut hostile = valid_spec.clone();
    hostile["scratch_bytes"] = json!("not-a-number");
    cases.push(json!({ "kind": "spec_from_payload", "payload": hostile }));
    let mut hostile = valid_spec.clone();
    hostile["scratch_bytes"] = json!("4096");
    cases.push(json!({ "kind": "spec_from_payload", "payload": hostile }));
    let mut hostile = valid_spec.clone();
    hostile["scratch_bytes"] = json!(1.9);
    cases.push(json!({ "kind": "spec_from_payload", "payload": hostile }));
    let mut hostile = valid_spec.clone();
    hostile["scratch_bytes"] = json!(true);
    cases.push(json!({ "kind": "spec_from_payload", "payload": hostile }));
    let mut hostile = valid_spec.clone();
    hostile["root"] = json!("");
    cases.push(json!({ "kind": "spec_from_payload", "payload": hostile }));
    let mut hostile = valid_spec.clone();
    hostile["root"] = json!("//a//b/./c/");
    cases.push(json!({ "kind": "spec_from_payload", "payload": hostile }));
    let mut hostile = valid_spec.clone();
    hostile["controls"] = json!([]);
    cases.push(json!({ "kind": "spec_from_payload", "payload": hostile }));
    let mut hostile = valid_spec.clone();
    hostile["extra"] = json!({ "ignored": true });
    cases.push(json!({ "kind": "spec_from_payload", "payload": hostile }));

    // The probe payload, including the rounding of the elapsed time.
    for elapsed in [
        0.0, 1.23456, 8.835, 1.2345, 0.0625, 2.675, 1.0005, -1.2345, 183.679, 1000.0,
    ] {
        cases.push(json!({
            "kind": "probe_payload",
            "probe": {
                "available": true,
                "platform": "linux-x86_64",
                "controls": ["a", "b"],
                "detail": "fine",
                "elapsed_ms": elapsed,
            },
        }));
    }
    cases.push(json!({
        "kind": "probe_payload",
        "probe": {
            "available": false,
            "platform": "darwin",
            "controls": [],
            "detail": "",
            "elapsed_ms": 0.0,
        },
    }));

    // The verdict payload and the judge.
    cases.push(json!({
        "kind": "verdict_payload",
        "verdict": { "allowed": true, "reason": "because", "controls": ["a"] },
    }));
    cases.push(json!({
        "kind": "verdict_payload",
        "verdict": { "allowed": false, "reason": "", "controls": [] },
    }));
    let probe_payloads = [
        json!(null),
        json!({
            "available": false,
            "platform": "linux",
            "controls": [],
            "detail": "namespaces denied",
            "elapsed_ms": 0.0,
        }),
        json!({
            "available": false,
            "platform": "linux",
            "controls": [],
            "detail": "",
            "elapsed_ms": 0.0,
        }),
        json!({
            "available": true,
            "platform": "linux",
            "controls": ["mount_namespace", "user_namespace"],
            "detail": "",
            "elapsed_ms": 0.0,
        }),
    ];
    for profile in ["portable", "strict_linux"] {
        for probe in &probe_payloads {
            cases.push(json!({ "kind": "judge", "profile": profile, "probe": probe }));
        }
    }

    // The scratch shadowing and the wait status.
    cases.push(json!({
        "kind": "unshadowed",
        "payload": {
            "controls": ALL,
            "device_bindings": [],
            "readonly_bindings": ["/tmp"],
            "root": "/tmp/kala-root",
            "scratch_bytes": 1 << 20,
            "scratch_paths": ["/dev/shm", "/tmp", "/var"],
            "writable_bindings": ["/tmp/work", "/var"],
            "workspace": "/tmp/kala-workspace",
        },
    }));
    for status in [0, 9, 3, 0xffff, 0x7f, 255, 32512, 32768] {
        cases.push(json!({ "kind": "exit_code", "status": status }));
    }

    // The operating-system reason of every error code the jail can produce.
    cases.push(json!({
        "kind": "os_strerror",
        "codes": [1, 2, 5, 9, 13, 16, 17, 20, 21, 22, 28, 30, 95],
    }));

    // The deterministic corpus of valid and hostile cases.
    let mut rng = Rng::new(0x5A17_C0DE_1234_5678);
    let root_paths = [
        "/tmp/kala-root",
        "/tmp/kala-root/",
        "//tmp//kala-root",
        "/tmp/kala-root/../kala-root",
        "relative/root",
        "",
        "/tmp/ünïcode root",
        "/tmp/with space",
    ];
    let workspace_paths = [
        "/tmp/kala-workspace",
        "/tmp/kala-workspace/",
        "relative/workspace",
        "",
        "/tmp/ünïcode workspace",
    ];
    for _ in 0..48 {
        let mut chosen: Vec<&str> = Vec::new();
        for control in ALL {
            if rng.next() % 3 != 0 {
                chosen.push(control);
            }
        }
        let root = *rng.pick(&root_paths);
        let workspace = *rng.pick(&workspace_paths);
        let scratch_bytes = match rng.next() % 4 {
            0 => 0,
            1 => 1 << 20,
            2 => -(rng.next() as i64 % 1_000_000),
            _ => (rng.next() % 1_000_000_000) as i64,
        };
        cases.push(json!({
            "kind": "build_spec",
            "controls": chosen,
            "root": root,
            "workspace": workspace,
            "import_roots": [],
            "scratch_bytes": scratch_bytes,
        }));
    }
    for _ in 0..48 {
        let mut payload = valid_spec.clone();
        let mutation = rng.next() % 6;
        let object = payload.as_object_mut().expect("object");
        match mutation {
            0 => {
                object.remove("scratch_bytes");
            }
            1 => {
                payload["workspace"] = json!(42);
            }
            2 => {
                payload["device_bindings"] = json!(["/dev/null", "/dev/zero"]);
            }
            3 => {
                payload["readonly_bindings"] = json!(null);
            }
            4 => {
                payload["root"] = json!(format!("/tmp/{}", "x".repeat(4096)));
            }
            _ => {
                payload["scratch_bytes"] = json!(-0.5);
            }
        }
        cases.push(json!({ "kind": "spec_from_payload", "payload": payload }));
    }
    for _ in 0..24 {
        let elapsed = (rng.next() % 100_000) as f64 / 1_000.0;
        cases.push(json!({
            "kind": "probe_payload",
            "probe": {
                "available": rng.next() % 2 == 0,
                "platform": "linux-x86_64",
                "controls": ["mount_namespace"],
                "detail": "d".repeat((rng.next() % 600) as usize),
                "elapsed_ms": elapsed,
            },
        }));
    }

    // The probe entry point, valid and malformed.
    let probe_spec = json!({
        "controls": ALL,
        "device_bindings": [],
        "readonly_bindings": [],
        "root": "/tmp/kala-root",
        "scratch_bytes": 1 << 20,
        "scratch_paths": ["/tmp"],
        "writable_bindings": [],
        "workspace": "/tmp/kala-workspace",
    });
    cases.push(json!({
        "kind": "probe_entry",
        "spec_payload": probe_spec.to_string(),
        "canary_payload": json!("/host/canary").to_string(),
    }));
    cases.push(json!({
        "kind": "probe_entry",
        "spec_payload": "{not json",
        "canary_payload": json!("/host/canary").to_string(),
    }));
    cases.push(json!({
        "kind": "probe_entry",
        "spec_payload": probe_spec.to_string(),
        "canary_payload": "{not json",
    }));

    // The probe entry point as a process: the exit code and the output line.
    cases.push(json!({
        "kind": "run_probe",
        "spec_payload": probe_spec.to_string(),
        "canary_payload": json!("/host/canary").to_string(),
    }));
    cases.push(json!({
        "kind": "run_probe",
        "spec_payload": "{bad",
        "canary_payload": "nope",
    }));

    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    for case in &cases {
        *counts
            .entry(case["kind"].as_str().expect("kind").to_owned())
            .or_default() += 1;
    }
    eprintln!("the differential corpus holds {} cases", cases.len());
    for (kind, count) in &counts {
        eprintln!("  {kind}: {count}");
    }
    assert!(cases.len() >= 120, "the corpus stays meaningful");
    let oracle = run_oracle(&cases);
    assert_eq!(oracle.len(), cases.len(), "every case is answered");
    for (index, case) in cases.iter().enumerate() {
        let expected = rust_answer(case, &oracle);
        assert_eq!(
            oracle[index], expected,
            "case {index} ({}) differs from the live Python",
            case["kind"]
        );
    }
    let _ = workspace;
}

/// The documented differences between the port and the Python source.
///
/// Every entry names a hostile payload that CPython accepts because
/// `SandboxSpec.from_payload` coerces the fields with `frozenset()` and
/// `tuple()`, which accept any iterable of any hashable value. The Rust port
/// requires a JSON array of strings and refuses the payload at the boundary.
#[test]
fn the_documented_differences_are_the_only_accepted_hostile_payloads() {
    let base = json!({
        "controls": ALL,
        "device_bindings": [],
        "readonly_bindings": ["/usr"],
        "root": "/tmp/kala-root",
        "scratch_bytes": 1 << 20,
        "scratch_paths": ["/dev/shm", "/tmp"],
        "writable_bindings": ["/tmp/kala-workspace"],
        "workspace": "/tmp/kala-workspace",
    });
    let mut cases: Vec<Value> = Vec::new();
    let mut shapes: Vec<(&str, Value)> = Vec::new();

    let mut payload = base.clone();
    payload["controls"] = json!("abc");
    shapes.push(("a bare string for the control set", payload.clone()));
    cases.push(json!({ "kind": "spec_from_payload", "payload": payload }));

    let mut payload = base.clone();
    payload["controls"] = json!([1, 2]);
    shapes.push(("numbers in the control set", payload.clone()));
    cases.push(json!({ "kind": "spec_from_payload", "payload": payload }));

    let mut payload = base.clone();
    payload["readonly_bindings"] = json!(["/usr", 7]);
    shapes.push(("a number in a binding list", payload.clone()));
    cases.push(json!({ "kind": "spec_from_payload", "payload": payload }));

    let mut payload = base.clone();
    payload["scratch_paths"] = json!("/tmp");
    shapes.push(("a bare string for a binding list", payload.clone()));
    cases.push(json!({ "kind": "spec_from_payload", "payload": payload }));

    let mut payload = base.clone();
    payload["device_bindings"] = json!({"/dev/null": true});
    shapes.push(("an object for a binding list", payload.clone()));
    cases.push(json!({ "kind": "spec_from_payload", "payload": payload }));

    let oracle = run_oracle(&cases);
    for (index, (label, payload)) in shapes.iter().enumerate() {
        let python = &oracle[index];
        assert_eq!(
            python["ok"],
            json!(true),
            "CPython accepts {label}; the difference must stay documented"
        );
        let rust = match SandboxSpec::from_payload(payload) {
            Ok(_) => panic!("the port must refuse {label}"),
            Err(error) => failed("SandboxSetupError", error.detail()),
        };
        assert_eq!(
            rust["message"],
            json!("the sandbox specification is malformed"),
            "the port refuses {label} with the malformed-payload error"
        );
    }
}

/// The jail that the Python source builds and the jail this crate builds are
/// the same jail: the same calls, in the same order, with the same flags.
#[test]
fn live_python_and_the_port_build_the_same_jail() {
    let scratch = tempfile::Builder::new()
        .prefix("kala-jail-")
        .tempdir()
        .expect("a temporary directory");
    let root = scratch.path().join("root");
    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&root).expect("the jail root");
    std::fs::create_dir_all(&workspace).expect("the workspace");
    let spec = json!({
        "controls": REQUIRED,
        "device_bindings": [],
        "readonly_bindings": ["/usr", "/bin", "/lib"],
        "root": root.to_string_lossy(),
        "scratch_bytes": 1 << 20,
        "scratch_paths": ["/dev/shm", "/tmp"],
        "writable_bindings": [workspace.to_string_lossy()],
        "workspace": workspace.to_string_lossy(),
    });
    let cases = vec![json!({ "kind": "jail_plan", "payload": spec })];
    let oracle = run_oracle(&cases);
    let python = &oracle[0];
    assert_eq!(python["ok"], json!(true), "the python plan: {python}");
    let python_calls = python["value"]["calls"].clone();

    let helper = PathBuf::from(env!("CARGO_BIN_EXE_kala-sandbox-inspect"));
    let output = Command::new(helper)
        .arg(spec.to_string())
        .arg(json!("/host/canary").to_string())
        .output()
        .expect("the inspector runs");
    assert!(
        output.status.success(),
        "the inspector failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inspected: Value =
        serde_json::from_slice(&output.stdout).expect("the inspector emits JSON");
    assert!(
        inspected.get("error").is_none(),
        "the native jail could not be built: {inspected}"
    );
    assert_eq!(
        inspected["calls"], python_calls,
        "the jail plan differs from the live Python"
    );
    assert_eq!(
        inspected["established"], python["value"]["established"],
        "the established controls differ from the live Python"
    );
}

/// The strict profile is probed by both implementations on this host and the
/// answer must be the same.
#[test]
fn live_python_and_the_port_probe_the_same_host() {
    let cases = vec![json!({
        "kind": "probe_sandbox",
        "import_roots": [],
        "scratch_bytes": 1 << 20,
    })];
    let oracle = run_oracle(&cases);
    let python = &oracle[0]["value"];

    let helper = PathBuf::from(env!("CARGO_BIN_EXE_kala-sandbox-probe"));
    let options = kala_sandbox::ProbeOptions::new(helper);
    let probe = kala_sandbox::probe_sandbox(&options);
    if python["available"] != json!(true) {
        // The host cannot establish the strict profile. Both implementations
        // must refuse it, and neither may advertise a control.
        assert!(!probe.available, "the native probe claimed a control");
        assert!(!probe.detail.is_empty(), "the refusal carries a reason");
        return;
    }
    assert!(probe.available, "the native probe failed: {probe:?}");
    assert_eq!(probe.platform, python["platform"]);
    let mut native = probe.controls.clone();
    native.sort();
    let mut expected: Vec<String> = python["controls"]
        .as_array()
        .expect("controls")
        .iter()
        .map(|item| item.as_str().expect("control").to_owned())
        .collect();
    expected.sort();
    assert_eq!(
        native, expected,
        "the verified controls differ from the live Python"
    );
}

/// The jail that carries a PID namespace is the same jail in both
/// implementations.
///
/// The Python implementation and this crate build the jail in a forked child
/// and report what the child sees: the controls that were established, the
/// controls that hold, the entries of the jail root and whether the child is
/// PID 1 of its namespace. The two fingerprints must be identical.
#[test]
fn live_python_and_the_port_build_the_same_pid_namespace_jail() {
    let scratch = tempfile::Builder::new()
        .prefix("kala-pid-jail-")
        .tempdir()
        .expect("a temporary directory");
    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("the workspace");
    let canary = scratch.path().join("canary");
    std::fs::write(&canary, b"kala-sandbox-canary").expect("the canary");

    // The same specification for both sides: the same bindings, the same
    // controls and the same workspace.
    let spec = json!({
        "controls": ALL,
        "device_bindings": ["/dev/null"],
        "readonly_bindings": ["/usr", "/bin", "/lib"],
        "root": scratch.path().join("root").to_string_lossy(),
        "scratch_bytes": 1 << 20,
        "scratch_paths": ["/dev/shm", "/tmp"],
        "writable_bindings": [workspace.to_string_lossy()],
        "workspace": workspace.to_string_lossy(),
    });
    let cases = vec![json!({
        "kind": "jail_fingerprint",
        "payload": spec,
        "canary": canary.to_string_lossy(),
    })];
    let oracle = run_oracle(&cases);
    let python = &oracle[0]["value"]["fingerprint"];
    assert!(
        python.get("error").is_none(),
        "the python jail could not be built: {python}"
    );

    let helper = PathBuf::from(env!("CARGO_BIN_EXE_kala-sandbox-inspect"));
    let output = Command::new(helper)
        .arg(spec.to_string())
        .arg(json!(canary.to_string_lossy()).to_string())
        .output()
        .expect("the inspector runs");
    assert!(
        output.status.success(),
        "the inspector failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let native: Value = serde_json::from_slice(&output.stdout).expect("the inspector emits JSON");
    assert!(
        native.get("error").is_none(),
        "the native jail could not be built: {native}"
    );
    assert_eq!(
        native["established"], python["established"],
        "the established controls differ from the live Python"
    );
    assert_eq!(
        native["verified"], python["verified"],
        "the verified controls differ from the live Python"
    );
    assert_eq!(
        native["failed"], python["failed"],
        "the failed controls differ from the live Python"
    );
    assert_eq!(
        native["root_entries"], python["root_entries"],
        "the jail root differs from the live Python"
    );
    assert_eq!(
        native["pid_is_one"],
        json!(true),
        "the native child is PID 1"
    );
    assert_eq!(
        python["pid_is_one"],
        json!(true),
        "the python child is PID 1"
    );
}
