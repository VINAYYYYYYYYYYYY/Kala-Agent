//! Hidden-line drawing and section geometry: policy and plain result data.
//!
//! This module mirrors `src/kala_agent/studio/design/drawing.py` exactly where
//! the behaviour is kernel independent: the four view bases, the three section
//! planes, the chord tolerance and point limits, the validation messages, and
//! the frozen result shapes. The native kernel calls live in
//! [`crate::native::drawing`]; this module is also compiled in the explicit
//! unavailable mode so callers and tests can share one set of result types.
//!
//! The divisions of responsibility follow the Python module:
//!
//! * **View-plane millimetres.** Every polyline and loop is a chain of
//!   `(u, v)` millimetres. `front` looks from `-Y` with `+X` to the right and
//!   `+Z` up, `top` from `+Z` with `+X` right and `+Y` up, `right` from `+X`
//!   with `+Y` right and `+Z` up, and `iso` from `(+X, -Y, +Z)` with `+Z` up.
//! * **Chord tolerance.** Views default to 0.05 mm and sections to 0.01 mm;
//!   the tolerance bounds the drawn polyline only. Section areas are measured
//!   on the exact faces of the cut, never on the sampled polygon.
//! * **Point caps.** A view keeps at most 20,000 points and reports
//!   `truncated = true` when a line did not fit. A section keeps at most
//!   20,000 points across its loops and reports the same flag.
//!
//! Nothing here holds an OpenCascade object: every result is plain data and is
//! `Send + Sync`, so an HTTP or journal layer can serialise it without the
//! kernel.

use std::fmt;

#[cfg(any(feature = "native", test))]
use crate::error::DrawingError;

/// Default chord tolerance of an HLR view, from ADR-0009.
pub const DEFAULT_CHORD_TOLERANCE_MM: f64 = 0.05;
/// Default chord tolerance of a section cut.
pub const DEFAULT_SECTION_CHORD_TOLERANCE_MM: f64 = 0.01;
/// Maximum number of points kept per view.
pub const MAX_POINTS_PER_VIEW: usize = 20_000;
/// Maximum number of points kept across the loops of one section.
pub const MAX_SECTION_POINTS: usize = 20_000;
/// The four views the Studio sheet draws, in drawing order.
pub const DEFAULT_VIEW_NAMES: [&str; 4] = ["front", "top", "right", "iso"];

/// Areas at or below this magnitude are treated as zero.
pub(crate) const AREA_EPSILON_MM2: f64 = 1e-9;
/// Relative slack when a measured area is compared with the loops it is built
/// from.
#[cfg(feature = "native")]
pub(crate) const AREA_CONSISTENCY_RATIO: f64 = 1e-6;

/// One of the four Studio drawing views.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ViewName {
    /// Looks from `-Y`, `+X` to the right and `+Z` up.
    Front,
    /// Looks from `+Z`, `+X` to the right and `+Y` up.
    Top,
    /// Looks from `+X`, `+Y` to the right and `+Z` up.
    Right,
    /// Looks from `(+X, -Y, +Z)` with `+Z` up.
    Iso,
}

impl ViewName {
    /// The lower-case view name used by the Python module and the HTTP schema.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Front => "front",
            Self::Top => "top",
            Self::Right => "right",
            Self::Iso => "iso",
        }
    }

    /// Parse a view name exactly (the Python module is case sensitive here).
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "front" => Some(Self::Front),
            "top" => Some(Self::Top),
            "right" => Some(Self::Right),
            "iso" => Some(Self::Iso),
            _ => None,
        }
    }

    /// The direction from the part towards the viewer and the horizontal axis
    /// of the view plane. The vertical axis is their cross product, so every
    /// view basis is right handed.
    #[cfg(feature = "native")]
    fn basis(self) -> ([f64; 3], [f64; 3]) {
        match self {
            Self::Front => ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0]),
            Self::Top => ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
            Self::Right => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            Self::Iso => {
                let third = 1.0 / 3.0_f64.sqrt();
                let half = 1.0 / 2.0_f64.sqrt();
                ([third, -third, third], [half, half, 0.0])
            }
        }
    }
}

