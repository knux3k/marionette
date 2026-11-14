//! Component values to and from JavaScript without JSON text (decision 0271).
//!
//! `Entity.get` used to write a component as JSON text, read that into a JSON value, write
//! that as text again and have the script engine parse it; `Entity.set` went the same way
//! back. Three quarters of a typical script's time went there. The functions here build the
//! JavaScript value straight from the reflected component, and a JSON value straight from
//! the JavaScript value.
//!
//! Both give exactly what the text path gives, including its number rounding (see
//! [`as_json_number`]) and key order. Whatever they are not sure to convert the same way
//! (maps, 128-bit integers, class instances, `toJSON`, huge integers, deep nesting) they
//! decline with `None`, and the caller takes the text path, which also reports every error.
//! `tests/convert.rs` holds the two paths against each other.

use bevy_reflect::serde::{SerializationData, TypedReflectSerializer};
use bevy_reflect::{
    PartialReflect, ReflectDeserialize, ReflectMut, ReflectRef, ReflectSerialize, TypeInfo, TypeRegistry,
};
use marionette_core::Registry;
use rquickjs::{Array, Ctx, Object, Type, Value};
use serde::ser::{self, Serialize};
use serde_json::{Map, Number, Value as Json};
use std::any::TypeId;
use std::fmt;

/// The number a float becomes on the text path: written by `serde_json` in shortest form
/// and read back by `serde_json`, whose reader is not always correctly rounded. Going through
/// the same writer and reader keeps every bit the same. Whole numbers below 2^24 are written
/// and read exactly, so they skip it. `None` for NaN and the infinities (JSON `null`).
fn as_json_number<T: Serialize + Copy + Into<f64>>(v: T) -> Option<f64> {
    let wide: f64 = v.into();
    if !wide.is_finite() {
        return None;
    }
    if wide.fract() == 0.0 && wide.abs() < 16_777_216.0 {
        return Some(wide);
    }
    let mut buf = [0u8; 40];
    let mut rest = &mut buf[..];
    serde_json::to_writer(&mut rest, &v).ok()?;
    let left = rest.len();
    let used = buf.len() - left;
    serde_json::from_slice::<f64>(&buf[..used]).ok()
}

/// Why the direct path stepped aside. Never shown: the text path runs instead.
#[derive(Debug)]
struct Declined;

impl fmt::Display for Declined {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("declined")
    }
}
impl std::error::Error for Declined {}
impl ser::Error for Declined {
    fn custom<T: fmt::Display>(_: T) -> Self {
        Declined
    }
}
impl From<rquickjs::Error> for Declined {
    fn from(_: rquickjs::Error) -> Self {
        Declined
    }
}

type Js<'js> = Result<Value<'js>, Declined>;

#[derive(Clone, Copy)]
struct ToJs<'a, 'js>(&'a Ctx<'js>);

impl<'js> ToJs<'_, 'js> {
    fn number(self, v: f64) -> Value<'js> {
        Value::new_float(self.0.clone(), v)
    }
    fn float<T: Serialize + Copy + Into<f64>>(self, v: T) -> Value<'js> {
        match as_json_number(v) {
            Some(n) => self.number(n),
            None => Value::new_null(self.0.clone()),
        }
    }
    fn string(self, s: &str) -> Js<'js> {
        Ok(rquickjs::String::from_str(self.0.clone(), s)?.into_value())
    }
    /// `{ name: value }`, as JSON writes an enum variant that carries data.
    fn tagged(self, name: &str, value: Value<'js>) -> Js<'js> {
        let o = Object::new(self.0.clone())?;
        put(&o, name, value)?;
        Ok(o.into_value())
    }
}

/// Defines a property. `__proto__` would set the prototype instead, so it is declined.
fn put<'js>(o: &Object<'js>, key: &str, value: Value<'js>) -> Result<(), Declined> {
    if key == "__proto__" {
        return Err(Declined);
    }
    o.set(key, value)?;
    Ok(())
}

