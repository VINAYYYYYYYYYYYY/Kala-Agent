//! Native strict-sandbox probe helper.
//!
//! This is deliberately a separate executable: the probe builds the jail in a
//! fresh, single-threaded process and then verifies it from the inside. The
//! library never executes Python for isolation.
//!
//! Usage: `kala-process-probe <spec-json> <canary-path-json>`.
//! Exit codes: `0` verified, `122` unavailable/failed.

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() != 3 {
        std::process::exit(2);
    }
    std::process::exit(kala_process::run_probe(&arguments[1], &arguments[2]));
}