impl fmt::Display for ViewName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One of the three principal section planes.
///
/// The plane is normal to the third axis of the name: `XZ` is the plane
/// normal to `Y`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SectionPlane {
    /// Normal to `Z`; `(u, v) = (x, y)`.
    Xy,
    /// Normal to `Y`; `(u, v) = (x, z)`.
    Xz,
    /// Normal to `X`; `(u, v) = (y, z)`.
    Yz,
}

impl SectionPlane {
    /// The upper-case plane name used by the Python module and the HTTP
    /// schema.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Xy => "XY",
            Self::Xz => "XZ",
            Self::Yz => "YZ",
        }
    }

    /// Parse a plane name case-insensitively, like Python's `str(plane).upper()`.
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_uppercase().as_str() {
            "XY" => Some(Self::Xy),
            "XZ" => Some(Self::Xz),
            "YZ" => Some(Self::Yz),
            _ => None,
        }
    }

    /// The plane normal and the plane's own horizontal and vertical axes.
    #[cfg(feature = "native")]
    fn basis(self) -> ([f64; 3], [f64; 3], [f64; 3]) {
        match self {
            Self::Xy => ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            Self::Xz => ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            Self::Yz => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        }
    }
}

impl fmt::Display for SectionPlane {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A point in view-plane or section-plane millimetres.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point2 {
    /// Horizontal coordinate.
    pub u: f64,
    /// Vertical coordinate.
    pub v: f64,
}

impl Point2 {
    /// Build a plane point.
    pub const fn new(u: f64, v: f64) -> Self {
        Self { u, v }
    }

    /// Euclidean distance to another plane point.
    pub fn distance(self, other: Self) -> f64 {
        (other.u - self.u).hypot(other.v - self.v)
    }

    /// The point as `[u, v]`.
    pub fn to_array(self) -> [f64; 2] {
        [self.u, self.v]
    }
}

/// An axis-aligned bounding box in plane millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds2D {
    /// Minimum horizontal coordinate.
    pub min_u: f64,
    /// Minimum vertical coordinate.
    pub min_v: f64,
    /// Maximum horizontal coordinate.
    pub max_u: f64,
    /// Maximum vertical coordinate.
    pub max_v: f64,
}

impl Bounds2D {
    /// Build a box from its four edges.
    pub const fn new(min_u: f64, min_v: f64, max_u: f64, max_v: f64) -> Self {
        Self {
            min_u,
            min_v,
            max_u,
            max_v,
        }
    }

    /// Extent along the horizontal axis.
    pub fn width(self) -> f64 {
        self.max_u - self.min_u
    }

    /// Extent along the vertical axis.
    pub fn height(self) -> f64 {
        self.max_v - self.min_v
    }

    /// Horizontal centre.
    pub fn center_u(self) -> f64 {
        (self.min_u + self.max_u) / 2.0
    }

    /// Vertical centre.
    pub fn center_v(self) -> f64 {
        (self.min_v + self.max_v) / 2.0
    }
}

/// One drawn line: a chain of view-plane points.
///
/// `closed` is true when the chain returns to its first point. The closing
/// point is included in `points`, so a consumer can stroke the loop without a
/// special case.
#[derive(Clone, Debug, PartialEq)]
pub struct Polyline {
    /// The points of the chain, in millimetres.
    pub points: Vec<Point2>,
    /// Whether the chain returns to its first point.
    pub closed: bool,
}

impl Polyline {
    /// Build a polyline.
    pub fn new(points: Vec<Point2>, closed: bool) -> Self {
        Self { points, closed }
    }

    /// The number of points in the polyline.
    pub fn point_count(&self) -> usize {
        self.points.len()
    }

