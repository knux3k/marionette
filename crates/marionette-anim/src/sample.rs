//! Sampling and blending clips into a pose. Plain `+ - * /` and `sqrt` only (rotations use
//! normalized linear interpolation, not `slerp`), so every platform computes the same pose.

use marionette_assets::rig::{Channel, Clip, Interpolation, Property, Rig, mul, trs};

/// Local (parent-relative) transform of every node, plus morph weights per node.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalPose {
    pub translation: Vec<[f32; 3]>,
    pub rotation: Vec<[f32; 4]>,
    pub scale: Vec<[f32; 3]>,
    pub morph_weights: Vec<Vec<f32>>,
}

impl LocalPose {
    /// The rest pose of `rig`.
    pub fn rest(rig: &Rig) -> Self {
        Self {
            translation: rig.nodes.iter().map(|n| n.translation).collect(),
            rotation: rig.nodes.iter().map(|n| n.rotation).collect(),
            scale: rig.nodes.iter().map(|n| n.scale).collect(),
            morph_weights: rig.morph_weights.clone(),
        }
    }

    /// Model-space matrix of every node (column-major).
    pub fn model_space(&self, rig: &Rig) -> Vec<[f32; 16]> {
        let local: Vec<[f32; 16]> = (0..self.translation.len())
            .map(|i| trs(self.translation[i], self.rotation[i], self.scale[i]))
            .collect();
        let mut out = local.clone();
        for i in rig.order() {
            if let Some(p) = rig.nodes[i].parent {
                out[i] = mul(&out[p], &local[i]);
            }
        }
        out
    }
}

/// Where `t` falls among `times`: (key before, key after, fraction between them).
fn locate(times: &[f32], t: f32) -> (usize, usize, f32) {
    let last = times.len() - 1;
    if t <= times[0] {
        return (0, 0, 0.0);
    }
    if t >= times[last] {
        return (last, last, 0.0);
    }
    let i = times.partition_point(|&x| x <= t) - 1;
    let span = times[i + 1] - times[i];
    let f = if span > 0.0 { (t - times[i]) / span } else { 0.0 };
    (i, i + 1, f)
}

