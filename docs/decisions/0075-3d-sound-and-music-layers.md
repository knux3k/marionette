# 0075: 3D sound that follows the head, and layered music

Date: 2026-11-13. Status: accepted.

## Context

Audio computed a gain (`1 / (1 + distance)`) and a stereo pan once, when a sound started.
Moving sources and turning heads did not change what was heard, looping sounds never
stopped, front and back sounded the same, and there was no music beyond one looping file. The
owner's games are VR, where sound must stay put in the world while the head turns. Existing
scenes and their logged gains must not change.

## Decision

- **One formula, `hear`** (`crates/marionette-audio/src/lib.rs`): gain, pan, low-pass cutoff
  and pitch of a source for a listener. Used by the tick (the `sound` event in the log) and
  by the windowed player every frame. Without a `SoundSpatial` component it is the old rule
  exactly, so old scenes log the same numbers.
- **`SoundSpatial`** (optional, on the source): `min_distance` (full gain, and full stereo
  width, from here), `max_distance`, `rolloff` (`Inverse`, `Linear`, `Constant`),
  `rear_muffle` (low-pass down to 4 kHz straight behind the listener: stereo panning cannot
  tell front from back, a darker sound can) and `doppler` (pitch from relative velocity,
  player only, clamped 0.5 to 2).
- **The player keeps voices** (at most 48, decision 0032's bounded rule): each started sound
  with an entity is re-heard every frame from the current listener pose, with a 30 ms glide.
  Sources with `rear_muffle` get their own kira sub-track with a low-pass filter. Looping
  voices stop when their entity or `AudioSource` is gone; one-shots finish.
- **Head tracking**: `Player::set_head(pose)` replaces the `AudioListener` entity's pose with
  a tracked head pose every frame (a headset module calls it). Without it the listener entity
  is read every frame, which is enough when the camera entity follows the head.
- **Music is state, not playback**: component `Music { layers: [{ sound, at, mix, level }],
  intensity, volume, fade, playing }`. Levels move on the tick toward 1 when `intensity >= at`
  (and `playing`), else 0, so they are saved, hashed and replayed; `music_layer` events log
  layers coming in and going out. The player starts all stems on one kira clock tick (sample
  aligned), loops them and follows the levels. Four built-in stems (`builtin:music_pad`,
  `_bass`, `_drums`, `_lead`; 4 s loops) make examples and tests work without files.
- **Operations**: `audio_music` steers a `Music`; `audio_mix` reports what the listener hears
  from every source and music layer as numbers, because an agent cannot listen.

## Alternatives

- kira's own spatial tracks and listener: they pan and attenuate, but have no rear filter or
  Doppler, and their numbers are not available headless. Our formula is testable and shared
  with the log.
- HRTF (binaural) rendering: the best localisation in headsets, but needs an HRTF data set
  and a convolution per voice (a new dependency and CPU per voice). The `hear` interface can
  grow into it later; noted as open.
- Horizontal re-sequencing (switching sections on bar lines): useful, but vertical layers
  cover the common "more intense, more instruments" case with far less machinery.

## Consequences

- Old scenes, their hashes and their logged gains are unchanged (`line6_proofs` pins hold).
- Windowed playback, glides, the rear filter and Doppler have never played through a real
  audio device or a headset (UNVERIFIED in STATE.md); only the numbers are tested.
- Open: HRTF, occlusion by geometry, reverb zones, beat-synced transitions between pieces.