    /// The bounding box, or `None` for an empty polyline.
    pub fn bounds(&self) -> Option<Bounds2D> {
        bounds_of(&self.points)
    }

    /// The length of the chain in millimetres.
    pub fn length_mm(&self) -> f64 {
        self.points
            .windows(2)
            .map(|pair| pair[0].distance(pair[1]))
            .sum()
    }
}

/// The visible and hidden pencil lines of one view.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewDrawing {
    /// Which view this is.
    pub name: ViewName,
    /// The visible lines, in drawing order.
    pub visible: Vec<Polyline>,
    /// The hidden lines, in drawing order.
    pub hidden: Vec<Polyline>,
    /// Whether the point cap dropped a line.
    pub truncated: bool,
}

impl ViewDrawing {
    /// The total number of points in the view.
    pub fn point_count(&self) -> usize {
        self.visible
            .iter()
            .map(Polyline::point_count)
            .sum::<usize>()
            + self.hidden.iter().map(Polyline::point_count).sum::<usize>()
    }

    /// The total number of polylines in the view.
    pub fn polyline_count(&self) -> usize {
        self.visible.len() + self.hidden.len()
    }
}

/// The views of one shape, in the order they were requested.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawing {
    /// The requested views.
    pub views: Vec<ViewDrawing>,
    /// The chord tolerance the views were drawn with.
    pub chord_tolerance_mm: f64,
    /// The point cap applied to every view.
    pub max_points: usize,
}

impl Drawing {
    /// Return one view by its lower-case name.
    pub fn view(&self, name: &str) -> Option<&ViewDrawing> {
        self.views.iter().find(|view| view.name.as_str() == name)
    }

    /// Return one view by its typed name.
    pub fn view_named(&self, name: ViewName) -> Option<&ViewDrawing> {
        self.views.iter().find(|view| view.name == name)
    }

    /// The names of the views in drawing order.
    pub fn view_names(&self) -> Vec<ViewName> {
        self.views.iter().map(|view| view.name).collect()
    }

    /// The total number of points across every view.
    pub fn total_point_count(&self) -> usize {
        self.views.iter().map(ViewDrawing::point_count).sum()
    }
}

/// One closed loop of a section cut.
///
/// `points` are the loop in the plane's own coordinates: for `XZ` they are
/// `(x, z)` in millimetres. They are the drawing of the loop, not the exact
/// curve.
///
/// `area_mm2` is always positive. It is measured by OpenCascade on the exact
/// planar face of the loop when `area_exact` is true; otherwise it is the area
/// of the drawn polygon and is explicitly an approximation.
///
/// `outer` is true for a boundary of the cut material and false for a hole
/// inside it. A loop inside a hole is material again and is `outer`.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionLoop {
    /// The loop in plane coordinates.
    pub points: Vec<Point2>,
    /// The enclosed area in square millimetres, always positive.
    pub area_mm2: f64,
    /// Whether this loop bounds material.
    pub outer: bool,
    /// Whether `area_mm2` is a kernel measurement of the exact face.
    pub area_exact: bool,
}

impl SectionLoop {
    /// The bounding box, or `None` for an empty loop.
    pub fn bounds(&self) -> Option<Bounds2D> {
        bounds_of(&self.points)
    }

    /// The area centroid in plane coordinates, or `None` for an empty loop.
    /// A loop with no enclosed area falls back to the centre of its bounds.
    pub fn centroid(&self) -> Option<Point2> {
        centroid_of(&self.points)
    }

    /// The number of points in the loop.
    pub fn point_count(&self) -> usize {
        self.points.len()
    }
}

