//! Exact analytic B-rep model and its consistency audit.
//!
//! The writer authors an ISO 10303-21 file directly instead of driving a C++
//! kernel. To keep that honest, every primitive is first described as an
//! explicit boundary model — vertices, edge curves, face surfaces and the
//! oriented edge loops that bound each face — the model is audited, and only
//! then serialised.
//!
//! The audit enforces the invariants of a closed, consistently oriented,
//! genus-0 solid:
//!
//! * every edge loop closes (each oriented edge starts where the previous one
//!   ended);
//! * every edge is used exactly twice across the whole shell, once in each
//!   direction;
//! * every vertex belongs to at least one edge, except for the single
//!   degenerate vertex loop of a fully closed spherical face;
//! * `V - E + F = 2`.
//!
//! The independent verification is the test-only OpenCascade re-import in
//! `tests/ocp_oracle.py`; this audit catches construction mistakes earlier and
//! without a kernel.

use crate::error::StepError;
use crate::serialize::Document;

/// A finite 3D point in millimetres.
pub(crate) type Point = [f64; 3];

/// An `AXIS2_PLACEMENT_3D`: origin plus unit axis and reference direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Placement {
    pub(crate) origin: Point,
    pub(crate) axis: Point,
    pub(crate) reference: Point,
}

/// The 3D curve of one edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Curve {
    /// An unbounded line used between two distinct vertices.
    Line { origin: Point, direction: Point },
    /// A full circle; the edge is closed and its two vertices coincide.
    Circle { placement: Placement, radius: f64 },
}

/// The analytic surface of one face.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Surface {
    Plane { placement: Placement },
    Cylinder { placement: Placement, radius: f64 },
    Sphere { placement: Placement, radius: f64 },
}

/// One edge with its canonical direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct EdgeDef {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) curve: Curve,
}

/// One use of an edge inside a face boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EdgeUse {
    pub(crate) edge: usize,
    pub(crate) forward: bool,
}

/// The boundary of one face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Boundary {
    /// A closed loop of edge uses.
    EdgeLoop(&'static [EdgeUse]),
    /// The single vertex that closes a fully closed surface, used for a
    /// complete spherical face.
    VertexLoop(usize),
}

/// One face with its surface and boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FaceDef {
    pub(crate) surface: Surface,
    pub(crate) boundary: Boundary,
}

/// A complete primitive solid.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Model {
    /// Vertex positions; the vertex index is the position in this list.
    pub(crate) vertices: Vec<Point>,
    pub(crate) edges: Vec<EdgeDef>,
    pub(crate) faces: Vec<FaceDef>,
}

