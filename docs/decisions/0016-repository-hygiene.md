# 0016: No identifying details in the repository

Date: 2026-11-11. Status: accepted (permanent rule).

## Decision

- No tracked file may contain personal names or nicknames, usernames, email addresses, machine
  names, the hardware or software of a particular machine, absolute user paths, names of the
  owner's other projects, or links to private chats or artifacts. Use "the project owner", "the
  engine repository", "the assets repository" and repo-relative paths.
- Performance numbers may name a generic hardware class and settings (for example "desktop GPU
  of the RTX 2080 class, 1920x1080, 4x MSAA").
- Commits use the identity `Marionette Contributors` with the address `noreply` at
  `marionette.invalid`; the existing history was rewritten to it.
- `crates/marionette-cli/tests/repo_hygiene.rs` fails on generic patterns from
  `docs/ai/forbidden-patterns.txt` (absolute user paths, private links) and on email addresses.

## Why

The repository will be public. Generic patterns catch the common leaks without the pattern file
itself listing anyone's details.

## Known gap

Older commits still contain earlier versions of a few files with such details: the identity was
rewritten, but file contents in history were not. Removing them needs a content rewrite of the
history, which needs the owner's approval before the first push.
