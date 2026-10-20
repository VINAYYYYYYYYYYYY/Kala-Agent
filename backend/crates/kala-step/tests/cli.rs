//! End-to-end tests for the `kala-step` CLI.
//!
//! The CLI is the operator/debug entry point and the driver for future service
//! and benchmark validation, so it is exercised as a real process: argument
//! handling, the overwrite guard, stdout/file output, invalid input exit codes
//! and the exact bytes it writes.

use std::process::{Command, Output};

mod common;

use common::TempDir;

const BINARY: &str = env!("CARGO_BIN_EXE_kala-step");

fn run(arguments: &[&str]) -> Output {
    Command::new(BINARY)
        .args(arguments)
        .output()
        .expect("the CLI binary runs")
}

#[test]
fn capability_reports_the_native_exporter() {
    let output = run(&["capability"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        "{\"available\":true,\"exporter\":\"kala-step native analytic B-rep writer\"}"
    );
}

#[test]
fn export_writes_an_iso_document_to_a_file() {
    let directory = TempDir::new("kala-step-cli");
    let path = directory.join("cli-box.step");
    let output = run(&[
        "export",
        "--kind",
        "box",
        "--size-x",
        "20",
        "--size-y",
        "30",
        "--size-z",
        "40",
        "--out",
        path.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "{output:?}");
    let bytes = std::fs::read(&path).expect("the CLI wrote the file");
    assert!(bytes.starts_with(b"ISO-10303-21;"));
    let document = String::from_utf8(bytes).unwrap();
    assert!(document.contains("CARTESIAN_POINT('',(-10.,-15.,-20.))"));
    let summary = String::from_utf8(output.stderr).unwrap();
    assert!(summary.contains("box exact B-rep"), "{summary}");
}

#[test]
fn export_prints_to_stdout_by_default() {
    let output = run(&["export", "--kind", "sphere", "--radius", "2.5"]);
    assert!(output.status.success(), "{output:?}");
    let document = String::from_utf8(output.stdout).unwrap();
    assert!(document.starts_with("ISO-10303-21;\n"));
    assert!(document.contains("SPHERICAL_SURFACE('',#"));
    assert!(document.contains("CARTESIAN_POINT('',(0.,0.,-2.5))"));
}

#[test]
fn export_refuses_to_overwrite_without_force() {
    let directory = TempDir::new("kala-step-cli");
    let path = directory.join("cli-guard.step");
    std::fs::write(&path, b"user data").unwrap();

    let refused = run(&["export", "--kind", "box", "--out", path.to_str().unwrap()]);
    assert!(!refused.status.success());
    let stderr = String::from_utf8(refused.stderr).unwrap();
    assert!(stderr.contains("refusing to overwrite"), "{stderr}");
    assert_eq!(std::fs::read(&path).unwrap(), b"user data");

    let forced = run(&[
        "export",
        "--kind",
        "box",
        "--force",
        "--out",
        path.to_str().unwrap(),
    ]);
    assert!(forced.status.success(), "{forced:?}");
    assert!(std::fs::read(&path).unwrap().starts_with(b"ISO-10303-21;"));
}

#[test]
fn invalid_inputs_fail_with_the_python_message_and_write_nothing() {
    let directory = TempDir::new("kala-step-cli");
    let path = directory.join("cli-invalid.step");

    for (arguments, expected) in [
        (
            vec!["export", "--kind", "box", "--size-x", "0.09"],
            "size_x must be between 0.1 and 2000.0 millimetres",
        ),
        (
            vec!["export", "--kind", "cylinder", "--radius", "inf"],
            "radius must be a finite number",
        ),
        (vec!["export", "--kind", "torus"], "unsupported kind: torus"),
        (
            vec!["export", "--kind", "box", "--radius", "5"],
            "--radius/--height only apply to cylinder and sphere",
        ),
        (vec!["export", "--kind"], "missing value for --kind"),
        (
            vec!["export", "--repeat", "0"],
            "--repeat must be at least 1",
        ),
        (vec!["explode"], "unknown command: explode"),
    ] {
        let output = run(&arguments);
        assert!(!output.status.success(), "{arguments:?} should fail");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains(expected),
            "{arguments:?}: stderr {stderr:?} does not mention {expected:?}"
        );
        assert!(!path.exists(), "{arguments:?} wrote a file");
    }
}

#[test]
fn repeat_produces_identical_bytes_and_a_timing_summary() {
    let output = run(&["export", "--kind", "cylinder", "--repeat", "25"]);
    assert!(output.status.success(), "{output:?}");
    let single = run(&["export", "--kind", "cylinder"]);
    assert_eq!(output.stdout, single.stdout);
    let summary = String::from_utf8(output.stderr).unwrap();
    assert!(summary.contains("25 export(s)"), "{summary}");
}

#[test]
fn help_text_documents_the_flags() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    let usage = String::from_utf8(output.stdout).unwrap();
    for flag in [
        "--kind", "--size-x", "--radius", "--height", "--out", "--force", "--repeat",
    ] {
        assert!(usage.contains(flag), "usage must mention {flag}");
    }
}
