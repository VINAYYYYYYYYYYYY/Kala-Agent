#![cfg(feature = "native")]

//! Native OCCT tessellation: counts, winding, normals, transforms and STL.

mod common;

use common::{assert_bounds, box_script, compile_json, script};
use kala_cad::{
    MAX_MESH_TRIANGLES, MAX_MESH_VERTICES, MeshData, MeshOptions, Shape, stl_bytes, tessellate,
};
use serde_json::json;

fn compile_shape(text: &str) -> Shape {
    compile_json(text).components.remove(0).local_shape
}

fn box_shape(size_x: f64, size_y: f64, size_z: f64) -> Shape {
    compile_shape(&box_script(size_x, size_y, size_z))
}

fn cylinder_shape(radius: f64, height: f64) -> Shape {
    compile_shape(&script(
        json!([{
            "name": "pin",
            "steps": [
                {"op": "sketch", "profile": "circle", "plane": "XY", "diameter": radius * 2.0},
                {"op": "extrude", "length": height}
            ]
        }]),
        json!([]),
    ))
}

fn triangle_normal(positions: &[f64], indices: &[u32], triangle: usize) -> [f64; 3] {
    let i0 = indices[triangle * 3] as usize;
    let i1 = indices[triangle * 3 + 1] as usize;
    let i2 = indices[triangle * 3 + 2] as usize;
    let a = [
        positions[i0 * 3],
        positions[i0 * 3 + 1],
        positions[i0 * 3 + 2],
    ];
    let b = [
        positions[i1 * 3],
        positions[i1 * 3 + 1],
        positions[i1 * 3 + 2],
    ];
    let c = [
        positions[i2 * 3],
        positions[i2 * 3 + 1],
        positions[i2 * 3 + 2],
    ];
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let mut normal = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let magnitude = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    if magnitude > 0.0 {
        normal = [
            normal[0] / magnitude,
            normal[1] / magnitude,
            normal[2] / magnitude,
        ];
    }
    normal
}

fn triangle_centroid(positions: &[f64], indices: &[u32], triangle: usize) -> [f64; 3] {
    let mut centroid = [0.0; 3];
    for corner in 0..3 {
        let index = indices[triangle * 3 + corner] as usize;
        for axis in 0..3 {
            centroid[axis] += positions[index * 3 + axis] / 3.0;
        }
    }
    centroid
}

#[test]
fn a_box_tessellates_into_the_expected_face_soup() {
    let shape = box_shape(20.0, 20.0, 10.0);
    let mesh = tessellate(&shape, MeshOptions::default()).unwrap();
    assert_eq!(mesh.vertex_count(), 24, "six planar faces, four nodes each");
    assert_eq!(
        mesh.triangle_count(),
        12,
        "six planar faces, two triangles each"
    );
    assert_eq!(mesh.positions.len(), mesh.vertex_count() * 3);
    assert_eq!(mesh.normals.len(), mesh.positions.len());
    assert!(mesh.is_well_formed());
    assert_eq!(mesh.deflection_mm, 0.1);
    assert_eq!(mesh.angular_deflection_rad, 0.5);
    assert!(mesh.positions.iter().all(|value| value.is_finite()));
    assert!(mesh.normals.iter().all(|value| value.is_finite()));
}

