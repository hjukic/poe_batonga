//! The arrow: which way to walk to the next exit, from poe2way's collected zone
//! terrain (`data/layouts/poe2way/`) matched against what we have recorded of this visit.
//!
//! poe2way stores one recorded instance per zone: the walkable floor's outline and
//! points of interest in game units. The in-game minimap shows that world turned
//! 45°, so the terrain is turned once at load and then shares the minimap's screen
//! axes (x right, y down). An instance can still differ from poe2way's (another
//! layout, mirrored, another scale), so the recorded minimap walls must line up
//! with the outline before the arrow is shown. Once they do, the arrow follows a
//! walkable path around the walls rather than the straight line.

use serde::{Deserialize, Serialize};
use std::collections::{BinaryHeap, HashMap};

/// Bundled zones: (area id, objects JSON, outline SVG).
const ZONES: [(&str, &str, &str); 2] = [
    (
        "G1_town",
        include_str!("../../../data/layouts/poe2way/G1_town.json"),
        include_str!("../../../data/layouts/poe2way/G1_town.svg"),
    ),
    (
        "G1_2",
        include_str!("../../../data/layouts/poe2way/G1_2.json"),
        include_str!("../../../data/layouts/poe2way/G1_2.svg"),
    ),
];

/// Grid cell for walls and paths, in game units (poe2way traces outlines on a 3-unit grid).
const CELL: f64 = 6.0;
/// Recorded walls this many cells from the outline still count as on it.
const NEAR_WALL: i32 = 2;
/// Minimap cells per game unit to try when no visit has been measured yet. The
/// encampment visit measured 0.80; the minimap zoom is probably the same everywhere.
const SCALE_MIN: f64 = 0.55;
const SCALE_MAX: f64 = 1.3;
const SCALE_STEP: f64 = 1.04;
/// Fewer recorded walls than this say too little about the layout.
const MIN_WALLS: usize = 150;
/// At most this many recorded walls are scored per hypothesis.
const SCORE_WALLS: usize = 600;
/// Share of recorded walls that must lie on the outline to accept a match.
const MIN_FIT: f64 = 0.5;
/// Two directions closer than this count as agreeing.
const SAME_WAY: f64 = 45.0;
/// Minimum lead over the best hypothesis pointing elsewhere.
const MIN_CONFIDENCE: f32 = 0.3;
/// How far along the path the arrow looks, in grid cells.
const LOOKAHEAD: usize = 12;
/// How far (cells) to look for walkable floor when the player maps just outside it.
const SNAP: i32 = 4;

#[derive(Deserialize)]
struct ZoneFile {
    name: Option<String>,
    objects: Vec<ObjectFile>,
}

#[derive(Deserialize)]
struct ObjectFile {
    kind: String,
    x: f64,
    y: f64,
    label: Option<String>,
    destination: Option<Destination>,
}

#[derive(Deserialize)]
struct Destination {
    #[serde(rename = "areaId")]
    area_id: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Clone)]
struct Object {
    kind: String,
    label: String,
    destination: Option<String>,
    /// Screen-oriented game units.
    at: (f64, f64),
}

/// A zone's terrain on a grid of `CELL`-sized cells, in screen-oriented game units.
pub struct Zone {
    name: String,
    objects: Vec<Object>,
    origin: (f64, f64),
    cols: usize,
    rows: usize,
    walkable: Vec<bool>,
    /// Grid cells within `NEAR_WALL` of the outline.
    near_wall: Vec<bool>,
    /// Summed-area table of `near_wall`, (cols + 1) x (rows + 1).
    wall_sum: Vec<u32>,
}

/// The in-game map shows the world grid turned 45° with its y axis pointing up:
/// screen = ((x - y), -(x + y)) / √2. Fitted on a recorded Clearfell Encampment
/// visit (2026-11-12, 2,744 walls, 87% on the outline at 0.80 cells per unit).
fn to_screen((x, y): (f64, f64)) -> (f64, f64) {
    let c = std::f64::consts::FRAC_1_SQRT_2;
    (c * (x - y), -c * (x + y))
}

