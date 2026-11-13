//! Level generation from a seed: text maps the template scripts read. Integer-only, so the
//! same seed gives the same map on every platform.

use crate::{Difficulty, Size};
use std::collections::VecDeque;

/// SplitMix64 (same generator as the simulation's RNG).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x5EED_5EED_5EED_5EED)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `lo..=hi`.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next_u64() % ((hi - lo + 1) as u64)) as i64
    }

    /// True with probability `percent` / 100.
    pub fn chance(&mut self, percent: u64) -> bool {
        self.next_u64() % 100 < percent
    }
}

/// Maze legend: `#` wall, `.` floor, `P` player start, `G` enemy start, `*` pickup, `E` goal.
pub fn maze(seed: u64, size: Size, difficulty: Difficulty) -> Vec<String> {
    let (cw, ch) = match size {
        Size::Small => (7, 5),
        Size::Medium => (9, 6),
        Size::Large => (12, 7),
    };
    let (w, h) = (cw * 2 + 1, ch * 2 + 1);
    let mut g = vec![vec![b'#'; w]; h];
    let mut rng = Rng::new(seed);

    // Perfect maze: depth-first carving from the top-left cell.
    let mut seen = vec![vec![false; cw]; ch];
    let mut stack = vec![(0usize, 0usize)];
    seen[0][0] = true;
    g[1][1] = b'.';
    while let Some(&(cx, cy)) = stack.last() {
        let mut next = Vec::new();
        for (dx, dy) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (cx as i64 + dx, cy as i64 + dy);
            if nx >= 0
                && ny >= 0
                && (nx as usize) < cw
                && (ny as usize) < ch
                && !seen[ny as usize][nx as usize]
            {
                next.push((nx as usize, ny as usize));
            }
        }
        if next.is_empty() {
            stack.pop();
            continue;
        }
        let (nx, ny) = next[rng.range(0, next.len() as i64 - 1) as usize];
        seen[ny][nx] = true;
        g[ny * 2 + 1][nx * 2 + 1] = b'.';
        g[cy + ny + 1][cx + nx + 1] = b'.';
        stack.push((nx, ny));
    }

    // Loops, so enemies can be walked around.
    let loops = match difficulty {
        Difficulty::Easy => 30,
        Difficulty::Normal => 20,
        Difficulty::Hard => 12,
    };
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            if g[y][x] != b'#' {
                continue;
            }
            let horizontal =
                g[y][x - 1] == b'.' && g[y][x + 1] == b'.' && g[y - 1][x] == b'#' && g[y + 1][x] == b'#';
            let vertical =
                g[y - 1][x] == b'.' && g[y + 1][x] == b'.' && g[y][x - 1] == b'#' && g[y][x + 1] == b'#';
            if (horizontal || vertical) && rng.chance(loops) {
                g[y][x] = b'.';
            }
        }
    }

    let start = (1usize, 1usize);
    g[start.1][start.0] = b'P';
    let dist = bfs(&g, start);
    let open: Vec<(usize, usize)> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| g[y][x] == b'.' && dist[y][x] != usize::MAX)
        .collect();
    // Goal: the farthest cell from the start.
    let goal = *open
        .iter()
        .max_by_key(|&&(x, y)| (dist[y][x], x, y))
        .expect("a maze has floor");
    g[goal.1][goal.0] = b'E';

    let pickups = match size {
        Size::Small => 5,
        Size::Medium => 7,
        Size::Large => 9,
    };
    let enemies = match difficulty {
        Difficulty::Easy => 1,
        Difficulty::Normal => 2,
        Difficulty::Hard => 3,
    } + usize::from(size == Size::Large);

    // Spread pickups and enemies: each new one is the cell farthest (in a straight line)
    // from everything placed so far; enemies also keep their distance from the start.
    let mut placed = vec![start, goal];
    let mut pick = |g: &mut Vec<Vec<u8>>, mark: u8, min_start: usize, rng: &mut Rng| {
        let candidates: Vec<(usize, usize)> = open
            .iter()
            .copied()
            .filter(|&(x, y)| g[y][x] == b'.' && dist[y][x] >= min_start)
            .collect();
        let best = candidates
            .iter()
            .map(|&(x, y)| {
                let d = placed
                    .iter()
                    .map(|&(px, py)| x.abs_diff(px).pow(2) + y.abs_diff(py).pow(2))
                    .min()
                    .unwrap_or(0);
                // A little noise breaks ties differently per seed.
                (d * 8 + (rng.next_u64() % 5) as usize, (x, y))
            })
            .max();
        if let Some((_, (x, y))) = best {
            g[y][x] = mark;
            placed.push((x, y));
        }
    };
    for _ in 0..pickups {
        pick(&mut g, b'*', 2, &mut rng);
    }
    for _ in 0..enemies {
        pick(&mut g, b'G', 8, &mut rng);
    }
    g.into_iter()
        .map(|r| String::from_utf8(r).expect("ascii"))
        .collect()
}

fn bfs(g: &[Vec<u8>], from: (usize, usize)) -> Vec<Vec<usize>> {
    let (w, h) = (g[0].len(), g.len());
    let mut d = vec![vec![usize::MAX; w]; h];
    let mut q = VecDeque::from([from]);
    d[from.1][from.0] = 0;
    while let Some((x, y)) = q.pop_front() {
        for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
            if nx < w && ny < h && g[ny][nx] != b'#' && d[ny][nx] == usize::MAX {
                d[ny][nx] = d[y][x] + 1;
                q.push_back((nx, ny));
            }
        }
    }
    d
}

