//! Running compiled job files: wasmtime instances, one per worker in use, fed chunk by chunk
//! through `marionette-jobs` (decision 0024 rules: chunks come from the data, results come
//! back in chunk order).

use crate::compile::{self, Compiled, Elem, FunctionInfo, MATH1, MATH2, ParamType, Safety, codes, trap};
use marionette_core::Diagnostic;
use marionette_jobs::Jobs;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use wasmtime::{
    Caller, Config, Engine, Func, Inlining, InstancePre, Linker, Memory, Module, OptLevel, Store, Trap, Val,
};

/// How often the watchdog thread advances the engine's epoch.
const EPOCH_MS: u64 = 10;
/// Default time limit of one chunk call, in milliseconds.
pub const DEFAULT_TIME_LIMIT_MS: u64 = 5_000;

/// The process-wide WebAssembly engines, one per [`Safety`] level (the time limit is an
/// engine setting). Functions are inlined; no threads, no relaxed SIMD. NaNs are made
/// canonical where they are stored (see the compiler), not after every operation.
fn engine(safety: Safety) -> &'static Engine {
    static CHECKED: OnceLock<Engine> = OnceLock::new();
    static UNCHECKED: OnceLock<Engine> = OnceLock::new();
    let cell = if safety.checked() { &CHECKED } else { &UNCHECKED };
    cell.get_or_init(|| {
        let mut config = Config::new();
        config.cranelift_opt_level(OptLevel::Speed);
        config.compiler_inlining(Inlining::Yes);
        config.relaxed_simd_deterministic(true);
        config.epoch_interruption(safety.checked());
        let engine = Engine::new(&config).expect("the WebAssembly engine configuration is valid");
        if safety.checked() {
            let ticker = engine.clone();
            std::thread::Builder::new()
                .name("marionette-jobts-watchdog".into())
                .spawn(move || {
                    loop {
                        std::thread::sleep(std::time::Duration::from_millis(EPOCH_MS));
                        ticker.increment_epoch();
                    }
                })
                .expect("the watchdog thread starts");
        }
        engine
    })
}

/// Why a job stopped (raised by the `trap` import).
#[derive(Debug)]
struct JobTrap {
    kind: i32,
    line: i32,
    a: i32,
    b: i32,
}

impl std::fmt::Display for JobTrap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "job trap {} at line {}", self.kind, self.line)
    }
}
impl std::error::Error for JobTrap {}

/// `Math.pow` with the special cases JavaScript defines and C's `pow` does not (same as the
/// script runtime, so compiled and interpreted jobs agree).
fn js_pow(x: f64, y: f64) -> f64 {
    if y.is_nan() || ((x == 1.0 || x == -1.0) && y.is_infinite()) {
        return f64::NAN;
    }
    libm::pow(x, y)
}

/// JavaScript ToInt32 for values outside the fast path (|x| >= 2^63, infinities, NaN).
fn to_int32_slow(x: f64) -> i32 {
    if !x.is_finite() {
        return 0;
    }
    let m = libm::fmod(libm::trunc(x), 4_294_967_296.0);
    let m = if m < 0.0 { m + 4_294_967_296.0 } else { m };
    (m as u32) as i32
}

fn math1(name: &str) -> fn(f64) -> f64 {
    match name {
        "sin" => libm::sin,
        "cos" => libm::cos,
        "tan" => libm::tan,
        "asin" => libm::asin,
        "acos" => libm::acos,
        "atan" => libm::atan,
        "sinh" => libm::sinh,
        "cosh" => libm::cosh,
        "tanh" => libm::tanh,
        "asinh" => libm::asinh,
        "acosh" => libm::acosh,
        "atanh" => libm::atanh,
        "exp" => libm::exp,
        "expm1" => libm::expm1,
        "log" => libm::log,
        "log1p" => libm::log1p,
        "log2" => libm::log2,
        "log10" => libm::log10,
        "cbrt" => libm::cbrt,
        other => unreachable!("unknown math function {other}"),
    }
}

fn math2(name: &str) -> fn(f64, f64) -> f64 {
    match name {
        "atan2" => libm::atan2,
        "pow" => js_pow,
        "fmod" => libm::fmod,
        "hypot" => libm::hypot,
        other => unreachable!("unknown math function {other}"),
    }
}

