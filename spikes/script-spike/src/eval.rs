//! Runs eval/tasks.luau and eval/tasks.ts against eval/tasks.json (first-attempt solutions
//! written by a coding AI from the task descriptions). Prints pass/fail per case.

use serde_json::{Value as J, json};
use std::path::Path;

fn same(a: &J, b: &J) -> bool {
    match (a, b) {
        (J::Number(x), J::Number(y)) => (x.as_f64().unwrap() - y.as_f64().unwrap()).abs() < 1e-9,
        (J::Array(x), J::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q)),
        (J::Object(x), J::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        // An empty Luau table reads back as either.
        (J::Array(x), J::Object(y)) | (J::Object(y), J::Array(x)) => x.is_empty() && y.is_empty(),
        _ => a == b,
    }
}

/// Replaces "$first"/"$second" with 1/2 (Luau) or 0/1 (TypeScript).
fn bases(v: &J, base: i64) -> J {
    match v {
        J::String(s) if s == "$first" => json!(base),
        J::String(s) if s == "$second" => json!(base + 1),
        J::Array(a) => J::Array(a.iter().map(|x| bases(x, base)).collect()),
        J::Object(o) => J::Object(o.iter().map(|(k, x)| (k.clone(), bases(x, base))).collect()),
        _ => v.clone(),
    }
}

type Runner<'a> = dyn FnMut(&str, &[J]) -> Result<(J, J), String> + 'a;

fn run(lang: &str, base: i64, tasks: &J, call: &mut Runner<'_>) -> (usize, usize) {
    let (mut pass, mut total) = (0, 0);
    for t in tasks["tasks"].as_array().unwrap() {
        let name = t["name"].as_str().unwrap();
        for (i, c) in t["cases"].as_array().unwrap().iter().enumerate() {
            let c = bases(c, base);
            total += 1;
            let args = c["args"].as_array().unwrap();
            let outcome = call(name, args);
            let ok = match &outcome {
                Ok((got, arg0)) => {
                    same(got, &c["expect"]) && c.get("expect_arg0").is_none_or(|e| same(arg0, e))
                }
                Err(_) => false,
            };
            if ok {
                pass += 1;
            } else {
                println!("{lang} FAIL {name}#{i}: {outcome:?} expected {}", c["expect"]);
            }
        }
    }
    println!("{lang} eval {pass}/{total}");
    (pass, total)
}

#[cfg(feature = "luau")]
pub fn luau(dir: &Path, tasks: &J) {
    use mlua::{Function, Lua, LuaSerdeExt, MultiValue, Value};
    // Plain tables, as a script would build them (no protected array metatable).
    let opts = mlua::serde::ser::Options::new().set_array_metatable(false);
    let lua = Lua::new();
    lua.sandbox(true).unwrap();
    let src = std::fs::read_to_string(dir.join("tasks.luau")).unwrap();
    if let Err(e) = lua.load(&src).set_name("=tasks.luau").exec() {
        println!("luau LOAD ERROR {e}");
        return;
    }
    run("luau", 1, tasks, &mut |name, args| {
        let f: Function = lua.globals().get(name).map_err(|e| e.to_string())?;
        let vals: Vec<Value> = args.iter().map(|a| lua.to_value_with(a, opts).unwrap()).collect();
        let r: MultiValue = f.call(MultiValue::from_iter(vals.clone())).map_err(|e| e.to_string())?;
        let first = r.into_iter().next().unwrap_or(Value::Nil);
        let got: J = lua.from_value(first).map_err(|e| e.to_string())?;
        let arg0: J = vals.first().map_or(J::Null, |v| lua.from_value(v.clone()).unwrap_or(J::Null));
        Ok((got, arg0))
    });
}

#[cfg(feature = "ts")]
pub fn ts(dir: &Path, tasks: &J) {
    use rquickjs::{Context, Function, Module, Object, Runtime, Value};
    let src = std::fs::read_to_string(dir.join("tasks.ts")).unwrap();
    let js = match crate::ts::strip(&src) {
        Ok(js) => js,
        Err(e) => {
            println!("ts STRIP ERROR {e}");
            return;
        }
    };
    let rt = Runtime::new().unwrap();
    let ctx = Context::full(&rt).unwrap();
    ctx.with(|ctx| {
        let (module, promise) = match Module::declare(ctx.clone(), "tasks.js", js).and_then(|m| m.eval()) {
            Ok(m) => m,
            Err(e) => {
                println!("ts LOAD ERROR {e} {:?}", ctx.catch());
                return;
            }
        };
        promise.finish::<()>().unwrap();
        let ns: Object = module.namespace().unwrap();
        run("ts", 0, tasks, &mut |name, args| {
            let f: Function = ns.get(name).map_err(|e| e.to_string())?;
            let vals: Vec<Value> = args
                .iter()
                .map(|a| ctx.json_parse(a.to_string()).unwrap())
                .collect();
            let mut call_args = rquickjs::function::Args::new(ctx.clone(), vals.len());
            for v in &vals {
                call_args.push_arg(v.clone()).unwrap();
            }
            let r: Value = f
                .call_arg(call_args)
                .map_err(|e| format!("{e} {:?}", ctx.catch()))?;
            let arg0 = vals.first().cloned().map_or(J::Null, |v| to_json(&ctx, v));
            Ok((to_json(&ctx, r), arg0))
        });
    });
}

#[cfg(feature = "ts")]
fn to_json<'js>(ctx: &rquickjs::Ctx<'js>, v: rquickjs::Value<'js>) -> J {
    ctx.json_stringify(v)
        .ok()
        .flatten()
        .map_or(J::Null, |s| serde_json::from_str(&s.to_string().unwrap()).unwrap())
}
