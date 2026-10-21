//! FreeCAD provider unit, contract and security tests.
//!
//! These tests use a **fake** `FreeCADCmd` (`tests/fixtures/fake_freecad.py`).
//! It exercises the real subprocess hand-off, the result-envelope parser, the
//! artifact containment and integrity checks, and the bounded timeout without
//! pretending to compute geometry. The real FreeCAD differential lives in
//! `freecad_differential.rs`.

mod support;

use std::collections::BTreeMap;
use std::path::Path;

use kala_cad_providers::{
    ALLOWED_PROVIDER_IDS, Availability, BundledPythonSelection, CadProvider, CadProviderBuildError,
    CadProviderError, CadProviderId, CadValueError, FreeCadProvider, FreeCadProviderOptions,
    MAX_REASON_LENGTH, MAX_VERSION_LENGTH, MAX_WARNING_LENGTH, MAX_WARNINGS, MeasuredBounds,
    MeasuredProperties, ProviderBuild, SUPPORTED_FREECAD_PYTHON_ABIS, UNAVAILABLE_REASON,
    bundled_python_abis, canonical_request_json, resolve_step_import_max_bytes,
    select_bundled_python_abi, validate_step_import_max_bytes,
};
use support::{
    fake_freecad, fake_provider, fake_provider_with, fuse_script, pad_script, place_only_script,
};

fn build(provider: &mut FreeCadProvider) -> Result<ProviderBuild, CadProviderError> {
    provider.build(&pad_script())
}

#[test]
fn probe_reads_a_real_version_and_reports_a_missing_one() {
    let (_directory, executable) = fake_freecad();
    let mut provider = fake_provider(&executable, "version_ok");
    let availability = provider.probe();
    assert!(availability.is_available());
    assert_eq!(availability.version(), Some("1.1.4"));
    assert_eq!(availability.reason(), None);

    let mut long = fake_provider(&executable, "version_long");
    let availability = long.probe();
    assert!(availability.is_available());
    assert_eq!(availability.version(), Some("1.2.3.4-beta.1"));

    let mut missing = fake_provider(&executable, "version_bad");
    let availability = missing.probe();
    assert!(!availability.is_available());
    assert_eq!(availability.version(), None);
    assert_eq!(
        availability.reason(),
        Some("FreeCADCmd did not report a FreeCAD version: this is not a FreeCAD banner")
    );
}

#[test]
fn a_missing_or_unknown_executable_reports_the_unavailable_reason() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let missing = directory.path().join("does-not-exist");
    let mut provider = fake_provider(&missing, "version_ok");
    assert_eq!(provider.probe().reason(), Some(UNAVAILABLE_REASON));

    let mut provider = FreeCadProvider::new(FreeCadProviderOptions {
        environ: Some(BTreeMap::new()),
        ..FreeCadProviderOptions::default()
    })
    .expect("a valid provider with no PATH");
    assert_eq!(provider.probe().reason(), Some(UNAVAILABLE_REASON));
}

#[test]
fn probe_is_cached_for_the_configured_duration() {
    let (directory, executable) = fake_freecad();
    let counter = directory.path().join("counter.log");
    let mut environment = support::fake_environment("version_ok");
    environment.insert(
        "FAKE_FREECAD_COUNTER".to_owned(),
        counter.to_string_lossy().into_owned(),
    );
    let mut cached = FreeCadProvider::new(FreeCadProviderOptions {
        executable: Some(executable.clone()),
        probe_cache_seconds: Some(60.0),
        environ: Some(environment.clone()),
        sys_path: Some(Vec::new()),
        ..FreeCadProviderOptions::default()
    })
    .expect("a valid provider");
    assert!(cached.probe().is_available());
    assert!(cached.probe().is_available());
    let lines = std::fs::read_to_string(&counter).expect("the counter");
    assert_eq!(lines.lines().count(), 1, "the probe was not cached");

    let mut fresh = FreeCadProvider::new(FreeCadProviderOptions {
        executable: Some(executable),
        probe_cache_seconds: Some(0.0),
        environ: Some(environment),
        sys_path: Some(Vec::new()),
        ..FreeCadProviderOptions::default()
    })
    .expect("a valid provider");
    assert!(fresh.probe().is_available());
    assert!(fresh.probe().is_available());
    let lines = std::fs::read_to_string(&counter).expect("the counter");
    assert_eq!(lines.lines().count(), 3, "a zero cache must probe again");
}

