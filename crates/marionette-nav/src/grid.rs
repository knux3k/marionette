//! The cube-sphere cell grid (decision 0231): a sphere cut into six faces of `n × n` cells,
//! each cell with a ground height, a walk weight and up to eight neighbours. All in the
//! surface's local space (unit directions from the centre), so a moving or turning world
//! keeps its grid.
//!
//! Cells are spaced by a warped cube projection (tangent warp from a rational
//! approximation: cell edges differ by at most 1.8× instead of about 3× for a plain cube map),
//! computed with `+ - * /` and `sqrt` only, so every platform builds the same grid.

use marionette_core::vmath::*;

/// No neighbour in this slot.
pub const NONE: u32 = u32::MAX;
/// Cells per tile side; tiles are the unit of change tracking.
pub const TILE: u32 = 16;

/// The eight neighbour offsets (di, dj), in slot order.
pub const OFFSETS: [(i32, i32); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// Face normal, u axis, v axis.
const FACES: [[[f32; 3]; 3]; 6] = [
    [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]],
    [[-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
    [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]],
    [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
    [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    [[0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
];

/// tan(π/4 · x) for x in [-1, 1] from the [5/4] Padé approximant (error below 1e-6).
fn warp(x: f32) -> f32 {
    let y = x * std::f32::consts::FRAC_PI_4;
    let y2 = y * y;
    y * (945.0 - 105.0 * y2 + y2 * y2) / (945.0 - 420.0 * y2 + 15.0 * y2 * y2)
}

/// A cube-sphere grid.
#[derive(Clone, Debug)]
pub struct Grid {
    /// Cells per face edge.
    pub n: u32,
    /// Base radius in metres (edge costs are measured at this radius).
    pub radius: f32,
    /// Warped cell boundaries on a face axis, `n + 1` values from -1 to 1.
    bounds: Vec<f32>,
    /// Unit direction of each cell centre.
    pub centre: Vec<[f32; 3]>,
    /// Neighbours per cell (`NONE` where a slot is empty, at cube corners).
    pub nbr: Vec<[u32; 8]>,
    /// Edge length per neighbour slot in millimetres at the base radius.
    pub len_mm: Vec<[u32; 8]>,
    /// Baked walk weight: 0 blocked, 1 normal, up to 255 (dearer).
    pub base: Vec<u8>,
    /// Ground distance from the centre in metres.
    pub height: Vec<f32>,
    /// Obstacles covering each cell (blocked while above 0).
    pub blocks: Vec<u16>,
    /// Smallest cell edge as an angle (radians), for line-of-sight steps.
    pub min_step: f32,
}

impl Grid {
    /// Builds the geometry of a grid (all cells walkable at `radius`).
    pub fn new(n: u32, radius: f32) -> Self {
        let n = n.clamp(4, 1024);
        let w1 = warp(1.0);
        let mut bounds: Vec<f32> = (0..=n)
            .map(|k| warp(2.0 * k as f32 / n as f32 - 1.0) / w1)
            .collect();
        bounds[0] = -1.0;
        bounds[n as usize] = 1.0;
        let cells = 6 * (n * n) as usize;
        let mut g = Grid {
            n,
            radius,
            bounds,
            centre: Vec::with_capacity(cells),
            nbr: vec![[NONE; 8]; cells],
            len_mm: vec![[0; 8]; cells],
            base: vec![1; cells],
            height: vec![radius; cells],
            blocks: vec![0; cells],
            min_step: 0.0,
        };
        for &[nrm, a, b] in &FACES {
            for j in 0..n {
                for i in 0..n {
                    let uc = (g.bounds[i as usize] + g.bounds[i as usize + 1]) * 0.5;
                    let vc = (g.bounds[j as usize] + g.bounds[j as usize + 1]) * 0.5;
                    let d = add(nrm, add(scale(a, uc), scale(b, vc)));
                    g.centre.push(norm(d).expect("face point is not zero"));
                }
            }
        }
        g.link();
        let mut min = f32::MAX;
        for c in 0..cells {
            for k in [1usize, 3, 4, 6] {
                let o = g.nbr[c][k];
                if o != NONE {
                    min = min.min(len(sub(g.centre[c], g.centre[o as usize])));
                }
            }
        }
        g.min_step = min;
        g
    }

    pub fn cells(&self) -> usize {
        self.centre.len()
    }

    /// Cell index of face `f`, column `i`, row `j`.
    pub fn index(&self, f: u32, i: u32, j: u32) -> u32 {
        f * self.n * self.n + j * self.n + i
    }

    /// (face, i, j) of a cell.
    pub fn coords(&self, c: u32) -> (u32, u32, u32) {
        let nn = self.n * self.n;
        (c / nn, c % self.n, (c % nn) / self.n)
    }

    /// The cell a (not necessarily unit) local direction points into.
    pub fn cell_of(&self, d: [f32; 3]) -> u32 {
        let (ax, ay, az) = (d[0].abs(), d[1].abs(), d[2].abs());
        let f = if ax >= ay && ax >= az {
            if d[0] >= 0.0 { 0 } else { 1 }
        } else if ay >= az {
            if d[1] >= 0.0 { 2 } else { 3 }
        } else if d[2] >= 0.0 {
            4
        } else {
            5
        };
        let [nrm, a, b] = FACES[f as usize];
        let t = dot(d, nrm);
        if t <= 0.0 {
            return 0; // zero vector
        }
        let (u, v) = (dot(d, a) / t, dot(d, b) / t);
        let i = self.axis(u);
        let j = self.axis(v);
        self.index(f, i, j)
    }

    fn axis(&self, u: f32) -> u32 {
        let k = self.bounds.partition_point(|b| *b <= u);
        (k.saturating_sub(1) as u32).min(self.n - 1)
    }

    /// Fills `nbr` and `len_mm`, crossing face seams by reflecting the inner neighbour,
    /// and makes the graph symmetric.
    fn link(&mut self) {
        let n = self.n as i32;
        let cells = self.cells();
        for c in 0..cells as u32 {
            let (f, i, j) = self.coords(c);
            let mut out = [NONE; 8];
            for (k, (di, dj)) in OFFSETS.iter().enumerate() {
                let (ni, nj) = (i as i32 + di, j as i32 + dj);
                let o = if (0..n).contains(&ni) && (0..n).contains(&nj) {
                    self.index(f, ni as u32, nj as u32)
                } else {
                    // Mirror the cell on the other side of this one onto the next face.
                    let (mi, mj) = ((i as i32 - di).clamp(0, n - 1), (j as i32 - dj).clamp(0, n - 1));
                    let m = self.index(f, mi as u32, mj as u32) as usize;
                    let here = self.centre[c as usize];
                    let v = sub(scale(here, 2.0), self.centre[m]);
                    self.cell_of(v)
                };
                if o != c && !out.contains(&o) {
                    out[k] = o;
                }
            }
            self.nbr[c as usize] = out;
        }
        // Symmetry: an edge a -> b gets b -> a in a free slot.
        for c in 0..cells as u32 {
            for k in 0..8 {
                let o = self.nbr[c as usize][k];
                if o == NONE || self.nbr[o as usize].contains(&c) {
                    continue;
                }
                if let Some(slot) = self.nbr[o as usize].iter().position(|x| *x == NONE) {
                    self.nbr[o as usize][slot] = c;
                } else {
                    self.nbr[c as usize][k] = NONE;
                }
            }
        }
        for c in 0..cells {
            for k in 0..8 {
                let o = self.nbr[c][k];
                if o != NONE {
                    let l = len(sub(self.centre[c], self.centre[o as usize])) * self.radius * 1000.0;
                    self.len_mm[c][k] = (l + 0.5) as u32;
                }
            }
        }
    }

    /// Walk weight now (0 = blocked).
    #[inline]
    pub fn weight(&self, c: u32) -> u8 {
        if self.blocks[c as usize] > 0 {
            0
        } else {
            self.base[c as usize]
        }
    }

    /// Whether moving from `c` through neighbour slot `k` is allowed: the target is open and
    /// a diagonal does not cut a blocked corner.
    #[inline]
    pub fn can_step(&self, c: u32, k: usize) -> bool {
        let o = self.nbr[c as usize][k];
        if o == NONE || self.weight(o) == 0 {
            return false;
        }
        let (di, dj) = OFFSETS[k];
        if di != 0 && dj != 0 {
            let a = self.nbr[c as usize][if di < 0 { 3 } else { 4 }];
            let b = self.nbr[c as usize][if dj < 0 { 1 } else { 6 }];
            if (a != NONE && self.weight(a) == 0) || (b != NONE && self.weight(b) == 0) {
                return false;
            }
        }
        true
    }

    /// Cost of the step `c` -> slot `k` (length times mean weight).
    #[inline]
    pub fn step_cost(&self, c: u32, k: usize) -> u64 {
        let o = self.nbr[c as usize][k];
        let w = self.weight(c).max(1) as u64 + self.weight(o) as u64;
        (self.len_mm[c as usize][k] as u64 * w).div_ceil(2)
    }

    /// Lower bound on the cost between two cells (chord length at weight 1).
    #[inline]
    pub fn estimate(&self, a: u32, b: u32) -> u64 {
        let l = len(sub(self.centre[a as usize], self.centre[b as usize])) * self.radius * 999.0;
        l as u64
    }

    /// Tile of a cell (for change tracking).
    pub fn tile_of(&self, c: u32) -> u32 {
        let (f, i, j) = self.coords(c);
        let per = self.n.div_ceil(TILE);
        f * per * per + (j / TILE) * per + i / TILE
    }

    pub fn tiles(&self) -> usize {
        let per = self.n.div_ceil(TILE) as usize;
        6 * per * per
    }

    /// Ground distance from the centre under a local direction, blended from the cell and its
    /// four side neighbours by inverse squared distance.
    pub fn ground(&self, d: [f32; 3]) -> f32 {
        let Some(u) = norm(d) else {
            return self.radius;
        };
        let c = self.cell_of(u);
        let mut sum = 0.0;
        let mut wsum = 0.0;
        let mut take = |x: u32| {
            let e = sub(u, self.centre[x as usize]);
            let w = 1.0 / (dot(e, e) + 1e-9);
            sum += self.height[x as usize] * w;
            wsum += w;
        };
        take(c);
        for k in [1usize, 3, 4, 6] {
            let o = self.nbr[c as usize][k];
            if o != NONE {
                take(o);
            }
        }
        sum / wsum
    }

    /// Whether the great-circle line between two cells crosses only open cells.
    pub fn line_open(&self, a: u32, b: u32) -> bool {
        self.line_open_dirs(self.centre[a as usize], self.centre[b as usize])
    }

    /// Like [`Grid::line_open`] between two unit directions.
    pub fn line_open_dirs(&self, pa: [f32; 3], pb: [f32; 3]) -> bool {
        let gap = sub(pb, pa);
        let steps = (len(gap) / (self.min_step * 0.5)) as u32 + 1;
        for s in 0..=steps {
            let p = add(pa, scale(gap, s as f32 / steps as f32));
            if self.weight(self.cell_of(p)) == 0 {
                return false;
            }
        }
        true
    }

    /// Cells whose centres lie within `metres` (chord at the base radius) of direction `d`,
    /// found by a flood from its cell. Sorted.
    pub fn cells_within(&self, d: [f32; 3], metres: f32) -> Vec<u32> {
        let Some(u) = norm(d) else {
            return Vec::new();
        };
        let r = metres / self.radius;
        let r2 = r * r;
        let start = self.cell_of(u);
        let mut out = vec![start];
        let mut seen = std::collections::BTreeSet::from([start]);
        let mut at = 0;
        while at < out.len() {
            let c = out[at];
            at += 1;
            for &o in &self.nbr[c as usize] {
                if o == NONE || seen.contains(&o) {
                    continue;
                }
                let e = sub(self.centre[o as usize], u);
                if dot(e, e) <= r2 {
                    seen.insert(o);
                    out.push(o);
                }
            }
        }
        out.sort_unstable();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cell_maps_back_to_itself_and_neighbours_are_close_and_symmetric() {
        let g = Grid::new(16, 100.0);
        assert_eq!(g.cells(), 6 * 256);
        for c in 0..g.cells() as u32 {
            assert_eq!(g.cell_of(g.centre[c as usize]), c);
            let mut count = 0;
            for &o in &g.nbr[c as usize] {
                if o == NONE {
                    continue;
                }
                count += 1;
                assert!(g.nbr[o as usize].contains(&c), "{c} -> {o} not symmetric");
                let d = len(sub(g.centre[c as usize], g.centre[o as usize]));
                assert!(d < 3.0 * g.min_step, "neighbour too far: {d} vs {}", g.min_step);
            }
            assert!(count >= 7, "cell {c} has {count} neighbours");
        }
    }

    #[test]
    fn warp_keeps_cells_even() {
        let g = Grid::new(32, 1.0);
        let mut max = 0.0f32;
        for c in 0..g.cells() {
            let o = g.nbr[c][4];
            if o != NONE {
                max = max.max(len(sub(g.centre[c], g.centre[o as usize])));
            }
        }
        assert!(max / g.min_step < 1.8, "ratio {}", max / g.min_step);
    }
}