/// Outline rings from an SVG path made of M/L/Z commands.
fn parse_outline(svg: &str) -> Vec<Vec<(f64, f64)>> {
    let d = svg.split(" d=\"").nth(1).and_then(|s| s.split('"').next()).unwrap_or("");
    let mut rings = Vec::new();
    let mut ring: Vec<(f64, f64)> = Vec::new();
    let mut number = String::new();
    let mut pair: Vec<f64> = Vec::new();
    let flush = |number: &mut String, pair: &mut Vec<f64>, ring: &mut Vec<(f64, f64)>| {
        if let Ok(n) = number.parse() {
            pair.push(n);
        }
        number.clear();
        if pair.len() == 2 {
            ring.push((pair[0], pair[1]));
            pair.clear();
        }
    };
    for ch in d.chars() {
        match ch {
            'M' | 'L' | 'Z' | 'z' | ',' | ' ' => {
                flush(&mut number, &mut pair, &mut ring);
                if matches!(ch, 'M' | 'Z' | 'z') && !ring.is_empty() {
                    rings.push(std::mem::take(&mut ring));
                }
            }
            _ => number.push(ch),
        }
    }
    flush(&mut number, &mut pair, &mut ring);
    if !ring.is_empty() {
        rings.push(ring);
    }
    rings
}

