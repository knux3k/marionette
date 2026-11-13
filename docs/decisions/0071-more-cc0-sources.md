# 0071: More CC0 sources for the pool: KayKit and cgbookcase; ShareTextures refused

Status: accepted (2026-11-13)

## Context

The pool (decision 0041) knew four CC0 libraries: Poly Haven, ambientCG, Kenney and Quaternius.
The owner asked for more default sources. Candidates were KayKit, ShareTextures, cgbookcase and
Smithsonian Open Access, with OpenGameArt, Freesound, Sketchfab and itch.io packs as opt-in
sources whose licenses vary per item. Each candidate's terms were read on its own site on
2026-11-13 before deciding.

## Decision

- **KayKit** becomes a library (`kaykit`). Its itch.io pages say "Free for personal and
  commercial use, no attribution required. (CC0 Licensed)", and each pack's official GitHub
  mirror (`github.com/KayKit-Game-Assets/<pack>`) carries a LICENSE.txt with "License: (Creative
  Commons Zero, CC0)". Assets must carry that LICENSE.txt (like Kenney and Quaternius archives).
- **cgbookcase** becomes a library (`cgbookcase`): "The textures are published under the CC0 1.0
  license" (its textures page). Pages and downloads must be on `cgbookcase.com` or its CDN
  `cgbookcase.b-cdn.net`.
- Libraries on a shared host get **URL prefixes** (`Library::prefixes`): KayKit owns only
  `https://github.com/KayKit-Game-Assets/` and `https://raw.githubusercontent.com/KayKit-Game-Assets/`,
  never all of GitHub. `pool_verify` uses the same check.
- **ShareTextures is refused.** Its license page calls it a "Custom CC0 ... license with
  specific restrictions", including "No asset redistribution on other websites ... without our
  written permission" and "No automated downloads". A public pool redistributes, so it cannot
  take these assets.
- **Smithsonian Open Access is not added yet.** Only items marked CC0 are CC0, the marking is a
  per-item API field, and the API needs a key; its documentation could not be read from the
  build machine. It needs its own adapter that checks the per-item field before it can be a
  source.
- **Opt-in sources** (OpenGameArt, Freesound, Sketchfab, itch.io packs) are not added in this
  change. Each needs a per-item license check from a machine-readable field; the admission rules
  already refuse anything without CC0 proof.

## Consequences

- `pool_add` and `pool_ingest` accept KayKit and cgbookcase assets with their proof; `pool_info`
  lists six libraries.
- Focus check (FOCUSES.md): data-only change in `marionette-pool`; no effect on performance or
  graphics; no new always-read docs.