fn linker(safety: Safety) -> wasmtime::Result<Linker<()>> {
    let mut l = Linker::new(engine(safety));
    l.func_wrap(
        "env",
        "trap",
        |_: Caller<'_, ()>, kind: i32, line: i32, a: i32, b: i32| -> wasmtime::Result<()> {
            Err(wasmtime::Error::new(JobTrap { kind, line, a, b }))
        },
    )?;
    l.func_wrap("env", "to_int32", to_int32_slow)?;
    for n in MATH1 {
        let f = math1(n);
        l.func_wrap("math", n, move |x: f64| f(x))?;
    }
    for n in MATH2 {
        let f = math2(n);
        l.func_wrap("math", n, move |x: f64, y: f64| f(x, y))?;
    }
    Ok(l)
}

/// One argument of [`JobProgram::run`]. Order and kinds follow the function's parameters.
///
/// - Chunked arrays (`F32`, `F64`, `I32` and the `*In` forms) must all have the run's
///   length; each chunk call sees only its own piece, starting at index 0. The mutable forms
///   are copied back after the call; the `In` forms are read only.
/// - `*All` arrays are passed whole to every chunk, read only (lookup tables, neighbours).
/// - `ChunkStart` fills an `i32` parameter with the index of the chunk's first element.
#[derive(Debug)]
pub enum Arg<'a> {
    F32(&'a mut [f32]),
    F64(&'a mut [f64]),
    I32(&'a mut [i32]),
    F32In(&'a [f32]),
    F64In(&'a [f64]),
    I32In(&'a [i32]),
    F32All(&'a [f32]),
    F64All(&'a [f64]),
    I32All(&'a [i32]),
    Number(f64),
    Int(i32),
    Bool(bool),
    ChunkStart,
}

impl Arg<'_> {
    fn describe(&self) -> &'static str {
        match self {
            Arg::F32(_) | Arg::F32In(_) | Arg::F32All(_) => "Float32Array",
            Arg::F64(_) | Arg::F64In(_) | Arg::F64All(_) => "Float64Array",
            Arg::I32(_) | Arg::I32In(_) | Arg::I32All(_) => "Int32Array",
            Arg::Number(_) => "number",
            Arg::Int(_) | Arg::ChunkStart => "i32",
            Arg::Bool(_) => "boolean",
        }
    }
}

/// An argument reduced to plain addresses, so chunk calls can run on any worker.
#[derive(Clone, Copy)]
enum Planned {
    Chunked { addr: usize, elem: Elem, write: bool },
    All { addr: usize, len: usize, elem: Elem },
    Scalar(Val),
    ChunkStart,
}

// SAFETY: the addresses come from slices borrowed for the whole `run` call; chunk calls
// touch disjoint ranges of the mutable ones and only read the others.
unsafe impl Send for Planned {}
unsafe impl Sync for Planned {}

struct Slot {
    store: Store<()>,
    memory: Memory,
    funcs: HashMap<String, Func>,
    /// The run whose whole (`*All`) arrays are already in this instance's memory.
    shared_run: u64,
}

/// A compiled job file, ready to run. Cheap to share between threads (`Arc<JobProgram>`).
pub struct JobProgram {
    compiled: Compiled,
    pre: InstancePre<()>,
    slots: Mutex<Vec<Slot>>,
    runs: AtomicU64,
    time_limit_ms: AtomicU64,
}

impl std::fmt::Debug for JobProgram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobProgram")
            .field("file", &self.compiled.file)
            .finish()
    }
}

fn internal(file: &str, e: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::error(
        codes::INTERNAL,
        format!("the WebAssembly runtime refused the job: {e}"),
    )
    .file(file)
    .fix("this is an engine bug; report it with the job file")
}

const ALIGN: usize = 16;

fn align(n: usize) -> usize {
    n.div_ceil(ALIGN) * ALIGN
}

impl JobProgram {
    /// Compiles job TypeScript with safety checks. Errors carry the file, line, column and
    /// a fix.
    pub fn compile(source: &str, file: &str) -> Result<Self, Vec<Diagnostic>> {
        Self::compile_with(source, file, Safety::Checked)
    }

    /// [`JobProgram::compile`] with a choice of [`Safety`].
    pub fn compile_with(source: &str, file: &str, safety: Safety) -> Result<Self, Vec<Diagnostic>> {
        let compiled = compile::compile_with(source, file, safety)?;
        Self::from_compiled(compiled).map_err(|d| vec![d])
    }

    /// The safety level the program was compiled with.
    pub fn safety(&self) -> Safety {
        self.compiled.safety
    }

    /// Reads and compiles a job file.
    pub fn load(path: &std::path::Path) -> Result<Self, Vec<Diagnostic>> {
        let file = path.to_string_lossy().replace('\\', "/");
        let source = std::fs::read_to_string(path).map_err(|e| {
            vec![
                Diagnostic::error(codes::READ, format!("cannot read the job file: {e}"))
                    .file(&file)
                    .fix("check the path (relative to the scene or the working directory)"),
            ]
        })?;
        Self::compile(&source, &file)
    }

    fn from_compiled(compiled: Compiled) -> Result<Self, Diagnostic> {
        let file = compiled.file.clone();
        let safety = compiled.safety;
        let module = Module::new(engine(safety), &compiled.wasm).map_err(|e| internal(&file, e))?;
        let pre = linker(safety)
            .and_then(|l| l.instantiate_pre(&module))
            .map_err(|e| internal(&file, e))?;
        Ok(Self {
            compiled,
            pre,
            slots: Mutex::new(Vec::new()),
            runs: AtomicU64::new(0),
            time_limit_ms: AtomicU64::new(DEFAULT_TIME_LIMIT_MS),
        })
    }

    /// The file name the program was compiled from.
    pub fn file(&self) -> &str {
        &self.compiled.file
    }

    /// Every function of the file (exported ones can be run).
    pub fn functions(&self) -> &[FunctionInfo] {
        &self.compiled.functions
    }

    /// The compiled WebAssembly module.
    pub fn wasm(&self) -> &[u8] {
        &self.compiled.wasm
    }

    /// Sets how long one chunk call may run before it is stopped with `jobts.timeout`.
    pub fn set_time_limit_ms(&self, ms: u64) {
        self.time_limit_ms.store(ms.max(EPOCH_MS), Ordering::Relaxed);
    }

    fn function(&self, name: &str) -> Result<&FunctionInfo, Diagnostic> {
        self.compiled
            .functions
            .iter()
            .find(|f| f.name == name && f.exported)
            .ok_or_else(|| {
                let names: Vec<&str> = self
                    .compiled
                    .functions
                    .iter()
                    .filter(|f| f.exported)
                    .map(|f| f.name.as_str())
                    .collect();
                Diagnostic::error(
                    codes::NO_FUNCTION,
                    format!("`{name}` is not an exported function of the job file"),
                )
                .file(self.file())
                .fix(format!(
                    "exported functions: {}; add `export` in front of the function",
                    names.join(", ")
                ))
            })
    }

    fn plan(&self, info: &FunctionInfo, len: usize, args: &mut [Arg]) -> Result<Vec<Planned>, Diagnostic> {
        let args_err = |m: String| {
            Diagnostic::error(codes::ARGS, m).file(self.file()).fix(format!(
                "`{}` takes ({})",
                info.name,
                signature(info)
            ))
        };
        if args.len() != info.params.len() {
            return Err(args_err(format!(
                "`{}` takes {} arguments, {} given",
                info.name,
                info.params.len(),
                args.len()
            )));
        }
        let mut plan = Vec::with_capacity(args.len());
        for (arg, (pname, p)) in args.iter_mut().zip(&info.params) {
            let chunked = |addr: usize, n: usize, elem: Elem, write: bool| {
                if n != len {
                    return Err(args_err(format!(
                        "array `{pname}` has {n} elements but the run covers {len}"
                    )));
                }
                Ok(Planned::Chunked { addr, elem, write })
            };
            let planned = match (arg, p) {
                (Arg::F32(s), ParamType::Array(Elem::F32)) => {
                    chunked(s.as_mut_ptr() as usize, s.len(), Elem::F32, true)?
                }
                (Arg::F64(s), ParamType::Array(Elem::F64)) => {
                    chunked(s.as_mut_ptr() as usize, s.len(), Elem::F64, true)?
                }
                (Arg::I32(s), ParamType::Array(Elem::I32)) => {
                    chunked(s.as_mut_ptr() as usize, s.len(), Elem::I32, true)?
                }
                (Arg::F32In(s), ParamType::Array(Elem::F32)) => {
                    chunked(s.as_ptr() as usize, s.len(), Elem::F32, false)?
                }
                (Arg::F64In(s), ParamType::Array(Elem::F64)) => {
                    chunked(s.as_ptr() as usize, s.len(), Elem::F64, false)?
                }
                (Arg::I32In(s), ParamType::Array(Elem::I32)) => {
                    chunked(s.as_ptr() as usize, s.len(), Elem::I32, false)?
                }
                (Arg::F32All(s), ParamType::Array(Elem::F32)) => Planned::All {
                    addr: s.as_ptr() as usize,
                    len: s.len(),
                    elem: Elem::F32,
                },
                (Arg::F64All(s), ParamType::Array(Elem::F64)) => Planned::All {
                    addr: s.as_ptr() as usize,
                    len: s.len(),
                    elem: Elem::F64,
                },
                (Arg::I32All(s), ParamType::Array(Elem::I32)) => Planned::All {
                    addr: s.as_ptr() as usize,
                    len: s.len(),
                    elem: Elem::I32,
                },
                (Arg::Number(v), ParamType::Number) => Planned::Scalar(Val::F64(v.to_bits())),
                (Arg::Int(v), ParamType::I32) => Planned::Scalar(Val::I32(*v)),
                (Arg::Bool(v), ParamType::Boolean) => Planned::Scalar(Val::I32(*v as i32)),
                (Arg::ChunkStart, ParamType::I32) => Planned::ChunkStart,
                (a, p) => {
                    return Err(args_err(format!(
                        "parameter `{pname}` is {} but a {} was given",
                        p.ts_name(),
                        a.describe()
                    )));
                }
            };
            plan.push(planned);
        }
        Ok(plan)
    }

    /// Runs the exported function `name` over `len` elements in chunks of `chunk`, spread over
    /// the workers of `jobs`. Returns one value per chunk, in chunk order, when the function
    /// returns a value (fold them in order for a deterministic reduction); otherwise empty.
    ///
    /// Results never depend on the number of threads: chunk boundaries come from `len` and
    /// `chunk` only, each call sees only its own chunk, and WebAssembly arithmetic is exactly
    /// specified (NaNs are canonicalized).
    pub fn run(
        &self,
        jobs: &Jobs,
        name: &str,
        len: usize,
        chunk: usize,
        args: &mut [Arg],
    ) -> Result<Vec<f64>, Diagnostic> {
        let info = self.function(name)?;
        let plan = self.plan(info, len, args)?;
        if len > i32::MAX as usize {
            return Err(
                Diagnostic::error(codes::ARGS, "a run covers at most 2^31 - 1 elements").file(self.file()),
            );
        }
        let run_id = self.runs.fetch_add(1, Ordering::Relaxed) + 1;
        let results = jobs.map_chunks(len, chunk.max(1), |range| {
            self.call_chunk(info, &plan, run_id, range)
        });
        let mut out = Vec::new();
        for r in results {
            if let Some(v) = r? {
                out.push(v);
            }
        }
        Ok(out)
    }

    /// Like [`JobProgram::run`] for a function that returns a number: the per-chunk values
    /// added up in chunk order.
    pub fn run_sum(
        &self,
        jobs: &Jobs,
        name: &str,
        len: usize,
        chunk: usize,
        args: &mut [Arg],
    ) -> Result<f64, Diagnostic> {
        Ok(self
            .run(jobs, name, len, chunk, args)?
            .iter()
            .fold(0.0, |a, b| a + b))
    }

    /// Calls the function once on the whole data (one chunk), on the calling thread.
    pub fn call(&self, name: &str, len: usize, args: &mut [Arg]) -> Result<Option<f64>, Diagnostic> {
        let r = self.run(&Jobs::inline(), name, len, len.max(1), args)?;
        Ok(r.first().copied())
    }

    fn take_slot(&self) -> Result<Slot, Diagnostic> {
        if let Some(s) = self.slots.lock().expect("slot pool").pop() {
            return Ok(s);
        }
        let mut store = Store::new(engine(self.compiled.safety), ());
        if self.compiled.safety.checked() {
            store.epoch_deadline_trap();
        }
        let instance = self
            .pre
            .instantiate(&mut store)
            .map_err(|e| internal(self.file(), e))?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| internal(self.file(), "no memory export"))?;
        let mut funcs = HashMap::new();
        for f in self.compiled.functions.iter().filter(|f| f.exported) {
            if let Some(func) = instance.get_func(&mut store, &f.name) {
                funcs.insert(f.name.clone(), func);
            }
        }
        Ok(Slot {
            store,
            memory,
            funcs,
            shared_run: 0,
        })
    }

    fn call_chunk(
        &self,
        info: &FunctionInfo,
        plan: &[Planned],
        run_id: u64,
        range: std::ops::Range<usize>,
    ) -> Result<Option<f64>, Diagnostic> {
        let mut slot = self.take_slot()?;
        let result = self.call_in(&mut slot, info, plan, run_id, range);
        // A slot whose call trapped is dropped (its state is unknown); others go back.
        if result.is_ok() {
            self.slots.lock().expect("slot pool").push(slot);
        }
        result
    }

    fn call_in(
        &self,
        slot: &mut Slot,
        info: &FunctionInfo,
        plan: &[Planned],
        run_id: u64,
        range: std::ops::Range<usize>,
    ) -> Result<Option<f64>, Diagnostic> {
        let n = range.len();
        // Layout: whole arrays first (copied once per run and instance), then chunk arrays.
        let mut offsets = Vec::with_capacity(plan.len());
        let mut at = ALIGN; // keep address 0 unused
        for p in plan {
            if let Planned::All { len, elem, .. } = p {
                offsets.push(at);
                at = align(at + len * elem.size() as usize);
            } else {
                offsets.push(0);
            }
        }
        for (i, p) in plan.iter().enumerate() {
            if let Planned::Chunked { elem, .. } = p {
                offsets[i] = at;
                at = align(at + n * elem.size() as usize);
            }
        }
        if at > u32::MAX as usize {
            return Err(Diagnostic::error(codes::ARGS, "one chunk's data exceeds 4 GiB")
                .file(self.file())
                .fix("use a smaller chunk size"));
        }
        let have = slot.memory.data_size(&slot.store);
        if at > have {
            let pages = (at - have).div_ceil(65_536) as u64;
            slot.memory.grow(&mut slot.store, pages).map_err(|e| {
                Diagnostic::error(codes::ARGS, format!("cannot grow job memory: {e}")).file(self.file())
            })?;
        }
        let refresh_shared = slot.shared_run != run_id;
        {
            let mem = slot.memory.data_mut(&mut slot.store);
            for (p, &off) in plan.iter().zip(&offsets) {
                match *p {
                    Planned::All { addr, len, elem } if refresh_shared => {
                        let bytes = len * elem.size() as usize;
                        // SAFETY: `addr` points at a live slice of `len` elements (see `Planned`).
                        let src = unsafe { std::slice::from_raw_parts(addr as *const u8, bytes) };
                        mem[off..off + bytes].copy_from_slice(src);
                    }
                    Planned::Chunked { addr, elem, .. } => {
                        let size = elem.size() as usize;
                        let bytes = n * size;
                        // SAFETY: the chunk `range` lies inside the slice, which lives for the run.
                        let src = unsafe {
                            std::slice::from_raw_parts((addr + range.start * size) as *const u8, bytes)
                        };
                        mem[off..off + bytes].copy_from_slice(src);
                    }
                    _ => {}
                }
            }
        }
        slot.shared_run = run_id;
        let mut params = Vec::with_capacity(plan.len() * 2);
        for (p, &off) in plan.iter().zip(&offsets) {
            match *p {
                Planned::Chunked { .. } => {
                    params.push(Val::I32(off as i32));
                    params.push(Val::I32(n as i32));
                }
                Planned::All { len, .. } => {
                    params.push(Val::I32(off as i32));
                    params.push(Val::I32(len as i32));
                }
                Planned::Scalar(v) => params.push(v),
                Planned::ChunkStart => params.push(Val::I32(range.start as i32)),
            }
        }
        let mut results = match info.returns {
            ParamType::Void => vec![],
            ParamType::Number => vec![Val::F64(0)],
            _ => vec![Val::I32(0)],
        };
        let func = slot.funcs[&info.name];
        if self.compiled.safety.checked() {
            let ticks = self.time_limit_ms.load(Ordering::Relaxed).div_ceil(EPOCH_MS);
            slot.store.set_epoch_deadline(ticks);
        }
        func.call(&mut slot.store, &params, &mut results)
            .map_err(|e| self.trap_diagnostic(info, &range, e))?;
        // Copy writable chunk arrays back.
        let mem = slot.memory.data(&slot.store);
        for (p, &off) in plan.iter().zip(&offsets) {
            if let Planned::Chunked {
                addr,
                elem,
                write: true,
            } = *p
            {
                let size = elem.size() as usize;
                let bytes = n * size;
                // SAFETY: as above; the chunk ranges of different calls never overlap.
                let dst =
                    unsafe { std::slice::from_raw_parts_mut((addr + range.start * size) as *mut u8, bytes) };
                dst.copy_from_slice(&mem[off..off + bytes]);
            }
        }
        Ok(match (info.returns, results.first()) {
            (ParamType::Number, Some(Val::F64(bits))) => Some(canonical(f64::from_bits(*bits))),
            (ParamType::I32, Some(Val::I32(v))) => Some(*v as f64),
            (ParamType::Boolean, Some(Val::I32(v))) => Some((*v != 0) as i32 as f64),
            _ => None,
        })
    }

    fn trap_diagnostic(
        &self,
        info: &FunctionInfo,
        range: &std::ops::Range<usize>,
        e: wasmtime::Error,
    ) -> Diagnostic {
        let chunk = format!("chunk {}..{}", range.start, range.end);
        if let Some(t) = e.downcast_ref::<JobTrap>() {
            let (msg, fix) = match t.kind {
                trap::INDEX => (
                    format!(
                        "index {} is outside the array (length {}) in `{}`, {chunk}",
                        t.a, t.b, info.name
                    ),
                    "chunked arrays are indexed from 0 to chunk length - 1; check the loop bound (use `a.length`)",
                ),
                trap::NO_RETURN => (
                    format!(
                        "`{}` reached its end without returning a value, {chunk}",
                        info.name
                    ),
                    "add a `return` on every path",
                ),
                _ => (
                    format!("integer remainder by zero in `{}`, {chunk}", info.name),
                    "check the divisor, or use number (float) operands, which give NaN like JavaScript",
                ),
            };
            return Diagnostic::error(codes::TRAP, msg)
                .file(self.file())
                .at(Some(t.line as usize), None)
                .fix(fix);
        }
        if let Some(Trap::Interrupt) = e.downcast_ref::<Trap>() {
            return Diagnostic::error(
                codes::TIMEOUT,
                format!(
                    "`{}` ran longer than {} ms on {chunk}",
                    info.name,
                    self.time_limit_ms.load(Ordering::Relaxed)
                ),
            )
            .file(self.file())
            .fix("look for a loop whose condition never becomes false");
        }
        Diagnostic::error(codes::TRAP, format!("`{}` stopped: {e}, {chunk}", info.name)).file(self.file())
    }
}

/// The one NaN results may contain (see `compile::FnCtx::canonical_nan`).
fn canonical(x: f64) -> f64 {
    if x.is_nan() { f64::NAN } else { x }
}

/// `name: type, ...` of a function, for messages.
pub fn signature(info: &FunctionInfo) -> String {
    info.params
        .iter()
        .map(|(n, p)| format!("{n}: {}", p.ts_name()))
        .collect::<Vec<_>>()
        .join(", ")
}
