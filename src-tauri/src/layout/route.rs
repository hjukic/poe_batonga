//! A way through the recorded map of the visit: from the player's cell to a
//! target cell, the shortest way that does not cross a recorded wall.
//!
//! The recorder's walls are minimap cells (2 px each) and how often each was
//! seen: thin lines (one or two cells; drawn twice where stitching drifted) with
//! real openings, gaps of a cell or two where the reader missed a pixel, and
//! specks. Fog, where nothing is recorded, is not a wall: the way may cross it
//! (it may be open ground nobody has seen). The search runs on a coarser grid,
//! `COARSE` minimap cells a side:
//! - a search cell is a wall when it holds a cell seen twice or more and its
//!   3x3 search cells hold at least `WALL_MIN` such cells (a line does); specks of
//!   one or two cells, and cells seen once, are not;
//! - a wall also closes its eight neighbours: a line becomes about 3 search cells
//!   (9 minimap cells) thick, which closes every gap of up to 8 cells (the
//!   reader's one- and two-cell gaps) and leaves every opening of 11 or more open
//!   (9-10 depend on where the grid falls); real openings are wider;
//! - cells the player walked through are open whatever was drawn there (the
//!   trail, steps up to `JUMP`), and a line only where the walk crossed it: a
//!   walk that dips into a line and comes out on the same side (open ground joins
//!   the two within `CROSSING`) opens nothing; in the owner's recordings 8-46% of the walked
//!   line lies within a cell of a recorded wall (the recording's error, not the
//!   ground's);
//! - moves go in 8 directions, never diagonally past a closed cell.
//!
//! The search covers the box around the player and the target with a margin,
//! cut to the box of what is recorded, and gives up after `MAX_EXPAND` cells:
//! then there is no way and the caller keeps its own needle.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

/// Minimap cells per search cell.
pub const COARSE: i32 = 3;
/// A wall cell seen fewer times is noise.
const TRUSTED: u32 = 2;
/// Trusted cells within a search cell's 3x3 neighbours that make it a wall.
const WALL_MIN: u32 = 3;
/// Trail steps longer than this (minimap cells, either axis) are jumps (a
/// waypoint, a re-found frame), not walked.
const JUMP: i32 = 30;
/// The search box: the box around player and target, this share of their
/// distance (minimap cells) wider on every side, at least `MARGIN_MIN`, at most
/// `MARGIN_MAX`; cut to the recorded map's box plus `OUTSIDE`.
const MARGIN_SHARE: f64 = 0.75;
const MARGIN_MIN: f64 = 150.0;
const MARGIN_MAX: f64 = 600.0;
/// Beyond the recorded box ground is open too, this far (minimap cells): a wall that
/// ends at the edge of what was seen can be gone round.
const OUTSIDE: f64 = 60.0;
/// The most search cells a grid may have (about 3,700 x 3,700 minimap cells): a
/// target farther than that (a wrong merge, a bad sighting) gets no way.
const MAX_GRID: i64 = 1_500_000;
/// The most search cells expanded before giving up (a way 2,200 cells long
/// across a crowded map needs 41,000; giving up after 60,000 takes 6 ms).
const MAX_EXPAND: usize = 60_000;
/// A player or target in a closed cell (next to a wall, an exit drawn on its
/// line) starts from the open cells this close (search cells), reached without
/// crossing a wall cell.
const ESCAPE: i32 = 3;
/// The needle looks this far ahead along the way (minimap cells).
pub const LOOK: f64 = 40.0;
/// Ground counts as recorded within this many search cells (15 minimap cells)
/// of a cell seen twice or walked.
const RECORDED_NEAR: i32 = 5;
/// A walk through closed cells crossed a line (and opens it) when the open cells
/// before and after are not joined by open ground within this many search cells
/// (30 minimap cells).
const CROSSING: i32 = 10;
/// Cells of the way kept from the last call cost this share (percent): the
/// needle stays on its way unless another is clearly shorter.
const KEEP: u32 = 90;
/// Corners are cut by sight over at most this many cells of the way.
const SMOOTH_SPAN: usize = 64;

/// Search cell flags.
const WALL: u8 = 1;
const NEAR: u8 = 2;
const WALKED: u8 = 4;
const KEPT: u8 = 8;
const GOAL: u8 = 16;
/// The search cell holds a wall cell seen twice (a line or a speck).
const SEEN: u8 = 32;

/// Costs per search step (hundredths of a search cell).
const STRAIGHT: u32 = 100;
const DIAGONAL: u32 = 141;

/// A borrowed view of the recorded map of the part: wall cells with how often
/// each was seen, and the walked trail (minimap cells, part coordinates).
#[derive(Clone, Copy)]
pub struct Map<'a> {
    pub walls: &'a HashMap<(i32, i32), u32>,
    pub trail: &'a [[i32; 2]],
}

#[derive(Clone, Debug)]
pub struct Way {
    /// Corners from the player to the target, straight between (minimap cells).
    #[allow(dead_code)] // tests and measurements
    pub points: Vec<(f64, f64)>,
    /// The search cells it passes (their centres, minimap cells): handed back as
    /// `keep` on the next call.
    pub cells: Vec<(i32, i32)>,
    /// Its length (minimap cells).
    pub length: f64,
    /// The farthest point of it within `LOOK` that the player sees in a straight line
    /// (the target when that is nearer).
    pub ahead: (f64, f64),
    /// Search cells expanded (for the time bound).
    #[allow(dead_code)] // tests and measurements
    pub expanded: usize,
    /// Its search cells that are wall cells the walk opened.
    #[allow(dead_code)] // tests and measurements
    pub opened: usize,
    /// Share of its search cells within 15 minimap cells of recorded ground (a
    /// wall cell seen twice, or the walked trail), and of the walked trail alone.
    pub recorded: f64,
    pub walked: f64,
}

impl Way {
    /// Nearly the straight line from the player to the target: no corner more than
    /// a search cell off it, and at most 5% longer.
    pub fn straight(&self) -> bool {
        let (a, b) = (self.points[0], self.points[self.points.len() - 1]);
        let d = (b.0 - a.0).hypot(b.1 - a.1);
        let off = |p: &(f64, f64)| {
            if d < 1e-9 {
                return 0.0;
            }
            ((b.0 - a.0) * (a.1 - p.1) - (a.0 - p.0) * (b.1 - a.1)).abs() / d
        };
        self.length <= 1.05 * d + 1.0 && self.points.iter().all(|p| off(p) <= COARSE as f64)
    }
}

