//! Marionette world-space UI: panels and distance-field text placed in the 3D world.
//!
//! - Scene types ([`components`]): `UiPanel` + `UiContent` (one entity per panel), `UiAnchor`,
//!   `UiFontConfig`.
//! - Input hook ([`runtime::UiPointers`]): any source (XR controllers, a mouse ray, a script)
//!   writes rays or touch points; the panels produce [`runtime::UiEvents`] and change their
//!   own values. Nothing here depends on a particular input source.
//! - Drawing: the module registers a `QuadDrawHook`; the renderer draws the quads with a
//!   distance-field shader, so text is crisp at any size and distance.
//!
//! Add it to an engine with `Engine::builder().module(Ui3dModule)` (or the `ui3d` cargo
//! feature of `marionette-agent`). Games that do not use it pay nothing.

pub mod components;
pub mod font;
pub mod layout;
pub mod module;
pub mod runtime;

pub use components::*;
pub use module::Ui3dModule;
pub use runtime::{PointerKind, UiEvent, UiEventKind, UiEvents, UiPointer, UiPointers, UiRuntime};
