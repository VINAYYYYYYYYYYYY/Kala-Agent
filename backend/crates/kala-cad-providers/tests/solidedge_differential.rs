//! Live differential against the repository's Python Solid Edge provider.
//!
//! The oracle runs the repository's own `SolidEdgeProvider` with the same
//! constructor arguments and the same token files, against an in-process mock
//! bridge for the health cases. The test compares availability (including the
//! exact reason), the off-Windows refusal, and the reachable-bridge refusal.
//!
//! Nothing here is evidence about Solid Edge or a Windows host: the mock
//! bridge implements the health contract only.

mod support;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use kala_cad_providers::{CadProviderError, SolidEdgeProvider, SolidEdgeProviderOptions};
use kala_json::{
    CanonicalObject, CanonicalValue, JsonText, Limits, canonical_dumps, strict_json_loads,
};
use support::{CannedResponse, MockBridge, health_body, pad_script};

const TIMEOUT_SECONDS: f64 = 2.0;

struct Case {
    id: String,
    platform: Option<String>,
    environ: BTreeMap<String, String>,
    url: Option<String>,
    token_file: Option<PathBuf>,
    /// One canned health response per pass, when this case reaches the bridge.
    response: Option<CannedResponse>,
    probe: bool,
    build: bool,
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn python_executable() -> PathBuf {
    if let Ok(path) = std::env::var("KALA_PYTHON") {
        return PathBuf::from(path);
    }
    let venv = repository_root().join(".venv/bin/python");
    if venv.is_file() {
        return venv;
    }
    PathBuf::from("python3")
}

fn field<'a>(object: &'a CanonicalObject, name: &str) -> Option<&'a CanonicalValue> {
    object.get(&JsonText::new(name))
}

fn as_object(value: &CanonicalValue) -> &CanonicalObject {
    match value {
        CanonicalValue::Object(object) => object,
        _ => panic!("expected a JSON object, found {value:?}"),
    }
}

fn as_array(value: &CanonicalValue) -> &[CanonicalValue] {
    match value {
        CanonicalValue::Array(items) => items,
        _ => panic!("expected a JSON array, found {value:?}"),
    }
}

fn as_text(value: Option<&CanonicalValue>) -> Option<String> {
    match value? {
        CanonicalValue::Text(text) => Some(text.to_string_lossy()),
        _ => None,
    }
}

fn as_bool(value: Option<&CanonicalValue>) -> Option<bool> {
    match value? {
        CanonicalValue::Bool(value) => Some(*value),
        _ => None,
    }
}

fn run_oracle(request: &CanonicalValue) -> CanonicalValue {
    let root = repository_root();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/solidedge_oracle.py");
    let payload = canonical_dumps(request, &Limits::default()).expect("canonical oracle request");
    let mut child = Command::new(python_executable())
        .arg("-I")
        .arg(&script)
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the Python oracle");
    child
        .stdin
        .take()
        .expect("oracle stdin")
        .write_all(payload.as_bytes())
        .expect("write the oracle request");
    let output = child.wait_with_output().expect("run the Python oracle");
    assert!(
        output.status.success(),
        "the Python oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    strict_json_loads(stdout.trim().as_bytes())
        .unwrap_or_else(|error| panic!("the Python oracle did not print JSON: {error}\n{stdout}"))
}

fn oracle_case<'a>(response: &'a CanonicalValue, id: &str) -> &'a CanonicalObject {
    let document = as_object(response);
    for case in as_array(field(document, "cases").expect("cases")) {
        let object = as_object(case);
        if as_text(field(object, "id")).as_deref() == Some(id) {
            return object;
        }
    }
    panic!("the oracle omitted {id}");
}

fn provider(case: &Case) -> SolidEdgeProvider {
    SolidEdgeProvider::new(SolidEdgeProviderOptions {
        platform: case.platform.clone(),
        environ: Some(case.environ.clone()),
        url: case.url.clone(),
        token_file: case.token_file.clone(),
        timeout_seconds: Some(TIMEOUT_SECONDS),
        probe_cache_seconds: Some(0.0),
    })
    .expect("a valid provider configuration")
}

