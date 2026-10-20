//! Analytic B-rep construction for the three studio primitives.
//!
//! Each builder authors the exact closed solid the Python OCP path builds
//! with `BRepPrimAPI_MakeBox` / `MakeCylinder` / `MakeSphere`, using only
//! analytic surfaces and curves:
//!
//! * **box** — six planar faces, twelve line edges, eight shared vertices,
//!   centred on the origin with extents `+/- size / 2`;
//! * **cylinder** — one cylindrical lateral face plus two planar caps, a seam
//!   line the lateral face uses twice, two full-circle edges shared with the
//!   caps, axis `+Z` from `-height / 2` to `+height / 2`, centred on the
//!   origin;
//! * **sphere** — one closed spherical face whose boundary is the degenerate
//!   vertex loop at the south pole, centred on the origin.
//!
//! Every face surface carries the outward normal in its own axis placement
//! with `same_sense = .T.`, so the orientation of a face is a property of the
//! surface frame rather than a flag. Millimetres are the file unit; the values
//! are the operator's dimension parameters, not scaled or tessellated copies.

use crate::error::StepError;
use crate::model::{Boundary, Curve, EdgeDef, EdgeUse, FaceDef, Model, Placement, Point, Surface};

/// `+X` reference direction shared by the analytic frames.
const REFERENCE_X: Point = [1.0, 0.0, 0.0];
/// `+Z` axis direction shared by the analytic frames.
const AXIS_Z: Point = [0.0, 0.0, 1.0];

// --------------------------------------------------------------------------
// box
// --------------------------------------------------------------------------

/// Corner bit triples: `0` selects the negative half extent of that axis.
const BOX_CORNERS: [[u8; 3]; 8] = [
    [0, 0, 0],
    [1, 0, 0],
    [1, 1, 0],
    [0, 1, 0],
    [0, 0, 1],
    [1, 0, 1],
    [1, 1, 1],
    [0, 1, 1],
];

