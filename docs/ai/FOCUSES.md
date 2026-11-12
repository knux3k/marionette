# The four focuses

The owner set four standing focuses for the engine (2026-11-12, decision 0060). Every change is
weighed against them. When two choices are otherwise equal, take the one that serves more of
them; when a change hurts one, say so in its decision record.

1. **Modularity, including custom code.**
2. **Limit context memory and optimize token usage.**
3. **Performance.**
4. **Graphics.**

## What each focus means here

### 1. Modularity, including custom code

- Every engine feature is a crate (or a `Module`, decision 0011) that a game can take or leave.
  Lower crates never depend on higher ones (crate map in ARCHITECTURE.md).
- A game's own code plugs in without forking the engine: TypeScript scripts (decision 0022),
  job TypeScript (decision 0030), and Rust modules and operations of its own (decision 0062,
  HOWTO.md "Add a game's own Rust module").
- Check: a new feature adds a crate or module, not branches inside an unrelated one.

### 2. Limit context memory and optimize token usage

This is about the AI that works on the engine and on games made with it. A task should need as
little text loaded as possible.

- Docs are small and split by task. Always-read docs stay under the budget in
  `crates/marionette-cli/tests/doc_budget.rs` (decision 0061). Long proof lists, logs and
  references are read only when a task needs them.
- Operation outputs are compact by default: paged lists, paths instead of pixels, summaries
  before details, no repeated input.
- MCP clients can load only the operation groups they need (`marionette mcp --tools`), and
  tool listings can leave out output schemas (`--lean`).
- Check: before adding a doc section or an output field, ask who reads it and how often.

### 3. Performance

- Budgets and how to measure: PERFORMANCE.md (decisions 0023 to 0026). Jobs: decision 0024.
  Stability per game mode: STABILITY.md.
- Check: a change on a hot path (tick, render, load) runs the perf proofs; a new budget gets a
  test.

### 4. Graphics

- The renderer (ARCHITECTURE.md, "Renderer") must look good, not only draw correctly: lighting,
  shadows, materials, sky, post-processing, at the performance budgets above.
- Every visual feature is opt-in through scene data, so existing reference images and scenes do
  not change by surprise.
- Check: a visual change adds or updates a reference image and says so in PROOFS.md.

## Gaps (2026-11-12 audit)

Ordered by size. Each line is a piece of work for one session; mark it done here when it lands.

| Focus | Gap | Status |
| --- | --- | --- |
| Tokens | START-HERE read order loaded about 75 KB (STATE.md alone 40 KB) before any task | done: 18.7 KB always-read set, budget test (decision 0061) |
| Tokens | `marionette mcp` listed all 72 operations with input and output schemas (158 KB) | done: `--tools core --lean` is 27.6 KB (decision 0061) |
| Graphics | No shadows; directional lights only (no point or spot lights) | done: `Shadows`, `PointLight`, `SpotLight` (decision 0063); shadows from local lights and cascades open |
| Graphics | No HDR or tone mapping (colors clamp at 1.0); no emissive materials; no bloom | done: `ToneMapping`, `Emissive` (decision 0063); bloom, glTF emissive open |
| Graphics | No sky, environment lighting or fog (ambient is a flat color) | done: `SkyLight`, `Fog` (decision 0063); image-based lighting, sky background open |
| Modularity | The module set and operation table are fixed in `marionette-agent`: a game cannot add a Rust module or operation without editing the engine | done: `Engine::builder()`, `marionette_cli::main_with`, template `examples/rust-module` (decision 0062) |
| Modularity | `marionette-agent` links every crate: no cargo features to leave out import, pool, generation, porting or job TypeScript | done: features `script`, `import`, `pool`, `generate`, `port`, `jobs-ts`, CI lean build (decision 0062) |
| Modularity | Job TypeScript cannot be called from scenes or scripts yet (WJ next) | open |
| Performance | No hardware-GPU run of frame budgets yet (needs the owner's PC) | open |
