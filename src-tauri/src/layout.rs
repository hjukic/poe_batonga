//! Layout recorder: stitches live minimap frames into a map of each zone visit and
//! pins points of interest from the game log (arrival, departure, NPC/boss lines).
//!
//! The minimap is centred on the player, so each frame's walls are matched against
//! the map so far to find how far the player moved. Recordings are saved per zone
//! instance (area + seed) under the app data folder, `layouts/<area>/<seed>.json`.
//! A visit that cannot be related to earlier parts of the same instance (another
//! entry, a teleport, tracking lost) starts a new part with its own coordinates.

mod graph;
pub mod guide;

use crate::minimap::MinimapView;
use crate::tracker::LogEvent;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

const FORMAT_VERSION: u32 = 1;
/// Frames with fewer wall cells carry too little shape to place reliably.
const MIN_FRAME_WALLS: usize = 30;
/// Share of a frame's wall cells that must land on known walls to accept a match.
const MIN_MATCH: f64 = 0.55;
/// Search radius in cells around the predicted move, then a wider retry.
const NEAR: i32 = 12;
const WIDE: i32 = 32;
/// Consecutive unmatched frames before a new part is started.
const LOST_LIMIT: u32 = 9;
const SAVE_EVERY: Duration = Duration::from_secs(15);
/// Zones the arrow is enabled for while it is being tuned.
const GUIDED: [&str; 3] = ["G1_1", "G1_town", "G1_2"];
/// Minimap cells per game unit measured per area (poe2way terrain; the older
/// `scales.json` held Qutm diagram scales and is no longer read).
const SCALES_FILE: &str = "terrain-scales.json";
/// Joining pieces of a visit: how often to try, the least shape to try with, the
/// coarse grid for the first search and the share of walls that must line up.
const MERGE_EVERY: Duration = Duration::from_secs(3);
const MERGE_MIN_WALLS: usize = 400;
/// A piece can be smaller: one minimap frame shows about 250-1000 wall cells.
const MERGE_MIN_PIECE: usize = 200;
const MERGE_COARSE: i32 = 6;
const MERGE_MATCH: f64 = 0.6;
/// At most this many wall cells are used to score the arrow's hypotheses.
const GUIDE_WALLS: usize = 3000;
/// Size of the overview grid sent to the overlay.
pub const VIEW_COLS: usize = 300;
pub const VIEW_ROWS: usize = 260;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Poi {
    /// "arrived", "left", "npc".
    pub kind: String,
    pub label: String,
    pub x: i32,
    pub y: i32,
    /// Unix seconds.
    pub at: u64,
}

#[derive(Clone, Serialize, Deserialize, Default, Debug)]
pub struct Part {
    pub started_at: u64,
    /// [x, y, times seen], in minimap cells relative to where the part started.
    pub walls: Vec<[i32; 3]>,
    pub trail: Vec<[i32; 2]>,
    pub pois: Vec<Poi>,
    pub frames: u32,
    pub matched: u32,
    pub lost: u32,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    pub version: u32,
    pub area: String,
    pub level: u32,
    pub seed: Option<u64>,
    /// Minimap pixels per cell, and the capture size those pixels come from.
    pub cell_px: usize,
    pub capture_px: [usize; 2],
    pub parts: Vec<Part>,
}

/// The part being recorded, with walls in a map for quick lookup.
struct Live {
    walls: HashMap<(i32, i32), u32>,
    pos: (i32, i32),
    velocity: (i32, i32),
    lost_streak: u32,
    /// Player cell inside the frame (the minimap is centred on the player).
    centre: Option<(i32, i32)>,
    part: Part,
}

impl Live {
    fn new() -> Self {
        Live {
            walls: HashMap::new(),
            pos: (0, 0),
            velocity: (0, 0),
            lost_streak: 0,
            centre: None,
            part: Part { started_at: now(), ..Part::default() },
        }
    }

    fn snapshot(&self) -> Part {
        let mut part = self.part.clone();
        let mut walls: Vec<[i32; 3]> = self.walls.iter().map(|(&(x, y), &n)| [x, y, n as i32]).collect();
        walls.sort_unstable();
        part.walls = walls;
        part
    }

    fn add_poi(&mut self, kind: &str, label: &str) {
        let (x, y) = self.pos;
        self.part.pois.push(Poi { kind: kind.into(), label: label.into(), x, y, at: now() });
    }
}

