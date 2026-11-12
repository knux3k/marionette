//! Image comparison for reference-image tests.

/// Result of comparing two RGBA8 images of the same size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Diff {
    /// Largest per-channel difference (0..255).
    pub max: u8,
    /// Mean absolute per-channel difference.
    pub mean: f64,
    /// Fraction of pixels where any channel differs by more than the threshold.
    pub over_threshold: f64,
}

/// Per-channel difference above which a pixel counts as different.
pub const REFERENCE_THRESHOLD: u8 = 24;

/// Compares two RGBA8 buffers. Panics if their lengths differ.
pub fn diff(a: &[u8], b: &[u8], threshold: u8) -> Diff {
    assert_eq!(a.len(), b.len(), "images differ in size");
    let mut max = 0u8;
    let mut sum = 0u64;
    let mut over = 0usize;
    for (pa, pb) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0.iter()) {
        let mut pixel_max = 0u8;
        for (x, y) in pa.iter().zip(pb) {
            let d = x.abs_diff(*y);
            sum += d as u64;
            pixel_max = pixel_max.max(d);
        }
        max = max.max(pixel_max);
        if pixel_max > threshold {
            over += 1;
        }
    }
    let pixels = (a.len() / 4).max(1);
    Diff {
        max,
        mean: sum as f64 / a.len().max(1) as f64,
        over_threshold: over as f64 / pixels as f64,
    }
}

/// Reference-image tolerance (see docs/decisions/0006-reference-images.md): at most 1% of
/// pixels may differ by more than [`REFERENCE_THRESHOLD`] in any channel, and the mean
/// difference must be under 2/255.
pub fn within_reference_tolerance(d: &Diff) -> bool {
    d.over_threshold <= 0.01 && d.mean <= 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_have_zero_diff() {
        let a = vec![10u8; 400];
        assert_eq!(
            diff(&a, &a, 0),
            Diff {
                max: 0,
                mean: 0.0,
                over_threshold: 0.0
            }
        );
    }

    #[test]
    fn counts_pixels_over_threshold() {
        let a = vec![0u8; 16];
        let mut b = a.clone();
        b[0] = 100; // one channel of one of four pixels
        let d = diff(&a, &b, REFERENCE_THRESHOLD);
        assert_eq!(d.max, 100);
        assert_eq!(d.over_threshold, 0.25);
    }
}
