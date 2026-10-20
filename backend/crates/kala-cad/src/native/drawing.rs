//! Native hidden-line drawing and section geometry.
//!
//! The kernel work is deliberately narrow: the C++ adapter runs
//! `HLRBRep_Algo`/`HLRBRep_HLRToShape` for one view, samples edges with
//! `GCPnts_QuasiUniformDeflection`, runs `BRepAlgoAPI_Section` for one plane,
//! connects the cut edges with `ShapeAnalysis_FreeBounds`, builds the planar
//! faces and measures their areas with `BRepGProp`. Everything else — the view
//! bases, the point caps, duplicate removal, loop nesting, the outer/hole
//! classification and the measurement-consistency policy — lives in Rust.
//!
//! The result is plain data; no kernel handle escapes this module.
//!
//! # Parity
//!
//! The module is a port of `src/kala_agent/studio/design/drawing.py`:
//!
//! * the same four view bases and three section bases;
//! * the same shared edge sampling (exact point caps, closed-curve repetition,
//!   single-point collapse);
//! * the same deduplication and point-cap order in a view;
//! * the same loop sorting, nesting classification and net-area fallbacks;
//! * the exact measured area is the kernel's `BRepGProp` area of the cut face,
//!   not the drawn polygon, and every area carries `area_exact`.

#![allow(unsafe_code)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;

use crate::drawing::{
    AREA_CONSISTENCY_RATIO, Bounds2D, DEFAULT_CHORD_TOLERANCE_MM,
    DEFAULT_SECTION_CHORD_TOLERANCE_MM, DEFAULT_VIEW_NAMES, Drawing, MAX_POINTS_PER_VIEW,
    MAX_SECTION_POINTS, Point2, Polyline, Section, SectionLoop, SectionPlane, ViewDrawing,
    ViewName, bounds_of, centroid_of, parse_section_plane, parse_view_name, point_in_polygon,
    python_repr_f64, section_basis, signed_area, validate_chord_tolerance, view_basis,
};
use crate::error::DrawingError;
use crate::native::ffi;
use crate::native::ffi::{FfiError, FfiSectionLoopInfo};
use crate::native::shape::Shape;

/// The four views of a shape with the ADR defaults.
///
/// This is the direct equivalent of Python's `hlr_views(shape)`.
pub fn hlr_views(shape: &Shape) -> Result<Drawing, DrawingError> {
    hlr_views_with_options(
        shape,
        &DEFAULT_VIEW_NAMES,
        DEFAULT_CHORD_TOLERANCE_MM,
        MAX_POINTS_PER_VIEW,
    )
}

/// The requested views of a shape.
///
/// * `views` are the lower-case Python names (`front`, `top`, `right`,
///   `iso`). An unknown name is refused with Python's exact message and
///   argument kind.
/// * `chord_tolerance_mm` bounds the distance between a polyline and the curve
///   it approximates; it must be positive and finite.
/// * `max_points` is the per-view point cap. A view whose next polyline would
///   exceed it stops there and reports `truncated = true`.
///
/// Views are drawn in the requested order. The Python module validates a view
/// name only when it reaches that view; this function validates every name up
/// front, so an unknown name is reported before any kernel work. The message
/// and refusal class are identical.
pub fn hlr_views_with_options(
    shape: &Shape,
    views: &[&str],
    chord_tolerance_mm: f64,
    max_points: usize,
) -> Result<Drawing, DrawingError> {
    require_shape(shape)?;
    let tolerance = validate_chord_tolerance(chord_tolerance_mm)?;
    if max_points < 1 {
        return Err(DrawingError::argument(format!(
            "the point limit must be positive: {max_points}"
        )));
    }
    let names = views
        .iter()
        .map(|name| parse_view_name(name))
        .collect::<Result<Vec<_>, _>>()?;
    let mut drawn = Vec::with_capacity(names.len());
    for name in names {
        drawn.push(draw_view(shape, name, tolerance, max_points)?);
    }
    Ok(Drawing {
        views: drawn,
        chord_tolerance_mm: tolerance,
        max_points,
    })
}