struct Seq<'a, 'js> {
    to: ToJs<'a, 'js>,
    items: Array<'js>,
    len: usize,
    variant: Option<&'static str>,
}

impl<'a, 'js> Seq<'a, 'js> {
    fn new(to: ToJs<'a, 'js>, variant: Option<&'static str>) -> Result<Self, Declined> {
        Ok(Self {
            to,
            items: Array::new(to.0.clone())?,
            len: 0,
            variant,
        })
    }
    fn push<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Declined> {
        self.items.set(self.len, value.serialize(self.to)?)?;
        self.len += 1;
        Ok(())
    }
    fn finish(self) -> Js<'js> {
        match self.variant {
            Some(name) => self.to.tagged(name, self.items.into_value()),
            None => Ok(self.items.into_value()),
        }
    }
}

/// A struct's fields. The text path reads the text into a map sorted by key before the
/// script sees it, so the fields are defined in that order here too.
struct Fields<'a, 'js> {
    to: ToJs<'a, 'js>,
    fields: Vec<(&'static str, Value<'js>)>,
    variant: Option<&'static str>,
}

impl<'js> Fields<'_, 'js> {
    fn push<T: ?Sized + Serialize>(&mut self, key: &'static str, value: &T) -> Result<(), Declined> {
        self.fields.push((key, value.serialize(self.to)?));
        Ok(())
    }
    fn finish(mut self) -> Js<'js> {
        self.fields.sort_by(|a, b| a.0.cmp(b.0));
        if self.fields.windows(2).any(|w| w[0].0 == w[1].0) {
            return Err(Declined);
        }
        let o = Object::new(self.to.0.clone())?;
        for (key, value) in self.fields {
            put(&o, key, value)?;
        }
        match self.variant {
            Some(name) => self.to.tagged(name, o.into_value()),
            None => Ok(o.into_value()),
        }
    }
}

macro_rules! whole {
    ($($method:ident: $t:ty),*) => {
        $(fn $method(self, v: $t) -> Js<'js> {
            // JSON text holds the integer exactly; the script engine reads it as the
            // nearest double, which is what `as f64` gives.
            Ok(self.number(v as f64))
        })*
    };
}

impl<'a, 'js> ser::Serializer for ToJs<'a, 'js> {
    type Ok = Value<'js>;
    type Error = Declined;
    type SerializeSeq = Seq<'a, 'js>;
    type SerializeTuple = Seq<'a, 'js>;
    type SerializeTupleStruct = Seq<'a, 'js>;
    type SerializeTupleVariant = Seq<'a, 'js>;
    type SerializeMap = ser::Impossible<Value<'js>, Declined>;
    type SerializeStruct = Fields<'a, 'js>;
    type SerializeStructVariant = Fields<'a, 'js>;

    whole!(serialize_i8: i8, serialize_i16: i16, serialize_i32: i32, serialize_i64: i64,
           serialize_u8: u8, serialize_u16: u16, serialize_u32: u32, serialize_u64: u64);

