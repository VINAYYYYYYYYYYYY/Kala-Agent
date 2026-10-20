//! Property-style native checks on meshes and STL serialization.
//!
//! These cases are generated deterministically inside the test; they do not
//! consume or alter the fixture. They check watertightness, outward winding,
//! finite unit normals, bounds, positive volume, index validity, and that
//! every packed STL record matches the mesh arrays and a zero attribute count.

mod common;

use kala_geometry::{
    Bounds, Mesh, PrimitiveSpec, build_spec, compute_bounds, signed_volume,
    welded_edges_by_position_pairs, write_binary_stl,
};

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next_u64(&mut self) -> u64 {
        // xorshift64*: deterministic across platforms, no external crates.
        let mut state = self.0;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        self.0 = state;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn unit(&mut self) -> f64 {
        // 53 random bits form a value in [0, 1).
        (self.next_u64() >> 11) as f64 / ((1_u64 << 53) as f64)
    }

    fn dimension(&mut self) -> f64 {
        0.1 + self.unit() * (2000.0 - 0.1)
    }

    fn count(&mut self, minimum: u32, maximum: u32) -> u32 {
        minimum + (self.next_u64() % u64::from(maximum - minimum + 1)) as u32
    }
}

fn assert_finite(values: &[f64], label: &str) {
    assert!(
        values.iter().all(|value| value.is_finite()),
        "{label} contains a non-finite value"
    );
}

fn assert_outward(positions: &[f64], indices: &[u32]) {
    for (triangle_index, triangle) in indices.chunks_exact(3).enumerate() {
        let (i0, i1, i2) = (triangle[0], triangle[1], triangle[2]);
        let v = |index: u32| {
            let base = index as usize * 3;
            (positions[base], positions[base + 1], positions[base + 2])
        };
        let (ax, ay, az) = v(i0);
        let (bx, by, bz) = v(i1);
        let (cx, cy, cz) = v(i2);
        let ux = bx - ax;
        let uy = by - ay;
        let uz = bz - az;
        let vx = cx - ax;
        let vy = cy - ay;
        let vz = cz - az;
        let (nx, ny, nz) = (uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx);
        let centroid = (
            (ax + bx + cx) / 3.0,
            (ay + by + cy) / 3.0,
            (az + bz + cz) / 3.0,
        );
        let dot = nx * centroid.0 + ny * centroid.1 + nz * centroid.2;
        assert!(
            dot > 0.0,
            "triangle {triangle_index} is not outward-wound (dot {dot})"
        );
    }
}

fn assert_normals_unit(mesh: &Mesh) {
    for (vertex, normal) in mesh.normals().chunks_exact(3).enumerate() {
        let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        assert!(
            (length - 1.0).abs() <= 1e-12,
            "vertex {vertex} normal length {length}"
        );
    }
}

