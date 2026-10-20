#![cfg(feature = "native")]

//! Native port of `tests/unit/studio_design/test_drawing.py`.
//!
//! The shapes are built with the native construction compiler rather than with
//! direct OCP primitives, so the tests exercise the same geometry the Studio
//! design service will hand to the drawing API:
//!
//! * the ADR-0009 L-bracket: a 60 x 45 x 8 base plate, a 60 x 40 x 8 upright,
//!   a diameter-10 bore along Y at `z = 24`, and four diameter-5.5 base holes
//!   along Z at `x = +-21.75` and `y in {16.25, 36.75}`;
//! * a 40 x 30 x 20 box with a diameter-10 through hole along Y;
//! * a 40 x 30 x 24 box with a diameter-20 bore and a diameter-8 island inside
//!   it. The box is 24 mm tall, not 20 mm: in the Python test the 20 mm bore
//!   is exactly tangent to the top and bottom faces, which splits the compiler
//!   solid in two and lets the section wire run along the tangency; the
//!   non-tangent box exercises the same three-loop nesting without that
//!   degenerate topology. The exact Python 20 mm shape is covered by
//!   `drawing_differential.rs`.
//!
//! Criterion 4 is checked on the L-bracket XZ section at `y = 4`: one outer
//! 60 x 40 loop, one inner diameter-10 loop centred at `(0, 24)`, and a net
//! area of `2400 - 25*pi` measured on the exact faces at relative 1e-9.

mod common;

use std::f64::consts::PI;

use kala_cad::{
    Bounds2D, CompilationResult, CompileOptions, DEFAULT_CHORD_TOLERANCE_MM,
    DEFAULT_SECTION_CHORD_TOLERANCE_MM, Drawing, DrawingErrorKind, MAX_POINTS_PER_VIEW, Point2,
    Polyline, Section, SectionLoop, SectionPlane, Shape, ViewDrawing, ViewName, compile_json_str,
    default_section_offset, hlr_views, hlr_views_with_options, section, section_with_options,
};
use serde_json::json;

const BRACKET_WIDTH_MM: f64 = 60.0;
const BRACKET_DEPTH_MM: f64 = 45.0;
const BRACKET_HEIGHT_MM: f64 = 40.0;
const BORE_DIAMETER_MM: f64 = 10.0;
const BORE_CENTRE_Z_MM: f64 = 24.0;
const SECTION_OFFSET_MM: f64 = 4.0;
const BOUNDS_TOLERANCE_MM: f64 = 0.05;

fn l_bracket() -> CompilationResult {
    compile_json_str(&common::l_bracket_script(), &CompileOptions::default())
        .expect("the L-bracket script compiles")
}

fn box_with_hole() -> CompilationResult {
    let script = common::script(
        json!([{
            "name": "block",
            "steps": [
                {"op": "sketch", "profile": "rectangle", "plane": "XZ",
                 "width": 40.0, "height": 20.0, "x": -20.0, "y": 0.0},
                {"op": "extrude", "length": 30.0},
                {"op": "sketch", "profile": "circle", "plane": "XZ",
                 "diameter": 10.0, "x": 0.0, "y": 10.0},
                {"op": "pocket"}
            ]
        }]),
        json!([{"op": "place", "component": "block", "y": 30.0}]),
    );
    compile_json_str(&script, &CompileOptions::default()).expect("the box script compiles")
}

fn box_with_island() -> CompilationResult {
    let script = common::script(
        json!([
            {
                "name": "block",
                "steps": [
                    {"op": "sketch", "profile": "rectangle", "plane": "XZ",
                     "width": 40.0, "height": 24.0, "x": -20.0, "y": 0.0},
                    {"op": "extrude", "length": 30.0},
                    {"op": "sketch", "profile": "circle", "plane": "XZ",
                     "diameter": 20.0, "x": 0.0, "y": 12.0},
                    {"op": "pocket"}
                ]
            },
            {
                "name": "island",
                "steps": [
                    {"op": "sketch", "profile": "circle", "plane": "XZ",
                     "diameter": 8.0, "x": 0.0, "y": 12.0},
                    {"op": "extrude", "length": 10.0, "reversed": true}
                ]
            }
        ]),
        json!([
            {"op": "place", "component": "block", "y": 30.0},
            {"op": "place", "component": "island", "y": 10.0}
        ]),
    );
    compile_json_str(&script, &CompileOptions::default()).expect("the island script compiles")
}