impl Zone {
    /// `rings` and object positions are screen-oriented game units.
    fn new(name: String, rings: &[Vec<(f64, f64)>], objects: Vec<Object>) -> Zone {
        let points = rings.iter().flatten();
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for &(x, y) in points {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        let pad = CELL * (NEAR_WALL + 2) as f64;
        let origin = (x0 - pad, y0 - pad);
        let cols = ((x1 - origin.0 + pad) / CELL).ceil() as usize + 1;
        let rows = ((y1 - origin.1 + pad) / CELL).ceil() as usize + 1;

        // Even-odd fill at cell centres: inside the outline is walkable floor.
        let mut walkable = vec![false; cols * rows];
        for row in 0..rows {
            let y = origin.1 + (row as f64 + 0.5) * CELL;
            let mut crossings: Vec<f64> = Vec::new();
            for ring in rings {
                for (i, &(ax, ay)) in ring.iter().enumerate() {
                    let (bx, by) = ring[(i + 1) % ring.len()];
                    if (ay <= y) != (by <= y) {
                        crossings.push(ax + (y - ay) / (by - ay) * (bx - ax));
                    }
                }
            }
            crossings.sort_by(f64::total_cmp);
            for span in crossings.chunks(2) {
                let [from, to] = span else { continue };
                let c0 = ((from - origin.0) / CELL - 0.5).ceil().max(0.0) as usize;
                let c1 = ((to - origin.0) / CELL - 0.5).floor().min(cols as f64 - 1.0);
                if c1 < 0.0 {
                    continue;
                }
                for col in c0..=c1 as usize {
                    walkable[row * cols + col] = true;
                }
            }
        }

        // Cells the outline passes through, widened by `NEAR_WALL`.
        let mut edge = vec![false; cols * rows];
        for ring in rings {
            for (i, &(ax, ay)) in ring.iter().enumerate() {
                let (bx, by) = ring[(i + 1) % ring.len()];
                let steps = ((bx - ax).hypot(by - ay) / (CELL / 2.0)).ceil().max(1.0) as usize;
                for s in 0..=steps {
                    let t = s as f64 / steps as f64;
                    let col = ((ax + t * (bx - ax) - origin.0) / CELL) as usize;
                    let row = ((ay + t * (by - ay) - origin.1) / CELL) as usize;
                    if col < cols && row < rows {
                        edge[row * cols + col] = true;
                    }
                }
            }
        }
        let mut near_wall = vec![false; cols * rows];
        for row in 0..rows as i32 {
            for col in 0..cols as i32 {
                if !edge[row as usize * cols + col as usize] {
                    continue;
                }
                for dy in -NEAR_WALL..=NEAR_WALL {
                    for dx in -NEAR_WALL..=NEAR_WALL {
                        let (c, r) = (col + dx, row + dy);
                        if c >= 0 && r >= 0 && (c as usize) < cols && (r as usize) < rows {
                            near_wall[r as usize * cols + c as usize] = true;
                        }
                    }
                }
            }
        }
        let mut wall_sum = vec![0u32; (cols + 1) * (rows + 1)];
        for row in 0..rows {
            for col in 0..cols {
                wall_sum[(row + 1) * (cols + 1) + col + 1] = u32::from(near_wall[row * cols + col])
                    + wall_sum[row * (cols + 1) + col + 1]
                    + wall_sum[(row + 1) * (cols + 1) + col]
                    - wall_sum[row * (cols + 1) + col];
            }
        }
        Zone { name, objects, origin, cols, rows, walkable, near_wall, wall_sum }
    }

    /// Share of the box (game units) that lies near the outline; outside the grid counts as not.
    fn wall_share(&self, (x0, y0): (f64, f64), (x1, y1): (f64, f64)) -> f64 {
        let to_col = |x: f64| ((x - self.origin.0) / CELL).floor();
        let to_row = |y: f64| ((y - self.origin.1) / CELL).floor();
        let (c0, c1, r0, r1) = (to_col(x0), to_col(x1) + 1.0, to_row(y0), to_row(y1) + 1.0);
        let area = (c1 - c0) * (r1 - r0);
        let clamp_c = |c: f64| c.clamp(0.0, self.cols as f64) as usize;
        let clamp_r = |r: f64| r.clamp(0.0, self.rows as f64) as usize;
        let (c0, c1, r0, r1) = (clamp_c(c0), clamp_c(c1), clamp_r(r0), clamp_r(r1));
        let at = |c: usize, r: usize| self.wall_sum[r * (self.cols + 1) + c] as f64;
        let inside = at(c1, r1) - at(c0, r1) - at(c1, r0) + at(c0, r0);
        if area > 0.0 { inside / area } else { 1.0 }
    }

    fn load(json: &str, svg: &str) -> Zone {
        let file: ZoneFile = serde_json::from_str(json).expect("bundled terrain is valid");
        let rings: Vec<Vec<(f64, f64)>> =
            parse_outline(svg).into_iter().map(|r| r.into_iter().map(to_screen).collect()).collect();
        let objects = file
            .objects
            .into_iter()
            .map(|o| Object {
                destination: o.destination.as_ref().and_then(|d| d.area_id.clone()),
                label: o
                    .destination
                    .and_then(|d| d.name)
                    .or(o.label)
                    .unwrap_or_else(|| o.kind.clone()),
                kind: o.kind,
                at: to_screen((o.x, o.y)),
            })
            .collect();
        Zone::new(file.name.unwrap_or_default(), &rings, objects)
    }

    fn cell(&self, (x, y): (f64, f64)) -> Option<(usize, usize)> {
        let (col, row) = ((x - self.origin.0) / CELL, (y - self.origin.1) / CELL);
        (col >= 0.0 && row >= 0.0 && (col as usize) < self.cols && (row as usize) < self.rows)
            .then(|| (col as usize, row as usize))
    }

    fn centre(&self, (col, row): (usize, usize)) -> (f64, f64) {
        (self.origin.0 + (col as f64 + 0.5) * CELL, self.origin.1 + (row as f64 + 0.5) * CELL)
    }

    fn on_wall(&self, at: (f64, f64)) -> bool {
        self.cell(at).is_some_and(|(c, r)| self.near_wall[r * self.cols + c])
    }

    /// Walking distance (in cell steps x 10) from every cell to the nearest of `goals`.
    fn distances(&self, goals: &[(f64, f64)]) -> Vec<u32> {
        let mut dist = vec![u32::MAX; self.cols * self.rows];
        let mut queue = BinaryHeap::new();
        for &goal in goals {
            if let Some((c, r)) = self.snap(goal) {
                dist[r * self.cols + c] = 0;
                queue.push(std::cmp::Reverse((0u32, c, r)));
            }
        }
        while let Some(std::cmp::Reverse((d, c, r))) = queue.pop() {
            if d > dist[r * self.cols + c] {
                continue;
            }
            for (dx, dy, cost) in NEIGHBOURS {
                let (nc, nr) = (c as i32 + dx, r as i32 + dy);
                if nc < 0 || nr < 0 || nc as usize >= self.cols || nr as usize >= self.rows {
                    continue;
                }
                let i = nr as usize * self.cols + nc as usize;
                if self.walkable[i] && d + cost < dist[i] {
                    dist[i] = d + cost;
                    queue.push(std::cmp::Reverse((d + cost, nc as usize, nr as usize)));
                }
            }
        }
        dist
    }

    /// The nearest walkable cell within `SNAP` cells.
    fn snap(&self, at: (f64, f64)) -> Option<(usize, usize)> {
        let (x, y) = ((at.0 - self.origin.0) / CELL, (at.1 - self.origin.1) / CELL);
        let mut best = None;
        for dy in -SNAP..=SNAP {
            for dx in -SNAP..=SNAP {
                let (c, r) = (x as i32 + dx, y as i32 + dy);
                if c < 0 || r < 0 || c as usize >= self.cols || r as usize >= self.rows {
                    continue;
                }
                if self.walkable[r as usize * self.cols + c as usize] {
                    let d = dx * dx + dy * dy;
                    if best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, (c as usize, r as usize)));
                    }
                }
            }
        }
        best.map(|(_, cell)| cell)
    }

    /// A point `LOOKAHEAD` cells along the shortest path from `from`, and the remaining
    /// path length in game units. None when no path is known.
    fn path_ahead(&self, dist: &[u32], from: (f64, f64)) -> Option<((f64, f64), f64)> {
        let (mut c, mut r) = self.snap(from)?;
        let total = dist[r * self.cols + c];
        if total == u32::MAX {
            return None;
        }
        for _ in 0..LOOKAHEAD {
            let here = dist[r * self.cols + c];
            let next = NEIGHBOURS
                .iter()
                .filter_map(|&(dx, dy, _)| {
                    let (nc, nr) = (c as i32 + dx, r as i32 + dy);
                    (nc >= 0 && nr >= 0 && (nc as usize) < self.cols && (nr as usize) < self.rows)
                        .then(|| (nc as usize, nr as usize))
                })
                .min_by_key(|&(nc, nr)| dist[nr * self.cols + nc]);
            match next {
                Some((nc, nr)) if dist[nr * self.cols + nc] < here => (c, r) = (nc, nr),
                _ => break,
            }
        }
        Some((self.centre((c, r)), total as f64 / 10.0 * CELL))
    }
}

