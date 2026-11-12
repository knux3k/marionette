//! `spring-game`: every `marionette` command (run, op, ops, mcp, ...) with this game's module
//! and operation added.

use std::process::ExitCode;

/// Counted heap, as in the `marionette` binary (decision 0032).
#[global_allocator]
static ALLOC: marionette_host::alloc::Counting = marionette_host::alloc::Counting;

fn main() -> ExitCode {
    match marionette_example_rust_module::engine() {
        Ok(engine) => marionette_cli::main_with(engine),
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
