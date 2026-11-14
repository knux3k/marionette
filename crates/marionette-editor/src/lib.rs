//! Marionette scene editor view (decision 0130): look at a scene from a free camera, click
//! things to see their components, play, pause and step the simulation, and make small edits
//! (move, duplicate, delete, undo) that go through the same operations as every other scene
//! change.
//!
//! Optional: the CLI and agent include it with the `editor` cargo feature (on by default).
//! Only [`window`] needs a screen; [`view`] renders the same picture offscreen, and every
//! command lives in [`state::Editor`] so it can be tested without a window.

pub mod camera;
pub mod edit;
pub mod inspect;
pub mod overlay;
pub mod pick;
pub mod state;
pub mod view;
pub mod window;

pub use camera::FlyCamera;
pub use state::{Editor, Host};
