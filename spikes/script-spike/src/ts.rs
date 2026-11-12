use crate::{Backend, Shared};
use oxc::allocator::Allocator;
use oxc::codegen::Codegen;
use oxc::parser::Parser;
use oxc::semantic::SemanticBuilder;
use oxc::span::SourceType;
use oxc::transformer::{TransformOptions, Transformer};
use rquickjs::{Context, Ctx, Function, Module, Object, Persistent, Runtime, Value};
use std::path::Path;

// Field order matters: saved functions must be dropped before the context and runtime.
pub struct Ts {
    fns: Option<Fns>,
    ctx: Context,
    rt: Runtime,
    generation: u32,
}

struct Fns {
    update: Persistent<Function<'static>>,
    noop: Persistent<Function<'static>>,
    compute: Persistent<Function<'static>>,
    order: Persistent<Function<'static>>,
}

const SCRIPT: &str = r#"
interface Host {
    pos(id: number): [number, number, number];
    vel(id: number): [number, number, number];
    set_pos(id: number, x: number, y: number, z: number): void;
    set_vel(id: number, x: number, y: number, z: number): void;
    rand(): number;
}
declare const host: Host;

const DT = 1 / 60;
const BOUNCE: number = __BOUNCE__;

export function update(id: number): void {
    let [x, y, z] = host.pos(id);
    let [vx, vy, vz] = host.vel(id);
    vy -= 9.8 * DT;
    if (y < 0) {
        y = 0;
        vy = -vy * BOUNCE;
        vx += (host.rand() - 0.5) * 0.1;
    }
    host.set_pos(id, x + vx * DT, y + vy * DT, z + vz * DT);
    host.set_vel(id, vx, vy, vz);
}

export function noop(): void {}

export function compute(n: number): number {
    let s = 0;
    for (let i = 1; i <= n; i++) {
        s += Math.sqrt(i) * 0.5 - (i % 7);
    }
    return s;
}

export function order(): string {
    const t = new Map<object, number>();
    for (let i = 1; i <= 64; i++) {
        t.set({ i }, i);
    }
    const out: string[] = [];
    for (const [, v] of t) {
        out.push(String(v));
    }
    return out.join(" ");
}
"#;

/// TypeScript to JavaScript by removing types (oxc). No type checking happens here.
pub fn strip(source: &str) -> Result<String, String> {
    let allocator = Allocator::default();
    let source_type = SourceType::ts().with_module(true);
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        return Err(format!("{:?}", parsed.diagnostics));
    }
    let mut program = parsed.program;
    let semantic = SemanticBuilder::new().build(&program);
    if !semantic.diagnostics.is_empty() {
        return Err(format!("{:?}", semantic.diagnostics));
    }
    let scoping = semantic.semantic.into_scoping();
    let options = TransformOptions::default();
    let ret = Transformer::new(&allocator, Path::new("game.ts"), &options)
        .build_with_scoping(scoping, &mut program);
    if ret.diagnostics.has_errors() {
        return Err(format!("{:?}", ret.diagnostics));
    }
    Ok(Codegen::new().build(&program).code)
}

fn js_err(ctx: &Ctx<'_>, e: rquickjs::Error) -> String {
    if let rquickjs::Error::Exception = e {
        let v: Value = ctx.catch();
        return format!("{v:?}");
    }
    e.to_string()
}

impl Ts {
    fn load(&mut self, source: &str) -> Result<(), String> {
        let js = strip(source)?;
        self.generation += 1;
        let name = format!("game_{}.js", self.generation);
        let fns = self.ctx.with(|ctx| -> Result<Fns, String> {
            let declared = Module::declare(ctx.clone(), name, js).map_err(|e| js_err(&ctx, e))?;
            let (module, promise) = declared.eval().map_err(|e| js_err(&ctx, e))?;
            promise.finish::<()>().map_err(|e| js_err(&ctx, e))?;
            let ns: Object = module.namespace().map_err(|e| js_err(&ctx, e))?;
            let get = |n: &str| -> Result<Persistent<Function<'static>>, String> {
                let f: Function = ns.get(n).map_err(|e| js_err(&ctx, e))?;
                Ok(Persistent::save(&ctx, f))
            };
            Ok(Fns {
                update: get("update")?,
                noop: get("noop")?,
                compute: get("compute")?,
                order: get("order")?,
            })
        })?;
        self.fns = Some(fns);
        Ok(())
    }
}