#[test]
fn build_parses_the_envelope_and_hashes_artifacts() {
    let (_directory, executable) = fake_freecad();
    let mut provider = fake_provider(&executable, "success");
    let build = build(&mut provider).expect("the fake build succeeds");
    assert_eq!(build.step_bytes(), b"ISO-10303-21\nFAKE STEP\n");
    assert_eq!(build.native_bytes(), Some(&b"PK\x03\x04FAKE FCSTD"[..]));
    assert_eq!(build.native_kind(), Some("fcstd"));
    assert_eq!(build.provider_version(), Some("9.9.9"));
    assert_eq!(build.valid(), Some(true));
    assert_eq!(build.solid_count(), Some(1));
    assert_eq!(build.warnings(), ["a fake provider warning"]);
    assert_eq!(build.measured().volume_mm3(), Some(120.0));
    assert_eq!(build.measured().area_mm2(), Some(148.0));
    assert_eq!(build.measured().mass_g(), None);
    assert_eq!(build.measured().centroid_mm(), None);
    let bounds = build.measured().bounds().expect("bounds");
    assert_eq!(bounds.min_mm(), [0.0, 0.0, 0.0]);
    assert_eq!(bounds.max_mm(), [4.0, 5.0, 6.0]);
    assert_eq!(bounds.size_mm(), [4.0, 5.0, 6.0]);
}