struct Grid {
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
    flags: Vec<u8>,
}

fn closed(flags: u8) -> bool {
    flags & (WALL | NEAR) != 0 && flags & WALKED == 0
}

impl Grid {
    fn index(&self, (i, j): (i32, i32)) -> Option<usize> {
        (i >= 0 && j >= 0 && i < self.w && j < self.h).then(|| (j * self.w + i) as usize)
    }

    /// Search coordinates of a minimap point (cell (x, y) spans x..x+1).
    fn at(&self, (x, y): (f64, f64)) -> (f64, f64) {
        ((x + 0.5 - self.x0 as f64) / COARSE as f64, (y + 0.5 - self.y0 as f64) / COARSE as f64)
    }

    fn cell(&self, p: (f64, f64)) -> (i32, i32) {
        let (u, v) = self.at(p);
        (u.floor() as i32, v.floor() as i32)
    }

    fn centre(&self, (i, j): (i32, i32)) -> (f64, f64) {
        let c = COARSE as f64;
        (self.x0 as f64 + (i as f64 + 0.5) * c - 0.5, self.y0 as f64 + (j as f64 + 0.5) * c - 0.5)
    }

    /// Outside the box is closed.
    fn open(&self, cell: (i32, i32)) -> bool {
        self.index(cell).is_some_and(|k| !closed(self.flags[k]))
    }

    /// Whether the straight line between two minimap points crosses only open cells.
    fn clear(&self, a: (f64, f64), b: (f64, f64)) -> bool {
        traverse(self.at(a), self.at(b), |cell| self.open(cell))
    }
}

/// Visits the search cells a segment passes (search coordinates), in order,
/// while `visit` returns true; true when it got to the end.
fn traverse((ax, ay): (f64, f64), (bx, by): (f64, f64), mut visit: impl FnMut((i32, i32)) -> bool) -> bool {
    let (mut i, mut j) = (ax.floor() as i32, ay.floor() as i32);
    let (ti, tj) = (bx.floor() as i32, by.floor() as i32);
    let (dx, dy) = (bx - ax, by - ay);
    let (si, sj) = (if dx > 0.0 { 1 } else { -1 }, if dy > 0.0 { 1 } else { -1 });
    let step = |d: f64| if d != 0.0 { 1.0 / d.abs() } else { f64::INFINITY };
    let (di, dj) = (step(dx), step(dy));
    let first = |a: f64, d: f64| {
        if d > 0.0 {
            (a.floor() + 1.0 - a) / d
        } else if d < 0.0 {
            (a - a.floor()) / -d
        } else {
            f64::INFINITY
        }
    };
    let (mut ti_max, mut tj_max) = (first(ax, dx), first(ay, dy));
    // Never more cells than the two axes' spans together.
    let most = (ti - i).abs() + (tj - j).abs();
    for _ in 0..=most {
        if !visit((i, j)) {
            return false;
        }
        if i == ti && j == tj {
            break;
        }
        if ti_max < tj_max {
            ti_max += di;
            i += si;
        } else {
            tj_max += dj;
            j += sj;
        }
    }
    true
}

/// The search grid around `from` and `to`, with the recorded walls, the walked
/// trail and the kept way.
fn build(map: &Map, from: (f64, f64), to: (f64, f64), keep: &[(i32, i32)]) -> Option<Grid> {
    let straight = (to.0 - from.0).hypot(to.1 - from.1);
    let margin = (straight * MARGIN_SHARE).clamp(MARGIN_MIN, MARGIN_MAX);
    let mut outer = (from.0.min(to.0) - margin, from.1.min(to.1) - margin, from.0.max(to.0) + margin, from.1.max(to.1) + margin);
    // The recorded map's box: beyond it nothing is known, so nothing to go around.
    let mut seen = (from.0.min(to.0), from.1.min(to.1), from.0.max(to.0), from.1.max(to.1));
    let points = map.walls.keys().map(|&(x, y)| (x, y)).chain(map.trail.iter().map(|&[x, y]| (x, y)));
    for (x, y) in points {
        let (x, y) = (x as f64, y as f64);
        seen = (seen.0.min(x), seen.1.min(y), seen.2.max(x), seen.3.max(y));
    }
    outer = (
        outer.0.max(seen.0 - OUTSIDE),
        outer.1.max(seen.1 - OUTSIDE),
        outer.2.min(seen.2 + OUTSIDE),
        outer.3.min(seen.3 + OUTSIDE),
    );
    // On multiples of `COARSE`: the same walls give the same search cells, whatever
    // the box (else a 9-10 cell gap opens and shuts as the player moves).
    let (x0, y0) = ((outer.0.floor() as i32).div_euclid(COARSE) * COARSE, (outer.1.floor() as i32).div_euclid(COARSE) * COARSE);
    let w = ((outer.2.ceil() as i32 - x0) / COARSE + 1).max(1);
    let h = ((outer.3.ceil() as i32 - y0) / COARSE + 1).max(1);
    if w as i64 * h as i64 > MAX_GRID {
        return None;
    }
    let mut grid = Grid { x0, y0, w, h, flags: vec![0; (w * h) as usize] };
    let index = |x: i32, y: i32| {
        let (i, j) = ((x - x0).div_euclid(COARSE), (y - y0).div_euclid(COARSE));
        (i >= 0 && j >= 0 && i < w && j < h).then(|| (j * w + i) as usize)
    };
    let mut count = vec![0u8; (w * h) as usize];
    let mut held = Vec::new();
    for (&(x, y), &n) in map.walls {
        if n < TRUSTED {
            continue;
        }
        if let Some(k) = index(x, y) {
            if count[k] == 0 {
                held.push(k);
                grid.flags[k] |= SEEN;
            }
            count[k] = count[k].saturating_add(1);
        }
    }
    // A wall only as part of a line: a speck has too few cells around it.
    let around = |k: usize| {
        let (i, j) = ((k as i32) % w, (k as i32) / w);
        let mut sum = 0u32;
        for dj in -1..=1 {
            for di in -1..=1 {
                sum += grid.index((i + di, j + dj)).map_or(0, |n| count[n] as u32);
            }
        }
        sum
    };
    let walls: Vec<usize> = held.iter().copied().filter(|&k| around(k) >= WALL_MIN).collect();
    for k in walls {
        let (i, j) = ((k as i32) % w, (k as i32) / w);
        for dj in -1..=1 {
            for di in -1..=1 {
                if let Some(n) = grid.index((i + di, j + dj)) {
                    grid.flags[n] |= if di == 0 && dj == 0 { WALL } else { NEAR };
                }
            }
        }
    }
    // The walked trail is open, and a line it crossed: where the walk went into closed
    // cells from one open cell and came out at another that open ground does not join
    // to the first nearby. A walk that only dips into a line and comes back out on
    // the same side, or stays inside its band (running beside it, drift), opens
    // nothing there; the player standing in one starts from the open cells by it.
    let mut run: Vec<usize> = Vec::new();
    let mut before: Option<(i32, i32)> = None;
    let mut last: Option<[i32; 2]> = None;
    for &p in map.trail {
        let jump = last.is_none_or(|a| (p[0] - a[0]).abs().max((p[1] - a[1]).abs()) > JUMP);
        if jump {
            run.clear();
            before = None;
        }
        let Some(a) = last.replace(p).filter(|_| !jump) else { continue };
        if index(a[0], a[1]).is_none() && index(p[0], p[1]).is_none() {
            continue;
        }
        let (pa, pb) = (grid.at((a[0] as f64, a[1] as f64)), grid.at((p[0] as f64, p[1] as f64)));
        let mut cells = Vec::new();
        traverse(pa, pb, |c| {
            cells.push(c);
            true
        });
        for c in cells {
            let Some(k) = grid.index(c) else { continue };
            if closed(grid.flags[k]) {
                if !run.contains(&k) {
                    run.push(k);
                }
                continue;
            }
            if !run.is_empty() {
                if before.is_some_and(|b| !joined(&grid, b, c)) {
                    for &k in &run {
                        grid.flags[k] |= WALKED;
                    }
                }
                run.clear();
            }
            grid.flags[k] |= WALKED;
            before = Some(c);
        }
    }
    for &(x, y) in keep {
        let (i, j) = ((x - x0).div_euclid(COARSE), (y - y0).div_euclid(COARSE));
        for dj in -1..=1 {
            for di in -1..=1 {
                if let Some(n) = grid.index((i + di, j + dj)) {
                    grid.flags[n] |= KEPT;
                }
            }
        }
    }
    Some(grid)
}

