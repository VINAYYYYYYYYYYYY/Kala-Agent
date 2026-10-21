//! Live FreeCAD cut guard: a pocket that removes nothing must say so.
//!
//! The 2026-10-20 flange repro sketched each bolt hole on `XZ` while the
//! body came from an `XY` revolve, and the FreeCAD adapter reported a valid
//! solid with a volume that still contained every hole. The adapter now
//! measures the body before and after every pocket and warns when the volume
//! did not decrease.
//!
//! This test drives the real `FreeCADCmd` through this crate's provider. It
//! skips only when no `FreeCADCmd` is installed; it never substitutes a fake
//! build for a missing FreeCAD.

use std::collections::BTreeMap;

use kala_cad_providers::{FreeCadProvider, FreeCadProviderOptions};
use kala_contracts::{ConstructionScript, load_construction_json_str};
use kala_json::{CanonicalObject, CanonicalValue, JsonText};

const PLATE_VOLUME_MM3: f64 = 24_695.221_316;
const PLATE_WITHOUT_CUT_MM3: f64 = 25_600.0;

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

fn step(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    object(entries)
}

fn sketch(plane: &str, diameter: f64, x: f64, y: f64) -> CanonicalValue {
    step(vec![
        ("op", text("sketch")),
        ("plane", text(plane)),
        ("profile", text("circle")),
        ("diameter", number(diameter)),
        ("x", number(x)),
        ("y", number(y)),
    ])
}

fn rectangle(plane: &str, width: f64, height: f64) -> CanonicalValue {
    step(vec![
        ("op", text("sketch")),
        ("plane", text(plane)),
        ("profile", text("rectangle")),
        ("width", number(width)),
        ("height", number(height)),
    ])
}

fn rectangle_at(plane: &str, width: f64, height: f64, x: f64, y: f64) -> CanonicalValue {
    step(vec![
        ("op", text("sketch")),
        ("plane", text(plane)),
        ("profile", text("rectangle")),
        ("width", number(width)),
        ("height", number(height)),
        ("x", number(x)),
        ("y", number(y)),
    ])
}

/// The normal 80 x 40 x 8 plate with four Ø6 through holes 8 mm from the edges.
fn plate_script() -> CanonicalValue {
    let mut steps = vec![
        rectangle("XY", 80.0, 40.0),
        step(vec![("op", text("extrude")), ("length", number(8.0))]),
    ];
    for (x, y) in [(8.0, 8.0), (72.0, 8.0), (72.0, 32.0), (8.0, 32.0)] {
        steps.push(sketch("XY", 6.0, x, y));
        steps.push(step(vec![("op", text("pocket"))]));
    }
    object(vec![
        (
            "summary",
            text("80 x 40 x 8 plate with four 6 mm through holes"),
        ),
        (
            "components",
            CanonicalValue::Array(vec![object(vec![
                ("name", text("plate")),
                ("steps", CanonicalValue::Array(steps)),
            ])]),
        ),
    ])
}

/// The same plate, but its one Ø6 hole is sketched entirely outside the body.
fn off_plate_script() -> CanonicalValue {
    object(vec![
        (
            "summary",
            text("80 x 40 x 8 plate with one hole outside the body"),
        ),
        (
            "components",
            CanonicalValue::Array(vec![object(vec![
                ("name", text("plate")),
                (
                    "steps",
                    CanonicalValue::Array(vec![
                        rectangle("XY", 80.0, 40.0),
                        step(vec![("op", text("extrude")), ("length", number(8.0))]),
                        sketch("XY", 6.0, 100.0, 20.0),
                        step(vec![("op", text("pocket"))]),
                    ]),
                ),
            ])]),
        ),
    ])
}

/// One 12-step half of the 2026-10-20 MiniMax flange repro.
fn flange_repro_half(name: &str, rectangle_x: f64, holes: [(f64, f64); 3]) -> CanonicalValue {
    let mut steps = vec![
        sketch("XY", 100.0, 0.0, 0.0),
        step(vec![("op", text("extrude")), ("length", number(12.0))]),
        sketch("XY", 30.0, 0.0, 0.0),
        step(vec![("op", text("pocket"))]),
        rectangle_at("XY", 100.0, 100.0, rectangle_x, -50.0),
        step(vec![("op", text("pocket"))]),
    ];
    for (x, y) in holes {
        steps.push(sketch("XY", 8.0, x, y));
        steps.push(step(vec![("op", text("pocket"))]));
    }
    object(vec![
        ("name", text(name)),
        ("steps", CanonicalValue::Array(steps)),
    ])
}

