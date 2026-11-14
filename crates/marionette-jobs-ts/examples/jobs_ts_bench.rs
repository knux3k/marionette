//! Benchmark: one particle step (gravity, drag, bouncy floor; `examples/jobs/particles.job.ts`)
//! written three ways, all producing bit-identical results:
//!
//! - a Rust job body on `marionette-jobs`,
//! - the job TypeScript file compiled to WebAssembly (this crate),
//! - the same file interpreted by the script engine (QuickJS-ng), one thread.
//!
//! Run in release: `cargo run --release -p marionette-jobs-ts --example jobs_ts_bench`.
//! Optional arguments: particle count (default 1,000,000) and steps (default 20).

use marionette_jobs::Jobs;
use marionette_jobs_ts::{Arg, JobProgram, Safety};
use std::time::Instant;

const SOURCE: &str = include_str!("../../../examples/jobs/particles.job.ts");
const DT: f64 = 1.0 / 60.0;
const K: f64 = 0.001;
const CHUNK: usize = 16_384;

fn data(n: usize, seed: u32) -> Vec<f32> {
    let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s % 20_000) as f32 / 1000.0 - 10.0
        })
        .collect()
}

type State = [Vec<f32>; 6];

fn state(n: usize) -> State {
    [
        data(n, 1),
        data(n, 2).iter().map(|y| y.abs()).collect(),
        data(n, 3),
        data(n, 4),
        data(n, 5),
        data(n, 6),
    ]
}

/// The Rust job body: same arithmetic as the TypeScript (64-bit math, 32-bit storage).
fn rust_chunk(s: [&mut [f32]; 6]) {
    let [px, py, pz, vx, vy, vz] = s;
    let drag = |v: f64| v - v * v.abs() * K;
    for i in 0..px.len() {
        let x = drag(vx[i] as f64);
        let mut y = drag(vy[i] as f64 + -9.81 * DT);
        let z = drag(vz[i] as f64);
        let mut h = py[i] as f64 + y * DT;
        if h < 0.0 {
            h = -h;
            y = -y * 0.8;
        }
        px[i] = (px[i] as f64 + x * DT) as f32;
        py[i] = h as f32;
        pz[i] = (pz[i] as f64 + z * DT) as f32;
        vx[i] = x as f32;
        vy[i] = y as f32;
        vz[i] = z as f32;
    }
}

fn rust_step(jobs: &Jobs, s: &mut State) {
    let n = s[0].len();
    // One lock per chunk holding that chunk's six slices: each call takes its own, so the
    // compiler can see every element has one writer (the same scheme as the job runtime).
    let mut columns = s.iter_mut().map(|v| v.chunks_mut(CHUNK));
    let mut chunks: Vec<std::sync::Mutex<Option<[&mut [f32]; 6]>>> = Vec::new();
    if let [Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)] = [(); 6].map(|_| columns.next()) {
        for (((((a, b), c), d), e), f) in a.zip(b).zip(c).zip(d).zip(e).zip(f) {
            chunks.push(std::sync::Mutex::new(Some([a, b, c, d, e, f])));
        }
    }
    jobs.map_chunks(n, CHUNK, |r| {
        let taken = chunks[r.start / CHUNK].lock().ok().and_then(|mut g| g.take());
        if let Some(slices) = taken {
            rust_chunk(slices);
        }
    });
}

fn ts_step(program: &JobProgram, jobs: &Jobs, s: &mut State) {
    let n = s[0].len();
    let [px, py, pz, vx, vy, vz] = s;
    program
        .run(
            jobs,
            "step",
            n,
            CHUNK,
            &mut [
                Arg::F32(px),
                Arg::F32(py),
                Arg::F32(pz),
                Arg::F32(vx),
                Arg::F32(vy),
                Arg::F32(vz),
                Arg::Number(DT),
                Arg::Number(K),
            ],
        )
        .expect("job runs");
}

fn interpreted(n: usize, steps: usize) -> (f64, Vec<Vec<f64>>) {
    let js = marionette_script::strip::strip(SOURCE, "particles.job.ts")
        .expect("strips")
        .replace("export function", "function");
    let rt = rquickjs::Runtime::new().expect("runtime");
    let ctx = rquickjs::Context::full(&rt).expect("context");
    let input: Vec<Vec<f64>> = state(n)
        .iter()
        .map(|v| v.iter().map(|&x| x as f64).collect())
        .collect();
    ctx.with(|ctx| {
        ctx.eval::<(), _>(js).expect("loads");
        ctx.eval::<(), _>(format!(
            "function prepare(inp) {{ globalThis.S = inp.map((x) => Float32Array.from(x)); }}
             function run() {{ const a = S; for (let r = 0; r < {steps}; r++) step(a[0], a[1], a[2], a[3], a[4], a[5], {DT}, {K}); }}
             function result() {{ return S.map((x) => Array.from(x)); }}"
        ))
        .expect("driver");
        let g = ctx.globals();
        let prepare: rquickjs::Function = g.get("prepare").expect("prepare");
        prepare.call::<_, ()>((input,)).expect("prepare runs");
        let run: rquickjs::Function = g.get("run").expect("run");
        let t = Instant::now();
        run.call::<_, ()>(()).expect("runs");
        let secs = t.elapsed().as_secs_f64();
        let result: rquickjs::Function = g.get("result").expect("result");
        (secs, result.call(()).expect("result"))
    })
}