/// Whether open ground joins two open cells within `CROSSING` search cells of the
/// first (4-connected, as the search moves without cutting a closed corner).
fn joined(grid: &Grid, a: (i32, i32), b: (i32, i32)) -> bool {
    let mut seen = std::collections::HashSet::from([a]);
    let mut queue = std::collections::VecDeque::from([a]);
    while let Some((i, j)) = queue.pop_front() {
        if (i, j) == b {
            return true;
        }
        for next in [(i + 1, j), (i - 1, j), (i, j + 1), (i, j - 1)] {
            if (next.0 - a.0).abs() > CROSSING || (next.1 - a.1).abs() > CROSSING || !grid.open(next) || !seen.insert(next) {
                continue;
            }
            queue.push_back(next);
        }
    }
    false
}

/// The open cells to start or end at for a point in `cell`: the cell itself when
/// open, else the open cells within `ESCAPE`, reached without crossing a wall
/// cell (`across`: also across one, for a target drawn on a line: an exit on a
/// cliff, or a point on a line drawn twice); with what getting there costs.
fn region(grid: &Grid, cell: (i32, i32), across: bool) -> Vec<(usize, u32)> {
    let Some(k) = grid.index(cell) else { return Vec::new() };
    if !closed(grid.flags[k]) {
        return vec![(k, 0)];
    }
    let mut out = Vec::new();
    let mut seen = vec![k];
    let mut queue = std::collections::VecDeque::from([cell]);
    while let Some((i, j)) = queue.pop_front() {
        for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let next = (i + di, j + dj);
            if (next.0 - cell.0).abs() > ESCAPE || (next.1 - cell.1).abs() > ESCAPE {
                continue;
            }
            let Some(n) = grid.index(next) else { continue };
            if seen.contains(&n) || (!across && grid.flags[n] & WALL != 0 && grid.flags[n] & WALKED == 0) {
                continue;
            }
            seen.push(n);
            if closed(grid.flags[n]) {
                queue.push_back(next);
            } else {
                let d = ((next.0 - cell.0) as f64).hypot((next.1 - cell.1) as f64);
                out.push((n, (d * STRAIGHT as f64) as u32));
            }
        }
    }
    out
}

/// The way from `from` to `to` (minimap cells, part coordinates) through the
/// recorded map, None when there is none within the search's bound. `keep`: the
/// cells of the way of the last call to the same target (hysteresis).
pub fn way(map: &Map, from: (f64, f64), to: (f64, f64), keep: &[(i32, i32)]) -> Option<Way> {
    search(map, from, to, keep).0
}

/// What a search cost: search cells expanded, and the grid's size.
#[derive(Clone, Copy, Debug, Default)]
struct Stats {
    expanded: usize,
    cells: usize,
}

