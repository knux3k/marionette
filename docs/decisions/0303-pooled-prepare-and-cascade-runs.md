# 0303: Instance preparation on a thread pool; cascades reuse unbroken runs

Date: 2026-11-15. Status: accepted. Second optimization block (0300-0309), focus 3.

## Context

After 0302 the realistic benchmark's frame on the reference PC is bound by recording (about
5 ms) again. Timed in the cloud: preparing mesh instances 1.7 ms, merging the chunks 0.5 ms,
copying instances into upload memory 1.2 ms (55,000 instances of 112 bytes), shadow map copies
0.25 ms, encoding 0.2 ms. The 8 preparation chunks took about 0.3 ms each, but the step took
1.6 to 2 ms: starting 7 threads every frame cost more than the work on them.

## Decision

- Chunks are prepared with rayon (already in the workspace for jobs) on its long-lived pool,
  16 chunks of at least 128 draws. Results are merged in draw order as before (0300).
- A sun map whose casters in a draw are one unbroken run of that draw's instances draws the
  run where it already is in the instance buffer; only broken runs are still copied after the
  main instances (0102). The far cascade usually holds every caster of a draw, so about a
  quarter of the uploaded instances go (55,400 to 40,850 on the benchmark).

The instances each pass draws, and their order, are unchanged.

## Proof

Realistic scene, cloud container, lavapipe, three alternated runs of 20 frames: median record
5.72 / 5.35 / 5.82 ms before, 4.56 / 4.72 / 4.65 ms after (about 17 percent less). A 1280x720
screenshot is byte-identical; render tests and the drift hash are unchanged.

## Rejected

- Gathering the upload on several threads: `WriteOnly` views are not `Send`, and splitting the
  write into several views was too noisy to prove in the cloud (tried during 0300).
- Reordering a draw's visible instances so every cascade's casters form one run: changes the
  order of equal-depth draws in the lit pass, which can change pixels.
