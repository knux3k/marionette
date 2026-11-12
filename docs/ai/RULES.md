# Project rules

1. Marionette is designed for Claude and MUST work with any coding agent: everything is plain
   files, a CLI with JSON output, and MCP.
2. The AI maintainer owns design and build decisions. Decide, record the decision in
   `docs/decisions/` (index in DECISIONS.md), and continue; do not ask the owner about design.
3. Ask the owner first before anything irreversible or outside the codebase: pushing,
   publishing, spending money or API credits, deleting anything outside the repository, creating
   remotes, opening windows or playing sound on a user's screen or speakers.
4. Providers are cloud APIs with each user's own keys (environment variables only); local models
   are allowed; one command switches a provider.
5. AI while playing is off by default; a game opts in explicitly.
6. VR is never required; it is an optional module.
7. License: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`); copyright lines read "The
   Marionette Contributors". The asset pool is CC0 (ASSETS-AND-LICENSES.md).
8. Permanent rule: no names, identifying details or machine details in the repository (decision
   0016). That includes commit messages. Use "the project owner", "the engine repository", "the
   assets repository", generic hardware classes, and repo-relative paths. The check is
   `crates/marionette-cli/tests/repo_hygiene.rs` with `docs/ai/forbidden-patterns.txt`.
9. Commits use the identity `Marionette Contributors` (repository-local git config). Small
   commits with clear messages. Credit AI authorship with a plain `Co-Authored-By: Claude`
   trailer: no email address, no session or chat links.
10. Determinism rules (ARCHITECTURE.md) are not optional: a change that alters an existing
    state hash or reference image must say so in its commit and in PROOFS.md, with the reason.
11. Errors are data: stable codes, never change what an existing code means.
12. Reports to the owner are under 10 lines, in plain words.
13. Every session ends by updating STATE.md and adding a SESSION-LOG.md row (START-HERE.md,
    "Every session ends by").
14. Every game mode, example scene and proof scene has a stability mode in
    `stability/suite.json` (STABILITY.md, decision 0031); add it in the same change.
15. Weigh every change against the four focuses (FOCUSES.md): modularity, token use,
    performance, graphics.
