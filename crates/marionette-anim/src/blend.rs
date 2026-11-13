//! Blend spaces and overlays (decision 0074).
//!
//! - A [`BlendSpace`] mixes clips by one or two parameters (for example idle, walk and run by
//!   speed, or eight directions by velocity x and z). An `Animator` layer whose `clip` is a
//!   space name plays the space. Weights come from gradient band interpolation, the method
//!   behind freeform 2D blend trees in other engines; with all points on a line it is plain
//!   linear interpolation between neighbours. A synced space plays every clip at the same
//!   phase, so feet stay matched while walk turns into run.
//! - An [`AnimOverlay`] is a layer over the base pose: masked to some bones (with their
//!   children) and either overriding them or adding to them (additive, relative to each
//!   clip's first frame), e.g. a wave on the upper body while the legs walk.
//!
//! Plain `+ - * /` and `sqrt` only, like `sample.rs`.

use crate::AnimLayer;
use crate::sample::{LocalPose, blend, normalize_quat};
use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use marionette_assets::rig::{Clip, Property, Rig};

/// One clip of a blend space, placed at parameter values (`x`, `y`). For a 1D space leave
/// `y` at 0.
#[derive(Reflect, Debug, Clone, PartialEq)]
pub struct BlendPoint {
    pub clip: String,
    pub x: f32,
    pub y: f32,
}

/// Clips mixed by a parameter pair. Play it with `anim_play` using its `name` as the clip.
#[derive(Reflect, Debug, Clone, PartialEq)]
pub struct BlendSpace {
    pub name: String,
    pub points: Vec<BlendPoint>,
    /// Parameter now.
    pub x: f32,
    pub y: f32,
    /// Parameter the space moves toward.
    pub target_x: f32,
    pub target_y: f32,
    /// Parameter units per second toward the target (0 = jump straight there).
    pub rate: f32,
    /// Play every clip at the same phase (walk/run cycles keep their footfalls together). The
    /// layer's `time` is then the phase, 0 to 1.
    pub sync: bool,
}

/// Blend spaces of an entity (used by its `Animator` and `AnimOverlays` layers).
#[derive(Component, Reflect, Debug, Clone, Default, PartialEq)]
pub struct BlendSpaces {
    pub spaces: Vec<BlendSpace>,
}

impl BlendSpaces {
    pub fn get(&self, name: &str) -> Option<&BlendSpace> {
        self.spaces.iter().find(|s| s.name == name)
    }

    /// Adds or replaces the space `name`'s points and sync flag (parameter kept when it exists).
    pub fn define(&mut self, name: &str, points: Vec<BlendPoint>, sync: bool) -> &mut BlendSpace {
        let i = match self.spaces.iter().position(|s| s.name == name) {
            Some(i) => i,
            None => {
                self.spaces.push(BlendSpace {
                    name: name.to_string(),
                    points: Vec::new(),
                    x: 0.0,
                    y: 0.0,
                    target_x: 0.0,
                    target_y: 0.0,
                    rate: 0.0,
                    sync,
                });
                self.spaces.len() - 1
            }
        };
        let s = &mut self.spaces[i];
        s.points = points;
        s.sync = sync;
        s
    }

    /// Sets where space `name` goes; with `rate` 0 (or `None` keeping the space's rate at 0)
    /// it jumps there now. Returns false when there is no such space.
    pub fn set_param(&mut self, name: &str, x: f32, y: f32, rate: Option<f32>) -> bool {
        let Some(s) = self.spaces.iter_mut().find(|s| s.name == name) else {
            return false;
        };
        s.target_x = x;
        s.target_y = y;
        if let Some(r) = rate {
            s.rate = r.max(0.0);
        }
        if s.rate <= 0.0 {
            s.x = x;
            s.y = y;
        }
        true
    }

    /// Moves every parameter toward its target by `rate * dt`.
    pub fn advance(&mut self, dt: f32) {
        for s in &mut self.spaces {
            if s.rate <= 0.0 {
                s.x = s.target_x;
                s.y = s.target_y;
                continue;
            }
            let (dx, dy) = (s.target_x - s.x, s.target_y - s.y);
            let dist = (dx * dx + dy * dy).sqrt();
            let step = s.rate * dt;
            if dist <= step || dist < 1e-6 {
                s.x = s.target_x;
                s.y = s.target_y;
            } else {
                s.x += dx * step / dist;
                s.y += dy * step / dist;
            }
        }
    }
}

