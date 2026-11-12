# Scripting spike (plan line 4)

The measurement harness behind decision 0019 (`docs/decisions/0019-scripting-language.md`).
The same game script in Luau (`src/luau.rs`, mlua) and TypeScript (`src/ts.rs`, oxc type
stripping + QuickJS-ng through rquickjs), measured the same way. Not a workspace member, so CI
does not build it; results are in the decision record.

```bash
cargo run --release --features luau -- all      # startup, compile, reload, call cost, math, sandbox globals
cargo run --release --features ts -- all
cargo run --release --features luau -- order    # run twice: object-key iteration order per process
cargo run --release --features luau,ts -- eval  # eval/tasks.json against eval/tasks.luau and eval/tasks.ts
```

`eval/` holds ten small gameplay tasks with test cases; `tasks.luau` and `tasks.ts` are
first-attempt solutions written by a coding AI from the task text and not edited after running.
Use a separate `--target-dir` per feature set to measure build time and binary size.
