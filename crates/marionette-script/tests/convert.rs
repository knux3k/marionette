//! Decision 0271: the direct conversions between components and script values give exactly
//! what the JSON text path gives, or decline.

use marionette_core::module::{Modules, registry_with};
use marionette_core::{Kind, Registry};
use marionette_script::{ScriptModule, convert};
use rquickjs::{Context, Ctx, Function, Object, Runtime, Value};
use serde_json::{Value as Json, json};
use std::path::Path;
use std::sync::Arc;

fn registry() -> Registry {
    fn inner() -> Registry {
        marionette_core::default_registry()
    }
    let modules: Modules = Arc::new(vec![Box::new(ScriptModule::new(inner))]);
    registry_with(&modules)
}

/// Strict sameness as a script could tell it: types, `Object.is` on numbers (so -0 and NaN
/// count), array lengths, and object keys in the same order.
const SAME: &str = r#"(function same(a, b) {
  if (typeof a !== typeof b) return false;
  if (a === null || b === null || typeof a !== "object") return Object.is(a, b);
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  if (Object.getPrototypeOf(a) !== Object.getPrototypeOf(b)) return false;
  const ka = Object.keys(a), kb = Object.keys(b);
  if (ka.length !== kb.length) return false;
  for (let i = 0; i < ka.length; i++) {
    if (ka[i] !== kb[i]) return false;
    if (!same(a[ka[i]], b[kb[i]])) return false;
  }
  return true;
})"#;

fn with_js<R>(f: impl FnOnce(Ctx<'_>) -> R) -> R {
    let rt = Runtime::new().unwrap();
    let ctx = Context::full(&rt).unwrap();
    ctx.with(f)
}

fn plain<'js>(ctx: &Ctx<'js>) -> Object<'js> {
    ctx.eval::<Object, _>("Object.prototype").unwrap()
}

/// Every component and resource value found in the repository's scenes that this registry
/// knows, plus values chosen to be awkward.
fn samples(registry: &Registry) -> Vec<(String, Json)> {
    let mut out = Vec::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut dirs = vec![root.join("examples"), root.join("games")];
    while let Some(dir) = dirs.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.to_string_lossy().ends_with(".scene.json") {
                let doc: Json = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
                let entities = doc["entities"].as_array().cloned().unwrap_or_default();
                let parts = entities
                    .iter()
                    .filter_map(|e| e["components"].as_object())
                    .chain(doc["resources"].as_object())
                    .flat_map(|m| m.iter());
                for (name, value) in parts {
                    if registry.get(name).is_some() {
                        out.push((name.clone(), value.clone()));
                    }
                }
            }
        }
    }
    let v = |x: f64, y: f64, z: f64| json!({ "x": x, "y": y, "z": z });
    for (x, y, z) in [
        (0.1, 0.2, 0.3),
        (-0.0, 1e-30, 1e30),
        (16_777_216.0, 16_777_217.0, 33_554_434.0),
        (1.0e-45, 3.4028235e38, -3.4028235e38),
        (0.30000000000000004, 1.0000000596046448, 123456.789),
        (1e-7, 1e21, 2.5),
    ] {
        out.push(("Velocity".into(), v(x, y, z)));
        out.push(("Position".into(), v(z, x, y)));
        out.push((
            "Transform".into(),
            json!({ "translation": v(x, y, z), "rotation": { "x": x, "y": 0, "z": 0, "w": 1 }, "scale": v(1.0, y, 1.0) }),
        ));
    }
    out
}

#[test]
fn reading_a_component_matches_the_text_path() {
    let registry = registry();
    let samples = samples(&registry);
    assert!(
        samples.len() > 100,
        "the repository's scenes were found ({})",
        samples.len()
    );
    let (mut direct, mut kinds) = (0, std::collections::BTreeSet::new());
    with_js(|ctx| {
        let same: Function = ctx.eval(SAME).unwrap();
        for (name, value) in &samples {
            let entry = registry.get(name).unwrap();
            let boxed = registry.from_json(entry, value).unwrap();
            let text = registry.to_json(boxed.as_partial_reflect()).to_string();
            let want: Value = ctx.json_parse(text.clone()).unwrap();
            if let Some(got) = convert::to_js(&ctx, &registry, boxed.as_partial_reflect()) {
                let ok: bool = same.call((got, want)).unwrap();
                assert!(ok, "`{name}` read differently: {text}");
                direct += 1;
                kinds.insert(name.clone());
            }
        }
    });
    assert!(
        direct * 10 >= samples.len() * 9,
        "most values take the direct path ({direct} of {})",
        samples.len()
    );
    assert!(kinds.len() >= 8, "{kinds:?}");
}

