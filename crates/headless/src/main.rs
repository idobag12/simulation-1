//! `embervale` — thin binary shim over [`headless::cli`] (SPEC §13). All
//! logic lives in the library so it is unit-tested; this file only collects
//! argv and forwards.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    headless::cli::main_with_args(&args)
}