fn search(map: &Map, from: (f64, f64), to: (f64, f64), keep: &[(i32, i32)]) -> (Option<Way>, Stats) {
    let mut stats = Stats::default();
    let Some(mut grid) = build(map, from, to, keep) else { return (None, stats) };
    stats.cells = grid.flags.len();
    let (start, goal) = (grid.cell(from), grid.cell(to));
    // The player is on one side of a line; a target on one is reached from either.
    let sources = region(&grid, start, false);
    let goals = region(&grid, goal, true);
    if sources.is_empty() || goals.is_empty() {
        return (None, stats);
    }
    for &(k, _) in &goals {
        grid.flags[k] |= GOAL;
    }
    // Octile distance to the goal cell, less what the goal's region spans; scaled
    // down where kept cells cost less, so it never overestimates.
    let slack = if goals.len() > 1 { ESCAPE as u32 * DIAGONAL } else { 0 };
    let scale = if keep.is_empty() { 100 } else { KEEP };
    let w = grid.w;
    let heuristic = |k: usize| {
        let (dx, dy) = (((k as i32) % w - goal.0).unsigned_abs(), ((k as i32) / w - goal.1).unsigned_abs());
        let octile = STRAIGHT * dx.max(dy) + (DIAGONAL - STRAIGHT) * dx.min(dy);
        octile.saturating_sub(slack) * scale / 100
    };
    let n = grid.flags.len();
    let mut cost = vec![u32::MAX; n];
    let mut parent = vec![u32::MAX; n];
    let mut heap = BinaryHeap::new();
    for &(k, c) in &sources {
        if c < cost[k] {
            cost[k] = c;
            heap.push(Reverse((c + heuristic(k), k as u32)));
        }
    }
    let mut expanded = 0;
    let mut reached = None;
    while let Some(Reverse((f, k))) = heap.pop() {
        let k = k as usize;
        let g = cost[k];
        if f > g.saturating_add(heuristic(k)) {
            continue;
        }
        if grid.flags[k] & GOAL != 0 {
            reached = Some(k);
            break;
        }
        expanded += 1;
        stats.expanded = expanded;
        if expanded > MAX_EXPAND {
            return (None, stats);
        }
        let (i, j) = ((k as i32) % w, (k as i32) / w);
        for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let next = (i + di, j + dj);
            let Some(m) = grid.index(next) else { continue };
            if closed(grid.flags[m]) {
                continue;
            }
            let diagonal = di != 0 && dj != 0;
            // Never diagonally past a closed cell: a thin diagonal line would leak.
            if diagonal && !(grid.open((i + di, j)) && grid.open((i, j + dj))) {
                continue;
            }
            let mut step = if diagonal { DIAGONAL } else { STRAIGHT };
            if grid.flags[m] & KEPT != 0 {
                step = step * KEEP / 100;
            }
            let c = g + step;
            if c < cost[m] {
                cost[m] = c;
                parent[m] = k as u32;
                heap.push(Reverse((c + heuristic(m), m as u32)));
            }
        }
    }
    let Some(mut k) = reached else { return (None, stats) };
    let mut cells = vec![(k as i32 % w, k as i32 / w)];
    while parent[k] != u32::MAX {
        k = parent[k] as usize;
        cells.push((k as i32 % w, k as i32 / w));
    }
    cells.reverse();
    // Corners cut by sight: from each corner, the farthest point of the way still
    // in view (up to `SMOOTH_SPAN` cells on).
    let mut along: Vec<(f64, f64)> = Vec::with_capacity(cells.len() + 2);
    along.push(from);
    along.extend(cells.iter().map(|&c| grid.centre(c)));
    along.push(to);
    let mut points = vec![from];
    let mut a = 0;
    while a + 1 < along.len() {
        let mut best = a + 1;
        for b in a + 2..=(a + SMOOTH_SPAN).min(along.len() - 1) {
            if grid.clear(along[a], along[b]) {
                best = b;
            } else {
                break;
            }
        }
        points.push(along[best]);
        a = best;
    }
    let length = points.windows(2).map(|p| (p[1].0 - p[0].0).hypot(p[1].1 - p[0].1)).sum();
    // How much of the way lies by recorded ground: the rest is a guess across fog.
    let near = |c: (i32, i32), mask: u8| {
        let r = RECORDED_NEAR;
        (-r..=r).any(|dj| (-r..=r).any(|di| grid.index((c.0 + di, c.1 + dj)).is_some_and(|k| grid.flags[k] & mask != 0)))
    };
    let share = |mask: u8| cells.iter().filter(|&&c| near(c, mask)).count() as f64 / cells.len().max(1) as f64;
    let (recorded, walked) = (share(SEEN | WALKED), share(WALKED));
    let opened = cells.iter().filter(|&&c| grid.index(c).is_some_and(|k| grid.flags[k] & (WALL | NEAR) != 0)).count();
    // The needle looks at the farthest point of the way within `LOOK` it can see from
    // the player in a straight line: past a corner the chord would cut the jamb.
    let first = (points[1].0 - from.0).hypot(points[1].1 - from.1);
    let mut ahead = point_along(&points, LOOK.min(first));
    let mut d = LOOK.min(first).floor() + 1.0;
    while d <= LOOK.min(length) {
        let p = point_along(&points, d);
        if !grid.clear(from, p) {
            break;
        }
        ahead = p;
        d += 1.0;
    }
    let way = Way {
        ahead,
        cells: cells.iter().map(|&c| grid.centre(c)).map(|(x, y)| (x.round() as i32, y.round() as i32)).collect(),
        points,
        length,
        expanded,
        opened,
        recorded,
        walked,
    };
    (Some(way), stats)
}

