//! The native init helper of the production worker jail.
//!
//! The helper is the process the accepted `kala-process` supervisor starts
//! instead of the worker. It builds the strict jail — namespaces, PID
//! namespace, mounts, `chroot` — and then runs the worker inside it as a
//! child of the namespace init, so that the worker cannot see the host
//! process table and the whole namespace dies with the jail.
//!
//! The helper is not a product entry point: the library never requires it to
//! exist, and the probe helper (`kala-sandbox-probe`) is a separate binary
//! that shares the same jail-building code.

use std::ffi::OsString;

use kala_sandbox::run_init;

fn main() {
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    std::process::exit(run_init(&arguments));
}