const NEIGHBOURS: [(i32, i32, u32); 8] =
    [(1, 0, 10), (-1, 0, 10), (0, 1, 10), (0, -1, 10), (1, 1, 14), (1, -1, 14), (-1, 1, 14), (-1, -1, 14)];

pub struct Catalog {
    zones: HashMap<String, Zone>,
}

impl Catalog {
    pub fn load() -> Catalog {
        Catalog { zones: ZONES.iter().map(|&(id, json, svg)| (id.to_string(), Zone::load(json, svg))).collect() }
    }
}

const H: f64 = std::f64::consts::FRAC_1_SQRT_2;
/// Turns in 45° steps, then the same mirrored, as 2x2 matrices [a, b, c, d]:
/// (x, y) -> (ax + by, cx + dy). The 45° steps also cover a wrong guess about the
/// in-game map's turn (`to_screen`).
const TURNS: [[f64; 4]; 16] = [
    [1.0, 0.0, 0.0, 1.0],
    [H, -H, H, H],
    [0.0, -1.0, 1.0, 0.0],
    [-H, -H, H, -H],
    [-1.0, 0.0, 0.0, -1.0],
    [-H, H, -H, -H],
    [0.0, 1.0, -1.0, 0.0],
    [H, H, -H, H],
    [-1.0, 0.0, 0.0, 1.0],
    [-H, -H, -H, H],
    [0.0, -1.0, -1.0, 0.0],
    [H, -H, -H, -H],
    [1.0, 0.0, 0.0, -1.0],
    [H, H, H, -H],
    [0.0, 1.0, 1.0, 0.0],
    [-H, H, H, H],
];

fn turn(t: &[f64; 4], (x, y): (f64, f64)) -> (f64, f64) {
    (t[0] * x + t[1] * y, t[2] * x + t[3] * y)
}

/// The inverse of a rotation/mirror is its transpose.
fn unturn(t: &[f64; 4], (x, y): (f64, f64)) -> (f64, f64) {
    (t[0] * x + t[2] * y, t[1] * x + t[3] * y)
}

fn angle_between(a: f32, b: f32) -> f64 {
    let d = ((a - b) as f64).rem_euclid(360.0);
    d.min(360.0 - d)
}

/// Screen direction in degrees (0 = right, 90 = down). A coincident target has none.
fn screen_bearing(dx: f64, dy: f64) -> Option<f32> {
    (dx.hypot(dy) > 1e-6).then(|| dy.atan2(dx).to_degrees() as f32)
}

/// "G1_2" < "G1_11": compare the numbers in area ids as numbers.
fn area_order(id: &str) -> Vec<(u32, String)> {
    id.split('_').map(|part| (part.parse().unwrap_or(u32::MAX), part.to_string())).collect()
}

