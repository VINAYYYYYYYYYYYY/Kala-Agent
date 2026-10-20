//! Small native fixture/benchmark helper for `kala-geometry`.
//!
//! This binary is not a service and makes no performance claim. It gives the
//! conductor a stable way to build fixture meshes, emit STL bytes, and run a
//! requested number of identical builds for later controlled comparisons.
//!
//! ```text
//! geometry_cli fixtures
//! geometry_cli build --kind box --parameters '{"subdivisions":2}' --out box.stl
//! geometry_cli build --kind sphere --iterations 20
//! ```

use std::process::ExitCode;
use std::time::Instant;

use kala_geometry::{
    DEFAULT_BOX_EDGE_SUBDIVISIONS, DEFAULT_CYLINDER_HEIGHT_SEGMENTS,
    DEFAULT_CYLINDER_RADIAL_SEGMENTS, DEFAULT_SPHERE_RINGS, DEFAULT_SPHERE_SECTORS,
    MAX_BOX_EDGE_SUBDIVISIONS, MAX_CYLINDER_HEIGHT_SEGMENTS, MAX_CYLINDER_RADIAL_SEGMENTS,
    MAX_SPHERE_RINGS, MAX_SPHERE_SECTORS, Mesh, PrimitiveParameters, build_primitive,
    compute_bounds, signed_volume, write_binary_stl,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut text, "{byte:02x}").unwrap();
    }
    text
}

fn summarize(id: &str, mesh: &Mesh, stl: &[u8]) -> Value {
    let bounds = compute_bounds(mesh.positions()).expect("built mesh has complete positions");
    json!({
        "id": id,
        "vertex_count": mesh.vertex_count(),
        "triangle_count": mesh.triangle_count(),
        "bounds": {"min": bounds.min, "max": bounds.max},
        "signed_volume": signed_volume(mesh),
        "stl_length": stl.len(),
        "stl_sha256": sha256_hex(stl),
    })
}

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  geometry_cli fixtures\n  geometry_cli build --kind <box|cylinder|sphere> \
         [--parameters <json>] [--out <file.stl>] [--iterations <n>]"
    );
    ExitCode::from(2)
}

fn build_fixture_cases() -> Result<(), String> {
    let cases: [(&str, &str, Value); 6] = [
        ("box_default", "box", json!({})),
        (
            "box_max",
            "box",
            json!({"subdivisions": MAX_BOX_EDGE_SUBDIVISIONS}),
        ),
        ("cylinder_default", "cylinder", json!({})),
        (
            "cylinder_max",
            "cylinder",
            json!({
                "radial_segments": MAX_CYLINDER_RADIAL_SEGMENTS,
                "height_segments": MAX_CYLINDER_HEIGHT_SEGMENTS,
            }),
        ),
        ("sphere_default", "sphere", json!({})),
        (
            "sphere_max",
            "sphere",
            json!({"rings": MAX_SPHERE_RINGS, "sectors": MAX_SPHERE_SECTORS}),
        ),
    ];
    for (id, kind, parameters) in cases {
        let parameters = PrimitiveParameters::from_json_value(&parameters)
            .map_err(|error| format!("{id}: {error}"))?;
        let mesh = build_primitive(kind, &parameters).map_err(|error| format!("{id}: {error}"))?;
        let stl = write_binary_stl(&mesh);
        println!("{}", summarize(id, &mesh, &stl));
    }
    // Keep the default constants observable in this helper even though the
    // default cases above use an empty parameter map.
    eprintln!(
        "defaults: box_subdivisions={DEFAULT_BOX_EDGE_SUBDIVISIONS} \
         cylinder={DEFAULT_CYLINDER_RADIAL_SEGMENTS}x{DEFAULT_CYLINDER_HEIGHT_SEGMENTS} \
         sphere={DEFAULT_SPHERE_RINGS}x{DEFAULT_SPHERE_SECTORS}"
    );
    Ok(())
}

fn run_build(arguments: &[String]) -> Result<(), String> {
    let mut kind: Option<String> = None;
    let mut parameters = "{}".to_owned();
    let mut out: Option<String> = None;
    let mut iterations: usize = 1;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--kind" => {
                index += 1;
                kind = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--kind requires a value".to_owned())?
                        .clone(),
                );
            }
            "--parameters" => {
                index += 1;
                parameters = arguments
                    .get(index)
                    .ok_or_else(|| "--parameters requires a value".to_owned())?
                    .clone();
            }
            "--out" => {
                index += 1;
                out = Some(
                    arguments
                        .get(index)
                        .ok_or_else(|| "--out requires a path".to_owned())?
                        .clone(),
                );
            }
            "--iterations" => {
                index += 1;
                iterations = arguments
                    .get(index)
                    .ok_or_else(|| "--iterations requires a count".to_owned())?
                    .parse()
                    .map_err(|_| "--iterations must be a positive integer".to_owned())?;
            }
            other => return Err(format!("unknown argument: {other}")),
        }
        index += 1;
    }
    if iterations == 0 {
        return Err("--iterations must be at least 1".to_owned());
    }
    let kind = kind.ok_or_else(|| "--kind is required".to_owned())?;
    let parameters =
        PrimitiveParameters::from_json_str(&parameters).map_err(|error| error.to_string())?;

    let started = Instant::now();
    let mut last_mesh = None;
    let mut last_stl = Vec::new();
    for _ in 0..iterations {
        let mesh = build_primitive(&kind, &parameters).map_err(|error| error.to_string())?;
        last_stl = write_binary_stl(&mesh);
        last_mesh = Some(mesh);
    }
    let elapsed = started.elapsed();

    let mesh = last_mesh.expect("iterations is at least one");
    if let Some(path) = out {
        std::fs::write(&path, &last_stl).map_err(|error| format!("write {path}: {error}"))?;
    }
    println!(
        "{}",
        summarize(
            &format!(
                "{kind} ({iterations} build{})",
                if iterations == 1 { "" } else { "s" }
            ),
            &mesh,
            &last_stl,
        )
    );
    eprintln!(
        "iterations={iterations} elapsed_ms={:.3}",
        elapsed.as_secs_f64() * 1000.0
    );
    Ok(())
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = arguments.first() else {
        return usage();
    };
    let result = match command.as_str() {
        "fixtures" if arguments.len() == 1 => build_fixture_cases(),
        "build" => run_build(&arguments[1..]),
        _ => return usage(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}