fn assert_close(actual: f64, expected: f64, absolute: f64, label: &str) {
    assert!(
        (actual - expected).abs() <= absolute,
        "{label}: {actual} differs from {expected} by more than {absolute}"
    );
}

fn assert_relative(actual: f64, expected: f64, relative: f64, label: &str) {
    let difference = (actual - expected).abs();
    assert!(
        difference <= relative * expected.abs(),
        "{label}: {actual} differs from {expected} by {difference} (relative tolerance {relative})"
    );
}

fn assert_2d_bounds(
    bounds: Bounds2D,
    minimum: (f64, f64),
    maximum: (f64, f64),
    tolerance: f64,
    label: &str,
) {
    assert_close(
        bounds.min_u,
        minimum.0,
        tolerance,
        &format!("{label} min_u"),
    );
    assert_close(
        bounds.min_v,
        minimum.1,
        tolerance,
        &format!("{label} min_v"),
    );
    assert_close(
        bounds.max_u,
        maximum.0,
        tolerance,
        &format!("{label} max_u"),
    );
    assert_close(
        bounds.max_v,
        maximum.1,
        tolerance,
        &format!("{label} max_v"),
    );
}

fn polyline_bounds_near<'a>(
    polylines: impl Iterator<Item = &'a Polyline>,
    u: f64,
    v: f64,
) -> &'a Polyline {
    polylines
        .min_by(|left, right| {
            let left_bounds = left.bounds().expect("a polyline has points");
            let right_bounds = right.bounds().expect("a polyline has points");
            let left_distance = (left_bounds.center_u() - u).hypot(left_bounds.center_v() - v);
            let right_distance = (right_bounds.center_u() - u).hypot(right_bounds.center_v() - v);
            left_distance
                .partial_cmp(&right_distance)
                .expect("finite distances")
        })
        .expect("at least one polyline")
}

fn section_at(result: &CompilationResult, plane: &str, offset: Option<f64>) -> Section {
    section_with_options(
        result.shape(),
        plane,
        offset,
        DEFAULT_SECTION_CHORD_TOLERANCE_MM,
    )
    .expect("the section is cut")
}

// --- ADR-0009 criterion 4: the section of the L-bracket ---

#[test]
fn section_has_exactly_one_outer_and_one_inner_loop() {
    let bracket = l_bracket();
    let cut = section_at(&bracket, "XZ", Some(SECTION_OFFSET_MM));
    assert_eq!(cut.outer_loop_count(), 1);
    assert_eq!(cut.inner_loop_count(), 1);
    assert_eq!(cut.loops.len(), 2);
}

#[test]
fn outer_loop_is_the_60_by_40_rectangle() {
    let bracket = l_bracket();
    let cut = section_at(&bracket, "XZ", Some(SECTION_OFFSET_MM));
    let outer = cut.outer_loops().next().expect("one outer loop");
    let bounds = outer.bounds().expect("bounded loop");
    assert_close(bounds.width(), 60.0, BOUNDS_TOLERANCE_MM, "outer width");
    assert_close(bounds.height(), 40.0, BOUNDS_TOLERANCE_MM, "outer height");
    assert_2d_bounds(
        bounds,
        (-30.0, 0.0),
        (30.0, 40.0),
        BOUNDS_TOLERANCE_MM,
        "outer bounds",
    );
    assert_relative(outer.area_mm2, 2400.0, 1e-9, "outer area");
    assert!(outer.area_exact, "the outer area is a kernel measurement");
}