/// What we know about the current visit, in minimap cells relative to the arrival point.
pub struct Evidence<'a> {
    pub area: &'a str,
    /// Area id the player came from; the arrival point is that zone's exit (or a waypoint).
    pub arrived_from: &'a str,
    pub walls: &'a [(i32, i32)],
    pub player: (i32, i32),
    /// Explicit objective completion from the checklist.
    pub completed: &'a [String],
    /// Minimap cells per game unit, measured on earlier visits, if any.
    pub scale: Option<f64>,
}

#[derive(Clone, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GuideView {
    pub variant: String,
    /// "entrance" for zone exits.
    pub target_kind: String,
    pub target: String,
    /// Screen direction in degrees: 0 = right, 90 = down.
    pub degrees: Option<f32>,
    /// "estimated" (walls match the terrain) or "unconfirmed".
    pub source: String,
    /// Walking distance in minimap cells.
    pub distance: f32,
    /// 0-1: how far ahead this match is of the best one pointing elsewhere.
    pub confidence: f32,
    /// The scale was measured on an earlier visit, not fitted now.
    pub measured: bool,
    /// The instance is rotated/mirrored relative to poe2way's.
    pub turned: bool,
}

/// One way the recorded visit could sit on the terrain: started at `anchor`,
/// turned by `turn`, `scale` minimap cells per game unit.
#[derive(Clone, Copy)]
struct Fit {
    anchor: (f64, f64),
    turn: usize,
    scale: f64,
    score: f64,
}

impl Fit {
    /// Minimap cells (relative to arrival) to screen-oriented game units.
    fn to_zone(&self, (x, y): (f64, f64)) -> (f64, f64) {
        let (u, v) = unturn(&TURNS[self.turn], (x / self.scale, y / self.scale));
        (self.anchor.0 + u, self.anchor.1 + v)
    }
}

/// Where the player can have arrived: the exits to the zone they came from; else
/// exits whose destination poe2way does not know, and waypoints.
fn anchors(zone: &Zone, arrived_from: &str) -> Vec<(f64, f64)> {
    let exits: Vec<(f64, f64)> = zone
        .objects
        .iter()
        .filter(|o| o.kind == "transition" && o.destination.as_deref() == Some(arrived_from))
        .map(|o| o.at)
        .collect();
    if !exits.is_empty() {
        return exits;
    }
    zone.objects
        .iter()
        .filter(|o| o.kind == "waypoint" || (o.kind == "transition" && o.destination.is_none()))
        .map(|o| o.at)
        .collect()
}

/// The next exit: onward to the lowest-numbered zone not just come from or done.
fn next_exit<'z>(zone: &'z Zone, arrived_from: &str, completed: &[String]) -> Option<(String, Vec<&'z Object>)> {
    let done = |o: &Object| completed.iter().any(|c| c.eq_ignore_ascii_case(&o.label));
    let next = zone
        .objects
        .iter()
        .filter(|o| o.kind == "transition" && !done(o))
        .filter_map(|o| o.destination.clone())
        .filter(|d| d != arrived_from && !d.ends_with("_town"))
        .min_by_key(|d| area_order(d))?;
    let exits = zone.objects.iter().filter(|o| o.destination.as_deref() == Some(&next)).collect();
    Some((next, exits))
}

fn sample(walls: &[(i32, i32)]) -> Vec<(f64, f64)> {
    let step = walls.len().div_ceil(SCORE_WALLS).max(1);
    walls.iter().step_by(step).map(|&(x, y)| (x as f64, y as f64)).collect()
}

/// Every placement of the visit on the terrain, scored by how many more recorded
/// walls lie on the outline than chance would put there (the outline's share of the
/// area they cover): squeezing the walls into a narrow corridor scores nothing. Best first.
fn fits(zone: &Zone, ev: &Evidence) -> Vec<Fit> {
    let walls = sample(ev.walls);
    let scales: Vec<f64> = match ev.scale {
        Some(s) => vec![s],
        None => std::iter::successors(Some(SCALE_MIN), |s| Some(s * SCALE_STEP)).take_while(|&s| s <= SCALE_MAX).collect(),
    };
    let mut out = Vec::new();
    for anchor in anchors(zone, ev.arrived_from) {
        for turn in 0..TURNS.len() {
            for &scale in &scales {
                let mut fit = Fit { anchor, turn, scale, score: 0.0 };
                let (mut lo, mut hi, mut hits) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN), 0);
                for &w in &walls {
                    let at = fit.to_zone(w);
                    (lo, hi) = ((lo.0.min(at.0), lo.1.min(at.1)), (hi.0.max(at.0), hi.1.max(at.1)));
                    hits += usize::from(zone.on_wall(at));
                }
                let found = hits as f64 / walls.len().max(1) as f64;
                let chance = zone.wall_share(lo, hi);
                fit.score = ((found - chance) / (1.0 - chance).max(1e-9)).max(0.0);
                out.push(fit);
            }
        }
    }
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out
}