/// Weight of each point at parameter `p` (gradient band interpolation, cartesian). Weights
/// sum to 1; outside the points the nearest edge wins.
pub fn point_weights(points: &[[f32; 2]], p: [f32; 2]) -> Vec<f32> {
    let n = points.len();
    if n <= 1 {
        return vec![1.0; n];
    }
    let mut w: Vec<f32> = (0..n)
        .map(|i| {
            let pi = points[i];
            let mut h = f32::MAX;
            for (j, pj) in points.iter().enumerate() {
                if i == j {
                    continue;
                }
                let e = [pj[0] - pi[0], pj[1] - pi[1]];
                let len2 = e[0] * e[0] + e[1] * e[1];
                if len2 <= 0.0 {
                    continue;
                }
                let t = 1.0 - ((p[0] - pi[0]) * e[0] + (p[1] - pi[1]) * e[1]) / len2;
                h = h.min(t);
            }
            if h == f32::MAX { 1.0 } else { h.max(0.0) }
        })
        .collect();
    let total: f32 = w.iter().sum();
    if total > 0.0 {
        for v in &mut w {
            *v /= total;
        }
    } else {
        w = vec![1.0 / n as f32; n];
    }
    w
}

/// What a layer's `clip` names: a model clip or a blend space.
pub enum Source<'a> {
    Clip(&'a Clip),
    Space(&'a BlendSpace, Vec<(&'a Clip, f32)>),
}

/// Resolves a layer clip name: blend spaces first, then the model's clips. `None` when the
/// name is unknown or a space names a clip the model lacks.
pub fn resolve<'a>(rig: &'a Rig, spaces: Option<&'a BlendSpaces>, name: &str) -> Option<Source<'a>> {
    if let Some(space) = spaces.and_then(|s| s.get(name)) {
        let clips: Option<Vec<&Clip>> = space.points.iter().map(|p| rig.clip(&p.clip)).collect();
        let clips = clips?;
        let at: Vec<[f32; 2]> = space.points.iter().map(|p| [p.x, p.y]).collect();
        let w = point_weights(&at, [space.x, space.y]);
        return Some(Source::Space(space, clips.into_iter().zip(w).collect()));
    }
    rig.clip(name).map(Source::Clip)
}

/// The first clip a name needs that the model lacks (the name itself, or a space's point).
pub fn missing_clip(rig: &Rig, spaces: Option<&BlendSpaces>, name: &str) -> Option<String> {
    if let Some(space) = spaces.and_then(|s| s.get(name)) {
        return space
            .points
            .iter()
            .find(|p| rig.clip(&p.clip).is_none())
            .map(|p| p.clip.clone());
    }
    rig.clip(name).is_none().then(|| name.to_string())
}

/// (duration the layer's time wraps at, time units per second) of a resolved layer source.
/// Synced spaces run in phase (0..1) at `1 / blended duration`.
pub fn timing(source: &Source) -> (f32, f32) {
    match source {
        Source::Clip(c) => (c.duration, 1.0),
        Source::Space(space, clips) => {
            if space.sync {
                let d: f32 = clips.iter().map(|(c, w)| c.duration * w).sum();
                if d > 0.0 { (1.0, 1.0 / d) } else { (0.0, 1.0) }
            } else {
                (clips.iter().map(|(c, _)| c.duration).fold(0.0, f32::max), 1.0)
            }
        }
    }
}

/// Expands layers into (clip, time, weight) triples for `sample::blend`.
pub fn expand<'a>(
    rig: &'a Rig,
    spaces: Option<&'a BlendSpaces>,
    layers: &[AnimLayer],
    at_start: bool,
) -> Vec<(&'a Clip, f32, f32)> {
    let mut out = Vec::new();
    for l in layers {
        if l.weight <= 0.0 {
            continue;
        }
        match resolve(rig, spaces, &l.clip) {
            Some(Source::Clip(c)) => out.push((c, if at_start { 0.0 } else { l.time }, l.weight)),
            Some(Source::Space(space, clips)) => {
                for (c, w) in clips {
                    if w <= 0.0 {
                        continue;
                    }
                    let t = if at_start {
                        0.0
                    } else if space.sync {
                        l.time * c.duration
                    } else if l.looping && c.duration > 0.0 {
                        l.time.rem_euclid(c.duration)
                    } else {
                        l.time.min(c.duration)
                    };
                    out.push((c, t, l.weight * w));
                }
            }
            None => {}
        }
    }
    out
}