    fn serialize_bool(self, v: bool) -> Js<'js> {
        Ok(Value::new_bool(self.0.clone(), v))
    }
    fn serialize_f32(self, v: f32) -> Js<'js> {
        Ok(self.float(v))
    }
    fn serialize_f64(self, v: f64) -> Js<'js> {
        Ok(self.float(v))
    }
    fn serialize_char(self, v: char) -> Js<'js> {
        self.string(v.encode_utf8(&mut [0; 4]))
    }
    fn serialize_str(self, v: &str) -> Js<'js> {
        self.string(v)
    }
    fn serialize_bytes(self, _: &[u8]) -> Js<'js> {
        Err(Declined)
    }
    fn serialize_none(self) -> Js<'js> {
        self.serialize_unit()
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Js<'js> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Js<'js> {
        Ok(Value::new_null(self.0.clone()))
    }
    fn serialize_unit_struct(self, _: &'static str) -> Js<'js> {
        self.serialize_unit()
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, variant: &'static str) -> Js<'js> {
        self.string(variant)
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(self, _: &'static str, value: &T) -> Js<'js> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        value: &T,
    ) -> Js<'js> {
        let inner = value.serialize(self)?;
        self.tagged(variant, inner)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Seq<'a, 'js>, Declined> {
        Seq::new(self, None)
    }
    fn serialize_tuple(self, _: usize) -> Result<Seq<'a, 'js>, Declined> {
        Seq::new(self, None)
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Seq<'a, 'js>, Declined> {
        Seq::new(self, None)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Seq<'a, 'js>, Declined> {
        Seq::new(self, Some(variant))
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Declined> {
        // Map keys have their own text rules; the text path knows them.
        Err(Declined)
    }
    fn serialize_struct(self, _: &'static str, len: usize) -> Result<Fields<'a, 'js>, Declined> {
        Ok(Fields {
            to: self,
            fields: Vec::with_capacity(len),
            variant: None,
        })
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Fields<'a, 'js>, Declined> {
        Ok(Fields {
            to: self,
            fields: Vec::with_capacity(len),
            variant: Some(variant),
        })
    }
}

macro_rules! seq_impl {
    ($($tr:ident: $method:ident),*) => {
        $(impl<'js> ser::$tr for Seq<'_, 'js> {
            type Ok = Value<'js>;
            type Error = Declined;
            fn $method<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Declined> {
                self.push(value)
            }
            fn end(self) -> Js<'js> {
                self.finish()
            }
        })*
    };
}
seq_impl!(SerializeSeq: serialize_element, SerializeTuple: serialize_element,
          SerializeTupleStruct: serialize_field, SerializeTupleVariant: serialize_field);

macro_rules! fields_impl {
    ($($tr:ident),*) => {
        $(impl<'js> ser::$tr for Fields<'_, 'js> {
            type Ok = Value<'js>;
            type Error = Declined;
            fn serialize_field<T: ?Sized + Serialize>(&mut self, key: &'static str, value: &T) -> Result<(), Declined> {
                self.push(key, value)
            }
            fn end(self) -> Js<'js> {
                self.finish()
            }
        })*
    };
}
fields_impl!(SerializeStruct, SerializeStructVariant);

/// A reflected value as the JavaScript value `JSON.parse` of its JSON text would give.
/// `None`: take the text path.
pub fn to_js<'js>(ctx: &Ctx<'js>, registry: &Registry, value: &dyn PartialReflect) -> Option<Value<'js>> {
    TypedReflectSerializer::new(value, registry.type_registry())
        .serialize(ToJs(ctx))
        .ok()
}

/// Nesting the direct path follows before it leaves the value (a cycle, most likely) to
/// `JSON.stringify`, which reports it.
const MAX_DEPTH: u32 = 32;

/// A script number as `serde_json` reads its `JSON.stringify` text.
fn number(n: f64) -> Option<Json> {
    if !n.is_finite() {
        return Some(Json::Null);
    }
    if n.fract() == 0.0 {
        // Up to 2^53 a whole number is printed digit for digit and read as an integer.
        // Past that the printed digits are rounded and the reader's integer differs: declined.
        const EXACT: f64 = 9_007_199_254_740_992.0;
        return if (0.0..=EXACT).contains(&n) {
            Some(Json::Number(Number::from(n as u64)))
        } else if (-EXACT..0.0).contains(&n) {
            Some(Json::Number(Number::from(n as i64)))
        } else {
            None
        };
    }
    // Shortest digits and exponent are the same whoever prints them, and the reader's
    // result depends on those only.
    as_json_number(n).and_then(Number::from_f64).map(Json::Number)
}

fn has_to_json(o: &Object<'_>) -> bool {
    !matches!(
        o.get::<_, Value>("toJSON").map(|v| v.type_of()),
        Ok(Type::Undefined)
    )
}