pub fn guide(catalog: &Catalog, ev: &Evidence) -> Option<GuideView> {
    let zone = catalog.zones.get(ev.area)?;
    let (_, exits) = next_exit(zone, ev.arrived_from, ev.completed)?;
    let target = exits.first()?.label.clone();
    let goals: Vec<(f64, f64)> = exits.iter().map(|o| o.at).collect();
    let mut view = GuideView {
        variant: format!("poe2way {}", zone.name),
        target_kind: "entrance".into(),
        target,
        degrees: None,
        source: "unconfirmed".into(),
        distance: 0.0,
        confidence: 0.0,
        measured: ev.scale.is_some(),
        turned: false,
    };
    if ev.walls.len() < MIN_WALLS || anchors(zone, ev.arrived_from).is_empty() {
        return Some(view);
    }

    let dist = zone.distances(&goals);
    let player = (ev.player.0 as f64, ev.player.1 as f64);
    // Screen bearing and walking distance (minimap cells) under one placement.
    let heading = |fit: &Fit| -> Option<(f32, f64)> {
        let at = fit.to_zone(player);
        let (ahead, length) = zone.path_ahead(&dist, at).or_else(|| {
            let goal = goals.iter().min_by(|a, b| {
                (a.0 - at.0).hypot(a.1 - at.1).total_cmp(&(b.0 - at.0).hypot(b.1 - at.1))
            })?;
            Some((*goal, (goal.0 - at.0).hypot(goal.1 - at.1)))
        })?;
        let (dx, dy) = turn(&TURNS[fit.turn], (ahead.0 - at.0, ahead.1 - at.1));
        Some((screen_bearing(dx, dy)?, length * fit.scale))
    };

    let all = fits(zone, ev);
    let best = *all.first()?;
    let Some((degrees, distance)) = heading(&best) else { return Some(view) };
    view.distance = distance as f32;
    view.turned = best.turn != 0;
    // The best placement pointing another way, among the clearly different ones.
    let rival = all
        .iter()
        .skip(1)
        .filter(|f| f.score > 0.0)
        .take(64)
        .find(|f| heading(f).is_none_or(|(d, _)| angle_between(d, degrees) > SAME_WAY))
        .map_or(0.0, |f| f.score);
    view.confidence = (1.0 - rival / best.score.max(1e-9)) as f32;
    if best.score >= MIN_FIT && view.confidence >= MIN_CONFIDENCE {
        view.degrees = Some(degrees);
        view.source = "estimated".into();
    }
    Some(view)
}

