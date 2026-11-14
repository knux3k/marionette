//! `factory-bench`: every `marionette` command (run, op, ops, mcp, perf_suite, stability, ...)
//! with the factory example module and its `perf_factory_scene` operation added.

use std::process::ExitCode;

/// Counted heap, as in the `marionette` binary (decision 0032).
#[global_allocator]
static ALLOC: marionette_host::alloc::Counting = marionette_host::alloc::Counting;

fn main() -> ExitCode {
    match marionette_example_factory::engine() {
        Ok(engine) => marionette_cli::main_with(engine),
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
