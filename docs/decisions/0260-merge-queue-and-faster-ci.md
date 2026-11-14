# 0260: Fast pull request check, full suite after merge (merge queue not available)

Date: 2026-11-13. Status: accepted.

## Context

Every pull request had to be current with main, so each merge forced the next one to rebase and
wait for CI again. Measured on recent runs: the Linux test job took about 25 minutes of work and
the Windows one about 42 (the debug build was 8 to 14 minutes of that), yet a run lasted over an
hour end to end because jobs waited for runners. CI ran twice per pull request (the `push` and
the `pull_request` trigger both fired on a pushed branch), and every new push to a pull request
kept the older run going.

## Decision

- Triggers: `pull_request`, `merge_group` (kept so a queue can be turned on later) and `push` to
  `main` only. A pushed branch no longer runs a second copy.
- Concurrency: a newer push to a pull request cancels the older run. Main runs are never
  cancelled.
- A pull request runs the fast check: format, build and tests, clippy, network, lean builds and
  the drift hash, on Linux only.
- Main (and a merge queue, if one is ever on) runs everything: Windows, stability, timing
  budgets, valgrind and the simulated headset as well.
- One job, `CI gate`, is the check to require. It fails if any job it needs failed or was
  cancelled, and passes when a job was skipped on purpose.
- Dropped the separate `cargo build` step and made the Rust cache save only on main.
- A ruleset on `main` requires a pull request and the `CI gate` check, not an up-to-date branch,
  so merging one pull request does not force the others to rebase. Repository auto-merge is on.
- GitHub merge queue was tried and is not offered here: it needs an organization-owned
  repository and this one is under a personal account (owner chose to leave it, 2026-11-14).

## Consequences

- The wait per pull request is the Linux job; the rebase loop is gone.
- Windows, stability, budget, valgrind and headset failures show on main after the merge. The
  thread that merged fixes a red main first.
- Moving the repository to an organization would allow a queue: add the `merge_queue` rule to the
  ruleset and the full suite then runs before merge.