/// The point `distance` along a polyline (its end when shorter).
fn point_along(points: &[(f64, f64)], distance: f64) -> (f64, f64) {
    let mut left = distance;
    for p in points.windows(2) {
        let d = (p[1].0 - p[0].0).hypot(p[1].1 - p[0].1);
        if d >= left && d > 0.0 {
            let t = left / d;
            return (p[0].0 + t * (p[1].0 - p[0].0), p[0].1 + t * (p[1].1 - p[0].1));
        }
        left -= d;
    }
    *points.last().unwrap_or(&(0.0, 0.0))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::layout::guide::{angle_between, screen_bearing};
    use std::time::Instant;

    /// Trusted wall cells (seen three times).
    pub fn walls(cells: impl IntoIterator<Item = (i32, i32)>) -> HashMap<(i32, i32), u32> {
        cells.into_iter().map(|c| (c, 3)).collect()
    }

    /// The cells of a straight line, ends included.
    pub fn line(a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
        let n = (b.0 - a.0).abs().max((b.1 - a.1).abs()).max(1);
        (0..=n)
            .map(|k| {
                let t = k as f64 / n as f64;
                ((a.0 as f64 + t * (b.0 - a.0) as f64).round() as i32, (a.1 as f64 + t * (b.1 - a.1) as f64).round() as i32)
            })
            .collect()
    }

    /// The outline of a rectangle.
    pub fn outline(x0: i32, y0: i32, x1: i32, y1: i32) -> Vec<(i32, i32)> {
        [line((x0, y0), (x1, y0)), line((x1, y0), (x1, y1)), line((x1, y1), (x0, y1)), line((x0, y1), (x0, y0))].concat()
    }

    fn find(walls: &HashMap<(i32, i32), u32>, from: (f64, f64), to: (f64, f64)) -> Option<Way> {
        way(&Map { walls, trail: &[] }, from, to, &[])
    }

    fn bearing(from: (f64, f64), to: (f64, f64)) -> f32 {
        screen_bearing(to.0 - from.0, to.1 - from.1).unwrap()
    }

    /// Where a polyline crosses the line y = `y` (x values).
    fn crossings(points: &[(f64, f64)], y: f64) -> Vec<f64> {
        points
            .windows(2)
            .filter(|p| (p[0].1 - y) * (p[1].1 - y) <= 0.0 && p[0].1 != p[1].1)
            .map(|p| p[0].0 + (y - p[0].1) / (p[1].1 - p[0].1) * (p[1].0 - p[0].0))
            .collect()
    }

    /// SplitMix64.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: i32) -> i32 {
            (self.next() % n as u64) as i32
        }
    }

    #[test]
    fn a_wall_between_sends_the_needle_to_the_doorway_first() {
        // A wall across, with a 20-cell doorway to the right.
        let map = walls([line((-200, 0), (59, 0)), line((80, 0), (200, 0))].concat());
        let (player, target) = ((0.0, 40.0), (0.0, -40.0));
        let way = find(&map, player, target).expect("a way through the doorway");
        let straight = bearing(player, target);
        let needle = bearing(player, way.ahead);
        let doorway = bearing(player, (69.5, 0.0));
        println!("straight {straight:.0}°, needle {needle:.0}°, doorway {doorway:.0}°, length {:.0}", way.length);
        assert!(angle_between(needle, doorway) < 10.0, "{needle} {doorway}");
        assert!(angle_between(needle, straight) > 45.0);
        // Through the doorway, about as long as the two straight legs.
        let x = crossings(&way.points, 0.0);
        assert!(x.len() == 1 && (60.0..80.0).contains(&x[0]), "{x:?}");
        assert!((155.0..175.0).contains(&way.length), "{}", way.length);
    }

    #[test]
    fn a_u_shaped_corridor_is_followed_round() {
        // Arms 24 cells wide on either side of a block, joined at the bottom.
        let map = walls([outline(0, 0, 88, 124), outline(24, 0, 64, 100)].concat());
        let (player, target) = ((12.0, 10.0), (76.0, 10.0));
        let way = find(&map, player, target).expect("a way round");
        let needle = bearing(player, way.ahead);
        println!("needle {needle:.0}°, length {:.0}, straight 64", way.length);
        // Down the left arm first (90° is down on screen), not across the block.
        assert!(angle_between(needle, 90.0) < 15.0, "{needle}");
        assert!((230.0..290.0).contains(&way.length), "{}", way.length);
        assert!(way.points.iter().any(|p| p.1 > 100.0));
    }

    #[test]
    fn across_fog_the_way_is_straight() {
        // Recorded walls behind and beside the player; the target lies where nothing
        // is recorded.
        let map = walls([line((-50, -60), (-50, 60)), line((-50, 50), (50, 50))].concat());
        let (player, target) = ((0.0, 0.0), (150.0, -120.0));
        let way = find(&map, player, target).expect("fog is no wall");
        let straight = 150f64.hypot(120.0);
        assert!(way.length < straight * 1.02, "{} {straight}", way.length);
        assert!(angle_between(bearing(player, way.ahead), bearing(player, target)) < 3.0);
    }

    #[test]
    fn specks_and_cells_seen_once_are_no_walls() {
        let mut rng = Rng(7);
        // Lone specks (one cell in 200; Clearfell's part 0 has about 20 specks
        // under 20 cells in 650,000) between player and target …
        let mut map: HashMap<(i32, i32), u32> = HashMap::new();
        for _ in 0..(240 * 120 / 200) {
            map.insert((rng.below(240) - 20, rng.below(120) - 60), 5);
        }
        // … and a whole line across seen only once.
        for cell in line((100, -100), (100, 100)) {
            map.insert(cell, 1);
        }
        let way = find(&map, (0.0, 0.0), (200.0, 0.0)).expect("noise is no wall");
        println!("specks: length {:.0} for 200", way.length);
        assert!(way.length < 200.0 * 1.1, "{}", way.length);
    }

    #[test]
    fn gaps_up_to_eight_cells_are_closed_openings_of_eleven_are_not() {
        for offset in 0..COARSE {
            let mut open = Vec::new();
            for gap in 1..=14 {
                // A closed room round the target, a gap of `gap` cells in the side
                // facing the player.
                let (x0, x1) = (-40 + offset, 40 + offset);
                let left = -(gap / 2) - 1 + offset;
                let right = left + gap + 1;
                let mut cells = [line((x0, -40), (x1, -40)), line((x1, -40), (x1, 40)), line((x0, -40), (x0, 40))].concat();
                cells.extend(line((x0, 40), (left, 40)));
                cells.extend(line((right, 40), (x1, 40)));
                let map = walls(cells);
                if let Some(way) = find(&map, (offset as f64, 120.0), (offset as f64, 0.0)) {
                    assert!(way.length < 125.0, "{gap}: {}", way.length);
                    open.push(gap);
                }
            }
            println!("grid offset {offset}: open gaps {open:?}");
            assert!(open.iter().all(|&g| g > 8), "{open:?}");
            assert!((11..=14).all(|g| open.contains(&g)), "{open:?}");
        }
    }

    #[test]
    fn the_same_walls_give_the_same_doors_wherever_the_player_stands() {
        // A closed room round the target, a gap of 9, 10 or 12 cells facing the
        // player; the player one cell further left at each call (the search box, and
        // with it an unaligned grid, moves with the player).
        for gap in [9, 10, 12] {
            let mut cells = [line((-40, -40), (40, -40)), line((40, -40), (40, 40)), line((-40, -40), (-40, 40))].concat();
            cells.extend(line((-40, 40), (-1, 40)));
            cells.extend(line((gap, 40), (40, 40)));
            // Far recorded cells: the search box, not the recorded box, sets the edge.
            cells.extend([(-1000, -600), (1000, 600)]);
            let map = walls(cells);
            let open: Vec<bool> = (0..6).map(|dx| find(&map, (-dx as f64, 120.0), (0.0, 0.0)).is_some()).collect();
            println!("gap {gap}: {open:?}");
            assert!(open.iter().all(|&o| o == open[0]), "gap {gap}: {open:?}");
        }
    }

    #[test]
    fn a_diagonal_line_does_not_leak() {
        // A closed diamond (diagonal lines one cell thin) round the target.
        let map = walls([line((0, -60), (60, 0)), line((60, 0), (0, 60)), line((0, 60), (-60, 0)), line((-60, 0), (0, -60))].concat());
        assert!(find(&map, (100.0, 100.0), (0.0, 0.0)).is_none());
    }

    #[test]
    fn the_kept_way_wins_over_one_slightly_shorter() {
        // A square island between player and target: two ways round it, told
        // apart by where they pass the island's middle.
        let map = walls(outline(-30, -30, 30, 30));
        let target = (0.0, -100.0);
        let side = |way: &Way| crossings(&way.points, 0.0)[0].signum();
        let left = find(&map, (-8.0, 100.0), target).unwrap();
        assert_eq!(side(&left), -1.0);
        // A step to the right makes the right way shorter, by little (3%).
        let fresh = find(&map, (8.0, 100.0), target).unwrap();
        assert_eq!(side(&fresh), 1.0);
        let kept = way(&Map { walls: &map, trail: &[] }, (8.0, 100.0), target, &left.cells).unwrap();
        assert_eq!(side(&kept), -1.0);
        // Far to the right (the right way 15% shorter) it wins all the same.
        let far = way(&Map { walls: &map, trail: &[] }, (48.0, 100.0), target, &left.cells).unwrap();
        assert_eq!(side(&far), 1.0);
    }

    #[test]
    fn a_walked_trail_opens_a_line_it_crosses() {
        // A closed room round the target, and the trail of a player who walked in
        // across its wall (a line drawn where the player could walk: stitching drift).
        let map = walls(outline(-40, -40, 40, 40));
        assert!(find(&map, (0.0, 120.0), (0.0, 0.0)).is_none());
        let trail: Vec<[i32; 2]> = (0..=12).map(|k| [0, 120 - 10 * k]).collect();
        let way = way(&Map { walls: &map, trail: &trail }, (0.0, 120.0), (0.0, 0.0), &[]).expect("walked");
        assert!(way.length < 125.0);
        // Jumps (a waypoint) open nothing.
        let jump = [[0, 120], [0, 0]];
        assert!(super::way(&Map { walls: &map, trail: &jump }, (0.0, 120.0), (0.0, 0.0), &[]).is_none());
    }

    #[test]
    fn a_trail_that_runs_beside_a_wall_opens_no_door() {
        // A wall across, ending at x = 40; the walk ran along it 4 cells off,
        // wobbling 4 cells over it twice (drift); the target is just behind it.
        let map = walls(line((-200, 0), (40, 0)));
        let trail: Vec<[i32; 2]> = (0..=12).map(|k| [-60 + 5 * k, if k == 4 || k == 8 { -4 } else { 4 }]).collect();
        let (player, target) = ((0.0, 10.0), (0.0, -20.0));
        let way = way(&Map { walls: &map, trail: &trail }, player, target, &[]).expect("round the end");
        println!("length {:.0}, opened {}", way.length, way.opened);
        assert_eq!(way.opened, 0);
        // Round the wall's end at x = 40, not through the grazed line.
        assert!(crossings(&way.points, 0.0).iter().all(|&x| x > 40.0), "{:?}", way.points);
        // A walk that went across, from 10 cells one side to 10 the other, opens it.
        let closed = walls(line((-200, 0), (200, 0)));
        let walked: Vec<[i32; 2]> = (0..=4).map(|k| [-20, 10 - 5 * k]).collect();
        let across = super::way(&Map { walls: &closed, trail: &walked }, player, target, &[]).expect("walked across");
        assert!(across.opened > 0);
        assert!(super::way(&Map { walls: &closed, trail: &trail }, player, target, &[]).is_none(), "the wobble opened it");
    }

    /// 40,000 wall cells: room outlines over 1,600 x 1,600 cells.
    fn big_map() -> HashMap<(i32, i32), u32> {
        let mut rng = Rng(11);
        let mut map = walls(outline(0, 0, 1600, 1600));
        while map.len() < 40_000 {
            let (x, y) = (rng.below(1500) + 20, rng.below(1500) + 20);
            let (w, h) = (rng.below(60) + 20, rng.below(60) + 20);
            // Keep the corners free for player and target.
            if (x < 120 && y < 120) || (x + w > 1480 && y + h > 1480) {
                continue;
            }
            map.extend(outline(x, y, x + w, y + h).into_iter().map(|c| (c, 3)));
        }
        map
    }

    #[test]
    fn the_search_stays_bounded_on_a_large_map() {
        let mut map = big_map();
        let cells = map.len();
        // The fastest of three calls (the machine may be busy), and what it cost.
        let time = |map: &HashMap<(i32, i32), u32>, to: (f64, f64)| {
            let mut best = (std::time::Duration::MAX, None, Stats::default());
            for _ in 0..3 {
                let started = Instant::now();
                let (found, stats) = search(&Map { walls: map, trail: &[] }, (40.0, 40.0), to, &[]);
                best = (best.0.min(started.elapsed()), found, stats);
            }
            best
        };
        let (near, way, stats) = time(&map, (300.0, 260.0));
        let way = way.expect("a way between the rooms");
        println!("{cells} wall cells: 330 cells away {near:?}, {stats:?}, length {:.0}", way.length);
        assert!(stats.expanded <= 2_000, "{stats:?}");
        let (across, way, stats) = time(&map, (1560.0, 1560.0));
        let way = way.expect("a way across");
        println!("across (2,150 cells): {across:?}, {stats:?}, length {:.0}", way.length);
        assert!(stats.expanded <= 45_000 && stats.cells <= 350_000, "{stats:?}");
        // A target walled in: the search gives up at its bound.
        map.extend(outline(1500, 1500, 1590, 1590).into_iter().map(|c| (c, 3)));
        let (none, way, stats) = time(&map, (1545.0, 1545.0));
        assert!(way.is_none());
        println!("walled in: {none:?}, {stats:?}");
        assert_eq!(stats.expanded, MAX_EXPAND + 1);
        // The test profile is optimised less than the app (opt-level 1 against 2):
        // release numbers are in the docs.
        for t in [near, across, none] {
            assert!(t.as_millis() < 60, "{t:?}");
        }
        // A target 4,000 cells off (a wrong merge, a bad sighting): no grid at all.
        map.insert((5000, 5000), 3);
        let started = Instant::now();
        let (way, stats) = search(&Map { walls: &map, trail: &[] }, (40.0, 40.0), (4000.0, 4000.0), &[]);
        assert!(way.is_none() && stats.cells == 0, "{stats:?}");
        assert!(started.elapsed().as_millis() < 20, "{:?}", started.elapsed());
    }

    #[test]
    fn the_needle_looks_at_the_corner_not_through_the_jamb() {
        // A door 10-20 cells to the right in a wall below the player, a corridor going
        // down behind it to the target.
        let map = walls([line((-60, 6), (8, 6)), line((22, 6), (80, 6)), line((8, 6), (8, 120)), line((22, 6), (22, 120))].concat());
        let (player, target) = ((0.0, 0.0), (15.0, 100.0));
        let way = find(&map, player, target).expect("through the door");
        let corner = way.points[1];
        let needle = bearing(player, way.ahead);
        println!("needle {needle:.0}°, corner {:?} {:.0}°, 40 along {:.0}°", corner, bearing(player, corner), bearing(player, point_along(&way.points, LOOK)));
        assert!(angle_between(needle, bearing(player, corner)) < 10.0, "{needle} {corner:?}");
        // The point 40 cells along is past the corner: the chord to it cuts the jamb.
        let grid = build(&Map { walls: &map, trail: &[] }, player, target, &[]).unwrap();
        assert!(!grid.clear(player, point_along(&way.points, LOOK)));
    }

    #[test]
    fn a_wall_ending_at_the_edge_of_what_was_seen_can_be_gone_round() {
        // All that is recorded is one line between player and target: its ends are the
        // edge of the recorded box. The way goes round an end.
        let map = walls(line((0, -100), (0, 100)));
        let way = find(&map, (-20.0, 0.0), (20.0, 0.0)).expect("round an end");
        assert!(way.length < 230.0, "{}", way.length);
        // Open ground reaches 50 cells and more beyond the recorded box (it was 6: a
        // doubled line's halo closed that).
        let grid = build(&Map { walls: &map, trail: &[] }, (-20.0, 0.0), (20.0, 0.0), &[]).unwrap();
        assert!(grid.y0 <= -150 && grid.y0 + grid.h * COARSE >= 150, "{} {}", grid.y0, grid.h);
    }

    /// The owner's Clearfell (seed 2693856399), part 0: wall cells (seen twice or
    /// more, and once) and the walked trail.
    pub fn clearfell() -> (HashMap<(i32, i32), u32>, Vec<[i32; 2]>) {
        let text = include_str!("../../tests/fixtures/clearfell-part0-map.txt");
        let (mut walls, mut trail) = (HashMap::new(), Vec::new());
        let mut section = "";
        for row in text.lines().filter(|l| !l.starts_with('#')) {
            match row {
                "walls" | "trail" => section = if row == "walls" { "walls" } else { "trail" },
                _ if section == "walls" => {
                    let (x, runs) = row.split_once(':').unwrap();
                    let x: i32 = x.parse().unwrap();
                    let mut n = 2;
                    for run in runs.split_whitespace() {
                        if run == "~" {
                            n = 1;
                            continue;
                        }
                        let (y, len) = run.split_once('+').unwrap();
                        let (y, len): (i32, i32) = (y.parse().unwrap(), len.parse().unwrap());
                        for k in 0..len {
                            walls.insert((x, y + k), n);
                        }
                    }
                }
                _ => {
                    for point in row.split_whitespace() {
                        let (x, y) = point.split_once(',').unwrap();
                        trail.push([x.parse().unwrap(), y.parse().unwrap()]);
                    }
                }
            }
        }
        (walls, trail)
    }

    /// Points every 2 cells along a polyline.
    fn samples(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
        let mut out = vec![points[0]];
        for p in points.windows(2) {
            let d = (p[1].0 - p[0].0).hypot(p[1].1 - p[0].1);
            let n = (d / 2.0).ceil().max(1.0) as usize;
            out.extend((1..=n).map(|k| {
                let t = k as f64 / n as f64;
                (p[0].0 + t * (p[1].0 - p[0].0), p[0].1 + t * (p[1].1 - p[0].1))
            }));
        }
        out
    }

    fn to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
        (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
    }

    fn median(mut v: Vec<f64>) -> f64 {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    }

    #[test]
    fn on_real_walls_the_way_keeps_to_the_walked_ground() {
        let (map, trail) = clearfell();
        let pt = |k: usize| (trail[k][0] as f64, trail[k][1] as f64);
        // Search cells with a wall the player has not walked across, that a polyline passes.
        let crossed = |grid: &Grid, points: &[(f64, f64)]| {
            let mut cells = std::collections::HashSet::new();
            for p in points.windows(2) {
                traverse(grid.at(p[0]), grid.at(p[1]), |c| {
                    if grid.index(c).is_some_and(|k| grid.flags[k] & WALL != 0 && grid.flags[k] & WALKED == 0) {
                        cells.insert(c);
                    }
                    true
                });
            }
            cells.len()
        };
        // Pairs 40 trail points apart (300-500 cells walked) along stretches without a jump.
        let mut pairs = Vec::new();
        let mut start = 0;
        for k in 1..=trail.len() {
            let jump = k == trail.len() || (trail[k][0] - trail[k - 1][0]).abs().max((trail[k][1] - trail[k - 1][1]).abs()) > JUMP;
            if jump {
                pairs.extend((start..k).step_by(10).filter(|&i| i + 40 < k).map(|i| (i, i + 40)));
                start = k;
            }
        }
        let (mut found, mut cross_way, mut cross_line, mut ratio) = (0, 0, 0, Vec::new());
        let (mut back_near, mut back_ratio, mut slowest) = (Vec::new(), Vec::new(), std::time::Duration::ZERO);
        let (mut back_opened, mut ahead_opened) = (0, 0);
        for &(i, j) in &pairs {
            let walked: Vec<(f64, f64)> = (i..=j).map(pt).collect();
            let length: f64 = walked.windows(2).map(|p| (p[1].0 - p[0].0).hypot(p[1].1 - p[0].1)).sum();
            let near = |points: &[(f64, f64)]| {
                let s = samples(points);
                s.iter().filter(|&&p| walked.windows(2).any(|w| to_segment(p, w[0], w[1]) <= 20.0)).count() as f64 / s.len() as f64
            };
            // To a target ahead (an icon in view): the walls, and what was walked up to the player.
            let ahead = Map { walls: &map, trail: &trail[..=i] };
            let started = Instant::now();
            let found_way = way(&ahead, pt(i), pt(j), &[]);
            slowest = slowest.max(started.elapsed());
            let grid = build(&ahead, pt(i), pt(j), &[]).unwrap();
            cross_line += crossed(&grid, &[pt(i), pt(j)]);
            if let Some(way) = found_way {
                found += 1;
                ahead_opened += (way.opened > 0) as usize;
                // Only where it starts in a wall (the trail lies on one), and on the
                // last step to a target on one (at most `ESCAPE` search cells).
                let last = way.points.len() - 1;
                cross_way += crossed(&grid, &way.points[..last]).saturating_sub(2);
                let (a, b) = (way.points[last - 1], way.points[last]);
                assert!((b.0 - a.0).hypot(b.1 - a.1) <= ((ESCAPE + 1) * COARSE) as f64 * 1.5 || crossed(&grid, &[a, b]) == 0);
                ratio.push(way.length / (pt(j).0 - pt(i).0).hypot(pt(j).1 - pt(i).1).max(1.0));
            }
            // Going back: the trail up to here is recorded.
            let started = Instant::now();
            let back = super::way(&Map { walls: &map, trail: &trail[..=j] }, pt(j), pt(i), &[]);
            slowest = slowest.max(started.elapsed());
            let Some(back) = back else { continue };
            back_near.push(near(&back.points));
            back_ratio.push(back.length / length);
            back_opened += (back.opened > 0) as usize;
        }
        println!(
            "{} pairs. Ahead: way found {found} ({ahead_opened} through a line the walk crossed), way/straight {:.2} (median); \
             unwalked wall cells crossed: straight {cross_line}, way {cross_way}. \
             Back: found {}, within 20 cells of the trail {:.2}, way/walked {:.2} (medians), {back_opened} through a line the walk crossed. \
             Slowest {slowest:?}",
            pairs.len(),
            median(ratio.clone()),
            back_near.len(),
            median(back_near.clone()),
            median(back_ratio.clone()),
        );
        assert!(pairs.len() >= 30);
        // A walk that ran inside a line's band (drift) opens nothing: fewer ways ahead.
        assert!(found * 100 >= pairs.len() * 85, "{found} of {}", pairs.len());
        assert!(back_near.len() * 100 >= pairs.len() * 95, "{} of {}", back_near.len(), pairs.len());
        assert!(cross_line >= 100 && cross_way == 0, "{cross_line} {cross_way}");
        assert!(median(back_near) >= 0.85);
        assert!(back_ratio.iter().all(|&r| r <= 1.05), "{back_ratio:?}");
    }

    #[test]
    fn on_real_walls_gaps_in_lines_are_closed() {
        let (map, _) = clearfell();
        let cells: Vec<(i32, i32)> = map.iter().filter(|(_, &n)| n >= TRUSTED).map(|(&c, _)| c).collect();
        let (x0, y0) = cells.iter().fold((i32::MAX, i32::MAX), |a, c| (a.0.min(c.0), a.1.min(c.1)));
        let (x1, y1) = cells.iter().fold((i32::MIN, i32::MIN), |a, c| (a.0.max(c.0), a.1.max(c.1)));
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        let at = |x: i32, y: i32| (x >= x0 && y >= y0 && x <= x1 && y <= y1).then(|| ((y - y0) * w + x - x0) as usize);
        let mut id = vec![u32::MAX; (w * h) as usize];
        for &c in &cells {
            id[at(c.0, c.1).unwrap()] = u32::MAX - 1;
        }
        // Lines: 8-connected groups.
        let mut sizes = Vec::new();
        for &c in &cells {
            if id[at(c.0, c.1).unwrap()] != u32::MAX - 1 {
                continue;
            }
            let k = sizes.len() as u32;
            let (mut stack, mut n) = (vec![c], 0);
            id[at(c.0, c.1).unwrap()] = k;
            while let Some((x, y)) = stack.pop() {
                n += 1;
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                    if let Some(m) = at(x + dx, y + dy).filter(|&m| id[m] == u32::MAX - 1) {
                        id[m] = k;
                        stack.push((x + dx, y + dy));
                    }
                }
            }
            sizes.push(n);
        }
        // Each line's narrowest gap to another line (of 3 cells or more), and its ends.
        let mut gap: Vec<Option<(i32, (i32, i32), (i32, i32))>> = vec![None; sizes.len()];
        for &(x, y) in &cells {
            let k = id[at(x, y).unwrap()] as usize;
            for dy in -12..=12 {
                for dx in -12..=12 {
                    let Some(m) = at(x + dx, y + dy) else { continue };
                    let other = id[m];
                    if other >= u32::MAX - 1 || other as usize == k || sizes[other as usize] < 3 {
                        continue;
                    }
                    let d = dx.abs().max(dy.abs()) - 1;
                    if gap[k].is_none_or(|g| d < g.0) {
                        gap[k] = Some((d, (x, y), (x + dx, y + dy)));
                    }
                }
            }
        }
        let grid = build(&Map { walls: &map, trail: &[] }, (x0 as f64, y0 as f64), (x1 as f64, y1 as f64), &[]).unwrap();
        let shut = |a: (i32, i32), b: (i32, i32)| {
            let (pa, pb) = (grid.at((a.0 as f64, a.1 as f64)), grid.at((b.0 as f64, b.1 as f64)));
            traverse(pa, pb, |cell| !grid.open(cell))
        };
        let lines: Vec<usize> = (0..sizes.len()).filter(|&k| sizes[k] >= 20).collect();
        let narrow: Vec<usize> = lines.iter().copied().filter(|&k| gap[k].is_some_and(|g| g.0 <= 8)).collect();
        let closed = narrow.iter().filter(|&&k| gap[k].is_some_and(|g| shut(g.1, g.2))).count();
        let wider: Vec<usize> = lines.iter().copied().filter(|&k| gap[k].is_some_and(|g| (9..=11).contains(&g.0))).collect();
        let wider_closed = wider.iter().filter(|&&k| gap[k].is_some_and(|g| shut(g.1, g.2))).count();
        let mut widths: Vec<i32> = narrow.iter().map(|&k| gap[k].unwrap().0).collect();
        widths.sort_unstable();
        println!(
            "{} lines (20+ cells): {} with a gap of 1-8 cells to another line (widths {widths:?}; leaks without thickening), {closed} closed; \
             {} with a narrowest gap of 9-11, {wider_closed} closed",
            lines.len(),
            narrow.len(),
            wider.len()
        );
        assert_eq!(closed, narrow.len());
    }
}
