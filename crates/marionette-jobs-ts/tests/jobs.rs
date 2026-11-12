//! Job TypeScript: results independent of thread count, a pinned hash (checked on Linux and
//! Windows by CI), bit-equality with the Rust job body and with the script interpreter,
//! compile errors an agent can act on, traps, time limits and hot reload.

use marionette_jobs::Jobs;
use marionette_jobs_ts::{Arg, JobLibrary, JobProgram, codes};
use sha2::{Digest, Sha256};

const PARTICLES: &str = include_str!("../../../examples/jobs/particles.job.ts");
const MIXED: &str = include_str!("jobs/mixed.job.ts");

/// Deterministic test data (no RNG crate needed).
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

struct Particles {
    p: [Vec<f32>; 3],
    v: [Vec<f32>; 3],
}

fn particles(n: usize) -> Particles {
    Particles {
        p: [
            data(n, 1),
            data(n, 2).iter().map(|y| y.abs()).collect(),
            data(n, 3),
        ],
        v: [data(n, 4), data(n, 5), data(n, 6)],
    }
}

fn simulate(program: &JobProgram, jobs: &Jobs, n: usize, steps: usize) -> (Particles, f64) {
    let mut s = particles(n);
    let mut energy = 0.0;
    for _ in 0..steps {
        let [px, py, pz] = &mut s.p;
        let [vx, vy, vz] = &mut s.v;
        program
            .run(
                jobs,
                "step",
                n,
                4096,
                &mut [
                    Arg::F32(px),
                    Arg::F32(py),
                    Arg::F32(pz),
                    Arg::F32(vx),
                    Arg::F32(vy),
                    Arg::F32(vz),
                    Arg::Number(1.0 / 60.0),
                    Arg::Number(0.001),
                ],
            )
            .unwrap();
        energy = program
            .run_sum(
                jobs,
                "energy",
                n,
                4096,
                &mut [Arg::F32In(vx), Arg::F32In(vy), Arg::F32In(vz)],
            )
            .unwrap();
    }
    (s, energy)
}