impl Model {
    /// Audit the boundary model.
    ///
    /// # Errors
    ///
    /// Returns a sanitized internal-consistency message.
    pub(crate) fn audit(&self) -> Result<(), String> {
        for (index, vertex) in self.vertices.iter().enumerate() {
            if vertex.iter().any(|value| !value.is_finite()) {
                return Err(format!("vertex {index} is not finite"));
            }
        }
        if self.vertices.is_empty() || self.faces.is_empty() {
            return Err("the primitive boundary is empty".to_owned());
        }
        for (index, edge) in self.edges.iter().enumerate() {
            if edge.start >= self.vertices.len() || edge.end >= self.vertices.len() {
                return Err(format!("edge {index} references an unknown vertex"));
            }
        }

        let mut balance: Vec<i32> = vec![0; self.edges.len()];
        let mut used_vertices = vec![false; self.vertices.len()];
        let mut vertex_loops: Vec<usize> = Vec::new();

        for (index, face) in self.faces.iter().enumerate() {
            let loop_uses = match face.boundary {
                Boundary::EdgeLoop(loop_uses) => loop_uses,
                Boundary::VertexLoop(vertex) => {
                    vertex_loops.push(vertex);
                    continue;
                }
            };
            if loop_uses.is_empty() {
                return Err(format!("face {index} has an empty boundary"));
            }
            let mut cursor: Option<usize> = None;
            let mut first: Option<usize> = None;
            for edge_use in loop_uses {
                let defined = self
                    .edges
                    .get(edge_use.edge)
                    .ok_or_else(|| format!("face {index} uses an unknown edge"))?;
                let (entry, exit) = if edge_use.forward {
                    (defined.start, defined.end)
                } else {
                    (defined.end, defined.start)
                };
                used_vertices[entry] = true;
                used_vertices[exit] = true;
                match cursor {
                    Some(cursor) if cursor != entry => {
                        return Err(format!("face {index} boundary is not closed"));
                    }
                    None => first = Some(entry),
                    Some(_) => {}
                }
                cursor = Some(exit);
                balance[edge_use.edge] += if edge_use.forward { 1 } else { -1 };
            }
            if cursor != first {
                return Err(format!("face {index} boundary is not closed"));
            }
        }

        for (index, value) in balance.iter().enumerate() {
            if *value != 0 {
                return Err(format!("edge {index} is not shared in both directions"));
            }
        }

        // A single fully closed face may close on one degenerate vertex loop
        // instead of edges (the complete sphere). Anything else with a vertex
        // loop would not describe a solid.
        if !vertex_loops.is_empty() {
            if !self.edges.is_empty() {
                return Err("a vertex loop is only valid for an edgeless face".to_owned());
            }
            if self.faces.len() != 1 || vertex_loops.len() != 1 {
                return Err("the vertex loop does not name one vertex".to_owned());
            }
            if vertex_loops[0] >= self.vertices.len() {
                return Err("the vertex loop does not name one vertex".to_owned());
            }
            return euler(self);
        }

        if let Some(unused) = used_vertices.iter().position(|used| !used) {
            return Err(format!("vertex {unused} is not used by any edge"));
        }
        euler(self)
    }

    /// Audit, serialise, and verify the finished ISO 10303-21 document.
    ///
    /// # Errors
    ///
    /// Returns [`StepError::Export`] when the model is inconsistent or a value
    /// cannot be represented as STEP geometry.
    pub(crate) fn into_bytes(self, product: &str) -> Result<Vec<u8>, StepError> {
        self.audit().map_err(StepError::Export)?;

        let mut document = Document::new();
        let mut vertex_ids = Vec::with_capacity(self.vertices.len());
        for vertex in &self.vertices {
            let point = document.point(*vertex)?;
            vertex_ids.push(document.vertex_point(point));
        }

        let mut edge_ids = Vec::with_capacity(self.edges.len());
        for edge in &self.edges {
            let curve = match edge.curve {
                Curve::Line { origin, direction } => {
                    let origin = document.point(origin)?;
                    let direction = document.direction(direction)?;
                    document.line(origin, direction)
                }
                Curve::Circle { placement, radius } => {
                    let placement = placement_id(&mut document, placement)?;
                    document.circle(placement, radius)?
                }
            };
            edge_ids.push(document.edge_curve(
                vertex_ids[edge.start],
                vertex_ids[edge.end],
                curve,
                true,
            ));
        }

        let mut face_ids = Vec::with_capacity(self.faces.len());
        for face in &self.faces {
            let surface = match face.surface {
                Surface::Plane { placement } => {
                    let placement = placement_id(&mut document, placement)?;
                    document.plane(placement)
                }
                Surface::Cylinder { placement, radius } => {
                    let placement = placement_id(&mut document, placement)?;
                    document.cylindrical_surface(placement, radius)?
                }
                Surface::Sphere { placement, radius } => {
                    let placement = placement_id(&mut document, placement)?;
                    document.spherical_surface(placement, radius)?
                }
            };
            let bound = match face.boundary {
                Boundary::EdgeLoop(uses) => {
                    let oriented: Vec<u32> = uses
                        .iter()
                        .map(|edge_use| {
                            document.oriented_edge(edge_ids[edge_use.edge], edge_use.forward)
                        })
                        .collect();
                    let loop_id = document.edge_loop(&oriented);
                    document.face_bound(loop_id)
                }
                Boundary::VertexLoop(vertex) => {
                    let loop_id = document.vertex_loop(vertex_ids[vertex]);
                    document.face_bound(loop_id)
                }
            };
            let face_id = document.advanced_face(&[bound], surface, true);
            face_ids.push(face_id);
        }

        let shell = document.closed_shell(&face_ids);
        let solid = document.manifold_solid_brep(shell);
        document.into_bytes(product, solid)
    }