/// Minimap cells per game unit for a finished visit, if its walls clearly match the terrain.
pub fn measure_scale(catalog: &Catalog, area: &str, arrived_from: &str, walls: &[(i32, i32)]) -> Option<f64> {
    let zone = catalog.zones.get(area)?;
    if walls.len() < MIN_WALLS {
        return None;
    }
    let ev = Evidence { area, arrived_from, walls, player: (0, 0), completed: &[], scale: None };
    let all = fits(zone, &ev);
    let best = all.first()?;
    // Another placement nearly as good at a different scale makes the scale unclear.
    let rival = all.iter().skip(1).find(|f| (f.scale / best.scale - 1.0).abs() > 0.1).map_or(0.0, |f| f.score);
    (best.score >= MIN_FIT && rival < best.score * (1.0 - MIN_CONFIDENCE as f64)).then_some(best.scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev<'a>(area: &'a str, from: &'a str, walls: &'a [(i32, i32)]) -> Evidence<'a> {
        Evidence { area, arrived_from: from, walls, player: (0, 0), completed: &[], scale: None }
    }

    /// Minimap walls a visit would record: the outline near the arrival point, as
    /// placed by `fit` (the inverse of `Fit::to_zone`).
    fn recorded_walls(zone: &Zone, fit: &Fit, radius: f64) -> Vec<(i32, i32)> {
        let mut walls = Vec::new();
        for row in 0..zone.rows {
            for col in 0..zone.cols {
                let i = row * zone.cols + col;
                // Cells on the outline itself: walkable next to unwalkable.
                let edge = zone.walkable[i]
                    && NEIGHBOURS.iter().any(|&(dx, dy, _)| {
                        let (c, r) = (col as i32 + dx, row as i32 + dy);
                        c >= 0 && r >= 0 && !zone.walkable[r as usize * zone.cols + c as usize]
                    });
                let at = zone.centre((col, row));
                if !edge || (at.0 - fit.anchor.0).hypot(at.1 - fit.anchor.1) > radius {
                    continue;
                }
                let (x, y) = turn(&TURNS[fit.turn], (at.0 - fit.anchor.0, at.1 - fit.anchor.1));
                walls.push(((x * fit.scale).round() as i32, (y * fit.scale).round() as i32));
            }
        }
        walls.sort_unstable();
        walls.dedup();
        walls
    }

    fn town_exit(zone: &Zone) -> (f64, f64) {
        zone.objects.iter().find(|o| o.destination.as_deref() == Some("G1_town")).unwrap().at
    }

    #[test]
    fn bundled_terrain_loads() {
        let catalog = Catalog::load();
        let clearfell = &catalog.zones["G1_2"];
        assert_eq!(clearfell.name, "Clearfell");
        let floor = clearfell.walkable.iter().filter(|&&w| w).count();
        assert!(floor > 10_000, "{floor} walkable cells");
        // Points of interest stand on (or right next to) the floor.
        for o in &clearfell.objects {
            assert!(clearfell.snap(o.at).is_some(), "{} off the floor", o.label);
        }
        assert!(catalog.zones.contains_key("G1_town"));
        assert!(!catalog.zones.contains_key("G1_1"));
    }

    #[test]
    fn outline_parses_into_closed_rings() {
        let rings = parse_outline(r#"<svg><path d="M0,0L10,0L10,10L0,10ZM2,2L4,2L4,4Z"/></svg>"#);
        assert_eq!(rings, vec![vec![(0., 0.), (10., 0.), (10., 10.), (0., 10.)], vec![(2., 2.), (4., 2.), (4., 4.)]]);
    }

    #[test]
    fn picks_the_onward_exit() {
        let catalog = Catalog::load();
        let clearfell = &catalog.zones["G1_2"];
        assert_eq!(next_exit(clearfell, "G1_town", &[]).unwrap().0, "G1_3");
        assert_eq!(next_exit(clearfell, "G1_3", &[]).unwrap().0, "G1_4");
        assert_eq!(next_exit(clearfell, "G1_town", &["Mud Burrow".into()]).unwrap().0, "G1_4");
        let town = &catalog.zones["G1_town"];
        let (next, exits) = next_exit(town, "G1_1", &[]).unwrap();
        assert_eq!(next, "G1_2");
        assert!(exits.len() >= 2, "every Clearfell exit is a goal");
        assert!(next_exit(town, "G1_2", &[]).is_none());
    }

    #[test]
    fn no_walls_yet_names_the_target_without_a_direction() {
        let view = guide(&Catalog::load(), &ev("G1_2", "G1_town", &[])).unwrap();
        assert_eq!(view.target, "Mud Burrow");
        assert_eq!(view.degrees, None);
        assert_eq!(view.source, "unconfirmed");
    }

    #[test]
    fn matching_walls_confirm_the_layout_and_scale() {
        let catalog = Catalog::load();
        let zone = &catalog.zones["G1_2"];
        let fit = Fit { anchor: town_exit(zone), turn: 0, scale: 0.8, score: 0.0 };
        let walls = recorded_walls(zone, &fit, 450.0);
        let view = guide(&catalog, &ev("G1_2", "G1_town", &walls)).unwrap();
        assert_eq!(view.source, "estimated", "{view:?}");
        assert!(view.degrees.is_some());
        assert!(!view.turned);
        let scale = measure_scale(&catalog, "G1_2", "G1_town", &walls).unwrap();
        assert!((scale / 0.8 - 1.0).abs() < 0.1, "scale {scale}");
    }

    #[test]
    fn a_mirrored_instance_is_recognised() {
        let catalog = Catalog::load();
        let zone = &catalog.zones["G1_2"];
        let fit = Fit { anchor: town_exit(zone), turn: 8, scale: 0.8, score: 0.0 };
        let walls = recorded_walls(zone, &fit, 450.0);
        let view = guide(&catalog, &ev("G1_2", "G1_town", &walls)).unwrap();
        assert_eq!(view.source, "estimated", "{view:?}");
        assert!(view.turned);
    }

    #[test]
    fn walls_of_another_layout_do_not_give_a_direction() {
        // A plain grid of walls fits no part of Clearfell well.
        let walls: Vec<(i32, i32)> = (0..40).flat_map(|i| [(i * 7 - 140, 0), (0, i * 7 - 140)]).chain(
            (0..120).map(|i| ((i * 37 % 200) - 100, (i * 53 % 200) - 100))).collect();
        let view = guide(&Catalog::load(), &ev("G1_2", "G1_town", &walls)).unwrap();
        assert_eq!(view.degrees, None, "{view:?}");
    }

    #[test]
    fn the_arrow_follows_the_path_around_a_wall() {
        // A U-shaped corridor: start bottom-left, goal bottom-right, a wall between.
        let ring = vec![(0., 0.), (300., 0.), (300., 300.), (200., 300.), (200., 60.), (100., 60.), (100., 300.), (0., 300.)];
        let objects = vec![
            Object { kind: "transition".into(), label: "Back".into(), destination: Some("A".into()), at: (50., 280.) },
            Object { kind: "transition".into(), label: "Onward".into(), destination: Some("B".into()), at: (250., 280.) },
        ];
        let zone = Zone::new("U".into(), &[ring], objects);
        let dist = zone.distances(&[(250., 280.)]);
        let (ahead, length) = zone.path_ahead(&dist, (50., 280.)).unwrap();
        // Straight to the goal is right (0°); the path first heads up the left arm.
        let bearing = screen_bearing(ahead.0 - 50., ahead.1 - 280.).unwrap();
        assert!((-135.0..=-45.0).contains(&bearing), "bearing {bearing}");
        assert!(length > 500.0, "path length {length}");
    }

    #[test]
    fn screen_bearings_match_human_directions() {
        for (x, y, degrees) in [(10., 0., 0.), (10., 10., 45.), (0., 10., 90.),
            (-10., 10., 135.), (-10., 0., 180.), (-10., -10., -135.),
            (0., -10., -90.), (10., -10., -45.)] {
            assert_eq!(screen_bearing(x, y), Some(degrees));
        }
        assert_eq!(screen_bearing(0., 0.), None);
    }

    #[test]
    fn area_ids_sort_by_number() {
        assert!(area_order("G1_3") < area_order("G1_11"));
        assert!(area_order("G1_2") < area_order("G1_3"));
    }
}