#[test]
fn inner_loop_is_the_diameter_10_bore() {
    let bracket = l_bracket();
    let cut = section_at(&bracket, "XZ", Some(SECTION_OFFSET_MM));
    let inner = cut.inner_loops().next().expect("one inner loop");
    let bounds = inner.bounds().expect("bounded loop");
    assert_close(
        bounds.width(),
        BORE_DIAMETER_MM,
        BOUNDS_TOLERANCE_MM,
        "bore width",
    );
    assert_close(
        bounds.height(),
        BORE_DIAMETER_MM,
        BOUNDS_TOLERANCE_MM,
        "bore height",
    );
    assert_close(bounds.center_u(), 0.0, BOUNDS_TOLERANCE_MM, "bore centre u");
    assert_close(
        bounds.center_v(),
        BORE_CENTRE_Z_MM,
        BOUNDS_TOLERANCE_MM,
        "bore centre v",
    );
    let centroid = inner.centroid().expect("bounded loop");
    assert_close(centroid.u, 0.0, BOUNDS_TOLERANCE_MM, "bore centroid u");
    assert_close(
        centroid.v,
        BORE_CENTRE_Z_MM,
        BOUNDS_TOLERANCE_MM,
        "bore centroid v",
    );
    assert_relative(inner.area_mm2, 25.0 * PI, 1e-9, "bore area");
    assert!(inner.area_exact, "the bore area is a kernel measurement");
}

#[test]
fn enclosed_area_matches_the_adr_value() {
    let bracket = l_bracket();
    let cut = section_at(&bracket, "XZ", Some(SECTION_OFFSET_MM));
    assert_relative(cut.area_mm2, 2400.0 - 25.0 * PI, 1e-9, "net area");
    assert!(cut.area_exact, "the net area is a kernel measurement");
}

#[test]
fn the_area_of_a_round_loop_is_the_area_of_the_circle() {
    let bracket = l_bracket();
    let cut = section_at(&bracket, "XZ", Some(SECTION_OFFSET_MM));
    let inner = cut.inner_loops().next().expect("one inner loop");
    // The drawn polygon is the sampled circle, so its area misses the exact
    // circle by far more than the 1e-3 relative band the Python test checks.
    let signed = inner
        .points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let following = inner.points[(index + 1) % inner.points.len()];
            point.u * following.v - following.u * point.v
        })
        .sum::<f64>()
        / 2.0;
    let drawn_area = signed.abs();
    assert!(
        (drawn_area - inner.area_mm2).abs() > inner.area_mm2 * 1e-3,
        "the drawn polygon must not be the measured circle area"
    );
    assert_relative(inner.area_mm2, 25.0 * PI, 1e-9, "bore area");
}

#[test]
fn section_misses_the_base_holes() {
    let bracket = l_bracket();
    let cut = section_at(&bracket, "XZ", Some(SECTION_OFFSET_MM));
    for inner in cut.inner_loops() {
        let bounds = inner.bounds().expect("bounded loop");
        assert_close(bounds.center_u(), 0.0, BOUNDS_TOLERANCE_MM, "hole u");
        assert_close(
            bounds.center_v(),
            BORE_CENTRE_Z_MM,
            BOUNDS_TOLERANCE_MM,
            "hole v",
        );
    }
}

#[test]
fn section_loops_are_closed() {
    let bracket = l_bracket();
    let cut = section_at(&bracket, "XZ", Some(SECTION_OFFSET_MM));
    for loop_ in &cut.loops {
        assert!(loop_.points.len() >= 3);
        assert_ne!(loop_.points[0], *loop_.points.last().expect("points"));
        let first = loop_.points[0];
        let last = *loop_.points.last().expect("points");
        assert!(
            first.distance(last) > 1e-6,
            "a closed loop is not drawn back onto its own first point"
        );
    }
}

#[test]
fn section_reports_its_plane_and_offset() {
    let bracket = l_bracket();
    let cut = section_at(&bracket, "XZ", Some(SECTION_OFFSET_MM));
    assert_eq!(cut.plane, SectionPlane::Xz);
    assert_eq!(cut.offset_mm, SECTION_OFFSET_MM);
    assert_eq!(cut.chord_tolerance_mm, DEFAULT_SECTION_CHORD_TOLERANCE_MM);
    assert!(!cut.truncated);
}

// --- ADR-0009 criterion 4 companion: the hidden lines of the bore ---

