//! The native init helper of the tools crate.
//!
//! The helper is the process the supervisor starts instead of the worker. It
//! builds the strict jail — namespaces, PID namespace, mounts, `chroot` — and
//! then runs the worker inside it as a child of the namespace init, so that
//! the worker cannot see the host process table and the whole namespace dies
//! with the jail.
//!
//! The helper is a thin launcher: the jail itself is built by the accepted
//! `kala-sandbox` crate, so the control the worker is given is exactly the
//! control the accepted production worker jail establishes.

use std::ffi::OsString;

use kala_sandbox::run_init;

fn main() {
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    std::process::exit(run_init(&arguments));
}
