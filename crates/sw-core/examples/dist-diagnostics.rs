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
        for diagnostic in dist.diagnostics() {
            total += 1;
            println!("dist {:?}: {:?}", diagnostic.severity, diagnostic.message);
        }
        for product in dist.products() {
            for diagnostic in &product.diagnostics {
                total += 1;
                println!(
                    "{} {:?}: {:?} {:?}",
                    product.name, diagnostic.severity, diagnostic.message, diagnostic.origin
                );
            }
        }
    }
    eprintln!("{total} diagnostics");
    ExitCode::SUCCESS
}