/// The whole MiniMax flange: two halves, each with three holes in removed material.
fn flange_repro_script() -> CanonicalValue {
    object(vec![
        ("summary", text("100 mm flange in two halves")),
        (
            "components",
            CanonicalValue::Array(vec![
                flange_repro_half(
                    "flange_right",
                    0.0,
                    [
                        (40.0, 0.0),
                        (20.0, 34.641_016_151_377_55),
                        (20.0, -34.641_016_151_377_55),
                    ],
                ),
                flange_repro_half(
                    "flange_left",
                    -100.0,
                    [
                        (-40.0, 0.0),
                        (-20.0, 34.641_016_151_377_55),
                        (-20.0, -34.641_016_151_377_55),
                    ],
                ),
            ]),
        ),
        (
            "assembly",
            CanonicalValue::Array(vec![
                step(vec![
                    ("op", text("place")),
                    ("component", text("flange_right")),
                ]),
                step(vec![
                    ("op", text("place")),
                    ("component", text("flange_left")),
                ]),
                step(vec![
                    ("op", text("fuse")),
                    (
                        "components",
                        CanonicalValue::Array(vec![text("flange_right"), text("flange_left")]),
                    ),
                ]),
            ]),
        ),
    ])
}

fn script(value: &CanonicalValue) -> ConstructionScript {
    let encoded = kala_json::canonical_dumps(value, &kala_json::Limits::default())
        .expect("the test script encodes");
    load_construction_json_str(&encoded).expect("a valid test script")
}

fn provider() -> Option<FreeCadProvider> {
    let mut provider = FreeCadProvider::new(FreeCadProviderOptions {
        probe_cache_seconds: Some(0.0),
        ..FreeCadProviderOptions::default()
    })
    .expect("the default configuration is valid");
    let availability = provider.probe();
    if availability.is_available() {
        return Some(provider);
    }
    eprintln!(
        "skipping the live FreeCAD cut guard: {}",
        availability.reason().unwrap_or("FreeCADCmd is unavailable")
    );
    None
}

#[test]
fn freecad_reports_a_pocket_that_removed_no_material() {
    let Some(mut provider) = provider() else {
        return;
    };
    let started = std::time::Instant::now();
    let build = provider
        .build(&script(&off_plate_script()))
        .expect("the off-plate hole is a legal script; FreeCAD must build it");
    let volume = build.measured().volume_mm3().expect("a measured volume");
    eprintln!(
        "off-plate build: {volume} mm3, warnings {:?} in {:?}",
        build.warnings(),
        started.elapsed()
    );
    assert!(
        (volume - PLATE_WITHOUT_CUT_MM3).abs() < 1e-6,
        "the off-plate pocket removed material: volume {volume}"
    );
    assert_eq!(build.warnings().len(), 1, "{:?}", build.warnings());
    assert_eq!(
        build.warnings()[0],
        "component 'plate': pocket at step index 3 on XY removed no material"
    );
}

#[test]
fn freecad_warns_once_for_every_ineffective_flange_pocket() {
    let Some(mut provider) = provider() else {
        return;
    };
    let build = provider
        .build(&script(&flange_repro_script()))
        .expect("the flange repro is a legal script; FreeCAD must build it");
    let volume = build.measured().volume_mm3().expect("a measured volume");
    eprintln!(
        "MiniMax flange repro: {volume} mm3, warnings {:?}",
        build.warnings()
    );
    assert_eq!(
        build.warnings(),
        [
            "component 'flange_right': pocket at step index 7 on XY removed no material",
            "component 'flange_right': pocket at step index 9 on XY removed no material",
            "component 'flange_right': pocket at step index 11 on XY removed no material",
            "component 'flange_left': pocket at step index 7 on XY removed no material",
            "component 'flange_left': pocket at step index 9 on XY removed no material",
            "component 'flange_left': pocket at step index 11 on XY removed no material",
        ]
    );
    // Every half removed its own side, so the fused part is the full flange
    // with the Ø30 bore but no bolt hole at all.
    assert!(
        (volume - 85_765.479_443).abs() < 1e-3,
        "the flange repro volume changed: {volume}"
    );
}

#[test]
fn freecad_still_cuts_the_four_hole_plate() {
    let Some(mut provider) = provider() else {
        return;
    };
    let build = provider
        .build(&script(&plate_script()))
        .expect("the four-hole plate builds");
    let volume = build.measured().volume_mm3().expect("a measured volume");
    eprintln!(
        "four-hole plate build: {volume} mm3, warnings {:?}",
        build.warnings()
    );
    assert!(
        (volume - PLATE_VOLUME_MM3).abs() < 1e-3,
        "the four-hole plate volume changed: {volume}"
    );
    assert!(build.warnings().is_empty(), "{:?}", build.warnings());
}

#[test]
fn the_bounded_adapter_environment_still_reaches_freecad() {
    // Guard against a test that passes only because the adapter silently
    // skipped: the provider is configured from an explicit environment map.
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
