# 0001: ECS is `bevy_ecs` standalone, schema from `bevy_reflect`

Date: 2026-11-11. Status: accepted.

## Context

DESIGN.md prefers `bevy_ecs` used standalone if it builds cleanly and its reflection can feed
the schema; otherwise `hecs` plus a hand-written registry.

## Decision

Use `bevy_ecs` 0.20.1 and `bevy_reflect` 0.20.1 with default features off
(`bevy_ecs`: `std`, `bevy_reflect`; `bevy_reflect`: `std`, `reflect_documentation`).

## Why

- It built cleanly on Windows (MSVC, Rust 1.99.1) with no extra system dependencies.
- `bevy_reflect`'s `TypeInfo` describes every field, so one walker (`schema.rs`) emits JSON
  Schema, and doc comments become schema `description`s.
- The same reflection data drives scene reading (`TypedReflectDeserializer`) and writing
  (`TypedReflectSerializer`), so the schema, the loader and the saver cannot drift apart.
  With `hecs` we would maintain that by hand for every type.
- Systems, schedules and queries come for free and are well documented.

## Consequences

- Our `Registry` stores small type-erased function pointers per type (insert, get,
  from_reflect) instead of using `ReflectComponent`/`ReflectResource`. In 0.20 resources are
  stored as components, and plain generic functions keep the registry uniform and short.
- Scene types are registered by short type name; registering two types with the same short
  name panics at startup.
- `multi_threaded` is off and the simulation schedule uses `SingleThreadedExecutor` for
  determinism. Parallelism can come later for systems proven independent.
- Bevy releases break APIs often. Pin exact minor versions and upgrade deliberately.
