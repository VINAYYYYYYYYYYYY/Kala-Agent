//! ISO 10303-21 document assembly.
//!
//! Entities are interned by their exact body text, so shared points,
//! directions and vertices are emitted once and the numbering is a pure
//! function of the insertion order: identical inputs always produce identical
//! bytes, with no clock, no filesystem and no kernel state.
//!
//! All emitted geometry is analytic: `CARTESIAN_POINT`, `DIRECTION`,
//! `LINE`, `CIRCLE`, `PLANE`, `CYLINDRICAL_SURFACE` and `SPHERICAL_SURFACE`.
//! There is no tessellation entity anywhere in the document.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::error::StepError;
use crate::model::Point;

/// The schema identifier used for an AP214 automotive-design part file.
pub(crate) const FILE_SCHEMA: &str = "AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }";

/// The `FILE_NAME` timestamp.
///
/// The writer is deterministic by design: dates and entity ordering are not a
/// contract for STEP consumers, and a fixed placeholder keeps repeated
/// exports byte-identical for hashing and differential tests.
pub(crate) const FILE_TIMESTAMP: &str = "1970-01-01T00:00:00";

/// The exact first bytes of an ISO 10303-21 exchange file.
pub const ISO_PREFIX: &[u8] = b"ISO-10303-21;";

/// The exact final statement of an ISO 10303-21 exchange file.
pub const ISO_TRAILER: &[u8] = b"END-ISO-10303-21;";

/// The line terminator written after [`ISO_TRAILER`].
pub const ISO_TERMINATOR: &[u8] = b"\n";

/// Serialise one finite `f64` as a STEP REAL.
///
/// Rust's `Display` for `f64` prints the shortest decimal form that parses
/// back to the same bit pattern, which is exactly what a reader needs. STEP
/// requires a decimal point in every REAL, so an integral value gains one
/// (`10` becomes `10.`). Negative zero is normalised to `0.` because STEP has
/// no signed zero and `-0.` would only be noise in the geometry.
///
/// # Errors
///
/// Returns [`StepError::Export`] for a non-finite value.
pub(crate) fn format_real(value: f64) -> Result<String, StepError> {
    if !value.is_finite() {
        return Err(StepError::Export(
            "the primitive produced a non-finite coordinate".to_owned(),
        ));
    }
    if value == 0.0 {
        return Ok("0.".to_owned());
    }
    let mut text = format!("{value}");
    if !text.bytes().any(|byte| matches!(byte, b'.' | b'e' | b'E')) {
        text.push('.');
    }
    Ok(text)
}

/// An ISO 10303-21 file under construction.
#[derive(Debug, Default)]
pub(crate) struct Document {
    entities: Vec<String>,
    index: BTreeMap<String, u32>,
}