#[test]
fn front_view_draws_the_bore() {
    let bracket = l_bracket();
    let drawing = hlr_views(bracket.shape()).expect("the bracket is drawn");
    let front = drawing.view("front").expect("the front view");
    let closed = front.visible.iter().filter(|line| line.closed);
    let count = closed.clone().count();
    assert!(
        count > 0,
        "the bore opening must be visible in the front view"
    );
    let bore = polyline_bounds_near(closed, 0.0, BORE_CENTRE_Z_MM)
        .bounds()
        .unwrap();
    assert_close(bore.width(), BORE_DIAMETER_MM, 0.1, "visible bore width");
    assert_close(bore.height(), BORE_DIAMETER_MM, 0.1, "visible bore height");
    assert_close(
        bore.center_v(),
        BORE_CENTRE_Z_MM,
        0.1,
        "visible bore centre v",
    );
}

#[test]
fn front_view_has_hidden_lines_for_the_bore() {
    let bracket = l_bracket();
    let drawing = hlr_views(bracket.shape()).expect("the bracket is drawn");
    let front = drawing.view("front").expect("the front view");
    assert!(
        !front.hidden.is_empty(),
        "the front view must carry hidden lines"
    );
    let hidden_closed = front.hidden.iter().filter(|line| line.closed);
    assert!(
        hidden_closed.clone().count() > 0,
        "the bore must be drawn as a hidden loop in the front view"
    );
    let bore = polyline_bounds_near(hidden_closed, 0.0, BORE_CENTRE_Z_MM)
        .bounds()
        .unwrap();
    assert_close(bore.width(), BORE_DIAMETER_MM, 0.1, "hidden bore width");
    assert_close(bore.height(), BORE_DIAMETER_MM, 0.1, "hidden bore height");
    assert_close(bore.center_u(), 0.0, 0.1, "hidden bore centre u");
    assert_close(
        bore.center_v(),
        BORE_CENTRE_Z_MM,
        0.1,
        "hidden bore centre v",
    );
}

#[test]
fn front_view_outline_is_the_part_envelope() {
    let bracket = l_bracket();
    let drawing = hlr_views(bracket.shape()).expect("the bracket is drawn");
    let front = drawing.view("front").expect("the front view");
    let (min_u, max_u, min_v, max_v) = visible_extents(front);
    assert_close(
        min_u,
        -BRACKET_WIDTH_MM / 2.0,
        BOUNDS_TOLERANCE_MM,
        "front u min",
    );
    assert_close(
        max_u,
        BRACKET_WIDTH_MM / 2.0,
        BOUNDS_TOLERANCE_MM,
        "front u max",
    );
    assert_close(min_v, 0.0, BOUNDS_TOLERANCE_MM, "front v min");
    assert_close(max_v, BRACKET_HEIGHT_MM, BOUNDS_TOLERANCE_MM, "front v max");
}

fn visible_extents(view: &ViewDrawing) -> (f64, f64, f64, f64) {
    let mut min_u = f64::INFINITY;
    let mut max_u = f64::NEG_INFINITY;
    let mut min_v = f64::INFINITY;
    let mut max_v = f64::NEG_INFINITY;
    for point in view.visible.iter().flat_map(|line| &line.points) {
        min_u = min_u.min(point.u);
        max_u = max_u.max(point.u);
        min_v = min_v.min(point.v);
        max_v = max_v.max(point.v);
    }
    (min_u, max_u, min_v, max_v)
}

// --- the four views ---

#[test]
fn drawing_has_the_four_views() {
    let bracket = l_bracket();
    let drawing = hlr_views(bracket.shape()).expect("the bracket is drawn");
    assert_eq!(
        drawing.view_names(),
        vec![
            ViewName::Front,
            ViewName::Top,
            ViewName::Right,
            ViewName::Iso
        ]
    );
    for view in &drawing.views {
        assert!(!view.visible.is_empty() || !view.hidden.is_empty());
        assert!(!view.truncated);
    }
}