struct Session {
    recording: Recording,
    /// Parts already finished in this instance (kept in `recording.parts`); `live` is extra.
    live: Live,
    /// The arrival POI waits for the first placed frame. Some("") is a live arrival
    /// from an unknown zone (e.g. after logging in); None means the app started mid-zone.
    arrived_from: Option<String>,
    started: bool,
    file: Option<PathBuf>,
    /// Layout recognised on an earlier piece of this instance (worked out once).
    layout_hint: std::cell::OnceCell<Option<String>>,
    /// The last arrow of this visit, kept while a new one cannot be worked out.
    last_guide: std::cell::RefCell<Option<guide::GuideView>>,
}

struct State {
    catalog: guide::Catalog,
    completed: HashMap<String, Vec<String>>,
    /// Open checklist targets per area, in checklist order.
    targets: HashMap<String, Vec<guide::NavTarget>>,
    /// Measured minimap cells per game unit, per area.
    scales: HashMap<String, Vec<f64>>,
    dir: Option<PathBuf>,
    session: Option<Session>,
    /// Area left most recently, for the next zone's "arrived" point.
    previous_area: Option<String>,
    last_save: Option<Instant>,
    last_view: Option<Instant>,
    last_merge: Option<Instant>,
}

pub struct Recorder(Mutex<State>);

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ViewPoi {
    kind: String,
    label: String,
    x: f32,
    y: f32,
}

/// Overview of the part being recorded, scaled to fit `VIEW_COLS` x `VIEW_ROWS`.
#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LayoutView {
    area: String,
    seed: Option<u64>,
    part: usize,
    frames: u32,
    matched: u32,
    cols: usize,
    rows: usize,
    walls: Vec<[u16; 2]>,
    trail: Vec<[f32; 2]>,
    player: [f32; 2],
    pois: Vec<ViewPoi>,
    guide: Option<guide::GuideView>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Finds how far the player moved: the shift that puts most frame walls on known walls.
/// `rel` are frame wall cells relative to the player. Returns (shift, share matched).
fn register(
    walls: &HashMap<(i32, i32), u32>,
    rel: &[(i32, i32)],
    centre: (i32, i32),
    radius: i32,
) -> ((i32, i32), f64) {
    let (min_x, max_x) = rel.iter().fold((i32::MAX, i32::MIN), |(a, b), &(x, _)| (a.min(x), b.max(x)));
    let (min_y, max_y) = rel.iter().fold((i32::MAX, i32::MIN), |(a, b), &(_, y)| (a.min(y), b.max(y)));
    // Dense window of known walls covering every candidate: 2 on a wall, 1 next to one
    // (absorbs jitter, while exact overlap still wins).
    let x0 = centre.0 - radius + min_x - 1;
    let y0 = centre.1 - radius + min_y - 1;
    let w = (max_x - min_x + 2 * radius + 3) as usize;
    let h = (max_y - min_y + 2 * radius + 3) as usize;
    let mut window = vec![0u8; w * h];
    // Visit whichever is smaller: the stored walls, or the window's own cells.
    let inside: Vec<(i32, i32)> = if walls.len() <= w * h {
        walls.keys().copied().collect()
    } else {
        (0..h as i32)
            .flat_map(|y| (0..w as i32).map(move |x| (x + x0, y + y0)))
            .filter(|cell| walls.contains_key(cell))
            .collect()
    };
    for (x, y) in inside {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (wx, wy) = (x + dx - x0, y + dy - y0);
                if wx >= 0 && wy >= 0 && (wx as usize) < w && (wy as usize) < h {
                    let cell = &mut window[wy as usize * w + wx as usize];
                    *cell = (*cell).max(if dx == 0 && dy == 0 { 2 } else { 1 });
                }
            }
        }
    }
    let mut best = ((centre.0, centre.1), 0usize, i32::MAX, 0usize);
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let (px, py) = (centre.0 + dx, centre.1 + dy);
            let (mut score, mut near) = (0usize, 0usize);
            for &(rx, ry) in rel {
                let v = window[(py + ry - y0) as usize * w + (px + rx - x0) as usize];
                score += v as usize;
                near += (v > 0) as usize;
            }
            let distance = dx * dx + dy * dy;
            if score > best.1 || (score == best.1 && distance < best.2) {
                best = ((px, py), score, distance, near);
            }
        }
    }
    (best.0, best.3 as f64 / rel.len().max(1) as f64)
}