/// A script value as the JSON value `JSON.stringify` and a JSON reader would give.
/// `None`: take the text path (also for every value `JSON.stringify` refuses or leaves out
/// at the top). `plain` is `Object.prototype`.
pub fn from_js<'js>(value: &Value<'js>, plain: &Object<'js>) -> Option<Json> {
    convert(value, plain, 0)
}

/// What an array element or the top value becomes; object members that return `Err(())`
/// here are left out by `JSON.stringify`.
fn convert<'js>(value: &Value<'js>, plain: &Object<'js>, depth: u32) -> Option<Json> {
    match member(value, plain, depth)? {
        Ok(v) => Some(v),
        // Left out of an object, `null` in an array; at the top there is no text at all.
        Err(()) if depth == 0 => None,
        Err(()) => Some(Json::Null),
    }
}

fn member<'js>(value: &Value<'js>, plain: &Object<'js>, depth: u32) -> Option<Result<Json, ()>> {
    if depth > MAX_DEPTH {
        return None;
    }
    Some(Ok(match value.type_of() {
        Type::Undefined | Type::Uninitialized | Type::Function | Type::Constructor | Type::Symbol => {
            return Some(Err(()));
        }
        Type::Null => Json::Null,
        Type::Bool => Json::Bool(value.as_bool()?),
        Type::Int | Type::Float => number(value.as_number()?)?,
        Type::String => Json::String(value.as_string()?.to_string().ok()?),
        Type::Array => {
            let a = value.as_array()?;
            if has_to_json(a.as_object()) {
                return None;
            }
            let mut out = Vec::with_capacity(a.len());
            for i in 0..a.len() {
                out.push(convert(&a.get::<Value>(i).ok()?, plain, depth + 1)?);
            }
            Json::Array(out)
        }
        Type::Object => {
            let o = value.as_object()?;
            if o.get_prototype().as_ref() != Some(plain) || has_to_json(o) {
                return None;
            }
            let mut out = Map::new();
            for key in o.keys::<String>() {
                let key = key.ok()?;
                let v = o.get::<_, Value>(key.as_str()).ok()?;
                if let Ok(v) = member(&v, plain, depth + 1)? {
                    out.insert(key, v);
                }
            }
            Json::Object(out)
        }
        // Big integers, promises, modules, exceptions, proxies and anything newer.
        _ => return None,
    }))
}

/// Writes a script object over `target`, field by field, without building a JSON value
/// (decision 0271). For the plain case only: structs of structs whose leaves are numbers,
/// booleans and strings, with every field given and nothing else. `None` means declined;
/// `target` may then be half written and must be thrown away.
///
/// The result is what the JSON path builds: the caller starts from `FromReflect` of the
/// current value, so fields reflection does not see hold their defaults, as they would
/// there, and each leaf takes its number the way [`from_js`] reads it.
pub fn patch<'js>(
    target: &mut dyn PartialReflect,
    value: &Value<'js>,
    plain: &Object<'js>,
    registry: &Registry,
) -> Option<()> {
    patch_at(target, value, plain, registry.type_registry(), 0)
}

fn patch_at<'js>(
    target: &mut dyn PartialReflect,
    value: &Value<'js>,
    plain: &Object<'js>,
    types: &TypeRegistry,
    depth: u32,
) -> Option<()> {
    if depth > MAX_DEPTH {
        return None;
    }
    if leaf(target, value)? {
        return Some(());
    }
    let id = target.get_represented_type_info()?.type_id();
    let ReflectMut::Struct(s) = target.reflect_mut() else {
        return None;
    };
    plain_struct(id, s.field_len(), types)?;
    if value.type_of() != Type::Object {
        return None;
    }
    let o = value.as_object()?;
    if o.get_prototype().as_ref() != Some(plain) || has_to_json(o) {
        return None;
    }
    let mut given = 0;
    for key in o.keys::<String>() {
        let key = key.ok()?;
        let v = o.get::<_, Value>(key.as_str()).ok()?;
        // A member JSON leaves out is a missing field there: declined.
        patch_at(s.field_mut(&key)?, &v, plain, types, depth + 1)?;
        given += 1;
    }
    (given == s.field_len()).then_some(())
}

