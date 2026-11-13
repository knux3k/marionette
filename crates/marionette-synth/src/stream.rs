//! Music and ambience streamed from disk. A reader thread decodes a WAV or OGG file in small
//! chunks, converts it to stereo at the synth's rate and pushes it into a fixed-size lock-free
//! ring; [`StreamSource`] (a [`Generator`]) pops from the ring on the audio thread. The whole
//! file is never in memory: the ring holds about a second, whatever the file's length.
//!
//! If the reader falls behind, the audio thread plays silence for the missing frames and counts
//! an underrun in [`Stats`] instead of waiting. [`StreamSettings::blocking`] flips that for
//! offline renders, so a test is repeatable whatever the thread timing.

use crate::engine::{Generator, Stats};
use rtrb::{Consumer, RingBuffer};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::thread::JoinHandle;
use std::time::Duration;
use symphonia::core::{
    audio::{Audio, GenericAudioBufferRef, conv::IntoSample, sample::Sample},
    codecs::{CodecParameters, audio::AudioDecoder},
    formats::{FormatReader, TrackType, probe::Hint},
    io::MediaSourceStream,
};

/// Ring size in frames (stereo): about 1.4 s at 48 kHz. Fixed, so memory is too.
const RING_FRAMES: usize = 65_536;
/// The reader refills when at least this many frames are free.
const REFILL: usize = 4096;

/// How a stream plays.
#[derive(Debug, Clone, Copy)]
pub struct StreamSettings {
    pub looping: bool,
    pub gain: f32,
    /// Synth frame to start sounding at (sample accurate; 0 = as soon as there is data).
    pub start_at: u64,
    /// Offline renders only: wait for data instead of playing silence on an underrun.
    pub blocking: bool,
}

impl Default for StreamSettings {
    fn default() -> Self {
        Self {
            looping: false,
            gain: 1.0,
            start_at: 0,
            blocking: false,
        }
    }
}

struct Shared {
    stop: AtomicBool,
    /// Reader has pushed the last frame.
    ended: AtomicBool,
    playing: AtomicBool,
    gain: AtomicU32,
    underruns: AtomicU64,
    /// Frames played so far.
    played: AtomicU64,
    /// Reader error text, if it failed after opening.
    failed: std::sync::Mutex<Option<String>>,
}

/// Game-thread control of one stream. Dropping it stops and joins the reader thread.
pub struct StreamHandle {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl StreamHandle {
    pub fn set_gain(&self, gain: f32) {
        self.shared.gain.store(gain.max(0.0).to_bits(), Relaxed);
    }
    /// Pauses or resumes (a paused stream keeps its place and its ring).
    pub fn set_playing(&self, on: bool) {
        self.shared.playing.store(on, Relaxed);
    }
    /// Frames of audio this stream has played.
    pub fn played(&self) -> u64 {
        self.shared.played.load(Relaxed)
    }
    /// Times the audio thread found the ring empty.
    pub fn underruns(&self) -> u64 {
        self.shared.underruns.load(Relaxed)
    }
    /// Reader error after opening (file vanished, corrupt data), if any.
    pub fn error(&self) -> Option<String> {
        self.shared.failed.lock().ok().and_then(|g| g.clone())
    }
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.shared.stop.store(true, Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Decodes a file into interleaved stereo f32 chunks at the file's own rate.
struct FileDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track: u32,
    pub rate: u32,
}

impl FileDecoder {
    fn open(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let format = symphonia::default::get_probe()
            .probe(&hint, mss, Default::default(), Default::default())
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let track = format
            .default_track(TrackType::Audio)
            .ok_or_else(|| format!("{} has no audio track", path.display()))?;
        let params = match track.codec_params.as_ref() {
            Some(CodecParameters::Audio(p)) => p,
            _ => return Err(format!("{} has no audio track", path.display())),
        };
        let rate = params
            .sample_rate
            .ok_or_else(|| format!("{} has no sample rate", path.display()))?;
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &Default::default())
            .map_err(|e| format!("cannot decode {}: {e}", path.display()))?;
        let track = track.id;
        Ok(Self {
            format,
            decoder,
            track,
            rate,
        })
    }