fn request(cases: &[Case], probe: bool, build: bool) -> CanonicalValue {
    let cases = cases
        .iter()
        .map(|case| {
            CanonicalValue::object([
                ("id", CanonicalValue::text(case.id.as_str())),
                (
                    "platform",
                    match &case.platform {
                        Some(platform) => CanonicalValue::text(platform.as_str()),
                        None => CanonicalValue::Null,
                    },
                ),
                (
                    "environ",
                    CanonicalValue::object(
                        case.environ
                            .iter()
                            .map(|(key, value)| {
                                (
                                    JsonText::new(key.as_str()),
                                    CanonicalValue::text(value.as_str()),
                                )
                            })
                            .collect::<kala_json::CanonicalObject>(),
                    ),
                ),
                (
                    "url",
                    match &case.url {
                        Some(url) => CanonicalValue::text(url.as_str()),
                        None => CanonicalValue::Null,
                    },
                ),
                (
                    "token_file",
                    match &case.token_file {
                        Some(path) => CanonicalValue::text(path.to_string_lossy().into_owned()),
                        None => CanonicalValue::Null,
                    },
                ),
                ("probe", CanonicalValue::Bool(case.probe && probe)),
                ("build", CanonicalValue::Bool(case.build && build)),
            ])
        })
        .collect();
    CanonicalValue::object([
        ("cases", CanonicalValue::Array(cases)),
        ("probe", CanonicalValue::Bool(probe)),
        ("build", CanonicalValue::Bool(build)),
    ])
}

fn queue_responses(bridge: &MockBridge, cases: &[Case], passes: usize) {
    // The oracle probes every case first, then the Rust side probes every
    // case, so the queue is pass-major: one full sequence per pass.
    for _ in 0..passes {
        for case in cases {
            if let Some(response) = &case.response {
                bridge.queue(response.clone());
            }
        }
    }
}

fn assert_availability(
    id: &str,
    chain: &str,
    rust: &kala_cad_providers::Availability,
    oracle: &CanonicalValue,
) {
    let object = as_object(oracle);
    assert_eq!(
        rust.is_available(),
        as_bool(field(object, "available")).expect("available"),
        "{id}: available ({chain})"
    );
    assert_eq!(
        rust.version(),
        as_text(field(object, "version")).as_deref(),
        "{id}: version ({chain})"
    );
    assert_eq!(
        rust.reason(),
        as_text(field(object, "reason")).as_deref(),
        "{id}: reason ({chain})"
    );
}

fn assert_build(
    id: &str,
    chain: &str,
    rust: &Result<kala_cad_providers::ProviderBuild, CadProviderError>,
    oracle: &CanonicalValue,
) {
    let object = as_object(oracle);
    let kind = as_text(field(object, "kind")).expect("build kind");
    match (rust, kind.as_str()) {
        (Err(CadProviderError::Unavailable(error)), "unavailable") => {
            assert_eq!(
                error.message(),
                as_text(field(object, "message")).expect("message"),
                "{id}: unavailable message ({chain})"
            );
            assert_eq!(
                error.reason(),
                as_text(field(object, "reason")).expect("reason"),
                "{id}: unavailable reason ({chain})"
            );
            assert_eq!(
                error.provider(),
                as_text(field(object, "provider")).expect("provider"),
                "{id}: unavailable provider ({chain})"
            );
        }
        (Err(CadProviderError::Build(error)), "build") => {
            assert_eq!(
                error.message(),
                as_text(field(object, "message")).expect("message"),
                "{id}: build message ({chain})"
            );
            assert_eq!(
                error.op(),
                as_text(field(object, "op")).as_deref(),
                "{id}: build op ({chain})"
            );
        }
        (Ok(_), "built") => panic!("{id}: Rust built where the oracle says built; compare facts"),
        (rust, oracle_kind) => panic!("{id}: Rust {rust:?} but Python reported {oracle_kind}"),
    }
}

fn token_fixtures(directory: &std::path::Path) -> BTreeMap<&'static str, PathBuf> {
    let mut files = BTreeMap::new();
    for (name, content) in [
        ("valid", "token-0123456789abcdef"),
        ("short", "too-short"),
        ("whitespace", "  token-0123456789abcdef  \n"),
    ] {
        let path = directory.join(name);
        std::fs::write(&path, content).expect("write a token fixture");
        files.insert(name, path);
    }
    let long = directory.join("long");
    std::fs::write(&long, "a".repeat(257)).expect("write a long token");
    files.insert("long", long);
    let non_ascii = directory.join("non_ascii");
    std::fs::write(&non_ascii, "é".repeat(20)).expect("write a non-ASCII token");
    files.insert("non_ascii", non_ascii);
    files
}