#[test]
fn top_view_is_measured_in_x_and_y() {
    let bracket = l_bracket();
    let drawing = hlr_views(bracket.shape()).expect("the bracket is drawn");
    let (min_u, max_u, min_v, max_v) = visible_extents(drawing.view("top").unwrap());
    assert_close(
        min_u,
        -BRACKET_WIDTH_MM / 2.0,
        BOUNDS_TOLERANCE_MM,
        "top u min",
    );
    assert_close(
        max_u,
        BRACKET_WIDTH_MM / 2.0,
        BOUNDS_TOLERANCE_MM,
        "top u max",
    );
    assert_close(min_v, 0.0, BOUNDS_TOLERANCE_MM, "top v min");
    assert_close(max_v, BRACKET_DEPTH_MM, BOUNDS_TOLERANCE_MM, "top v max");
}

#[test]
fn right_view_is_measured_in_y_and_z() {
    let bracket = l_bracket();
    let drawing = hlr_views(bracket.shape()).expect("the bracket is drawn");
    let (min_u, max_u, min_v, max_v) = visible_extents(drawing.view("right").unwrap());
    assert_close(min_u, 0.0, BOUNDS_TOLERANCE_MM, "right u min");
    assert_close(max_u, BRACKET_DEPTH_MM, BOUNDS_TOLERANCE_MM, "right u max");
    assert_close(min_v, 0.0, BOUNDS_TOLERANCE_MM, "right v min");
    assert_close(max_v, BRACKET_HEIGHT_MM, BOUNDS_TOLERANCE_MM, "right v max");
}

#[test]
fn iso_view_is_drawn() {
    let bracket = l_bracket();
    let drawing = hlr_views(bracket.shape()).expect("the bracket is drawn");
    let iso = drawing.view("iso").expect("the iso view");
    assert!(!iso.visible.is_empty());
    let (min_u, max_u, min_v, max_v) = visible_extents(iso);
    assert!(max_u - min_u > 40.0);
    assert!(max_v - min_v > 20.0);
}

#[test]
fn only_the_requested_views_are_drawn() {
    let bracket = l_bracket();
    let drawing = hlr_views_with_options(
        bracket.shape(),
        &["front"],
        DEFAULT_CHORD_TOLERANCE_MM,
        MAX_POINTS_PER_VIEW,
    )
    .expect("the front view is drawn");
    assert_eq!(drawing.view_names(), vec![ViewName::Front]);
    assert!(drawing.total_point_count() > 0);
}

#[test]
fn unknown_view_is_refused() {
    let bracket = l_bracket();
    let error = hlr_views_with_options(
        bracket.shape(),
        &["side"],
        DEFAULT_CHORD_TOLERANCE_MM,
        MAX_POINTS_PER_VIEW,
    )
    .expect_err("an unknown view is refused");
    assert_eq!(error.kind(), DrawingErrorKind::Argument);
    assert_eq!(
        error.message(),
        "unknown view 'side'; expected one of ['front', 'iso', 'right', 'top']"
    );
}

// --- the section of a simpler part ---

#[test]
fn section_of_a_box_with_a_through_hole() {
    let box_ = box_with_hole();
    let cut = section_at(&box_, "XZ", Some(15.0));
    assert_eq!(cut.outer_loop_count(), 1);
    assert_eq!(cut.inner_loop_count(), 1);
    let outer = cut.outer_loops().next().unwrap();
    let inner = cut.inner_loops().next().unwrap();
    let outer_bounds = outer.bounds().unwrap();
    assert_close(
        outer_bounds.width(),
        40.0,
        BOUNDS_TOLERANCE_MM,
        "outer width",
    );
    assert_close(
        outer_bounds.height(),
        20.0,
        BOUNDS_TOLERANCE_MM,
        "outer height",
    );
    assert_relative(outer.area_mm2, 800.0, 1e-9, "outer area");
    assert!(outer.area_exact);
    let inner_bounds = inner.bounds().unwrap();
    assert_close(
        inner_bounds.width(),
        10.0,
        BOUNDS_TOLERANCE_MM,
        "hole width",
    );
    assert_close(inner_bounds.center_u(), 0.0, BOUNDS_TOLERANCE_MM, "hole u");
    assert_close(inner_bounds.center_v(), 10.0, BOUNDS_TOLERANCE_MM, "hole v");
    assert_relative(inner.area_mm2, 25.0 * PI, 1e-9, "hole area");
    assert!(inner.area_exact);
    assert_relative(cut.area_mm2, 800.0 - 25.0 * PI, 1e-9, "net area");
    assert!(cut.area_exact);
}

