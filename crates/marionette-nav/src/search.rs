//! Path searches on a [`Grid`] (decision 0232): A* for one agent, a goal-rooted Dijkstra
//! shared by every agent heading to the same cell, and string pulling. Costs are integers,
//! ties break by cell index, so the same grid and request give the same path everywhere.

use crate::grid::{Grid, NONE};
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Per-thread search memory, reset by generation stamps (no clearing per search).
struct Scratch {
    stamp: Vec<u32>,
    gen_: u32,
    cost: Vec<u64>,
    parent: Vec<u32>,
    closed: Vec<u32>,
    heap: BinaryHeap<Reverse<(u64, u32)>>,
}

impl Scratch {
    fn new() -> Self {
        Self {
            stamp: Vec::new(),
            gen_: 0,
            cost: Vec::new(),
            parent: Vec::new(),
            closed: Vec::new(),
            heap: BinaryHeap::new(),
        }
    }

    fn begin(&mut self, cells: usize) {
        if self.stamp.len() != cells {
            self.stamp = vec![0; cells];
            self.closed = vec![0; cells];
            self.cost = vec![0; cells];
            self.parent = vec![NONE; cells];
            self.gen_ = 0;
        }
        self.gen_ = self.gen_.wrapping_add(1);
        if self.gen_ == 0 {
            self.stamp.fill(0);
            self.closed.fill(0);
            self.gen_ = 1;
        }
        self.heap.clear();
    }

    #[inline]
    fn cost(&self, c: u32) -> u64 {
        if self.stamp[c as usize] == self.gen_ {
            self.cost[c as usize]
        } else {
            u64::MAX
        }
    }

    #[inline]
    fn set(&mut self, c: u32, cost: u64, parent: u32) {
        self.stamp[c as usize] = self.gen_;
        self.cost[c as usize] = cost;
        self.parent[c as usize] = parent;
    }

    #[inline]
    fn close(&mut self, c: u32) -> bool {
        if self.closed[c as usize] == self.gen_ {
            return false;
        }
        self.closed[c as usize] = self.gen_;
        true
    }
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::new());
}

/// A found route: cells from start to goal (or to the closest reachable cell).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub cells: Vec<u32>,
    /// False when the goal could not be reached and the route ends as close as possible.
    pub complete: bool,
}

/// The open cell nearest to `c` (itself if open), searching up to `limit` cells outward.
pub fn nearest_open(g: &Grid, c: u32, limit: usize) -> Option<u32> {
    if g.weight(c) > 0 {
        return Some(c);
    }
    let mut ring = vec![c];
    let mut seen = std::collections::BTreeSet::from([c]);
    let mut at = 0;
    while at < ring.len() && ring.len() <= limit {
        let x = ring[at];
        at += 1;
        let mut best: Option<(u64, u32)> = None;
        for &o in &g.nbr[x as usize] {
            if o == NONE || !seen.insert(o) {
                continue;
            }
            if g.weight(o) > 0 {
                let d = g.estimate(c, o);
                if best.is_none_or(|b| (d, o) < b) {
                    best = Some((d, o));
                }
            }
            ring.push(o);
        }
        if let Some((_, o)) = best {
            return Some(o);
        }
    }
    None
}

/// Heuristic weight in percent: A* expands far fewer cells when it trusts the straight-line
/// estimate a little more than exactly (on an 8-way grid the true cost is up to 8% above
/// it). Routes cost at most this much more than the best one; smoothing straightens them.
pub const HEURISTIC_PERCENT: u64 = 110;

/// A* from `start` to `goal`, expanding at most `budget` cells. The start cell may be
/// blocked (an agent standing where an obstacle just appeared walks out of it).
pub fn astar(g: &Grid, start: u32, goal: u32, budget: u32) -> Route {
    if start == goal {
        return Route {
            cells: vec![start],
            complete: true,
        };
    }
    SCRATCH.with(|s| {
        let s = &mut *s.borrow_mut();
        s.begin(g.cells());
        let h = |c: u32| g.estimate(c, goal) * HEURISTIC_PERCENT / 100;
        s.set(start, 0, NONE);
        s.heap.push(Reverse((h(start), start)));
        let mut best = (g.estimate(start, goal), start);
        let mut expanded = 0;
        let mut reached = false;
        while let Some(Reverse((_, c))) = s.heap.pop() {
            if !s.close(c) {
                continue;
            }
            if c == goal {
                reached = true;
                break;
            }
            let est = g.estimate(c, goal);
            if (est, c) < best {
                best = (est, c);
            }
            expanded += 1;
            if expanded >= budget {
                break;
            }
            let base = s.cost(c);
            for k in 0..8 {
                if !g.can_step(c, k) {
                    continue;
                }
                let o = g.nbr[c as usize][k];
                let nc = base + g.step_cost(c, k);
                if nc < s.cost(o) {
                    s.set(o, nc, c);
                    s.heap.push(Reverse((nc + h(o), o)));
                }
            }
        }
        let end = if reached { goal } else { best.1 };
        Route {
            cells: walk_back(s, end),
            complete: reached,
        }
    })
}

fn walk_back(s: &Scratch, end: u32) -> Vec<u32> {
    let mut out = vec![end];
    let mut c = end;
    while s.parent[c as usize] != NONE {
        c = s.parent[c as usize];
        out.push(c);
    }
    out.reverse();
    out
}