/// The closed loops of one section cut.
///
/// `area_mm2` is the net enclosed area of the cut, holes removed: the area
/// hatching is drawn into. It is measured on the exact planar face of the net
/// region when `area_exact` is true. The flag is false when the kernel could
/// not build the face, when the face did not agree with the loops, or when
/// the point cap dropped a loop and the cut is therefore incomplete.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    /// The section plane.
    pub plane: SectionPlane,
    /// The position of the plane along its normal, in millimetres.
    pub offset_mm: f64,
    /// The closed loops of the cut, ordered by measured area (largest first).
    pub loops: Vec<SectionLoop>,
    /// The net enclosed area in square millimetres.
    pub area_mm2: f64,
    /// The chord tolerance the loops were drawn with.
    pub chord_tolerance_mm: f64,
    /// Whether the point cap dropped a loop.
    pub truncated: bool,
    /// Whether `area_mm2` is a kernel measurement of the exact net face.
    pub area_exact: bool,
}

impl Section {
    /// Iterate over the boundaries of the cut material.
    pub fn outer_loops(&self) -> impl Iterator<Item = &SectionLoop> + '_ {
        self.loops.iter().filter(|loop_| loop_.outer)
    }

    /// Iterate over the holes inside the cut material.
    pub fn inner_loops(&self) -> impl Iterator<Item = &SectionLoop> + '_ {
        self.loops.iter().filter(|loop_| !loop_.outer)
    }

    /// The number of material boundaries.
    pub fn outer_loop_count(&self) -> usize {
        self.outer_loops().count()
    }

    /// The number of holes.
    pub fn inner_loop_count(&self) -> usize {
        self.inner_loops().count()
    }

    /// The total number of points in the section.
    pub fn point_count(&self) -> usize {
        self.loops.iter().map(SectionLoop::point_count).sum()
    }
}

/// The view basis: direction towards the viewer and the horizontal axis.
#[cfg(feature = "native")]
pub(crate) fn view_basis(name: ViewName) -> ([f64; 3], [f64; 3]) {
    name.basis()
}

/// The section basis: normal, horizontal axis and vertical axis.
#[cfg(feature = "native")]
pub(crate) fn section_basis(plane: SectionPlane) -> ([f64; 3], [f64; 3], [f64; 3]) {
    plane.basis()
}

/// Validate a chord tolerance with the exact Python refusal message.
#[cfg(any(feature = "native", test))]
pub(crate) fn validate_chord_tolerance(value: f64) -> Result<f64, DrawingError> {
    if !value.is_finite() || value <= 0.0 {
        return Err(DrawingError::argument(format!(
            "the chord tolerance must be a positive finite number: {}",
            python_repr_f64(value)
        )));
    }
    Ok(value)
}

/// Parse a view name with the exact Python refusal message for an unknown one.
///
/// The Python message sorts its known names, which is `['front', 'iso',
/// 'right', 'top']` for the four Studio views.
#[cfg(any(feature = "native", test))]
pub(crate) fn parse_view_name(value: &str) -> Result<ViewName, DrawingError> {
    ViewName::parse(value).ok_or_else(|| {
        DrawingError::argument(format!(
            "unknown view {}; expected one of ['front', 'iso', 'right', 'top']",
            python_repr_str(value)
        ))
    })
}

/// Parse a section plane with the exact Python refusal message for an unknown
/// one. The input is case insensitive, like Python's `str(plane).upper()`.
#[cfg(any(feature = "native", test))]
pub(crate) fn parse_section_plane(value: &str) -> Result<SectionPlane, DrawingError> {
    SectionPlane::parse(value).ok_or_else(|| {
        DrawingError::argument(format!(
            "unknown section plane {}; expected one of ['XY', 'XZ', 'YZ']",
            python_repr_str(value)
        ))
    })
}

/// The bounding box of a chain of plane points.
pub(crate) fn bounds_of(points: &[Point2]) -> Option<Bounds2D> {
    let first = points.first()?;
    let mut min_u = first.u;
    let mut min_v = first.v;
    let mut max_u = first.u;
    let mut max_v = first.v;
    for point in &points[1..] {
        min_u = min_u.min(point.u);
        min_v = min_v.min(point.v);
        max_u = max_u.max(point.u);
        max_v = max_v.max(point.v);
    }
    Some(Bounds2D::new(min_u, min_v, max_u, max_v))
}