#[test]
fn a_tangent_cut_reports_an_approximate_area() {
    let box_ = box_with_hole();
    // The plane z = 5 touches the bore of the box: the cut wire touches itself
    // and the module reports the drawn polygon area as an approximation.
    let cut = section_at(&box_, "XY", Some(5.0));
    assert_eq!(cut.loops.len(), 1);
    assert_relative(cut.loops[0].area_mm2, 1200.0, 1e-9, "tangent loop area");
    assert!(!cut.loops[0].area_exact);
    assert_relative(cut.area_mm2, 1200.0, 1e-9, "tangent net area");
    assert!(!cut.area_exact);
}

#[test]
fn an_island_inside_a_hole_is_material_again() {
    let box_ = box_with_island();
    let cut = section_at(&box_, "XZ", Some(15.0));
    assert_eq!(cut.outer_loop_count(), 2);
    assert_eq!(cut.inner_loop_count(), 1);
    assert_relative(
        cut.area_mm2,
        960.0 - 100.0 * PI + 16.0 * PI,
        1e-9,
        "nested net area",
    );
    assert!(cut.area_exact);
    for loop_ in &cut.loops {
        assert!(loop_.area_exact);
    }
}

#[test]
fn section_of_a_plane_that_misses_the_part_is_empty() {
    let box_ = box_with_hole();
    let cut = section_at(&box_, "XZ", Some(100.0));
    assert!(cut.loops.is_empty());
    assert_eq!(cut.area_mm2, 0.0);
    assert!(cut.area_exact);
}

#[test]
fn section_on_the_other_planes() {
    let box_ = box_with_hole();
    let xy_cut = section_at(&box_, "XY", Some(5.0));
    let yz_cut = section_at(&box_, "YZ", Some(10.0));
    let xy_outer = xy_cut.outer_loops().next().unwrap();
    let xy_bounds = xy_outer.bounds().unwrap();
    assert_close(xy_bounds.width(), 40.0, BOUNDS_TOLERANCE_MM, "XY width");
    assert_close(xy_bounds.height(), 30.0, BOUNDS_TOLERANCE_MM, "XY height");
    let yz_outer = yz_cut.outer_loops().next().unwrap();
    let yz_bounds = yz_outer.bounds().unwrap();
    assert_close(yz_bounds.width(), 30.0, BOUNDS_TOLERANCE_MM, "YZ width");
    assert_close(yz_bounds.height(), 20.0, BOUNDS_TOLERANCE_MM, "YZ height");
}

#[test]
fn a_section_through_the_hole_splits_the_part() {
    let box_ = box_with_hole();
    let cut = section_at(&box_, "YZ", Some(0.0));
    assert_eq!(cut.outer_loop_count(), 2);
    assert_eq!(cut.inner_loop_count(), 0);
    assert_relative(cut.area_mm2, 2.0 * 30.0 * 5.0, 1e-9, "split net area");
    assert!(cut.area_exact);
    for loop_ in cut.outer_loops() {
        assert_relative(loop_.area_mm2, 150.0, 1e-9, "split loop area");
        assert!(loop_.area_exact);
    }
}

#[test]
fn unknown_section_plane_is_refused() {
    let bracket = l_bracket();
    let error = section_with_options(
        bracket.shape(),
        "AB",
        Some(0.0),
        DEFAULT_SECTION_CHORD_TOLERANCE_MM,
    )
    .expect_err("an unknown plane is refused");
    assert_eq!(error.kind(), DrawingErrorKind::Argument);
    assert_eq!(
        error.message(),
        "unknown section plane 'AB'; expected one of ['XY', 'XZ', 'YZ']"
    );
}

