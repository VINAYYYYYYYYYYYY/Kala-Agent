//! Live differential against the repository's Python FreeCAD provider.
//!
//! The oracle runs the repository's own `FreeCadProvider` in an isolated
//! interpreter against a real `FreeCADCmd`, builds the shared parity corpus
//! plus a deterministic randomized corpus, imports every STEP it produced and
//! every STEP this crate produced, and prints the observed facts. This test
//! compares the measured values, the refusal messages and operations, the
//! script hand-off bytes, and the imported STEP bounds. The live adapter accepts
//! every corpus script, so the live run has no refusal; refusal parsing and op
//! naming are exercised by `freecad_unit.rs` against a fake executable.
//!
//! The test skips only when no `FreeCADCmd` is installed on the host. It never
//! substitutes a fake build for a missing FreeCAD.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use kala_cad_providers::{
    CadProviderError, FreeCadProvider, FreeCadProviderOptions, ProviderBuild,
    canonical_request_json,
};
use kala_contracts::{ConstructionScript, load_construction};
use kala_json::{
    CanonicalObject, CanonicalValue, JsonText, Limits, canonical_dumps, strict_json_loads,
};

const VOLUME_RELATIVE_TOLERANCE: f64 = 1e-9;
const BOUNDS_ABSOLUTE_TOLERANCE_MM: f64 = 1e-6;

struct Case {
    id: String,
    script_value: CanonicalValue,
    script: ConstructionScript,
}

struct OracleBuild {
    provider_version: Option<String>,
    volume_mm3: Option<f64>,
    area_mm2: Option<f64>,
    bounds: Option<([f64; 3], [f64; 3])>,
    mass_g: Option<f64>,
    centroid_mm: Option<[f64; 3]>,
    warnings: Vec<String>,
    valid: Option<bool>,
    solid_count: Option<i64>,
    native_kind: Option<String>,
    native_present: bool,
    step_size: usize,
    imported: Option<([f64; 3], [f64; 3], i64)>,
    rust_imported: Option<([f64; 3], [f64; 3], i64)>,
    import_error: Option<String>,
}

