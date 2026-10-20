//! Raw C ABI declarations for the narrow OpenCascade adapter.
//!
//! Nothing above this module sees a kernel pointer. Every function returns a
//! status code (or an opaque owned pointer) and writes a bounded exception
//! class name into [`FfiError`]. The adapter catches all C++ exceptions, so a
//! `Standard_Failure` never unwinds into Rust.

#![allow(unsafe_code)]

use std::ffi::{c_char, c_void};

pub(crate) const OK: i32 = 0;
pub(crate) const NOT_DONE: i32 = 1;
pub(crate) const NULL_RESULT: i32 = 2;
pub(crate) const EXCEPTION: i32 = 3;
pub(crate) const INVALID_ARGUMENT: i32 = 4;
pub(crate) const OVER_VERTEX: i32 = 5;
pub(crate) const OVER_TRIANGLE: i32 = 6;
pub(crate) const NO_TRIANGLES: i32 = 7;
pub(crate) const VOID_BOX: i32 = 8;
pub(crate) const NORMALS_FAILED: i32 = 9;

pub(crate) const ERROR_MESSAGE_SIZE: usize = 1024;

/// Bounded error transport written by the C++ adapter.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct FfiError {
    pub code: i32,
    pub message_len: u32,
    pub message: [u8; ERROR_MESSAGE_SIZE],
}

impl Default for FfiError {
    fn default() -> Self {
        Self {
            code: OK,
            message_len: 0,
            message: [0; ERROR_MESSAGE_SIZE],
        }
    }
}

impl FfiError {
    /// The exception class name (or the adapter's static marker) as UTF-8.
    pub(crate) fn text(&self) -> String {
        let length = (self.message_len as usize).min(ERROR_MESSAGE_SIZE);
        String::from_utf8_lossy(&self.message[..length]).into_owned()
    }
}

/// Kernel measurement fields, matching the C struct layout.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct FfiMeasurement {
    pub volume: f64,
    pub area: f64,
    pub centroid: [f64; 3],
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub solid_count: i64,
    pub valid: i32,
    pub reserved: i32,
}

/// Topology occurrence counts, matching the C struct layout.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct FfiTopology {
    pub solids: i64,
    pub shells: i64,
    pub faces: i64,
    pub edges: i64,
    pub vertices: i64,
}

/// Publishability fields, matching the C struct layout.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct FfiPublishState {
    pub solid_count: i64,
    pub volume: f64,
    pub is_null: i32,
    pub valid: i32,
}

/// One section-cut loop as the adapter measured it, matching the C layout.
///
/// `area_measured` is 1 only when the kernel built the planar face of a simple
/// wire and measured a finite positive area. `simple` is 1 when the wire walks
/// its boundary once. The Rust layer owns the area-consistency policy and the
/// outer/hole classification.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct FfiSectionLoopInfo {
    pub point_count: i64,
    pub length: f64,
    pub area: f64,
    pub area_measured: i32,
    pub simple: i32,
    pub reserved: i32,
}