#[test]
fn box_triangles_are_outward_and_their_stored_normals_agree() {
    let shape = box_shape(20.0, 20.0, 10.0);
    let mesh = tessellate(&shape, MeshOptions::default()).unwrap();
    let centre = [10.0, 10.0, 5.0];
    let mut zero_normals = 0;
    for vertex in 0..mesh.vertex_count() {
        let normal = [
            mesh.normals[vertex * 3],
            mesh.normals[vertex * 3 + 1],
            mesh.normals[vertex * 3 + 2],
        ];
        let magnitude =
            (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        if magnitude == 0.0 {
            zero_normals += 1;
        } else {
            assert!(
                (magnitude - 1.0).abs() <= 1e-9,
                "normal was not unit length"
            );
            let nonzero = normal.iter().filter(|value| value.abs() > 1e-12).count();
            assert_eq!(nonzero, 1, "a box normal should be axis aligned");
            assert!(
                normal
                    .iter()
                    .any(|value| (value.abs() - 1.0).abs() <= 1e-12),
                "a box normal should be +/-1 on one axis"
            );
        }
    }
    assert_eq!(
        zero_normals, 0,
        "the kernel must provide finite box normals"
    );

    for triangle in 0..mesh.triangle_count() {
        let geometric = triangle_normal(&mesh.positions, &mesh.indices, triangle);
        let centroid = triangle_centroid(&mesh.positions, &mesh.indices, triangle);
        let outward = [
            centroid[0] - centre[0],
            centroid[1] - centre[1],
            centroid[2] - centre[2],
        ];
        let dot = geometric[0] * outward[0] + geometric[1] * outward[1] + geometric[2] * outward[2];
        assert!(dot > 0.0, "triangle {triangle} is wound inward");
        for corner in 0..3 {
            let vertex = mesh.indices[triangle * 3 + corner] as usize;
            let stored = [
                mesh.normals[vertex * 3],
                mesh.normals[vertex * 3 + 1],
                mesh.normals[vertex * 3 + 2],
            ];
            let agreement =
                geometric[0] * stored[0] + geometric[1] * stored[1] + geometric[2] * stored[2];
            assert!(
                agreement > 0.99,
                "triangle {triangle} corner {corner} normal disagrees with winding"
            );
        }
    }
}

#[test]
fn a_cylinder_has_side_and_cap_triangles_with_finite_normals() {
    let shape = cylinder_shape(3.0, 8.0);
    let mesh = tessellate(&shape, MeshOptions::default()).unwrap();
    assert!(mesh.vertex_count() > 0);
    assert!(mesh.triangle_count() > 0);
    assert!(mesh.positions.iter().all(|value| value.is_finite()));
    assert!(mesh.normals.iter().all(|value| value.is_finite()));
    let mut cap_triangles = 0;
    for triangle in 0..mesh.triangle_count() {
        let geometric = triangle_normal(&mesh.positions, &mesh.indices, triangle);
        if geometric[2].abs() > 0.999 && geometric[0].abs() < 1e-6 && geometric[1].abs() < 1e-6 {
            cap_triangles += 1;
        }
        for corner in 0..3 {
            let vertex = mesh.indices[triangle * 3 + corner] as usize;
            let normal = [
                mesh.normals[vertex * 3],
                mesh.normals[vertex * 3 + 1],
                mesh.normals[vertex * 3 + 2],
            ];
            let magnitude =
                (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
            assert!((magnitude - 1.0).abs() <= 1e-9);
            let agreement =
                geometric[0] * normal[0] + geometric[1] * normal[1] + geometric[2] * normal[2];
            assert!(agreement > 0.98, "cylinder normal disagrees with winding");
        }
    }
    assert!(cap_triangles > 0, "the cylinder caps must be tessellated");
}

#[test]
fn a_placement_translates_positions_and_keeps_normals() {
    let local_script = script(
        json!([{
            "name": "plate",
            "steps": [
                {"op": "sketch", "profile": "rectangle", "plane": "XY",
                 "width": 10.0, "height": 10.0},
                {"op": "extrude", "length": 5.0}
            ]
        }]),
        json!([]),
    );
    let placed_script = script(
        json!([{
            "name": "plate",
            "steps": [
                {"op": "sketch", "profile": "rectangle", "plane": "XY",
                 "width": 10.0, "height": 10.0},
                {"op": "extrude", "length": 5.0}
            ]
        }]),
        json!([{"op": "place", "component": "plate", "x": 5.0, "y": 6.0, "z": 7.0}]),
    );
    let local = tessellate(&compile_shape(&local_script), MeshOptions::default()).unwrap();
    let placed_result = compile_json(&placed_script);
    let placed = tessellate(&placed_result.assembly_shape, MeshOptions::default()).unwrap();
    assert_eq!(local.vertex_count(), placed.vertex_count());
    assert_eq!(local.triangle_count(), placed.triangle_count());
    for index in 0..local.positions.len() {
        let axis = index % 3;
        let shift = [5.0, 6.0, 7.0][axis];
        assert!(
            (placed.positions[index] - (local.positions[index] + shift)).abs() <= 1e-9,
            "position {index} was not translated"
        );
    }
    for (left, right) in local.normals.iter().zip(placed.normals.iter()) {
        assert!(
            (left - right).abs() <= 1e-12,
            "a translation must not rotate normals"
        );
    }
    let bounds = placed.positions.chunks_exact(3).fold(
        ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]),
        |mut bounds, point| {
            for (axis, value) in point.iter().enumerate() {
                bounds.0[axis] = bounds.0[axis].min(*value);
                bounds.1[axis] = bounds.1[axis].max(*value);
            }
            bounds
        },
    );
    assert_bounds(
        &kala_cad::Bounds::new(bounds.0, bounds.1).unwrap(),
        [5.0, 6.0, 7.0],
        [15.0, 16.0, 12.0],
        1e-9,
        "placed mesh",
    );
}

#[test]
fn the_vertex_and_triangle_limits_refuse_rather_than_truncate() {
    let shape = box_shape(20.0, 20.0, 10.0);
    let vertex_limited = MeshOptions {
        max_vertices: 1,
        ..MeshOptions::default()
    };
    let error = tessellate(&shape, vertex_limited).expect_err("must refuse");
    assert_eq!(
        error.message(),
        "the mesh would exceed the 1 vertex limit; refusing to truncate it"
    );

    let triangle_limited = MeshOptions {
        max_triangles: 1,
        ..MeshOptions::default()
    };
    let error = tessellate(&shape, triangle_limited).expect_err("must refuse");
    assert_eq!(
        error.message(),
        "the mesh would exceed the 1 triangle limit; refusing to truncate it"
    );

    assert_eq!(MAX_MESH_VERTICES, 200_000);
    assert_eq!(MAX_MESH_TRIANGLES, 400_000);
}

