# 0077: Crash-safe file replace for any file (`marionette_host::write_replacing`)

Date: 2026-11-15. Status: accepted.

## Context

`SaveStore::write` (decision 0076) already replaced `.msave` files crash-safely, but only
inside a slot folder and only for the engine's own format. Games that keep their own save
format (a port that must stay readable by the original game, a settings file, a level a
tool writes) had no engine way to replace a file without risking a half-written one after a
crash, and would each have written their own temp-and-rename code.

## Decision

- `marionette_host::write_replacing(target, bytes, backup: Option<&Path>)`: write
  `.<name>.<pid>.tmp` beside the target and flush it, rename the old target to `backup` when
  one is given, rename the temp file onto the target, flush the folder on Unix. The temp file
  is removed when any step fails. The caller picks the backup name, so a game can keep the
  naming its original reader falls back to (for example `<file>.bak`).
- It lives in `marionette-host` (what the engine holds from the OS, always built), not in the
  optional `marionette-save`, so games built without the `save` feature can use it.
- `SaveStore::write` now calls it (one code path); its temp files are named
  `.<slot>.msave.<pid>.tmp`, which the store's stale-temp sweep already matches.
- `marionette_host::sync_dir` is public for callers that rename files themselves.

## Consequences

- One tested implementation of the crash-safety rule. Between the two renames only the backup
  exists; readers must fall back to it, as `SaveStore::read` does.
- No change to `.msave` contents or the drift hash.
