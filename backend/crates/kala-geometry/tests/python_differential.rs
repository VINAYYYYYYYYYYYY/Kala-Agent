//! Live Python differential test.
//!
//! This test requires the repository virtualenv (or `KALA_GEOMETRY_PYTHON`) and
//! the product source tree. Python is an oracle only: the Rust library never
//! spawns a process. If no usable oracle is available the test fails rather
//! than silently skipping, except for an explicit opt-out
//! (`KALA_SKIP_PYTHON_DIFFERENTIAL=1`) used by offline environments where the
//! fixture gate still runs.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{as_array, as_str, as_u64, f64_bits_hex, load_json, repo_root, sha256_hex};
use kala_geometry::{
    Mesh, PrimitiveParameters, build_primitive, compute_bounds, signed_volume, write_binary_stl,
};
use serde_json::{Value, json};

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next_u64(&mut self) -> u64 {
        let mut state = self.0;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        self.0 = state;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / ((1_u64 << 53) as f64)
    }

    fn dimension(&mut self) -> f64 {
        0.1 + self.unit() * (2000.0 - 0.1)
    }

    fn count(&mut self, minimum: u32, maximum: u32) -> u32 {
        minimum + (self.next_u64() % u64::from(maximum - minimum + 1)) as u32
    }
}

fn build_fixture_mesh(case: &Value) -> Mesh {
    let kind = as_str(&case["kind"]);
    let parameters = PrimitiveParameters::from_json_value(&case["parameters"]).unwrap();
    build_primitive(kind, &parameters).unwrap()
}

fn canonical_input(fixture: &Value) -> (Value, Vec<(String, String)>) {
    let mut arrays = Vec::new();
    let mut expected = Vec::new();
    for case in as_array(&fixture["meshes"]) {
        let mesh = build_fixture_mesh(case);
        let id = as_str(&case["id"]);
        for (suffix, bits, expected_digest) in [
            (
                "positions",
                mesh.positions()
                    .iter()
                    .map(|value| f64_bits_hex(*value))
                    .collect::<Vec<_>>(),
                as_str(&case["positions"]["sha256_of_compact_json"]).to_owned(),
            ),
            (
                "normals",
                mesh.normals()
                    .iter()
                    .map(|value| f64_bits_hex(*value))
                    .collect::<Vec<_>>(),
                as_str(&case["normals"]["sha256_of_compact_json"]).to_owned(),
            ),
        ] {
            let array_id = format!("{id}.{suffix}");
            arrays.push(json!({"id": array_id, "bits": bits}));
            expected.push((array_id, expected_digest));
        }
    }

    let mut floats = Vec::new();
    let fixed = [
        f64::MIN_POSITIVE,
        5e-324,
        f64::MAX,
        -0.0,
        0.0,
        1.0,
        1e-5,
        1e-4,
        1e15,
        1e16,
        1e-7,
        0.1,
        1.234_567_890_123_456_7,
        -1.224_646_799_147_353_3e-15,
    ];
    for value in fixed {
        floats.push(json!({"bits": f64_bits_hex(value), "repr": common::python_float_repr(value)}));
    }
    let mut rng = Rng::new(0x666c_6f61_7472_6570);
    let mut generated = 0;
    while generated < 5000 {
        let value = f64::from_bits(rng.next_u64());
        if !value.is_finite() {
            continue;
        }
        floats.push(json!({"bits": f64_bits_hex(value), "repr": common::python_float_repr(value)}));
        generated += 1;
    }

    (json!({"arrays": arrays, "floats": floats}), expected)
}

