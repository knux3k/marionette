//! Images made by game code that may change every frame (decision 0184): a screen inside the
//! world, a minimap, a painted canvas. Drawn wherever an image name goes (`Sprite::image`,
//! `UiImage`, world quads, `ShaderMaterial::textures`) as `game:<name>`.
//!
//! Like [`crate::mesh::GameMeshes`], the store is versioned: the renderer uploads an image when
//! the version it holds differs, into the same GPU texture when the size is unchanged (no new
//! allocation, no new bind groups), and frees it with the next frame after [`GameImages::remove`].
//! Render data: not in scenes or the state hash.

use std::collections::BTreeMap;
use std::sync::Arc;

/// Largest side of a game image in pixels (the smallest texture limit wgpu guarantees).
pub const MAX_SIDE: u32 = 2048;

/// RGBA8 pixels, sRGB, row 0 at the top.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GameImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Sample the nearest texel (pixel art) instead of blending; sprites, UI and world quads
    /// honour it. A `ShaderMaterial` shares one sampler, so pixel-exact shaders read the
    /// image with `textureLoad`.
    pub nearest: bool,
}

impl GameImage {
    /// Checks the size and the pixel count.
    pub fn validate(&self) -> Result<(), String> {
        if self.width == 0 || self.height == 0 || self.width > MAX_SIDE || self.height > MAX_SIDE {
            return Err(format!(
                "a game image is {}x{}; each side must be 1 to {MAX_SIDE}",
                self.width, self.height
            ));
        }
        let want = self.width as usize * self.height as usize * 4;
        if self.rgba.len() != want {
            return Err(format!(
                "a {}x{} game image needs {want} bytes of RGBA, got {}",
                self.width,
                self.height,
                self.rgba.len()
            ));
        }
        Ok(())
    }
}

/// A stored image and the version it was stored at.
#[derive(Debug, Clone)]
pub struct ImageEntry {
    pub version: u64,
    pub image: Arc<GameImage>,
}

/// The current game images. Cloning is one reference count.
pub type ImageMap = Arc<BTreeMap<String, ImageEntry>>;

/// Prefix of a game image's name where an image name is expected.
pub const PREFIX: &str = "game:";

/// Game images by name (decision 0184).
#[derive(bevy_ecs::prelude::Resource, Debug, Clone, Default)]
pub struct GameImages {
    map: ImageMap,
    next_version: u64,
}

impl GameImages {
    /// Stores `image` under `name` (replacing any earlier one), after checking it.
    pub fn set(&mut self, name: &str, image: GameImage) -> Result<(), String> {
        if name.is_empty() {
            return Err("a game image needs a name".into());
        }
        image.validate()?;
        self.next_version += 1;
        let entry = ImageEntry {
            version: self.next_version,
            image: Arc::new(image),
        };
        Arc::make_mut(&mut self.map).insert(name.to_string(), entry);
        Ok(())
    }

    /// Replaces the pixels of an image already stored at the same size, reusing its buffer
    /// when no frame still holds the old copy (the common case: one write per frame).
    pub fn update(&mut self, name: &str, write: impl FnOnce(&mut [u8])) -> Result<(), String> {
        self.next_version += 1;
        let version = self.next_version;
        let entry = Arc::make_mut(&mut self.map)
            .get_mut(name)
            .ok_or_else(|| format!("no game image named `{name}` (store one with GameImages::set)"))?;
        write(&mut Arc::make_mut(&mut entry.image).rgba);
        entry.version = version;
        Ok(())
    }

    /// Removes an image; its GPU copy is freed with the next frame. Returns whether it existed.
    pub fn remove(&mut self, name: &str) -> bool {
        self.map.contains_key(name) && Arc::make_mut(&mut self.map).remove(name).is_some()
    }

    pub fn get(&self, name: &str) -> Option<&Arc<GameImage>> {
        self.map.get(name).map(|e| &e.image)
    }

    /// The version an image was last stored at; it grows on every `set` and `update`.
    pub fn version(&self, name: &str) -> Option<u64> {
        self.map.get(name).map(|e| e.version)
    }

    /// Names in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.map.keys().map(String::as_str)
    }

    /// A cheap copy of every image for one frame.
    pub fn snapshot(&self) -> ImageMap {
        Arc::clone(&self.map)
    }

    /// Convenience for modules: stores into the world's resource, creating it.
    pub fn set_in(world: &mut bevy_ecs::world::World, name: &str, image: GameImage) -> Result<(), String> {
        world
            .get_resource_or_insert_with(GameImages::default)
            .set(name, image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32) -> GameImage {
        GameImage {
            width: w,
            height: h,
            rgba: vec![255; (w * h * 4) as usize],
            nearest: true,
        }
    }

    #[test]
    fn versions_grow_and_frames_keep_their_copy() {
        let mut g = GameImages::default();
        g.set("screen", img(4, 2)).unwrap();
        let v1 = g.version("screen").unwrap();
        let held = g.snapshot();
        g.update("screen", |px| px[0] = 7).unwrap();
        assert!(g.version("screen").unwrap() > v1);
        assert_eq!(g.get("screen").unwrap().rgba[0], 7);
        assert_eq!(held["screen"].image.rgba[0], 255);
        assert!(g.update("nope", |_| ()).is_err());
        assert!(
            g.set(
                "bad",
                GameImage {
                    rgba: vec![0; 3],
                    ..img(1, 1)
                }
            )
            .is_err()
        );
        assert!(g.set("huge", img(MAX_SIDE + 1, 1)).is_err());
        assert!(g.remove("screen") && !g.remove("screen"));
    }
}
