//! The native sandbox probe helper.
//!
//! The helper is the child of [`kala_sandbox::probe_sandbox`]. It builds the
//! strict jail in its own process, verifies the controls from the inside and
//! writes one JSON line to standard output. It is not a product entry point
//! and the library never requires it to exist.

use kala_sandbox::{PROBE_FAILURE_EXIT, run_probe};

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() < 3 {
        eprintln!("the probe payload is malformed");
        std::process::exit(PROBE_FAILURE_EXIT);
    }
    let code = run_probe(&arguments[1], &arguments[2]);
    std::process::exit(code);
}
