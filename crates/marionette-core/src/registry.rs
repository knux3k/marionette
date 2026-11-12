//! The schema registry: every component and resource type the engine knows, by name.
//!
//! Types are described by `bevy_reflect`. The same reflection data drives three things,
//! so they cannot drift apart: JSON Schema output (`schema.rs`), scene reading, and
//! scene writing.

use crate::error::{Diagnostic, codes};
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::resource::Resource;
use bevy_ecs::world::World;
use bevy_reflect::serde::{TypedReflectDeserializer, TypedReflectSerializer};
use bevy_reflect::{
    FromReflect, GetTypeRegistration, PartialReflect, Reflect, TypeInfo, TypePath, TypeRegistry, Typed,
};
use serde::de::DeserializeSeed;
use serde_json::Value;
use std::any::TypeId;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Component,
    Resource,
}

/// Type-erased operations for one registered type.
pub struct TypeEntry {
    /// Name used in scene files and the schema (the short type name, e.g. `Position`).
    pub name: &'static str,
    pub kind: Kind,
    pub type_id: TypeId,
    pub type_path: &'static str,
    pub info: &'static TypeInfo,
    /// Builds the concrete value from (possibly dynamic) reflected data.
    from_reflect: fn(&dyn PartialReflect) -> Option<Box<dyn Reflect>>,
    /// Inserts a concrete value (as returned by `from_reflect`). `entity` is `None` for resources.
    insert: fn(&mut World, Option<Entity>, Box<dyn Reflect>),
    /// Reads the value. `entity` is `None` for resources.
    get: for<'w> fn(&'w World, Option<Entity>) -> Option<&'w dyn Reflect>,
    /// Makes the type known to a world before any value exists (components only), so
    /// queries that mention it optionally work on every world.
    init: fn(&mut World),
}

impl TypeEntry {
    pub fn from_reflect(&self, value: &dyn PartialReflect) -> Option<Box<dyn Reflect>> {
        (self.from_reflect)(value)
    }
    pub fn insert(&self, world: &mut World, entity: Option<Entity>, value: Box<dyn Reflect>) {
        (self.insert)(world, entity, value)
    }
    pub fn get<'w>(&self, world: &'w World, entity: Option<Entity>) -> Option<&'w dyn Reflect> {
        (self.get)(world, entity)
    }
}

#[derive(Default)]
pub struct Registry {
    types: TypeRegistry,
    entries: BTreeMap<&'static str, TypeEntry>,
}

fn boxed_from_reflect<T: FromReflect>(v: &dyn PartialReflect) -> Option<Box<dyn Reflect>> {
    T::from_reflect(v).map(|t| Box::new(t) as Box<dyn Reflect>)
}

fn downcast<T: Reflect>(value: Box<dyn Reflect>) -> T {
    *value
        .into_any()
        .downcast::<T>()
        .expect("TypeEntry::insert called with a value from another type's from_reflect")
}

impl Registry {
    /// An empty registry (no built-ins). See [`crate::default_registry`].
    pub fn new() -> Self {
        Self {
            types: TypeRegistry::new(),
            entries: BTreeMap::new(),
        }
    }

    pub fn register_component<T>(&mut self)
    where
        T: Component + Reflect + FromReflect + Typed + TypePath + GetTypeRegistration,
    {
        self.add::<T>(
            Kind::Component,
            |world| {
                world.register_component::<T>();
            },
            |world, entity, value| {
                let entity = entity.expect("component insert needs an entity");
                world.entity_mut(entity).insert(downcast::<T>(value));
            },
            |world, entity| {
                let entity = entity?;
                world.get::<T>(entity).map(|c| c as &dyn Reflect)
            },
        );
    }

    pub fn register_resource<T>(&mut self)
    where
        T: Resource + Reflect + FromReflect + Typed + TypePath + GetTypeRegistration,
    {
        self.add::<T>(
            Kind::Resource,
            |_| {},
            |world, _, value| world.insert_resource(downcast::<T>(value)),
            |world, _| world.get_resource::<T>().map(|r| r as &dyn Reflect),
        );
    }

    fn add<T>(
        &mut self,
        kind: Kind,
        init: fn(&mut World),
        insert: fn(&mut World, Option<Entity>, Box<dyn Reflect>),
        get: for<'w> fn(&'w World, Option<Entity>) -> Option<&'w dyn Reflect>,
    ) where
        T: Reflect + FromReflect + Typed + TypePath + GetTypeRegistration,
    {
        let name = T::short_type_path();
        if let Some(existing) = self.entries.get(name) {
            // Programmer error at startup, not a data error: fail loudly.
            panic!(
                "{}",
                Diagnostic::error(
                    codes::REGISTRY_DUPLICATE,
                    format!(
                        "type name `{name}` is registered twice ({} and {})",
                        existing.type_path,
                        T::type_path()
                    )
                )
                .fix("rename one of the types; scene files refer to types by short name")
            );
        }
        self.types.register::<T>();
        self.entries.insert(
            name,
            TypeEntry {
                name,
                kind,
                type_id: TypeId::of::<T>(),
                type_path: T::type_path(),
                info: T::type_info(),
                from_reflect: boxed_from_reflect::<T>,
                insert,
                get,
                init,
            },
        );
    }

    /// Registers every component type with `world` (no values are created), so systems
    /// and renderers can build queries over optional components that no entity has yet.
    pub fn init_world(&self, world: &mut World) {
        for e in self.entries.values() {
            (e.init)(world);
        }
    }

    /// Looks a type up by scene name.
    pub fn get(&self, name: &str) -> Option<&TypeEntry> {
        self.entries.get(name)
    }

    /// All entries, sorted by name.
    pub fn entries(&self) -> impl Iterator<Item = &TypeEntry> {
        self.entries.values()
    }

    pub fn of_kind(&self, kind: Kind) -> impl Iterator<Item = &TypeEntry> {
        self.entries.values().filter(move |e| e.kind == kind)
    }

    /// The underlying `bevy_reflect` registry (includes field types).
    pub fn type_registry(&self) -> &TypeRegistry {
        &self.types
    }

    /// Serializes a reflected value of a registered type to JSON.
    ///
    /// The value is written with `serde_json` (which prints `f32` in shortest round-trip
    /// form) and read back as a [`Value`], so `0.1_f32` is stored as `0.1`, not
    /// `0.10000000149011612`.
    pub fn to_json(&self, value: &dyn PartialReflect) -> Value {
        let ser = TypedReflectSerializer::new(value, &self.types);
        let text = serde_json::to_string(&ser).expect("registered types serialize to JSON");
        serde_json::from_str(&text).expect("serde_json output parses")
    }

    /// Deserializes JSON into the concrete value of the registered type `entry`.
    pub fn from_json(&self, entry: &TypeEntry, value: &Value) -> Result<Box<dyn Reflect>, String> {
        let registration = self
            .types
            .get(entry.type_id)
            .expect("entries are registered in the type registry");
        let dynamic = TypedReflectDeserializer::new(registration, &self.types)
            .deserialize(value)
            .map_err(|e| e.to_string())?;
        entry
            .from_reflect(dynamic.as_ref())
            .ok_or_else(|| format!("value does not form a complete `{}`", entry.name))
    }
}