    /// Number of faces.
    #[cfg(test)]
    pub(crate) fn face_count(&self) -> usize {
        self.faces.len()
    }
}

fn placement_id(document: &mut Document, placement: Placement) -> Result<u32, StepError> {
    let origin = document.point(placement.origin)?;
    let axis = document.direction(placement.axis)?;
    let reference = document.direction(placement.reference)?;
    Ok(document.axis2_placement(origin, axis, reference))
}

fn euler(model: &Model) -> Result<(), String> {
    let vertices = i64::try_from(model.vertices.len()).map_err(|_| "vertex overflow")?;
    let edges = i64::try_from(model.edges.len()).map_err(|_| "edge overflow")?;
    let faces = i64::try_from(model.faces.len()).map_err(|_| "face overflow")?;
    if vertices - edges + faces != 2 {
        return Err("the solid is not a closed genus-0 shell".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod test_support {
    /// The Euler characteristic of the closed solids under test.
    pub(crate) const fn characteristic(vertices: usize, edges: usize, faces: usize) -> i64 {
        vertices as i64 - edges as i64 + faces as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_support::characteristic;

    const SQUARE: [EdgeUse; 4] = [
        EdgeUse {
            edge: 0,
            forward: true,
        },
        EdgeUse {
            edge: 1,
            forward: true,
        },
        EdgeUse {
            edge: 2,
            forward: true,
        },
        EdgeUse {
            edge: 3,
            forward: true,
        },
    ];

    #[test]
    fn a_single_open_face_is_not_a_solid() {
        let model = Model {
            vertices: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            edges: vec![
                EdgeDef {
                    start: 0,
                    end: 1,
                    curve: Curve::Line {
                        origin: [0.0, 0.0, 0.0],
                        direction: [1.0, 0.0, 0.0],
                    },
                },
                EdgeDef {
                    start: 1,
                    end: 2,
                    curve: Curve::Line {
                        origin: [1.0, 0.0, 0.0],
                        direction: [0.0, 1.0, 0.0],
                    },
                },
                EdgeDef {
                    start: 2,
                    end: 3,
                    curve: Curve::Line {
                        origin: [1.0, 1.0, 0.0],
                        direction: [-1.0, 0.0, 0.0],
                    },
                },
                EdgeDef {
                    start: 3,
                    end: 0,
                    curve: Curve::Line {
                        origin: [0.0, 1.0, 0.0],
                        direction: [0.0, -1.0, 0.0],
                    },
                },
            ],
            faces: vec![FaceDef {
                surface: Surface::Plane {
                    placement: Placement {
                        origin: [0.0, 0.0, 0.0],
                        axis: [0.0, 0.0, 1.0],
                        reference: [1.0, 0.0, 0.0],
                    },
                },
                boundary: Boundary::EdgeLoop(&SQUARE),
            }],
        };
        assert_eq!(
            model.audit().unwrap_err(),
            "edge 0 is not shared in both directions"
        );
    }

    #[test]
    fn non_finite_vertices_are_refused() {
        let model = Model {
            vertices: vec![[f64::NAN, 0.0, 0.0]],
            edges: vec![],
            faces: vec![FaceDef {
                surface: Surface::Sphere {
                    placement: Placement {
                        origin: [0.0, 0.0, 0.0],
                        axis: [0.0, 0.0, 1.0],
                        reference: [1.0, 0.0, 0.0],
                    },
                    radius: 1.0,
                },
                boundary: Boundary::VertexLoop(0),
            }],
        };
        assert_eq!(model.audit().unwrap_err(), "vertex 0 is not finite");
    }

    #[test]
    fn euler_characteristic_helper_matches_the_closed_solids() {
        // box: 8 - 12 + 6, cylinder: 2 - 3 + 3, sphere: 1 - 0 + 1.
        assert_eq!(characteristic(8, 12, 6), 2);
        assert_eq!(characteristic(2, 3, 3), 2);
        assert_eq!(characteristic(1, 0, 1), 2);
    }
}