/// A layer over the base pose, limited to `mask` bones.
#[derive(Reflect, Debug, Clone, PartialEq)]
pub struct AnimOverlay {
    pub name: String,
    /// Bone (node) names; each includes its children. Empty = the whole model.
    pub mask: Vec<String>,
    /// Adds the clips' motion (relative to their first frame) instead of replacing bones.
    pub additive: bool,
    /// Strength now (0..1).
    pub weight: f32,
    pub target_weight: f32,
    /// Seconds for a full 0 to 1 strength change.
    pub fade: f32,
    /// Clips (or blend spaces) of this overlay, cross-faded like `Animator` layers.
    pub layers: Vec<AnimLayer>,
}

/// Overlays applied in order on top of the entity's `Animator` pose.
#[derive(Component, Reflect, Debug, Clone, Default, PartialEq)]
pub struct AnimOverlays {
    pub overlays: Vec<AnimOverlay>,
}

impl AnimOverlays {
    /// The overlay `name`, added (empty, full strength, whole model, override) if missing.
    pub fn entry(&mut self, name: &str) -> &mut AnimOverlay {
        let i = match self.overlays.iter().position(|o| o.name == name) {
            Some(i) => i,
            None => {
                self.overlays.push(AnimOverlay {
                    name: name.to_string(),
                    mask: Vec::new(),
                    additive: false,
                    weight: 0.0,
                    target_weight: 1.0,
                    fade: 0.0,
                    layers: Vec::new(),
                });
                self.overlays.len() - 1
            }
        };
        &mut self.overlays[i]
    }
}

impl AnimOverlay {
    /// Fades the overlay's strength to `weight` over `fade` seconds (0 = now).
    pub fn set_weight(&mut self, weight: f32, fade: f32) {
        self.target_weight = weight.clamp(0.0, 1.0);
        self.fade = fade.max(0.0);
        if self.fade == 0.0 {
            self.weight = self.target_weight;
        }
    }
}

/// Per node: inside the mask (the node or an ancestor is named) and moved by `clips`.
pub fn node_mask(rig: &Rig, mask: &[String], clips: &[(&Clip, f32, f32)]) -> Vec<bool> {
    let n = rig.nodes.len();
    let mut animated = vec![false; n];
    for (c, _, _) in clips {
        for ch in &c.channels {
            if ch.node < n {
                animated[ch.node] = true;
            }
        }
    }
    let named: Vec<bool> = rig.nodes.iter().map(|nd| mask.contains(&nd.name)).collect();
    (0..n)
        .map(|i| {
            if !animated[i] {
                return false;
            }
            if mask.is_empty() {
                return true;
            }
            let mut p = Some(i);
            let mut guard = 0;
            while let Some(q) = p {
                if named[q] {
                    return true;
                }
                p = rig.nodes[q].parent;
                guard += 1;
                if guard > n {
                    break;
                }
            }
            false
        })
        .collect()
}

fn qmul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

