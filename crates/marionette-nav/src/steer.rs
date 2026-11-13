//! Crowd steering (decision 0232): follow the route, slow down on arrival, and avoid other
//! agents by predicting the closest approach of each pair (a light velocity-obstacle rule)
//! plus a push apart when they overlap. Neighbours come from a sorted spatial hash, agents
//! are processed in chunks on the job pool, and each agent only writes its own result, so
//! the thread count never changes the outcome.

use marionette_core::vmath::*;

/// One agent as steering sees it (surface-local space).
#[derive(Clone, Copy, Debug)]
pub struct Body {
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub radius: f32,
    pub surface: u64,
}

/// What an agent wants this tick.
#[derive(Clone, Copy, Debug)]
pub struct Want {
    /// Where it is heading (surface-local), or `None` to stand.
    pub target: Option<[f32; 3]>,
    /// Slow down near the target (it is the goal, not a waypoint).
    pub last: bool,
    pub speed: f32,
    pub accel: f32,
}

/// Agents sorted by spatial bucket, with an open-addressing table from bucket to its run in
/// the sorted list (lookups only, never iterated, so it cannot change any order).
/// A table slot: surface, bucket, start and end in `order`.
type Slot = Option<(u64, [i32; 3], u32, u32)>;

pub struct Buckets {
    size: f32,
    /// Agent indices, sorted by (surface, bucket, index).
    order: Vec<u32>,
    /// Table slots: (surface, bucket, start, end) or empty.
    slots: Vec<Slot>,
    mask: usize,
}

fn hash(surface: u64, k: [i32; 3]) -> usize {
    let mut h = surface.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    for v in k {
        h = (h ^ (v as u32 as u64)).wrapping_mul(0x0000_0100_0000_01B3);
    }
    (h ^ (h >> 29)) as usize
}

impl Buckets {
    pub fn new(bodies: &[Body], size: f32) -> Self {
        let size = size.max(1e-3);
        let mut keys: Vec<(u64, [i32; 3], u32)> = bodies
            .iter()
            .enumerate()
            .map(|(i, b)| (b.surface, key(b.pos, size), i as u32))
            .collect();
        keys.sort_unstable();
        let cap = (keys.len() * 2).next_power_of_two().max(16);
        let mask = cap - 1;
        let mut slots: Vec<Slot> = vec![None; cap];
        let mut at = 0;
        while at < keys.len() {
            let (sf, k, _) = keys[at];
            let mut end = at + 1;
            while end < keys.len() && keys[end].0 == sf && keys[end].1 == k {
                end += 1;
            }
            let mut h = hash(sf, k) & mask;
            while slots[h].is_some() {
                h = (h + 1) & mask;
            }
            slots[h] = Some((sf, k, at as u32, end as u32));
            at = end;
        }
        Self {
            size,
            order: keys.into_iter().map(|e| e.2).collect(),
            slots,
            mask,
        }
    }

    fn run(&self, surface: u64, k: [i32; 3]) -> &[u32] {
        let mut h = hash(surface, k) & self.mask;
        while let Some((sf, kk, a, b)) = self.slots[h] {
            if sf == surface && kk == k {
                return &self.order[a as usize..b as usize];
            }
            h = (h + 1) & self.mask;
        }
        &[]
    }

    /// Calls `f(index)` for agents in the 27 buckets around `p` on `surface`, in a fixed order.
    pub fn around(&self, surface: u64, p: [f32; 3], mut f: impl FnMut(u32)) {
        let k = key(p, self.size);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &i in self.run(surface, [k[0] + dx, k[1] + dy, k[2] + dz]) {
                        f(i);
                    }
                }
            }
        }
    }
}

fn key(p: [f32; 3], size: f32) -> [i32; 3] {
    [
        (p[0] / size).floor() as i32,
        (p[1] / size).floor() as i32,
        (p[2] / size).floor() as i32,
    ]
}

/// Tunables for steering.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub neighbour_distance: f32,
    pub horizon: f32,
    pub max_neighbours: u32,
    pub dt: f32,
}

/// The new velocity of agent `i` (tangent to the surface at its position).
pub fn velocity(i: usize, bodies: &[Body], wants: &[Want], buckets: &Buckets, p: &Params) -> [f32; 3] {
    let me = bodies[i];
    let w = wants[i];
    let up = norm(me.pos).unwrap_or([0.0, 1.0, 0.0]);
    let tangent = |v: [f32; 3]| sub(v, scale(up, dot(v, up)));
    let mut desired = [0.0; 3];
    if let Some(t) = w.target {
        let to = tangent(sub(t, me.pos));
        let d = len(to);
        if d > 1e-4 {
            let mut speed = w.speed;
            if w.last {
                // Arrive: v² = 2·a·d keeps stopping distance within reach.
                let stop = (2.0 * w.accel.max(0.1) * d).sqrt();
                speed = speed.min(stop);
            }
            desired = scale(to, speed / d);
        }
    }
    let mut push = [0.0f32; 3];
    let mut seen = 0;
    let reach2 = p.neighbour_distance * p.neighbour_distance;
    buckets.around(me.surface, me.pos, |j| {
        let j = j as usize;
        if j == i || seen >= p.max_neighbours {
            return;
        }
        let o = bodies[j];
        let rel = sub(o.pos, me.pos);
        let d2 = dot(rel, rel);
        if d2 > reach2 {
            return;
        }
        seen += 1;
        let r = me.radius + o.radius;
        let d = d2.sqrt();
        if d < r {
            // Overlap: separate, half each.
            let away = if d > 1e-5 {
                scale(rel, -1.0 / d)
            } else if i < j {
                [1.0, 0.0, 0.0]
            } else {
                [-1.0, 0.0, 0.0]
            };
            push = add(push, scale(away, (r - d) / p.dt * 0.5));
            return;
        }
        let rv = sub(desired, o.vel);
        let rv2 = dot(rv, rv);
        if rv2 < 1e-6 {
            return;
        }
        let t = dot(rel, rv) / rv2;
        if t <= 0.0 || t > p.horizon {
            return;
        }
        let closest = sub(rel, scale(rv, t));
        let c = len(closest);
        if c >= r {
            return;
        }
        // Sidestep away from the closest point; dodge harder the sooner and deeper.
        let side = if c > 1e-5 {
            scale(closest, -1.0 / c)
        } else {
            norm(cross(up, rv)).unwrap_or([1.0, 0.0, 0.0])
        };
        let strength = (r - c) / t.max(p.dt);
        push = add(push, scale(side, strength * 0.5));
    });
    let mut v = tangent(add(desired, push));
    let cap = w.speed * 1.2;
    let l = len(v);
    if l > cap && l > 0.0 {
        v = scale(v, cap / l);
    }
    // Accelerate toward it.
    let dv = sub(v, tangent(me.vel));
    let dl = len(dv);
    let max = w.accel * p.dt;
    if dl > max && dl > 0.0 {
        add(tangent(me.vel), scale(dv, max / dl))
    } else {
        v
    }
}