enum OracleOutcome {
    Refused { message: String, op: Option<String> },
    Built(Box<OracleBuild>),
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

fn as_float(value: Option<&CanonicalValue>) -> Option<f64> {
    match value? {
        CanonicalValue::Float(number) => Some(*number),
        CanonicalValue::Integer(number) => num_traits::ToPrimitive::to_f64(number),
        _ => None,
    }
}

fn as_bool(value: Option<&CanonicalValue>) -> Option<bool> {
    match value? {
        CanonicalValue::Bool(value) => Some(*value),
        _ => None,
    }
}

fn as_usize(value: Option<&CanonicalValue>) -> Option<usize> {
    match value? {
        CanonicalValue::Integer(number) => num_traits::ToPrimitive::to_usize(number),
        _ => None,
    }
}

fn as_i64(value: Option<&CanonicalValue>) -> Option<i64> {
    match value? {
        CanonicalValue::Integer(number) => num_traits::ToPrimitive::to_i64(number),
        _ => None,
    }
}

fn as_triplet(value: Option<&CanonicalValue>) -> Option<[f64; 3]> {
    let CanonicalValue::Array(items) = value? else {
        return None;
    };
    if items.len() != 3 {
        return None;
    }
    Some([
        as_float(items.first())?,
        as_float(items.get(1))?,
        as_float(items.get(2))?,
    ])
}

fn as_imported(value: Option<&CanonicalValue>) -> Option<([f64; 3], [f64; 3], i64)> {
    let object = as_object(value?);
    Some((
        as_triplet(field(object, "min"))?,
        as_triplet(field(object, "max"))?,
        as_i64(field(object, "solids"))?,
    ))
}

fn fixture_cases() -> Vec<Case> {
    let path = repository_root().join("tests/fixtures/construction_parity.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let value = strict_json_loads(text.as_bytes()).expect("the parity fixture is strict JSON");
    let document = as_object(&value);
    as_array(field(document, "cases").expect("cases"))
        .iter()
        .map(|case| {
            let object = as_object(case);
            let id = as_text(field(object, "id")).expect("case id");
            let script_value = field(object, "script").expect("case script").clone();
            let script = load_construction(&script_value)
                .unwrap_or_else(|error| panic!("{id}: {}", error.message()));
            Case {
                id,
                script_value,
                script,
            }
        })
        .collect()
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut state = self.0;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        self.0 = state;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn int(&mut self, low: i64, high: i64) -> i64 {
        debug_assert!(low <= high);
        let span = (high - low + 1) as u64;
        low + (self.next() % span) as i64
    }

    fn pick(&mut self, choices: &[&str]) -> String {
        choices[(self.next() % choices.len() as u64) as usize].to_owned()
    }

    fn quarter(&mut self, low: i64, high: i64) -> f64 {
        self.int(low * 4, high * 4) as f64 / 4.0
    }
}

fn value_float(value: f64) -> CanonicalValue {
    CanonicalValue::float(value).expect("a finite generated value")
}

fn value_text(value: &str) -> CanonicalValue {
    CanonicalValue::text(value)
}

fn value_bool(value: bool) -> CanonicalValue {
    CanonicalValue::Bool(value)
}

fn step(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    CanonicalValue::object(entries)
}

fn component(name: &str, steps: Vec<CanonicalValue>) -> CanonicalValue {
    CanonicalValue::object([
        ("name", value_text(name)),
        ("steps", CanonicalValue::Array(steps)),
    ])
}

fn script(
    summary: &str,
    components: Vec<CanonicalValue>,
    assembly: Vec<CanonicalValue>,
) -> CanonicalValue {
    CanonicalValue::object([
        ("summary", value_text(summary)),
        ("components", CanonicalValue::Array(components)),
        ("assembly", CanonicalValue::Array(assembly)),
    ])
}

fn sketch(
    plane: &str,
    profile: &str,
    x: f64,
    y: f64,
    width: Option<f64>,
    height: Option<f64>,
    diameter: Option<f64>,
) -> CanonicalValue {
    let mut entries = vec![
        ("op", value_text("sketch")),
        ("plane", value_text(plane)),
        ("profile", value_text(profile)),
        ("x", value_float(x)),
        ("y", value_float(y)),
    ];
    if let Some(width) = width {
        entries.push(("width", value_float(width)));
    }
    if let Some(height) = height {
        entries.push(("height", value_float(height)));
    }
    if let Some(diameter) = diameter {
        entries.push(("diameter", value_float(diameter)));
    }
    step(entries)
}

fn extrude(length: f64, reversed: bool) -> CanonicalValue {
    step(vec![
        ("op", value_text("extrude")),
        ("length", value_float(length)),
        ("reversed", value_bool(reversed)),
    ])
}

fn place(name: &str, x: f64, y: f64, z: f64) -> CanonicalValue {
    step(vec![
        ("op", value_text("place")),
        ("component", value_text(name)),
        ("x", value_float(x)),
        ("y", value_float(y)),
        ("z", value_float(z)),
    ])
}

fn fuse(names: &[&str]) -> CanonicalValue {
    step(vec![
        ("op", value_text("fuse")),
        (
            "components",
            CanonicalValue::Array(names.iter().map(|name| value_text(name)).collect()),
        ),
    ])
}

fn random_pad(rng: &mut Rng, index: usize) -> CanonicalValue {
    let plane = rng.pick(&["XY", "XZ", "YZ"]);
    let circle = rng.int(0, 2) == 0;
    let width = rng.quarter(2, 18);
    let height = rng.quarter(2, 18);
    let x = rng.quarter(-8, 8);
    let y = rng.quarter(-8, 8);
    let length = rng.quarter(1, 14);
    let reversed = rng.int(0, 4) == 0;
    let profile = if circle {
        sketch(&plane, "circle", x, y, None, None, Some(width))
    } else {
        sketch(&plane, "rectangle", x, y, Some(width), Some(height), None)
    };
    script(
        &format!("random pad {index}"),
        vec![component("body", vec![profile, extrude(length, reversed)])],
        Vec::new(),
    )
}

fn random_revolve(rng: &mut Rng, index: usize) -> CanonicalValue {
    let x = rng.quarter(2, 6);
    let width = rng.quarter(1, 3);
    let height = rng.quarter(1, 3);
    let sketch = sketch("XY", "rectangle", x, 0.0, Some(width), Some(height), None);
    let revolve = step(vec![
        ("op", value_text("revolve")),
        ("plane", value_text("XY")),
        ("angle", value_float(360.0)),
    ]);
    script(
        &format!("random revolve {index}"),
        vec![component("body", vec![sketch, revolve])],
        Vec::new(),
    )
}

fn random_pocket(rng: &mut Rng, index: usize) -> CanonicalValue {
    let width = rng.quarter(24, 36);
    let height = rng.quarter(24, 36);
    let diameter = rng.quarter(2, 3);
    let offset = rng.quarter(6, 8);
    let depth = rng.quarter(3, 6);
    script(
        &format!("random pocket {index}"),
        vec![component(
            "body",
            vec![
                sketch("XY", "rectangle", 0.0, 0.0, Some(width), Some(height), None),
                extrude(10.0, false),
                sketch("XY", "circle", offset, offset, None, None, Some(diameter)),
                step(vec![
                    ("op", value_text("pocket")),
                    ("length", value_float(depth)),
                ]),
            ],
        )],
        Vec::new(),
    )
}

fn random_placed(rng: &mut Rng, index: usize) -> CanonicalValue {
    let pad = random_pad(rng, 900 + index);
    let object = as_object(&pad);
    let components = field(object, "components").expect("components").clone();
    let mut x = rng.quarter(-4, 4);
    let y = rng.quarter(-4, 4);
    let z = rng.quarter(0, 4);
    if x == 0.0 && y == 0.0 && z == 0.0 {
        x = 1.0;
    }
    script(
        &format!("random placed {index}"),
        as_array(&components).to_vec(),
        vec![place("body", x, y, z)],
    )
}

fn random_place_only_assembly(rng: &mut Rng, index: usize) -> CanonicalValue {
    let count = rng.int(2, 4) as usize;
    let mut components = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut assembly = Vec::new();
    for component_index in 0..count {
        let name = format!("c{component_index}");
        components.push(component(
            &name,
            vec![
                sketch(
                    "XY",
                    "rectangle",
                    0.0,
                    0.0,
                    Some(rng.quarter(4, 10)),
                    Some(rng.quarter(4, 10)),
                    None,
                ),
                extrude(rng.quarter(1, 4), false),
            ],
        ));
        names.push(name);
    }
    for (component_index, name) in names.iter().enumerate() {
        assembly.push(place(
            name,
            rng.quarter(0, 3) + component_index as f64 * 12.0,
            0.0,
            0.0,
        ));
    }
    script(
        &format!("random place-only assembly {index}"),
        components,
        assembly,
    )
}

fn random_fuse_assembly(_rng: &mut Rng, index: usize) -> CanonicalValue {
    script(
        &format!("random fuse refusal {index}"),
        vec![
            component(
                "a",
                vec![
                    sketch("XY", "rectangle", 0.0, 0.0, Some(10.0), Some(10.0), None),
                    extrude(5.0, false),
                ],
            ),
            component(
                "b",
                vec![
                    sketch("XY", "rectangle", 0.0, 0.0, Some(10.0), Some(10.0), None),
                    extrude(5.0, false),
                ],
            ),
        ],
        vec![
            place("a", 0.0, 0.0, 0.0),
            place("b", 40.0, 0.0, 0.0),
            fuse(&["a", "b"]),
        ],
    )
}

fn random_cases() -> Vec<Case> {
    let mut rng = Rng(0x0BAD_C0DE_1234_5678);
    let mut scripts: Vec<(String, CanonicalValue)> = Vec::new();
    for index in 0..6 {
        scripts.push((format!("random_pad_{index}"), random_pad(&mut rng, index)));
    }
    for index in 0..2 {
        scripts.push((
            format!("random_revolve_{index}"),
            random_revolve(&mut rng, index),
        ));
    }
    for index in 0..2 {
        scripts.push((
            format!("random_pocket_{index}"),
            random_pocket(&mut rng, index),
        ));
    }
    for index in 0..2 {
        scripts.push((
            format!("random_placed_{index}"),
            random_placed(&mut rng, index),
        ));
    }
    for index in 0..2 {
        scripts.push((
            format!("random_place_only_assembly_{index}"),
            random_place_only_assembly(&mut rng, index),
        ));
    }
    scripts.push((
        "random_fuse_assembly".to_owned(),
        random_fuse_assembly(&mut rng, 0),
    ));
    scripts
        .into_iter()
        .map(|(id, script_value)| {
            let script = load_construction(&script_value).unwrap_or_else(|error| {
                panic!(
                    "{id}: {} in {}",
                    error.message(),
                    canonical_dumps(&script_value, &Limits::default()).expect("canonical")
                )
            });
            Case {
                id,
                script_value,
                script,
            }
        })
        .collect()
}

fn run_oracle(request: &CanonicalValue) -> CanonicalValue {
    let root = repository_root();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/freecad_oracle.py");
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
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "the Python oracle failed: {}\nstdout: {}",
        String::from_utf8_lossy(&output.stderr),
        stdout
    );
    parse_oracle_json(&stdout)
}

fn parse_oracle_json(stdout: &str) -> CanonicalValue {
    if let Ok(value) = strict_json_loads(stdout.trim().as_bytes()) {
        return value;
    }
    for line in stdout.lines().rev() {
        let line = line.trim();
        if line.starts_with('{') {
            if let Ok(value) = strict_json_loads(line.as_bytes()) {
                return value;
            }
        }
    }
    panic!("the Python oracle did not print a JSON document: {stdout}");
}

fn oracle_cases(response: &CanonicalValue) -> BTreeMap<String, OracleOutcome> {
    let document = as_object(response);
    let mut outcomes = BTreeMap::new();
    for case in as_array(field(document, "cases").expect("cases")) {
        let object = as_object(case);
        let id = as_text(field(object, "id")).expect("case id");
        if let Some(refused) = field(object, "refused") {
            let refused = as_object(refused);
            outcomes.insert(
                id,
                OracleOutcome::Refused {
                    message: as_text(field(refused, "message")).expect("refusal message"),
                    op: as_text(field(refused, "op")),
                },
            );
        } else {
            let build = as_object(field(object, "build").expect("build"));
            let measured = as_object(field(build, "measured").expect("measured"));
            let bounds = match field(measured, "bounds") {
                Some(CanonicalValue::Null) | None => None,
                Some(value) => {
                    let object = as_object(value);
                    Some((
                        as_triplet(field(object, "min")).expect("bounds min"),
                        as_triplet(field(object, "max")).expect("bounds max"),
                    ))
                }
            };
            outcomes.insert(
                id,
                OracleOutcome::Built(Box::new(OracleBuild {
                    provider_version: as_text(field(build, "provider_version")),
                    volume_mm3: as_float(field(measured, "volume_mm3")),
                    area_mm2: as_float(field(measured, "area_mm2")),
                    bounds,
                    mass_g: as_float(field(measured, "mass_g")),
                    centroid_mm: as_triplet(field(measured, "centroid_mm")),
                    warnings: as_array(field(build, "warnings").expect("warnings"))
                        .iter()
                        .filter_map(|warning| as_text(Some(warning)))
                        .collect(),
                    valid: as_bool(field(build, "valid")),
                    solid_count: as_i64(field(build, "solid_count")),
                    native_kind: as_text(field(build, "native_kind")),
                    native_present: as_bool(field(build, "native_present")).expect("native"),
                    step_size: as_usize(field(build, "step_size")).expect("step size"),
                    imported: as_imported(field(build, "imported")),
                    rust_imported: as_imported(field(build, "rust_imported")),
                    import_error: as_text(field(build, "import_error")),
                })),
            );
        }
    }
    outcomes
}

fn close(left: f64, right: f64, absolute: f64, relative: f64) -> bool {
    if left == right {
        return true;
    }
    let difference = (left - right).abs();
    difference <= absolute || difference <= relative * left.abs().max(right.abs())
}

fn compare_triplet(case: &str, axis: &str, rust: [f64; 3], oracle: [f64; 3]) {
    for index in 0..3 {
        assert!(
            close(
                rust[index],
                oracle[index],
                BOUNDS_ABSOLUTE_TOLERANCE_MM,
                1e-9
            ),
            "{case}: {axis} axis {index} differs: rust {rust:?}, python {oracle:?}"
        );
    }
}

fn compare_build(case: &str, rust: &ProviderBuild, oracle: &OracleBuild) {
    assert!(
        oracle.import_error.is_none(),
        "{case}: the Python importer failed: {:?}",
        oracle.import_error
    );
    assert_eq!(
        rust.provider_version(),
        oracle.provider_version.as_deref(),
        "{case}: provider version"
    );
    assert_eq!(
        rust.measured().volume_mm3(),
        oracle.volume_mm3,
        "{case}: volume"
    );
    assert_eq!(rust.measured().area_mm2(), oracle.area_mm2, "{case}: area");
    assert_eq!(rust.measured().mass_g(), oracle.mass_g, "{case}: mass");
    assert_eq!(
        rust.measured().centroid_mm(),
        oracle.centroid_mm,
        "{case}: centroid"
    );
    match (rust.measured().bounds(), oracle.bounds) {
        (None, None) => {}
        (Some(rust_bounds), Some((minimum, maximum))) => {
            compare_triplet(case, "minimum", rust_bounds.min_mm(), minimum);
            compare_triplet(case, "maximum", rust_bounds.max_mm(), maximum);
        }
        (rust_bounds, oracle_bounds) => panic!(
            "{case}: one side reported bounds and the other did not: rust {rust_bounds:?}, python {oracle_bounds:?}"
        ),
    }
    assert_eq!(rust.warnings(), oracle.warnings, "{case}: warnings");
    assert_eq!(rust.valid(), oracle.valid, "{case}: valid");
    assert_eq!(
        rust.solid_count(),
        oracle.solid_count,
        "{case}: solid count"
    );
    assert_eq!(
        rust.native_kind(),
        oracle.native_kind.as_deref(),
        "{case}: native kind"
    );
    assert_eq!(
        rust.native_bytes().is_some(),
        oracle.native_present,
        "{case}: native artifact presence"
    );
    assert!(
        rust.step_bytes().starts_with(b"ISO-10303-21"),
        "{case}: the Rust STEP does not start with ISO-10303-21"
    );
    assert!(
        rust.native_bytes()
            .is_some_and(|bytes| bytes.starts_with(b"PK")),
        "{case}: the Rust FCStd is not a zip archive"
    );
    assert!(
        oracle.step_size > 0,
        "{case}: the Python provider exported no STEP bytes"
    );
    let (rust_minimum, rust_maximum, rust_solids) = oracle
        .rust_imported
        .expect("the oracle did not import the Rust STEP");
    let (oracle_minimum, oracle_maximum, oracle_solids) = oracle
        .imported
        .expect("the oracle did not import its own STEP");
    compare_triplet(case, "imported minimum", rust_minimum, oracle_minimum);
    compare_triplet(case, "imported maximum", rust_maximum, oracle_maximum);
    assert_eq!(
        rust_solids, oracle_solids,
        "{case}: the imported STEP solid counts differ"
    );
    let volume = rust.measured().volume_mm3().expect("a volume");
    if let Some(oracle_volume) = oracle.volume_mm3 {
        assert!(
            close(volume, oracle_volume, 0.0, VOLUME_RELATIVE_TOLERANCE),
            "{case}: volume outside the differential tolerance"
        );
    }
}

#[test]
fn freecad_provider_matches_the_live_python_provider() {
    let options = FreeCadProviderOptions {
        probe_cache_seconds: Some(0.0),
        ..FreeCadProviderOptions::default()
    };
    let mut provider = FreeCadProvider::new(options).expect("the default configuration is valid");
    let availability = provider.probe();
    if !availability.is_available() {
        eprintln!(
            "skipping the FreeCAD differential: {}",
            availability.reason().unwrap_or("FreeCADCmd is unavailable")
        );
        return;
    }

    let fixture = fixture_cases();
    let random = random_cases();
    let mut cases: Vec<Case> = Vec::new();
    cases.extend(fixture);
    cases.extend(random);
    let temporary = tempfile::tempdir().expect("a temporary directory");
    let mut request_cases: Vec<CanonicalValue> = Vec::new();
    type RustOutcome = (
        Case,
        Result<ProviderBuild, CadProviderError>,
        Option<PathBuf>,
    );
    let mut rust_outcomes: BTreeMap<String, RustOutcome> = BTreeMap::new();
    for case in cases {
        let outcome = provider.build(&case.script);
        let step_path = match &outcome {
            Ok(build) => {
                let path = temporary.path().join(format!("{}.step", case.id));
                std::fs::write(&path, build.step_bytes()).expect("write the Rust STEP");
                Some(path)
            }
            Err(_) => None,
        };
        request_cases.push(CanonicalValue::object([
            ("id", value_text(&case.id)),
            ("script", case.script_value.clone()),
            (
                "rust_step_path",
                match &step_path {
                    Some(path) => value_text(&path.to_string_lossy()),
                    None => CanonicalValue::Null,
                },
            ),
        ]));
        rust_outcomes.insert(case.id.clone(), (case, outcome, step_path));
    }
    let request = CanonicalValue::object([
        ("cases", CanonicalValue::Array(request_cases)),
        ("include_driver_source", CanonicalValue::Bool(true)),
    ]);
    let response = run_oracle(&request);
    let document = as_object(&response);
    let driver_source = as_text(field(document, "driver_source")).expect("driver source");
    assert_eq!(
        driver_source,
        kala_cad_providers::DRIVER_SOURCE,
        "the embedded driver is not the Python provider's driver"
    );
    let outcomes = oracle_cases(&response);
    assert_eq!(
        outcomes.len(),
        rust_outcomes.len(),
        "the oracle omitted a case"
    );
    let mut supported = 0;
    let mut refused = 0;
    let mut fixture_supported = 0;
    let mut fixture_refused = 0;
    for (id, (case, rust, _)) in &rust_outcomes {
        let oracle = outcomes.get(id).expect("a matching oracle case");
        match (rust, oracle) {
            (Ok(build), OracleOutcome::Built(oracle)) => {
                compare_build(id, build, oracle);
                supported += 1;
                if !id.starts_with("random_") {
                    fixture_supported += 1;
                }
            }
            (Err(error), OracleOutcome::Refused { message, op }) => {
                assert_eq!(&error.message(), message, "{id}: refusal message");
                assert_eq!(error.op(), op.as_deref(), "{id}: refusal op");
                refused += 1;
                if !id.starts_with("random_") {
                    fixture_refused += 1;
                }
            }
            (Ok(_), OracleOutcome::Refused { message, op }) => {
                panic!("{id}: Rust built but Python refused: {message} (op {op:?})")
            }
            (Err(error), OracleOutcome::Built(_)) => {
                panic!("{id}: Rust refused but Python built: {}", error.message())
            }
        }
        // Every script, supported or refused, has the same hand-off bytes.
        assert_eq!(
            canonical_request_json(&case.script).expect("canonical request bytes"),
            oracle_request_json(&response, id),
            "{id}: canonical request bytes"
        );
    }
    assert_eq!(
        fixture_supported, 28,
        "the corpus supported-case count changed"
    );
    assert_eq!(fixture_refused, 0, "the adapter refused a corpus case");
    eprintln!(
        "FreeCAD differential: {supported} builds and {refused} refusals matched; {} corpus cases, {} randomized cases",
        fixture_cases().len(),
        random_cases().len()
    );
}

fn oracle_request_json(response: &CanonicalValue, id: &str) -> String {
    let document = as_object(response);
    for case in as_array(field(document, "cases").expect("cases")) {
        let object = as_object(case);
        if as_text(field(object, "id")).as_deref() == Some(id) {
            return as_text(field(object, "request_json")).expect("request json");
        }
    }
    panic!("the oracle omitted {id}");
}
