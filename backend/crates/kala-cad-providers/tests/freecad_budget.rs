//! Live FreeCAD budget measurements: the 100-hole plate and a 500-step design.
//!
//! These tests drive the real `FreeCADCmd` through this crate's provider. They
//! skip only when no `FreeCADCmd` is installed; they never substitute a fake
//! build for a missing FreeCAD. The timings they print are the measurement the
//! design deadline is sized from.

use std::collections::BTreeMap;

use kala_cad_providers::{FreeCadProvider, FreeCadProviderOptions};
use kala_contracts::{ConstructionScript, load_construction_json_str};
use kala_json::{CanonicalObject, CanonicalValue, JsonText};

fn text(value: &str) -> CanonicalValue {
    CanonicalValue::text(value)
}

fn number(value: f64) -> CanonicalValue {
    CanonicalValue::float(value).expect("finite test number")
}

fn object(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    let mut object = CanonicalObject::new();
    for (key, value) in entries {
        object.insert(JsonText::new(key), value);
    }
    CanonicalValue::Object(object)
}

fn sketch(plane: &str, diameter: f64, x: f64, y: f64) -> CanonicalValue {
    object(vec![
        ("op", text("sketch")),
        ("plane", text(plane)),
        ("profile", text("circle")),
        ("diameter", number(diameter)),
        ("x", number(x)),
        ("y", number(y)),
    ])
}

fn rectangle(plane: &str, width: f64, height: f64) -> CanonicalValue {
    object(vec![
        ("op", text("sketch")),
        ("plane", text(plane)),
        ("profile", text("rectangle")),
        ("width", number(width)),
        ("height", number(height)),
    ])
}

fn extrude(length: f64) -> CanonicalValue {
    object(vec![("op", text("extrude")), ("length", number(length))])
}

fn pocket() -> CanonicalValue {
    object(vec![("op", text("pocket"))])
}

fn plate_script(
    summary: &str,
    width: f64,
    height: f64,
    thickness: f64,
    diameter: f64,
    holes: &[(f64, f64)],
) -> CanonicalValue {
    let mut steps = vec![rectangle("XY", width, height), extrude(thickness)];
    for (x, y) in holes {
        steps.push(sketch("XY", diameter, *x, *y));
        steps.push(pocket());
    }
    object(vec![
        ("summary", text(summary)),
        (
            "components",
            CanonicalValue::Array(vec![object(vec![
                ("name", text("plate")),
                ("steps", CanonicalValue::Array(steps)),
            ])]),
        ),
    ])
}

/// The 200 x 200 x 5 mm plate with a 10 x 10 grid of Ø3 through holes: 202 steps.
fn hundred_hole_plate() -> CanonicalValue {
    let mut holes = Vec::new();
    for row in 0..10 {
        for column in 0..10 {
            holes.push((20.0 + 16.0 * column as f64, 20.0 + 16.0 * row as f64));
        }
    }
    plate_script(
        "200 x 200 x 5 mm plate with a 10 x 10 grid of 3 mm through holes",
        200.0,
        200.0,
        5.0,
        3.0,
        &holes,
    )
}

/// The same plate, but three holes are sketched entirely outside the body.
fn hundred_hole_plate_with_three_off_plate() -> CanonicalValue {
    let mut holes = Vec::new();
    for row in 0..10 {
        for column in 0..10 {
            holes.push((20.0 + 16.0 * column as f64, 20.0 + 16.0 * row as f64));
        }
    }
    holes[0] = (300.0, 20.0);
    holes[11] = (300.0, 300.0);
    holes[99] = (-100.0, -100.0);
    plate_script(
        "200 x 200 x 5 mm plate with three holes outside the body",
        200.0,
        200.0,
        5.0,
        3.0,
        &holes,
    )
}

/// A 500-step design: a 2 000 x 2 000 x 5 mm plate with 249 Ø6 through holes.
fn five_hundred_step_plate() -> CanonicalValue {
    let mut holes = Vec::new();
    for index in 0..249 {
        let column = (index % 16) as f64;
        let row = (index / 16) as f64;
        holes.push((100.0 + 110.0 * column, 100.0 + 110.0 * row));
    }
    plate_script(
        "2000 x 2000 x 5 mm plate with 249 through holes",
        2000.0,
        2000.0,
        5.0,
        6.0,
        &holes,
    )
}

