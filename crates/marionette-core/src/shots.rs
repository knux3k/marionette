//! Camera shots during play (decision 0286): game code asks the renderer for a picture from a
//! pose of its choosing, either read back to the CPU (a photo the game saves) or kept on the GPU
//! as an image that sprites and screen UI can show (a live camera screen).
//!
//! The renderer that draws the frame (window, headset or offscreen) takes the requests after
//! the frame's ticks, draws them in the same frame as the main view (sharing its per-frame
//! work), and puts finished read-backs into [`Shots::done`] a frame or more later, without
//! waiting for the GPU. Headless runs draw nothing: requests wait, and [`Shots::ask`] refuses
//! more than [`Shots::LIMIT`] waiting ones, so nothing grows without bound. The resource is not
//! saved or hashed, and reading `done` must never change simulated state that is hashed.

use bevy_ecs::prelude::Resource;

/// One picture asked for. The pose is in world space; the camera looks down its local -Z
/// with +Y up, as an engine `Camera` does.
#[derive(Debug, Clone, PartialEq)]
pub struct ShotRequest {
    /// The caller's number for it, given back in [`ShotDone::id`].
    pub id: u64,
    /// Empty: read the picture back into [`Shots::done`]. Otherwise a camera screen's name:
    /// the picture is shown as the image `shot:<name>` ([`Shots::image_name`]) by sprites and
    /// screen UI nodes, which keep the last picture drawn into it and are black until the
    /// first.
    pub image: String,
    pub position: [f32; 3],
    /// Rotation quaternion (x, y, z, w).
    pub rotation: [f32; 4],
    pub fov_y_degrees: f32,
    pub near: f32,
    pub far: f32,
    /// Pixels, clamped to 16..=[`Shots::MAX_SIDE`] by the renderer. Both 0: the size of the
    /// screen (the window, or one eye of a headset) the frame is drawn at.
    pub width: u32,
    pub height: u32,
}

/// A read-back picture: RGBA8 in sRGB, rows top to bottom, tightly packed
/// (`width * height * 4` bytes). `error` is non-empty when the picture could not be read.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShotDone {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub error: String,
}

/// Shots asked for this frame and shots finished. Insert it to use shots; the renderer
/// ignores worlds without it.
#[derive(Resource, Debug, Default)]
pub struct Shots {
    pub requests: Vec<ShotRequest>,
    pub done: Vec<ShotDone>,
}

impl Shots {
    /// Most requests waiting, and most finished pictures held, at once.
    pub const LIMIT: usize = 8;
    /// Largest side of a picture in pixels.
    pub const MAX_SIDE: u32 = 4096;
    /// Image names of camera screens start with this.
    pub const IMAGE_PREFIX: &'static str = "shot:";

    /// Queues a request; false (and nothing queued) when [`Shots::LIMIT`] are already waiting,
    /// or a read-back would find `done` full.
    pub fn ask(&mut self, request: ShotRequest) -> bool {
        let reads = self.requests.iter().filter(|r| r.image.is_empty()).count();
        if self.requests.len() >= Self::LIMIT
            || (request.image.is_empty() && reads + self.done.len() >= Self::LIMIT)
        {
            return false;
        }
        // One live picture per image a frame: a newer pose replaces the waiting one.
        if !request.image.is_empty()
            && let Some(old) = self.requests.iter_mut().find(|r| r.image == request.image)
        {
            *old = request;
            return true;
        }
        self.requests.push(request);
        true
    }

    /// Takes the finished pictures.
    pub fn take_done(&mut self) -> Vec<ShotDone> {
        std::mem::take(&mut self.done)
    }

    /// The image name a camera screen called `name` is shown under.
    pub fn image_name(name: &str) -> String {
        format!("{}{name}", Self::IMAGE_PREFIX)
    }

    /// Width and height as the renderer will draw them, on a `screen`-sized frame.
    pub fn clamp_size(width: u32, height: u32, screen: (u32, u32)) -> (u32, u32) {
        let (w, h) = if width == 0 && height == 0 {
            screen
        } else {
            (width, height)
        };
        (w.clamp(16, Self::MAX_SIDE), h.clamp(16, Self::MAX_SIDE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(id: u64, image: &str) -> ShotRequest {
        ShotRequest {
            id,
            image: image.into(),
            position: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            fov_y_degrees: 60.0,
            near: 0.05,
            far: 1000.0,
            width: 64,
            height: 36,
        }
    }

    #[test]
    fn requests_are_bounded_and_screens_keep_one_waiting() {
        let mut s = Shots::default();
        for i in 0..Shots::LIMIT as u64 {
            assert!(s.ask(req(i, "")));
        }
        assert!(!s.ask(req(99, "")), "a ninth read-back is refused");
        s.requests.clear();
        assert!(s.ask(req(1, "screen")));
        assert!(s.ask(req(2, "screen")));
        assert_eq!(s.requests.len(), 1);
        assert_eq!(s.requests[0].id, 2, "the newer pose replaces the waiting one");
        s.done = vec![ShotDone::default(); Shots::LIMIT];
        assert!(!s.ask(req(3, "")), "no read-back while done is full");
        assert!(s.ask(req(4, "other")), "screens do not wait on done");
    }

    #[test]
    fn sizes_are_clamped() {
        assert_eq!(Shots::clamp_size(1, 99_999, (640, 360)), (16, Shots::MAX_SIDE));
        assert_eq!(
            Shots::clamp_size(0, 0, (640, 360)),
            (640, 360),
            "0 by 0 is the screen's size"
        );
        assert_eq!(Shots::image_name("cam"), "shot:cam");
    }
}
