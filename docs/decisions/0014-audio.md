# 0014: Audio decided in the simulation, played by the presentation layer

Date: 2026-11-11. Status: accepted.

## Decision

- `AudioSource` (sound, volume, looping, trigger: `OnStart`, `OnCollision`, `OnJump`,
  `OnAction`) and `AudioListener` are components. In `Stage::Effects` the simulation writes a
  `sound` event to `EventLog` with gain `volume * listener / (1 + distance)` and pan (projection
  on the listener's right axis), using only `+ - * / sqrt`.
- Headless runs stop there: the event log is the null backend that tests assert on
  (`observe_events` with `kind: "sound"`).
- In a window, `marionette_audio::Player` (kira 0.12 on cpal) plays new `sound` events.
- Built-in sounds (`builtin:beep`, `builtin:jump`, `builtin:thud`) are generated in code; WAV and
  OGG files are decoded by kira (features `wav`, `pcm`, `ogg`, `vorbis`).

## Why

Keeping playback out of the simulation means no audio device, thread or timing can affect
state, and tests need no sound card.