/// Seconds for `steps` steps, from the fastest single step (the machine may be shared, so
/// the minimum is the most repeatable number).
fn time(steps: usize, mut f: impl FnMut()) -> f64 {
    f(); // warm up (instances, pages, caches)
    let mut best = f64::MAX;
    for _ in 0..steps {
        let t = Instant::now();
        f();
        best = best.min(t.elapsed().as_secs_f64());
    }
    best * steps as f64
}

fn main() {
    let args: Vec<usize> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let n = args.first().copied().unwrap_or(1_000_000);
    let steps = args.get(1).copied().unwrap_or(20);
    let cores = std::thread::available_parallelism().map_or(1, |c| c.get());

    let t = Instant::now();
    let program = JobProgram::compile(SOURCE, "particles.job.ts").expect("compiles");
    let compile_ms = t.elapsed().as_secs_f64() * 1e3;
    let unchecked =
        JobProgram::compile_with(SOURCE, "particles.job.ts", Safety::Unchecked).expect("compiles");

    // Correctness first: the three versions agree bit for bit.
    let check_n = 20_000;
    let mut a = state(check_n);
    let mut b = state(check_n);
    let mut u = state(check_n);
    for _ in 0..3 {
        rust_step(&Jobs::inline(), &mut a);
        ts_step(&program, &Jobs::with_threads(cores), &mut b);
        ts_step(&unchecked, &Jobs::with_threads(cores), &mut u);
    }
    assert_eq!(a, b, "Rust and compiled TypeScript differ");
    assert_eq!(a, u, "Rust and unchecked compiled TypeScript differ");
    let (_, c) = interpreted(check_n, 3);
    let c: Vec<Vec<f32>> = c.iter().map(|v| v.iter().map(|&x| x as f32).collect()).collect();
    assert_eq!(a.to_vec(), c, "interpreted TypeScript differs");

    let per = |secs: f64, items: usize, steps: usize| secs * 1e9 / (items * steps) as f64;
    let mut rows: Vec<(String, f64)> = Vec::new();
    let one = Jobs::inline();
    let many = Jobs::with_threads(cores);
    let mut s = state(n);
    rows.push((
        "Rust job body, 1 thread".into(),
        per(time(steps, || rust_step(&one, &mut s)), n, steps),
    ));
    let mut s = state(n);
    rows.push((
        format!("Rust job body, {cores} threads"),
        per(time(steps, || rust_step(&many, &mut s)), n, steps),
    ));
    let mut s = state(n);
    rows.push((
        "Job TypeScript (Wasm), 1 thread".into(),
        per(time(steps, || ts_step(&program, &one, &mut s)), n, steps),
    ));
    let mut s = state(n);
    rows.push((
        format!("Job TypeScript (Wasm), {cores} threads"),
        per(time(steps, || ts_step(&program, &many, &mut s)), n, steps),
    ));
    let mut s = state(n);
    rows.push((
        "Job TypeScript, unchecked, 1 thread".into(),
        per(time(steps, || ts_step(&unchecked, &one, &mut s)), n, steps),
    ));
    let mut s = state(n);
    rows.push((
        format!("Job TypeScript, unchecked, {cores} threads"),
        per(time(steps, || ts_step(&unchecked, &many, &mut s)), n, steps),
    ));
    let interp_n = (n / 10).max(1000);
    let interp_steps = (steps / 4).max(1);
    let (secs, _) = interpreted(interp_n, interp_steps);
    rows.push((
        "Script interpreter (QuickJS-ng), 1 thread".into(),
        per(secs, interp_n, interp_steps),
    ));

    println!(
        "particle step, {n} particles x {steps} steps, chunk {CHUNK}; results bit-identical across all versions"
    );
    println!("compile time of the job file: {compile_ms:.1} ms");
    let base = rows[0].1;
    for (name, ns) in &rows {
        println!(
            "{name:<44} {ns:>8.2} ns/particle   {:>7.2}x the 1-thread Rust time",
            ns / base
        );
    }
}
