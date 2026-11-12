//! Marionette renderer on wgpu: forward PBR for glTF models, batched 2D sprites,
//! offscreen capture to PNG, and an optional window.
//!
//! Rendering reads the ECS world through [`extract::extract`], which takes `&World`, so it
//! cannot change simulation state.

pub mod assets;
pub mod compare;
pub mod extract;
pub mod gpu;
pub mod renderer;
pub mod window;

pub use extract::{FrameData, View, extract};
pub use gpu::{Gpu, Offscreen};
pub use renderer::Renderer;