fn probe_cases(bridge_url: &str, files: &BTreeMap<&'static str, PathBuf>) -> Vec<Case> {
    let mut cases = Vec::new();
    let valid = files["valid"].clone();
    let mut base_environ = BTreeMap::new();
    base_environ.insert(
        "KALA_SOLIDEDGE_TOKEN_FILE".to_owned(),
        valid.to_string_lossy().into_owned(),
    );
    let case = |id: &str,
                platform: Option<&str>,
                environ: BTreeMap<String, String>,
                url: Option<String>,
                token_file: Option<PathBuf>,
                response: Option<CannedResponse>| Case {
        id: id.to_owned(),
        platform: platform.map(str::to_owned),
        environ,
        url,
        token_file,
        response,
        probe: true,
        build: false,
    };
    cases.push(case(
        "linux_default",
        None,
        BTreeMap::new(),
        None,
        None,
        None,
    ));
    cases.push(case(
        "linux_explicit",
        Some("linux"),
        BTreeMap::new(),
        None,
        None,
        None,
    ));
    cases.push(case(
        "darwin",
        Some("darwin"),
        BTreeMap::new(),
        None,
        None,
        None,
    ));
    for (id, url) in [
        ("url_https", "https://127.0.0.1:8797"),
        ("url_other_host", "http://example.com:8797"),
        ("url_no_port", "http://127.0.0.1"),
        ("url_port_zero", "http://127.0.0.1:0"),
        ("url_path", "http://127.0.0.1:8797/v1"),
        ("url_query", "http://127.0.0.1:8797/?a=1"),
        ("url_fragment", "http://127.0.0.1:8797/#x"),
        ("url_userinfo", "http://user:pass@127.0.0.1:8797"),
    ] {
        cases.push(case(
            id,
            Some("win32"),
            base_environ.clone(),
            Some(url.to_owned()),
            Some(valid.clone()),
            None,
        ));
    }
    let response = Some(CannedResponse::json(
        200,
        &health_body(true, true, "2024", ""),
    ));
    cases.push(case(
        "token_missing",
        Some("win32"),
        BTreeMap::new(),
        Some(bridge_url.to_owned()),
        Some(files["short"].with_file_name("does-not-exist")),
        None,
    ));
    for name in ["short", "long", "non_ascii"] {
        cases.push(case(
            name,
            Some("win32"),
            BTreeMap::new(),
            Some(bridge_url.to_owned()),
            Some(files[name].clone()),
            None,
        ));
    }
    cases.push(case(
        "token_whitespace",
        Some("win32"),
        BTreeMap::new(),
        Some(bridge_url.to_owned()),
        Some(files["whitespace"].clone()),
        response.clone(),
    ));
    let mut env_token = BTreeMap::new();
    env_token.insert(
        "KALA_SOLIDEDGE_TOKEN_FILE".to_owned(),
        files["valid"].to_string_lossy().into_owned(),
    );
    cases.push(case(
        "token_from_environment",
        Some("win32"),
        env_token,
        Some(bridge_url.to_owned()),
        None,
        response.clone(),
    ));
    let local_app_data_root = files["valid"].parent().expect("a directory").to_path_buf();
    let default_token = local_app_data_root
        .join("Kala")
        .join("SolidEdgeBridge")
        .join("bridge.token");
    std::fs::create_dir_all(default_token.parent().expect("a parent"))
        .expect("the default token directory");
    std::fs::write(&default_token, "token-0123456789abcdef").expect("the default token");
    let mut local_app_data = BTreeMap::new();
    local_app_data.insert(
        "LOCALAPPDATA".to_owned(),
        local_app_data_root.to_string_lossy().into_owned(),
    );
    cases.push(case(
        "token_from_local_app_data",
        Some("win32"),
        local_app_data,
        Some(bridge_url.to_owned()),
        None,
        response.clone(),
    ));
    cases.push(case(
        "health_available",
        Some("win32"),
        base_environ.clone(),
        Some(bridge_url.to_owned()),
        Some(valid.clone()),
        response.clone(),
    ));
    cases.push(case(
        "health_unavailable_with_detail",
        Some("win32"),
        base_environ.clone(),
        Some(bridge_url.to_owned()),
        Some(valid.clone()),
        Some(CannedResponse::json(
            200,
            &health_body(false, true, "2024", "Solid Edge is not running"),
        )),
    ));
    cases.push(case(
        "health_no_capability",
        Some("win32"),
        base_environ.clone(),
        Some(bridge_url.to_owned()),
        Some(valid.clone()),
        Some(CannedResponse::json(
            200,
            &health_body(true, false, "2024", ""),
        )),
    ));
    cases.push(case(
        "health_blank_detail",
        Some("win32"),
        base_environ.clone(),
        Some(bridge_url.to_owned()),
        Some(valid.clone()),
        Some(CannedResponse::json(
            200,
            &health_body(false, true, "2024", "   "),
        )),
    ));
    cases.push(case(
        "health_401",
        Some("win32"),
        base_environ.clone(),
        Some(bridge_url.to_owned()),
        Some(valid.clone()),
        Some(CannedResponse::json(
            401,
            r#"{"code":"invalid_request","message":"authentication failed"}"#,
        )),
    ));
    cases.push(case(
        "health_not_json",
        Some("win32"),
        base_environ.clone(),
        Some(bridge_url.to_owned()),
        Some(valid.clone()),
        Some(CannedResponse::json(200, "not json")),
    ));
    cases.push(case(
        "health_redirect",
        Some("win32"),
        base_environ.clone(),
        Some(bridge_url.to_owned()),
        Some(valid.clone()),
        Some(CannedResponse::json(302, "").with_headers(vec![(
            "Location".to_owned(),
            "http://example.com/".to_owned(),
        )])),
    ));
    let localhost = bridge_url.replace("127.0.0.1", "localhost");
    cases.push(case(
        "url_localhost",
        Some("win32"),
        base_environ.clone(),
        Some(localhost),
        Some(valid.clone()),
        response.clone(),
    ));
    cases.push(case(
        "url_trailing_slash",
        Some("win32"),
        base_environ.clone(),
        Some(format!("{bridge_url}/")),
        Some(valid.clone()),
        response,
    ));
    cases
}