unsafe extern "C" {
    pub(crate) fn kala_occt_version() -> *const c_char;

    pub(crate) fn kala_shape_new_null(error: *mut FfiError) -> *mut c_void;
    pub(crate) fn kala_shape_copy(handle: *const c_void, error: *mut FfiError) -> *mut c_void;
    pub(crate) fn kala_shape_free(handle: *mut c_void);
    pub(crate) fn kala_shape_is_null(handle: *const c_void) -> i32;
    pub(crate) fn kala_shape_solid_count(handle: *const c_void, error: *mut FfiError) -> i64;
    pub(crate) fn kala_shape_is_valid(
        handle: *const c_void,
        out: *mut i32,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_shape_publish_state(
        handle: *const c_void,
        out: *mut FfiPublishState,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_shape_volume(
        handle: *const c_void,
        out: *mut f64,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_shape_topology(
        handle: *const c_void,
        out: *mut FfiTopology,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_shape_measure(
        handle: *const c_void,
        out: *mut FfiMeasurement,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_optimal_box_ex(
        handle: *const c_void,
        use_triangulation: i32,
        out: *mut f64,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_shape_y_midpoint(
        handle: *const c_void,
        out: *mut f64,
        error: *mut FfiError,
    ) -> i32;

    pub(crate) fn kala_hlr_views(
        handle: *const c_void,
        direction: *const f64,
        u_axis: *const f64,
        tolerance: f64,
        out: *mut *mut c_void,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_drawing_line_count(drawing: *const c_void, hidden: i32) -> i64;
    pub(crate) fn kala_drawing_point_count(drawing: *const c_void, hidden: i32, line: i64) -> i64;
    pub(crate) fn kala_drawing_is_closed(drawing: *const c_void, hidden: i32, line: i64) -> i32;
    pub(crate) fn kala_drawing_copy_points(
        drawing: *const c_void,
        hidden: i32,
        line: i64,
        destination: *mut f64,
        capacity: usize,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_drawing_free(drawing: *mut c_void);

    pub(crate) fn kala_section_create(
        handle: *const c_void,
        normal: *const f64,
        offset: f64,
        u_axis: *const f64,
        v_axis: *const f64,
        tolerance: f64,
        out: *mut *mut c_void,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_section_loop_count(section: *const c_void) -> i64;
    pub(crate) fn kala_section_loop_info(
        section: *const c_void,
        index: i64,
        out: *mut FfiSectionLoopInfo,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_section_loop_copy_points(
        section: *const c_void,
        index: i64,
        destination: *mut f64,
        capacity: usize,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_section_net_area(
        section: *const c_void,
        outer: i64,
        holes: *const i64,
        hole_count: usize,
        out: *mut f64,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_section_free(section: *mut c_void);

    pub(crate) fn kala_polygon_face(
        points: *const f64,
        count: i32,
        out: *mut *mut c_void,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_circle_face(
        cx: f64,
        cy: f64,
        cz: f64,
        nx: f64,
        ny: f64,
        nz: f64,
        radius: f64,
        out: *mut *mut c_void,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_prism(
        face: *const c_void,
        dx: f64,
        dy: f64,
        dz: f64,
        out: *mut *mut c_void,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_revolve_y(
        face: *const c_void,
        radians: f64,
        out: *mut *mut c_void,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_boolean_cut(
        left: *const c_void,
        right: *const c_void,
        out: *mut *mut c_void,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_boolean_fuse(
        left: *const c_void,
        right: *const c_void,
        out: *mut *mut c_void,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_compound(
        shapes: *const *const c_void,
        count: usize,
        out: *mut *mut c_void,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_translate(
        handle: *const c_void,
        dx: f64,
        dy: f64,
        dz: f64,
        out: *mut *mut c_void,
        error: *mut FfiError,
    ) -> i32;

    pub(crate) fn kala_mesh_create(
        handle: *const c_void,
        deflection: f64,
        angular_deflection: f64,
        parallel: i32,
        max_vertices: i64,
        max_triangles: i64,
        error: *mut FfiError,
    ) -> *mut c_void;
    pub(crate) fn kala_mesh_vertex_count(mesh: *const c_void) -> i64;
    pub(crate) fn kala_mesh_triangle_count(mesh: *const c_void) -> i64;
    pub(crate) fn kala_mesh_copy(
        mesh: *const c_void,
        positions: *mut f64,
        positions_len: usize,
        normals: *mut f64,
        normals_len: usize,
        indices: *mut u32,
        indices_len: usize,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_mesh_free(mesh: *mut c_void);

    pub(crate) fn kala_buffer_size(buffer: *const c_void) -> i64;
    pub(crate) fn kala_buffer_copy(
        buffer: *const c_void,
        destination: *mut u8,
        capacity: usize,
        error: *mut FfiError,
    ) -> i32;
    pub(crate) fn kala_buffer_free(buffer: *mut c_void);
    pub(crate) fn kala_step_export(handle: *const c_void, error: *mut FfiError) -> *mut c_void;
    pub(crate) fn kala_step_import(
        data: *const u8,
        length: usize,
        error: *mut FfiError,
    ) -> *mut c_void;
}

/// The linked OpenCascade version, owned as a Rust string.
pub(crate) fn occt_version() -> String {
    // SAFETY: the adapter returns a pointer to a static compile-time string
    // literal; it is never freed and remains valid for the process lifetime.
    let pointer = unsafe { kala_occt_version() };
    if pointer.is_null() {
        return "unknown".to_owned();
    }
    // SAFETY: `pointer` is a valid NUL-terminated static string.
    unsafe { std::ffi::CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned()
}

/// Map a status to an exception class name, for the `EXCEPTION` status only.
pub(crate) fn exception_class(status: i32, error: &FfiError) -> String {
    if status == EXCEPTION {
        let text = error.text();
        if text.is_empty() {
            "unknown".to_owned()
        } else {
            text
        }
    } else {
        "invalid argument".to_owned()
    }
}
