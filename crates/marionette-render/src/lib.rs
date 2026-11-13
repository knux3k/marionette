//! Marionette renderer on wgpu: forward PBR for glTF models, batched 2D sprites,
//! offscreen capture to PNG, and an optional window.
//!
//! Rendering reads the ECS world through [`extract::extract`], which takes `&World`, so it
//! cannot change simulation state.

pub use marionette_assets as assets;
pub mod compare;
mod environment;
pub mod extract;
pub mod gpu;
mod post;
pub mod renderer;
mod skinning;
pub mod window;
mod worldquads;

pub use extract::{EnvironmentData, FrameData, View, extract, extract_at};
pub use gpu::{FrameTiming, Gpu, GpuObjects, Offscreen};
pub use renderer::{MAX_VIEWS, RenderStats, Renderer};