#[test]
fn invalid_tolerances_and_null_shapes_are_refused_with_source_messages() {
    let shape = box_shape(20.0, 20.0, 10.0);
    for deflection in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let options = MeshOptions {
            deflection_mm: deflection,
            ..MeshOptions::default()
        };
        let error = tessellate(&shape, options).expect_err("must refuse");
        assert_eq!(
            error.message(),
            "the mesh deflection must be a positive finite number"
        );
    }
    for angular in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let options = MeshOptions {
            angular_deflection_rad: angular,
            ..MeshOptions::default()
        };
        let error = tessellate(&shape, options).expect_err("must refuse");
        assert_eq!(
            error.message(),
            "the angular deflection must be a positive finite number"
        );
    }
    let error =
        tessellate(&Shape::null().unwrap(), MeshOptions::default()).expect_err("must refuse");
    assert_eq!(error.message(), "the mesh needs a non-null shape");
}

#[test]
fn binary_stl_is_stable_and_matches_the_mesh_arrays() {
    let shape = box_shape(20.0, 20.0, 10.0);
    let mesh = tessellate(&shape, MeshOptions::default()).unwrap();
    let first = stl_bytes(&mesh, kala_cad::STL_HEADER_FILL).unwrap();
    let second = stl_bytes(&mesh, kala_cad::STL_HEADER_FILL).unwrap();
    assert_eq!(first, second, "STL serialisation must be deterministic");
    assert_eq!(first.len(), 80 + 4 + mesh.triangle_count() * 50);
    assert_eq!(&first[..80], &{
        let mut header = kala_cad::STL_HEADER_FILL.to_vec();
        header.resize(80, b' ');
        header
    });
    let count = u32::from_le_bytes(first[80..84].try_into().unwrap()) as usize;
    assert_eq!(count, mesh.triangle_count());

    for triangle in 0..mesh.triangle_count() {
        let offset = 84 + triangle * 50;
        let mut values = [0.0_f32; 12];
        for (index, value) in values.iter_mut().enumerate() {
            let start = offset + index * 4;
            *value = f32::from_le_bytes(first[start..start + 4].try_into().unwrap());
        }
        let attribute = u16::from_le_bytes(first[offset + 48..offset + 50].try_into().unwrap());
        assert_eq!(attribute, 0);

        let geometric = triangle_normal(&mesh.positions, &mesh.indices, triangle);
        for axis in 0..3 {
            assert!(
                (values[axis] as f64 - geometric[axis]).abs() <= 1e-6,
                "STL normal axis {axis} disagrees with the winding"
            );
        }
        for corner in 0..3 {
            let vertex = mesh.indices[triangle * 3 + corner] as usize;
            for axis in 0..3 {
                let expected = mesh.positions[vertex * 3 + axis] as f32;
                let actual = values[3 + corner * 3 + axis];
                assert_eq!(actual.to_bits(), expected.to_bits());
            }
        }
    }
}

#[test]
fn an_empty_mesh_serialises_as_a_header_and_zero_triangles() {
    let mesh = MeshData {
        positions: Vec::new(),
        normals: Vec::new(),
        indices: Vec::new(),
        deflection_mm: 0.1,
        angular_deflection_rad: 0.5,
    };
    let bytes = stl_bytes(&mesh, b"short header").unwrap();
    assert_eq!(bytes.len(), 84);
    assert_eq!(&bytes[..12], b"short header");
    assert!(bytes[12..80].iter().all(|byte| *byte == b' '));
    assert_eq!(&bytes[80..84], &0u32.to_le_bytes());
}

#[test]
fn a_malformed_hand_built_mesh_is_refused_without_panicking() {
    let malformed = MeshData {
        positions: vec![0.0, 0.0, 0.0],
        normals: vec![0.0, 0.0, 0.0],
        indices: vec![0, 1, 2],
        deflection_mm: 0.1,
        angular_deflection_rad: 0.5,
    };
    let error = stl_bytes(&malformed, b"x").expect_err("must refuse");
    assert!(error.message().contains("inconsistent"));

    let mismatched = MeshData {
        positions: vec![0.0; 9],
        normals: vec![],
        indices: vec![0, 1, 2],
        deflection_mm: 0.1,
        angular_deflection_rad: 0.5,
    };
    assert!(stl_bytes(&mismatched, b"x").is_err());
}

#[test]
fn a_long_stl_header_is_truncated_to_eighty_bytes() {
    let mesh = MeshData {
        positions: Vec::new(),
        normals: Vec::new(),
        indices: Vec::new(),
        deflection_mm: 0.1,
        angular_deflection_rad: 0.5,
    };
    let header = vec![b'x'; 100];
    let bytes = stl_bytes(&mesh, &header).unwrap();
    assert_eq!(&bytes[..80], &header[..80]);
}