fn script(value: &CanonicalValue) -> ConstructionScript {
    let encoded = kala_json::canonical_dumps(value, &kala_json::Limits::default())
        .expect("the test script encodes");
    load_construction_json_str(&encoded).expect("a valid test script")
}

fn provider() -> Option<FreeCadProvider> {
    let mut provider = FreeCadProvider::new(FreeCadProviderOptions {
        probe_cache_seconds: Some(0.0),
        timeout_seconds: Some(3_500.0),
        ..FreeCadProviderOptions::default()
    })
    .expect("the default configuration is valid");
    let availability = provider.probe();
    if availability.is_available() {
        return Some(provider);
    }
    eprintln!(
        "skipping the live FreeCAD budget measurement: {}",
        availability.reason().unwrap_or("FreeCADCmd is unavailable")
    );
    None
}

fn expected_volume(width: f64, height: f64, thickness: f64, diameter: f64, holes: usize) -> f64 {
    width * height * thickness
        - holes as f64 * std::f64::consts::PI * (diameter / 2.0).powi(2) * thickness
}

#[test]
fn freecad_builds_the_hundred_hole_plate_exactly() {
    let Some(mut provider) = provider() else {
        return;
    };
    let started = std::time::Instant::now();
    let build = provider
        .build(&script(&hundred_hole_plate()))
        .expect("the 100-hole plate builds");
    let elapsed = started.elapsed();
    let volume = build.measured().volume_mm3().expect("a measured volume");
    let expected = expected_volume(200.0, 200.0, 5.0, 3.0, 100);
    eprintln!(
        "100-hole plate (202 steps): {volume} mm3 (expected {expected}), {} warnings, {elapsed:?}",
        build.warnings().len()
    );
    assert!(
        (volume - expected).abs() < 1e-6,
        "the 100-hole plate volume is {volume}, expected {expected}"
    );
    assert!(
        build.warnings().is_empty(),
        "the 100-hole plate has ineffective cuts: {:?}",
        build.warnings()
    );
}

#[test]
fn freecad_counts_three_off_plate_holes_exactly() {
    let Some(mut provider) = provider() else {
        return;
    };
    let build = provider
        .build(&script(&hundred_hole_plate_with_three_off_plate()))
        .expect("the plate with three off-plate holes builds");
    let volume = build.measured().volume_mm3().expect("a measured volume");
    let expected = expected_volume(200.0, 200.0, 5.0, 3.0, 97);
    let warnings: Vec<String> = build.warnings().to_vec();
    eprintln!("100-hole plate with 3 off-plate holes: {volume} mm3, warnings {warnings:?}");
    assert!(
        (volume - expected).abs() < 1e-6,
        "the plate volume is {volume}, expected {expected}"
    );
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert!(
        warnings
            .iter()
            .all(|warning| warning.contains("removed no material")),
        "{warnings:?}"
    );
}

#[test]
fn freecad_builds_a_five_hundred_step_design() {
    let Some(mut provider) = provider() else {
        return;
    };
    let parsed = script(&five_hundred_step_plate());
    assert_eq!(parsed.components()[0].steps().len(), 500);
    let started = std::time::Instant::now();
    let build = provider.build(&parsed).expect("the 500-step plate builds");
    let elapsed = started.elapsed();
    let volume = build.measured().volume_mm3().expect("a measured volume");
    let expected = expected_volume(2000.0, 2000.0, 5.0, 6.0, 249);
    eprintln!("500-step plate: {volume} mm3 (expected {expected}) in {elapsed:?}");
    assert!(
        (volume - expected).abs() < 1e-3,
        "the 500-step plate volume is {volume}, expected {expected}"
    );
}

#[test]
fn the_bounded_adapter_environment_still_reaches_freecad() {
    let mut environment = BTreeMap::new();
    environment.insert(
        "PATH".to_owned(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".to_owned()),
    );
    environment.insert(
        "HOME".to_owned(),
        std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_owned()),
    );
    let mut provider = FreeCadProvider::new(FreeCadProviderOptions {
        environ: Some(environment),
        probe_cache_seconds: Some(0.0),
        ..FreeCadProviderOptions::default()
    })
    .expect("the default configuration is valid");
    let availability = provider.probe();
    if !availability.is_available() {
        eprintln!(
            "skipping the environment probe: {}",
            availability.reason().unwrap_or("FreeCADCmd is unavailable")
        );
        return;
    }
    assert_eq!(availability.version(), Some("1.1.4"));
}
