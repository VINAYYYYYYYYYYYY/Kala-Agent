//! The `kala-agent` binary: `python -m kala_agent.cli`'s native counterpart.

use std::io;

fn main() {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let code = kala_cli::main(None, None, None, &mut stdout, &mut stderr);
    std::process::exit(code);
}