/// The signed area of a closed chain of plane points.
pub(crate) fn signed_area(points: &[Point2]) -> f64 {
    let count = points.len();
    if count < 3 {
        return 0.0;
    }
    let mut total = 0.0;
    for index in 0..count {
        let current = points[index];
        let following = points[(index + 1) % count];
        total += current.u * following.v - following.u * current.v;
    }
    total / 2.0
}

/// The area centroid of a closed chain of plane points.
///
/// A chain with no enclosed area (or fewer than three points) falls back to
/// the centre of its bounds, exactly like the Python module.
pub(crate) fn centroid_of(points: &[Point2]) -> Option<Point2> {
    let bounds = bounds_of(points)?;
    let area = signed_area(points);
    if area.abs() <= AREA_EPSILON_MM2 {
        return Some(Point2::new(bounds.center_u(), bounds.center_v()));
    }
    let count = points.len();
    let mut sum_u = 0.0;
    let mut sum_v = 0.0;
    for index in 0..count {
        let current = points[index];
        let following = points[(index + 1) % count];
        let cross = current.u * following.v - following.u * current.v;
        sum_u += (current.u + following.u) * cross;
        sum_v += (current.v + following.v) * cross;
    }
    Some(Point2::new(sum_u / (6.0 * area), sum_v / (6.0 * area)))
}

/// Whether a point lies inside a closed polygon, by the even-odd rule.
#[cfg(any(feature = "native", test))]
pub(crate) fn point_in_polygon(point: Point2, polygon: &[Point2]) -> bool {
    let count = polygon.len();
    if count < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = count - 1;
    for index in 0..count {
        let (x_i, y_i) = (polygon[index].u, polygon[index].v);
        let (x_j, y_j) = (polygon[previous].u, polygon[previous].v);
        if (y_i > point.v) != (y_j > point.v) {
            let crossing = (x_j - x_i) * (point.v - y_i) / (y_j - y_i) + x_i;
            if point.u < crossing {
                inside = !inside;
            }
        }
        previous = index;
    }
    inside
}

/// Python's `repr()` for the finite and non-finite floats this module formats.
#[cfg(any(feature = "native", test))]
///
/// Rust's `Debug` for `f64` already emits the shortest round-trip decimal with
/// Python's scientific-notation thresholds (`1e-5` and `1e16`); the only
/// difference is the exponent spelling, which Python pads to two digits and
/// always signs (`1e-07`, `1e+16`).
pub(crate) fn python_repr_f64(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value == f64::INFINITY {
        return "inf".to_owned();
    }
    if value == f64::NEG_INFINITY {
        return "-inf".to_owned();
    }
    let negative = value.is_sign_negative();
    let text = format!("{:?}", value.abs());
    let body = if let Some((mantissa, exponent)) = text.split_once('e') {
        let exponent: i32 = exponent.parse().unwrap_or(0);
        format!("{mantissa}e{exponent:+03}")
    } else if text.contains('.') {
        text
    } else {
        format!("{text}.0")
    };
    if negative { format!("-{body}") } else { body }
}

/// Python's `repr()` for the short view/plane names this module refuses.
///
/// The Studio API only ever passes short ASCII names here. Backslashes,
/// quotes and the common control escapes are handled so the refusal text does
/// not depend on the caller's quoting.
#[cfg(any(feature = "native", test))]
pub(crate) fn python_repr_str(value: &str) -> String {
    let has_single = value.contains('\'');
    let has_double = value.contains('"');
    let quote = if has_single && !has_double { '"' } else { '\'' };
    let mut text = String::with_capacity(value.len() + 2);
    text.push(quote);
    for character in value.chars() {
        match character {
            '\\' => text.push_str("\\\\"),
            '\n' => text.push_str("\\n"),
            '\r' => text.push_str("\\r"),
            '\t' => text.push_str("\\t"),
            character if character == quote => {
                text.push('\\');
                text.push(character);
            }
            character if character.is_control() => {
                let code = character as u32;
                if code <= 0xff {
                    text.push_str(&format!("\\x{code:02x}"));
                } else {
                    text.push_str(&format!("\\u{code:04x}"));
                }
            }
            character => text.push(character),
        }
    }
    text.push(quote);
    text
}