fn hash(s: &Particles, energy: f64) -> String {
    let mut h = Sha256::new();
    for a in s.p.iter().chain(&s.v) {
        for x in a {
            h.update(x.to_le_bytes());
        }
    }
    h.update(energy.to_le_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The same run gives the same hash on 1, 2, 3 and 8 worker threads. 200,003 particles: more
/// than `PARALLEL_MIN_LEN`, so the pool really splits the work, and not a multiple of the chunk.
#[test]
fn results_do_not_depend_on_thread_count() {
    let program = JobProgram::compile(PARTICLES, "particles.job.ts").unwrap();
    let n = 200_003;
    let (one, e1) = simulate(&program, &Jobs::inline(), n, 5);
    let reference = hash(&one, e1);
    for threads in [2, 3, 8] {
        let (many, e) = simulate(&program, &Jobs::with_threads(threads), n, 5);
        assert_eq!(hash(&many, e), reference, "{threads} threads");
    }
}

/// Pinned: CI runs this on Linux and Windows, so equal hashes there mean equal results across
/// operating systems (the transcendental functions come from `libm`, not the C library).
#[test]
fn pinned_hash() {
    let program = JobProgram::compile(PARTICLES, "particles.job.ts").unwrap();
    let (s, e) = simulate(&program, &Jobs::with_threads(4), 70_001, 20);
    let wave = JobProgram::compile(
        "export function wave(a: Float64Array, t: number): void {\n\
           for (let i = 0; i < a.length; i++) {\n\
             a[i] = Math.sin(a[i] + t) * Math.exp(-Math.abs(a[i]) * 0.1) + Math.atan2(a[i], 3) + Math.pow(Math.abs(a[i]), 0.37) + Math.cbrt(a[i]);\n\
           }\n\
         }\n",
        "wave.job.ts",
    )
    .unwrap();
    let mut a: Vec<f64> = data(10_000, 9).iter().map(|&x| x as f64).collect();
    wave.call("wave", a.len(), &mut [Arg::F64(&mut a), Arg::Number(0.25)])
        .unwrap();
    let mut h = Sha256::new();
    h.update(hash(&s, e).as_bytes());
    for x in &a {
        h.update(x.to_le_bytes());
    }
    let got: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        got,
        "994155611f9d326a5f21b3676f2604afef1d4afb291c268990b8cc6644b73f0a"
    );
}

/// The same particle step written as a Rust job body gives bit-identical results.
#[test]
fn equals_the_rust_job_body() {
    let program = JobProgram::compile(PARTICLES, "particles.job.ts").unwrap();
    let n = 10_007;
    let (ts, _) = simulate(&program, &Jobs::inline(), n, 3);
    let mut s = particles(n);
    for _ in 0..3 {
        rust_step(&mut s, 1.0 / 60.0, 0.001);
    }
    assert_eq!(hash(&ts, 0.0), hash(&s, 0.0));
}

fn rust_step(s: &mut Particles, dt: f64, k: f64) {
    let drag = |v: f64| v - v * v.abs() * k;
    for i in 0..s.p[0].len() {
        let x = drag(s.v[0][i] as f64);
        let mut y = drag(s.v[1][i] as f64 + -9.81 * dt);
        let z = drag(s.v[2][i] as f64);
        let mut h = s.p[1][i] as f64 + y * dt;
        if h < 0.0 {
            h = -h;
            y = -y * 0.8;
        }
        s.p[0][i] = (s.p[0][i] as f64 + x * dt) as f32;
        s.p[1][i] = h as f32;
        s.p[2][i] = (s.p[2][i] as f64 + z * dt) as f32;
        s.v[0][i] = x as f32;
        s.v[1][i] = y as f32;
        s.v[2][i] = z as f32;
    }
}

/// Runs a job file in QuickJS (the script interpreter) chunk by chunk, like the engine does.
fn interpret(source: &str, driver: &str, inputs: Vec<Vec<f64>>) -> Vec<Vec<f64>> {
    let js = marionette_script::strip::strip(source, "job.ts")
        .unwrap()
        .replace("export function", "function");
    let rt = rquickjs::Runtime::new().unwrap();
    let ctx = rquickjs::Context::full(&rt).unwrap();
    ctx.with(|ctx| {
        ctx.eval::<(), _>(js).unwrap();
        ctx.eval::<(), _>(driver).unwrap();
        let f: rquickjs::Function = ctx.globals().get("driver").unwrap();
        f.call((inputs,)).unwrap()
    })
}

#[test]
fn compiled_and_interpreted_runs_are_bit_identical() {
    let n = 5_003;
    let chunk = 1000;
    let a0 = data(n, 11);
    let b0: Vec<f64> = data(n, 12).iter().map(|&x| x as f64 * 7.3).collect();
    let ids0: Vec<i32> = data(n, 13).iter().map(|&x| (x * 1000.0) as i32).collect();

    let program = JobProgram::compile(MIXED, "mixed.job.ts").unwrap();
    let (mut a, mut b, mut ids) = (a0.clone(), b0.clone(), ids0.clone());
    for _ in 0..3 {
        program
            .run(
                &Jobs::with_threads(3),
                "mix",
                n,
                chunk,
                &mut [
                    Arg::F32(&mut a),
                    Arg::F64(&mut b),
                    Arg::I32(&mut ids),
                    Arg::ChunkStart,
                ],
            )
            .unwrap();
    }

    let driver = format!(
        "function driver(inp) {{
           const a = Float32Array.from(inp[0]), b = Float64Array.from(inp[1]), ids = Int32Array.from(inp[2]);
           for (let r = 0; r < 3; r++)
             for (let s = 0; s < a.length; s += {chunk}) {{
               const e = Math.min(a.length, s + {chunk});
               mix(a.subarray(s, e), b.subarray(s, e), ids.subarray(s, e), s);
             }}
           return [Array.from(a), Array.from(b), Array.from(ids)];
         }}"
    );
    let out = interpret(
        MIXED,
        &driver,
        vec![
            a0.iter().map(|&x| x as f64).collect(),
            b0.clone(),
            ids0.iter().map(|&x| x as f64).collect(),
        ],
    );
    let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(
        bits(&out[0]),
        bits(&a.iter().map(|&x| x as f64).collect::<Vec<_>>()),
        "Float32Array"
    );
    assert_eq!(bits(&out[1]), bits(&b), "Float64Array");
    assert_eq!(
        out[2],
        ids.iter().map(|&x| x as f64).collect::<Vec<_>>(),
        "Int32Array"
    );

    // And the particle step.
    let (ts, _) = simulate(
        &JobProgram::compile(PARTICLES, "p.ts").unwrap(),
        &Jobs::inline(),
        3001,
        4,
    );
    let s0 = particles(3001);
    let driver = "function driver(inp) {
        const a = inp.map((x) => Float32Array.from(x));
        for (let r = 0; r < 4; r++) { step(a[0], a[1], a[2], a[3], a[4], a[5], 1 / 60, 0.001); }
        return a.map((x) => Array.from(x));
    }";
    let out = interpret(
        PARTICLES,
        driver,
        s0.p.iter()
            .chain(&s0.v)
            .map(|v| v.iter().map(|&x| x as f64).collect())
            .collect(),
    );
    for (i, arr) in ts.p.iter().chain(&ts.v).enumerate() {
        assert_eq!(
            bits(&out[i]),
            bits(&arr.iter().map(|&x| x as f64).collect::<Vec<_>>()),
            "particle array {i}"
        );
    }
}

