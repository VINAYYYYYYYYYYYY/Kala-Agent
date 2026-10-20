//! Platform-specific, descriptor/handle-relative filesystem primitives.
//!
//! Only the two supported product-development families are compiled with a
//! real implementation.  Other targets fail closed instead of falling back to
//! path canonicalization.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(crate) use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::*;

#[cfg(not(any(unix, windows)))]
mod unsupported;
#[cfg(not(any(unix, windows)))]
pub(crate) use unsupported::*;
