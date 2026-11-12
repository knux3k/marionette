//! Jobs give identical results for any thread count.

use marionette_jobs::Jobs;

fn work(jobs: &Jobs) -> (Vec<f32>, f32, Vec<u32>) {
    let mut v: Vec<f32> = (0..100_003).map(|i| (i as f32) * 0.001).collect();
    jobs.for_each_chunk_mut(&mut v, 1000, |c, chunk| {
        for (k, x) in chunk.iter_mut().enumerate() {
            *x = *x * 1.0001 + (c * 1000 + k) as f32 * 1e-7;
        }
    });
    // Ordered reduction: per-chunk partial sums, folded in chunk order.
    let partial = jobs.map_chunks(v.len(), 4096, |r| v[r].iter().sum::<f32>());
    let total = partial.iter().fold(0.0f32, |a, b| a + b);
    // Uneven segments.
    let mut seg: Vec<u32> = vec![1; 1000];
    let ends = [3, 3, 250, 251, 999, 1000];
    jobs.for_each_segment_mut(&mut seg, &ends, |i, s| {
        for (k, x) in s.iter_mut().enumerate() {
            *x = (i * 10_000 + k) as u32;
        }
    });
    (v, total, seg)
}

#[test]
fn results_do_not_depend_on_thread_count() {
    let one = work(&Jobs::inline());
    for threads in [2, 3, 8] {
        let many = work(&Jobs::with_threads(threads));
        assert!(
            one.0.iter().zip(&many.0).all(|(a, b)| a.to_bits() == b.to_bits()),
            "chunk writes differ with {threads} threads"
        );
        assert_eq!(
            one.1.to_bits(),
            many.1.to_bits(),
            "reduction differs with {threads} threads"
        );
        assert_eq!(one.2, many.2, "segment writes differ with {threads} threads");
    }
}

#[test]
fn two_arrays_are_chunked_together() {
    let mut a: Vec<u32> = (0..10_000).collect();
    let mut b: Vec<u32> = vec![0; 10_000];
    Jobs::with_threads(4).for_each_chunk_mut2(&mut a, &mut b, 128, |_, x, y| {
        for (p, q) in x.iter().zip(y.iter_mut()) {
            *q = p * 2;
        }
    });
    assert!(a.iter().zip(&b).all(|(p, q)| *q == p * 2));
}

#[test]
#[should_panic(expected = "last segment end")]
fn segments_must_cover_the_data() {
    let mut v = vec![0u8; 10];
    Jobs::inline().for_each_segment_mut(&mut v, &[5], |_, _| {});
}