#[test]
fn the_handoff_request_is_the_canonical_script_bytes() {
    let (directory, executable) = fake_freecad();
    let dump = directory.path().join("request.json");
    let mut extra = BTreeMap::new();
    extra.insert(
        "FAKE_FREECAD_REQUEST_DUMP".to_owned(),
        dump.to_string_lossy().into_owned(),
    );
    let mut provider = fake_provider_with(&executable, "success", extra);
    let script = pad_script();
    build(&mut provider).expect("the fake build succeeds");
    let observed = std::fs::read_to_string(&dump).expect("the request dump");
    assert_eq!(
        observed,
        canonical_request_json(&script).expect("canonical request bytes")
    );
    assert!(observed.starts_with(r#"{"script":{"assembly":[],"components":[{"name":"body","#));
    assert!(
        !observed.contains(":null"),
        "exclude_none must drop null fields"
    );
}

#[test]
fn the_child_environment_is_scrubbed_and_carries_the_protocol_variables() {
    let (directory, executable) = fake_freecad();
    let dump = directory.path().join("environment.json");
    let sys_path = directory.path().join("adapter");
    std::fs::create_dir_all(&sys_path).expect("the sys path");
    let mut environment = support::fake_environment("success");
    environment.insert(
        "FAKE_FREECAD_DUMP".to_owned(),
        dump.to_string_lossy().into_owned(),
    );
    let mut provider = FreeCadProvider::new(FreeCadProviderOptions {
        executable: Some(executable),
        environ: Some(environment),
        sys_path: Some(vec![sys_path.clone(), directory.path().join("absent")]),
        probe_cache_seconds: Some(0.0),
        ..FreeCadProviderOptions::default()
    })
    .expect("a valid provider");
    build(&mut provider).expect("the fake build succeeds");
    let record = kala_json::strict_json_loads(&std::fs::read(&dump).expect("the env dump"))
        .expect("the dump is strict JSON");
    let observed = match &record {
        kala_json::CanonicalValue::Object(object) => object,
        _ => panic!("the environment dump is not an object"),
    };
    let text = |name: &str| {
        observed
            .get(&kala_json::JsonText::new(name))
            .and_then(kala_json::CanonicalValue::as_text)
            .map(kala_json::JsonText::to_string_lossy)
    };
    for forbidden in [
        "OPENAI_API_KEY",
        "KALA_WORKER_TOKEN",
        "DATABASE_PASSWORD",
        "MY_SECRET",
        "AWS_CREDENTIAL",
        "PYTHONPATH",
    ] {
        assert_eq!(
            text(forbidden),
            None,
            "{forbidden} reached the FreeCAD child"
        );
    }
    assert_eq!(text("PYTHONDONTWRITEBYTECODE").as_deref(), Some("1"));
    assert_eq!(
        text("KALA_FREECAD_MAX_ARTIFACT_BYTES").as_deref(),
        Some("67108864")
    );
    assert_eq!(
        text("KALA_FREECAD_MAX_STEP_BYTES").as_deref(),
        Some("67108864")
    );
    assert_eq!(
        text("KALA_FREECAD_SYS_PATH").as_deref(),
        Some(sys_path.to_string_lossy().as_ref())
    );
    assert!(
        text("KALA_FREECAD_REQUEST")
            .is_some_and(|value| value.ends_with("kala-freecad-request.json"))
    );
    assert!(
        text("KALA_FREECAD_OUTPUT")
            .is_some_and(|value| value.ends_with("kala-freecad-response.json"))
    );
    let base = support::fake_environment("success");
    assert_eq!(text("PATH").as_deref(), Some(base["PATH"].as_str()));
    assert_eq!(text("HOME").as_deref(), Some(base["HOME"].as_str()));
    assert_eq!(text("LANG").as_deref(), Some("C.UTF-8"));
}

#[test]
fn a_refusal_names_the_assembly_operation() {
    let (_directory, executable) = fake_freecad();
    let mut provider = fake_provider(&executable, "refuse");
    let error = provider
        .build(&fuse_script())
        .expect_err("the fake refuses");
    assert_eq!(
        error.message(),
        "FreeCADOperationError: did not produce a solid (op: fuse)"
    );
    // Python appends the op to the message but leaves the `op` field unset on
    // this path; only the provider's own pre-build refusal names the op.
    assert_eq!(error.op(), None);

    let mut provider = fake_provider(&executable, "refuse");
    let error = provider.build(&pad_script()).expect_err("the fake refuses");
    assert_eq!(
        error.message(),
        "FreeCADOperationError: did not produce a solid"
    );
    assert_eq!(error.op(), None);

    let mut provider = fake_provider(&executable, "refuse_no_op");
    let error = provider.build(&pad_script()).expect_err("the fake refuses");
    assert_eq!(error.message(), "FreeCAD refused this script");

    let mut provider = fake_provider(&executable, "refuse_none");
    let error = provider.build(&pad_script()).expect_err("the fake refuses");
    assert_eq!(error.message(), "None");
}

#[test]
fn a_place_only_assembly_is_sent_to_freecad_instead_of_refused() {
    let (_directory, executable) = fake_freecad();
    let mut provider = fake_provider(&executable, "no_response");
    let error = provider
        .build(&place_only_script())
        .expect_err("the fake returns no result");
    // The provider must not pre-refuse the place-only assembly: the adapter
    // owns the export target and is the one that builds it.
    assert!(
        error
            .message()
            .starts_with("FreeCAD did not report a result (exit 3)"),
        "{}",
        error.message()
    );
    assert_eq!(error.op(), None);
}

#[test]
fn artifact_containment_and_integrity_errors_are_explicit() {
    let (_directory, executable) = fake_freecad();
    let cases = [
        (
            "escape",
            "the FreeCAD STEP artifact is outside the build workspace",
        ),
        (
            "symlink_escape",
            "the FreeCAD STEP artifact is outside the build workspace",
        ),
        (
            "size_over",
            "the FreeCAD STEP export is 1000000000000000000000000000000 bytes, \
             above the configured limit of 67108864 bytes",
        ),
        (
            "size_mismatch",
            "the FreeCAD STEP artifact changed size before it was read",
        ),
        (
            "digest_mismatch",
            "the FreeCAD STEP artifact does not match the digest FreeCAD reported",
        ),
        (
            "descriptor_incomplete",
            "the FreeCAD STEP artifact descriptor is incomplete",
        ),
        ("missing_step", "FreeCAD did not export a STEP file"),
        ("missing_native", "FreeCAD did not export an FCStd file"),
        ("bad_artifacts", "FreeCAD reported no artifacts"),
    ];
    for (mode, expected) in cases {
        let mut provider = fake_provider(&executable, mode);
        let error = build(&mut provider).expect_err(mode);
        assert_eq!(error.message(), expected, "mode {mode}");
        assert_eq!(error.op(), None, "mode {mode}");
    }
}

#[test]
fn response_envelope_errors_report_the_exit_and_a_bounded_stderr_tail() {
    let (_directory, executable) = fake_freecad();
    for mode in ["invalid_json", "non_dict", "oversize"] {
        let mut provider = fake_provider(&executable, mode);
        let error = build(&mut provider).expect_err(mode);
        assert_eq!(
            error.message(),
            "FreeCAD did not report a result (exit 0)",
            "mode {mode}"
        );
    }
    let mut provider = fake_provider(&executable, "no_response");
    let error = build(&mut provider).expect_err("no response");
    let message = error.message();
    assert!(
        message.starts_with("FreeCAD did not report a result (exit 3): "),
        "{message}"
    );
    let detail = message
        .strip_prefix("FreeCAD did not report a result (exit 3): ")
        .expect("the prefix");
    assert_eq!(detail.chars().count(), 400, "the stderr tail is bounded");
    assert!(detail.chars().all(|character| character == 'x'), "{detail}");

    let mut provider = fake_provider(&executable, "no_response_empty_stderr");
    let error = build(&mut provider).expect_err("no response");
    assert_eq!(error.message(), "FreeCAD did not report a result (exit 4)");
}

#[test]
fn a_build_is_bounded_by_the_configured_wall_time() {
    let (_directory, executable) = fake_freecad();
    let environment = support::fake_environment("sleep_build");
    let mut provider = FreeCadProvider::new(FreeCadProviderOptions {
        executable: Some(executable),
        timeout_seconds: Some(5.0),
        environ: Some(environment),
        sys_path: Some(Vec::new()),
        probe_cache_seconds: Some(0.0),
        ..FreeCadProviderOptions::default()
    })
    .expect("a valid provider");
    let started = std::time::Instant::now();
    let error = provider
        .build(&pad_script())
        .expect_err("the fake does not finish");
    assert_eq!(error.message(), "FreeCAD did not finish within 5 seconds");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(15),
        "the timeout did not bound the build"
    );
}

#[test]
fn unreported_values_are_never_invented() {
    let (_directory, executable) = fake_freecad();
    let mut provider = fake_provider(&executable, "edge_metrics");
    let build = build(&mut provider).expect("the fake build succeeds");
    assert_eq!(build.measured().volume_mm3(), None);
    assert_eq!(build.measured().area_mm2(), None);
    let bounds = build
        .measured()
        .bounds()
        .expect("numeric strings are floats");
    assert_eq!(bounds.min_mm(), [-1.5, -2.5, -3.5]);
    assert_eq!(bounds.max_mm(), [4.5, 5.5, 6.5]);
    assert_eq!(build.valid(), None);
    assert_eq!(build.solid_count(), None);
    assert_eq!(
        build.provider_version().expect("version").chars().count(),
        MAX_VERSION_LENGTH
    );
    // The payload carries 22 usable warnings: fourteen are shown unchanged
    // and one summary states the eight that were suppressed.
    assert_eq!(build.warnings().len(), 15);
    assert_eq!(build.warnings()[0], "kept");
    assert_eq!(build.warnings()[1].chars().count(), MAX_WARNING_LENGTH);
    assert_eq!(build.warnings()[13], "www");
    assert_eq!(build.warnings()[14], "… and 8 more warnings");
}

#[test]
fn invalid_bounds_are_dropped_instead_of_guessed() {
    let (_directory, executable) = fake_freecad();
    for mode in ["bounds_bad", "bounds_inverted", "bounds_nan"] {
        let mut provider = fake_provider(&executable, mode);
        let build = build(&mut provider).expect(mode);
        assert_eq!(build.measured().bounds(), None, "mode {mode}");
        assert!(build.measured().volume_mm3().is_some(), "mode {mode}");
    }
}

#[test]
fn warnings_and_versions_are_bounded() {
    let (_directory, executable) = fake_freecad();
    let mut provider = fake_provider(&executable, "warning_truncation");
    let build = build(&mut provider).expect("the fake build succeeds");
    // Twenty warnings bound to the first fourteen unchanged lines plus one
    // summary that keeps the number of suppressed warnings.
    assert_eq!(build.warnings().len(), MAX_WARNINGS - 1);
    for warning in &build.warnings()[..MAX_WARNINGS - 2] {
        assert_eq!(warning.chars().count(), MAX_WARNING_LENGTH);
    }
    assert_eq!(build.warnings()[MAX_WARNINGS - 2], "… and 6 more warnings");
}

#[test]
fn constructor_validation_matches_python() {
    let error = FreeCadProvider::new(FreeCadProviderOptions {
        timeout_seconds: Some(4.999),
        environ: Some(BTreeMap::new()),
        ..FreeCadProviderOptions::default()
    })
    .expect_err("below the minimum");
    assert_eq!(
        error.message(),
        "the FreeCAD build timeout must be between 5 and 3600 seconds"
    );
    let error = FreeCadProvider::new(FreeCadProviderOptions {
        timeout_seconds: Some(3_600.5),
        environ: Some(BTreeMap::new()),
        ..FreeCadProviderOptions::default()
    })
    .expect_err("above the maximum");
    assert_eq!(
        error.message(),
        "the FreeCAD build timeout must be between 5 and 3600 seconds"
    );
    let error = FreeCadProvider::new(FreeCadProviderOptions {
        probe_cache_seconds: Some(-1.0),
        environ: Some(BTreeMap::new()),
        ..FreeCadProviderOptions::default()
    })
    .expect_err("negative cache");
    assert_eq!(
        error.message(),
        "the probe cache duration must be non-negative"
    );
    let error = FreeCadProvider::new(FreeCadProviderOptions {
        max_step_bytes: Some(0),
        environ: Some(BTreeMap::new()),
        ..FreeCadProviderOptions::default()
    })
    .expect_err("zero limit");
    assert_eq!(
        error.message(),
        "the STEP import limit must be between 1 and 536870912 bytes"
    );
    let error = validate_step_import_max_bytes(536_870_913).expect_err("above the maximum");
    assert_eq!(
        error.message(),
        "the STEP import limit must be between 1 and 536870912 bytes"
    );
    let provider = FreeCadProvider::new(FreeCadProviderOptions {
        timeout_seconds: Some(5.0),
        probe_cache_seconds: Some(0.0),
        max_step_bytes: Some(1),
        environ: Some(BTreeMap::new()),
        ..FreeCadProviderOptions::default()
    })
    .expect("the boundary values are accepted");
    assert_eq!(provider.max_step_bytes(), 1);
    assert_eq!(provider.timeout_seconds(), 5.0);

    // `5.5` formats like Python's `%g`.
    let error = FreeCadProvider::new(FreeCadProviderOptions {
        timeout_seconds: Some(5.5),
        environ: Some(BTreeMap::new()),
        ..FreeCadProviderOptions::default()
    });
    assert!(error.is_ok());
}

#[test]
fn the_step_limit_is_resolved_from_the_environment() {
    let mut environment = BTreeMap::new();
    assert_eq!(
        resolve_step_import_max_bytes(&environment).expect("the default"),
        64 * 1024 * 1024
    );
    environment.insert("KALA_STEP_IMPORT_MAX_BYTES".to_owned(), "  ".to_owned());
    assert_eq!(
        resolve_step_import_max_bytes(&environment).expect("a blank value keeps the default"),
        64 * 1024 * 1024
    );
    environment.insert("KALA_STEP_IMPORT_MAX_BYTES".to_owned(), "1024".to_owned());
    assert_eq!(
        resolve_step_import_max_bytes(&environment).expect("1024"),
        1024
    );
    environment.insert("KALA_STEP_IMPORT_MAX_BYTES".to_owned(), "1_000".to_owned());
    assert_eq!(
        resolve_step_import_max_bytes(&environment).expect("Python int underscores"),
        1000
    );
    for (value, expected) in [
        (
            "abc",
            "KALA_STEP_IMPORT_MAX_BYTES must be an integer number of bytes",
        ),
        (
            "-1",
            "the STEP import limit must be between 1 and 536870912 bytes",
        ),
        (
            "999999999999999999999999999999999999999999",
            "the STEP import limit must be between 1 and 536870912 bytes",
        ),
    ] {
        environment.insert("KALA_STEP_IMPORT_MAX_BYTES".to_owned(), value.to_owned());
        assert_eq!(
            resolve_step_import_max_bytes(&environment)
                .expect_err(value)
                .message(),
            expected,
            "{value}"
        );
    }
    // An explicit argument wins over the environment.
    environment.insert("KALA_STEP_IMPORT_MAX_BYTES".to_owned(), "abc".to_owned());
    let provider = FreeCadProvider::new(FreeCadProviderOptions {
        max_step_bytes: Some(2048),
        environ: Some(environment),
        ..FreeCadProviderOptions::default()
    })
    .expect("the explicit limit wins");
    assert_eq!(provider.max_step_bytes(), 2048);
}

#[test]
fn provider_status_publishes_the_static_identity() {
    let (_directory, executable) = fake_freecad();
    let mut provider = fake_provider(&executable, "version_ok");
    let status = provider.status();
    assert_eq!(status.id(), "freecad");
    assert_eq!(status.name(), "FreeCAD");
    assert!(status.available());
    assert_eq!(status.version(), Some("1.1.4"));
    assert_eq!(status.reason(), None);
    assert_eq!(status.measured_by(), "FreeCAD");
    assert_eq!(status.mode(), "external");
}

#[test]
fn the_provider_set_is_closed_and_has_no_default() {
    assert_eq!(
        ALLOWED_PROVIDER_IDS,
        ["opencascade", "freecad", "solidedge"]
    );
    assert_eq!(CadProviderId::ALL.len(), 3);
    assert_eq!(
        CadProviderId::parse("freecad"),
        Some(CadProviderId::FreeCad)
    );
    assert_eq!(CadProviderId::parse("FreeCAD"), None);
    assert_eq!(CadProviderId::parse("solidworks"), None);
    for id in CadProviderId::ALL {
        assert!(ALLOWED_PROVIDER_IDS.contains(&id.as_str()));
        assert_eq!(CadProviderId::parse(id.as_str()), Some(id));
    }
    assert_eq!(format!("{}", CadProviderId::SolidEdge), "solidedge");
}

#[test]
fn the_contract_models_validate_like_python() {
    let availability = Availability::new(true, Some("v".repeat(100)), Some("r".repeat(600)));
    assert_eq!(
        availability.version().expect("version").chars().count(),
        MAX_VERSION_LENGTH
    );
    assert_eq!(
        availability.reason().expect("reason").chars().count(),
        MAX_REASON_LENGTH
    );
    let availability = Availability::new(false, None::<String>, Some(""));
    assert_eq!(availability.reason(), Some(""));

    let bounds = MeasuredBounds::new([0.0, 0.0, 0.0], [2.0, 3.0, 4.0]).expect("valid bounds");
    assert_eq!(bounds.size_mm(), [2.0, 3.0, 4.0]);
    assert_eq!(
        MeasuredBounds::new([f64::NAN, 0.0, 0.0], [1.0, 1.0, 1.0])
            .expect_err("non-finite")
            .message(),
        "provider bounds must be finite numbers"
    );
    assert_eq!(
        MeasuredBounds::new([2.0, 0.0, 0.0], [1.0, 1.0, 1.0])
            .expect_err("inverted")
            .message(),
        "provider bounds minimum exceeds maximum"
    );

    assert_eq!(
        MeasuredProperties::new(Some(f64::NAN), None, None, None, None)
            .expect_err("volume")
            .message(),
        "provider volume_mm3 must be a finite number"
    );
    assert_eq!(
        MeasuredProperties::new(None, Some(f64::INFINITY), None, None, None)
            .expect_err("area")
            .message(),
        "provider area_mm2 must be a finite number"
    );
    assert_eq!(
        MeasuredProperties::new(None, None, None, Some(f64::NEG_INFINITY), None)
            .expect_err("mass")
            .message(),
        "provider mass_g must be a finite number"
    );
    assert_eq!(
        MeasuredProperties::new(None, None, None, None, Some([0.0, f64::NAN, 0.0]))
            .expect_err("centroid")
            .message(),
        "provider centroid_mm must be three finite numbers"
    );
    let measured = MeasuredProperties::new(
        Some(1.0),
        Some(2.0),
        Some(bounds),
        None,
        Some([0.5, 0.5, 0.5]),
    )
    .expect("valid properties");
    assert_eq!(measured.volume_mm3(), Some(1.0));
    assert_eq!(measured.area_mm2(), Some(2.0));
    assert_eq!(measured.centroid_mm(), Some([0.5, 0.5, 0.5]));
    assert_eq!(MeasuredProperties::default().volume_mm3(), None);

    assert_eq!(
        ProviderBuild::new(
            Vec::new(),
            MeasuredProperties::default(),
            None,
            Vec::new(),
            None,
            None,
            None,
            None,
        )
        .expect_err("empty STEP")
        .message(),
        "a provider build must return STEP bytes"
    );
    assert_eq!(
        ProviderBuild::new(
            b"ISO-10303-21".to_vec(),
            MeasuredProperties::default(),
            None,
            Vec::new(),
            None,
            Some(String::new()),
            None,
            None,
        )
        .expect_err("empty kind")
        .message(),
        "a native artifact kind must not be empty"
    );
    assert_eq!(
        ProviderBuild::new(
            b"ISO-10303-21".to_vec(),
            MeasuredProperties::default(),
            None,
            Vec::new(),
            None,
            None,
            None,
            Some(-1),
        )
        .expect_err("negative solids")
        .message(),
        "a solid count must be non-negative"
    );
    let build = ProviderBuild::new(
        b"ISO-10303-21".to_vec(),
        MeasuredProperties::default(),
        Some("v".repeat(100)),
        vec!["w".repeat(400); 20],
        Some(b"native".to_vec()),
        Some("fcstd".to_owned()),
        Some(false),
        Some(0),
    )
    .expect("a valid build");
    assert_eq!(
        build.provider_version().expect("version").chars().count(),
        MAX_VERSION_LENGTH
    );
    assert_eq!(build.warnings().len(), MAX_WARNINGS - 1);
    for warning in &build.warnings()[..MAX_WARNINGS - 2] {
        assert_eq!(warning.chars().count(), MAX_WARNING_LENGTH);
    }
    assert_eq!(build.warnings()[MAX_WARNINGS - 2], "… and 6 more warnings");
    assert_eq!(build.solid_count(), Some(0));
    assert_eq!(build.valid(), Some(false));

    let error = CadProviderBuildError::new("x".repeat(1000));
    assert_eq!(error.message().chars().count(), MAX_REASON_LENGTH);
    let error = CadProviderBuildError::with_op("refused", "fuse");
    assert_eq!(error.op(), Some("fuse"));
    let error = CadValueError::new("a value error");
    assert_eq!(error.to_string(), "a value error");
}

/// Create an adapter source directory with one `site-packages-cpNNN` sibling
/// per supported ABI. Returns `.../adapters/freecad/src`.
fn bundled_adapter_fixture(root: &Path) -> std::path::PathBuf {
    let adapter = root.join("adapters").join("freecad");
    let source = adapter.join("src");
    std::fs::create_dir_all(source.join("kala_agent_freecad")).expect("the adapter source");
    std::fs::write(source.join("kala_agent_freecad").join("__init__.py"), "")
        .expect("the adapter module");
    std::fs::write(source.join("kala_agent_freecad").join("worker.py"), "")
        .expect("the adapter worker");
    for abi in SUPPORTED_FREECAD_PYTHON_ABIS {
        std::fs::create_dir_all(adapter.join(format!("site-packages-{abi}")))
            .expect("the per-ABI directory");
    }
    source
}

#[test]
fn every_supported_abi_selects_only_its_own_directory() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let source = bundled_adapter_fixture(directory.path());
    let adapter = directory.path().join("adapters").join("freecad");

    assert_eq!(
        bundled_python_abis(&source),
        SUPPORTED_FREECAD_PYTHON_ABIS
            .iter()
            .map(|abi| (*abi).to_owned())
            .collect::<Vec<String>>()
    );
    for abi in SUPPORTED_FREECAD_PYTHON_ABIS {
        match select_bundled_python_abi(&source, abi) {
            BundledPythonSelection::Matched {
                abi: selected,
                directory,
            } => {
                assert_eq!(selected, abi);
                assert_eq!(directory, adapter.join(format!("site-packages-{abi}")));
            }
            other => panic!("{abi} was not selected: {other:?}"),
        }
    }
    assert_eq!(
        select_bundled_python_abi(&source, "cp310"),
        BundledPythonSelection::Unsupported {
            detected: "cp310".to_owned(),
            supported: SUPPORTED_FREECAD_PYTHON_ABIS
                .iter()
                .map(|abi| (*abi).to_owned())
                .collect(),
        }
    );
    assert_eq!(
        select_bundled_python_abi(directory.path(), "cp314"),
        BundledPythonSelection::Unbundled
    );
}