impl Document {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Number of interned entities.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entities.len()
    }

    /// Intern one entity body and return its `#`-number.
    pub(crate) fn intern(&mut self, body: impl Into<String>) -> u32 {
        let body = body.into();
        if let Some(number) = self.index.get(&body) {
            return *number;
        }
        self.entities.push(body.clone());
        let number = u32::try_from(self.entities.len()).expect("entity count fits in u32");
        self.index.insert(body, number);
        number
    }

    pub(crate) fn point(&mut self, point: Point) -> Result<u32, StepError> {
        let [x, y, z] = point;
        let (x, y, z) = (format_real(x)?, format_real(y)?, format_real(z)?);
        Ok(self.intern(format!("CARTESIAN_POINT('',({x},{y},{z}))")))
    }

    pub(crate) fn direction(&mut self, direction: Point) -> Result<u32, StepError> {
        let [x, y, z] = direction;
        let (x, y, z) = (format_real(x)?, format_real(y)?, format_real(z)?);
        Ok(self.intern(format!("DIRECTION('',({x},{y},{z}))")))
    }

    pub(crate) fn axis2_placement(&mut self, origin: u32, axis: u32, ref_direction: u32) -> u32 {
        self.intern(format!(
            "AXIS2_PLACEMENT_3D('',#{origin},#{axis},#{ref_direction})"
        ))
    }

    pub(crate) fn vertex_point(&mut self, point: u32) -> u32 {
        self.intern(format!("VERTEX_POINT('',#{point})"))
    }

    pub(crate) fn line(&mut self, origin: u32, direction: u32) -> u32 {
        let vector = self.intern(format!("VECTOR('',#{direction},1.)"));
        self.intern(format!("LINE('',#{origin},#{vector})"))
    }

    pub(crate) fn circle(&mut self, placement: u32, radius: f64) -> Result<u32, StepError> {
        let radius = format_real(radius)?;
        Ok(self.intern(format!("CIRCLE('',#{placement},{radius})")))
    }

    pub(crate) fn plane(&mut self, placement: u32) -> u32 {
        self.intern(format!("PLANE('',#{placement})"))
    }

    pub(crate) fn cylindrical_surface(
        &mut self,
        placement: u32,
        radius: f64,
    ) -> Result<u32, StepError> {
        let radius = format_real(radius)?;
        Ok(self.intern(format!("CYLINDRICAL_SURFACE('',#{placement},{radius})")))
    }

    pub(crate) fn spherical_surface(
        &mut self,
        placement: u32,
        radius: f64,
    ) -> Result<u32, StepError> {
        let radius = format_real(radius)?;
        Ok(self.intern(format!("SPHERICAL_SURFACE('',#{placement},{radius})")))
    }

    pub(crate) fn edge_curve(&mut self, start: u32, end: u32, curve: u32, same_sense: bool) -> u32 {
        let sense = if same_sense { ".T." } else { ".F." };
        self.intern(format!("EDGE_CURVE('',#{start},#{end},#{curve},{sense})"))
    }

    pub(crate) fn oriented_edge(&mut self, edge: u32, forward: bool) -> u32 {
        let sense = if forward { ".T." } else { ".F." };
        self.intern(format!("ORIENTED_EDGE('',*,*,#{edge},{sense})"))
    }

    pub(crate) fn edge_loop(&mut self, oriented_edges: &[u32]) -> u32 {
        let body = join_refs(oriented_edges);
        self.intern(format!("EDGE_LOOP('',({body}))"))
    }

    pub(crate) fn vertex_loop(&mut self, vertex: u32) -> u32 {
        self.intern(format!("VERTEX_LOOP('',#{vertex})"))
    }

    pub(crate) fn face_bound(&mut self, boundary: u32) -> u32 {
        self.intern(format!("FACE_BOUND('',#{boundary},.T.)"))
    }

    pub(crate) fn advanced_face(&mut self, bounds: &[u32], surface: u32, same_sense: bool) -> u32 {
        let sense = if same_sense { ".T." } else { ".F." };
        let body = join_refs(bounds);
        self.intern(format!("ADVANCED_FACE('',({body}),#{surface},{sense})"))
    }

    pub(crate) fn closed_shell(&mut self, faces: &[u32]) -> u32 {
        let body = join_refs(faces);
        self.intern(format!("CLOSED_SHELL('',({body}))"))
    }

    pub(crate) fn manifold_solid_brep(&mut self, shell: u32) -> u32 {
        self.intern(format!("MANIFOLD_SOLID_BREP('',#{shell})"))
    }

    /// Append the representation context, unit declarations and product
    /// skeleton, then serialise the whole document.
    ///
    /// `product` names the emitted part; `solid` is the
    /// `MANIFOLD_SOLID_BREP` entity built by the primitive.
    ///
    /// # Errors
    ///
    /// Returns [`StepError::Export`] if a coordinate could not be represented
    /// or the finished document is not a complete ISO 10303-21 file.
    pub(crate) fn into_bytes(mut self, product: &str, solid: u32) -> Result<Vec<u8>, StepError> {
        let origin = self.point([0.0, 0.0, 0.0])?;
        let axis = self.direction([0.0, 0.0, 1.0])?;
        let reference = self.direction([1.0, 0.0, 0.0])?;
        let placement = self.axis2_placement(origin, axis, reference);

        let length_unit = self.intern("( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) )");
        let plane_angle_unit =
            self.intern("( NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.) )");
        let solid_angle_unit =
            self.intern("( NAMED_UNIT(*) SI_UNIT($,.STERADIAN.) SOLID_ANGLE_UNIT() )");
        let uncertainty = self.intern(format!(
            "UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-07),#{length_unit},\
             'distance_accuracy_value','confusion accuracy')"
        ));
        let context = self.intern(format!(
            "( GEOMETRIC_REPRESENTATION_CONTEXT(3) \
             GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#{uncertainty})) \
             GLOBAL_UNIT_ASSIGNED_CONTEXT((#{length_unit},#{plane_angle_unit},\
             #{solid_angle_unit})) REPRESENTATION_CONTEXT('Context #1',\
             '3D Context with UNIT and UNCERTAINTY') )"
        ));
        let representation = self.intern(format!(
            "ADVANCED_BREP_SHAPE_REPRESENTATION('',(#{placement},#{solid}),#{context})"
        ));

        let application_context = self
            .intern("APPLICATION_CONTEXT('core data for automotive mechanical design processes')");
        self.intern(format!(
            "APPLICATION_PROTOCOL_DEFINITION('international standard',\
             'automotive_design',2000,#{application_context})"
        ));
        let product_context = self.intern(format!(
            "PRODUCT_CONTEXT('',#{application_context},'mechanical')"
        ));
        let product_definition = self.intern(format!(
            "PRODUCT('{product}','{product}','',(#{product_context}))"
        ));
        let formation = self.intern(format!(
            "PRODUCT_DEFINITION_FORMATION('','',#{product_definition})"
        ));
        let definition_context = self.intern(format!(
            "PRODUCT_DEFINITION_CONTEXT('part definition',#{application_context},'design')"
        ));
        let definition = self.intern(format!(
            "PRODUCT_DEFINITION('design','',#{formation},#{definition_context})"
        ));
        let definition_shape =
            self.intern(format!("PRODUCT_DEFINITION_SHAPE('','',#{definition})"));
        self.intern(format!(
            "SHAPE_DEFINITION_REPRESENTATION(#{definition_shape},#{representation})"
        ));
        self.intern(format!(
            "PRODUCT_RELATED_PRODUCT_CATEGORY('part',$,(#{product_definition}))"
        ));

        let mut text = String::with_capacity(self.entities.len() * 64 + 512);
        text.push_str("ISO-10303-21;\nHEADER;\n");
        text.push_str("FILE_DESCRIPTION(('Kala Studio native exact B-rep primitive'),'2;1');\n");
        let _ = writeln!(
            text,
            "FILE_NAME('kala-primitive.step','{FILE_TIMESTAMP}',\
             ('Kala Studio native STEP exporter'),('Kala Agent'),\
             'kala-step native analytic B-rep writer','kala-step 0.1.0','');"
        );
        let _ = writeln!(text, "FILE_SCHEMA(('{FILE_SCHEMA}'));");
        text.push_str("ENDSEC;\nDATA;\n");
        for (index, body) in self.entities.iter().enumerate() {
            let _ = writeln!(text, "#{} = {body};", index + 1);
        }
        text.push_str("ENDSEC;\nEND-ISO-10303-21;\n");

        let bytes = text.into_bytes();
        crate::check_document(&bytes)?;
        Ok(bytes)
    }
}