/// Finds where a separately recorded piece of the same visit lies on the current map:
/// a coarse search over every overlap, then a fine one around the best few.
/// `piece` walls are in the piece's own coordinates; returns the shift onto `walls`.
fn locate_piece(walls: &HashMap<(i32, i32), u32>, piece: &[(i32, i32)]) -> Option<(i32, i32)> {
    if walls.len() < MERGE_MIN_WALLS || piece.len() < MERGE_MIN_PIECE {
        return None;
    }
    let c = MERGE_COARSE;
    let coarse = |(x, y): (i32, i32)| (x.div_euclid(c), y.div_euclid(c));
    let mut map: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    for &cell in walls.keys() {
        map.insert(coarse(cell));
    }
    let mut cells: Vec<(i32, i32)> = piece.iter().map(|&cell| coarse(cell)).collect();
    cells.sort_unstable();
    cells.dedup();
    let bounds = |it: &mut dyn Iterator<Item = (i32, i32)>| {
        it.fold((i32::MAX, i32::MAX, i32::MIN, i32::MIN), |(a, b, cc, d), (x, y)| (a.min(x), b.min(y), cc.max(x), d.max(y)))
    };
    let (mx0, my0, mx1, my1) = bounds(&mut map.iter().copied());
    let (px0, py0, px1, py1) = bounds(&mut cells.iter().copied());
    // Dense copy of the coarse map for fast lookups.
    let (w, h) = ((mx1 - mx0 + 1) as usize, (my1 - my0 + 1) as usize);
    let mut grid = vec![false; w * h];
    for &(x, y) in &map {
        grid[(y - my0) as usize * w + (x - mx0) as usize] = true;
    }
    let mut scored = Vec::new();
    for dy in (my0 - py1)..=(my1 - py0) {
        for dx in (mx0 - px1)..=(mx1 - px0) {
            let hits = cells
                .iter()
                .filter(|&&(x, y)| {
                    let (gx, gy) = (x + dx - mx0, y + dy - my0);
                    gx >= 0 && gy >= 0 && (gx as usize) < w && (gy as usize) < h && grid[gy as usize * w + gx as usize]
                })
                .count();
            if hits * 4 >= cells.len() {
                scored.push((hits, (dx, dy)));
            }
        }
    }
    scored.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
    let best = scored.first()?;
    // A different, nearly as good place means the piece is ambiguous (e.g. a straight corridor).
    if scored.iter().any(|&(hits, (dx, dy))| {
        ((dx - best.1 .0).abs() > 2 || (dy - best.1 .1).abs() > 2) && hits * 10 > best.0 * 9
    }) {
        return None;
    }
    let candidates: Vec<(i32, i32)> = scored.iter().take(3).map(|&(_, (dx, dy))| (dx * c, dy * c)).collect();
    candidates
        .into_iter()
        .map(|shift| register(walls, piece, shift, c + 2))
        .filter(|&(_, share)| share >= MERGE_MATCH)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(shift, _)| shift)
}

/// Joins earlier pieces of this visit onto the live map once they can be placed on it,
/// so arrival points and NPCs found before a restart or a lost track still count.
fn merge_pieces(session: &mut Session) -> bool {
    let live = &mut session.live;
    for index in 0..session.recording.parts.len() {
        let part = &session.recording.parts[index];
        let piece: Vec<(i32, i32)> = part.walls.iter().map(|&[x, y, _]| (x, y)).collect();
        let Some((dx, dy)) = locate_piece(&live.walls, &piece) else { continue };
        let part = session.recording.parts.remove(index);
        for [x, y, n] in part.walls {
            *live.walls.entry((x + dx, y + dy)).or_insert(0) += n as u32;
        }
        let mut trail: Vec<[i32; 2]> = part.trail.iter().map(|&[x, y]| [x + dx, y + dy]).collect();
        trail.append(&mut live.part.trail);
        live.part.trail = trail;
        let mut pois: Vec<Poi> = part
            .pois
            .into_iter()
            .map(|p| Poi { x: p.x + dx, y: p.y + dy, ..p })
            .chain(live.part.pois.drain(..))
            .collect();
        // Once a real arrival is known, "app started here" points mean nothing.
        if pois.iter().any(|p| p.kind == "arrived") {
            pois.retain(|p| p.kind != "start");
        }
        pois.sort_by_key(|p| p.at);
        live.part.pois = pois;
        live.part.frames += part.frames;
        live.part.matched += part.matched;
        live.part.lost += part.lost;
        live.part.started_at = live.part.started_at.min(part.started_at);
        return true;
    }
    false
}