/// Platformer rows from top to bottom. Legend: `#` ground, `=` one-way platform, `^` spikes,
/// `e` walking enemy, `*` pickup, `P` player start, `G` goal, `.` air.
pub const PLATFORMER_ROWS: usize = 12;

#[allow(clippy::needless_range_loop)] // two rows change per column
pub fn platformer(seed: u64, size: Size, difficulty: Difficulty) -> Vec<String> {
    let h = PLATFORMER_ROWS;
    let length = match size {
        Size::Small => 64,
        Size::Medium => 96,
        Size::Large => 136,
    };
    let (gap_chance, max_gap, hazard_chance) = match difficulty {
        Difficulty::Easy => (20, 2, 25),
        Difficulty::Normal => (30, 3, 40),
        Difficulty::Hard => (40, 3, 55),
    };
    let mut rng = Rng::new(seed);
    let mut g = vec![vec![b'.'; length + 32]; h];
    let fill = |g: &mut Vec<Vec<u8>>, x: usize, top: usize| {
        for row in g.iter_mut().take(h).skip(top) {
            row[x] = b'#';
        }
    };

    // Surface row of each segment: 5 (high) to 10 (low).
    let mut top = 9usize;
    let mut x = 0usize;
    for c in 0..7 {
        fill(&mut g, c, top);
    }
    g[top - 1][2] = b'P';
    x += 7;
    let mut after_gap;
    while x < length {
        // A gap, sometimes.
        after_gap = false;
        if rng.chance(gap_chance) {
            let w = rng.range(2, max_gap) as usize;
            // Pickups arc over the gap.
            let mid = x + w / 2;
            g[top - 3][mid] = b'*';
            x += w;
            after_gap = true;
        }
        let len = rng.range(4, 9) as usize;
        let up_limit = if after_gap { 1 } else { 2 };
        let dh = rng.range(-2, up_limit);
        top = (top as i64 - dh).clamp(5, 10) as usize;
        let end = x + len;
        for c in x..end {
            fill(&mut g, c, top);
        }
        let seg = end - x;
        if seg >= 7 && rng.chance(hazard_chance) {
            // One hazard just before the middle (at least three tiles of ground after it):
            // spikes or an enemy.
            let c = x + (seg - 1) / 2;
            g[top - 1][c] = if rng.chance(50) { b'^' } else { b'e' };
        }
        if seg >= 7 && rng.chance(35) {
            // A floating platform with pickups on it.
            let p = x + 1 + rng.range(0, (seg - 5) as i64) as usize;
            for c in p..p + 3 {
                if g[top - 4][c] == b'.' {
                    g[top - 4][c] = b'=';
                }
                g[top - 5][c] = b'*';
            }
        } else {
            // Pickups along the ground.
            for c in (x + 1..end.saturating_sub(1)).step_by(3) {
                if g[top - 1][c] == b'.' && g[top - 2][c] == b'.' {
                    g[top - 2][c] = b'*';
                }
            }
        }
        x = end;
    }
    // A flat run-out with the goal.
    for c in x..x + 8 {
        fill(&mut g, c, top);
    }
    g[top - 1][x + 5] = b'G';
    let width = x + 8;
    g.into_iter()
        .map(|mut r| {
            r.truncate(width);
            String::from_utf8(r).expect("ascii")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mazes_are_connected_and_have_every_piece() {
        for seed in 1..40 {
            for size in [Size::Small, Size::Medium, Size::Large] {
                let m = maze(seed, size, Difficulty::Normal);
                let g: Vec<Vec<u8>> = m.iter().map(|r| r.as_bytes().to_vec()).collect();
                let count = |c: u8| g.iter().flatten().filter(|&&b| b == c).count();
                assert_eq!(count(b'P'), 1);
                assert_eq!(count(b'E'), 1);
                assert!(count(b'*') >= 5);
                assert!(count(b'G') >= 2);
                let d = bfs(&g, (1, 1));
                for (y, row) in g.iter().enumerate() {
                    for (x, &c) in row.iter().enumerate() {
                        if c != b'#' {
                            assert_ne!(d[y][x], usize::MAX, "seed {seed}: ({x},{y}) unreachable");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn same_seed_same_level() {
        assert_eq!(
            maze(7, Size::Medium, Difficulty::Hard),
            maze(7, Size::Medium, Difficulty::Hard)
        );
        assert_eq!(
            platformer(7, Size::Medium, Difficulty::Hard),
            platformer(7, Size::Medium, Difficulty::Hard)
        );
        assert_ne!(
            maze(7, Size::Medium, Difficulty::Hard),
            maze(8, Size::Medium, Difficulty::Hard)
        );
    }

    #[test]
    fn platformer_has_start_and_goal_and_no_long_gaps() {
        for seed in 1..40 {
            let m = platformer(seed, Size::Medium, Difficulty::Hard);
            let g: Vec<&[u8]> = m.iter().map(|r| r.as_bytes()).collect();
            assert_eq!(g.iter().flat_map(|r| r.iter()).filter(|&&b| b == b'P').count(), 1);
            assert_eq!(g.iter().flat_map(|r| r.iter()).filter(|&&b| b == b'G').count(), 1);
            let mut gap = 0;
            for (x, &c) in g[PLATFORMER_ROWS - 1].iter().enumerate() {
                if c == b'#' {
                    gap = 0;
                } else {
                    gap += 1;
                    assert!(gap <= 3, "seed {seed}: gap wider than 3 at {x}");
                }
            }
        }
    }
}
