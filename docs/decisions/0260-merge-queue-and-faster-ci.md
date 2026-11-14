# 0260: Merge queue, fast pull request check, full suite at the queue

Date: 2026-11-13. Status: accepted.

## Context

Every pull request had to be current with main, so each merge forced the next one to rebase and
wait for CI again. Measured on recent runs: the Linux test job took about 25 minutes of work and
the Windows one about 42 (the debug build was 8 to 14 minutes of that), yet a run lasted over an
hour end to end because jobs waited for runners. CI ran twice per pull request (the `push` and
the `pull_request` trigger both fired on a pushed branch), and every new push to a pull request
kept the older run going.

## Decision

- Triggers: `pull_request`, `merge_group` and `push` to `main` only. A pushed branch no longer
  runs a second copy.
- Concurrency: a newer push to a pull request cancels the older run. Merge queue and main runs
  are never cancelled.
- A pull request runs the fast check: format, build and tests, clippy, network, lean builds and
  the drift hash, on Linux only.
- The merge queue and main run everything: Windows, stability, timing budgets, valgrind and the
  simulated headset as well. Nothing reaches main without the full suite; a failure there ejects
  the pull request from the queue instead of breaking main.
- One job, `CI gate`, is the check to require. It fails if any job it needs failed or was
  cancelled, and passes when a job was skipped on purpose.
- Dropped the separate `cargo build` step (the test step builds the same thing) and made the
  Rust cache save only on main, so pull requests read it without each uploading a new copy.
- The queue is a repository ruleset on `main` (require a pull request, require the `CI gate`
  check, require merge queue; no bypass), turned on 2026-11-14. It is not in the workflow files;
  repository auto-merge is on so a pull request joins the queue with auto-merge.

## Consequences

- The wait per pull request drops to the Linux job, and the 1-hour rebase loop goes away
  because the queue tests each pull request on top of main plus those ahead of it.
- Windows-only breakage is found at the queue, not on the pull request.
- With the queue on, threads merge by adding a pull request to the queue (auto merge), not by a
  direct merge.
