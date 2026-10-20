//! Deep canonical-JSON compatibility against the live Python contract.
//!
//! `kala_agent.contracts.models.CanonicalJsonObject` has no fixed nesting
//! limit; the previous native implementation stopped at 1,024 levels. These
//! tests compare canonical bytes, lengths and SHA-256 digests with the live
//! Python implementation at depths 1,500/5,000/10,000/20,000 and exercise the
//! native parse/clone/equality/debug/drop path on a 2 MiB stack. A final
//! probe records the host-specific CPython recursion boundary so a native
//! domain wider than Python's incidental runtime limit is reported, not
//! mistaken for identical accepted sets.

mod support;

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use kala_json::CanonicalJsonError;
use kala_persistence::{
    CanonicalValue, canonicalize_json_object, canonicalize_object_value, parse_json_object,
    persistence_limits, sha256_text,
};
use serde_json::Value;
use support::{python_executable, repository_root};

const LEAF: &str = r#"{"big":1180591620717411303424,"f":-0.0,"n":null,"u":"\u03ba \ud83d\ude00"}"#;

fn deep_oracle_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/deep_json_oracle.py")
}

fn run_deep_oracle(args: &[&str], stdin: &[u8]) -> Vec<u8> {
    let mut child = Command::new(python_executable())
        .arg(deep_oracle_script())
        .args(args)
        .env("PYTHONPATH", repository_root().join("src"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn deep JSON oracle");
    child
        .stdin
        .as_mut()
        .expect("oracle stdin")
        .write_all(stdin)
        .expect("write oracle input");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait for oracle");
    assert!(
        output.status.success(),
        "deep JSON oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn run_deep_oracle_json(args: &[&str], stdin: &[u8]) -> Value {
    serde_json::from_slice(&run_deep_oracle(args, stdin))
        .expect("deep JSON oracle returned invalid JSON")
}

fn on_two_mib_stack<F, T>(operation: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(operation)
        .expect("spawn 2 MiB thread")
        .join()
        .expect("deep operation must not overflow a 2 MiB stack")
}

/// Alternating arrays/objects around a mixed leaf; mirrors the oracle builder.
fn deep_source(depth: usize, tag: &str) -> String {
    let tag = serde_json::to_string(tag).expect("serialize tag");
    let mut source = format!(r#"{{"tag":{tag},"x":"#);
    let mut closers: Vec<char> = Vec::with_capacity(depth);
    for level in 0..depth {
        if level % 2 == 0 {
            source.push('[');
            closers.push(']');
        } else {
            source.push_str(r#"{"k":"#);
            closers.push('}');
        }
    }
    source.push_str(LEAF);
    for closer in closers.into_iter().rev() {
        source.push(closer);
    }
    source.push('}');
    source
}

fn array_chain_source(depth: usize) -> String {
    let mut source = String::from(r#"{"x":"#);
    source.push_str(&"[".repeat(depth));
    source.push('0');
    source.push_str(&"]".repeat(depth));
    source.push('}');
    source
}

fn distinct_key_object_source(depth: usize) -> String {
    let mut source = String::from(r#"{"x":"#);
    for level in 0..depth {
        source.push_str(&format!(r#"{{"k{level}":"#));
    }
    source.push('0');
    for _ in 0..depth {
        source.push('}');
    }
    source.push('}');
    source
}

fn assert_native_matches(label: &str, source: &str, result: &Value, expected_depth: usize) {
    let label = label.to_owned();
    assert!(
        result["ok"].as_bool().expect("oracle ok flag"),
        "live Python rejected {label}: {}",
        result["error"].as_str().unwrap_or("<no error>")
    );
    let expected = result["output"].as_str().expect("oracle output");
    let expected_digest = result["sha256"].as_str().expect("oracle digest");
    let expected_length = result["length"].as_u64().expect("oracle length") as usize;

    let rust = on_two_mib_stack({
        let source = source.to_owned();
        let label = label.clone();
        move || {
            canonicalize_json_object(&source)
                .unwrap_or_else(|error| panic!("Rust rejected Python-accepted {label}: {error}"))
        }
    });
    assert_eq!(rust, expected, "canonical bytes for {label}");
    assert_eq!(rust.len(), expected_length, "length for {label}");
    assert_eq!(
        sha256_text(&rust).to_hex(),
        expected_digest,
        "digest for {label}"
    );

    // Every value-lifecycle operation must also survive the ordinary thread
    // stack while the value is this deep.
    let owned = rust.clone();
    on_two_mib_stack({
        let label = label.clone();
        move || {
            let value = parse_json_object(&owned)
                .unwrap_or_else(|error| panic!("reparse {label}: {error}"));
            assert!(
                matches!(value, CanonicalValue::Object(_)),
                "top level for {label}"
            );
            let cloned = value.clone();
            assert_eq!(value, cloned, "clone equality for {label}");
            let reserialized = canonicalize_object_value(&value)
                .unwrap_or_else(|error| panic!("reserialize {label}: {error}"));
            assert_eq!(reserialized, owned, "roundtrip bytes for {label}");
            let debug = format!("{value:?}");
            assert!(!debug.is_empty(), "debug output for {label}");
            drop(value);
            drop(cloned);
            drop(debug);
        }
    });

    // The same source must still be refused by the bounded tool domain once
    // it exceeds 48 levels.
    assert!(
        expected_depth > 48,
        "case {label} is meant to exceed the tool bound"
    );
    let tool_result = on_two_mib_stack({
        let source = source.to_owned();
        move || kala_json::parse_strict(source.as_bytes(), &kala_json::Limits::default())
    });
    assert_eq!(
        tool_result,
        Err(CanonicalJsonError::DepthExceeded),
        "tool-depth refusal for {label}"
    );
}

#[test]
fn deep_canonical_bytes_match_live_python() {
    let cases = [
        ("mixed-1500", deep_source(1_500, "compat-1500"), 1_500usize),
        ("arrays-5000", array_chain_source(5_000), 5_000),
        ("mixed-5000", deep_source(5_000, "compat-5000"), 5_000),
        ("objects-1500", distinct_key_object_source(1_500), 1_500),
        ("mixed-10000", deep_source(10_000, "compat-10000"), 10_000),
        ("mixed-20000", deep_source(20_000, "compat-20000"), 20_000),
    ];
    let sources: Vec<&str> = cases.iter().map(|(_, source, _)| source.as_str()).collect();
    let request = serde_json::to_vec(&sources).expect("serialize oracle request");
    let results = run_deep_oracle_json(&["canonical"], &request);
    let results = results.as_array().expect("oracle result array");
    assert_eq!(results.len(), cases.len());

    for ((label, source, depth), result) in cases.iter().zip(results) {
        assert_native_matches(label, source, result, *depth);
    }
}

#[test]
fn persistence_depth_is_unbounded_while_tools_stay_at_48() {
    let limits = persistence_limits();
    assert_eq!(limits.max_json_depth, usize::MAX);
    assert!(limits.max_json_depth > kala_json::Limits::DEFAULT_MAX_JSON_DEPTH);

    let source = deep_source(20_000, "unbounded");
    assert!(
        on_two_mib_stack({
            let source = source.clone();
            move || canonicalize_json_object(&source)
        })
        .is_ok()
    );
    assert_eq!(
        kala_json::parse_strict(source.as_bytes(), &kala_json::Limits::default()),
        Err(CanonicalJsonError::DepthExceeded)
    );
}

#[test]
fn reports_native_domain_beyond_live_python_runtime_limit() {
    // CPython's JSON decoder/encoder recursion boundary is process-dependent
    // (its C stack), not a product contract limit. Probe a few depths; for
    // every depth Python accepts, compare bytes. For depths where this host's
    // Python raises its incidental recursion failure, verify the native
    // well-formed document still round-trips and report the divergence.
    let depths = [25_000usize, 40_000, 60_000, 100_000];
    let sources: Vec<String> = depths
        .iter()
        .map(|depth| array_chain_source(*depth))
        .collect();
    let source_refs: Vec<&str> = sources.iter().map(String::as_str).collect();
    let request = serde_json::to_vec(&source_refs).expect("serialize probe request");
    let results = run_deep_oracle_json(&["canonical"], &request);
    let results = results.as_array().expect("probe result array");
    assert_eq!(results.len(), depths.len());

    let mut python_accepted = Vec::new();
    let mut python_rejected = Vec::new();
    for ((depth, source), result) in depths.iter().zip(sources.iter()).zip(results) {
        let python_ok = result["ok"].as_bool().expect("probe ok flag");
        let depth_value = *depth;
        let rust = on_two_mib_stack({
            let source = source.clone();
            move || {
                canonicalize_json_object(&source).unwrap_or_else(|error| {
                    panic!("native rejected well-formed depth {depth_value}: {error}")
                })
            }
        });
        assert_eq!(rust, *source, "native bytes at depth {depth}");
        if python_ok {
            assert_eq!(
                rust,
                result["output"].as_str().expect("probe output"),
                "bytes at Python-accepted depth {depth}"
            );
            python_accepted.push(*depth);
        } else {
            python_rejected.push((
                *depth,
                result["error"].as_str().unwrap_or("<no error>").to_owned(),
            ));
        }
    }

    println!(
        "live Python CanonicalJsonObject accepted depths {python_accepted:?}; \
         incidental recursion failures {python_rejected:?}. Native accepted and \
         round-tripped every probed depth (25k, 40k, 60k, 100k); where Python's \
         host-specific C recursion limit rejects a well-formed document, the \
         accepted sets differ by design."
    );
}

#[test]
fn deep_clone_and_equality_can_cross_thread_boundaries() {
    let source = deep_source(10_000, "thread");
    let canonical = on_two_mib_stack({
        let source = source.clone();
        move || canonicalize_json_object(&source).expect("canonical")
    });
    let value = on_two_mib_stack({
        let canonical = canonical.clone();
        move || parse_json_object(&canonical).expect("parse")
    });
    let moved = value.clone();
    let handle = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let clone = moved.clone();
            assert_eq!(moved, clone);
            drop(moved);
            drop(clone);
        })
        .expect("spawn thread");
    handle.join().expect("deep value thread");
    on_two_mib_stack(move || drop(value));
    let reencoded = on_two_mib_stack({
        let canonical = canonical.clone();
        move || {
            canonicalize_object_value(&parse_json_object(&canonical).expect("reparse"))
                .expect("reserialize")
        }
    });
    assert_eq!(reencoded, canonical);
}
