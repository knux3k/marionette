//! The `marionette` binary: the CLI library with the built-in engine.

use std::process::ExitCode;

/// Counted heap, so `live_bytes` is available to memory checks on every platform (decision
/// 0032); the cost is a few relaxed atomic adds per allocation. A game's own binary declares
/// the same line to keep the heap figures.
#[global_allocator]
static ALLOC: marionette_host::alloc::Counting = marionette_host::alloc::Counting;

fn main() -> ExitCode {
    marionette_cli::main()
}