fn join_refs(ids: &[u32]) -> String {
    let mut text = String::with_capacity(ids.len() * 4);
    for (index, id) in ids.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        text.push('#');
        let _ = write!(text, "{id}");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reals_are_shortest_round_trip_with_a_decimal_point() {
        assert_eq!(format_real(0.0).unwrap(), "0.");
        assert_eq!(format_real(-0.0).unwrap(), "0.");
        assert_eq!(format_real(10.0).unwrap(), "10.");
        assert_eq!(format_real(-5.0).unwrap(), "-5.");
        assert_eq!(format_real(2.5).unwrap(), "2.5");
        assert_eq!(
            format_real(0.30000000000000004).unwrap(),
            "0.30000000000000004"
        );
        let parsed: f64 = format_real(1.4323745724554573).unwrap().parse().unwrap();
        assert_eq!(parsed.to_bits(), 1.4323745724554573_f64.to_bits());
    }

    #[test]
    fn non_finite_reals_are_refused() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(matches!(
                format_real(value),
                Err(StepError::Export(message))
                    if message == "the primitive produced a non-finite coordinate"
            ));
        }
    }

    #[test]
    fn identical_entities_are_interned_once() {
        let mut document = Document::new();
        let first = document.point([0.0, 0.0, 0.0]).unwrap();
        let second = document.point([0.0, 0.0, 0.0]).unwrap();
        assert_eq!(first, second);
        assert_eq!(document.len(), 1);
        assert!(document.point([f64::NAN, 0.0, 0.0]).is_err());
    }
}