/// Sets a number leaf from a script number, read as [`number`] reads it. `Some(false)`:
/// `target` is not a number.
fn set_number(target: &mut dyn PartialReflect, value: f64) -> Option<bool> {
    let n = || match number(value)? {
        Json::Number(n) => Some(n),
        _ => None,
    };
    macro_rules! whole {
        ($($t:ty),*) => {
            $(if let Some(slot) = target.try_downcast_mut::<$t>() {
                let n = n()?;
                *slot = match (n.as_u64(), n.as_i64()) {
                    (Some(u), _) => <$t>::try_from(u).ok()?,
                    (None, Some(i)) => <$t>::try_from(i).ok()?,
                    _ => return None,
                };
                return Some(true);
            })*
        };
    }
    if let Some(slot) = target.try_downcast_mut::<f32>() {
        let n = n()?;
        *slot = match (n.as_u64(), n.as_i64()) {
            (Some(u), _) => u as f32,
            (None, Some(i)) => i as f32,
            _ => n.as_f64()? as f32,
        };
        return Some(true);
    }
    if let Some(slot) = target.try_downcast_mut::<f64>() {
        let n = n()?;
        *slot = match (n.as_u64(), n.as_i64()) {
            (Some(u), _) => u as f64,
            (None, Some(i)) => i as f64,
            _ => n.as_f64()?,
        };
        return Some(true);
    }
    whole!(u8, u16, u32, u64, i8, i16, i32, i64);
    Some(false)
}

/// Sets a number, boolean or string leaf. `Some(false)`: `target` is not a leaf.
fn leaf(target: &mut dyn PartialReflect, value: &Value<'_>) -> Option<bool> {
    if let Some(slot) = target.try_downcast_mut::<bool>() {
        *slot = value.as_bool()?;
        return Some(true);
    }
    if let Some(slot) = target.try_downcast_mut::<String>() {
        *slot = value.as_string()?.to_string().ok()?;
        return Some(true);
    }
    if !is_number(target) {
        return Some(false);
    }
    set_number(target, value.as_number()?)
}

fn is_number(target: &dyn PartialReflect) -> bool {
    target
        .get_represented_type_info()
        .is_some_and(|i| kind_of(i.type_id()).is_some_and(|k| k != LeafKind::Bool))
}

/// A struct whose fields are the whole of its JSON form: no JSON form of its own and no
/// field left out of it.
fn plain_struct(id: TypeId, fields: usize, types: &TypeRegistry) -> Option<()> {
    let registration = types.get(id)?;
    if registration.data::<ReflectDeserialize>().is_some()
        || registration.data::<ReflectSerialize>().is_some()
    {
        return None;
    }
    if let Some(skips) = registration.data::<SerializationData>()
        && (0..fields).any(|i| skips.is_field_skipped(i))
    {
        return None;
    }
    Some(())
}

/// A number or boolean leaf type.
#[derive(Debug, Clone, Copy, PartialEq)]
enum LeafKind {
    F32,
    F64,
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    Bool,
}

fn kind_of(id: TypeId) -> Option<LeafKind> {
    macro_rules! kinds {
        ($($t:ty => $k:ident),*) => {
            $(if id == TypeId::of::<$t>() { return Some(LeafKind::$k); })*
        };
    }
    kinds!(f32 => F32, f64 => F64, u8 => U8, u16 => U16, u32 => U32, u64 => U64,
           i8 => I8, i16 => I16, i32 => I32, i64 => I64, bool => Bool);
    None
}

/// Leaves a [`Plan`] can carry (the size of the shared number buffer).
pub const PLAN_LEAVES: usize = 64;