/// A small generator (the test must not depend on the machine): xorshift64*.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Doubles of every size, and ones shaped like game values.
    fn double(&mut self) -> f64 {
        let bits = self.next();
        match bits % 5 {
            0 => f64::from_bits(self.next()),
            1 => (self.next() % 2_000_000) as f64 / 1000.0 - 1000.0,
            2 => f64::from(f32::from_bits(self.next() as u32)),
            3 => (self.next() % 100_000) as f64 * (1.0 / 60.0),
            _ => (self.next() as i64 >> (self.next() % 64)) as f64,
        }
    }
}

#[test]
fn numbers_match_the_text_path_both_ways() {
    let registry = registry();
    let entry = registry.get("Velocity").unwrap();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut written, mut read) = (0, 0);
    with_js(|ctx| {
        let plain = plain(&ctx);
        let same: Function = ctx.eval(SAME).unwrap();
        for _ in 0..60_000 {
            let n = rng.double();
            // Script to engine.
            let v = Value::new_float(ctx.clone(), n);
            let text = ctx
                .json_stringify(v.clone())
                .unwrap()
                .unwrap()
                .to_string()
                .unwrap();
            let want: Json = serde_json::from_str(&text).unwrap();
            if let Some(got) = convert::from_js(&v, &plain) {
                assert_eq!(got, want, "{n:e} ({text}) written differently");
                written += 1;
            }
            // Engine to script, as an f32 field.
            let single = n as f32;
            if single.is_finite() {
                let c = registry
                    .from_json(entry, &json!({ "x": 0, "y": 0, "z": 0 }))
                    .map(|mut b| {
                        let bevy_reflect::ReflectMut::Struct(s) = b.reflect_mut() else {
                            panic!("a struct")
                        };
                        *s.field_mut("x").unwrap().try_downcast_mut::<f32>().unwrap() = single;
                        b
                    })
                    .unwrap();
                let text = registry.to_json(c.as_partial_reflect()).to_string();
                let want: Value = ctx.json_parse(text.clone()).unwrap();
                let got = convert::to_js(&ctx, &registry, c.as_partial_reflect()).expect("plain struct");
                let ok: bool = same.call((got, want)).unwrap();
                assert!(ok, "{single:e} read differently: {text}");
                read += 1;
            }
        }
    });
    assert!(written > 45_000 && read > 40_000, "{written} {read}");
}

/// Script values of every kind `JSON.stringify` treats specially.
const VALUES: &[&str] = &[
    "({})",
    "[]",
    "null",
    "true",
    "'text'",
    "'quote \\\" and \\u0000 and \\ud83d\\ude00'",
    "'\\ud800'",
    "0",
    "-0",
    "1.5",
    "NaN",
    "Infinity",
    "9007199254740992",
    "9007199254740994",
    "-9007199254740992",
    "1e21",
    "1.2345678901234567e+30",
    "5e-324",
    "undefined",
    "(() => 1)",
    "Symbol('s')",
    "10n",
    "({ a: 1, b: undefined, c: () => 1, d: Symbol('x'), e: null })",
    "[1, undefined, () => 1, , NaN, -0]",
    "({ z: 1, a: 2, 10: 3, 2: 4 })",
    "({ a: { b: { c: [1, { d: 'x' }] } } })",
    "({ toJSON() { return 5; } })",
    "({ a: { toJSON() { return [1]; } } })",
    "new Date(0)",
    "new Map([[1, 2]])",
    "new Number(5)",
    "new (class P { constructor() { this.x = 1; } })()",
    "Object.create(null)",
    "Object.create({ inherited: 1 })",
    "Object.defineProperty({ a: 1 }, 'hidden', { value: 2, enumerable: false })",
    "({ get g() { return 7; } })",
    "new Float32Array([1, 2])",
    "(() => { const a = { n: 1 }; a.self = a; return a; })()",
    "(() => { const s = { n: 1 }; return { a: s, b: s }; })()",
    "new Proxy({ a: 1 }, {})",
    "({ ['__proto__']: 1 })",
    "(() => { class L extends Array {} const l = new L(); l.push(1, 2); return l; })()",
    "new Error('e')",
];

