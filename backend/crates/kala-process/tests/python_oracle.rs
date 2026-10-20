//! Test-only differential against the live Python source.
//!
//! The Rust crate never invokes Python at product time. This test uses the
//! legacy implementation as an oracle so payload shapes, environment rules,
//! path confinement diagnostics, sanitisation and judge behaviour are checked
//! against actual source rather than against a hand-written expectation.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kala_process::{
    ChildEnvironment, IsolationProfile, SandboxProbe, SandboxSpec, build_child_environment,
    judge_sandbox, resolve_workspace_path, sanitize_child_stderr,
};
use serde_json::{Value, json};
use tempfile::TempDir;

#[test]
fn live_python_source_matches_the_native_contract() {
    let repository = repository_root();
    let python = if repository.join(".venv/bin/python").is_file() {
        repository.join(".venv/bin/python")
    } else {
        PathBuf::from("python3")
    };

    let (guard, workspace) = workspace();
    std::fs::create_dir(workspace.join("real")).expect("real directory");
    std::fs::write(workspace.join("real/nested.txt"), b"value").expect("nested");
    #[cfg(unix)]
    std::os::unix::fs::symlink(workspace.join("real"), workspace.join("link-inside"))
        .expect("symlink");

    let spec = SandboxSpec {
        controls: kala_process::STRICT_REQUIRED_CONTROLS
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<BTreeSet<_>>(),
        root: workspace.join("root"),
        workspace: workspace.clone(),
        readonly_bindings: vec!["/usr".to_owned()],
        writable_bindings: vec![workspace.to_string_lossy().into_owned()],
        device_bindings: vec!["/dev/null".to_owned()],
        scratch_paths: vec!["/dev/shm".to_owned(), "/tmp".to_owned()],
        scratch_bytes: 1 << 20,
    };
    let probes = [
        SandboxProbe {
            available: false,
            platform: "linux-x86_64".to_owned(),
            controls: Vec::new(),
            detail: "denied".to_owned(),
            elapsed_ms: 1.23456,
        },
        SandboxProbe {
            available: true,
            platform: "linux-x86_64".to_owned(),
            controls: vec!["mount_namespace".to_owned(), "user_namespace".to_owned()],
            detail: String::new(),
            elapsed_ms: 0.0,
        },
    ];
    let probe_payloads: Vec<Value> = probes.iter().map(SandboxProbe::to_payload).collect();
    let judgements = [
        (IsolationProfile::Portable, None),
        (IsolationProfile::StrictLinux, None),
        (IsolationProfile::StrictLinux, Some(&probes[0])),
        (IsolationProfile::StrictLinux, Some(&probes[1])),
    ];
    let request_judgements: Vec<Value> = judgements
        .iter()
        .map(|(profile, probe)| {
            json!({
                "profile": profile.as_str(),
                "probe": probe.map(SandboxProbe::to_payload),
            })
        })
        .collect();
    let sanitize_payloads: Vec<Vec<u8>> = vec![
        b"ImportError: no module named x at WS/mod.py\n".to_vec(),
        b"\x00\x01binary\xff".to_vec(),
        b"first\nsecond\nthird\nfourth\nfifth".to_vec(),
        b"".to_vec(),
    ];
    let workspace_text = workspace.to_string_lossy().into_owned();
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut request = json!({
        "workspace": workspace_text,
        "import_roots": [repository.join("src").to_string_lossy()],
        "probes": probe_payloads,
        "spec": spec.to_payload(),
        "judgements": request_judgements,
        "sanitize": sanitize_payloads.iter().map(|payload| {
            json!({
                "payload": base64(payload),
                "replacement": workspace_text,
            })
        }).collect::<Vec<_>>(),
        "workspace_paths": [
            "real/nested.txt",
            "missing.txt",
            "../outside.txt",
            "/etc/passwd",
            "real/../../outside.txt",
            "link-inside",
            "back\\slash.txt",
            "..",
            ".",
            "",
            "a".repeat(2_048),
            "a".repeat(100),
            vec!["a"; 40].join("/"),
        ],
    });
    #[cfg(unix)]
    {
        request["signal_fixture"] = json!(env!("CARGO_BIN_EXE_kala-process-fixture"));
    }
    let script = repository.join("backend/crates/kala-process/tests/python_oracle.py");
    let output = Command::new(python)
        .arg(script)
        .current_dir(&repository)
        .env("PYTHONPATH", repository.join("src"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("python oracle starts");
    let mut child = output;
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(request.to_string().as_bytes())
        .expect("request written");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("python oracle completes");
    assert!(
        output.status.success(),
        "python oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let oracle: Value = serde_json::from_slice(&output.stdout).expect("oracle emits JSON");

    assert_eq!(oracle["platform"], kala_process::platform_name());
    assert_eq!(oracle["probes"], Value::Array(probe_payloads.clone()));
    assert_eq!(oracle["spec_payload"], spec.to_payload());
    let expected_judgements: Vec<Value> = judgements
        .iter()
        .map(|(profile, probe)| judge_sandbox(*profile, *probe).to_payload())
        .collect();
    assert_eq!(oracle["judgements"], Value::Array(expected_judgements));

    let our_environment =
        build_child_environment(&workspace, &[repository.join("src")], &[]).expect("env");
    assert_eq!(
        oracle["environment"],
        serde_json::to_value(environment_map(&our_environment)).expect("environment JSON")
    );
    let sanitized: Vec<String> = sanitize_payloads
        .iter()
        .map(|payload| sanitize_child_stderr(payload, Some(&workspace_text)))
        .collect();
    assert_eq!(
        oracle["sanitized"],
        serde_json::to_value(sanitized).expect("sanitized JSON")
    );

    #[cfg(unix)]
    {
        // A real signal death: Python's `Popen.returncode` is `-9`, while the
        // conventional shell status would be 137. The compatibility message
        // must use the Python value.
        assert_eq!(oracle["signal_returncode"], json!(-9));
        assert_eq!(
            kala_process::ExitStatus::signalled(9).python_exit_code(),
            oracle["signal_returncode"].as_i64().expect("return code")
        );
    }

    let long_path = "a".repeat(2_048);
    let wide_segment = "a".repeat(100);
    let many_segments = vec!["a"; 40].join("/");
    let paths = [
        "real/nested.txt",
        "missing.txt",
        "../outside.txt",
        "/etc/passwd",
        "real/../../outside.txt",
        "link-inside",
        "back\\slash.txt",
        "..",
        ".",
        "",
        long_path.as_str(),
        wide_segment.as_str(),
        many_segments.as_str(),
    ];
    let oracle_paths = oracle["workspace_paths"].as_array().expect("array");
    assert_eq!(oracle_paths.len(), paths.len());
    for (candidate, expected) in paths.iter().zip(oracle_paths) {
        let our = resolve_workspace_path(&workspace, candidate);
        if expected["ok"] == Value::Bool(false) {
            assert!(
                our.is_err(),
                "Python refused {candidate:?}, Rust accepted it"
            );
        } else {
            let path = our.expect("Rust accepts the same valid path");
            assert_eq!(
                path.to_string_lossy().replace('\\', "/"),
                expected["path"].as_str().expect("path string")
            );
        }
    }
    drop(guard);
}

fn environment_map(environment: &ChildEnvironment) -> std::collections::BTreeMap<&str, &str> {
    environment
        .entries()
        .iter()
        .map(|entry| (entry.key.as_str(), entry.value.as_str()))
        .collect()
}

fn base64(payload: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::new();
    for chunk in payload.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(TABLE[(first >> 2) as usize] as char);
        output.push(TABLE[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            output.push(TABLE[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(TABLE[(third & 0x3f) as usize] as char);
        } else {
            output.push('=');
        }
    }
    output
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repository root")
}

fn workspace() -> (TempDir, PathBuf) {
    let directory = tempfile::Builder::new()
        .prefix("kala-oracle-")
        .tempdir()
        .expect("temporary directory");
    let path = std::fs::canonicalize(directory.path()).expect("canonical workspace");
    (directory, path)
}
