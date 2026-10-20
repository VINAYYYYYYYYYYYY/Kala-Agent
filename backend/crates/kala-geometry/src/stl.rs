//! Deterministic binary STL export for studio preview meshes.
//!
//! Layout matches `kala_agent.studio.geometry.stl`: an 80-byte ASCII-filled
//! header, a little-endian `u32` triangle count, then per triangle a 12-byte
//! geometric normal, three 12-byte `f32` vertices, and a zero `u16` attribute
//! count. The triangle normal is recomputed from positions, not copied from
//! the smooth per-vertex normal array.
//!
//! This is the only module that narrows geometry to `f32`. Everything before
//! packing is `f64`.

use crate::meshes::{Mesh, signed_face_normal};

/// Binary STL header size in bytes.
pub const STL_HEADER_SIZE: usize = 80;
/// Binary STL triangle record size in bytes.
pub const STL_TRIANGLE_SIZE: usize = 50;
/// One triangle per STL record.
pub const TRIANGLES_PER_FACE: usize = 1;
/// Python struct layout for one triangle record.
pub const STL_BINARY_FORMAT: &str = "<12fH";
/// ASCII header fill, space-padded to [`STL_HEADER_SIZE`].
pub const STL_HEADER_FILL: &[u8] = b"kala-studio analytic tessellation, binary STL export";

fn push_f32_le(output: &mut Vec<u8>, value: f64) {
    output.extend_from_slice(&(value as f32).to_le_bytes());
}

/// Serialise a mesh as a deterministic binary STL byte string.
///
/// The output is byte-identical to the Python writer for the same mesh on the
/// same platform: header, little-endian triangle count, and records in mesh
/// index order. A zero-triangle mesh produces the 84-byte empty STL.
#[must_use]
pub fn write_binary_stl(mesh: &Mesh) -> Vec<u8> {
    let positions = mesh.positions();
    let indices = mesh.indices();
    let triangle_count = indices.len() / 3;
    let mut header = [b' '; STL_HEADER_SIZE];
    let fill_length = STL_HEADER_FILL.len().min(STL_HEADER_SIZE);
    header[..fill_length].copy_from_slice(&STL_HEADER_FILL[..fill_length]);

    let mut output = Vec::with_capacity(STL_HEADER_SIZE + 4 + triangle_count * STL_TRIANGLE_SIZE);
    output.extend_from_slice(&header);
    output.extend_from_slice(&(triangle_count as u32).to_le_bytes());

    for triangle in indices.chunks_exact(3) {
        let (i0, i1, i2) = (triangle[0], triangle[1], triangle[2]);
        let base0 = i0 as usize * 3;
        let base1 = i1 as usize * 3;
        let base2 = i2 as usize * 3;
        let ax = positions[base0];
        let ay = positions[base0 + 1];
        let az = positions[base0 + 2];
        let bx = positions[base1];
        let by = positions[base1 + 1];
        let bz = positions[base1 + 2];
        let cx = positions[base2];
        let cy = positions[base2 + 1];
        let cz = positions[base2 + 2];

        let (mut nx, mut ny, mut nz) = signed_face_normal(ax, ay, az, bx, by, bz, cx, cy, cz);
        let magnitude_sq = nx * nx + ny * ny + nz * nz;
        if magnitude_sq > 0.0 {
            // Python uses ``magnitude_sq**0.5`` here, not math.sqrt, so keep
            // the pow call for bit parity.
            let inverse = 1.0 / magnitude_sq.powf(0.5);
            nx *= inverse;
            ny *= inverse;
            nz *= inverse;
        }

        push_f32_le(&mut output, nx);
        push_f32_le(&mut output, ny);
        push_f32_le(&mut output, nz);
        push_f32_le(&mut output, ax);
        push_f32_le(&mut output, ay);
        push_f32_le(&mut output, az);
        push_f32_le(&mut output, bx);
        push_f32_le(&mut output, by);
        push_f32_le(&mut output, bz);
        push_f32_le(&mut output, cx);
        push_f32_le(&mut output, cy);
        push_f32_le(&mut output, cz);
        output.extend_from_slice(&0_u16.to_le_bytes());
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_mesh_writes_header_and_zero_count() {
        let stl = write_binary_stl(&Mesh::empty());
        assert_eq!(stl.len(), STL_HEADER_SIZE + 4);
        assert_eq!(&stl[..STL_HEADER_FILL.len()], STL_HEADER_FILL);
        assert!(
            stl[STL_HEADER_FILL.len()..STL_HEADER_SIZE]
                .iter()
                .all(|byte| *byte == b' ')
        );
        assert_eq!(&stl[STL_HEADER_SIZE..], &[0, 0, 0, 0]);
    }

    #[test]
    fn default_box_layout_is_exact() {
        let mesh =
            crate::meshes::build_spec(&crate::primitives::PrimitiveSpec::default_box()).unwrap();
        let stl = write_binary_stl(&mesh);
        assert_eq!(
            stl.len(),
            STL_HEADER_SIZE + 4 + mesh.triangle_count() * STL_TRIANGLE_SIZE
        );
        assert_eq!(
            u32::from_le_bytes(
                stl[STL_HEADER_SIZE..STL_HEADER_SIZE + 4]
                    .try_into()
                    .unwrap()
            ),
            mesh.triangle_count() as u32
        );
        for record in stl[STL_HEADER_SIZE + 4..].chunks_exact(STL_TRIANGLE_SIZE) {
            assert_eq!(&record[48..50], &[0, 0]);
        }
        assert_eq!(stl, write_binary_stl(&mesh));
    }
}