impl Recorder {
    pub fn new(dir: Option<PathBuf>) -> Self {
        let scales = dir
            .as_ref()
            .and_then(|d| std::fs::read(d.join(SCALES_FILE)).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Recorder(Mutex::new(State {
            catalog: guide::Catalog::load(),
            completed: HashMap::new(),
            targets: HashMap::new(),
            scales,
            dir,
            session: None,
            previous_area: None,
            last_save: None,
            last_view: None,
            last_merge: None,
        }))
    }

    /// Log events: zone changes start a new recording, live ones also leave POIs.
    pub fn on_log(&self, event: &LogEvent, live: bool) {
        let Ok(mut state) = self.0.lock() else { return };
        match event {
            LogEvent::ZoneEntered { area, level, seed } => {
                let left = state.session.as_ref().map(|s| s.recording.area.clone());
                if let Some(session) = state.session.as_mut() {
                    if live && session.started {
                        session.live.add_poi("left", area);
                    }
                }
                state.finish();
                // A live arrival with no zone before it (e.g. a new character) still starts at the arrival point.
                state.previous_area = if live { Some(left.unwrap_or_default()) } else { None };
                let recording = state
                    .load(area, *seed)
                    .unwrap_or(Recording {
                        version: FORMAT_VERSION,
                        area: area.clone(),
                        level: *level,
                        seed: *seed,
                        cell_px: crate::minimap::CELL,
                        capture_px: [0, 0],
                        parts: vec![],
                    });
                let arrived_from = state.previous_area.clone();
                let file = state.path(area, *seed);
                state.session = Some(Session { recording, live: Live::new(), arrived_from, started: false, file, layout_hint: Default::default(), last_guide: Default::default() });
            }
            LogEvent::Dialogue { speaker, .. } if live => {
                if let Some(session) = state.session.as_mut().filter(|s| s.started) {
                    let known = session.live.part.pois.iter().any(|p| p.kind == "npc" && &p.label == speaker);
                    if !known {
                        session.live.add_poi("npc", speaker);
                    }
                }
            }
            LogEvent::Dialogue { .. } => {}
        }
    }

    /// One analysed minimap frame. Returns an overview for the overlay about once a second.
    pub fn on_frame(&self, frame: &MinimapView, capture_px: [usize; 2]) -> Option<LayoutView> {
        let mut state = self.0.lock().ok()?;
        let session = state.session.as_mut()?;
        if !frame.visible {
            return None;
        }
        session.recording.capture_px = capture_px;
        let live = &mut session.live;
        if let Some([x, y]) = frame.player {
            let found = (x.round() as i32, y.round() as i32);
            live.centre = match live.centre {
                Some(c) if (c.0 - found.0).abs() > 4 || (c.1 - found.1).abs() > 4 => Some(c),
                _ => Some(found),
            };
        }
        let centre = live.centre?;
        let rel: Vec<(i32, i32)> =
            frame.walls.iter().map(|&[x, y]| (x as i32 - centre.0, y as i32 - centre.1)).collect();
        if rel.len() < MIN_FRAME_WALLS {
            return None;
        }
        live.part.frames += 1;
        let placed = if live.walls.is_empty() {
            Some(live.pos)
        } else {
            let predicted = (live.pos.0 + live.velocity.0, live.pos.1 + live.velocity.1);
            let (near, score) = register(&live.walls, &rel, predicted, NEAR);
            if score >= MIN_MATCH {
                Some(near)
            } else {
                let (wide, score) = register(&live.walls, &rel, live.pos, WIDE);
                (score >= MIN_MATCH).then_some(wide)
            }
        };
        match placed {
            Some(pos) => {
                live.velocity = ((pos.0 - live.pos.0).clamp(-NEAR, NEAR), (pos.1 - live.pos.1).clamp(-NEAR, NEAR));
                live.pos = pos;
                live.lost_streak = 0;
                live.part.matched += 1;
                for &(rx, ry) in &rel {
                    *live.walls.entry((pos.0 + rx, pos.1 + ry)).or_insert(0) += 1;
                }
                let moved = live.part.trail.last().is_none_or(|&[x, y]| (x - pos.0).abs() + (y - pos.1).abs() >= 2);
                if moved {
                    live.part.trail.push([pos.0, pos.1]);
                }
                if !session.started {
                    session.started = true;
                    // Without a live zone change (app started mid-zone) the origin is unknown.
                    match session.arrived_from.take() {
                        Some(from) => session.live.add_poi("arrived", &from),
                        None => session.live.add_poi("start", ""),
                    }
                }
            }
            None => {
                live.part.lost += 1;
                live.lost_streak += 1;
                if live.lost_streak >= LOST_LIMIT {
                    // Lost track (teleport, map overlay, big jump): keep what we have, start over.
                    let finished = std::mem::replace(&mut session.live, Live::new());
                    session.live.centre = finished.centre;
                    if finished.part.matched > 0 {
                        session.recording.parts.push(finished.snapshot());
                    }
                }
            }
        }
        let due_merge = state.last_merge.is_none_or(|t| t.elapsed() >= MERGE_EVERY);
        if due_merge {
            state.last_merge = Some(Instant::now());
            if let Some(session) = state.session.as_mut().filter(|s| !s.recording.parts.is_empty()) {
                merge_pieces(session);
            }
        }
        let due_save = state.last_save.is_none_or(|t| t.elapsed() >= SAVE_EVERY);
        if due_save {
            state.save();
            state.last_save = Some(Instant::now());
        }
        let due_view = state.last_view.is_none_or(|t| t.elapsed() >= Duration::from_millis(900));
        if !due_view {
            return None;
        }
        state.last_view = Some(Instant::now());
        state.view()
    }

    pub fn view(&self) -> Option<LayoutView> {
        self.0.lock().ok()?.view()
    }

    /// Writes the current recording now (before a restart).
    pub fn save_now(&self) {
        if let Ok(state) = self.0.lock() {
            state.save();
        }
    }

    /// The open checklist steps that have a target, as (area, target) in checklist order.
    pub fn set_targets(&self, targets: Vec<(String, guide::NavTarget)>) {
        let Ok(mut state) = self.0.lock() else { return };
        state.targets.clear();
        for (area, target) in targets {
            state.targets.entry(area).or_default().push(target);
        }
    }

    /// Replacing the snapshot also handles unticking/resetting a checklist.
    pub fn set_completed(&self, objectives: Vec<(String, String)>) -> Option<LayoutView> {
        let mut state = self.0.lock().ok()?;
        state.completed.clear();
        for (area, label) in objectives {
            state.completed.entry(area).or_default().push(label);
        }
        state.view()
    }
}

impl State {
    fn path(&self, area: &str, seed: Option<u64>) -> Option<PathBuf> {
        let safe = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' }).collect::<String>();
        let name = seed.map_or_else(|| format!("unknown-{}", now()), |s| s.to_string());
        Some(self.dir.as_ref()?.join(safe(area)).join(format!("{name}.json")))
    }

