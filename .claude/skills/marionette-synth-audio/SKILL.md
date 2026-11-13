---
name: marionette-synth-audio
description: Make sound in code and check it without speakers, with synth voices, filters, envelopes, generators, music streamed from disk and offline render. Use when a game needs synthesised or streamed audio, or audio must be verified.
---

# Procedural and streamed audio

The synth runs on the audio thread. The game sends it commands through a lock-free queue and
never blocks it. Nothing in the callback allocates or locks (a test counts allocations).

## Check a sound with numbers first

1. List sounds: `marionette op synth_patches`. Presets: `pluck`, `pad`, `bass`, `lead`,
   `thump`, `wind`, `bell`. A `custom` patch names only the fields it changes.
2. Write a score (JSON, times in seconds): `notes` with `at`, `dur`, `note` (MIDI) or `hz`,
   `velocity`, `patch`; `streams` with `file`, `at`, `gain`, `looping`; `master` gain changes.
3. `marionette op synth_render '{"file":"x.score.json","wav":"out/x.wav","envelope_ms":250}'`
   gives `report` (peak, rms, dc, clipped, max_step, dominant_hz, first_sound_frame), `stats`
   (stolen, dropped, late, underruns) and `hash`. A score is a few lines of JSON.
4. Read it: pitch matches `dominant_hz`; `clipped` is 0; `max_step` far above the signal slope
   is a click; `first_sound_frame` is the exact start; `envelope` shows when it is loud.
   Do not pin `hash` across platforms (math libraries differ); pin tolerances.

## In a game (Rust)

- `marionette_audio::SoundPlayer::start_synth(voices)` plays a synth on the device and returns
  a synth host whose `handle` takes commands: `note_on(at, id, hz, velocity, patch)`,
  `note_off`, `master`, `install(slot, generator)`.
- `at` is a frame of the synth timeline: `handle.now() + handle.frames(0.1)` is 100 ms ahead,
  sample accurate. A command that arrives late runs now and counts in `stats.late`.
- Own sample code: implement `marionette_synth::Generator` (`render` adds into the buffer; no
  allocation, locks or I/O inside) and `install` it. Preallocate in `new`.
- Long music and ambience: `open_stream(path, stats, settings)` then `install` the source.
  Only a ring of about a second is in memory. WAV and OGG; the file may be any rate.

## On the real device

`marionette audio-check --seconds 8 --voices 64` plays a stress run and reports
`slow_blocks` (callbacks slower than the audio) and underruns; exit 0 is clean, 2 means no
device. Run it only on a PC that is idle, and exit what you start.