#[test]
fn solidedge_probe_matches_the_live_python_provider() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let files = token_fixtures(directory.path());
    let bridge = MockBridge::start();
    let cases = probe_cases(&bridge.url(), &files);
    queue_responses(&bridge, &cases, 2);
    let response = run_oracle(&request(&cases, true, false));
    for case in &cases {
        let oracle = as_object(
            field(oracle_case(&response, &case.id), "availability").expect("availability"),
        );
        let mut provider = provider(case);
        let availability = provider.probe();
        assert_availability(
            &case.id,
            "probe",
            &availability,
            &CanonicalValue::Object(oracle.clone()),
        );
    }
}

fn build_cases(bridge_url: &str, files: &BTreeMap<&'static str, PathBuf>) -> Vec<Case> {
    let valid = files["valid"].clone();
    let mut environ = BTreeMap::new();
    environ.insert(
        "KALA_SOLIDEDGE_TOKEN_FILE".to_owned(),
        valid.to_string_lossy().into_owned(),
    );
    let case = |id: &str,
                platform: Option<&str>,
                url: Option<String>,
                response: Option<CannedResponse>| Case {
        id: id.to_owned(),
        platform: platform.map(str::to_owned),
        environ: environ.clone(),
        url,
        token_file: Some(valid.clone()),
        response,
        probe: false,
        build: true,
    };
    vec![
        case("build_linux", None, None, None),
        case(
            "build_win32_healthy",
            Some("win32"),
            Some(bridge_url.to_owned()),
            Some(CannedResponse::json(
                200,
                &health_body(true, true, "2024", ""),
            )),
        ),
        case(
            "build_win32_no_capability",
            Some("win32"),
            Some(bridge_url.to_owned()),
            Some(CannedResponse::json(
                200,
                &health_body(true, false, "2024", ""),
            )),
        ),
        case(
            "build_win32_unreachable",
            Some("win32"),
            Some(bridge_url.to_owned()),
            Some(CannedResponse::json(401, "{}")),
        ),
    ]
}

#[test]
fn solidedge_build_refusals_match_the_live_python_provider() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let files = token_fixtures(directory.path());
    let bridge = MockBridge::start();
    let cases = build_cases(&bridge.url(), &files);
    queue_responses(&bridge, &cases, 2);
    let response = run_oracle(&request(&cases, false, true));
    for case in &cases {
        let oracle = as_object(field(oracle_case(&response, &case.id), "build").expect("build"));
        let mut provider = provider(case);
        let outcome = provider.build(&pad_script());
        assert_build(
            &case.id,
            "build",
            &outcome,
            &CanonicalValue::Object(oracle.clone()),
        );
    }
}