/// One Dijkstra from `goal` outward until every start is reached (or `budget` cells are
/// expanded), then a route per start by following parents toward the goal. Starts the
/// search did not reach get `None` (the caller falls back to [`astar`]). The grid's edges
/// are symmetric, so goal-to-start costs equal start-to-goal costs.
pub fn shared(g: &Grid, goal: u32, starts: &[u32], budget: u32) -> Vec<Option<Route>> {
    SCRATCH.with(|s| {
        let s = &mut *s.borrow_mut();
        s.begin(g.cells());
        s.set(goal, 0, NONE);
        s.heap.push(Reverse((0, goal)));
        let mut left: std::collections::BTreeSet<u32> = starts.iter().copied().collect();
        left.remove(&goal);
        let mut expanded = 0;
        while let Some(Reverse((_, c))) = s.heap.pop() {
            if left.is_empty() || expanded >= budget {
                break;
            }
            if !s.close(c) {
                continue;
            }
            left.remove(&c);
            expanded += 1;
            let base = s.cost(c);
            for k in 0..8 {
                let o = g.nbr[c as usize][k];
                if o == NONE {
                    continue;
                }
                // Reverse edge o -> c must be walkable: c open (it is), o may be a blocked
                // start, so check the corner rule from o's side through c's slot.
                let Some(back) = g.nbr[o as usize].iter().position(|x| *x == c) else {
                    continue;
                };
                let start_here = starts.contains(&o);
                if (g.weight(o) == 0 && !start_here) || !g.can_step(o, back) {
                    continue;
                }
                let nc = base + g.step_cost(o, back);
                if nc < s.cost(o) {
                    s.set(o, nc, c);
                    s.heap.push(Reverse((nc, o)));
                }
            }
        }
        starts
            .iter()
            .map(|&st| {
                if st == goal {
                    return Some(Route {
                        cells: vec![goal],
                        complete: true,
                    });
                }
                if s.closed[st as usize] != s.gen_ {
                    return None;
                }
                let mut cells = vec![st];
                let mut c = st;
                while s.parent[c as usize] != NONE {
                    c = s.parent[c as usize];
                    cells.push(c);
                }
                Some(Route {
                    cells,
                    complete: true,
                })
            })
            .collect()
    })
}

/// Drops cells that a straight line can skip (string pulling with line of sight). Keeps the
/// first and last cell.
pub fn smooth(g: &Grid, cells: &[u32]) -> Vec<u32> {
    if cells.len() <= 2 {
        return cells.to_vec();
    }
    let last = cells.len() - 1;
    let mut out = vec![cells[0]];
    let mut a = 0;
    while a < last {
        // Gallop: the farthest cell seen from `a` by doubling steps, then halving between
        // the last seen and the first hidden. Every kept segment is checked open.
        let mut seen = a + 1;
        let mut step = 1;
        let mut hidden = None;
        loop {
            let j = (seen + step).min(last);
            if j == seen {
                break;
            }
            if g.line_open(cells[a], cells[j]) {
                seen = j;
                step *= 2;
            } else {
                hidden = Some(j);
                break;
            }
        }
        if let Some(mut hi) = hidden {
            while hi - seen > 1 {
                let mid = (seen + hi) / 2;
                if g.line_open(cells[a], cells[mid]) {
                    seen = mid;
                } else {
                    hi = mid;
                }
            }
        }
        out.push(cells[seen]);
        a = seen;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn astar_goes_round_a_wall_and_shared_agrees() {
        let mut g = Grid::new(32, 50.0);
        let a = g.index(4, 4, 16);
        let b = g.index(4, 28, 16);
        // Wall across the face between them, open only near the top.
        for j in 0..30 {
            let w = g.index(4, 16, j);
            g.blocks[w as usize] = 1;
        }
        let r = astar(&g, a, b, 100_000);
        assert!(r.complete);
        assert_eq!(r.cells.first(), Some(&a));
        assert_eq!(r.cells.last(), Some(&b));
        assert!(r.cells.iter().all(|c| g.weight(*c) > 0));
        let cost = |cells: &[u32]| -> u64 {
            cells
                .windows(2)
                .map(|w| {
                    let k = g.nbr[w[0] as usize].iter().position(|x| *x == w[1]).unwrap();
                    g.step_cost(w[0], k)
                })
                .sum()
        };
        let s = shared(&g, b, &[a], 1_000_000);
        let sr = s[0].clone().unwrap();
        assert_eq!(sr.cells.first(), Some(&a));
        let best = cost(&sr.cells);
        let found = cost(&r.cells);
        assert!(
            found >= best && found <= best * HEURISTIC_PERCENT / 100,
            "{found} vs best {best}"
        );
        let sm = smooth(&g, &r.cells);
        assert!(sm.len() >= 3 && sm.len() < r.cells.len());
        for w in sm.windows(2) {
            assert!(g.line_open(w[0], w[1]));
        }
    }

    #[test]
    fn unreachable_goal_gives_a_partial_route() {
        let mut g = Grid::new(16, 10.0);
        let goal = g.index(2, 8, 8);
        for &o in &g.nbr[goal as usize].clone() {
            g.blocks[o as usize] = 1;
        }
        let start = g.index(3, 8, 8);
        let r = astar(&g, start, goal, 1_000_000);
        assert!(!r.complete);
        assert!(r.cells.len() > 1);
    }

    #[test]
    fn routes_cross_face_seams() {
        let g = Grid::new(16, 10.0);
        let a = g.index(0, 8, 8);
        let b = g.index(1, 8, 8); // opposite face
        let r = astar(&g, a, b, 1_000_000);
        assert!(r.complete);
        let faces: std::collections::BTreeSet<u32> = r.cells.iter().map(|c| g.coords(*c).0).collect();
        assert!(faces.len() >= 3, "crosses an in-between face: {faces:?}");
    }
}