fn mesh_cases() -> Vec<Value> {
    let mut cases = vec![
        json!({"id": "box_default", "kind": "box", "parameters": {}}),
        json!({"id": "box_small_tall", "kind": "box", "parameters": {
            "size_x": 0.1, "size_y": 2000.0, "size_z": 123.45678901234567, "subdivisions": 2
        }}),
        json!({"id": "box_flat", "kind": "box", "parameters": {
            "size_x": 2000.0, "size_y": 0.1, "size_z": 0.1, "subdivisions": 5
        }}),
        json!({"id": "cylinder_default", "kind": "cylinder", "parameters": {}}),
        json!({"id": "cylinder_min_counts", "kind": "cylinder", "parameters": {
            "radius": 0.1, "height": 2000.0, "radial_segments": 6, "height_segments": 1
        }}),
        json!({"id": "cylinder_tall_segments", "kind": "cylinder", "parameters": {
            "radius": 1234.5, "height": 0.1, "radial_segments": 7, "height_segments": 64
        }}),
        json!({"id": "cylinder_max", "kind": "cylinder", "parameters": {
            "radius": 5.0, "height": 10.0, "radial_segments": 256, "height_segments": 64
        }}),
        json!({"id": "sphere_default", "kind": "sphere", "parameters": {}}),
        json!({"id": "sphere_min_counts", "kind": "sphere", "parameters": {
            "radius": 0.1, "rings": 3, "sectors": 4
        }}),
        json!({"id": "sphere_odd", "kind": "sphere", "parameters": {
            "radius": 2000.0, "rings": 5, "sectors": 7
        }}),
        json!({"id": "sphere_max", "kind": "sphere", "parameters": {
            "radius": 123.45678901234567, "rings": 96, "sectors": 128
        }}),
    ];
    let mut rng = Rng::new(0x7261_6e64_6f6d_7368);
    for index in 0..6 {
        cases.push(json!({
            "id": format!("random_{index}"),
            "kind": "box",
            "parameters": {
                "size_x": rng.dimension(),
                "size_y": rng.dimension(),
                "size_z": rng.dimension(),
                "subdivisions": rng.count(1, 4),
            }
        }));
        cases.push(json!({
            "id": format!("random_cylinder_{index}"),
            "kind": "cylinder",
            "parameters": {
                "radius": rng.dimension(),
                "height": rng.dimension(),
                "radial_segments": rng.count(6, 12),
                "height_segments": rng.count(1, 3),
            }
        }));
        cases.push(json!({
            "id": format!("random_sphere_{index}"),
            "kind": "sphere",
            "parameters": {
                "radius": rng.dimension(),
                "rings": rng.count(3, 8),
                "sectors": rng.count(4, 10),
            }
        }));
    }
    cases
}

fn invalid_cases() -> Vec<Value> {
    vec![
        json!({"id": "empty_kind", "kind": "", "parameters": {}}),
        json!({"id": "unknown_kind", "kind": "torus", "parameters": {}}),
        json!({"id": "bool_dimension", "kind": "box", "parameters": {"size_x": true}}),
        json!({"id": "string_dimension", "kind": "box", "parameters": {"size_x": "10"}}),
        json!({"id": "null_dimension", "kind": "box", "parameters": {"size_y": null}}),
        json!({"id": "low_dimension", "kind": "box", "parameters": {"size_x": 0.01}}),
        json!({"id": "high_dimension", "kind": "box", "parameters": {"size_z": 2000.01}}),
        json!({"id": "float_count", "kind": "box", "parameters": {"subdivisions": 2.0}}),
        json!({"id": "bool_count", "kind": "box", "parameters": {"subdivisions": true}}),
        json!({"id": "low_count", "kind": "box", "parameters": {"subdivisions": 0}}),
        json!({"id": "high_count", "kind": "box", "parameters": {"subdivisions": 65}}),
        json!({"id": "unknown_box_parameter", "kind": "box", "parameters": {"radius": 5}}),
        json!({"id": "unknown_cylinder_parameter", "kind": "cylinder", "parameters": {"size_x": 5}}),
        json!({"id": "unknown_sphere_parameter", "kind": "sphere", "parameters": {"radial_segments": 8}}),
        json!({"id": "low_radial", "kind": "cylinder", "parameters": {"radial_segments": 5}}),
        json!({"id": "high_height_segments", "kind": "cylinder", "parameters": {"height_segments": 65}}),
        json!({"id": "low_rings", "kind": "sphere", "parameters": {"rings": 2}}),
        json!({"id": "high_sectors", "kind": "sphere", "parameters": {"sectors": 129}}),
    ]
}

fn python_executable(root: &Path) -> PathBuf {
    if let Ok(configured) = std::env::var("KALA_GEOMETRY_PYTHON") {
        return PathBuf::from(configured);
    }
    for candidate in [".venv/bin/python", ".venv/Scripts/python.exe"] {
        let path = root.join(candidate);
        if path.is_file() {
            return path;
        }
    }
    PathBuf::from("python3")
}

fn bits(value: &Value) -> Vec<String> {
    as_array(value)
        .iter()
        .map(|entry| as_str(entry).to_owned())
        .collect()
}

fn assert_bits(label: &str, values: &[f64], expected: &Value) {
    let expected = bits(expected);
    let actual: Vec<String> = values.iter().map(|value| f64_bits_hex(*value)).collect();
    assert_eq!(actual, expected, "{label} exact float bits");
}

