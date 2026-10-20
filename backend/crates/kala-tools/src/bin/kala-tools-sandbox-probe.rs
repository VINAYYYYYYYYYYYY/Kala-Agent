//! The native sandbox probe helper of the tools crate.
//!
//! The helper is the child of the strict-profile probe. It builds the strict
//! jail in its own process, verifies the controls from the inside and writes
//! one JSON line to standard output. It is not a product entry point and the
//! library never requires it to exist.
//!
//! The helper is a thin launcher: the jail itself is built by the accepted
//! `kala-sandbox` crate, so the control that is verified here is exactly the
//! control the production worker jail establishes.

use kala_sandbox::{PROBE_FAILURE_EXIT, run_probe};

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() < 3 {
        eprintln!("the probe payload is malformed");
        std::process::exit(PROBE_FAILURE_EXIT);
    }
    std::process::exit(run_probe(&arguments[1], &arguments[2]));
}