#[test]
fn the_build_puts_only_the_matching_abi_directory_on_sys_path() {
    let (directory, executable) = fake_freecad();
    let source = bundled_adapter_fixture(directory.path());
    let adapter = directory.path().join("adapters").join("freecad");
    let separator = if cfg!(windows) { ';' } else { ':' };
    for abi in SUPPORTED_FREECAD_PYTHON_ABIS {
        let dump = directory.path().join(format!("environment-{abi}.json"));
        let mut environment = support::fake_environment("success");
        environment.insert("FAKE_FREECAD_PYTHON_ABI".to_owned(), abi.to_owned());
        environment.insert(
            "FAKE_FREECAD_PYTHON_VERSION".to_owned(),
            format!("3.{}.8", &abi[3..]),
        );
        environment.insert(
            "FAKE_FREECAD_DUMP".to_owned(),
            dump.to_string_lossy().into_owned(),
        );
        let mut provider = FreeCadProvider::new(FreeCadProviderOptions {
            executable: Some(executable.clone()),
            probe_cache_seconds: Some(0.0),
            environ: Some(environment),
            sys_path: Some(vec![source.clone()]),
            ..FreeCadProviderOptions::default()
        })
        .expect("a valid provider");
        assert!(provider.probe().is_available(), "{abi} must be available");
        build(&mut provider).expect("the fake build succeeds");

        let record = kala_json::strict_json_loads(&std::fs::read(&dump).expect("the env dump"))
            .expect("the dump is strict JSON");
        let observed = match &record {
            kala_json::CanonicalValue::Object(object) => object,
            _ => panic!("the environment dump is not an object"),
        };
        let sys_path = observed
            .get(&kala_json::JsonText::new("KALA_FREECAD_SYS_PATH"))
            .and_then(kala_json::CanonicalValue::as_text)
            .map(kala_json::JsonText::to_string_lossy)
            .expect("KALA_FREECAD_SYS_PATH");
        let expected = format!(
            "{}{separator}{}",
            source.display(),
            adapter.join(format!("site-packages-{abi}")).display()
        );
        assert_eq!(sys_path, expected, "{abi} selected the wrong sys.path");
        let entries: Vec<&str> = sys_path.split(separator).collect();
        assert_eq!(entries.len(), 2, "{abi} put extra directories on sys.path");
    }
}