    /// Next packet as stereo frames pushed onto `out`; false at the end of the file.
    fn next(&mut self, out: &mut Vec<[f32; 2]>) -> Result<bool, String> {
        loop {
            let packet = match self.format.next_packet().map_err(|e| e.to_string())? {
                Some(p) if p.track_id == self.track => p,
                Some(_) => continue,
                None => return Ok(false),
            };
            let buf = self.decoder.decode(&packet).map_err(|e| e.to_string())?;
            match &buf {
                GenericAudioBufferRef::U8(b) => append(b, out),
                GenericAudioBufferRef::U16(b) => append(b, out),
                GenericAudioBufferRef::U24(b) => append(b, out),
                GenericAudioBufferRef::U32(b) => append(b, out),
                GenericAudioBufferRef::S8(b) => append(b, out),
                GenericAudioBufferRef::S16(b) => append(b, out),
                GenericAudioBufferRef::S24(b) => append(b, out),
                GenericAudioBufferRef::S32(b) => append(b, out),
                GenericAudioBufferRef::F32(b) => append(b, out),
                GenericAudioBufferRef::F64(b) => append(b, out),
            }
            return Ok(true);
        }
    }
}

fn append<S: Sample + IntoSample<f32> + Copy>(
    b: &symphonia::core::audio::AudioBuffer<S>,
    out: &mut Vec<[f32; 2]>,
) {
    let l = b.plane(0).unwrap_or(&[]);
    let r = b.plane(1).unwrap_or(l);
    for (a, c) in l.iter().zip(r.iter()) {
        out.push([(*a).into_sample(), (*c).into_sample()]);
    }
}

/// Audio-thread side: a [`Generator`] reading the ring.
pub struct StreamSource {
    rx: Consumer<[f32; 2]>,
    shared: Arc<Shared>,
    stats: Arc<Stats>,
    start_at: u64,
    blocking: bool,
    gain_now: f32,
}

/// Opens `path` (WAV or OGG) for streaming at the synth's sample rate. Fails if the file cannot
/// be opened or decoded at all; later read errors are reported by [`StreamHandle::error`]
/// and end the stream. Install the source with `SynthHandle::install`.
pub fn open_stream(
    path: &Path,
    stats: &Arc<Stats>,
    settings: StreamSettings,
) -> Result<(StreamSource, StreamHandle), String> {
    let mut decoder = FileDecoder::open(path)?;
    let (mut tx, rx) = RingBuffer::<[f32; 2]>::new(RING_FRAMES);
    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        ended: AtomicBool::new(false),
        playing: AtomicBool::new(true),
        gain: AtomicU32::new(settings.gain.max(0.0).to_bits()),
        underruns: AtomicU64::new(0),
        played: AtomicU64::new(0),
        failed: std::sync::Mutex::new(None),
    });
    let reader_shared = shared.clone();
    let reader_stats = stats.clone();
    let path = path.to_path_buf();
    let looping = settings.looping;
    let thread = std::thread::Builder::new()
        .name("marionette-stream".into())
        .spawn(move || {
            let mut chunk: Vec<[f32; 2]> = Vec::with_capacity(8192);
            let mut resampler = Resampler::default();
            let mut converted: Vec<[f32; 2]> = Vec::with_capacity(16_384);
            loop {
                if reader_shared.stop.load(Relaxed) {
                    return;
                }
                if tx.slots() < REFILL {
                    std::thread::sleep(Duration::from_millis(4));
                    continue;
                }
                chunk.clear();
                match decoder.next(&mut chunk) {
                    Ok(true) => {
                        let target = reader_stats.rate().max(8000);
                        resampler.run(&chunk, decoder.rate, target, &mut converted);
                        for f in converted.drain(..) {
                            // Wait for room rather than drop audio; stop is checked each lap.
                            let mut f = f;
                            while let Err(rtrb::PushError::Full(back)) = tx.push(f) {
                                f = back;
                                if reader_shared.stop.load(Relaxed) {
                                    return;
                                }
                                std::thread::sleep(Duration::from_millis(2));
                            }
                        }
                    }
                    Ok(false) => {
                        if looping {
                            match FileDecoder::open(&path) {
                                Ok(d) => decoder = d,
                                Err(e) => {
                                    set_failed(&reader_shared, e);
                                    reader_shared.ended.store(true, Relaxed);
                                    return;
                                }
                            }
                        } else {
                            reader_shared.ended.store(true, Relaxed);
                            return;
                        }
                    }
                    Err(e) => {
                        set_failed(&reader_shared, e);
                        reader_shared.ended.store(true, Relaxed);
                        return;
                    }
                }
            }
        })
        .map_err(|e| format!("cannot start the stream reader: {e}"))?;
    Ok((
        StreamSource {
            rx,
            shared: shared.clone(),
            stats: stats.clone(),
            start_at: settings.start_at,
            blocking: settings.blocking,
            gain_now: settings.gain.max(0.0),
        },
        StreamHandle {
            shared,
            thread: Some(thread),
        },
    ))
}

