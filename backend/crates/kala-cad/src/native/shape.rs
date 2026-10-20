//! Owning Rust handle for one native `TopoDS_Shape`.
//!
//! The handle heap-owns a C++ `TopoDS_Shape` created by the adapter. Drop calls
//! the matching `kala_shape_free` exactly once. Cloning performs a kernel handle
//! copy through the adapter, never an aliased raw pointer.
//!
//! `Shape` is deliberately **not** `Send` or `Sync`: the raw handle makes that
//! true automatically, and OCCT mutation (boolean build, tessellation) is
//! thread-affine for a given shape. A future session service should keep a
//! shape on one worker thread and move only measured plain data across
//! threads. Independent shapes may be built and measured concurrently.

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::fmt;
use std::ptr::NonNull;

use crate::error::KernelError;
use crate::native::ffi;
use crate::native::ffi::FfiError;

/// An owned exact B-Rep shape. The Rust value owns exactly one C++ handle.
///
/// A shape is deliberately thread-affine: OCCT mutation (boolean build,
/// tessellation) is not shared across threads through this handle. Move plain
/// measured data instead. These two checks must fail to compile:
///
/// ```compile_fail
/// fn assert_send<T: Send>() {}
/// assert_send::<kala_cad::Shape>();
/// ```
///
/// ```compile_fail
/// fn assert_sync<T: Sync>() {}
/// assert_sync::<kala_cad::Shape>();
/// ```
pub struct Shape {
    handle: NonNull<c_void>,
}

impl Shape {
    /// Create a kernel null shape (`TopoDS_Shape()`).
    pub fn null() -> Result<Self, KernelError> {
        let mut error = FfiError::default();
        // SAFETY: the adapter receives a valid out-error pointer and returns
        // an owned handle or null.
        let handle = unsafe { ffi::kala_shape_new_null(&mut error) };
        Self::from_raw(handle, &error)
    }

    /// Whether the underlying `TopoDS_Shape` is null.
    pub fn is_null(&self) -> bool {
        // SAFETY: `self.handle` is a live owned pointer for `&self`.
        unsafe { ffi::kala_shape_is_null(self.handle.as_ptr()) != 0 }
    }

    /// Copy the kernel handle into an independent Rust owner.
    pub fn try_clone_shape(&self) -> Result<Self, KernelError> {
        let mut error = FfiError::default();
        // SAFETY: `self.handle` is live for the duration of the call.
        let handle = unsafe { ffi::kala_shape_copy(self.handle.as_ptr(), &mut error) };
        Self::from_raw(handle, &error)
    }

    /// `BRepCheck_Analyzer` validity for this exact shape.
    pub fn is_valid(&self) -> Result<bool, KernelError> {
        let mut valid = 0_i32;
        let mut error = FfiError::default();
        // SAFETY: `self.handle` is live for the duration of the call and the
        // out-pointer is valid.
        let status =
            unsafe { ffi::kala_shape_is_valid(self.handle.as_ptr(), &mut valid, &mut error) };
        if status != ffi::OK {
            return Err(KernelError::new(ffi::exception_class(status, &error)));
        }
        Ok(valid != 0)
    }

    /// The number of `TopAbs_SOLID` sub-shapes.
    pub fn solid_count(&self) -> Result<usize, KernelError> {
        let mut error = FfiError::default();
        // SAFETY: `self.handle` is live for the duration of the call.
        let count = unsafe { ffi::kala_shape_solid_count(self.handle.as_ptr(), &mut error) };
        if count < 0 {
            return Err(KernelError::new(ffi::exception_class(error.code, &error)));
        }
        Ok(count as usize)
    }

    pub(crate) fn as_ptr(&self) -> *const c_void {
        self.handle.as_ptr()
    }

    /// Take ownership of a handle returned by the adapter.
    pub(crate) fn from_owned_raw(handle: *mut c_void) -> Result<Self, KernelError> {
        NonNull::new(handle)
            .map(|handle| Self { handle })
            .ok_or_else(|| KernelError::new("allocation"))
    }

    fn from_raw(handle: *mut c_void, error: &FfiError) -> Result<Self, KernelError> {
        NonNull::new(handle)
            .map(|handle| Self { handle })
            .ok_or_else(|| {
                let class = error.text();
                KernelError::new(if class.is_empty() {
                    "allocation".to_owned()
                } else {
                    class
                })
            })
    }
}

impl Drop for Shape {
    fn drop(&mut self) {
        // SAFETY: this is the unique owner of the pointer; Drop runs once.
        unsafe { ffi::kala_shape_free(self.handle.as_ptr()) };
    }
}

impl fmt::Debug for Shape {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Shape")
            .field("null", &self.is_null())
            .finish_non_exhaustive()
    }
}