/// Edge table: `(start corner, end corner)`; the line runs from start to end.
const BOX_EDGES: [(usize, usize); 12] = [
    (0, 1),
    (1, 2),
    (2, 3),
    (3, 0),
    (4, 5),
    (5, 6),
    (6, 7),
    (7, 4),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

/// Face table: counterclockwise corner loop seen from outside, outward normal,
/// in-plane reference direction, and the oriented edge uses.
///
/// The edge indices refer to [`BOX_EDGES`]: `0..4` are the bottom ring,
/// `4..8` the top ring and `8..12` the vertical edges.
struct BoxFace {
    loop_corners: [usize; 4],
    normal: Point,
    reference: Point,
    uses: [EdgeUse; 4],
}

const fn use_of(edge: usize, forward: bool) -> EdgeUse {
    EdgeUse { edge, forward }
}

const BOX_FACES: [BoxFace; 6] = [
    // bottom (z-): 0 -> 3 -> 2 -> 1, every edge travelled against its table
    // direction.
    BoxFace {
        loop_corners: [0, 3, 2, 1],
        normal: [0.0, 0.0, -1.0],
        reference: REFERENCE_X,
        uses: [
            use_of(3, false),
            use_of(2, false),
            use_of(1, false),
            use_of(0, false),
        ],
    },
    // top (z+): 4 -> 5 -> 6 -> 7.
    BoxFace {
        loop_corners: [4, 5, 6, 7],
        normal: [0.0, 0.0, 1.0],
        reference: REFERENCE_X,
        uses: [
            use_of(4, true),
            use_of(5, true),
            use_of(6, true),
            use_of(7, true),
        ],
    },
    // x-: 0 -> 4 -> 7 -> 3.
    BoxFace {
        loop_corners: [0, 4, 7, 3],
        normal: [-1.0, 0.0, 0.0],
        reference: [0.0, 1.0, 0.0],
        uses: [
            use_of(8, true),
            use_of(7, false),
            use_of(11, false),
            use_of(3, true),
        ],
    },
    // x+: 1 -> 2 -> 6 -> 5.
    BoxFace {
        loop_corners: [1, 2, 6, 5],
        normal: [1.0, 0.0, 0.0],
        reference: [0.0, 1.0, 0.0],
        uses: [
            use_of(1, true),
            use_of(10, true),
            use_of(5, false),
            use_of(9, false),
        ],
    },
    // y-: 0 -> 1 -> 5 -> 4.
    BoxFace {
        loop_corners: [0, 1, 5, 4],
        normal: [0.0, -1.0, 0.0],
        reference: [0.0, 0.0, 1.0],
        uses: [
            use_of(0, true),
            use_of(9, true),
            use_of(4, false),
            use_of(8, false),
        ],
    },
    // y+: 3 -> 7 -> 6 -> 2.
    BoxFace {
        loop_corners: [3, 7, 6, 2],
        normal: [0.0, 1.0, 0.0],
        reference: [0.0, 0.0, 1.0],
        uses: [
            use_of(11, true),
            use_of(6, false),
            use_of(10, false),
            use_of(2, true),
        ],
    },
];

fn corner_point(key: [u8; 3], half: [f64; 3]) -> Point {
    [
        if key[0] == 0 { -half[0] } else { half[0] },
        if key[1] == 0 { -half[1] } else { half[1] },
        if key[2] == 0 { -half[2] } else { half[2] },
    ]
}

/// Build the centred box solid; extents are `+/- size / 2` on every axis.
///
/// # Errors
///
/// Returns [`StepError::Export`] for a degenerate (zero-length) edge, which
/// validated dimensions cannot produce.
pub(crate) fn box_model(size_x: f64, size_y: f64, size_z: f64) -> Result<Model, StepError> {
    let half = [size_x / 2.0, size_y / 2.0, size_z / 2.0];
    let vertices: Vec<Point> = BOX_CORNERS
        .iter()
        .map(|key| corner_point(*key, half))
        .collect();

    let mut edges = Vec::with_capacity(BOX_EDGES.len());
    for (start, end) in BOX_EDGES {
        let origin = vertices[start];
        let target = vertices[end];
        let delta = [
            target[0] - origin[0],
            target[1] - origin[1],
            target[2] - origin[2],
        ];
        let length = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
        if !length.is_finite() || length == 0.0 {
            return Err(StepError::Export(
                "the box produced a degenerate edge".to_owned(),
            ));
        }
        edges.push(EdgeDef {
            start,
            end,
            curve: Curve::Line {
                origin,
                direction: [delta[0] / length, delta[1] / length, delta[2] / length],
            },
        });
    }

    let mut faces = Vec::with_capacity(BOX_FACES.len());
    for face in &BOX_FACES {
        faces.push(FaceDef {
            surface: Surface::Plane {
                placement: Placement {
                    origin: vertices[face.loop_corners[0]],
                    axis: face.normal,
                    reference: face.reference,
                },
            },
            boundary: Boundary::EdgeLoop(&face.uses),
        });
    }

    Ok(Model {
        vertices,
        edges,
        faces,
    })
}

// --------------------------------------------------------------------------
// cylinder
// --------------------------------------------------------------------------

/// Edge table for the cylinder: `0` bottom circle, `1` seam line, `2` top
/// circle.
const CYLINDER_LATERAL_USES: [EdgeUse; 4] = [
    // Top circle travelled against its own direction, down the seam, forward
    // along the bottom circle, then back up the seam: the closed rectangle in
    // the surface's parametric space whose seam is identified with itself.
    EdgeUse {
        edge: 2,
        forward: false,
    },
    EdgeUse {
        edge: 1,
        forward: false,
    },
    EdgeUse {
        edge: 0,
        forward: true,
    },
    EdgeUse {
        edge: 1,
        forward: true,
    },
];

const CYLINDER_TOP_CAP_USES: [EdgeUse; 1] = [EdgeUse {
    edge: 2,
    forward: true,
}];

const CYLINDER_BOTTOM_CAP_USES: [EdgeUse; 1] = [EdgeUse {
    edge: 0,
    forward: false,
}];

/// Build the centred solid cylinder; the axis is `+Z` from `-height / 2` to
/// `+height / 2`.
pub(crate) fn cylinder_model(radius: f64, height: f64) -> Model {
    let half = height / 2.0;
    let bottom_seam = [radius, 0.0, -half];
    let top_seam = [radius, 0.0, half];
    let bottom_center = [0.0, 0.0, -half];
    let top_center = [0.0, 0.0, half];

    let edges = vec![
        EdgeDef {
            start: 0,
            end: 0,
            curve: Curve::Circle {
                placement: Placement {
                    origin: bottom_center,
                    axis: AXIS_Z,
                    reference: REFERENCE_X,
                },
                radius,
            },
        },
        EdgeDef {
            start: 0,
            end: 1,
            curve: Curve::Line {
                origin: bottom_seam,
                direction: AXIS_Z,
            },
        },
        EdgeDef {
            start: 1,
            end: 1,
            curve: Curve::Circle {
                placement: Placement {
                    origin: top_center,
                    axis: AXIS_Z,
                    reference: REFERENCE_X,
                },
                radius,
            },
        },
    ];

    let faces = vec![
        FaceDef {
            surface: Surface::Cylinder {
                placement: Placement {
                    origin: bottom_center,
                    axis: AXIS_Z,
                    reference: REFERENCE_X,
                },
                radius,
            },
            boundary: Boundary::EdgeLoop(&CYLINDER_LATERAL_USES),
        },
        FaceDef {
            surface: Surface::Plane {
                placement: Placement {
                    origin: top_center,
                    axis: AXIS_Z,
                    reference: REFERENCE_X,
                },
            },
            boundary: Boundary::EdgeLoop(&CYLINDER_TOP_CAP_USES),
        },
        FaceDef {
            surface: Surface::Plane {
                placement: Placement {
                    origin: bottom_center,
                    axis: [0.0, 0.0, -1.0],
                    reference: REFERENCE_X,
                },
            },
            boundary: Boundary::EdgeLoop(&CYLINDER_BOTTOM_CAP_USES),
        },
    ];

    Model {
        vertices: vec![bottom_seam, top_seam],
        edges,
        faces,
    }
}

// --------------------------------------------------------------------------
// sphere
// --------------------------------------------------------------------------

/// Build the centred solid sphere.
///
/// The complete spherical surface is a single face whose boundary is the
/// degenerate vertex loop at the south pole: the topological closure the
/// OpenCascade writer also uses for a full sphere.
pub(crate) fn sphere_model(radius: f64) -> Model {
    Model {
        vertices: vec![[0.0, 0.0, -radius]],
        edges: Vec::new(),
        faces: vec![FaceDef {
            surface: Surface::Sphere {
                placement: Placement {
                    origin: [0.0, 0.0, 0.0],
                    axis: AXIS_Z,
                    reference: REFERENCE_X,
                },
                radius,
            },
            boundary: Boundary::VertexLoop(0),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_audits_and_counts_match_a_cube() {
        let model = box_model(10.0, 10.0, 10.0).unwrap();
        model.audit().unwrap();
        assert_eq!(model.vertices.len(), 8);
        assert_eq!(model.edges.len(), 12);
        assert_eq!(model.face_count(), 6);
    }

    #[test]
    fn cylinder_and_sphere_audit() {
        let cylinder = cylinder_model(5.0, 10.0);
        cylinder.audit().unwrap();
        assert_eq!(cylinder.vertices.len(), 2);
        assert_eq!(cylinder.edges.len(), 3);
        assert_eq!(cylinder.face_count(), 3);

        let sphere = sphere_model(5.0);
        sphere.audit().unwrap();
        assert_eq!(sphere.vertices.len(), 1);
        assert_eq!(sphere.edges.len(), 0);
        assert_eq!(sphere.face_count(), 1);
    }

    #[test]
    fn box_edges_are_axis_aligned_unit_directions() {
        let model = box_model(20.0, 30.0, 40.0).unwrap();
        for edge in &model.edges {
            let Curve::Line { direction, .. } = edge.curve else {
                panic!("box edges must be lines");
            };
            let length = (direction[0] * direction[0]
                + direction[1] * direction[1]
                + direction[2] * direction[2])
                .sqrt();
            assert!((length - 1.0).abs() < 1e-15);
            assert_eq!(direction.iter().filter(|value| **value != 0.0).count(), 1);
        }
    }
}