#[test]
fn whole_arrays_and_chunk_start() {
    let program = JobProgram::compile(
        "export function look(out: Int32Array, table: Int32Array, start: i32): void {\n\
           for (let i = 0; i < out.length; i++) { out[i] = table[(start + i) % table.length] * 10; }\n\
         }",
        "look.job.ts",
    )
    .unwrap();
    let table: Vec<i32> = (0..7).collect();
    let mut out = vec![0; 100_000];
    program
        .run(
            &Jobs::with_threads(4),
            "look",
            out.len(),
            333,
            &mut [Arg::I32(&mut out), Arg::I32All(&table), Arg::ChunkStart],
        )
        .unwrap();
    assert!(out.iter().enumerate().all(|(i, &v)| v == (i % 7) as i32 * 10));
}

#[test]
fn unannotated_counters_widen_to_number_when_needed() {
    let program = JobProgram::compile(
        "export function avg(a: Float32Array): number {\n\
           let s = 0;\n\
           let n = 0;\n\
           for (let i = 0; i < a.length; i++) { s += a[i]; n++; }\n\
           return n > 0 ? s / n : 0;\n\
         }",
        "avg.job.ts",
    )
    .unwrap();
    let a = [1.5f32, 2.0, 2.5];
    let r = program.call("avg", 3, &mut [Arg::F32In(&a)]).unwrap();
    assert_eq!(r, Some(2.0));
}

fn errors(source: &str) -> Vec<marionette_core::Diagnostic> {
    match JobProgram::compile(source, "bad.job.ts") {
        Ok(_) => panic!("expected errors for:\n{source}"),
        Err(e) => e,
    }
}

/// Code outside the subset fails with a stable code, the exact position and a fix.
#[test]
fn compile_errors_name_the_problem_and_the_fix() {
    let cases: &[(&str, &str, usize, usize)] = &[
        (
            "export function f(a: Float32Array): void {\n  const s = \"hi\";\n}",
            codes::UNSUPPORTED,
            2,
            13,
        ),
        (
            "export function f(a: Float32Array): void {\n  const o = { x: 1 };\n}",
            codes::UNSUPPORTED,
            2,
            13,
        ),
        (
            "export function f(a: Float32Array): void {\n  const g = (x: number) => x;\n}",
            codes::UNSUPPORTED,
            2,
            13,
        ),
        (
            "export function f(a: Float32Array): void {\n  a[0] = Math.random();\n}",
            codes::UNKNOWN_NAME,
            2,
            10,
        ),
        (
            "export function f(a: Float32Array, x: number): void {\n  a[x] = 1;\n}",
            codes::TYPE,
            2,
            5,
        ),
        ("export function f(a): void {}", codes::SIGNATURE, 1, 19),
        ("export function f(a: string): void {}", codes::SIGNATURE, 1, 19),
        (
            "export function f(a: Float32Array): void {\n  a[0] = q;\n}",
            codes::UNKNOWN_NAME,
            2,
            10,
        ),
        (
            "export function f(a: Float32Array): void {\n  if (a.length) { a[0] = 1; }\n}",
            codes::TYPE,
            2,
            7,
        ),
        (
            "export function f(a: Float32Array): void {\n  let i: i32 = 0;\n  i = a[0];\n}",
            codes::TYPE,
            3,
            7,
        ),
        (
            "let counter = 0;\nexport function f(a: Float32Array): void {}",
            codes::TOP_LEVEL,
            1,
            1,
        ),
        (
            "import { x } from \"./other\";\nexport function f(a: Float32Array): void {}",
            codes::TOP_LEVEL,
            1,
            1,
        ),
        (
            "class A {}\nexport function f(a: Float32Array): void {}",
            codes::UNSUPPORTED,
            1,
            1,
        ),
        (
            "export function f(a: Float32Array): number {\n  return;\n}",
            codes::TYPE,
            2,
            3,
        ),
        (
            "export function f(a: Float32Array): void {\n  for (const x of a) {}\n}",
            codes::UNSUPPORTED,
            2,
            3,
        ),
        (
            "export function f(a: Float32Array): void {\n  let x = 1\n  x = \n}",
            codes::SYNTAX,
            4,
            1,
        ),
    ];
    for (source, code, line, column) in cases {
        let e = errors(source);
        let d = &e[0];
        assert_eq!(
            (d.code, d.line, d.column),
            (*code, Some(*line), Some(*column)),
            "{source}\n{d:?}"
        );
        assert_eq!(d.file.as_deref(), Some("bad.job.ts"));
        if *code != codes::SYNTAX {
            assert!(
                d.fix.as_deref().is_some_and(|f| !f.is_empty()),
                "no fix for {source}: {d:?}"
            );
        }
    }
    // Every function is checked: two broken functions give two errors.
    let e = errors(
        "export function f(a: Float32Array): void { a[0] = \"x\"; }\nexport function g(a: Float32Array): void { a[0] = null; }",
    );
    assert_eq!(e.len(), 2);
}

