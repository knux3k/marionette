//! JSON Schema output. One document describes the scene file format and, under `$defs`,
//! every registered component and resource type. The document is itself a valid JSON
//! Schema (draft 2020-12) for scene files.

use crate::registry::{Kind, Registry};
use crate::scene::{FORMAT, VERSION};
use bevy_reflect::TypeInfo;
use bevy_reflect::enums::VariantInfo;
use serde_json::{Map, Value, json};
use std::any::TypeId;

const ENTITY_DEF: &str = "marionette.Entity";

/// Builds the full schema document.
pub fn schema(registry: &Registry) -> Value {
    let mut defs = Map::new();
    let mut component_props = Map::new();
    let mut resource_props = Map::new();
    let mut component_names = Vec::new();
    let mut resource_names = Vec::new();

    for entry in registry.entries() {
        let mut def = type_schema(entry.info);
        if let Value::Object(obj) = &mut def {
            obj.insert("title".into(), json!(entry.name));
            obj.insert(
                "x-marionette-kind".into(),
                json!(match entry.kind {
                    Kind::Component => "component",
                    Kind::Resource => "resource",
                }),
            );
            obj.insert("x-rust-type".into(), json!(entry.type_path));
        }
        defs.insert(entry.name.into(), def);
        let reference = json!({ "$ref": format!("#/$defs/{}", entry.name) });
        match entry.kind {
            Kind::Component => {
                component_props.insert(entry.name.into(), reference);
                component_names.push(entry.name);
            }
            Kind::Resource => {
                resource_props.insert(entry.name.into(), reference);
                resource_names.push(entry.name);
            }
        }
    }

    defs.insert(
        ENTITY_DEF.into(),
        json!({
            "type": "object",
            "description": "One entity. `id` is stable across saves and is how other entities refer to it.",
            "required": ["id", "components"],
            "additionalProperties": false,
            "properties": {
                "id": { "type": "integer", "minimum": 1, "description": "Stable, unique, non-zero entity id." },
                "name": { "type": "string" },
                "components": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": component_props
                }
            }
        }),
    );

    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": format!("urn:marionette:scene:{VERSION}"),
        "title": format!("Marionette scene (format version {VERSION})"),
        "type": "object",
        "required": ["format", "version", "entities"],
        "additionalProperties": false,
        "properties": {
            "format": { "const": FORMAT },
            "version": { "const": VERSION },
            "resources": {
                "type": "object",
                "additionalProperties": false,
                "properties": resource_props
            },
            "entities": { "type": "array", "items": { "$ref": format!("#/$defs/{ENTITY_DEF}") } }
        },
        "$defs": defs,
        "x-marionette": {
            "engine_version": env!("CARGO_PKG_VERSION"),
            "components": component_names,
            "resources": resource_names
        }
    })
}

fn with_docs(mut schema: Value, docs: Option<&str>) -> Value {
    if let (Value::Object(obj), Some(docs)) = (&mut schema, docs) {
        let docs: Vec<&str> = docs.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        obj.insert("description".into(), json!(docs.join(" ")));
    }
    schema
}

fn opt_schema(info: Option<&'static TypeInfo>) -> Value {
    info.map_or_else(|| json!({}), type_schema)
}

