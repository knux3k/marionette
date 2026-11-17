//! License files for a release archive of the `marionette` binary (decision 0141).
//!
//! `cargo run -p marionette-package --example engine_licenses -- <target-triple> <out-dir>`
//! writes `marionette-LICENSE-MIT`, `marionette-LICENSE-APACHE` and `THIRD-PARTY.txt` (every
//! third-party crate linked into `marionette-cli` for that target, with its license texts).
//! Run from anywhere inside the engine workspace; the release workflow runs it per target.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [triple, out] = args.as_slice() else {
        eprintln!("usage: engine_licenses <target-triple> <out-dir>");
        return ExitCode::from(2);
    };
    // The example is built inside the workspace, so the workspace root is two folders above
    // this crate's manifest; that is where the license files and Cargo.lock live.
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    match marionette_package::licenses::write_for(
        &workspace,
        None,
        "marionette-cli",
        triple,
        "Rust crates linked into the marionette binary",
        Path::new(out),
    ) {
        Ok(count) => {
            println!("{count} third-party crates listed in {out}/THIRD-PARTY.txt");
            ExitCode::SUCCESS
        }
        Err(errors) => {
            eprintln!("{errors}");
            ExitCode::FAILURE
        }
    }
}