#[test]
fn live_python_matches_rust_meshes_stl_canonical_json_and_errors() {
    if std::env::var_os("KALA_SKIP_PYTHON_DIFFERENTIAL").is_some() {
        eprintln!("KALA_SKIP_PYTHON_DIFFERENTIAL is set; fixture gate remains authoritative");
        return;
    }

    let root = repo_root();
    let fixture = load_json("geometry_stl.json");
    let (canonical, expected_digests) = canonical_input(&fixture);
    let meshes = mesh_cases();
    let invalid = invalid_cases();

    let request = json!({
        "canonical": canonical,
        "meshes": meshes,
        "invalid": invalid,
    });

    let workdir =
        std::env::temp_dir().join(format!("kala-geometry-python-diff-{}", std::process::id()));
    fs::create_dir_all(&workdir).unwrap();
    let input_path = workdir.join("input.json");
    let output_path = workdir.join("output.json");
    fs::write(&input_path, serde_json::to_vec(&request).unwrap()).unwrap();

    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/geometry_oracle.py");
    let python = python_executable(&root);
    let output = Command::new(&python)
        .arg(&script)
        .arg(&input_path)
        .arg(&output_path)
        .current_dir(&root)
        .env("PYTHONPATH", root.join("src"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap_or_else(|error| panic!("failed to start {}: {error}", python.display()));
    assert!(
        output.status.success(),
        "Python oracle failed with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let response: Value = serde_json::from_slice(&fs::read(&output_path).unwrap())
        .expect("oracle output must be valid JSON");

    // 1. CPython's own json.dumps must reproduce every fixture compact digest
    //    from the arrays Rust emitted.
    for (array_id, expected_digest) in expected_digests {
        let actual = as_str(&response["canonical"]["arrays"][array_id.as_str()]).to_owned();
        assert_eq!(
            actual, expected_digest,
            "{array_id} Python canonical digest"
        );
    }

    // 2. CPython repr(float) must agree with the test-only Rust formatter.
    let mismatches = as_array(&response["canonical"]["float_mismatches"]);
    assert!(
        mismatches.is_empty(),
        "float repr mismatches (first 5): {:?}",
        &mismatches[..mismatches.len().min(5)]
    );

    // 3. Every live Python mesh must match Rust bit-for-bit, including STL.
    for case in &meshes {
        let id = as_str(&case["id"]);
        let parameters = PrimitiveParameters::from_json_value(&case["parameters"]).unwrap();
        let mesh = build_primitive(as_str(&case["kind"]), &parameters)
            .unwrap_or_else(|error| panic!("{id} Rust build: {error}"));
        let oracle = &response["meshes"][id];

        assert_bits(
            &format!("{id} positions"),
            mesh.positions(),
            &oracle["positions"],
        );
        assert_bits(&format!("{id} normals"), mesh.normals(), &oracle["normals"]);

        let expected_indices: Vec<u32> = as_array(&oracle["indices"])
            .iter()
            .map(|value| as_u64(value) as u32)
            .collect();
        assert_eq!(mesh.indices(), expected_indices.as_slice(), "{id} indices");

        let stl = write_binary_stl(&mesh);
        assert_eq!(
            sha256_hex(&stl),
            as_str(&oracle["stl_sha256"]),
            "{id} STL digest"
        );
        assert_eq!(
            stl.len() as u64,
            as_u64(&oracle["stl_length"]),
            "{id} STL length"
        );

        let bounds = compute_bounds(mesh.positions()).unwrap();
        let oracle_bounds = &oracle["bounds"];
        let expected_min = bits(&oracle_bounds["min"]);
        let expected_max = bits(&oracle_bounds["max"]);
        let actual_min: Vec<String> = bounds
            .min
            .iter()
            .map(|value| f64_bits_hex(*value))
            .collect();
        let actual_max: Vec<String> = bounds
            .max
            .iter()
            .map(|value| f64_bits_hex(*value))
            .collect();
        assert_eq!(actual_min, expected_min, "{id} bounds min bits");
        assert_eq!(actual_max, expected_max, "{id} bounds max bits");

        assert_eq!(
            f64_bits_hex(signed_volume(&mesh)),
            as_str(&oracle["signed_volume"]),
            "{id} signed volume bits"
        );
    }

    // 4. Invalid requests must produce the same ValueError text.
    for case in &invalid {
        let id = as_str(&case["id"]);
        let parameters = PrimitiveParameters::from_json_value(&case["parameters"]).unwrap();
        let error = build_primitive(as_str(&case["kind"]), &parameters)
            .expect_err("invalid oracle case unexpectedly built in Rust");
        let oracle = &response["invalid"][id];
        assert_eq!(
            oracle["type"].as_str(),
            Some("ValueError"),
            "{id} Python error type"
        );
        assert_eq!(
            as_str(&oracle["message"]),
            error.message(),
            "{id} validation message"
        );
    }

    let _ = fs::remove_dir_all(&workdir);
}