fn assert_stl_matches_mesh(mesh: &Mesh) {
    let positions = mesh.positions();
    let stl = write_binary_stl(mesh);
    assert_eq!(stl, write_binary_stl(mesh), "STL must be deterministic");
    assert_eq!(stl.len(), 84 + mesh.triangle_count() * 50);
    let header_count = u32::from_le_bytes(stl[80..84].try_into().unwrap());
    assert_eq!(header_count as usize, mesh.triangle_count());
    assert_eq!(&stl[..80], &{
        let mut header = [b' '; 80];
        let fill = kala_geometry::STL_HEADER_FILL;
        header[..fill.len()].copy_from_slice(fill);
        header
    });

    for (record_index, record) in stl[84..].chunks_exact(50).enumerate() {
        assert_eq!(
            &record[48..50],
            &[0, 0],
            "record {record_index} attribute count"
        );
        let mut values = [0.0_f32; 12];
        for (slot, value) in values.iter_mut().enumerate() {
            let start = slot * 4;
            *value = f32::from_le_bytes(record[start..start + 4].try_into().unwrap());
        }
        let normal = [values[0], values[1], values[2]];
        let normal_length =
            (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        assert!(
            (normal_length - 1.0).abs() <= 1e-6,
            "record {record_index} normal length {normal_length}"
        );

        let triangle = &mesh.indices()[record_index * 3..record_index * 3 + 3];
        let position = |index: u32| {
            let base = index as usize * 3;
            [positions[base], positions[base + 1], positions[base + 2]]
        };
        let [ax, ay, az] = position(triangle[0]);
        let [bx, by, bz] = position(triangle[1]);
        let [cx, cy, cz] = position(triangle[2]);
        let (ux, uy, uz) = (bx - ax, by - ay, bz - az);
        let (vx, vy, vz) = (cx - ax, cy - ay, cz - az);
        let (mut nx, mut ny, mut nz) = (uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx);
        let magnitude_sq = nx * nx + ny * ny + nz * nz;
        if magnitude_sq > 0.0 {
            let inverse = 1.0 / magnitude_sq.powf(0.5);
            nx *= inverse;
            ny *= inverse;
            nz *= inverse;
        }
        assert_eq!(
            [values[0], values[1], values[2]],
            [nx as f32, ny as f32, nz as f32],
            "record {record_index} must use the per-triangle geometric normal"
        );

        for (corner, index) in triangle.iter().enumerate() {
            let base = *index as usize * 3;
            for axis in 0..3 {
                assert_eq!(
                    values[3 + corner * 3 + axis],
                    positions[base + axis] as f32,
                    "record {record_index} corner {corner} axis {axis}"
                );
            }
        }
    }
}

fn check_box(size: [f64; 3], subdivisions: u32) {
    let spec = PrimitiveSpec::Box { size, subdivisions };
    let mesh = build_spec(&spec).unwrap();
    assert_finite(mesh.positions(), "box positions");
    assert_finite(mesh.normals(), "box normals");
    assert_eq!(mesh.positions().len() % 3, 0);
    assert_eq!(mesh.normals().len(), mesh.positions().len());
    assert_eq!(mesh.indices().len() % 3, 0);
    assert!(
        mesh.indices()
            .iter()
            .all(|index| (*index as usize) < mesh.vertex_count())
    );
    assert!(
        mesh.indices()
            .chunks_exact(3)
            .all(|triangle| triangle[0] != triangle[1]
                && triangle[1] != triangle[2]
                && triangle[0] != triangle[2])
    );
    assert_outward(mesh.positions(), mesh.indices());
    assert_normals_unit(&mesh);

    let bounds = compute_bounds(mesh.positions()).unwrap();
    assert_eq!(
        bounds,
        Bounds {
            min: [-size[0] / 2.0, -size[1] / 2.0, -size[2] / 2.0],
            max: [size[0] / 2.0, size[1] / 2.0, size[2] / 2.0],
        }
    );
    assert!(
        welded_edges_by_position_pairs(&mesh)
            .values()
            .all(|uses| *uses == 2),
        "box must be watertight"
    );
    let volume = signed_volume(&mesh);
    let expected = size[0] * size[1] * size[2];
    assert!(
        (volume - expected).abs() <= 1e-9 * expected,
        "box volume {volume} vs {expected}"
    );
    assert_stl_matches_mesh(&mesh);
}

fn check_cylinder(radius: f64, height: f64, radial_segments: u32, height_segments: u32) {
    let spec = PrimitiveSpec::Cylinder {
        radius,
        height,
        radial_segments,
        height_segments,
    };
    let mesh = build_spec(&spec).unwrap();
    assert_finite(mesh.positions(), "cylinder positions");
    assert_finite(mesh.normals(), "cylinder normals");
    assert_eq!(mesh.normals().len(), mesh.positions().len());
    assert!(
        mesh.indices()
            .iter()
            .all(|index| (*index as usize) < mesh.vertex_count())
    );
    assert_outward(mesh.positions(), mesh.indices());
    assert_normals_unit(&mesh);

    let bounds = compute_bounds(mesh.positions()).unwrap();
    assert_eq!(bounds.min[2], -height / 2.0);
    assert_eq!(bounds.max[2], height / 2.0);
    assert_eq!(bounds.max[0], radius);
    for axis in 0..2 {
        assert!(bounds.min[axis] >= -radius);
        assert!(bounds.max[axis] <= radius);
    }
    assert!(
        welded_edges_by_position_pairs(&mesh)
            .values()
            .all(|uses| *uses == 2),
        "cylinder must be watertight"
    );
    let volume = signed_volume(&mesh);
    let ideal = std::f64::consts::PI * radius * radius * height;
    assert!(volume > 0.0 && volume < ideal * 1.000_000_1);
    assert_stl_matches_mesh(&mesh);
}

fn check_sphere(radius: f64, rings: u32, sectors: u32) {
    let spec = PrimitiveSpec::Sphere {
        radius,
        rings,
        sectors,
    };
    let mesh = build_spec(&spec).unwrap();
    assert_finite(mesh.positions(), "sphere positions");
    assert_finite(mesh.normals(), "sphere normals");
    assert_eq!(mesh.normals().len(), mesh.positions().len());
    assert!(
        mesh.indices()
            .iter()
            .all(|index| (*index as usize) < mesh.vertex_count())
    );
    assert_outward(mesh.positions(), mesh.indices());
    assert_normals_unit(&mesh);

    let bounds = compute_bounds(mesh.positions()).unwrap();
    assert_eq!(bounds.min[2], -radius);
    assert_eq!(bounds.max[2], radius);
    for axis in 0..3 {
        assert!(bounds.min[axis] >= -radius);
        assert!(bounds.max[axis] <= radius);
    }
    assert!(
        welded_edges_by_position_pairs(&mesh)
            .values()
            .all(|uses| *uses == 2),
        "sphere must be watertight"
    );
    let volume = signed_volume(&mesh);
    let ideal = 4.0 / 3.0 * std::f64::consts::PI * radius * radius * radius;
    assert!(volume > 0.0 && volume < ideal * 1.000_000_1);
    assert_stl_matches_mesh(&mesh);
}

#[test]
fn minimum_tessellations_are_valid() {
    check_box([10.0, 10.0, 10.0], 1);
    check_cylinder(5.0, 10.0, 6, 1);
    check_sphere(5.0, 3, 4);
}

#[test]
fn deterministic_random_valid_requests_are_outward_and_watertight() {
    let mut rng = Rng::new(0x6b61_6c61_6765_6f6d);
    for _ in 0..8 {
        check_box(
            [rng.dimension(), rng.dimension(), rng.dimension()],
            rng.count(1, 5),
        );
    }
    for _ in 0..8 {
        check_cylinder(
            rng.dimension(),
            rng.dimension(),
            rng.count(6, 12),
            rng.count(1, 4),
        );
    }
    for _ in 0..8 {
        check_sphere(rng.dimension(), rng.count(3, 8), rng.count(4, 10));
    }
}

#[test]
fn maximum_fixture_tessellations_stay_below_resource_caps() {
    for spec in [
        PrimitiveSpec::Box {
            size: [10.0; 3],
            subdivisions: kala_geometry::MAX_BOX_EDGE_SUBDIVISIONS,
        },
        PrimitiveSpec::Cylinder {
            radius: 5.0,
            height: 10.0,
            radial_segments: kala_geometry::MAX_CYLINDER_RADIAL_SEGMENTS,
            height_segments: kala_geometry::MAX_CYLINDER_HEIGHT_SEGMENTS,
        },
        PrimitiveSpec::Sphere {
            radius: 5.0,
            rings: kala_geometry::MAX_SPHERE_RINGS,
            sectors: kala_geometry::MAX_SPHERE_SECTORS,
        },
    ] {
        let mesh = build_spec(&spec).unwrap();
        assert!(mesh.vertex_count() <= kala_geometry::MAX_MESH_VERTICES);
        assert!(mesh.triangle_count() <= kala_geometry::MAX_MESH_TRIANGLES);
        assert!(
            welded_edges_by_position_pairs(&mesh)
                .values()
                .all(|uses| *uses == 2),
            "{spec:?} must stay watertight at maximum tessellation"
        );
    }
}