/// Runs the arrow on a real recording's first part:
/// `LAYOUT_FILE=<json> FROM=G1_town cargo test --release guide_on_recording -- --ignored --nocapture`.
#[cfg(test)]
#[test]
#[ignore]
fn guide_on_recording() {
    let path = std::env::var("LAYOUT_FILE").expect("set LAYOUT_FILE");
    let from = std::env::var("FROM").unwrap_or_default();
    let recording: super::Recording = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let catalog = Catalog::load();
    for part in &recording.parts {
        let Some(arrival) = part.pois.iter().find(|p| p.kind == "arrived") else { continue };
        let walls: Vec<(i32, i32)> =
            part.walls.iter().filter(|w| w[2] >= 2).map(|w| (w[0] - arrival.x, w[1] - arrival.y)).collect();
        let player = part.trail.last().map_or((0, 0), |t| (t[0] - arrival.x, t[1] - arrival.y));
        let ev = Evidence { area: &recording.area, arrived_from: &from, walls: &walls, player, completed: &[], scale: None };
        let zone = &catalog.zones[recording.area.as_str()];
        let all = fits(zone, &ev);
        eprintln!("{} walls, player {:?}", walls.len(), player);
        for f in all.iter().take(10) {
            eprintln!("  turn {} scale {:.3} score {:.3} anchor {:?}", f.turn, f.scale, f.score, f.anchor);
        }
        eprintln!("{:?}", guide(&catalog, &ev));
    }
}
