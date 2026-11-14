# 0106: Reference counts are cloned with `Arc::clone`, and the `Box<dyn>` review

Date: 2026-11-14. Status: accepted. Follows decisions 0104 and 0105 (the owner's code rules,
rules 3 and 4).

## Decision

1. Every clone of an `Arc`, `Rc` or `Weak` is written `Arc::clone(&x)` (70 sites changed,
   34 of them the script bridge handed to each engine function), so a reader sees a counter
   increment and not a deep copy. Every crate root denies `clippy::clone_on_ref_ptr`; the
   `repo_hygiene` test checks the line next to the 0105 one.
2. The 25 clones clippy's `redundant_clone` found (a value cloned and then never used again)
   are removed; the rest of the `.clone()` calls hand owned data to an output or another
   thread and stay. The one left in the stability harness has a comment.
3. All 40 `Box<dyn ..>` were reviewed. Each is a real plug-in point with several
   implementations chosen at run time: engine modules, AI providers and HTTP transports,
   registry reflection values, audio generators and decoders, and lazily built render
   pipelines. None would be simpler as a struct or an enum, so none changed.

## Proof

State hashes and allocations per tick identical on every `sim/` case of `perf_suite`
(before and after, two alternated runs each); tick times within the cloud's run-to-run noise.
Drift hash unchanged; all tests pass.
