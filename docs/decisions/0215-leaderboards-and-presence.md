# 0215: Leaderboards and rich presence behind interfaces (`marionette-platform`)

Date: 2026-11-16. Status: accepted.

## Context

Games post weekly or per-level scores and show friends what a player is doing. Stores answer
leaderboard calls later over the network and can fail or not know a board; the game must then
keep its own local best. Like achievements (0213), the same game code must run without a store.

## Decision

- Trait `Leaderboards`: `find(board, create)` (create takes a `SortOrder` and a
  `ScoreDisplay`), `upload(board, score, keep_best, details)`, `download(board, RowRange)`
  (`Global`, `AroundPlayer`, `Friends`), and `poll()` returning `BoardEvent`s (`Found`,
  `Uploaded` with `changed` and `rank`, `Rows`). Every request gets exactly one answer, also
  on failure. Names under 128 bytes (`MAX_BOARD_NAME`), at most 64 detail integers
  (`MAX_DETAILS`): the strictest common store limits.
- Trait `Presence`: `set_presence(key, value)`, `clear_presence()`; keys and values up to 256
  bytes.
- `OfflinePlatform` implements both: boards in `<dir>/leaderboards.json` with the player as the
  only row-holder (`set_player`), presence in memory (readable for tests).

## Alternatives

- **Callbacks per request**: closures held across frames need boxed trait objects and make
  ownership harder to follow; numbered requests and a polled event list are plain data.
- **Blocking calls**: a store round trip in a frame stalls it. Rejected.

## Consequences

- The Steam backend implements both on the flat API (a separate change: it adds `unsafe` C
  calls, which need the owner's review first).
- Games keep their local best whenever an answer says `found: false` or `ok: false`.
