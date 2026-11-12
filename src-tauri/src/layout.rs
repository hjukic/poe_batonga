//! Layout recorder: stitches live minimap frames into a map of each zone visit and
//! pins points of interest from the game log (arrival, departure, NPC/boss lines).
//!
//! The minimap is centred on the player, so each frame's walls are matched against
//! the map so far to find how far the player moved. Recordings are saved per zone
//! instance (area + seed) under the app data folder, `layouts/<area>/<seed>.json`.
//! A visit that cannot be related to earlier parts of the same instance (another
//! entry, a teleport, tracking lost) starts a new part with its own coordinates.
//!
//! In zones with layout graphs the navigator (`nav.rs`) is fed every measured
//! step and log anchor and gives the needle; it carries on across new parts. What
//! it learnt is kept beside the recording, `layouts/<area>/<seed>.nav.json`.

mod graph;
pub mod guide;
mod nav;
#[cfg(test)]
mod nav_sim;

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
/// A gap in placed frames longer than this (seconds) is a tracking loss for the navigator.
const NAV_GAP: f64 = 2.0;
/// A checklist step the log completes within this many seconds of a live NPC
/// line puts the player at the step's target.
const LOG_ANCHOR: Duration = Duration::from_secs(5);
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
/// At most this many wall cells are used to score the terrain arrow's hypotheses.
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
    /// The area id the player came from ("" = unknown or none).
    came_from: String,
    /// The belief over layout and position, in zones with layout graphs.
    navigator: Option<nav::Navigator>,
    /// The last placed position fed to the navigator, in the live part's
    /// coordinates (None at the start of a part), and when it was placed.
    fed: Option<(i32, i32)>,
    fed_at: Option<Instant>,
    /// The last live NPC line in this visit.
    last_dialogue: Option<Instant>,
    /// The last arrow of this visit, kept while a new one cannot be worked out.
    last_guide: std::cell::RefCell<Option<guide::GuideView>>,
}

impl Session {
    fn nav_file(&self) -> Option<PathBuf> {
        Some(self.file.as_ref()?.with_extension("nav.json"))
    }

    /// Hands the navigator the move to `pos`; a gap or a new part is a loss first.
    fn feed(&mut self, pos: (i32, i32)) {
        let Some(nav) = self.navigator.as_mut() else { return };
        let gap = self.fed_at.map(|t| t.elapsed().as_secs_f64());
        match self.fed {
            Some(prev) => {
                if let Some(gap) = gap.filter(|&g| g > NAV_GAP) {
                    nav.lost(gap);
                }
                nav.moved((pos.0 - prev.0) as f64, (pos.1 - prev.1) as f64);
            }
            // A new part: where it starts relative to the last one is unknown.
            None => {
                if let Some(gap) = gap {
                    nav.lost(gap);
                }
            }
        }
        self.fed = Some(pos);
        self.fed_at = Some(Instant::now());
    }
}