impl Backend for Ts {
    const NAME: &'static str = "ts";

    fn source(variant: u32) -> String {
        SCRIPT.replace("__BOUNCE__", &format!("0.{}", 8 - variant % 3))
    }

    fn new(world: Shared, source: &str) -> Result<Self, String> {
        let rt = Runtime::new().map_err(|e| e.to_string())?;
        let ctx = Context::full(&rt).map_err(|e| e.to_string())?;
        ctx.with(|ctx| -> rquickjs::Result<()> {
            let g = ctx.globals();
            let host = Object::new(ctx.clone())?;
            let w = world.clone();
            host.set("pos", Function::new(ctx.clone(), move |id: usize| w.borrow().pos[id].to_vec())?)?;
            let w = world.clone();
            host.set("vel", Function::new(ctx.clone(), move |id: usize| w.borrow().vel[id].to_vec())?)?;
            let w = world.clone();
            host.set(
                "set_pos",
                Function::new(ctx.clone(), move |id: usize, x: f64, y: f64, z: f64| {
                    w.borrow_mut().pos[id] = [x, y, z];
                })?,
            )?;
            let w = world.clone();
            host.set(
                "set_vel",
                Function::new(ctx.clone(), move |id: usize, x: f64, y: f64, z: f64| {
                    w.borrow_mut().vel[id] = [x, y, z];
                })?,
            )?;
            let w = world;
            host.set("rand", Function::new(ctx.clone(), move || w.borrow_mut().rand())?)?;
            g.set("host", host)?;
            // Remove clocks, unseeded randomness and GC-timing observers.
            for name in ["Date", "WeakRef", "FinalizationRegistry", "performance"] {
                g.remove(name)?;
            }
            let math: Object = g.get("Math")?;
            math.remove("random")?;
            Ok(())
        })
        .map_err(|e| e.to_string())?;
        let mut ts = Self {
            fns: None,
            ctx,
            rt,
            generation: 0,
        };
        ts.load(source)?;
        Ok(ts)
    }

    fn reload(&mut self, source: &str) -> Result<(), String> {
        self.load(source)
    }

    fn call_noop(&mut self, n: usize) {
        let f = self.fns.as_ref().unwrap().noop.clone();
        self.ctx.with(|ctx| {
            let f = f.restore(&ctx).unwrap();
            for _ in 0..n {
                f.call::<_, ()>(()).unwrap();
            }
        });
    }

    fn tick(&mut self, entities: usize) {
        let f = self.fns.as_ref().unwrap().update.clone();
        self.ctx.with(|ctx| {
            let f = f.restore(&ctx).unwrap();
            for id in 0..entities {
                if let Err(e) = f.call::<_, ()>((id,)) {
                    panic!("{}", js_err(&ctx, e));
                }
            }
        });
    }

    fn compute(&mut self, n: u32) -> f64 {
        let f = self.fns.as_ref().unwrap().compute.clone();
        self.ctx.with(|ctx| f.restore(&ctx).unwrap().call((n,)).unwrap())
    }

    fn memory(&self) -> usize {
        self.rt.memory_usage().memory_used_size as usize
    }

    fn order_signature(&mut self) -> String {
        let f = self.fns.as_ref().unwrap().order.clone();
        self.ctx.with(|ctx| f.restore(&ctx).unwrap().call(()).unwrap())
    }

    fn globals(&mut self) -> Vec<String> {
        self.ctx.with(|ctx| {
            let names: Vec<String> = ctx
                .eval(
                    "Object.getOwnPropertyNames(globalThis).filter(n => n !== 'globalThis').sort()",
                )
                .unwrap();
            names
        })
    }

    fn compile_only(source: &str) -> Result<(), String> {
        // Type stripping only (QuickJS compiles to bytecode inside `Module::declare`).
        strip(source).map(|_| ())
    }
}
