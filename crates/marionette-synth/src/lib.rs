//! Marionette synth: procedural and streamed audio that runs without a sound card.
//!
//! - [`create`] gives a [`Synth`] (runs on the audio thread: voices, filters, envelopes,
//!   [`Generator`]s for a game's own sample code) and a [`SynthHandle`] (the game's side). They
//!   talk through a lock-free queue; nothing in `Synth::render` allocates, locks or blocks.
//! - Commands carry the frame they run at, so notes start and stop sample accurately.
//! - [`stream::open_stream`] plays long WAV/OGG files from disk through a fixed ring.
//! - [`score::render_score`] renders a [`Score`] to a buffer with the same code, and
//!   [`analyze`] reports level, clipping, clicks, pitch and spectrum as numbers.
//!
//! Hosting on a real device (kira) lives in `marionette-audio` (decision 0220).

pub mod analyze;
pub mod dsp;
pub mod engine;
pub mod score;
pub mod stream;
mod voice;

pub use analyze::{Report, analyze, hash_frames, wav_stereo_bytes};
pub use engine::{Command, GENERATOR_SLOTS, Generator, Stats, StatsSnapshot, Synth, SynthHandle, create};
pub use score::{PRESETS, Rendered, Score, render_score};
pub use stream::{StreamHandle, StreamSettings, StreamSource, open_stream};
pub use voice::Patch;