/// JSON Schema for one reflected type, matching how `bevy_reflect`'s serde support
/// writes it.
pub fn type_schema(info: &'static TypeInfo) -> Value {
    match info {
        TypeInfo::Opaque(o) => opaque_schema(o.type_id(), o.type_path()),
        TypeInfo::Struct(s) => {
            let mut props = Map::new();
            let mut required = Vec::new();
            for field in s.iter() {
                props.insert(
                    field.name().into(),
                    with_docs(opt_schema(field.type_info()), field.docs()),
                );
                required.push(field.name());
            }
            with_docs(
                json!({
                    "type": "object",
                    "properties": props,
                    "required": required,
                    "additionalProperties": false
                }),
                s.docs(),
            )
        }
        TypeInfo::TupleStruct(t) if t.field_len() == 1 => {
            // Serialized as a newtype: the bare inner value.
            with_docs(opt_schema(t.field_at(0).and_then(|f| f.type_info())), t.docs())
        }
        TypeInfo::TupleStruct(t) => {
            let items: Vec<Value> = t.iter().map(|f| opt_schema(f.type_info())).collect();
            with_docs(tuple_schema(items), t.docs())
        }
        TypeInfo::Tuple(t) => tuple_schema(t.iter().map(|f| opt_schema(f.type_info())).collect()),
        TypeInfo::List(l) => json!({ "type": "array", "items": opt_schema(l.item_info()) }),
        TypeInfo::Set(_) => {
            json!({ "type": "array", "uniqueItems": true })
        }
        TypeInfo::Array(a) => json!({
            "type": "array",
            "items": opt_schema(a.item_info()),
            "minItems": a.capacity(),
            "maxItems": a.capacity()
        }),
        TypeInfo::Map(m) => {
            json!({ "type": "object", "additionalProperties": opt_schema(m.value_info()) })
        }
        TypeInfo::Enum(e) => {
            let is_option = e.type_path_table().module_path() == Some("core::option")
                && e.type_path_table().ident() == Some("Option");
            if is_option {
                let inner = e.variant("Some").and_then(|v| match v {
                    VariantInfo::Tuple(t) => t.field_at(0).and_then(|f| f.type_info()),
                    _ => None,
                });
                return json!({ "anyOf": [opt_schema(inner), { "type": "null" }] });
            }
            let variants: Vec<Value> = e
                .iter()
                .map(|v| match v {
                    VariantInfo::Unit(u) => json!({ "const": u.name() }),
                    VariantInfo::Struct(s) => {
                        let mut props = Map::new();
                        let mut required = Vec::new();
                        for f in s.iter() {
                            props.insert(f.name().into(), opt_schema(f.type_info()));
                            required.push(f.name());
                        }
                        single_key(
                            s.name(),
                            json!({
                                "type": "object",
                                "properties": props,
                                "required": required,
                                "additionalProperties": false
                            }),
                        )
                    }
                    VariantInfo::Tuple(t) if t.field_len() == 1 => {
                        single_key(t.name(), opt_schema(t.field_at(0).and_then(|f| f.type_info())))
                    }
                    VariantInfo::Tuple(t) => single_key(
                        t.name(),
                        tuple_schema(t.iter().map(|f| opt_schema(f.type_info())).collect()),
                    ),
                })
                .collect();
            with_docs(json!({ "oneOf": variants }), e.docs())
        }
    }
}

fn single_key(key: &str, value: Value) -> Value {
    json!({
        "type": "object",
        "properties": { key: value },
        "required": [key],
        "additionalProperties": false
    })
}

fn tuple_schema(items: Vec<Value>) -> Value {
    let n = items.len();
    json!({ "type": "array", "prefixItems": items, "minItems": n, "maxItems": n })
}

fn opaque_schema(id: TypeId, type_path: &str) -> Value {
    macro_rules! int {
        ($t:ty) => {
            if id == TypeId::of::<$t>() {
                return json!({ "type": "integer", "minimum": <$t>::MIN, "maximum": <$t>::MAX });
            }
        };
    }
    int!(u8);
    int!(u16);
    int!(u32);
    int!(u64);
    int!(i8);
    int!(i16);
    int!(i32);
    int!(i64);
    if id == TypeId::of::<usize>() {
        return json!({ "type": "integer", "minimum": 0 });
    }
    if id == TypeId::of::<isize>() {
        return json!({ "type": "integer" });
    }
    if id == TypeId::of::<f32>() || id == TypeId::of::<f64>() {
        return json!({ "type": "number" });
    }
    if id == TypeId::of::<bool>() {
        return json!({ "type": "boolean" });
    }
    if id == TypeId::of::<String>() {
        return json!({ "type": "string" });
    }
    if id == TypeId::of::<char>() {
        return json!({ "type": "string", "minLength": 1, "maxLength": 1 });
    }
    json!({ "description": format!("opaque type `{type_path}`") })
}