#[test]
fn an_unsupported_python_reports_an_honest_unavailable_reason() {
    let (directory, executable) = fake_freecad();
    let source = bundled_adapter_fixture(directory.path());
    let mut environment = support::fake_environment("success");
    environment.insert("FAKE_FREECAD_PYTHON_ABI".to_owned(), "cp310".to_owned());
    environment.insert(
        "FAKE_FREECAD_PYTHON_VERSION".to_owned(),
        "3.10.14".to_owned(),
    );
    let mut provider = FreeCadProvider::new(FreeCadProviderOptions {
        executable: Some(executable),
        probe_cache_seconds: Some(0.0),
        environ: Some(environment),
        sys_path: Some(vec![source]),
        ..FreeCadProviderOptions::default()
    })
    .expect("a valid provider");
    let availability = provider.probe();
    assert!(!availability.is_available());
    let reason = availability.reason().expect("a reason");
    assert!(
        reason.contains("3.10.14"),
        "the version is not named: {reason}"
    );
    assert!(
        reason.contains("cp310"),
        "the detected ABI is not named: {reason}"
    );
    for abi in SUPPORTED_FREECAD_PYTHON_ABIS {
        assert!(
            reason.contains(abi),
            "the supported {abi} is not named: {reason}"
        );
    }

    let error = provider
        .build(&pad_script())
        .expect_err("an unsupported interpreter cannot build");
    let unavailable = error.unavailable().expect("an unavailable error");
    assert!(
        unavailable.reason().contains("3.10.14"),
        "the build refusal does not name the interpreter: {}",
        unavailable.reason()
    );
}

#[test]
fn an_executable_path_with_spaces_is_never_shell_split() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let spaced = directory.path().join("free cad");
    std::fs::create_dir_all(&spaced).expect("the spaced directory");
    let executable = spaced.join("FreeCADCmd");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_freecad.py"),
        &executable,
    )
    .expect("copy the fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))
            .expect("mark executable");
    }
    let mut provider = fake_provider(&executable, "success");
    let build = build(&mut provider).expect("the spaced executable runs");
    assert!(build.step_bytes().starts_with(b"ISO-10303-21"));
}
