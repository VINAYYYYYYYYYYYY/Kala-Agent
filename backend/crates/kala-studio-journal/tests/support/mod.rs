//! Shared integration-test helpers.
//!
//! Every helper here is test-only. Python is spawned only by the differential
//! tests; the library itself never calls Python.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::Value;

/// The repository root, three levels above the crate manifest.
pub fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// The Python interpreter used as the oracle.
pub fn python_executable() -> PathBuf {
    if let Ok(path) = std::env::var("KALA_PYTHON") {
        return PathBuf::from(path);
    }
    let venv = repository_root().join(".venv/bin/python");
    if venv.is_file() {
        return venv;
    }
    let python3 = Path::new("python3");
    if Command::new(python3)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        return python3.to_path_buf();
    }
    panic!("no Python oracle available; set KALA_PYTHON to the repository virtual environment")
}

/// The path of the oracle script.
pub fn python_oracle_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/python_oracle.py")
}

/// A live Python oracle process that keeps the journal open.
///
/// The process speaks JSON lines: one request per line, one response per
/// line. The journal stays open between requests, so the oracle exercises the
/// same cached-head behaviour the native journal does.
pub struct PythonOracle {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl PythonOracle {
    /// Spawn the oracle process.
    pub fn spawn() -> Self {
        let mut child = Command::new(python_executable())
            .arg(python_oracle_script())
            .env("PYTHONPATH", repository_root().join("src"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the Python oracle");
        let stdin = child.stdin.take().expect("oracle stdin");
        let stdout = BufReader::new(child.stdout.take().expect("oracle stdout"));
        Self {
            child,
            stdin,
            stdout,
        }
    }

    /// Send one request and return the response.
    pub fn request(&mut self, request: Value) -> Value {
        let line = serde_json::to_string(&request).expect("serialize the request");
        self.stdin
            .write_all(line.as_bytes())
            .expect("write the request");
        self.stdin.write_all(b"\n").expect("write the newline");
        self.stdin.flush().expect("flush the request");
        let mut response = String::new();
        self.stdout
            .read_line(&mut response)
            .expect("read the response");
        assert!(
            !response.is_empty(),
            "the Python oracle closed its output; the request was {line}"
        );
        serde_json::from_str(&response)
            .unwrap_or_else(|error| panic!("oracle JSON: {error}; response={response}"))
    }

    /// Send one request that must succeed and return its result.
    pub fn ok(&mut self, request: Value) -> Value {
        let label = request.to_string();
        let response = self.request(request);
        assert_eq!(
            response["ok"],
            Value::Bool(true),
            "the oracle refused the request {label}: {response}"
        );
        response["result"].clone()
    }

    /// Send one request that must fail and return its error.
    pub fn err(&mut self, request: Value) -> Value {
        let label = request.to_string();
        let response = self.request(request);
        assert_eq!(
            response["ok"],
            Value::Bool(false),
            "the oracle accepted the request {label}: {response}"
        );
        response["error"].clone()
    }
}

impl Drop for PythonOracle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Base64-encode bytes without a dependency.
pub fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let first = chunk[0] as u32;
        let second = chunk.get(1).copied().unwrap_or(0) as u32;
        let third = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (first << 16) | (second << 8) | third;
        out.push(ALPHABET[((triple >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Decode base64 text without a dependency.
pub fn base64_decode(text: &str) -> Vec<u8> {
    let mut buffer: u32 = 0;
    let mut bits = 0_u32;
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    for character in text.bytes() {
        let value = match character {
            b'A'..=b'Z' => u32::from(character - b'A'),
            b'a'..=b'z' => u32::from(character - b'a') + 26,
            b'0'..=b'9' => u32::from(character - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            other => panic!("invalid base64 character {other}"),
        };
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    out
}
