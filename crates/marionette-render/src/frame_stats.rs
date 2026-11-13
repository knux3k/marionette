//! Frame pacing numbers of a windowed run (decision 0173). Fixed-size histograms, so a run of
//! any length uses the same memory (MEMORY-AND-CRASHES.md: every log bounded).

use serde_json::{Value, json};

/// Bucket width of [`Histogram`] in milliseconds.
pub const BUCKET_MS: f64 = 0.05;
/// Buckets; times at or above `BUCKETS * BUCKET_MS` (200 ms) land in the last one.
pub const BUCKETS: usize = 4000;

/// Times in milliseconds, 0.05 ms resolution up to 200 ms, exact count, mean and max.
#[derive(Clone)]
pub struct Histogram {
    counts: Box<[u32; BUCKETS]>,
    count: u64,
    sum: f64,
    max: f64,
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            counts: Box::new([0; BUCKETS]),
            count: 0,
            sum: 0.0,
            max: 0.0,
        }
    }
}

impl Histogram {
    pub fn add(&mut self, ms: f64) {
        let ms = ms.max(0.0);
        let i = ((ms / BUCKET_MS) as usize).min(BUCKETS - 1);
        self.counts[i] = self.counts[i].saturating_add(1);
        self.count += 1;
        self.sum += ms;
        self.max = self.max.max(ms);
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    /// The upper edge of the bucket holding fraction `q` (0..1) of the samples; 0 when empty.
    pub fn quantile(&self, q: f64) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        let want = ((q * self.count as f64).ceil() as u64).max(1);
        let mut seen = 0u64;
        for (i, c) in self.counts.iter().enumerate() {
            seen += u64::from(*c);
            if seen >= want {
                return ((i + 1) as f64 * BUCKET_MS).min(self.max);
            }
        }
        self.max
    }

    /// How many samples are above `ms`.
    pub fn above(&self, ms: f64) -> u64 {
        let from = ((ms / BUCKET_MS).ceil() as usize).min(BUCKETS);
        self.counts[from..].iter().map(|c| u64::from(*c)).sum()
    }

    /// `{count, mean, p50, p95, p99, max}` in milliseconds, rounded to 0.01.
    pub fn summary(&self) -> Value {
        let r = |x: f64| (x * 100.0).round() / 100.0;
        let mean = if self.count == 0 {
            0.0
        } else {
            self.sum / self.count as f64
        };
        json!({
            "count": self.count,
            "mean_ms": r(mean),
            "p50_ms": r(self.quantile(0.50)),
            "p95_ms": r(self.quantile(0.95)),
            "p99_ms": r(self.quantile(0.99)),
            "max_ms": r(self.max),
        })
    }
}

/// Everything a windowed run measures about its frames.
#[derive(Clone, Default)]
pub struct FrameStats {
    /// Wall time between consecutive presents.
    pub interval: Histogram,
    /// Main-thread work of a frame: simulation ticks, extraction, encoding, submit, present.
    pub cpu: Histogram,
    /// From the OS input event reaching the window to the present call of the first frame
    /// that simulated it. The display adds its queue (about `frame_latency` refreshes with
    /// vsync) on top; that part cannot be seen from inside the process.
    pub input_latency: Histogram,
    /// Frames that ran 0, 1, 2 and 3 or more simulation ticks. With a display refresh equal
    /// to the tick rate, anything outside the "1" column is visible judder.
    pub ticks_per_frame: [u64; 4],
    /// Frames the surface skipped (timeout, occluded, outdated).
    pub skipped: u64,
    pub resizes: u64,
    pub focus_losses: u64,
    /// Display refresh rate the OS reports for the window's monitor, if any.
    pub refresh_hz: Option<f64>,
    /// Present mode actually in use (after falling back from the requested one).
    pub present_mode: String,
    pub frame_latency: u32,
    pub size: (u32, u32),
    pub fullscreen: bool,
    /// Main-thread time of the warm-up frames (`window::WARMUP_FRAMES`), not in the histograms.
    pub warmup_ms: f64,
}

impl FrameStats {
    pub fn ticks(&mut self, n: u32) {
        self.ticks_per_frame[(n as usize).min(3)] += 1;
    }

    /// The numbers as JSON, for the `run` output and `--frame-stats` files.
    pub fn summary(&self) -> Value {
        let frames = self.cpu.count();
        let budget = self.refresh_hz.map(|hz| 1000.0 / hz);
        json!({
            "frames": frames,
            "warmup_ms": (self.warmup_ms * 100.0).round() / 100.0,
            "present_mode": self.present_mode,
            "frame_latency": self.frame_latency,
            "refresh_hz": self.refresh_hz,
            "size": [self.size.0, self.size.1],
            "fullscreen": self.fullscreen,
            "interval": self.interval.summary(),
            "cpu": self.cpu.summary(),
            "input_latency": self.input_latency.summary(),
            // Presents that took longer than 1.5 refreshes: a refresh was shown twice.
            "missed_refreshes": budget.map(|b| self.interval.above(b * 1.5)),
            "ticks_per_frame": {
                "0": self.ticks_per_frame[0],
                "1": self.ticks_per_frame[1],
                "2": self.ticks_per_frame[2],
                "3+": self.ticks_per_frame[3],
            },
            "skipped": self.skipped,
            "resizes": self.resizes,
            "focus_losses": self.focus_losses,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_come_from_bucket_edges() {
        let mut h = Histogram::default();
        for i in 0..100 {
            h.add(16.0 + f64::from(i) * 0.01); // 16.00 .. 16.99
        }
        assert_eq!(h.count(), 100);
        assert!((h.quantile(0.5) - 16.5).abs() < 0.06, "{}", h.quantile(0.5));
        assert!(h.quantile(0.99) <= 16.99 + 1e-9);
        assert_eq!(h.above(17.0), 0);
        h.add(500.0);
        assert_eq!(h.above(17.0), 1);
        assert_eq!(h.summary()["max_ms"], 500.0);
    }

    #[test]
    fn empty_stats_summarize_to_zeros() {
        let s = FrameStats::default();
        let v = s.summary();
        assert_eq!(v["interval"]["p99_ms"], 0.0);
        assert_eq!(v["missed_refreshes"], Value::Null);
    }
}
