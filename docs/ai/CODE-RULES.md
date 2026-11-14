# Rust code rules

The owner's rules for engine code (decision 0104), written for an agent changing it. They
apply to new and touched code; the cleanup of older code is tracked in STATE.md.

## Rules

1. **No `unsafe` without the owner's review.** Only calls into C libraries or the OS may use
   it, in the files listed in `UNSAFE_ALLOWED` in `crates/marionette-cli/tests/repo_hygiene.rs`,
   every block with a `// SAFETY:` comment. Every other crate denies `unsafe_code` through
   `[workspace.lints]` (`[lints] workspace = true` in its `Cargo.toml`). If a change seems to
   need `unsafe`, stop and bring the owner the exact code, why safe code cannot do it, the
   measured gain and the invariants.
2. **No `.unwrap()` or `.expect()` outside `#[cfg(test)]` and tests.** Return
   `Result<T, Errors>` (or the crate's error type) and use `?`. A value that truly cannot be
   missing still gets a fallback or an error, not a panic.
3. **Borrow before cloning.** Pass `&T`, `&mut T`, `&[T]`, `&str`. A `.clone()` that stays
   needs a short comment saying why (ownership crosses a thread, a cheap `Arc` count, a value
   kept past the borrow). Write `Arc::clone(&x)` for reference counts so it reads as cheap.
4. **Flat and concrete.** Plain `struct`s and `enum`s and contiguous `Vec`s first. Generics,
   lifetimes and `Box<dyn Trait>` only where several real types must share one path (module
   and operation registries, provider plug-ins, audio generators).
5. **Data structures only as complex as they need to be.** In hot paths (per frame, per
   entity, per instance) prefer parallel arrays and indices over maps of maps; reuse buffers
   across frames instead of allocating.
6. **Clippy-clean** with the workspace settings: `cargo clippy --all-targets -- -D warnings`
   on the newest stable Rust (CI uses it; run `rustup toolchain install stable` first).

## For reviewers coming from C

Pull requests explain ownership in C terms: which struct owns the heap data (`Vec` is a
`malloc`ed array with length and capacity, `Box` a single `malloc`ed object), how it is lent
(`&T` is a `const *` the compiler proves outlives its use, `&mut T` an exclusive pointer),
`Option` and `Result` as tagged unions in place of `NULL` and return codes, a `dyn Trait` as a
vtable pointer, `Arc` as a reference count, `Mutex` and `RefCell` as checked locks.
