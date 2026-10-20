#![cfg(feature = "native")]

//! FFI lifetime, failure-reuse and concurrency proofs.
//!
//! `Shape` is intentionally thread-affine: these tests move only plain data
//! across threads and build/measure/export independent shapes inside each
//! worker. That proves repeated and concurrent native calls without an
//! unsound `Send`/`Sync` claim for an OCCT handle.

mod common;

use std::collections::BTreeSet;

use common::{assert_volume, box_script, compile_json, l_bracket_script, measure};
use kala_cad::{
    CompileOptions, MeshOptions, Shape, compile_json_str, measure_shape, probe_capability,
    step_bytes, step_shape, stl_bytes, tessellate,
};
use serde_json::json;

fn box_shape(size: f64) -> Shape {
    compile_json(&box_script(size, size, size))
        .components
        .remove(0)
        .local_shape
}

fn resident_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages = statm.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(pages * 4096)
}

#[test]
fn repeated_compile_measure_mesh_and_drop_does_not_grow_without_bound() {
    // Warm up allocator caches and OCCT's own pools before the baseline.
    for _ in 0..20 {
        let result = compile_json(&box_script(20.0, 20.0, 10.0));
        let measured = measure(&result);
        assert_volume(measured.volume_mm3, 4000.0, 1e-9, "warm-up");
        let mesh = tessellate(&result.assembly_shape, MeshOptions::default()).unwrap();
        assert!(mesh.triangle_count() > 0);
    }
    let before = resident_bytes();
    for iteration in 0..1_000 {
        let result = compile_json(&box_script(20.0, 20.0, 10.0));
        let measured = measure(&result);
        assert_volume(measured.volume_mm3, 4000.0, 1e-9, "repeat");
        let mesh = tessellate(&result.assembly_shape, MeshOptions::default()).unwrap();
        let stl = stl_bytes(&mesh, kala_cad::STL_HEADER_FILL).unwrap();
        assert_eq!(stl.len(), 84 + mesh.triangle_count() * 50);
        if iteration == 0 {
            let step = step_bytes(&result.assembly_shape).unwrap();
            let imported = step_shape(&step).unwrap();
            assert_volume(
                measure_shape(&imported).unwrap().volume_mm3,
                4000.0,
                1e-6,
                "repeat step",
            );
        }
    }
    let after = resident_bytes();
    if let (Some(before), Some(after)) = (before, after) {
        let growth = after.saturating_sub(before);
        eprintln!("resident growth over 1000 compile/measure/mesh/drop cycles: {growth} bytes");
        assert!(
            growth < 128 * 1024 * 1024,
            "resident memory grew by {growth} bytes; possible handle leak"
        );
    }
}

#[test]
fn measured_plain_data_can_cross_threads_but_a_shape_is_thread_affine() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<kala_cad::Measurement>();
    assert_send_sync::<kala_cad::Bounds>();
    assert_send_sync::<kala_cad::MeshData>();
    assert_send_sync::<kala_cad::CheckReport>();
    // `Shape` is covered by the two compile-fail doctests on its type.
    let measured = measure(&compile_json(&box_script(2.0, 3.0, 4.0)));
    let worker = std::thread::spawn(move || measured.volume_mm3);
    assert!((worker.join().unwrap() - 24.0).abs() <= 1e-9);
}

#[test]
fn a_cloned_shape_outlives_the_original_owner() {
    let original = box_shape(20.0);
    let clone = original.try_clone_shape().expect("handle copy");
    drop(original);
    let measured = measure_shape(&clone).unwrap();
    assert_volume(measured.volume_mm3, 8000.0, 1e-12, "clone");
    assert_eq!(measured.solid_count, 1);
}

#[test]
fn independent_shapes_keep_independent_lifetimes() {
    let mut shapes = Vec::new();
    for size in 1..=20 {
        shapes.push((size, box_shape(size as f64)));
    }
    let mut kept = Vec::new();
    for (index, (size, shape)) in shapes.into_iter().enumerate() {
        if index % 2 == 0 {
            kept.push((size, shape));
        }
    }
    for (size, shape) in kept {
        let measured = measure_shape(&shape).unwrap();
        assert_volume(
            measured.volume_mm3,
            (size * size * size) as f64,
            1e-12,
            "independent shape",
        );
    }
}