struct State {
    catalog: guide::Catalog,
    completed: HashMap<String, Vec<String>>,
    /// Open checklist targets per area, in checklist order.
    targets: HashMap<String, Vec<guide::NavTarget>>,
    /// Every checklist target seen, per area (to find a step the log just completed).
    known_targets: HashMap<String, Vec<guide::NavTarget>>,
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

/// Writes JSON through a temporary file, so a crash never leaves half a file.
fn write_json(path: &std::path::Path, value: &impl Serialize) {
    let result = (|| -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec(value)?)?;
        std::fs::rename(tmp, path)
    })();
    if let Err(error) = result {
        eprintln!("Layout: could not save {}: {error}", path.display());
    }
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
            known_targets: HashMap::new(),
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
                    // Leaving through an exit: the player stands at it (a re-entry of the same zone is not one).
                    if live && *area != session.recording.area {
                        if let Some(nav) = session.navigator.as_mut() {
                            nav.seen(nav::Seen::Exit(area));
                        }
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
                let mut session = Session {
                    recording,
                    live: Live::new(),
                    came_from: arrived_from.clone().unwrap_or_default(),
                    arrived_from,
                    started: false,
                    file,
                    navigator: None,
                    fed: None,
                    fed_at: None,
                    last_dialogue: None,
                    last_guide: Default::default(),
                };
                session.navigator = state.navigator(&session);
                state.session = Some(session);
            }
            LogEvent::Dialogue { speaker, .. } if live => {
                if let Some(session) = state.session.as_mut() {
                    session.last_dialogue = Some(Instant::now());
                }
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
                session.feed(pos);
                let live = &mut session.live;
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
                    // The navigator carries on; the new part's coordinates are new.
                    let finished = std::mem::replace(&mut session.live, Live::new());
                    session.live.centre = finished.centre;
                    session.fed = None;
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
            let known = state.known_targets.entry(area.clone()).or_default();
            if !known.contains(&target) {
                known.push(target.clone());
            }
            state.targets.entry(area).or_default().push(target);
        }
    }

    /// Replacing the snapshot also handles unticking/resetting a checklist.
    pub fn set_completed(&self, objectives: Vec<(String, String)>) -> Option<LayoutView> {
        let mut state = self.0.lock().ok()?;
        let before = std::mem::take(&mut state.completed);
        for (area, label) in objectives {
            state.completed.entry(area).or_default().push(label);
        }
        state.anchor_log_completions(&before);
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
        if let (Some(nav), Some(path)) = (&session.navigator, session.nav_file()) {
            write_json(&path, &nav.memory());
        }
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
        write_json(&path, &out);
    }

    /// A navigator for a new visit, in areas with layout graphs; it starts from
    /// what an earlier visit of the same instance learnt.
    fn navigator(&self, session: &Session) -> Option<nav::Navigator> {
        let area = self.catalog.graph(&session.recording.area)?;
        let memory: Option<nav::Memory> = session
            .nav_file()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let arrival = match &session.arrived_from {
            Some(from) => nav::Arrival::From(from),
            None => nav::Arrival::Unknown,
        };
        let scale = self.scale(&session.recording.area).unwrap_or(graph::SCALE);
        Some(nav::Navigator::new(&area, arrival, memory.as_ref(), scale, session.recording.seed.unwrap_or(0)))
    }

    /// Measured minimap cells per game unit: this area's median, else all areas'.
    /// The minimap's zoom is probably the same everywhere.
    fn scale(&self, area: &str) -> Option<f64> {
        let median = |mut all: Vec<f64>| {
            all.sort_by(f64::total_cmp);
            all.get(all.len() / 2).copied()
        };
        self.scales
            .get(area)
            .and_then(|all| median(all.clone()))
            .or_else(|| median(self.scales.values().flatten().copied().collect()))
    }

    /// A checklist step of this area that the log just completed (a live NPC line
    /// a moment ago) puts the player at its target.
    fn anchor_log_completions(&mut self, before: &HashMap<String, Vec<String>>) {
        let Some(session) = self.session.as_mut() else { return };
        if session.last_dialogue.is_none_or(|t| t.elapsed() > LOG_ANCHOR) {
            return;
        }
        let area = session.recording.area.as_str();
        let Some(nav) = session.navigator.as_mut() else { return };
        let was = before.get(area).cloned().unwrap_or_default();
        let known = self.known_targets.get(area).map(Vec::as_slice).unwrap_or_default();
        for label in self.completed.get(area).into_iter().flatten().filter(|l| !was.contains(l)) {
            let Some(target) = known.iter().find(|t| &t.label == label) else { continue };
            if let Some(exit) = &target.exit {
                nav.seen(nav::Seen::Exit(exit));
            } else if let Some(tag) = &target.tag {
                nav.seen(nav::Seen::Tag(tag));
            }
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
        if !self.catalog.guided(&session.recording.area) {
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
        let completed = self.completed.get(area).map(Vec::as_slice).unwrap_or_default();
        let scale = self.scale(area);
        if let Some(nav) = &session.navigator {
            // The checklist's open targets, then the onward exit (zones without a
            // checklist, or targets no layout has).
            let mut targets = self.targets.get(area).cloned().unwrap_or_default();
            if let Some(graph) = self.catalog.graph(area) {
                targets.extend(graph::target(&graph, &session.came_from, completed));
            }
            return nav.guide(&targets).map(|view| guide::GuideView { measured: scale.is_some(), ..view });
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
        guide::terrain(
            &self.catalog,
            &guide::Evidence {
                area,
                arrived_from,
                walls: &walls,
                anchored: arrival.is_some(),
                player: (live.pos.0 - ax, live.pos.1 - ay),
                completed,
                scale,
            },
        )
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

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("poe-batonga-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_navigator_is_fed_and_remembers_the_instance() {
        let dir = scratch("nav");
        let recorder = Recorder::new(Some(dir.clone()));
        enter(&recorder, "G1_town");
        enter(&recorder, "G1_2");
        for step in 0..40 {
            recorder.on_frame(&frame((step * 4, 0)), [300, 260]);
        }
        {
            let state = recorder.0.lock().unwrap();
            let nav = state.session.as_ref().unwrap().navigator.as_ref().expect("Clearfell has layout graphs");
            let (walked, _) = nav.progress();
            assert!(walked > 150.0, "{walked}");
        }
        // Leaving through the Mud Burrow exit is an anchor; the memory is saved beside the recording.
        enter(&recorder, "G1_3");
        let file = dir.join("G1_2").join("7.nav.json");
        let memory: nav::Memory = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(memory.layouts.len(), 33);
        assert!(memory.walked > 150.0);
        // Back in the same instance: the navigator starts from what was learnt.
        enter(&recorder, "G1_2");
        let state = recorder.0.lock().unwrap();
        let nav = state.session.as_ref().unwrap().navigator.as_ref().unwrap();
        assert_eq!(nav.progress().0, memory.walked);
        let top = memory.layouts.iter().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert!(nav.ranking().iter().take(3).any(|(id, _)| id == top), "{:?}", &nav.ranking()[..3]);
        drop(state);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_step_the_log_completes_puts_the_player_at_its_target() {
        let recorder = Recorder::new(None);
        enter(&recorder, "G1_town");
        enter(&recorder, "G1_2");
        recorder.on_frame(&frame((0, 0)), [300, 260]);
        let beira = guide::NavTarget { label: "Beira".into(), tag: Some("boss".into()), ..Default::default() };
        recorder.set_targets(vec![("G1_2".into(), beira.clone())]);
        let anchored = |r: &Recorder| r.0.lock().unwrap().session.as_ref().unwrap().navigator.as_ref().unwrap().progress().1;
        // Ticked by hand (no NPC line just before): not an anchor.
        recorder.set_completed(vec![("G1_2".into(), "Beira".into())]);
        assert!(!anchored(&recorder));
        recorder.set_completed(vec![]);
        // Completed from the log: the player stands at a boss node.
        recorder.on_log(&LogEvent::Dialogue { speaker: "Beira".into(), text: "…".into() }, true);
        recorder.set_targets(vec![]);
        recorder.set_completed(vec![("G1_2".into(), "Beira".into())]);
        assert!(anchored(&recorder));
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
    let mut session = Session {
        recording, live, arrived_from: None, came_from: String::new(), started: true, file: None, navigator: None,
        fed: None, fed_at: None, last_dialogue: None, last_guide: Default::default(),
    };
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

/// The navigator on real recordings, parts in order (a new part is a loss of
/// unknown length; "arrived" and "left" points are exit anchors):
/// `RECORDINGS=<dir> cargo test --release navigator_on_recordings -- --ignored --nocapture`.
#[cfg(test)]
#[test]
#[ignore]
fn navigator_on_recordings() {
    let dir = PathBuf::from(std::env::var("RECORDINGS").expect("set RECORDINGS"));
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .flat_map(|area| std::fs::read_dir(area.path()).into_iter().flatten().flatten())
        .map(|f| f.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json") && !p.to_string_lossy().ends_with(".nav.json"))
        .collect();
    files.sort();
    let graphs = graph::Graphs::default();
    for path in files {
        let recording: Recording = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let own = recording.area.as_str();
        println!("\n== {} seed {:?}: {} parts", own, recording.seed, recording.parts.len());
        let Some(area) = graphs.get(own) else {
            println!("   no layout graphs (terrain arrow)");
            continue;
        };
        let mut nav = nav::Navigator::new(&area, nav::Arrival::Unknown, None, graph::SCALE, recording.seed.unwrap_or(0));
        let top = |nav: &nav::Navigator| {
            nav.ranking().iter().take(3).map(|(id, p)| format!("{id} {:.2}", p)).collect::<Vec<_>>().join(", ")
        };
        for (i, part) in recording.parts.iter().enumerate() {
            if i > 0 {
                nav.lost(f64::INFINITY);
            }
            // Each point of interest at the trail point nearest to it, in time order.
            let mut pois: Vec<&Poi> = part.pois.iter().collect();
            pois.sort_by_key(|p| p.at);
            let mut at: Vec<(usize, &Poi)> = Vec::new();
            let mut from = 0;
            for poi in pois {
                let d = |t: &[i32; 2]| (t[0] - poi.x).abs() + (t[1] - poi.y).abs();
                let index = (from..part.trail.len())
                    .find(|&j| d(&part.trail[j]) <= 3)
                    .or_else(|| (from..part.trail.len()).min_by_key(|&j| d(&part.trail[j])))
                    .unwrap_or(from);
                at.push((index, poi));
                from = index;
            }
            let mut anchors = String::new();
            for j in 0..part.trail.len() {
                if j > 0 {
                    let (dx, dy) = (part.trail[j][0] - part.trail[j - 1][0], part.trail[j][1] - part.trail[j - 1][1]);
                    // Joined pieces share coordinates: a jump between them is a move
                    // whose way is unknown.
                    if dx.abs().max(dy.abs()) > WIDE {
                        nav.lost(30.0);
                    }
                    nav.moved(dx as f64, dy as f64);
                }
                for (_, poi) in at.iter().filter(|(index, _)| *index == j) {
                    match poi.kind.as_str() {
                        "arrived" | "left" if poi.label != own && !poi.label.is_empty() => {
                            nav.seen(nav::Seen::Exit(&poi.label));
                            anchors += &format!(" {} {}", poi.kind, poi.label);
                        }
                        // Into the same zone again (a checkpoint after dying, a new instance): somewhere else.
                        "arrived" | "left" => nav.lost(f64::INFINITY),
                        _ => {}
                    }
                }
            }
            let (walked, _) = nav.progress();
            println!("   part {i:2}: {:4} trail points, walked {:5.0} |{anchors} | {}", part.trail.len(), walked, top(&nav));
        }
        let mut exits: Vec<(&String, &String)> = area.names.iter().collect();
        exits.sort();
        for (id, name) in exits {
            let target = guide::NavTarget { label: name.clone(), exit: Some(id.clone()), ..Default::default() };
            match nav.guide(&[target]) {
                Some(v) => println!(
                    "   needle to {name}: {:?}° {} confidence {:.2} distance {:.0} cells, variant {:?}",
                    v.degrees.map(|d| d.round()), v.source, v.confidence, v.distance, v.variant
                ),
                None => println!("   needle to {name}: none"),
            }
        }
        // Which layouts put The Grelwood to the left of Mud Burrow on screen (the big map shows it so).
        if own == "G1_2" {
            let mut fits = Vec::new();
            for layout in &area.layouts {
                let node = |exit: &str| layout.nodes.iter().find(|n| n.exit.as_deref() == Some(exit)).map(|n| (n.x, n.y));
                let (Some(mud), Some(wood)) = (node("G1_3"), node("G1_4")) else { continue };
                let (sx, sy) = guide::to_screen((wood.0 - mud.0, wood.1 - mud.1));
                let bearing = guide::screen_bearing(sx, sy).unwrap_or(0.0);
                if guide::angle_between(bearing, 180.0) <= 25.0 {
                    fits.push(format!("{} ({:.0}°, {:.0} units)", layout.id, bearing, sx.hypot(sy)));
                }
            }
            println!("   layouts with The Grelwood left of Mud Burrow (±25°): {}", fits.join(", "));
        }
    }
}