#[test]
fn lowercase_section_plane_is_accepted() {
    let bracket = l_bracket();
    let cut = section_at(&bracket, "xz", Some(SECTION_OFFSET_MM));
    assert_eq!(cut.plane, SectionPlane::Xz);
}

#[test]
fn default_section_offset_is_the_mid_plane() {
    let bracket = l_bracket();
    let offset = default_section_offset(bracket.shape()).expect("the bracket has bounds");
    assert_close(offset, 22.5, 1e-6, "default offset");
}

#[test]
fn a_section_without_an_offset_uses_the_mid_plane() {
    let bracket = l_bracket();
    let cut = section(bracket.shape()).expect("the default section is cut");
    assert_close(cut.offset_mm, 22.5, 1e-6, "default offset");
    assert_eq!(cut.plane, SectionPlane::Xz);
}

// --- the public contract of the module ---

#[test]
fn results_are_plain_native_data() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Drawing>();
    assert_send_sync::<ViewDrawing>();
    assert_send_sync::<Polyline>();
    assert_send_sync::<Section>();
    assert_send_sync::<SectionLoop>();
    assert_send_sync::<Bounds2D>();
    assert_send_sync::<Point2>();

    let bracket = l_bracket();
    let drawing: Drawing = hlr_views(bracket.shape()).unwrap();
    let cut: Section = section_at(&bracket, "XZ", Some(SECTION_OFFSET_MM));
    assert!(drawing.chord_tolerance_mm > 0.0);
    assert!(cut.area_mm2.is_finite());
    assert!(
        drawing.views[0].visible[0]
            .points
            .iter()
            .all(|point| { point.u.is_finite() && point.v.is_finite() })
    );
}

#[test]
fn a_null_shape_is_refused() {
    let null = Shape::null().expect("a null kernel handle");
    let error = hlr_views(&null).expect_err("a null shape cannot be drawn");
    assert_eq!(error.kind(), DrawingErrorKind::Shape);
    assert_eq!(error.message(), "the drawing needs a non-null shape");
    let error =
        section_with_options(&null, "XZ", Some(0.0), 0.01).expect_err("a null shape cannot be cut");
    assert_eq!(error.kind(), DrawingErrorKind::Shape);
    assert_eq!(error.message(), "the drawing needs a non-null shape");
}

#[test]
fn the_point_cap_is_the_exact_end_of_the_curve() {
    let box_ = box_with_hole();
    let drawing = hlr_views_with_options(
        box_.shape(),
        &["front"],
        DEFAULT_CHORD_TOLERANCE_MM,
        MAX_POINTS_PER_VIEW,
    )
    .unwrap();
    let view = drawing.view("front").unwrap();
    for line in &view.visible {
        if line.closed {
            assert!(line.points[0].distance(*line.points.last().unwrap()) <= 1e-6);
            continue;
        }
        assert!(line.points.len() >= 2);
        assert_ne!(line.points[0], *line.points.last().unwrap());
    }
}

#[test]
fn a_tight_chord_tolerance_is_closer_to_the_curve() {
    let bracket = l_bracket();
    let coarse =
        hlr_views_with_options(bracket.shape(), &["front"], 0.5, MAX_POINTS_PER_VIEW).unwrap();
    let fine =
        hlr_views_with_options(bracket.shape(), &["front"], 0.001, MAX_POINTS_PER_VIEW).unwrap();
    assert!(fine.total_point_count() > coarse.total_point_count());
}

#[test]
fn an_invalid_chord_tolerance_is_refused() {
    let bracket = l_bracket();
    let error = hlr_views_with_options(bracket.shape(), &["front"], 0.0, MAX_POINTS_PER_VIEW)
        .expect_err("zero tolerance is refused");
    assert_eq!(error.kind(), DrawingErrorKind::Argument);
    assert_eq!(
        error.message(),
        "the chord tolerance must be a positive finite number: 0.0"
    );
    let error = section_with_options(bracket.shape(), "XZ", Some(0.0), -1.0)
        .expect_err("a negative tolerance is refused");
    assert_eq!(error.kind(), DrawingErrorKind::Argument);
    assert_eq!(
        error.message(),
        "the chord tolerance must be a positive finite number: -1.0"
    );
    let error = hlr_views_with_options(bracket.shape(), &["front"], f64::NAN, 10)
        .expect_err("a non-finite tolerance is refused");
    assert_eq!(
        error.message(),
        "the chord tolerance must be a positive finite number: nan"
    );
}

