//! Uploading images a game makes in code (decision 0184).
//!
//! A `game:<name>` image name resolves to the frame's `GameImages` entry. The first draw that
//! needs it creates one GPU texture; every later version of the same size is written into
//! that texture, so the bind groups that point at it stay valid and nothing is allocated per
//! frame. A new size, a changed `nearest` flag or a removed image drops the GPU copy and every
//! cached bind group that used it; the next draw makes them again.

use super::Renderer;
use crate::extract::FrameData;
use marionette_core::image::{GameImage, PREFIX};
use std::sync::Arc;

/// The GPU copy of one game image.
pub(crate) struct LiveImage {
    version: u64,
    width: u32,
    height: u32,
    nearest: bool,
    texture: wgpu::Texture,
}

/// A view of a game image: (view, width, height, sample nearest).
pub(crate) type ImageView = (wgpu::TextureView, u32, u32, bool);

impl Renderer {
    /// Keeps the frame's images, writes changed ones into their textures and drops the GPU
    /// copies of images that were removed or changed size.
    pub(super) fn sync_game_images(&mut self, frame: &FrameData) {
        self.game_images = Arc::clone(&frame.game_images);
        let mut stale = Vec::new();
        for (name, live) in &mut self.live_images {
            match self.game_images.get(name) {
                Some(e) if e.version == live.version => {}
                Some(e)
                    if e.image.width == live.width
                        && e.image.height == live.height
                        && e.image.nearest == live.nearest =>
                {
                    write(&self.queue, &live.texture, &e.image);
                    live.version = e.version;
                }
                _ => stale.push(name.clone()),
            }
        }
        for name in stale {
            self.drop_game_image(&name);
        }
        // A name that failed (not stored yet) is tried again once the game stores it.
        let images = &self.game_images;
        let stored = |k: &String| k.strip_prefix(PREFIX).is_some_and(|n| images.contains_key(n));
        self.sprite_textures.retain(|k, v| v.is_ok() || !stored(k));
        if let Some(g) = self.game.as_mut() {
            g.textures.retain(|k, v| v.is_ok() || !stored(k));
        }
    }

    fn drop_game_image(&mut self, name: &str) {
        self.live_images.remove(name);
        let full = format!("{PREFIX}{name}");
        self.sprite_textures.remove(&full);
        if let Some(g) = self.game.as_mut() {
            g.textures.remove(&full);
            // Material keys end with `#tex|tex|`: drop the groups that bound this image.
            g.groups.retain(|key, _| {
                key.rsplit_once('#')
                    .is_none_or(|(_, textures)| !textures.split('|').any(|t| t == full))
            });
        }
    }

    /// A view of `image` when it names a game image (`None` for every other name), making
    /// its texture on first use.
    pub(super) fn game_image_view(&mut self, image: &str) -> Option<Result<ImageView, String>> {
        let name = image.strip_prefix(PREFIX)?;
        let Some(entry) = self.game_images.get(name) else {
            return Some(Err(format!(
                "no game image named `{name}` (store one with GameImages::set)"
            )));
        };
        let img = &entry.image;
        if let Some(live) = self.live_images.get(name) {
            let view = live.texture.create_view(&wgpu::TextureViewDescriptor::default());
            return Some(Ok((view, live.width, live.height, live.nearest)));
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("game image"),
            size: extent(img),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        write(&self.queue, &texture, img);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let found = (view, img.width, img.height, img.nearest);
        self.live_images.insert(
            name.to_string(),
            LiveImage {
                version: entry.version,
                width: img.width,
                height: img.height,
                nearest: img.nearest,
                texture,
            },
        );
        Some(Ok(found))
    }

    /// Game images with a GPU copy (tests and the asset count).
    pub fn live_game_images(&self) -> usize {
        self.live_images.len()
    }
}

fn extent(img: &GameImage) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: img.width,
        height: img.height,
        depth_or_array_layers: 1,
    }
}

fn write(queue: &wgpu::Queue, texture: &wgpu::Texture, img: &GameImage) {
    queue.write_texture(
        texture.as_image_copy(),
        &img.rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * img.width),
            rows_per_image: Some(img.height),
        },
        extent(img),
    );
}