/// How one component type is read by scripts with no per-call lookups by name (decision
/// 0271): its number and boolean leaves in a fixed order (fields sorted by name at every
/// level, as the JSON path orders them) and a JavaScript function body, `make`, that builds
/// the object from the leaves in a `Float64Array` `b`. An object literal is the cheapest way
/// to make an object in the script engine.
///
/// Only for structs of structs with number and boolean leaves; everything else has no plan
/// and takes [`to_js`]. Writing stays with [`patch`]: checking a script's object in generated
/// JavaScript measured slower than doing it here.
pub struct Plan {
    leaves: Vec<(Vec<usize>, LeafKind)>,
    pub make: String,
}

pub fn plan(info: &'static TypeInfo, types: &TypeRegistry) -> Option<Plan> {
    let mut plan = Plan {
        leaves: Vec::new(),
        make: String::from("return "),
    };
    build(info, types, &mut Vec::new(), &mut plan)?;
    plan.make.push(';');
    Some(plan)
}

fn build(
    info: &'static TypeInfo,
    types: &TypeRegistry,
    path: &mut Vec<usize>,
    plan: &mut Plan,
) -> Option<()> {
    let TypeInfo::Struct(s) = info else {
        return None;
    };
    if path.len() > 8 {
        return None;
    }
    plain_struct(info.type_id(), s.field_len(), types)?;
    let mut fields: Vec<(usize, &'static str, &bevy_reflect::NamedField)> =
        s.iter().enumerate().map(|(i, f)| (i, f.name(), f)).collect();
    fields.sort_by(|a, b| a.1.cmp(b.1));
    if fields.iter().any(|f| f.1 == "__proto__") || fields.windows(2).any(|w| w[0].1 == w[1].1) {
        return None;
    }
    plan.make.push('{');
    for (n, (index, name, field)) in fields.iter().enumerate() {
        if n > 0 {
            plan.make.push(',');
        }
        // A JSON string is a JavaScript string literal.
        plan.make
            .push_str(&serde_json::to_string(name).expect("a string is JSON"));
        plan.make.push(':');
        path.push(*index);
        if let Some(kind) = kind_of(field.type_id()) {
            let slot = plan.leaves.len();
            if slot >= PLAN_LEAVES {
                return None;
            }
            plan.leaves.push((path.clone(), kind));
            plan.make.push_str(&format!("b[{slot}]"));
            if kind == LeafKind::Bool {
                plan.make.push_str("!==0");
            }
        } else {
            build(field.type_info()?, types, path, plan)?;
        }
        path.pop();
    }
    plan.make.push('}');
    Some(())
}

impl Plan {
    pub fn leaves(&self) -> usize {
        self.leaves.len()
    }

    /// Writes the leaves of `value` into `out` as the numbers a script sees. `None` when a
    /// float is not finite (JSON `null`): the general path handles it.
    pub fn read(&self, value: &dyn PartialReflect, out: &mut [f64]) -> Option<()> {
        for ((path, kind), slot) in self.leaves.iter().zip(out) {
            let mut v = value;
            for &i in path {
                let ReflectRef::Struct(s) = v.reflect_ref() else {
                    return None;
                };
                v = s.field_at(i)?;
            }
            macro_rules! whole {
                ($t:ty) => {
                    *v.try_downcast_ref::<$t>()? as f64
                };
            }
            *slot = match kind {
                LeafKind::F32 => as_json_number(*v.try_downcast_ref::<f32>()?)?,
                LeafKind::F64 => as_json_number(*v.try_downcast_ref::<f64>()?)?,
                LeafKind::U8 => whole!(u8),
                LeafKind::U16 => whole!(u16),
                LeafKind::U32 => whole!(u32),
                LeafKind::U64 => whole!(u64),
                LeafKind::I8 => whole!(i8),
                LeafKind::I16 => whole!(i16),
                LeafKind::I32 => whole!(i32),
                LeafKind::I64 => whole!(i64),
                LeafKind::Bool => f64::from(u8::from(*v.try_downcast_ref::<bool>()?)),
            };
        }
        Some(())
    }
}