/// Unit quaternion of `q` (identity if it is zero).
pub fn normalize_quat(q: [f32; 4]) -> [f32; 4] {
    let len2 = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
    if len2 > 0.0 {
        let inv = 1.0 / len2.sqrt();
        q.map(|c| c * inv)
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

fn dot4(a: [f32; 4], b: [f32; 4]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

/// Samples one channel at time `t` into `out` (`width` floats).
fn sample_channel(ch: &Channel, t: f32, out: &mut [f32]) {
    let w = ch.width;
    let (i0, i1, f) = locate(&ch.times, t);
    match ch.interpolation {
        Interpolation::Step => out.copy_from_slice(&ch.values[i0 * w..i0 * w + w]),
        Interpolation::Linear => {
            let a = &ch.values[i0 * w..i0 * w + w];
            let b = &ch.values[i1 * w..i1 * w + w];
            // Rotations take the short way round.
            let sign = if ch.property == Property::Rotation {
                let d: f32 = (0..4).map(|k| a[k] * b[k]).sum();
                if d < 0.0 { -1.0 } else { 1.0 }
            } else {
                1.0
            };
            for k in 0..w {
                out[k] = a[k] + (b[k] * sign - a[k]) * f;
            }
        }
        Interpolation::CubicSpline => {
            // Per key: in-tangent, value, out-tangent (each `w` floats).
            let key = |i: usize, part: usize| &ch.values[i * 3 * w + part * w..i * 3 * w + part * w + w];
            if i0 == i1 {
                out.copy_from_slice(key(i0, 1));
            } else {
                let dt = ch.times[i1] - ch.times[i0];
                let (p0, m0, p1, m1) = (key(i0, 1), key(i0, 2), key(i1, 1), key(i1, 0));
                let (f2, f3) = (f * f, f * f * f);
                let h00 = 2.0 * f3 - 3.0 * f2 + 1.0;
                let h10 = f3 - 2.0 * f2 + f;
                let h01 = -2.0 * f3 + 3.0 * f2;
                let h11 = f3 - f2;
                for k in 0..w {
                    out[k] = h00 * p0[k] + h10 * dt * m0[k] + h01 * p1[k] + h11 * dt * m1[k];
                }
            }
        }
    }
}

/// Writes `clip` at time `t` over `pose` (nodes the clip does not animate keep their value).
pub fn sample_clip(clip: &Clip, t: f32, pose: &mut LocalPose) {
    let mut buf = [0.0f32; 4];
    for ch in &clip.channels {
        if ch.node >= pose.translation.len() {
            continue;
        }
        match ch.property {
            Property::Translation => {
                sample_channel(ch, t, &mut buf[..3]);
                pose.translation[ch.node] = [buf[0], buf[1], buf[2]];
            }
            Property::Scale => {
                sample_channel(ch, t, &mut buf[..3]);
                pose.scale[ch.node] = [buf[0], buf[1], buf[2]];
            }
            Property::Rotation => {
                sample_channel(ch, t, &mut buf);
                pose.rotation[ch.node] = normalize_quat(buf);
            }
            Property::Weights => {
                let mut w = vec![0.0; ch.width];
                sample_channel(ch, t, &mut w);
                let slot = &mut pose.morph_weights[ch.node];
                if slot.len() < w.len() {
                    slot.resize(w.len(), 0.0);
                }
                slot[..w.len()].copy_from_slice(&w);
            }
        }
    }
}

/// Blends clips sampled at their times by weight. Weights are normalized; with no weight
/// at all the rest pose is returned. Rotations are blended by weighted sum on one
/// hemisphere, then normalized.
pub fn blend(rig: &Rig, layers: &[(&Clip, f32, f32)]) -> LocalPose {
    let rest = LocalPose::rest(rig);
    let total: f32 = layers.iter().map(|l| l.2.max(0.0)).sum();
    if total <= 0.0 {
        return rest;
    }
    let active: Vec<&(&Clip, f32, f32)> = layers.iter().filter(|l| l.2 > 0.0).collect();
    if active.len() == 1 {
        let mut pose = rest;
        sample_clip(active[0].0, active[0].1, &mut pose);
        return pose;
    }
    let n = rig.nodes.len();
    let mut acc = LocalPose {
        translation: vec![[0.0; 3]; n],
        rotation: vec![[0.0; 4]; n],
        scale: vec![[0.0; 3]; n],
        morph_weights: rest.morph_weights.iter().map(|w| vec![0.0; w.len()]).collect(),
    };
    for (clip, t, w) in active {
        let w = w / total;
        let mut pose = rest.clone();
        sample_clip(clip, *t, &mut pose);
        for i in 0..n {
            for k in 0..3 {
                acc.translation[i][k] += pose.translation[i][k] * w;
                acc.scale[i][k] += pose.scale[i][k] * w;
            }
            let q = pose.rotation[i];
            let s = if dot4(acc.rotation[i], q) < 0.0 { -w } else { w };
            for k in 0..4 {
                acc.rotation[i][k] += q[k] * s;
            }
            let mw = &pose.morph_weights[i];
            let slot = &mut acc.morph_weights[i];
            if slot.len() < mw.len() {
                slot.resize(mw.len(), 0.0);
            }
            for (k, v) in mw.iter().enumerate() {
                slot[k] += v * w;
            }
        }
    }
    for r in &mut acc.rotation {
        *r = normalize_quat(*r);
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(property: Property, interpolation: Interpolation, times: &[f32], values: &[f32]) -> Channel {
        let width = match property {
            Property::Rotation => 4,
            _ => 3,
        };
        Channel {
            node: 0,
            property,
            interpolation,
            times: times.to_vec(),
            values: values.to_vec(),
            width,
        }
    }

    #[test]
    fn linear_step_and_clamping() {
        let ch = channel(
            Property::Translation,
            Interpolation::Linear,
            &[0.0, 1.0],
            &[0.0, 0.0, 0.0, 2.0, 4.0, 6.0],
        );
        let mut out = [0.0; 3];
        sample_channel(&ch, 0.25, &mut out);
        assert_eq!(out, [0.5, 1.0, 1.5]);
        sample_channel(&ch, -1.0, &mut out);
        assert_eq!(out, [0.0, 0.0, 0.0]);
        sample_channel(&ch, 9.0, &mut out);
        assert_eq!(out, [2.0, 4.0, 6.0]);
        let step = Channel {
            interpolation: Interpolation::Step,
            ..ch
        };
        sample_channel(&step, 0.99, &mut out);
        assert_eq!(out, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn rotation_takes_the_short_way() {
        // q and -q are the same rotation; halfway must not pass through zero.
        let ch = channel(
            Property::Rotation,
            Interpolation::Linear,
            &[0.0, 1.0],
            &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -1.0],
        );
        let mut out = [0.0; 4];
        sample_channel(&ch, 0.5, &mut out);
        assert_eq!(normalize_quat(out), [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn cubic_spline_hits_keys_and_is_smooth() {
        // Zero tangents: an ease-in-out between 0 and 1.
        let ch = Channel {
            node: 0,
            property: Property::Translation,
            interpolation: Interpolation::CubicSpline,
            times: vec![0.0, 2.0],
            values: [[0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3], [1.0; 3], [0.0; 3]].concat(),
            width: 3,
        };
        let mut out = [0.0; 3];
        sample_channel(&ch, 1.0, &mut out);
        assert_eq!(out, [0.5; 3]);
        sample_channel(&ch, 0.5, &mut out);
        assert!((out[0] - 0.15625).abs() < 1e-6);
        sample_channel(&ch, 2.0, &mut out);
        assert_eq!(out, [1.0; 3]);
    }
}