/// The default section: the `XZ` plane at the mid-depth of the shape.
///
/// This is the direct equivalent of Python's `section(shape)`.
pub fn section(shape: &Shape) -> Result<Section, DrawingError> {
    section_with_options(
        shape,
        SectionPlane::Xz.as_str(),
        None,
        DEFAULT_SECTION_CHORD_TOLERANCE_MM,
    )
}

/// A section cut through a shape.
///
/// * `plane` is `XY`, `XZ` or `YZ`, case insensitive like Python's
///   `str(plane).upper()`. The plane is normal to the third axis of the name,
///   so `XZ` with an offset of 4 is the plane `y = 4`.
/// * `offset_mm` is measured from the global origin along the plane normal.
///   `None` uses the mid-plane of the shape's bounds normal to `Y`, which is
///   ADR-0009's default position (the same offset regardless of the plane
///   name, exactly like the Python module).
/// * `chord_tolerance_mm` bounds the drawn loops only. The areas are measured
///   on the exact faces of the cut.
///
/// The point cap is the module constant [`MAX_SECTION_POINTS`], matching
/// Python; a section that exceeds it reports `truncated = true` and an
/// inexact net area.
pub fn section_with_options(
    shape: &Shape,
    plane: &str,
    offset_mm: Option<f64>,
    chord_tolerance_mm: f64,
) -> Result<Section, DrawingError> {
    require_shape(shape)?;
    let plane_name = parse_section_plane(plane)?;
    let offset = match offset_mm {
        Some(value) => {
            if !value.is_finite() {
                return Err(DrawingError::argument(format!(
                    "the section offset must be a finite number: {}",
                    python_repr_f64(value)
                )));
            }
            value
        }
        None => default_section_offset(shape)?,
    };
    let tolerance = validate_chord_tolerance(chord_tolerance_mm)?;

    let (normal, u_axis, v_axis) = section_basis(plane_name);
    let handle = match SectionHandle::create(shape, &normal, offset, &u_axis, &v_axis, tolerance)? {
        SectionCreate::Created(handle) => handle,
        SectionCreate::NotDone => {
            return Err(DrawingError::shape(format!(
                "the section at {} offset {} mm did not complete",
                plane_name.as_str(),
                python_repr_f64(offset)
            )));
        }
    };

    let loop_count = handle.loop_count()?;
    let mut records = Vec::with_capacity(loop_count);
    for index in 0..loop_count {
        let info = handle.loop_info(index)?;
        let points = handle.loop_points(index, info.point_count)?;
        if points.len() < 3 {
            continue;
        }
        let Some(bounds) = bounds_of(&points) else {
            continue;
        };
        let polygon_area = signed_area(&points).abs();
        let (area_mm2, area_exact) = if info.simple && info.area_measured {
            let slack = AREA_CONSISTENCY_RATIO * info.area.abs().max(1.0);
            let allowance = info.length * tolerance + slack;
            if (info.area - polygon_area).abs() <= allowance {
                (info.area, true)
            } else {
                // The kernel built a face that does not describe the loop; an
                // approximation is never reported as a measurement.
                (polygon_area, false)
            }
        } else {
            (polygon_area, false)
        };
        records.push(LoopRecord {
            wire_index: index,
            points,
            bounds,
            area_mm2,
            area_exact,
            simple: info.simple,
            depth: 0,
            parent: None,
        });
    }

    // Python sorts by (-area, min_u, min_v) with a stable sort.
    records.sort_by(|left, right| {
        right
            .area_mm2
            .partial_cmp(&left.area_mm2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                left.bounds
                    .min_u
                    .partial_cmp(&right.bounds.min_u)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                left.bounds
                    .min_v
                    .partial_cmp(&right.bounds.min_v)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let mut kept = Vec::with_capacity(records.len());
    let mut used = 0usize;
    let mut truncated = false;
    for record in records {
        if used + record.points.len() > MAX_SECTION_POINTS {
            truncated = true;
            continue;
        }
        used += record.points.len();
        kept.push(record);
    }

    classify_loops(&mut kept);
    let (area_mm2, mut area_exact) = net_area(&handle, &kept)?;
    if truncated {
        // A dropped loop leaves a hole in the cut, so the area no longer
        // covers the whole section and cannot be called exact.
        area_exact = false;
    }
    let loops = kept
        .into_iter()
        .map(|record| SectionLoop {
            points: record.points,
            area_mm2: record.area_mm2,
            outer: record.depth % 2 == 0,
            area_exact: record.area_exact,
        })
        .collect();
    Ok(Section {
        plane: plane_name,
        offset_mm: offset,
        loops,
        area_mm2,
        chord_tolerance_mm: tolerance,
        truncated,
        area_exact,
    })
}

/// The mid-plane offset of the shape's bounds normal to `Y`.
///
/// This is the default section position of ADR-0009: the plane that cuts the
/// part through the middle of its depth.
pub fn default_section_offset(shape: &Shape) -> Result<f64, DrawingError> {
    require_shape(shape)?;
    let mut out = 0.0;
    let mut error = FfiError::default();
    // SAFETY: shape is a live handle and the out-pointer is valid.
    let status = unsafe { ffi::kala_shape_y_midpoint(shape.as_ptr(), &mut out, &mut error) };
    match status {
        ffi::OK => Ok(out),
        ffi::VOID_BOX => Err(DrawingError::shape("the shape has no finite bounds")),
        status => Err(cut_error(status, &error)),
    }
}

fn require_shape(shape: &Shape) -> Result<(), DrawingError> {
    if shape.is_null() {
        return Err(DrawingError::shape("the drawing needs a non-null shape"));
    }
    Ok(())
}

fn draw_view(
    shape: &Shape,
    name: ViewName,
    tolerance: f64,
    max_points: usize,
) -> Result<ViewDrawing, DrawingError> {
    let (direction, u_axis) = view_basis(name);
    let drawing = HlrDrawing::create(shape, &direction, &u_axis, tolerance)?;
    let visible = dedupe_polylines(drawing.lines(false)?);
    let hidden = dedupe_polylines(drawing.lines(true)?);

    let mut kept_visible = Vec::new();
    let mut kept_hidden = Vec::new();
    let mut used = 0usize;
    let mut truncated = false;
    for polyline in visible {
        if used + polyline.point_count() > max_points {
            truncated = true;
            break;
        }
        used += polyline.point_count();
        kept_visible.push(polyline);
    }
    for polyline in hidden {
        if used + polyline.point_count() > max_points {
            truncated = true;
            break;
        }
        used += polyline.point_count();
        kept_hidden.push(polyline);
    }
    Ok(ViewDrawing {
        name,
        visible: kept_visible,
        hidden: kept_hidden,
        truncated,
    })
}

/// One cut loop, with the policy numbers Rust owns.
struct LoopRecord {
    /// The adapter's loop index, needed to ask the kernel for the net face.
    wire_index: usize,
    points: Vec<Point2>,
    bounds: Bounds2D,
    area_mm2: f64,
    area_exact: bool,
    simple: bool,
    depth: usize,
    parent: Option<usize>,
}

/// The kernel facts of one cut loop.
struct LoopInfo {
    point_count: i64,
    length: f64,
    area: f64,
    area_measured: bool,
    simple: bool,
}

fn classify_loops(records: &mut [LoopRecord]) {
    for index in 0..records.len() {
        let centroid = centroid_of(&records[index].points);
        let own_area = records[index].area_mm2;
        let Some(centroid) = centroid else {
            continue;
        };
        let mut depth = 0usize;
        let mut parent = None;
        let mut parent_area = f64::INFINITY;
        for (other_index, other) in records.iter().enumerate() {
            if other_index == index {
                continue;
            }
            if other.area_mm2 <= own_area + crate::drawing::AREA_EPSILON_MM2 {
                continue;
            }
            if !point_in_polygon(centroid, &other.points) {
                continue;
            }
            depth += 1;
            if other.area_mm2 < parent_area {
                parent_area = other.area_mm2;
                parent = Some(other_index);
            }
        }
        records[index].depth = depth;
        records[index].parent = parent;
    }
}

fn net_area(section: &SectionHandle, records: &[LoopRecord]) -> Result<(f64, bool), DrawingError> {
    let mut total = 0.0;
    for (index, region) in records.iter().enumerate() {
        if region.depth % 2 != 0 {
            continue;
        }
        let children = records
            .iter()
            .enumerate()
            .filter(|(_, child)| child.parent == Some(index))
            .map(|(child_index, _)| child_index)
            .collect::<Vec<_>>();
        if !region.simple || children.iter().any(|&child| !records[child].simple) {
            return Ok(signed_area_sum(records));
        }
        let holes = children
            .iter()
            .map(|&child| records[child].wire_index as i64)
            .collect::<Vec<_>>();
        match section.net_area(region.wire_index as i64, &holes)? {
            Some(area) if net_area_is_consistent(area, region, &children, records) => {
                total += area;
            }
            _ => return Ok(signed_area_sum(records)),
        }
    }
    Ok((total, true))
}

fn net_area_is_consistent(
    area: f64,
    region: &LoopRecord,
    children: &[usize],
    records: &[LoopRecord],
) -> bool {
    let slack = AREA_CONSISTENCY_RATIO * region.area_mm2.abs().max(1.0);
    let removed = children
        .iter()
        .map(|&child| records[child].area_mm2)
        .sum::<f64>();
    let lowest = region.area_mm2 - removed - slack;
    let highest = region.area_mm2 + slack;
    lowest <= area && area <= highest
}

fn signed_area_sum(records: &[LoopRecord]) -> (f64, bool) {
    let total = records
        .iter()
        .map(|record| {
            if record.depth % 2 == 0 {
                record.area_mm2
            } else {
                -record.area_mm2
            }
        })
        .sum();
    (total, records.iter().all(|record| record.area_exact))
}

/// Drop polylines that repeat an earlier one exactly.
///
/// The hidden-line algorithm reports the same edge more than once when the
/// edge is split across several statuses. The duplicates carry no extra
/// geometry, so the drawing keeps one copy of each, preserving the order of
/// the first occurrence exactly like Python's tuple-keyed set.
///
/// The key is the point chain alone: Python's set is keyed on `points` and
/// ignores `closed`, so two polylines with the same chain are one line even if
/// their closing flags differ.
fn dedupe_polylines(polylines: Vec<Polyline>) -> Vec<Polyline> {
    let mut seen: HashMap<u64, Vec<usize>> = HashMap::new();
    let mut unique: Vec<Polyline> = Vec::with_capacity(polylines.len());
    for polyline in polylines {
        let key = polyline_hash(&polyline);
        let duplicate = seen.get(&key).is_some_and(|indices| {
            indices
                .iter()
                .any(|&index| unique[index].points == polyline.points)
        });
        if duplicate {
            continue;
        }
        seen.entry(key).or_default().push(unique.len());
        unique.push(polyline);
    }
    unique
}

fn polyline_hash(polyline: &Polyline) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut mix = |value: u64| {
        hash ^= value;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    mix(polyline.points.len() as u64);
    for point in &polyline.points {
        for value in [point.u, point.v] {
            // `-0.0 == 0.0`, so both must hash alike.
            let bits = if value == 0.0 {
                0.0_f64.to_bits()
            } else {
                value.to_bits()
            };
            mix(bits);
        }
    }
    hash
}

fn draw_error(status: i32, error: &FfiError) -> DrawingError {
    DrawingError::kernel(format!(
        "the geometry kernel could not draw the shape: {}",
        ffi::exception_class(status, error)
    ))
}

fn cut_error(status: i32, error: &FfiError) -> DrawingError {
    DrawingError::kernel(format!(
        "the geometry kernel could not cut the shape: {}",
        ffi::exception_class(status, error)
    ))
}

/// The owned drawing of one view, freed on drop.
struct HlrDrawing {
    handle: NonNull<c_void>,
}

impl HlrDrawing {
    fn create(
        shape: &Shape,
        direction: &[f64; 3],
        u_axis: &[f64; 3],
        tolerance: f64,
    ) -> Result<Self, DrawingError> {
        let mut raw: *mut c_void = std::ptr::null_mut();
        let mut error = FfiError::default();
        // SAFETY: the shape is live, both axis arrays are 3 doubles, and the
        // out-pointer is valid for the call.
        let status = unsafe {
            ffi::kala_hlr_views(
                shape.as_ptr(),
                direction.as_ptr(),
                u_axis.as_ptr(),
                tolerance,
                &mut raw,
                &mut error,
            )
        };
        if status != ffi::OK {
            return Err(draw_error(status, &error));
        }
        NonNull::new(raw)
            .map(|handle| Self { handle })
            .ok_or_else(|| draw_error(ffi::EXCEPTION, &FfiError::default()))
    }

    fn lines(&self, hidden: bool) -> Result<Vec<Polyline>, DrawingError> {
        let flag = if hidden { 1 } else { 0 };
        // SAFETY: the handle is live for `&self` and the flag is 0 or 1.
        let count = unsafe { ffi::kala_drawing_line_count(self.handle.as_ptr(), flag) };
        if count < 0 {
            return Err(draw_error(ffi::EXCEPTION, &FfiError::default()));
        }
        let mut polylines = Vec::with_capacity(count as usize);
        for index in 0..count {
            // SAFETY: the handle is live and the index is inside the count.
            let point_count =
                unsafe { ffi::kala_drawing_point_count(self.handle.as_ptr(), flag, index) };
            let capacity = flat_capacity(point_count, || {
                draw_error(ffi::EXCEPTION, &FfiError::default())
            })?;
            let mut buffer = vec![0.0_f64; capacity];
            let mut error = FfiError::default();
            // SAFETY: the destination has room for `capacity` doubles.
            let status = unsafe {
                ffi::kala_drawing_copy_points(
                    self.handle.as_ptr(),
                    flag,
                    index,
                    buffer.as_mut_ptr(),
                    capacity,
                    &mut error,
                )
            };
            if status != ffi::OK {
                return Err(draw_error(status, &error));
            }
            // SAFETY: the handle is live and the index is inside the count.
            let closed =
                unsafe { ffi::kala_drawing_is_closed(self.handle.as_ptr(), flag, index) } != 0;
            let points = buffer
                .chunks_exact(2)
                .map(|pair| Point2::new(pair[0], pair[1]))
                .collect();
            polylines.push(Polyline::new(points, closed));
        }
        Ok(polylines)
    }
}

impl Drop for HlrDrawing {
    fn drop(&mut self) {
        // SAFETY: this is the unique owner of the pointer; Drop runs once.
        unsafe { ffi::kala_drawing_free(self.handle.as_ptr()) };
    }
}

enum SectionCreate {
    Created(SectionHandle),
    NotDone,
}

/// The owned section cut, freed on drop. It keeps the cut wires so the net
/// face of a region can be built and measured after classification.
struct SectionHandle {
    handle: NonNull<c_void>,
}

impl SectionHandle {
    fn create(
        shape: &Shape,
        normal: &[f64; 3],
        offset: f64,
        u_axis: &[f64; 3],
        v_axis: &[f64; 3],
        tolerance: f64,
    ) -> Result<SectionCreate, DrawingError> {
        let mut raw: *mut c_void = std::ptr::null_mut();
        let mut error = FfiError::default();
        // SAFETY: the shape is live, each axis array is 3 doubles, and the
        // out-pointer is valid for the call.
        let status = unsafe {
            ffi::kala_section_create(
                shape.as_ptr(),
                normal.as_ptr(),
                offset,
                u_axis.as_ptr(),
                v_axis.as_ptr(),
                tolerance,
                &mut raw,
                &mut error,
            )
        };
        match status {
            ffi::OK => NonNull::new(raw)
                .map(|handle| SectionCreate::Created(Self { handle }))
                .ok_or_else(|| cut_error(ffi::EXCEPTION, &FfiError::default())),
            ffi::NOT_DONE => Ok(SectionCreate::NotDone),
            status => Err(cut_error(status, &error)),
        }
    }

    fn loop_count(&self) -> Result<usize, DrawingError> {
        // SAFETY: the handle is live for `&self`.
        let count = unsafe { ffi::kala_section_loop_count(self.handle.as_ptr()) };
        usize::try_from(count).map_err(|_| cut_error(ffi::EXCEPTION, &FfiError::default()))
    }

    fn loop_info(&self, index: usize) -> Result<LoopInfo, DrawingError> {
        let mut info = FfiSectionLoopInfo::default();
        let mut error = FfiError::default();
        // SAFETY: the handle is live; the out-pointer is valid.
        let status = unsafe {
            ffi::kala_section_loop_info(self.handle.as_ptr(), index as i64, &mut info, &mut error)
        };
        if status != ffi::OK {
            return Err(cut_error(status, &error));
        }
        Ok(LoopInfo {
            point_count: info.point_count,
            length: info.length,
            area: info.area,
            area_measured: info.area_measured != 0,
            simple: info.simple != 0,
        })
    }

    fn loop_points(&self, index: usize, point_count: i64) -> Result<Vec<Point2>, DrawingError> {
        let capacity = flat_capacity(point_count, || {
            cut_error(ffi::EXCEPTION, &FfiError::default())
        })?;
        let mut buffer = vec![0.0_f64; capacity];
        let mut error = FfiError::default();
        // SAFETY: the destination has room for `capacity` doubles.
        let status = unsafe {
            ffi::kala_section_loop_copy_points(
                self.handle.as_ptr(),
                index as i64,
                buffer.as_mut_ptr(),
                capacity,
                &mut error,
            )
        };
        if status != ffi::OK {
            return Err(cut_error(status, &error));
        }
        Ok(buffer
            .chunks_exact(2)
            .map(|pair| Point2::new(pair[0], pair[1]))
            .collect())
    }

    /// Measure the exact face of a region with its holes cut out.
    ///
    /// `None` means the kernel could not build the face or produced no usable
    /// area; the caller falls back to the summed loop areas and reports the
    /// net area as not exact.
    fn net_area(&self, outer: i64, holes: &[i64]) -> Result<Option<f64>, DrawingError> {
        let mut area = 0.0;
        let mut error = FfiError::default();
        // SAFETY: the handle is live, every index is inside the loop list, and
        // the out-pointer is valid.
        let status = unsafe {
            ffi::kala_section_net_area(
                self.handle.as_ptr(),
                outer,
                holes.as_ptr(),
                holes.len(),
                &mut area,
                &mut error,
            )
        };
        match status {
            ffi::OK => Ok(Some(area)),
            ffi::NOT_DONE => Ok(None),
            status => Err(cut_error(status, &error)),
        }
    }
}

impl Drop for SectionHandle {
    fn drop(&mut self) {
        // SAFETY: this is the unique owner of the pointer; Drop runs once.
        unsafe { ffi::kala_section_free(self.handle.as_ptr()) };
    }
}

/// The number of doubles in a flat `(u, v)` buffer, with overflow checks.
fn flat_capacity<F>(point_count: i64, error: F) -> Result<usize, DrawingError>
where
    F: FnOnce() -> DrawingError,
{
    usize::try_from(point_count)
        .ok()
        .and_then(|value| value.checked_mul(2))
        .ok_or_else(error)
}