fn set_failed(s: &Shared, e: String) {
    if let Ok(mut g) = s.failed.lock() {
        *g = Some(e);
    }
}

/// Linear resampler that keeps its place between chunks (runs on the reader thread, never the
/// audio thread). Files at the synth's rate pass through unchanged.
#[derive(Default)]
struct Resampler {
    pos: f64,
    last: [f32; 2],
}

impl Resampler {
    fn run(&mut self, input: &[[f32; 2]], from: u32, to: u32, out: &mut Vec<[f32; 2]>) {
        if from == to {
            out.extend_from_slice(input);
            return;
        }
        let step = f64::from(from) / f64::from(to);
        // `pos` is relative to `last` (index -1) then input[0..].
        while self.pos < input.len() as f64 {
            let i = self.pos.floor();
            let t = (self.pos - i) as f32;
            let a = if i < 0.0 { self.last } else { input[i as usize] };
            let b = input.get((i + 1.0) as usize).copied().unwrap_or(a);
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            self.pos += step;
        }
        self.pos -= input.len() as f64;
        self.last = input.last().copied().unwrap_or(self.last);
        // The next chunk's first sample is at index 0 and `pos` may sit before it (-1..0).
    }
}

impl Generator for StreamSource {
    fn render(&mut self, out: &mut [[f32; 2]], frame: u64, _rate: f32) {
        let gain_target = f32::from_bits(self.shared.gain.load(Relaxed));
        let playing = self.shared.playing.load(Relaxed);
        let (mut played, mut missed) = (0u64, false);
        for (i, o) in out.iter_mut().enumerate() {
            if frame + (i as u64) < self.start_at || !playing {
                continue;
            }
            let f = loop {
                match self.rx.pop() {
                    Ok(f) => break Some(f),
                    Err(_) => {
                        if self.shared.ended.load(Relaxed) {
                            // The reader may have pushed its last frames before setting `ended`.
                            if let Ok(f) = self.rx.pop() {
                                break Some(f);
                            }
                            break None;
                        }
                        if !self.blocking {
                            missed = true;
                            break None;
                        }
                        std::thread::yield_now();
                    }
                }
            };
            let Some(f) = f else { continue };
            self.gain_now += (gain_target - self.gain_now) * 0.001;
            o[0] += f[0] * self.gain_now;
            o[1] += f[1] * self.gain_now;
            played += 1;
        }
        // One count per block that came up short, not per missing frame.
        if missed {
            self.shared.underruns.fetch_add(1, Relaxed);
            self.stats.underruns.fetch_add(1, Relaxed);
        }
        if played > 0 {
            self.shared.played.fetch_add(played, Relaxed);
        }
    }

    fn finished(&self) -> bool {
        self.shared.ended.load(Relaxed) && self.rx.is_empty()
    }
}