#[test]
fn out_of_range_index_stops_with_the_line() {
    let program = JobProgram::compile(
        "export function f(a: Float32Array): void {\n  for (let i = 0; i <= a.length; i++) {\n    a[i] = 1;\n  }\n}",
        "oob.job.ts",
    )
    .unwrap();
    let mut a = vec![0f32; 10];
    let e = program
        .run(&Jobs::inline(), "f", 10, 4, &mut [Arg::F32(&mut a)])
        .unwrap_err();
    assert_eq!((e.code, e.line), (codes::TRAP, Some(3)), "{e:?}");
    assert!(
        e.message.contains("index 4") && e.message.contains("length 4"),
        "{e:?}"
    );
    // The program still works afterwards.
    let mut b = vec![0f32; 3];
    let e = program
        .run(&Jobs::inline(), "f", 3, 4, &mut [Arg::F32(&mut b)])
        .unwrap_err();
    assert!(e.message.contains("index 3"), "{e:?}");
}

#[test]
fn endless_loops_hit_the_time_limit() {
    let program = JobProgram::compile(
        "export function spin(a: Float32Array): void {\n  let x = 0;\n  while (x >= 0) { x = x + 1; if (x > 1000) { x = 0; } }\n}",
        "spin.job.ts",
    )
    .unwrap();
    program.set_time_limit_ms(100);
    let mut a = vec![0f32; 1];
    let e = program
        .run(&Jobs::inline(), "spin", 1, 1, &mut [Arg::F32(&mut a)])
        .unwrap_err();
    assert_eq!(e.code, codes::TIMEOUT, "{e:?}");
}

#[test]
fn wrong_arguments_are_reported() {
    let program = JobProgram::compile(PARTICLES, "particles.job.ts").unwrap();
    let mut a = vec![0f32; 4];
    let e = program.run(&Jobs::inline(), "nope", 4, 4, &mut []).unwrap_err();
    assert_eq!(e.code, codes::NO_FUNCTION);
    assert!(e.fix.as_deref().unwrap().contains("step"));
    let e = program
        .run(&Jobs::inline(), "energy", 4, 4, &mut [Arg::F32(&mut a)])
        .unwrap_err();
    assert_eq!(e.code, codes::ARGS);
    let mut b = vec![0f64; 4];
    let mut c = vec![0f32; 5];
    let e = program
        .run(
            &Jobs::inline(),
            "energy",
            4,
            4,
            &mut [Arg::F64(&mut b), Arg::F32(&mut a), Arg::F32(&mut c)],
        )
        .unwrap_err();
    assert!(e.message.contains("Float32Array"), "{e:?}");
}

#[test]
fn hot_reload_keeps_the_last_good_version() {
    let dir = std::env::temp_dir().join(format!("marionette-jobts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("k.job.ts");
    let v1 = "export function k(a: Int32Array): void { for (let i = 0; i < a.length; i++) { a[i] = 1; } }";
    std::fs::write(&path, v1).unwrap();
    let mut lib = JobLibrary::new(&dir);
    let run = |lib: &mut JobLibrary| {
        let mut a = vec![0; 3];
        lib.get("k.job.ts")
            .unwrap()
            .call("k", 3, &mut [Arg::I32(&mut a)])
            .unwrap();
        a[0]
    };
    assert_eq!(run(&mut lib), 1);
    assert_eq!(lib.reload()[0].status, "unchanged");
    std::fs::write(&path, v1.replace("= 1", "= 2")).unwrap();
    assert_eq!(lib.reload()[0].status, "reloaded");
    assert_eq!(run(&mut lib), 2);
    std::fs::write(&path, v1.replace("= 1", "= \"x\"")).unwrap();
    let r = lib.reload();
    assert_eq!(r[0].status, "failed");
    assert_eq!(r[0].errors[0].code, codes::UNSUPPORTED);
    assert_eq!(run(&mut lib), 2, "the last good version keeps running");
    assert_eq!(lib.pending("k.job.ts").len(), 1);
    std::fs::remove_dir_all(&dir).ok();
}