#[test]
fn a_non_finite_section_offset_is_refused() {
    let bracket = l_bracket();
    for (offset, expected) in [
        (f64::NAN, "nan"),
        (f64::INFINITY, "inf"),
        (f64::NEG_INFINITY, "-inf"),
    ] {
        let error = section_with_options(bracket.shape(), "XZ", Some(offset), 0.01)
            .expect_err("a non-finite offset is refused");
        assert_eq!(error.kind(), DrawingErrorKind::Argument);
        assert_eq!(
            error.message(),
            format!("the section offset must be a finite number: {expected}")
        );
    }
}

#[test]
fn the_point_limit_is_refused_at_zero() {
    let bracket = l_bracket();
    let error = hlr_views_with_options(bracket.shape(), &["front"], 0.05, 0)
        .expect_err("a zero point limit is refused");
    assert_eq!(error.kind(), DrawingErrorKind::Argument);
    assert_eq!(error.message(), "the point limit must be positive: 0");
}

#[test]
fn the_point_limit_truncates_the_view() {
    let bracket = l_bracket();
    let drawing = hlr_views_with_options(bracket.shape(), &["front"], 0.05, 5).unwrap();
    let view = drawing.view("front").unwrap();
    assert!(view.truncated);
    assert!(view.point_count() <= 5);
}

#[test]
fn the_default_chord_tolerances_are_the_adr_values() {
    assert_eq!(DEFAULT_CHORD_TOLERANCE_MM, 0.05);
    assert_eq!(DEFAULT_SECTION_CHORD_TOLERANCE_MM, 0.01);
    assert_eq!(MAX_POINTS_PER_VIEW, 20_000);
}

#[test]
fn the_drawing_is_deterministic() {
    let bracket = l_bracket();
    let first = hlr_views_with_options(
        bracket.shape(),
        &["front", "top"],
        DEFAULT_CHORD_TOLERANCE_MM,
        MAX_POINTS_PER_VIEW,
    )
    .unwrap();
    let second = hlr_views_with_options(
        bracket.shape(),
        &["front", "top"],
        DEFAULT_CHORD_TOLERANCE_MM,
        MAX_POINTS_PER_VIEW,
    )
    .unwrap();
    assert_eq!(first.views[0].visible, second.views[0].visible);
    assert_eq!(first.views[1].hidden, second.views[1].hidden);
}

#[test]
fn repeated_drawing_and_section_handles_are_released() {
    let bracket = l_bracket();
    let first = hlr_views_with_options(
        bracket.shape(),
        &["front"],
        DEFAULT_CHORD_TOLERANCE_MM,
        MAX_POINTS_PER_VIEW,
    )
    .expect("the first drawing");
    for _ in 0..50 {
        let drawing = hlr_views_with_options(
            bracket.shape(),
            &["front"],
            DEFAULT_CHORD_TOLERANCE_MM,
            MAX_POINTS_PER_VIEW,
        )
        .expect("a repeated drawing");
        assert_eq!(drawing.views[0].visible, first.views[0].visible);
        let cut = section_at(&bracket, "XZ", Some(SECTION_OFFSET_MM));
        assert_eq!(cut.loops.len(), 2);
    }
    // A refused draw must not poison the handles or the kernel state.
    for _ in 0..50 {
        hlr_views_with_options(
            bracket.shape(),
            &["side"],
            DEFAULT_CHORD_TOLERANCE_MM,
            MAX_POINTS_PER_VIEW,
        )
        .expect_err("the view stays unknown");
    }
    let again = hlr_views_with_options(
        bracket.shape(),
        &["front"],
        DEFAULT_CHORD_TOLERANCE_MM,
        MAX_POINTS_PER_VIEW,
    )
    .expect("the drawing still works");
    assert_eq!(again, first);
}