fn conj(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

/// Normalized linear interpolation from `a` to `b` the short way round.
pub fn nlerp(a: [f32; 4], b: [f32; 4], w: f32) -> [f32; 4] {
    let d = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    let s = if d < 0.0 { -1.0 } else { 1.0 };
    normalize_quat([
        a[0] + (b[0] * s - a[0]) * w,
        a[1] + (b[1] * s - a[1]) * w,
        a[2] + (b[2] * s - a[2]) * w,
        a[3] + (b[3] * s - a[3]) * w,
    ])
}

/// Applies one overlay to `base` at strength `w`.
pub fn apply_overlay(
    rig: &Rig,
    base: &mut LocalPose,
    clips: &[(&Clip, f32, f32)],
    reference: Option<&[(&Clip, f32, f32)]>,
    mask: &[String],
    w: f32,
) {
    if w <= 0.0 || clips.is_empty() {
        return;
    }
    let over = blend(rig, clips);
    let reference = reference.map(|r| blend(rig, r));
    let inside = node_mask(rig, mask, clips);
    let morphed: Vec<bool> = {
        let mut m = vec![false; rig.nodes.len()];
        for (c, _, _) in clips {
            for ch in c.channels.iter().filter(|ch| ch.property == Property::Weights) {
                if ch.node < m.len() {
                    m[ch.node] = inside[ch.node];
                }
            }
        }
        m
    };
    for i in 0..rig.nodes.len().min(base.translation.len()) {
        if !inside[i] {
            continue;
        }
        match &reference {
            None => {
                for k in 0..3 {
                    base.translation[i][k] += (over.translation[i][k] - base.translation[i][k]) * w;
                    base.scale[i][k] += (over.scale[i][k] - base.scale[i][k]) * w;
                }
                base.rotation[i] = nlerp(base.rotation[i], over.rotation[i], w);
            }
            Some(r) => {
                for k in 0..3 {
                    base.translation[i][k] += (over.translation[i][k] - r.translation[i][k]) * w;
                    if r.scale[i][k] != 0.0 {
                        base.scale[i][k] *= 1.0 + (over.scale[i][k] / r.scale[i][k] - 1.0) * w;
                    }
                }
                let delta = qmul(conj(r.rotation[i]), over.rotation[i]);
                let delta = nlerp([0.0, 0.0, 0.0, 1.0], delta, w);
                base.rotation[i] = normalize_quat(qmul(base.rotation[i], delta));
            }
        }
        if morphed[i] {
            let slot = &mut base.morph_weights[i];
            let o = &over.morph_weights[i];
            if slot.len() < o.len() {
                slot.resize(o.len(), 0.0);
            }
            for k in 0..o.len() {
                let target = match &reference {
                    None => o[k],
                    Some(r) => slot[k] + o[k] - r.morph_weights[i].get(k).copied().unwrap_or(0.0),
                };
                slot[k] += (target - slot[k]) * w;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn one_dimension_is_linear_between_neighbours() {
        let pts = [[0.0, 0.0], [1.5, 0.0], [4.0, 0.0]];
        assert!(close(&point_weights(&pts, [0.0, 0.0]), &[1.0, 0.0, 0.0]));
        assert!(close(&point_weights(&pts, [0.75, 0.0]), &[0.5, 0.5, 0.0]));
        assert!(close(&point_weights(&pts, [3.375, 0.0]), &[0.0, 0.25, 0.75]));
        // Outside the range the edge point holds.
        assert!(close(&point_weights(&pts, [9.0, 0.0]), &[0.0, 0.0, 1.0]));
        assert!(close(&point_weights(&pts, [-1.0, 0.0]), &[1.0, 0.0, 0.0]));
    }

    #[test]
    fn two_dimensions_hit_points_and_sum_to_one() {
        let pts = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [-1.0, 0.0], [0.0, -1.0]];
        for (i, p) in pts.iter().enumerate() {
            let w = point_weights(&pts, *p);
            assert!((w[i] - 1.0).abs() < 1e-5, "{w:?}");
        }
        let w = point_weights(&pts, [0.5, 0.5]);
        assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        assert!(w[1] > 0.2 && w[2] > 0.2 && (w[1] - w[2]).abs() < 1e-5, "{w:?}");
        assert_eq!(w[3], 0.0);
        assert_eq!(w[4], 0.0);
    }

    #[test]
    fn parameters_move_at_their_rate() {
        let mut s = BlendSpaces::default();
        s.define("move", vec![], true);
        s.set_param("move", 3.0, 4.0, Some(10.0));
        assert_eq!((s.spaces[0].x, s.spaces[0].y), (0.0, 0.0));
        s.advance(0.25);
        assert!(close(&[s.spaces[0].x, s.spaces[0].y], &[1.5, 2.0]));
        s.advance(1.0);
        assert_eq!((s.spaces[0].x, s.spaces[0].y), (3.0, 4.0));
        assert!(s.set_param("move", 1.0, 0.0, Some(0.0)));
        assert_eq!(s.spaces[0].x, 1.0);
        assert!(!s.set_param("none", 1.0, 0.0, None));
    }

    #[test]
    fn quaternion_helpers() {
        let q = normalize_quat([0.0, 0.3, 0.0, 0.9]);
        let id = qmul(conj(q), q);
        assert!(close(&id, &[0.0, 0.0, 0.0, 1.0]));
        assert!(close(&nlerp(q, q, 0.4), &q));
        assert!(close(&nlerp([0.0, 0.0, 0.0, 1.0], q, 1.0), &q));
    }
}
