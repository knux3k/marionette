use crate::{Backend, Shared};
use mlua::chunk::Compiler;
use mlua::{Function, Lua, LuaOptions, StdLib, Table, Value};

pub struct Luau {
    lua: Lua,
    update: Function,
    noop: Function,
    compute: Function,
}

const SCRIPT: &str = r#"--!strict
local DT = 1 / 60
local BOUNCE = __BOUNCE__

function update(id: number)
    local x, y, z = host.pos(id)
    local vx, vy, vz = host.vel(id)
    vy -= 9.8 * DT
    if y < 0 then
        y = 0
        vy = -vy * BOUNCE
        vx += (host.rand() - 0.5) * 0.1
    end
    host.set_pos(id, x + vx * DT, y + vy * DT, z + vz * DT)
    host.set_vel(id, vx, vy, vz)
end

function noop() end

function compute(n: number): number
    local s = 0
    for i = 1, n do
        s += math.sqrt(i) * 0.5 - (i % 7)
    end
    return s
end

function order(): string
    local keys = {}
    local t = {}
    for i = 1, 64 do
        local k = { i = i }
        keys[i] = k
        t[k] = i
    end
    local out = {}
    for _, v in t do
        table.insert(out, tostring(v))
    end
    return table.concat(out, " ")
end
"#;

fn err(e: mlua::Error) -> String {
    e.to_string()
}

impl Luau {
    fn load(lua: &Lua, source: &str) -> Result<(Function, Function, Function), String> {
        lua.load(source).set_name("=game.luau").exec().map_err(err)?;
        let g = lua.globals();
        Ok((
            g.get("update").map_err(err)?,
            g.get("noop").map_err(err)?,
            g.get("compute").map_err(err)?,
        ))
    }
}

impl Backend for Luau {
    const NAME: &'static str = "luau";

    fn source(variant: u32) -> String {
        SCRIPT.replace("__BOUNCE__", &format!("0.{}", 8 - variant % 3))
    }

    fn new(world: Shared, source: &str) -> Result<Self, String> {
        // Only libraries with no clock, file or OS access.
        let lua = Lua::new_with(
            StdLib::MATH | StdLib::STRING | StdLib::TABLE | StdLib::BIT | StdLib::UTF8 | StdLib::BUFFER,
            LuaOptions::default(),
        )
        .map_err(err)?;
        lua.set_compiler(Compiler::new().set_optimization_level(2));
        let host = lua.create_table().map_err(err)?;
        let w = world.clone();
        host.set(
            "pos",
            lua.create_function(move |_, id: usize| {
                let p = w.borrow().pos[id];
                Ok((p[0], p[1], p[2]))
            })
            .map_err(err)?,
        )
        .map_err(err)?;
        let w = world.clone();
        host.set(
            "vel",
            lua.create_function(move |_, id: usize| {
                let v = w.borrow().vel[id];
                Ok((v[0], v[1], v[2]))
            })
            .map_err(err)?,
        )
        .map_err(err)?;
        let w = world.clone();
        host.set(
            "set_pos",
            lua.create_function(move |_, (id, x, y, z): (usize, f64, f64, f64)| {
                w.borrow_mut().pos[id] = [x, y, z];
                Ok(())
            })
            .map_err(err)?,
        )
        .map_err(err)?;
        let w = world.clone();
        host.set(
            "set_vel",
            lua.create_function(move |_, (id, x, y, z): (usize, f64, f64, f64)| {
                w.borrow_mut().vel[id] = [x, y, z];
                Ok(())
            })
            .map_err(err)?,
        )
        .map_err(err)?;
        let w = world;
        host.set("rand", lua.create_function(move |_, ()| Ok(w.borrow_mut().rand())).map_err(err)?)
            .map_err(err)?;
        let g = lua.globals();
        g.set("host", host).map_err(err)?;
        // Seeded randomness only: the built-in generator is seeded from the clock.
        let math: Table = g.get("math").map_err(err)?;
        math.set("random", Value::Nil).map_err(err)?;
        math.set("randomseed", Value::Nil).map_err(err)?;
        // Freeze the standard library; script globals go to a per-thread proxy table.
        lua.sandbox(true).map_err(err)?;
        let (update, noop, compute) = Self::load(&lua, source)?;
        Ok(Self {
            lua,
            update,
            noop,
            compute,
        })
    }

    fn reload(&mut self, source: &str) -> Result<(), String> {
        let (u, n, c) = Self::load(&self.lua, source)?;
        self.update = u;
        self.noop = n;
        self.compute = c;
        Ok(())
    }

    fn call_noop(&mut self, n: usize) {
        for _ in 0..n {
            self.noop.call::<()>(()).unwrap();
        }
    }

    fn tick(&mut self, entities: usize) {
        for id in 0..entities {
            self.update.call::<()>(id).unwrap();
        }
    }

    fn compute(&mut self, n: u32) -> f64 {
        self.compute.call(n).unwrap()
    }

    fn memory(&self) -> usize {
        self.lua.used_memory()
    }

    fn order_signature(&mut self) -> String {
        let f: Function = self.lua.globals().get("order").unwrap();
        f.call(()).unwrap()
    }

    fn globals(&mut self) -> Vec<String> {
        // The sandbox proxy's __index is the frozen global table.
        let g = self.lua.globals();
        let base: Table = g
            .metatable()
            .and_then(|m| m.get::<Table>("__index").ok())
            .unwrap_or(g);
        let mut names: Vec<String> = base
            .pairs::<String, Value>()
            .filter_map(|kv| kv.ok())
            .flat_map(|(k, v)| match v {
                Value::Table(t) => t
                    .pairs::<String, Value>()
                    .filter_map(|kv| kv.ok())
                    .map(|(m, _)| format!("{k}.{m}"))
                    .collect::<Vec<_>>(),
                _ => vec![k],
            })
            .collect();
        names.sort();
        names
    }

    fn compile_only(source: &str) -> Result<(), String> {
        Compiler::new()
            .set_optimization_level(2)
            .compile(source)
            .map(|_| ())
            .map_err(err)
    }
}
