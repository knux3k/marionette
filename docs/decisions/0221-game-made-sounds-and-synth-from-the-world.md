# 0221: Sounds made in game code, synth commands from game systems, pitch per source

Date: 2026-11-16. Status: accepted.

## Context

A game that makes every sound effect and ambience loop in code (sample buffers computed at
load) could not play them: `AudioSource` and `sound` events resolve only built-in sounds and
files. A game's music synth (0220) could only reach the device through `Player::start_synth`,
but in windowed play the runner owns the `Player`; game systems see only the world. Many
games also play one-shots at a random pitch and change a loop's pitch with its state (wind
higher in a storm), and nothing carried a pitch.

## Decision

- **`SoundBank` resource**: named mono `Arc<[f32]>` buffers with a sample rate, played as
  `bank:<name>` anywhere a sound name goes. Headless runs log them like any sound. The
  windowed player converts each new buffer to kira data once (mono to stereo frames) when the
  bank's version moves, and forgets removed names. Bounded at 16M samples in all
  (MEMORY-AND-CRASHES.md); `insert` refuses past it.
- **`SynthOutbox` resource**: synth `Command`s queued by game systems (notes, master gain,
  `Install` of a game `Generator`). The windowed player starts a synth host the first time the
  outbox holds something and forwards every command each frame; it writes back `live`, the
  synth's `rate` and `now` so a game can schedule ahead. Bounded at 512 waiting commands;
  dropped and refused commands are counted. A generator is `Send` but not `Sync`, so the list
  sits in a `Mutex` reached only through `&mut self` (`get_mut`, never locked).
- **`SoundPitch` component**: a playback rate for the source on its entity, logged as
  `pitch` in the `sound` event and followed every frame for loops (times any Doppler).
  A `sound` event's `pitch` is honoured by the player for one-shots. A new optional component
  rather than a field, so existing scenes hash the same (as 0075).
- **`Player::frame(world, dt)`**: bank sync, new events, voice following, music and the outbox
  in one call. Windowed play, the editor and XR call it; `play_new` and `update` stay public.

## Alternatives

- Game sample buffers written as WAV files and played from disk: a disk round trip at every
  start, temp files to clean, and no way to replace a sound while running.
- Games mixing their own loops inside one `Generator`: works, but every game would rewrite
  voice following, spatial hearing and fades that the player already has.
- The runner handing the `Player` to game code: game systems run inside the schedule with the
  world only; a resource keeps the game side the same headless and windowed.

## Consequences

- A game fills a bank once and plays effects through ordinary events and `AudioSource`s;
  streamed or generated music goes through the outbox. Headless tests read the log.
- Converting a bank buffer copies it once on the main thread (about 8 bytes a sample).
- Not done: a filter or reverb on the whole mix (for example under water), and a per-voice
  filter other than `rear_muffle`.