#[test]
fn solidedge_url_resolution_matches_python_without_network() {
    // A missing token file stops every accepted URL before a socket is opened,
    // so this compares the URL parser alone.
    let directory = tempfile::tempdir().expect("a temporary directory");
    let missing = directory.path().join("missing.token");
    let urls = [
        "http://127.0.0.1:8797",
        "HTTP://127.0.0.1:8797",
        "http://LOCALHOST:8797",
        "http://[::1]:8797",
        "http://::1:8797",
        "http://127.0.0.1:0008797",
        "http://127.0.0.1:65535",
        "http://127.0.0.1:065535",
        "http://127.0.0.1:65536",
        "http://127.0.0.1:http",
        "http://127.0.0.1:-1",
        "http://127.0.0.1:1",
        "http://127.0.0.1:",
        "http://127.0.0.1",
        "http://127.0.0.1:8797/",
        "http://127.0.0.1:8797//",
        "http://127.0.0.1:8797/?",
        "http://127.0.0.1:8797/#",
        "http://127.0.0.1:8797/?a",
        "http://127.0.0.1:8797/#a",
        "http://@127.0.0.1:8797",
        "http://:pass@127.0.0.1:8797",
        "http://user@127.0.0.1:8797",
        "http://127.0.0.1:8797@evil:80",
        "https://127.0.0.1:8797",
        "ftp://127.0.0.1:8797",
        "127.0.0.1:8797",
        "http:127.0.0.1:8797",
        "http://[::1:8797",
        "http://[::1]:8797/x",
        "http://127.0.0.2:8797",
        "http://0.0.0.0:8797",
        "http://",
        "http:///x",
        "  http://127.0.0.1:8797  ",
        "http://127.0.0.1:8797\n",
        "http\t://127.0.0.1:8797",
        "http://127.0.0.1:8797\\x",
        "http://127.0.0.1:8797?",
    ];
    let mut generated: Vec<String> = urls.iter().map(|url| (*url).to_owned()).collect();
    let schemes = ["http", "HTTP", "https", "ftp", ""];
    let hosts = [
        "127.0.0.1",
        "localhost",
        "::1",
        "[::1]",
        "127.0.0.2",
        "example.com",
        "",
        "LOCALHOST",
    ];
    let ports = [
        ":8797", "", ":0", ":65535", ":65536", ":http", ":-1", ":000123",
    ];
    let paths = ["", "/", "/x", "//", "/?", "/#", "/?a", "/#a"];
    let userinfos = ["", "user@", ":pass@", "user:pass@", "@"];
    let mut state: u64 = 0xC0FF_EE12_3456_789A;
    for _ in 0..180 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        fn pick<'a>(values: &'a [&'a str], state: u64) -> &'a str {
            values[(state >> 8) as usize % values.len()]
        }
        let scheme = pick(&schemes, state);
        let host = pick(&hosts, state.rotate_left(3));
        let port = pick(&ports, state.rotate_left(5));
        let path = pick(&paths, state.rotate_left(9));
        let userinfo = pick(&userinfos, state.rotate_left(11));
        let separator = if scheme.is_empty() { "" } else { "://" };
        generated.push(format!("{scheme}{separator}{userinfo}{host}{port}{path}"));
        state = state.rotate_left(7) ^ state.wrapping_mul(0x2545_F491_4F6C_DD1D);
    }
    let cases: Vec<Case> = generated
        .iter()
        .enumerate()
        .map(|(index, url)| Case {
            id: format!("url_{index:03}"),
            platform: Some("win32".to_owned()),
            environ: BTreeMap::new(),
            url: Some(url.clone()),
            token_file: Some(missing.clone()),
            response: None,
            probe: true,
            build: false,
        })
        .collect();
    assert!(cases.len() >= 200, "the URL corpus shrank");
    let response = run_oracle(&request(&cases, true, false));
    let mut accepted = 0_usize;
    let mut rejected = 0_usize;
    for case in &cases {
        let oracle_case = oracle_case(&response, &case.id);
        let mut provider = provider(case);
        let availability = provider.probe();
        if availability
            .reason()
            .is_some_and(|reason| reason.contains("token file was not found"))
        {
            accepted += 1;
        } else {
            rejected += 1;
        }
        match field(oracle_case, "exception") {
            Some(exception) => {
                let exception = as_text(Some(exception)).expect("the exception text");
                // Documented difference: Python's `urlparse().port` raises
                // outside its try block for an out-of-range or non-numeric
                // port; the native provider classifies it as an invalid URL.
                assert!(
                    exception.starts_with("ValueError: "),
                    "{}: unexpected Python exception {exception}",
                    case.id
                );
                assert!(!availability.is_available(), "{}", case.id);
                assert_eq!(
                    availability.reason(),
                    Some("the Solid Edge bridge URL is not a loopback HTTP URL"),
                    "{}",
                    case.id
                );
            }
            None => {
                let oracle = as_object(field(oracle_case, "availability").expect("availability"));
                assert_availability(
                    &case.id,
                    "url",
                    &availability,
                    &CanonicalValue::Object(oracle.clone()),
                );
            }
        }
    }
    assert!(
        accepted >= 10,
        "the URL corpus lost accepted URLs: {accepted}"
    );
    assert!(
        rejected >= 50,
        "the URL corpus lost refused URLs: {rejected}"
    );
}