    /// An earlier recording of the same instance, to add a part to.
    fn load(&self, area: &str, seed: Option<u64>) -> Option<Recording> {
        seed?;
        let bytes = std::fs::read(self.path(area, seed)?).ok()?;
        serde_json::from_slice::<Recording>(&bytes).ok().filter(|r| r.version == FORMAT_VERSION)
    }

    fn save(&self) {
        let Some(session) = &self.session else { return };
        let mut parts = session.recording.parts.clone();
        if session.live.part.matched > 0 {
            parts.push(session.live.snapshot());
        }
        if parts.is_empty() {
            return;
        }
        let Some(path) = session.file.clone() else { return };
        let r = &session.recording;
        let out = Recording {
            version: r.version,
            area: r.area.clone(),
            level: r.level,
            seed: r.seed,
            cell_px: r.cell_px,
            capture_px: r.capture_px,
            parts,
        };
        let result = (|| -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec(&out)?)?;
            std::fs::rename(tmp, &path)
        })();
        if let Err(error) = result {
            eprintln!("Layout: could not save {}: {error}", path.display());
        }
    }

    /// Saves and closes the current recording, and learns the zone's scale from it.
    fn finish(&mut self) {
        self.save();
        let Some(session) = self.session.take() else { return };
        let pois = &session.live.part.pois;
        let Some(arrival) = pois.iter().filter(|p| p.kind == "arrived").min_by_key(|p| p.at) else { return };
        let area = &session.recording.area;
        let walls: Vec<(i32, i32)> = session
            .live
            .walls
            .iter()
            .filter(|(_, &n)| n >= 2)
            .map(|(&(x, y), _)| (x - arrival.x, y - arrival.y))
            .collect();
        if let Some(scale) = guide::measure_scale(&self.catalog, area, &arrival.label, &walls) {
            self.scales.entry(area.clone()).or_default().push(scale);
            if let Some(dir) = &self.dir {
                let _ = std::fs::create_dir_all(dir);
                if let Ok(json) = serde_json::to_vec_pretty(&self.scales) {
                    let _ = std::fs::write(dir.join(SCALES_FILE), json);
                }
            }
        }
    }

    /// The arrow, kept on screen: when it cannot be worked out for a moment (tracking
    /// lost, a piece without enough to go on), the last one stays, marked unconfirmed.
    fn guide(&self, session: &Session) -> Option<guide::GuideView> {
        if !GUIDED.contains(&session.recording.area.as_str()) {
            return None;
        }
        match self.fresh_guide(session) {
            Some(view) => {
                *session.last_guide.borrow_mut() = Some(view.clone());
                Some(view)
            }
            None => session.last_guide.borrow().clone().map(|view| guide::GuideView { source: "unconfirmed".into(), ..view }),
        }
    }

    fn fresh_guide(&self, session: &Session) -> Option<guide::GuideView> {
        let area = session.recording.area.as_str();
        let live = &session.live;
        if live.lost_streak > 0 {
            return None;
        }
        // Everything relative to the (first) arrival point, where the terrain match starts;
        // a piece without one (tracking lost, overlay restarted) is placed anywhere.
        let arrival = live.part.pois.iter().filter(|p| p.kind == "arrived").min_by_key(|p| p.at);
        let (ax, ay) = arrival.map_or((0, 0), |a| (a.x, a.y));
        let arrived_from = arrival.map_or("", |a| a.label.as_str());
        let mut walls: Vec<(i32, i32)> =
            live.walls.iter().filter(|(_, &n)| n >= 2).map(|(&(x, y), _)| (x - ax, y - ay)).collect();
        if walls.len() > GUIDE_WALLS {
            walls.sort_unstable();
            let step = walls.len().div_ceil(GUIDE_WALLS);
            walls = walls.into_iter().step_by(step).collect();
        }
        // The minimap's zoom is probably the same everywhere: without a measurement
        // for this area, use the median of all areas.
        let median = |mut all: Vec<f64>| {
            all.sort_by(f64::total_cmp);
            all.get(all.len() / 2).copied()
        };
        let scale = self
            .scales
            .get(area)
            .and_then(|all| median(all.clone()))
            .or_else(|| median(self.scales.values().flatten().copied().collect()));
        let layout_hint = if arrival.is_some() {
            None
        } else {
            session.layout_hint.get_or_init(|| self.recognised_layout(session, scale)).as_deref()
        };
        guide::guide(
            &self.catalog,
            &guide::Evidence {
                area,
                arrived_from,
                walls: &walls,
                trail: &live.part.trail.iter().map(|&[x, y]| (x - ax, y - ay)).collect::<Vec<_>>(),
                anchored: arrival.is_some(),
                layout_hint,
                player: (live.pos.0 - ax, live.pos.1 - ay),
                targets: self.targets.get(area).map(Vec::as_slice).unwrap_or_default(),
                completed: self.completed.get(area).map(Vec::as_slice).unwrap_or_default(),
                scale,
            },
        )
    }

    /// The layout an earlier piece of this instance (one with an arrival point) matches.
    fn recognised_layout(&self, session: &Session, scale: Option<f64>) -> Option<String> {
        let area = session.recording.area.as_str();
        session.recording.parts.iter().filter_map(|part| {
            let arrival = part.pois.iter().filter(|p| p.kind == "arrived").min_by_key(|p| p.at)?;
            let trail: Vec<(i32, i32)> = part.trail.iter().map(|&[x, y]| (x - arrival.x, y - arrival.y)).collect();
            let player = *trail.last()?;
            let view = guide::guide(&self.catalog, &guide::Evidence {
                area, targets: &[], arrived_from: &arrival.label, walls: &[], trail: &trail, anchored: true, layout_hint: None,
                player, completed: &[], scale,
            })?;
            (view.source != "unconfirmed" && !view.variant.is_empty()).then_some(view.variant)
        }).next()
    }

    fn view(&self) -> Option<LayoutView> {
        let session = self.session.as_ref()?;
        let live = &session.live;
        let solid: Vec<(i32, i32)> = live.walls.iter().filter(|(_, &n)| n >= 2).map(|(&k, _)| k).collect();
        let points = solid.iter().copied().chain(live.part.trail.iter().map(|&[x, y]| (x, y))).chain([live.pos]);
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (x, y) in points {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        let span_x = (x1 - x0 + 1) as f32;
        let span_y = (y1 - y0 + 1) as f32;
        let scale = (VIEW_COLS as f32 / span_x).min(VIEW_ROWS as f32 / span_y).min(1.0);
        // Centre the drawing in the grid.
        let off_x = (VIEW_COLS as f32 - span_x * scale) / 2.0;
        let off_y = (VIEW_ROWS as f32 - span_y * scale) / 2.0;
        let at = |x: i32, y: i32| [(x - x0) as f32 * scale + off_x, (y - y0) as f32 * scale + off_y];
        let mut cells: Vec<[u16; 2]> = solid
            .iter()
            .map(|&(x, y)| {
                let [cx, cy] = at(x, y);
                [(cx as u16).min(VIEW_COLS as u16 - 1), (cy as u16).min(VIEW_ROWS as u16 - 1)]
            })
            .collect();
        cells.sort_unstable();
        cells.dedup();
        Some(LayoutView {
            area: session.recording.area.clone(),
            seed: session.recording.seed,
            part: session.recording.parts.len() + 1,
            frames: live.part.frames,
            matched: live.part.matched,
            cols: VIEW_COLS,
            rows: VIEW_ROWS,
            walls: cells,
            trail: live.part.trail.iter().map(|&[x, y]| at(x, y)).collect(),
            player: at(live.pos.0, live.pos.1),
            pois: live
                .part
                .pois
                .iter()
                .map(|p| {
                    let [x, y] = at(p.x, p.y);
                    ViewPoi { kind: p.kind.clone(), label: p.label.clone(), x, y }
                })
                .collect(),
            guide: self.guide(session),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wiggly corridor, as wall cells in world coordinates.
    fn world() -> Vec<(i32, i32)> {
        let mut cells = Vec::new();
        for x in -200..200 {
            let wiggle = ((x as f32) / 9.0).sin() * 6.0 + ((x as f32) / 23.0).cos() * 9.0;
            cells.push((x, wiggle as i32 - 12));
            cells.push((x, wiggle as i32 + 14 + (x / 40) % 3));
        }
        cells
    }

    /// What the minimap shows with the player at `pos`: walls within a window, as frame cells.
    fn frame(pos: (i32, i32)) -> MinimapView {
        let centre = (75, 65);
        let walls = world()
            .into_iter()
            .map(|(x, y)| (x - pos.0 + centre.0, y - pos.1 + centre.1))
            .filter(|&(x, y)| (0..150).contains(&x) && (0..130).contains(&y))
            .map(|(x, y)| [x as u16, y as u16])
            .collect();
        MinimapView {
            enabled: true,
            visible: true,
            message: String::new(),
            cols: 150,
            rows: 130,
            walls,
            player: Some([centre.0 as f32, centre.1 as f32]),
        }
    }

    fn enter(recorder: &Recorder, area: &str) {
        recorder.on_log(&LogEvent::ZoneEntered { area: area.into(), level: 1, seed: Some(7) }, true);
    }

    #[test]
    fn follows_the_player_along_a_corridor() {
        let recorder = Recorder::new(None);
        enter(&recorder, "G1_1");
        let mut x = 0;
        for step in 0..60 {
            x += if step < 30 { 5 } else { -3 };
            recorder.on_frame(&frame((x, 0)), [300, 260]);
        }
        let state = recorder.0.lock().unwrap();
        let live = &state.session.as_ref().unwrap().live;
        assert_eq!(live.part.lost, 0);
        // Started at x=5, so the recorded position is relative to that.
        assert_eq!(live.pos, (x - 5, 0));
    }

    #[test]
    fn pins_arrival_npcs_and_departure() {
        let recorder = Recorder::new(None);
        enter(&recorder, "G1_1");
        recorder.on_frame(&frame((0, 0)), [300, 260]);
        enter(&recorder, "G1_2");
        for x in [0, 4, 8] {
            recorder.on_frame(&frame((x, 0)), [300, 260]);
        }
        let npc = LogEvent::Dialogue { speaker: "Wounded Man".into(), text: "Hello".into() };
        recorder.on_log(&npc, true);
        recorder.on_log(&npc, true);
        let state = recorder.0.lock().unwrap();
        let pois = &state.session.as_ref().unwrap().live.part.pois;
        let summary: Vec<(&str, &str, i32)> = pois.iter().map(|p| (p.kind.as_str(), p.label.as_str(), p.x)).collect();
        assert_eq!(summary, [("arrived", "G1_1", 0), ("npc", "Wounded Man", 8)]);
    }

    #[test]
    fn history_from_the_log_leaves_no_points() {
        let recorder = Recorder::new(None);
        recorder.on_log(&LogEvent::ZoneEntered { area: "G1_1".into(), level: 1, seed: Some(1) }, false);
        recorder.on_frame(&frame((0, 0)), [300, 260]);
        let state = recorder.0.lock().unwrap();
        let pois = &state.session.as_ref().unwrap().live.part.pois;
        assert_eq!(pois.len(), 1);
        assert_eq!(pois[0].kind, "start");
    }

    #[test]
    fn completion_untick_and_tracking_loss_update_guidance() {
        let recorder = Recorder::new(None);
        // The Riverbank's arrow heads for the Miller first (unconfirmed before a walk).
        enter(&recorder, "G1_1");
        recorder.on_frame(&frame((0, 0)), [300, 260]);
        let guide = recorder.view().unwrap().guide.unwrap();
        assert_eq!(guide.target, "The Bloated Miller");
        assert_ne!(guide.source, "estimated");

        enter(&recorder, "G1_town");
        recorder.on_frame(&frame((0, 0)), [300, 260]);
        enter(&recorder, "G1_2");
        for x in [0, 4, 8] {
            recorder.on_frame(&frame((x, 0)), [300, 260]);
        }
        let guide = recorder.view().unwrap().guide.unwrap();
        assert_eq!(guide.target, "Mud Burrow");
        // A short corridor that is not Clearfell confirms nothing.
        assert_ne!(guide.source, "estimated");

        let guide = recorder.set_completed(vec![("G1_2".into(), "Mud Burrow".into())]).unwrap().guide.unwrap();
        assert_eq!(guide.target, "The Grelwood");
        let guide = recorder.set_completed(vec![]).unwrap().guide.unwrap();
        assert_eq!(guide.target, "Mud Burrow");

        // Tracking lost: the last arrow stays, unconfirmed.
        recorder.0.lock().unwrap().session.as_mut().unwrap().live.lost_streak = 1;
        let guide = recorder.view().unwrap().guide.unwrap();
        assert_eq!(guide.target, "Mud Burrow");
        assert_eq!(guide.source, "unconfirmed");
    }
}

/// Joins the pieces of a real recording: `LAYOUT_FILE=<json> cargo test --release join_real -- --ignored --nocapture`.
#[cfg(test)]
#[test]
#[ignore]
fn join_real_recording() {
    let path = std::env::var("LAYOUT_FILE").expect("set LAYOUT_FILE");
    let mut recording: Recording = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let biggest = (0..recording.parts.len()).max_by_key(|&i| recording.parts[i].walls.len()).unwrap();
    let first = recording.parts.remove(biggest);
    let mut live = Live::new();
    live.walls = first.walls.iter().map(|&[x, y, n]| ((x, y), n as u32)).collect();
    live.part = Part { walls: vec![], ..first };
    let parts = recording.parts.len();
    let mut session = Session { recording, live, arrived_from: None, started: true, file: None, layout_hint: Default::default(), last_guide: Default::default() };
    let started = Instant::now();
    while merge_pieces(&mut session) {}
    let recorder = Recorder::new(None);
    let guide = {
        let mut state = recorder.0.lock().unwrap();
        state.session = Some(session);
        let view = state.view().unwrap().guide;
        session = state.session.take().unwrap();
        view
    };
    println!("guide: {guide:?}");
    if let Ok(out) = std::env::var("LAYOUT_OUT") {
        session.recording.parts = vec![session.live.snapshot()];
        std::fs::write(out, serde_json::to_vec(&session.recording).unwrap()).unwrap();
    }
    println!(
        "joined {} of {parts} pieces in {:?}; pois {:?}",
        parts - session.recording.parts.len(),
        started.elapsed(),
        session.live.part.pois.iter().map(|p| (&p.kind, &p.label, p.x, p.y)).collect::<Vec<_>>()
    );
}
