//! Scripting spike (plan line 4): the same game script in Luau (mlua) and TypeScript
//! (oxc type stripping + QuickJS-ng via rquickjs), measured the same way.
//!
//! Run one backend at a time so build cost and binary size can be measured separately:
//!   cargo run --release --features luau -- all
//!   cargo run --release --features ts -- all
//!   cargo run --release --features luau,ts -- all
//! Subcommands: `all` (every measurement), `order` (prints iteration-order signatures, run
//! twice in separate processes to compare), `hash` (determinism hash only).

use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

#[cfg(all(feature = "luau", feature = "ts"))]
mod eval;
#[cfg(feature = "luau")]
mod luau;
#[cfg(feature = "ts")]
mod ts;

/// Plain world the scripts move: positions and velocities in f64, seeded random numbers.
pub struct World {
    pub pos: Vec<[f64; 3]>,
    pub vel: Vec<[f64; 3]>,
    pub rng: u64,
}

impl World {
    pub fn new(n: usize) -> Self {
        let mut w = Self {
            pos: Vec::with_capacity(n),
            vel: Vec::with_capacity(n),
            rng: 1,
        };
        for i in 0..n {
            let f = i as f64;
            w.pos.push([f, 10.0 + (i % 13) as f64, -f]);
            w.vel.push([0.5, 0.0, -0.25]);
        }
        w
    }
    /// SplitMix64, same as the engine's `SimRng`; uniform in [0, 1).
    pub fn rand(&mut self) -> f64 {
        self.rng = self.rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }
    pub fn hash(&self) -> String {
        let mut h = Sha256::new();
        for (p, v) in self.pos.iter().zip(&self.vel) {
            for x in p.iter().chain(v) {
                h.update(x.to_bits().to_le_bytes());
            }
        }
        h.update(self.rng.to_le_bytes());
        h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect()
    }
}

pub type Shared = Rc<RefCell<World>>;

/// What each language binding offers the spike.
pub trait Backend: Sized {
    const NAME: &'static str;
    /// Script source in this language: `variant` changes one constant (for reload tests).
    fn source(variant: u32) -> String;
    /// Creates a sandboxed VM, installs the host API and runs the script's top level.
    fn new(world: Shared, source: &str) -> Result<Self, String>;
    /// Replaces the script with new source in the running VM.
    fn reload(&mut self, source: &str) -> Result<(), String>;
    /// Calls the script's `noop()` `n` times.
    fn call_noop(&mut self, n: usize);
    /// Calls `update(id)` for every entity once.
    fn tick(&mut self, entities: usize);
    /// Calls `compute(n)` (pure arithmetic inside the VM).
    fn compute(&mut self, n: u32) -> f64;
    /// Bytes of VM heap in use.
    fn memory(&self) -> usize;
    /// A string describing table/object iteration order for keys that are objects.
    fn order_signature(&mut self) -> String;
    /// Globals reachable from scripts (sorted), to audit the sandbox.
    fn globals(&mut self) -> Vec<String>;
    /// Source-to-loadable compile time on its own (TS: type stripping; Luau: bytecode).
    fn compile_only(source: &str) -> Result<(), String>;
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn measure<B: Backend>(what: &str) {
    let name = B::NAME;
    if what == "order" {
        let w = Rc::new(RefCell::new(World::new(1)));
        let mut b = B::new(w, &B::source(0)).expect("load");
        println!("{name} order {}", b.order_signature());
        return;
    }
    // Determinism: 1,000 entities, 1,000 ticks, twice; hash must match (and match the other
    // language, since both do the same f64 operations in the same order).
    let run = || {
        let w = Rc::new(RefCell::new(World::new(1000)));
        let mut b = B::new(w.clone(), &B::source(0)).expect("load");
        for _ in 0..1000 {
            b.tick(1000);
        }
        let h = w.borrow().hash();
        h
    };
    let (h1, h2) = (run(), run());
    println!("{name} hash {h1} {}", if h1 == h2 { "same-twice" } else { "DIFFERENT" });
    if what == "hash" {
        return;
    }

    // Startup: new VM + host API + run the script top level.
    let src = B::source(0);
    let startup: Vec<f64> = (0..50)
        .map(|_| {
            let w = Rc::new(RefCell::new(World::new(1)));
            let t = Instant::now();
            let b = B::new(w, &src).expect("load");
            let e = t.elapsed().as_secs_f64() * 1e6;
            drop(b);
            e
        })
        .collect();
    println!("{name} startup_us {:.0}", median(startup));
    let compile: Vec<f64> = (0..50)
        .map(|_| {
            let t = Instant::now();
            B::compile_only(&src).expect("compile");
            t.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    println!("{name} compile_us {:.0}", median(compile));

    let w = Rc::new(RefCell::new(World::new(10_000)));
    let mut b = B::new(w, &src).expect("load");
    println!("{name} memory_kb {}", b.memory() / 1024);

    // Reload: alternate between two variants, each time a different constant.
    let reload: Vec<f64> = (0..50)
        .map(|i| {
            let s = B::source(1 + i % 2);
            let t = Instant::now();
            b.reload(&s).expect("reload");
            t.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    println!("{name} reload_us {:.0}", median(reload));

    // Empty call from Rust into the script.
    let n = 1_000_000;
    let t = Instant::now();
    b.call_noop(n);
    println!("{name} noop_call_ns {:.0}", t.elapsed().as_secs_f64() * 1e9 / n as f64);

    // Per-entity update: 4 host calls (pos, vel, set_pos, set_vel) plus arithmetic.
    b.tick(10_000); // warm up
    let t = Instant::now();
    let ticks = 50;
    for _ in 0..ticks {
        b.tick(10_000);
    }
    let per = t.elapsed().as_secs_f64() * 1e9 / (ticks * 10_000) as f64;
    println!("{name} entity_update_ns {per:.0}");
    println!("{name} entities_per_ms_at_1ms {:.0}", 1e6 / per);

    // Pure script arithmetic.
    let t = Instant::now();
    let r = b.compute(1_000_000);
    println!(
        "{name} compute_1m_ms {:.1} (result {r})",
        t.elapsed().as_secs_f64() * 1e3
    );
    println!("{name} globals {}", b.globals().join(","));
}

fn main() {
    let what = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    #[cfg(all(feature = "luau", feature = "ts"))]
    if what == "eval" {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("eval");
        let tasks: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("tasks.json")).unwrap()).unwrap();
        eval::luau(&dir, &tasks);
        eval::ts(&dir, &tasks);
        return;
    }
    #[cfg(feature = "luau")]
    measure::<luau::Luau>(&what);
    #[cfg(feature = "ts")]
    measure::<ts::Ts>(&what);
    let _ = &what;
}
