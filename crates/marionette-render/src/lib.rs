//! Marionette renderer on wgpu: forward PBR for glTF models, batched 2D sprites,
//! offscreen capture to PNG, and an optional window.
//!
//! Rendering reads the ECS world through [`extract::extract`], which takes `&World`, so it
//! cannot change simulation state.

pub use marionette_assets as assets;
pub mod compare;
pub mod cull;
mod environment;
pub mod extract;
pub mod frame_stats;
pub mod gpu;
pub mod particles;
mod pass_timer;
mod post;
mod probes;
mod raytrace;
pub mod renderer;
pub mod scale;
mod skinning;
pub mod window;
mod worldquads;

pub use extract::{EnvironmentData, FrameData, ProbeData, View, extract, extract_at};
pub use gpu::{FrameTiming, Gpu, GpuObjects, Offscreen};
pub use renderer::{MAX_VIEWS, RenderStats, Renderer};