#[cfg(test)]
mod tests {
    use super::{
        Bounds2D, Point2, SectionPlane, ViewName, centroid_of, parse_section_plane,
        parse_view_name, point_in_polygon, python_repr_f64, python_repr_str, signed_area,
    };

    fn square(size: f64) -> Vec<Point2> {
        vec![
            Point2::new(0.0, 0.0),
            Point2::new(size, 0.0),
            Point2::new(size, size),
            Point2::new(0.0, size),
        ]
    }

    #[test]
    fn python_float_repr_matches_the_refusal_examples() {
        assert_eq!(python_repr_f64(0.0), "0.0");
        assert_eq!(python_repr_f64(-0.0), "-0.0");
        assert_eq!(python_repr_f64(5.0), "5.0");
        assert_eq!(python_repr_f64(-1.0), "-1.0");
        assert_eq!(python_repr_f64(0.001), "0.001");
        assert_eq!(python_repr_f64(1e-5), "1e-05");
        assert_eq!(python_repr_f64(1e-7), "1e-07");
        assert_eq!(python_repr_f64(1e16), "1e+16");
        assert_eq!(python_repr_f64(1.5e20), "1.5e+20");
        assert_eq!(python_repr_f64(1e15), "1000000000000000.0");
        assert_eq!(python_repr_f64(f64::INFINITY), "inf");
        assert_eq!(python_repr_f64(f64::NEG_INFINITY), "-inf");
        assert_eq!(python_repr_f64(f64::NAN), "nan");
    }

    #[test]
    fn python_string_repr_matches_the_refusal_examples() {
        assert_eq!(python_repr_str("side"), "'side'");
        assert_eq!(python_repr_str("it's"), "\"it's\"");
        assert_eq!(python_repr_str("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(python_repr_str("a\\b"), "'a\\\\b'");
        assert_eq!(python_repr_str("\n"), "'\\n'");
    }

    #[test]
    fn validation_messages_match_the_python_module() {
        let view = parse_view_name("side").expect_err("unknown view");
        assert_eq!(
            view.message(),
            "unknown view 'side'; expected one of ['front', 'iso', 'right', 'top']"
        );
        let plane = parse_section_plane("AB").expect_err("unknown plane");
        assert_eq!(
            plane.message(),
            "unknown section plane 'AB'; expected one of ['XY', 'XZ', 'YZ']"
        );
        assert_eq!(parse_section_plane("xz").unwrap(), SectionPlane::Xz);
        assert_eq!(ViewName::parse("front"), Some(ViewName::Front));
        assert_eq!(ViewName::Iso.as_str(), "iso");
    }

    #[test]
    fn plane_geometry_helpers_are_exact() {
        let points = square(2.0);
        assert_eq!(signed_area(&points), 4.0);
        let centroid = centroid_of(&points).unwrap();
        assert_eq!(centroid, Point2::new(1.0, 1.0));
        assert!(point_in_polygon(Point2::new(1.0, 1.0), &points));
        assert!(!point_in_polygon(Point2::new(3.0, 1.0), &points));
        assert_eq!(
            kala_cad_test_bounds(&points),
            Bounds2D::new(0.0, 0.0, 2.0, 2.0)
        );
    }

    fn kala_cad_test_bounds(points: &[Point2]) -> Bounds2D {
        super::bounds_of(points).expect("non-empty")
    }
}
