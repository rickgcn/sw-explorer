//! Dumps every diagnostic produced while opening the given
//! distributions. Research/debugging utility.
//!
//! Usage:
//!
//! ```sh
//! cargo run -p sw-core --example dist-diagnostics -- <dist-dir>...
//! ```
use std::path::PathBuf;
use std::process::ExitCode;
use sw_core::distribution::Distribution;

fn main() -> ExitCode {
    let mut total = 0usize;
    for arg in std::env::args_os().skip(1) {
        let path = PathBuf::from(arg);
        let dist = match Distribution::open(&path) {
            Ok(dist) => dist,
            Err(error) => {
                eprintln!("{}: cannot open: {error}", path.display());
                continue;
            }
        };
        for diagnostic in dist.all_diagnostics() {
            total += 1;
            let origin = diagnostic
                .origin
                .as_ref()
                .map(|o| format!(" ({o})"))
                .unwrap_or_default();
            println!(
                "{:?}: {:?}{origin}",
                diagnostic.severity, diagnostic.message
            );
        }
    }
    eprintln!("{total} diagnostics");
    ExitCode::SUCCESS
}