#[test]
fn repeated_kernel_failures_do_not_poison_later_successful_calls() {
    let script = common::script(
        json!([{
            "name": "plate",
            "steps": [
                {"op": "sketch", "profile": "rectangle", "plane": "XY",
                 "width": 10.0, "height": 10.0},
                {"op": "extrude", "length": 5.0},
                {"op": "sketch", "profile": "rectangle", "plane": "XY",
                 "width": 20.0, "height": 20.0},
                {"op": "pocket"}
            ]
        }]),
        json!([]),
    );
    for _ in 0..50 {
        let error = compile_json_str(&script, &CompileOptions::default())
            .expect_err("the cut must remove the whole solid");
        assert!(
            error.message().contains("removed the whole solid"),
            "unexpected failure: {}",
            error.message()
        );
    }
    let recovered = compile_json(&box_script(20.0, 20.0, 10.0));
    assert_volume(measure(&recovered).volume_mm3, 4000.0, 1e-9, "recovered");

    for _ in 0..50 {
        let error = step_shape(b"not a step document").expect_err("must fail");
        assert!(!error.message().is_empty());
    }
    let step = step_bytes(&recovered.assembly_shape).unwrap();
    let imported = step_shape(&step).unwrap();
    assert_volume(
        measure_shape(&imported).unwrap().volume_mm3,
        4000.0,
        1e-6,
        "recovered step",
    );
}

#[test]
fn concurrent_independent_compiles_measurements_and_meshes_are_consistent() {
    let mut workers = Vec::new();
    for worker in 0..8 {
        workers.push(std::thread::spawn(move || {
            let mut volumes = Vec::new();
            for iteration in 0..20 {
                let size = 5.0 + worker as f64 + iteration as f64 / 10.0;
                let result = compile_json(&box_script(size, size, size));
                let measured = measure_shape(&result.assembly_shape).unwrap();
                let expected = size * size * size;
                assert!((measured.volume_mm3 - expected).abs() <= expected * 1e-9);
                let mesh = tessellate(&result.assembly_shape, MeshOptions::default()).unwrap();
                assert!(mesh.triangle_count() > 0);
                volumes.push(measured.volume_mm3);
            }
            volumes
        }));
    }
    let mut all = Vec::new();
    for worker in workers {
        all.extend(worker.join().expect("worker must not panic"));
    }
    assert_eq!(all.len(), 160);
    assert!(all.iter().all(|volume| *volume > 0.0));
}

#[test]
fn concurrent_step_exports_do_not_race_the_global_unit_settings() {
    let mut workers = Vec::new();
    for worker in 0..6 {
        workers.push(std::thread::spawn(move || {
            let size = 6.0 + worker as f64;
            let script = box_script(size, size + 1.0, size + 2.0);
            let expected = size * (size + 1.0) * (size + 2.0);
            for _ in 0..5 {
                let result = compile_json(&script);
                let step = step_bytes(&result.assembly_shape).unwrap();
                assert!(step.starts_with(b"ISO-10303-21"));
                let imported = step_shape(&step).unwrap();
                let measured = measure_shape(&imported).unwrap();
                assert!((measured.volume_mm3 - expected).abs() <= expected * 1e-6);
                assert_eq!(measured.solid_count, 1);
            }
            expected
        }));
    }
    let mut expected = Vec::new();
    for worker in workers {
        expected.push(worker.join().expect("worker must not panic"));
    }
    assert_eq!(expected.len(), 6);
}

#[test]
fn a_stepped_shape_does_not_retain_the_input_step_buffer() {
    let original = compile_json(&l_bracket_script());
    let step = step_bytes(&original.assembly_shape).unwrap();
    let expected = measure(&original);
    let imported = {
        let owned = step.clone();
        step_shape(&owned).unwrap()
    };
    drop(step);
    let measured = measure_shape(&imported).unwrap();
    assert_volume(
        measured.volume_mm3,
        expected.volume_mm3,
        1e-6,
        "buffer lifetime",
    );
}

#[test]
fn null_shapes_can_be_created_copied_and_dropped_repeatedly() {
    for _ in 0..200 {
        let shape = Shape::null().unwrap();
        assert!(shape.is_null());
        let copy = shape.try_clone_shape().unwrap();
        assert!(copy.is_null());
        assert_eq!(
            measure_shape(&copy).unwrap_err().message(),
            "the shape is null and cannot be measured"
        );
    }
}

#[test]
fn the_capability_probe_is_real_repeatable_and_versioned() {
    let first = probe_capability();
    let second = probe_capability();
    assert_eq!(first, second);
    assert!(
        first.available,
        "the native kernel must be available: {:?}",
        first.reason
    );
    assert_eq!(first.backend, "native-occt");
    assert!(
        first.kernel.starts_with("7.9"),
        "unexpected OCCT version {}",
        first.kernel
    );
    assert!(first.reason.is_none());
    assert!(kala_cad::design_available());
}

#[test]
fn many_shapes_can_coexist_and_be_dropped_in_any_order() {
    let mut shapes = Vec::new();
    for size in [3.0, 7.0, 11.0, 19.0, 23.0] {
        shapes.push(box_shape(size));
    }
    let mut kept = BTreeSet::new();
    while let Some(shape) = shapes.pop() {
        let measured = measure_shape(&shape).unwrap();
        kept.insert(measured.volume_mm3.round() as i64);
    }
    assert_eq!(kept.len(), 5);
}