#[test]
fn writing_a_value_matches_the_text_path_or_declines() {
    let mut direct = 0;
    with_js(|ctx| {
        let plain = plain(&ctx);
        for source in VALUES {
            let v: Value = ctx.eval(*source).unwrap();
            let text = ctx
                .json_stringify(v.clone())
                .ok()
                .flatten()
                .and_then(|s| s.to_string().ok());
            let want: Option<Json> = text.as_deref().and_then(|t| serde_json::from_str(t).ok());
            // Reading an exception clears it for the next value.
            let _ = ctx.catch();
            if let Some(got) = convert::from_js(&v, &plain) {
                assert_eq!(
                    Some(&got),
                    want.as_ref(),
                    "{source} written differently ({text:?})"
                );
                direct += 1;
            }
        }
    });
    assert!(direct >= 20, "the plain values take the direct path ({direct})");
}

/// Objects a script might pass to `Entity.set`, built from each sample.
fn variants(value: &Json) -> Vec<Json> {
    let mut out = vec![value.clone()];
    if let Json::Object(m) = value {
        for key in m.keys() {
            let mut less = m.clone();
            less.remove(key);
            out.push(Json::Object(less));
            for wrong in [
                json!("text"),
                json!(null),
                json!(1.5),
                json!(-3),
                json!(true),
                json!({}),
                json!([1, 2, 3]),
            ] {
                let mut other = m.clone();
                other.insert(key.clone(), wrong);
                out.push(Json::Object(other));
            }
        }
        let mut more = m.clone();
        more.insert("extra".into(), json!(1));
        out.push(Json::Object(more));
    }
    out
}

#[test]
fn patching_a_component_matches_the_text_path_or_declines() {
    let registry = registry();
    let samples = samples(&registry);
    let (mut patched, mut declined) = (0, 0);
    with_js(|ctx| {
        let plain = plain(&ctx);
        for (name, value) in &samples {
            let entry = registry.get(name).unwrap();
            if entry.kind != Kind::Component {
                continue;
            }
            let base = registry.from_json(entry, value).unwrap();
            for candidate in variants(value) {
                let js: Value = ctx.json_parse(candidate.to_string()).unwrap();
                let want = registry.from_json(entry, &candidate).ok();
                let mut copy = entry.from_reflect(base.as_partial_reflect()).unwrap();
                if convert::patch(copy.as_partial_reflect_mut(), &js, &plain, &registry).is_some() {
                    let want = want
                        .unwrap_or_else(|| panic!("`{name}` took {candidate}, which the text path refuses"));
                    assert_eq!(
                        registry.to_json(copy.as_partial_reflect()),
                        registry.to_json(want.as_partial_reflect()),
                        "`{name}` set differently from {candidate}"
                    );
                    assert!(
                        copy.reflect_partial_eq(want.as_partial_reflect())
                            .unwrap_or(false)
                    );
                    patched += 1;
                } else {
                    declined += 1;
                }
            }
        }
    });
    assert!(
        patched > 100 && declined > 100,
        "{patched} patched, {declined} declined"
    );
}

#[test]
fn plans_read_what_the_text_path_reads() {
    let registry = registry();
    let samples = samples(&registry);
    let transform = convert::plan(registry.get("Transform").unwrap().info, registry.type_registry())
        .expect("a struct of number structs has a plan");
    assert_eq!(transform.leaves(), 10);
    assert!(
        convert::plan(registry.get("Script").unwrap().info, registry.type_registry()).is_none(),
        "strings have none"
    );
    let mut made = 0;
    with_js(|ctx| {
        let same: Function = ctx.eval(SAME).unwrap();
        let function: Function = ctx.eval("(body) => new Function('b', body)").unwrap();
        for (name, value) in &samples {
            let entry = registry.get(name).unwrap();
            let Some(plan) = convert::plan(entry.info, registry.type_registry()) else {
                continue;
            };
            let make: Function = function.call((plan.make.as_str(),)).unwrap();
            let base = registry.from_json(entry, value).unwrap();
            let mut leaves = [0.0; convert::PLAN_LEAVES];
            let text = registry.to_json(base.as_partial_reflect()).to_string();
            if plan.read(base.as_partial_reflect(), &mut leaves).is_some() {
                let buf = rquickjs::TypedArray::<f64>::new_copy(ctx.clone(), &leaves[..]).unwrap();
                let got: Value = make.call((buf,)).unwrap();
                let want: Value = ctx.json_parse(text.clone()).unwrap();
                let ok: bool = same.call((got, want)).unwrap();
                assert!(ok, "`{name}` made differently: {text}\n{}", plan.make);
                made += 1;
            }
        }
    });
    assert!(made > 50, "{made} made");
}
