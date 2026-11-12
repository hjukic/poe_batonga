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
//! it learnt is kept beside the recording, `layouts/<area>/<seed>.nav.json`, and
//! what it was fed in `<seed>.obs.jsonl` (the flight recorder, `obs.rs`).

mod graph;
pub mod guide;
mod nav;
#[cfg(test)]
mod nav_sim;
mod obs;

use crate::minimap::MinimapView;
use crate::tracker::LogEvent;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// 2: recordings made before v0.1.59 hold scene pixels as walls and pieces stitched
/// far off (a town keeps one file for all visits): they are not loaded any more.
const FORMAT_VERSION: u32 = 2;
/// Frames with fewer wall cells carry too little shape to place reliably.
const MIN_FRAME_WALLS: usize = 30;
/// Share of a frame's wall cells that must land on known walls to accept a match.
const MIN_MATCH: f64 = 0.55;
/// Search radius in cells around the predicted move, then a wider retry.
const NEAR: i32 = 12;
const WIDE: i32 = 32;
/// Consecutive unmatched frames before a new part is started (about three seconds).
const LOST_LIMIT: u32 = 25;
const SAVE_EVERY: Duration = Duration::from_secs(15);
/// Far icons from a whole-screen scan stand in for this many frames after it
/// (the scan comes every `minimap::SCAN_EVERY`-th big-map frame).
const FAR_FRAMES: u32 = crate::minimap::SCAN_EVERY;
/// A big-map wall cell is kept once seen with the player this many cells (either axis)
/// from where it was first seen (I9). The character is about 115 cells tall in the owner's
/// captures, so this does not clear it: what keeps its cells out is that the patches of its
/// clothes that pass the line colour are small and move with it, so a cell is rarely hit
/// again from 24 cells away; the same holds for the scene, which scrolls faster. On a walk
/// built from the captures 0-1.5% of scene cells were still kept.
const HOLD_MOVE: i32 = 24;

/// A needle without a placed frame for this long is not "estimated": the player
/// may have walked on with the map hidden, and the bearing is a screen direction.
const STALE: Duration = Duration::from_secs(2);

/// Gold only while frames come in.
fn not_stale(view: guide::GuideView, fed_at: Option<Instant>) -> guide::GuideView {
    if view.source == "estimated" && fed_at.is_none_or(|t| t.elapsed() > STALE) {
        return guide::GuideView { source: "unconfirmed".into(), ..view };
    }
    view
}

/// Joining pieces of a visit: how often to try, the least shape to try with, the
/// coarse grid for the first search and the share of walls that must line up.
const MERGE_EVERY: Duration = Duration::from_secs(3);
const MERGE_MIN_WALLS: usize = 400;
/// A piece can be smaller: one minimap frame shows about 250-1000 wall cells.
const MERGE_MIN_PIECE: usize = 200;
const MERGE_COARSE: i32 = 6;
const MERGE_MATCH: f64 = 0.6;
/// A frame that cannot be placed near the player's last place (after a gap: frames not
/// read, a fast move) is looked for this far from it, in cells, the way pieces are joined.
const REFIND: i32 = 150;
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
    /// Wall cells and how often seen: all of them place frames (`register`).
    walls: HashMap<(i32, i32), u32>,
    /// Big-map wall cells not yet seen from two player positions `HOLD_MOVE` apart, with
    /// the first: left out of the recording and the overview …
    held: HashMap<(i32, i32), Held>,
    /// … which take these: every cell not held, with how often seen.
    kept: HashMap<(i32, i32), u32>,
    pos: (i32, i32),
    velocity: (i32, i32),
    lost_streak: u32,
    /// Player cell inside the frame (the minimap is centred on the player).
    centre: Option<(i32, i32)>,
    /// Size of the last frame: another size is another view (minimap or big map).
    frame_size: (usize, usize),
    part: Part,
    /// The map was loaded from an earlier visit and the player is not found on it yet:
    /// frames are looked for on it (around `hint`, e.g. where the player came in by the same
    /// exit before), and after `FIND_FRAMES` without a place a new part starts.
    unplaced: bool,
    hint: Option<(i32, i32)>,
    tries: u32,
}

/// A big-map wall cell not kept yet (`Live::held`).
#[derive(Clone, Copy, Debug)]
struct Held {
    /// Where the player was when it was first seen.
    first: (i32, i32),
    /// Frames that showed it, and frames whose area it lay in.
    seen: u32,
    in_view: u32,
}

/// A held cell is kept once seen in at least half the frames it lay in (map lines are
/// drawn in every frame; lit twigs and foam pass the line colour in some frames only, at
/// cells that change as the scene moves: the owner's Clearfell of 2026-11-12 grew 127,000
/// wall cells, a white blob, from them), and dropped once seen in under a quarter of 16.
const HELD_SHARE: f64 = 0.5;
/// … and only once it lay in this many frames: two chance hits early on are no line.
const HELD_MIN_VIEW: u32 = 6;
const HELD_DROP: (u32, f64) = (16, 0.25);

/// The eight cells around a cell.
const NEIGHBOURS: [(i32, i32); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
/// Frames to find the player on a known map before starting a new part (about 8 s): nothing
/// of the visit is fed to the navigator while it is looked for, so looking longer costs nothing.
const FIND_FRAMES: u32 = 64;
/// Times a wall cell counts as seen at most: more sightings add nothing.
const SEEN_MOST: u32 = 50;

impl Live {
    fn new() -> Self {
        Live {
            walls: HashMap::new(),
            held: HashMap::new(),
            kept: HashMap::new(),
            pos: (0, 0),
            velocity: (0, 0),
            lost_streak: 0,
            centre: None,
            frame_size: (0, 0),
            part: Part { started_at: now(), ..Part::default() },
            unplaced: false,
            hint: None,
            tries: 0,
        }
    }

    /// The map of an earlier visit to this instance, to go on with: the instance keeps its
    /// layout, so a return draws no second copy of what is known. `hint`: where to look first.
    fn from_part(part: Part, hint: Option<(i32, i32)>) -> Self {
        let mut live = Live::new();
        for &[x, y, n] in &part.walls {
            live.walls.insert((x, y), n as u32);
            live.kept.insert((x, y), n as u32);
        }
        live.pos = hint.unwrap_or((0, 0));
        live.part = part;
        live.part.walls = vec![];
        live.unplaced = !live.walls.is_empty();
        live.hint = hint;
        live
    }

    /// The wall cells that are kept: every one but those held back.
    fn kept(&self) -> impl Iterator<Item = (&(i32, i32), &u32)> {
        self.kept.iter()
    }

    /// One frame's wall cells (`rel`, from the player) seen with the player at `pos`. The
    /// big map shows the game scene through it, and some of the scene passes the line
    /// colour (water foam, wet stones, the character's clothes: Riverbank captures, I9):
    /// a big-map cell is kept once seen from two places `HOLD_MOVE` apart, where a map line
    /// stays on its cell and the scene, which scrolls faster, or the character, who moves
    /// with the player, mostly does not. Minimap cells are kept at once (the fault was not
    /// seen there).
    ///
    /// A cell that is not known but has a known neighbour is that neighbour seen a cell
    /// off: it counts for the neighbour and adds nothing, so walls seen again (a return to
    /// the zone, a frame placed a cell off) do not thicken or double.
    #[cfg(test)]
    fn add_walls(&mut self, rel: &[(i32, i32)], pos: (i32, i32), big: bool) {
        self.add_walls_in(rel, pos, big, None);
    }

    /// `add_walls`, with the frame's area in the part's cells (x0, y0, x1, y1): held cells
    /// in it that the frame does not show count as missed (`Held`).
    fn add_walls_in(&mut self, rel: &[(i32, i32)], pos: (i32, i32), big: bool, area: Option<[i32; 4]>) {
        if let (true, Some([x0, y0, x1, y1])) = (big, area) {
            let mut noise = Vec::new();
            for (&(x, y), held) in self.held.iter_mut() {
                if x >= x0 && x < x1 && y >= y0 && y < y1 {
                    held.in_view += 1;
                    if held.in_view >= HELD_DROP.0 && (held.seen as f64) < HELD_DROP.1 * held.in_view as f64 {
                        noise.push((x, y));
                    }
                }
            }
            for cell in noise {
                self.held.remove(&cell);
                self.walls.remove(&cell);
            }
        }
        // Which known cell each one is, decided before this frame adds any: a line's own
        // cells touch each other and are all new.
        let cells: Vec<((i32, i32), bool)> = rel
            .iter()
            .map(|&(rx, ry)| {
                let cell = (pos.0 + rx, pos.1 + ry);
                let known = self.known_as(cell);
                (known, known == cell)
            })
            .collect();
        for (cell, exact) in cells {
            self.count(cell, 1, pos, big, exact);
        }
    }

    /// The known wall cell `cell` is: itself, else a known neighbour, else itself (new).
    fn known_as(&self, cell: (i32, i32)) -> (i32, i32) {
        if self.walls.contains_key(&cell) {
            return cell;
        }
        NEIGHBOURS.iter().map(|&(dx, dy)| (cell.0 + dx, cell.1 + dy)).find(|c| self.walls.contains_key(c)).unwrap_or(cell)
    }

    /// Counts `times` sightings of the wall at `cell`. Only a sighting of the cell itself
    /// (`exact`) frees a held one: a neighbour seen from elsewhere is often scene.
    fn count(&mut self, cell: (i32, i32), times: u32, pos: (i32, i32), big: bool, exact: bool) {
        {
            let n = self.walls.entry(cell).or_insert(0);
            *n = (*n + times).min(SEEN_MOST);
            let n = *n;
            if !big {
                self.held.remove(&cell);
            } else if n == times {
                self.held.insert(cell, Held { first: pos, seen: 1, in_view: 1 });
            } else if let Some(held) = self.held.get_mut(&cell) {
                // A neighbour's sighting is not this cell's: in a patch of scene specks every
                // cell has neighbours seen now and then.
                if exact {
                    held.seen = (held.seen + 1).min(held.in_view.max(1));
                }
                let far = (held.first.0 - pos.0).abs().max((held.first.1 - pos.1).abs()) >= HOLD_MOVE;
                if exact && far && held.in_view >= HELD_MIN_VIEW && held.seen as f64 >= HELD_SHARE * held.in_view as f64 {
                    self.held.remove(&cell);
                }
            }
            if !self.held.contains_key(&cell) {
                self.kept.insert(cell, n);
            }
        }
    }

    fn snapshot(&self) -> Part {
        let mut part = self.part.clone();
        let mut walls: Vec<[i32; 3]> = self.kept().map(|(&(x, y), &n)| [x, y, n as i32]).collect();
        walls.sort_unstable();
        part.walls = walls;
        part
    }

    fn add_poi(&mut self, kind: &str, label: &str) {
        let (x, y) = self.pos;
        self.part.pois.push(Poi { kind: kind.into(), label: label.into(), x, y, at: now() });
    }
}

/// A boss named in a zone's checklist, and where the layout graphs put it: at the
/// exit to an area, at a tagged node, or (neither given) at a node tagged "boss".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Boss {
    /// As the checklist writes it ("Beira", "The Bloated Miller").
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

impl Boss {
    /// The log names the speaker in full ("Beira of the Rotten Pack"); the checklist
    /// may shorten it.
    fn speaks_as(&self, speaker: &str) -> bool {
        let (name, speaker) = (self.name.to_lowercase(), speaker.to_lowercase());
        name.len() >= 4 && speaker.contains(&name)
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
    /// Exit icons outside the wall frame from the last whole-screen icon scan, in
    /// this part's coordinates, and how many frames ago that scan was.
    far_icons: FarIcons,
    far_age: u32,
    /// Exits the map showed by name during this part: (x, y) in the part's cells and
    /// the zone they lead to. An exit does not move: seen once, its place is known.
    named_exits: Vec<(f32, f32, String)>,
    /// Every exit icon's place seen during this part, named or not (the part's cells).
    exit_places: Vec<(f32, f32)>,
    /// Where this part saw the zone's waypoint icon (the part's cells).
    waypoint_at: Option<(f32, f32)>,
    /// The last arrow of this visit, kept while a new one cannot be worked out.
    last_guide: std::cell::RefCell<Option<guide::GuideView>>,
    /// The flight recorder: what the navigator is fed (with a folder only).
    obs: std::cell::RefCell<Option<obs::Log>>,
    /// Bosses whose first line in this visit was already taken as "near its place".
    bosses_heard: Vec<String>,
    /// Where the live part saw each boss pair's icon alive and grey (gone with the part),
    /// and the zone's bosses the map showed dead in this visit (`Recorder::dead_bosses`).
    boss_places: Vec<BossPlace>,
    dead: Vec<String>,
    /// Named icons by place, for `confirm_named`, and the frames fed so far.
    named_seen: Vec<NamedSeen>,
    frame_no: u32,
    /// Checkpoints handed to the navigator in this part (the part's cells).
    checkpoint_marks: Vec<(f32, f32)>,
    /// When the zone was entered, and what the first placed frame showed at the
    /// arrival point (in the live part's coordinates; gone with the part).
    entered: Instant,
    came_in: Option<CameIn>,
}

/// Where the live part saw a boss pair's icon (the latest place), alive or grey, and the
/// one checklist boss the pair is (it names it, or the zone has one boss).
#[derive(Clone, Debug)]
struct BossPlace {
    at: (f32, f32),
    alive: bool,
    pair: String,
    who: Option<String>,
}

/// A named icon (`confirm_named`): its sprite, place (the part's cells), in how many
/// frames it was read there, and the last.
#[derive(Clone, Debug)]
struct NamedSeen {
    sprite: Option<&'static str>,
    at: (f32, f32),
    frames: u32,
    last: u32,
}

/// The same named icon in two frames: this close (cells), within this many frames.
const SAME_ICON: f32 = 4.0;
const NAMED_FRAMES: u32 = 33;
/// A grey twin this close (cells) to where this part saw the pair alive is that boss, dead.
const SAME_PLACE: f32 = 12.0;

/// The arrival point and what tells whether the player came in by the exit there.
struct CameIn {
    from: String,
    at: (i32, i32),
    /// Seconds from the zone entry to the first placed frame.
    delay: f64,
    /// A waypoint icon within `ARRIVAL_WAYPOINT` cells.
    waypoint: bool,
    /// Exit icons within `ARRIVAL_EXIT` cells: their labels (None = unread).
    exits: Vec<Option<String>>,
}

/// Around the arrival point: a waypoint this close (cells) means the player may have
/// come by it; an exit this close is the one come through.
const ARRIVAL_WAYPOINT: f32 = 15.0;
const ARRIVAL_EXIT: f32 = 40.0;

/// The "sprite" of an exit icon that is not in the frame: an exit read earlier, or the one
/// the player came in by, placed by the recorded map (`nav::ExitIcon::remembered`).
const REMEMBERED: &str = "remembered";

/// Seconds after the zone entry within which the first placed frame is the arrival.
const ARRIVAL_SOON: f64 = 5.0;
/// The arrival exit counts beside another named exit at least this far from it (cells).
const ARRIVAL_APART: f32 = 80.0;

/// The exit the player came in by, as an icon of this frame: where they appeared (the
/// part's cells), named after the zone they came from. Only when they came by the exit
/// (placed soon after the entry, an exit icon by them, no waypoint), and only beside
/// another exit read by name far enough away: the pair is what tells layouts apart
/// (the owner's Clearfell, 2026-11-12: the town's exit and the Mud Burrow's name fit two
/// of 33 layouts, and the navigator held a third at 90%).
fn arrival_icon(came_in: Option<&CameIn>, icons: &[crate::minimap::bigmap::Icon], pos: (i32, i32)) -> Option<crate::minimap::bigmap::Icon> {
    let c = came_in.filter(|c| !c.from.is_empty() && c.delay <= ARRIVAL_SOON && !c.waypoint && !c.exits.is_empty())?;
    let (x, y) = ((c.at.0 - pos.0) as f32, (c.at.1 - pos.1) as f32);
    // The name read at the arrival, else the zone's id (the navigator takes either).
    let name = c.exits.iter().flatten().next().cloned().unwrap_or_else(|| c.from.clone());
    let other = |i: &&crate::minimap::bigmap::Icon| {
        i.kind == "exit" && icon_label(i).is_some_and(|l| l != name) && (i.x - x).hypot(i.y - y) >= ARRIVAL_APART
    };
    icons.iter().find(other)?;
    Some(crate::minimap::bigmap::Icon { kind: "exit", x, y, label: Some(name), sprite: Some(REMEMBERED) })
}

/// The zone an exit icon's label names (as in the area's connectionNames), when the
/// reader could read it: a named anchor. None is "some exit".
fn icon_label(icon: &crate::minimap::bigmap::Icon) -> Option<String> {
    icon.label.clone().filter(|l| !l.is_empty())
}

/// Exits kept from the last whole-window scan: place in the part's cells, label.
type FarIcons = Vec<(f32, f32, Option<String>)>;

/// Which of the zone's bosses a map icon may be (for `Recorder::dead_bosses`): only a boss
/// pair's own icon, alive or grey (it carries its place's name: the Miller's, the
/// Riverbank), in its own area, for the boss the pair names, else (unnamed) for any of the
/// zone's bosses; never a pair known to be no checklist boss's (the Grelwood's plant). An
/// orange skull (a unique monster) is read and logged and counts for none: an ordinary
/// unique walks about, and no campaign boss was seen as one. A grey skull: none.
pub(crate) fn boss_candidates<'b>(icon: &crate::minimap::bigmap::Icon, area: &str, bosses: &'b [Boss]) -> Vec<&'b Boss> {
    if !matches!(icon.kind, "boss" | "boss-dead") {
        return vec![];
    }
    let sprites = crate::minimap::bigmap::sprites();
    let Some(s) = icon.sprite.and_then(|id| sprites.iter().find(|s| s.id == id)).filter(|s| s.pair.is_some()) else { return vec![] };
    if s.area.as_deref() != Some(area) || !s.checklist {
        return vec![];
    }
    match &s.boss {
        Some(name) => bosses.iter().filter(|b| b.name.eq_ignore_ascii_case(name)).collect(),
        None => bosses.iter().collect(),
    }
}

/// The boss pair (`data/icons/index.json`) an icon's sprite belongs to, if any.
fn boss_pair(icon: &crate::minimap::bigmap::Icon) -> Option<String> {
    let sprites = crate::minimap::bigmap::sprites();
    icon.sprite.and_then(|id| sprites.iter().find(|s| s.id == id)).and_then(|s| s.pair.clone())
}

/// The checkpoints of a flight recorder's icons line (written once a part, as the recorder
/// hands them to the navigator: `Session::new_checkpoints`), for the replay.
#[cfg(test)]
fn checkpoint_list(icons: &[crate::minimap::bigmap::Icon]) -> Vec<(f64, f64)> {
    icons.iter().filter(|i| i.kind == "checkpoint").map(|i| (i.x as f64, i.y as f64)).collect()
}

/// The exits handed to the navigator with one frame (`Session::feed_icons`; the replay
/// of a flight recording builds them the same way). Every third big-map frame lists every
/// icon on screen, the frames between only those inside the wall frame: icons outside it
/// are kept from the last whole scan for a few frames.
fn exit_list(
    icons: &[crate::minimap::bigmap::Icon],
    whole: bool,
    pos: (i32, i32),
    size: (usize, usize),
    far_icons: &mut FarIcons,
    far_age: &mut u32,
) -> Vec<nav::ExitIcon> {
    let (hw, hh) = (size.0 as f32 / 2.0, size.1 as f32 / 2.0);
    let outside = |x: f32, y: f32| x.abs() > hw || y.abs() > hh;
    let fed: Vec<&crate::minimap::bigmap::Icon> = icons.iter().filter(|i| i.kind == "exit").collect();
    let mut list: Vec<nav::ExitIcon> = fed
        .iter()
        .map(|i| nav::ExitIcon { dx: i.x as f64, dy: i.y as f64, to: icon_label(i), remembered: i.sprite == Some(REMEMBERED) })
        .collect();
    if whole {
        // A whole-window scan (marked by the reader): its far icons replace the
        // kept ones, none if it saw none.
        // Exits not in the frame (remembered ones) are added to every frame anyway.
        *far_icons = fed
            .iter()
            .filter(|i| outside(i.x, i.y) && i.sprite != Some(REMEMBERED))
            .map(|i| (pos.0 as f32 + i.x, pos.1 as f32 + i.y, icon_label(i)))
            .collect();
        *far_age = 0;
    } else if *far_age < FAR_FRAMES {
        *far_age += 1;
        for (x, y, to) in far_icons.iter() {
            let (dx, dy) = (x - pos.0 as f32, y - pos.1 as f32);
            // Not when this frame shows it already (an icon in the capture's margin).
            let shown = fed.iter().any(|i| (i.x - dx).abs() <= 10.0 && (i.y - dy).abs() <= 10.0);
            if outside(dx, dy) && !shown {
                list.push(nav::ExitIcon { dx: dx as f64, dy: dy as f64, to: to.clone(), remembered: false });
            }
        }
    }
    list
}

/// Remembers the frame's exit icons that were read by name, at their place in the part.
fn note_named_exits(named: &mut Vec<(f32, f32, String)>, icons: &[crate::minimap::bigmap::Icon], pos: (i32, i32)) {
    for icon in icons.iter().filter(|i| i.kind == "exit") {
        let Some(to) = icon_label(icon) else { continue };
        let at = (pos.0 as f32 + icon.x, pos.1 as f32 + icon.y);
        match named.iter_mut().find(|(_, _, name)| *name == to) {
            Some(known) => (known.0, known.1) = at,
            None => named.push((at.0, at.1, to)),
        }
    }
}

/// The named exits as points of the part ("exit", their name), so they are saved with the
/// map and a return to the instance (or an app restart in it) has them again.
fn keep_named_exits(pois: &mut Vec<Poi>, named: &[(f32, f32, String)]) {
    for (x, y, name) in named {
        let (x, y) = (x.round() as i32, y.round() as i32);
        match pois.iter_mut().find(|p| p.kind == "exit" && p.label == *name) {
            Some(poi) => (poi.x, poi.y) = (x, y),
            None => pois.push(Poi { kind: "exit".into(), label: name.clone(), x, y, at: now() }),
        }
    }
}

/// Exits read by name earlier in this part (`Session::named_exits`, the part's cells) that
/// this frame does not show, as icons from the player at `pos`.
fn remembered_exits(named: &[(f32, f32, String)], icons: &[crate::minimap::bigmap::Icon], pos: (i32, i32)) -> Vec<crate::minimap::bigmap::Icon> {
    named
        .iter()
        .map(|(x, y, name)| (x - pos.0 as f32, y - pos.1 as f32, name))
        .filter(|&(dx, dy, name)| {
            !icons.iter().any(|i| i.kind == "exit" && (i.label.as_ref() == Some(name) || (i.x - dx).hypot(i.y - dy) <= 20.0))
        })
        .map(|(x, y, name)| crate::minimap::bigmap::Icon { kind: "exit", x, y, label: Some(name.clone()), sprite: Some(REMEMBERED) })
        .collect()
}

/// The tag of a target that is the zone's waypoint icon (`tracker::NavigationTarget::icon`).
const WAYPOINT_TARGET: &str = "icon:waypoint";

/// A target that is a map icon, not a place of the layout graphs.
fn is_icon_target(target: &guide::NavTarget) -> bool {
    target.tag.as_deref().is_some_and(|tag| tag.starts_with("icon:"))
}

/// A needle straight from the player at `pos` to `at` (the part's cells): sure ("estimated",
/// confidence 1) or likely ("rough", 0.5).
fn straight_needle(variant: &str, kind: &str, target: &guide::NavTarget, (x, y): (f32, f32), pos: (i32, i32), sure: bool) -> guide::GuideView {
    let (dx, dy) = (x - pos.0 as f32, y - pos.1 as f32);
    guide::GuideView {
        variant: variant.into(),
        target_kind: kind.into(),
        target: target.label.clone(),
        degrees: Some(dy.atan2(dx).to_degrees()),
        source: if sure { "estimated" } else { "rough" }.into(),
        distance: dx.hypot(dy),
        confidence: if sure { 1.0 } else { 0.5 },
    }
}

/// The needle straight to a target's icon at `at` (the part's cells), sure: the icon is
/// the thing. None until the map has shown it.
fn icon_guide(target: &guide::NavTarget, at: Option<(f32, f32)>, pos: (i32, i32)) -> Option<guide::GuideView> {
    Some(straight_needle("icon", "waypoint", target, at?, pos, true))
}

/// Remembers where the frame's exit icons are, one place per exit (icons within
/// `SAME_EXIT` cells are the same exit seen again).
fn note_exit_places(places: &mut Vec<(f32, f32)>, icons: &[crate::minimap::bigmap::Icon], pos: (i32, i32)) {
    const SAME_EXIT: f32 = 20.0;
    for icon in icons.iter().filter(|i| i.kind == "exit") {
        let at = (pos.0 as f32 + icon.x, pos.1 as f32 + icon.y);
        match places.iter_mut().find(|p| (p.0 - at.0).hypot(p.1 - at.1) <= SAME_EXIT) {
            Some(known) => *known = at,
            None => places.push(at),
        }
    }
}

/// A likely needle to `target`'s exit when the part has seen exactly one exit icon and
/// no name: with two or more there is no telling which.
fn only_exit_guide(target: &guide::NavTarget, places: &[(f32, f32)], pos: (i32, i32)) -> Option<guide::GuideView> {
    target.exit.as_ref()?;
    let [at] = places else { return None };
    Some(straight_needle("only exit", "entrance", target, *at, pos, false))
}

/// The needle straight to `target`'s exit when the map has shown that exit by name:
/// sure, as the icon is the exit. For zones the navigator does not cover.
fn named_exit_guide(target: &guide::NavTarget, named: &[(f32, f32, String)], pos: (i32, i32)) -> Option<guide::GuideView> {
    target.exit.as_ref()?;
    let (x, y, _) = named.iter().find(|(_, _, name)| name.eq_ignore_ascii_case(&target.label))?;
    Some(straight_needle("named exit", "entrance", target, (*x, *y), pos, true))
}

impl Session {
    /// The frame's icons with the named kinds that colour does not make (a boss's icon, a
    /// grey one, a skull) kept only once read at the same map place (`SAME_ICON` cells, the
    /// same sprite) in two frames: an icon stays where it is, the scene a sprite was matched
    /// on does not. A boss pair's grey icon counts only frames without a live icon of its
    /// pair (the dead rule wants it grey and alone). Exits, checkpoints and waypoints pass
    /// as read.
    fn confirm_named(&mut self, icons: &[crate::minimap::bigmap::Icon], pos: (i32, i32)) -> Vec<crate::minimap::bigmap::Icon> {
        self.frame_no += 1;
        let frame = self.frame_no;
        self.named_seen.retain(|n| frame - n.last <= NAMED_FRAMES);
        let live: Vec<String> = icons.iter().filter(|i| i.kind == "boss").filter_map(boss_pair).collect();
        let mut out = Vec::with_capacity(icons.len());
        for icon in icons {
            if matches!(icon.kind, "exit" | "checkpoint" | "waypoint") {
                out.push(icon.clone());
                continue;
            }
            if icon.kind == "boss-dead" && boss_pair(icon).is_some_and(|p| live.contains(&p)) {
                continue;
            }
            let at = (pos.0 as f32 + icon.x, pos.1 as f32 + icon.y);
            let seen = self.named_seen.iter_mut().find(|n| n.sprite == icon.sprite && (n.at.0 - at.0).hypot(n.at.1 - at.1) <= SAME_ICON);
            let frames = match seen {
                Some(n) => {
                    if n.last != frame {
                        (n.frames, n.last) = (n.frames + 1, frame);
                    }
                    n.at = at;
                    n.frames
                }
                None => {
                    self.named_seen.push(NamedSeen { sprite: icon.sprite, at, frames: 1, last: frame });
                    1
                }
            };
            if frames >= 2 {
                out.push(icon.clone());
            }
        }
        out
    }

    /// Keeps where this part saw each boss pair's icon, alive or grey, and names a boss dead
    /// by one rule, the one the Riverbank showed (the Miller's icon with its place name
    /// while he lived; 4 minutes after his last line the grey twin, nameless, at the same
    /// place): this part saw the pair's live icon at a place, and now reads its grey twin
    /// within `SAME_PLACE` cells of it and no live one of that pair. The boss is the one the
    /// pair names, or the zone's only boss for an unnamed pair. A grey skull, a grey twin
    /// never seen alive, a pair of another area: never.
    fn see_bosses(&mut self, icons: &[crate::minimap::bigmap::Icon], pos: (i32, i32), bosses: &[Boss]) {
        let area = self.recording.area.clone();
        let live: Vec<String> = icons.iter().filter(|i| i.kind == "boss").filter_map(boss_pair).collect();
        for icon in icons.iter().filter(|i| matches!(i.kind, "boss" | "boss-dead")) {
            let Some(pair) = boss_pair(icon) else { continue };
            let at = (pos.0 as f32 + icon.x, pos.1 as f32 + icon.y);
            let alive = icon.kind == "boss";
            if !alive && !live.contains(&pair) {
                let dead = self
                    .boss_places
                    .iter()
                    .filter(|b| b.alive && b.pair == pair && (b.at.0 - at.0).hypot(b.at.1 - at.1) <= SAME_PLACE)
                    .find_map(|b| b.who.clone());
                if let Some(name) = dead.filter(|n| !self.dead.contains(n)) {
                    self.dead.push(name);
                }
            }
            let who = match boss_candidates(icon, &area, bosses).as_slice() {
                [one] => Some(one.name.clone()),
                _ => None,
            };
            match self.boss_places.iter_mut().find(|b| b.alive == alive && b.pair == pair) {
                Some(b) => b.at = at,
                None => self.boss_places.push(BossPlace { at, alive, pair, who }),
            }
        }
        // Where to look again in the next frame: each pair seen, from where the player is.
        let mut hints: Vec<(String, [f32; 2])> = Vec::new();
        for b in self.boss_places.iter().rev() {
            if !hints.iter().any(|(p, _)| *p == b.pair) {
                hints.push((b.pair.clone(), [b.at.0 - pos.0 as f32, b.at.1 - pos.1 as f32]));
            }
        }
        crate::minimap::bigmap::set_icon_hints(hints);
    }

    /// The checkpoints of the frame not seen before in this part (within `MARK_SAME`
    /// cells), as the flight recorder writes them: each is handed to the navigator once,
    /// so a replay feeds exactly what the app fed.
    fn new_checkpoints(&mut self, icons: &[crate::minimap::bigmap::Icon], pos: (i32, i32)) -> Vec<(f64, f64)> {
        let mut new = Vec::new();
        for icon in icons.iter().filter(|i| i.kind == "checkpoint") {
            let at = (pos.0 as f32 + icon.x, pos.1 as f32 + icon.y);
            if !self.checkpoint_marks.iter().any(|m| ((m.0 - at.0).hypot(m.1 - at.1) as f64) <= obs::MARK_SAME) {
                self.checkpoint_marks.push(at);
                new.push((icon.x as f64, icon.y as f64));
            }
        }
        new
    }

    /// Hands the navigator the frame's exit icons (see `exit_list`) and its new
    /// checkpoints; the named kinds only once seen in two frames (`confirm_named`).
    fn feed_icons(&mut self, icons: &[crate::minimap::bigmap::Icon], whole: bool, pos: (i32, i32), size: (usize, usize), bosses: &[Boss]) {
        let mut icons = self.confirm_named(icons, pos);
        // The exit the player came in by is a named exit at a known place of this map:
        // with another exit read by name it fixes the layout as two named icons do.
        icons.extend(arrival_icon(self.came_in.as_ref(), &icons, pos));
        // So is every exit read by name earlier in this part: a visit is one map, so two
        // exits seen minutes apart are a pair as much as two seen in one frame.
        icons.extend(remembered_exits(&self.named_exits, &icons, pos));
        let icons = icons.as_slice();
        self.see_bosses(icons, pos, bosses);
        let checkpoints = self.new_checkpoints(icons, pos);
        let Some(nav) = self.navigator.as_mut() else { return };
        if let Some(log) = self.obs.get_mut() {
            // Far exits this frame uses or replaces: the replay needs this frame's icons.
            let far = !self.far_icons.is_empty() && (whole || self.far_age < FAR_FRAMES);
            log.icons(obs::now_ms(), icons, whole, size, far);
        }
        let list = exit_list(icons, whole, pos, size, &mut self.far_icons, &mut self.far_age);
        nav.icons(&list);
        nav.checkpoints(&checkpoints, &list);
    }

    fn obs_file(&self) -> Option<PathBuf> {
        Some(self.file.as_ref()?.with_extension("obs.jsonl"))
    }

    fn nav_file(&self) -> Option<PathBuf> {
        Some(self.file.as_ref()?.with_extension("nav.json"))
    }

    /// Hands the navigator the move to `pos`. Within a part the move is measured
    /// exactly (the frames were registered), however long the pause between them;
    /// only a new part is a loss.
    /// A new part starts: its cells are new, so every place noted in the old part's cells
    /// means nothing in it. Kept, the named exits of a saved map that could not be found
    /// again were read at the wrong places in the new part: the town's exit 70 cells from
    /// the Grelwood's, which fits no layout (the owner's Clearfell, 2026-11-12).
    fn forget_places(&mut self) {
        self.fed = None;
        self.far_icons.clear();
        self.named_exits.clear();
        self.exit_places.clear();
        self.waypoint_at = None;
        self.came_in = None;
        self.boss_places.clear();
        self.named_seen.clear();
        self.checkpoint_marks.clear();
        crate::minimap::bigmap::set_icon_hints(vec![]);
    }

    fn feed(&mut self, pos: (i32, i32)) {
        let Some(nav) = self.navigator.as_mut() else { return };
        let mut log = self.obs.get_mut().as_mut();
        let t = if log.is_some() { obs::now_ms() } else { 0 };
        if let Some(log) = log.as_deref_mut() {
            log.frame();
        }
        let gap = self.fed_at.map(|t| t.elapsed().as_secs_f64());
        match self.fed {
            Some(prev) => {
                let (dx, dy) = (pos.0 - prev.0, pos.1 - prev.1);
                if let Some(log) = log {
                    log.moved(t, dx, dy);
                }
                nav.moved(dx as f64, dy as f64)
            }
            // A new part: where it starts relative to the last one is unknown.
            None => {
                if let Some(gap) = gap {
                    if let Some(log) = log {
                        log.lost(t, gap);
                    }
                    nav.lost(gap);
                }
            }
        }
        self.fed = Some(pos);
        self.fed_at = Some(Instant::now());
    }
}

struct State {
    /// The next frame's icons came from a whole-window scan.
    icons_whole: bool,
    /// Map icons for the next frame, in minimap cells from the player.
    icons: Vec<crate::minimap::bigmap::Icon>,
    /// (area, label) of targets whose step the log completed.
    log_completed: std::collections::HashSet<(String, String)>,
    /// The game's layout graphs, built per area on first use.
    graphs: graph::Graphs,
    completed: HashMap<String, Vec<String>>,
    /// Open checklist targets per area, in checklist order.
    targets: HashMap<String, Vec<guide::NavTarget>>,
    /// Every checklist target seen, per area (to find a step the log just completed).
    known_targets: HashMap<String, Vec<guide::NavTarget>>,
    /// Per area: its bosses from the checklists, with where each stands.
    bosses: HashMap<String, Vec<Boss>>,
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
    /// Only the needle is new: the map (walls, trail, pois) is left out and the page
    /// keeps the one it has. Sent on the frames between two full views.
    light: bool,
    /// The recognised layout (full views only).
    #[serde(skip_serializing_if = "Option::is_none")]
    layout: Option<nav::Picture>,
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
/// a coarse search over every overlap (or only within `REFIND` of `around`), then a
/// fine one around the best few.
/// `piece` walls are in the piece's own coordinates; returns the shift onto `walls`.
fn locate_piece(walls: &HashMap<(i32, i32), u32>, piece: &[(i32, i32)], around: Option<(i32, i32)>) -> Option<(i32, i32)> {
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
    let (mut sx0, mut sy0, mut sx1, mut sy1) = (mx0 - px1, my0 - py1, mx1 - px0, my1 - py0);
    if let Some((x, y)) = around {
        (sx0, sy0) = (sx0.max((x - REFIND).div_euclid(c)), sy0.max((y - REFIND).div_euclid(c)));
        (sx1, sy1) = (sx1.min((x + REFIND).div_euclid(c)), sy1.min((y + REFIND).div_euclid(c)));
    }
    let mut scored = Vec::new();
    for dy in sy0..=sy1 {
        for dx in sx0..=sx1 {
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
        let Some((dx, dy)) = locate_piece(&live.walls, &piece, None) else { continue };
        let part = session.recording.parts.remove(index);
        // A kept piece's cells are kept; a cell next to a known one is that one.
        let cells: Vec<((i32, i32), u32)> = part.walls.iter().map(|&[x, y, n]| (live.known_as((x + dx, y + dy)), n as u32)).collect();
        for (cell, n) in cells {
            let pos = live.pos;
            live.count(cell, n, pos, false, true);
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
        // Flight recordings are kept for a month.
        if let Some(dir) = &dir {
            obs::prune(dir);
        }
        Recorder(Mutex::new(State {
            graphs: Default::default(),
            completed: HashMap::new(),
            targets: HashMap::new(),
            known_targets: HashMap::new(),
            bosses: HashMap::new(),
            icons: Vec::new(),
            icons_whole: false,
            log_completed: Default::default(),
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
        let mut state = crate::relock(&self.0);
        match event {
            LogEvent::ZoneEntered { area, level, seed } => {
                let left = state.session.as_ref().map(|s| s.recording.area.clone());
                let respawn = live && left.as_deref() == Some(area.as_str());
                if let Some(session) = state.session.as_mut() {
                    if live && session.started {
                        session.live.add_poi("left", area);
                    }
                    // Leaving to another zone: probably through its exit, but a portal,
                    // a waypoint or a death leave from anywhere: soft evidence only.
                    if live && *area != session.recording.area {
                        if let Some(nav) = session.navigator.as_mut() {
                            if let Some(log) = session.obs.get_mut() {
                                log.seen(obs::now_ms(), nav::Seen::Left(area));
                            }
                            nav.seen(nav::Seen::Left(area));
                        }
                    }
                }
                state.finish();
                // The map reader compares this area's boss icons too; nothing seen yet.
                crate::minimap::bigmap::set_icon_area(Some(area));
                crate::minimap::bigmap::set_icon_hints(vec![]);
                // A live arrival with no zone before it (e.g. a new character) still starts at the arrival point.
                state.previous_area = if live { Some(left.unwrap_or_default()) } else { None };
                // A town (seed 1) has one file for every visit: pieces of many visits piled
                // up there and drifted (the owner's Clearfell Encampment, 2026-11-12). Its
                // map is small and seen again in seconds: each visit starts a new one.
                let recording = Some(seed)
                    .filter(|seed| **seed != Some(1))
                    .and_then(|seed| state.load(area, *seed))
                    .unwrap_or(Recording {
                        version: FORMAT_VERSION,
                        area: area.clone(),
                        level: *level,
                        seed: *seed,
                        cell_px: crate::minimap::CELL,
                        capture_px: [0, 0],
                        parts: vec![],
                    });
                // From a zone that is no neighbour (no exit leads there): the zone's start, not
                // that zone's exit. A new character's spawn in the Riverbank was recorded as
                // "arrived from G1_2", the old character's last zone (the owner's run of
                // 2026-11-12); the navigator already started at the start node, the recording
                // and the flight recorder took the exit's name.
                let arrived_from = state.previous_area.clone().map(|from| {
                    let neighbour = from.is_empty() || state.graphs.get(area).is_none_or(|g| g.names.contains_key(&from));
                    if neighbour { from } else { String::new() }
                });
                // A return to the instance goes on with its map: the largest part, the player
                // looked for first where they came in by the same exit before.
                let mut recording = recording;
                let largest = (0..recording.parts.len()).max_by_key(|&i| recording.parts[i].walls.len());
                // Not after a death: the zone is generated again where it was left (a respawn at a
                // checkpoint), and found on the old map from anywhere the map went on about 100
                // cells off (the owner's Clearfell, 2026-11-12). A new part starts; the old one
                // is joined when the walls line up (`merge_pieces`).
                let live = match largest {
                    Some(i) if !respawn && recording.parts[i].walls.len() >= MERGE_MIN_WALLS => {
                        let part = recording.parts.remove(i);
                        let from = arrived_from.as_deref();
                        // Came in by that exit before, or left by it (a visit whose arrival was not
                        // known, as after an app restart, still noted where it left for town).
                        let at = |kind: &str| part.pois.iter().filter(|p| p.kind == kind && Some(p.label.as_str()) == from).map(|p| (p.x, p.y)).last();
                        let hint = from.filter(|from| !from.is_empty()).and_then(|_| at("arrived").or_else(|| at("left")));
                        Live::from_part(part, hint)
                    }
                    _ => Live::new(),
                };
                let file = state.path(area, *seed);
                // The exits the continued map knows by name.
                let known_exits: Vec<(f32, f32, String)> =
                    live.part.pois.iter().filter(|p| p.kind == "exit").map(|p| (p.x as f32, p.y as f32, p.label.clone())).collect();
                let mut session = Session {
                    recording,
                    live,
                    came_from: arrived_from.clone().unwrap_or_default(),
                    arrived_from,
                    started: false,
                    file,
                    navigator: None,
                    fed: None,
                    fed_at: None,
                    far_icons: Vec::new(),
                    far_age: FAR_FRAMES,
                    named_exits: known_exits,
                    exit_places: Vec::new(),
                    waypoint_at: None,
                    last_guide: Default::default(),
                    obs: Default::default(),
                    bosses_heard: Vec::new(),
                    boss_places: Vec::new(),
                    dead: Vec::new(),
                    named_seen: Vec::new(),
                    frame_no: 0,
                    checkpoint_marks: Vec::new(),
                    entered: Instant::now(),
                    came_in: None,
                };
                if let Some((navigator, start)) = state.navigator(&session) {
                    session.navigator = Some(navigator);
                    let history = state.dir.as_ref().map(|d| d.join(obs::HISTORY_FILE));
                    *session.obs.get_mut() = session.obs_file().map(|path| obs::Log::new(path, history, &start, obs::now_ms()));
                }
                state.session = Some(session);
            }
            LogEvent::Dialogue { speaker, .. } if live => {
                let State { session, bosses, .. } = &mut *state;
                if let Some(session) = session.as_mut().filter(|s| s.started) {
                    let known = session.live.part.pois.iter().any(|p| p.kind == "npc" && &p.label == speaker);
                    if !known {
                        session.live.add_poi("npc", speaker);
                    }
                    // A boss of this zone speaks when the player gets to it: the first
                    // line of a visit says the player is near that boss's place.
                    let boss = bosses
                        .get(&session.recording.area)
                        .and_then(|all| all.iter().find(|boss| boss.speaks_as(speaker)))
                        .filter(|boss| !session.bosses_heard.contains(&boss.name));
                    if let (Some(boss), Some(nav)) = (boss, session.navigator.as_mut()) {
                        session.bosses_heard.push(boss.name.clone());
                        let seen = match (&boss.exit, &boss.tag) {
                            (Some(exit), _) => nav::Seen::NearExit(exit),
                            (None, Some(tag)) => nav::Seen::NearTag(tag),
                            (None, None) => nav::Seen::NearTag("boss"),
                        };
                        if let Some(log) = session.obs.get_mut() {
                            log.seen(obs::now_ms(), seen);
                        }
                        nav.seen(seen);
                        // A picture of the map at that moment: is there a boss icon to read?
                        crate::minimap::request_sample();
                    }
                }
            }
            LogEvent::Dialogue { .. } => {}
        }
    }

    /// One analysed minimap frame. Returns an overview for the overlay, at most every 300 ms.
    pub fn on_frame(&self, frame: &MinimapView, capture_px: [usize; 2]) -> Option<LayoutView> {
        let mut state = crate::relock(&self.0);
        let icons = std::mem::take(&mut state.icons);
        let whole = std::mem::take(&mut state.icons_whole);
        let State { session, bosses, .. } = &mut *state;
        let session = session.as_mut()?;
        let bosses: &[Boss] = bosses.get(&session.recording.area).map_or(&[], Vec::as_slice);
        if !frame.visible {
            return None;
        }
        session.recording.capture_px = capture_px;
        let live = &mut session.live;
        if let Some([x, y]) = frame.player {
            let found = (x.round() as i32, y.round() as i32);
            // A frame of another size comes from the other view: its player cell is new.
            let same_view = live.frame_size == (frame.cols, frame.rows);
            live.frame_size = (frame.cols, frame.rows);
            live.centre = match live.centre {
                Some(c) if same_view && ((c.0 - found.0).abs() > 4 || (c.1 - found.1).abs() > 4) => Some(c),
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
        let mut gave_up = false;
        let placed = if live.walls.is_empty() {
            Some(live.pos)
        } else if live.unplaced {
            // On the map of an earlier visit: where on it, unambiguous only.
            let found = locate_piece(&live.walls, &rel, live.hint).or_else(|| live.hint.and_then(|_| locate_piece(&live.walls, &rel, None)));
            if found.is_some() {
                live.unplaced = false;
            } else {
                live.tries += 1;
                if live.tries >= FIND_FRAMES {
                    // Not found: the known map waits in the parts (joined when it can be),
                    // this visit starts a part of its own.
                    let known = std::mem::replace(live, Live::new());
                    live.centre = known.centre;
                    live.frame_size = known.frame_size;
                    session.recording.parts.push(known.snapshot());
                    gave_up = true;
                }
            }
            found
        } else {
            let predicted = (live.pos.0 + live.velocity.0, live.pos.1 + live.velocity.1);
            let (near, score) = register(&live.walls, &rel, predicted, NEAR);
            if score >= MIN_MATCH {
                Some(near)
            } else {
                let (wide, score) = register(&live.walls, &rel, live.pos, WIDE);
                // After a gap the player may be further away: a frame with enough of the map
                // (a wide big-map frame) is looked for around the last place, unambiguous only.
                (score >= MIN_MATCH).then_some(wide).or_else(|| locate_piece(&live.walls, &rel, Some(live.pos)))
            }
        };
        if gave_up {
            session.forget_places();
        }
        match placed {
            Some(pos) => {
                session.feed(pos);
                session.feed_icons(&icons, whole, pos, (frame.cols, frame.rows), bosses);
                note_named_exits(&mut session.named_exits, &icons, pos);
                keep_named_exits(&mut session.live.part.pois, &session.named_exits);
                note_exit_places(&mut session.exit_places, &icons, pos);
                if let Some(icon) = icons.iter().find(|i| i.kind == "waypoint") {
                    session.waypoint_at = Some((pos.0 as f32 + icon.x, pos.1 as f32 + icon.y));
                }
                let live = &mut session.live;
                live.velocity = ((pos.0 - live.pos.0).clamp(-NEAR, NEAR), (pos.1 - live.pos.1).clamp(-NEAR, NEAR));
                live.pos = pos;
                live.lost_streak = 0;
                live.part.matched += 1;
                // The frame's area on the part's map, for the held cells it should show.
                let area = [pos.0 - centre.0, pos.1 - centre.1, pos.0 - centre.0 + frame.cols as i32, pos.1 - centre.1 + frame.rows as i32];
                live.add_walls_in(&rel, pos, frame.message.starts_with("Reading big map"), Some(area));
                let moved = live.part.trail.last().is_none_or(|&[x, y]| (x - pos.0).abs() + (y - pos.1).abs() >= 2);
                if moved {
                    live.part.trail.push([pos.0, pos.1]);
                }
                if !session.started {
                    session.started = true;
                    // Without a live zone change (app started mid-zone) the origin is unknown.
                    match session.arrived_from.take() {
                        Some(from) => {
                            session.live.add_poi("arrived", &from);
                            let near = |i: &&crate::minimap::bigmap::Icon, kind: &str, d: f32| i.kind == kind && i.x.hypot(i.y) <= d;
                            session.came_in = Some(CameIn {
                                at: pos,
                                delay: session.entered.elapsed().as_secs_f64(),
                                waypoint: icons.iter().any(|i| near(&i, "waypoint", ARRIVAL_WAYPOINT)),
                                exits: icons.iter().filter(|i| near(i, "exit", ARRIVAL_EXIT)).map(icon_label).collect(),
                                from,
                            });
                        }
                        None => session.live.add_poi("start", ""),
                    }
                }
            }
            None => {
                let live = &mut session.live;
                live.part.lost += 1;
                live.lost_streak += 1;
                if live.lost_streak >= LOST_LIMIT {
                    // Lost track (teleport, map overlay, big jump): keep what we have, start over.
                    // The navigator carries on; the new part's coordinates are new.
                    let finished = std::mem::replace(&mut session.live, Live::new());
                    session.live.centre = finished.centre;
                    session.live.frame_size = finished.frame_size;
                    session.forget_places();
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
        // The needle on every frame, so it turns as the player moves; the map with it
        // about once a second (it is the larger part and changes slowly).
        let full = state.last_view.is_none_or(|t| t.elapsed() >= Duration::from_millis(900));
        if full {
            state.last_view = Some(Instant::now());
            return state.view();
        }
        state.needle_view()
    }

    pub fn view(&self) -> Option<LayoutView> {
        crate::relock(&self.0).view()
    }

    /// Writes the current recording now (before a restart).
    pub fn save_now(&self) {
        let state = crate::relock(&self.0);
        state.save();
        if let Some(session) = &state.session {
            if let Some(log) = session.obs.borrow_mut().as_mut() {
                log.flush_now();
            }
        }
    }

    /// The open checklist steps that have a target, as (area, target) in checklist order.
    pub fn set_targets(&self, targets: Vec<(String, guide::NavTarget)>) {
        let mut state = crate::relock(&self.0);
        state.targets.clear();
        for (area, target) in targets {
            let known = state.known_targets.entry(area.clone()).or_default();
            if !known.contains(&target) {
                known.push(target.clone());
            }
            state.targets.entry(area).or_default().push(target);
        }
    }

    /// The map icons of the frame about to be handed to `on_frame` (zone exits,
    /// checkpoints, the waypoint), in minimap cells from the player; its exits go to
    /// the navigator with the frame. `whole`: a whole-window scan, listing every icon on
    /// screen (other frames list only those inside the wall frame): the far icons of the
    /// last scan are replaced by this one's, also when it has none.
    pub fn on_icons(&self, icons: &[crate::minimap::bigmap::Icon], whole: bool) {
        let mut state = crate::relock(&self.0);
        state.icons = icons.to_vec();
        state.icons_whole = whole;
    }

    /// The current zone's bosses (names as given to `set_bosses`) that the map showed
    /// dead in this visit (see `Session::see_bosses` for when a grey icon counts). Empty
    /// outside a zone, and whenever no boss icon was read: being wrong about the icons
    /// costs nothing.
    pub fn dead_bosses(&self) -> Vec<String> {
        crate::relock(&self.0).session.as_ref().map(|s| s.dead.clone()).unwrap_or_default()
    }

    /// The zones' bosses, from the checklists: (area, boss). Set once at start.
    pub fn set_bosses(&self, bosses: Vec<(String, Boss)>) {
        let mut state = crate::relock(&self.0);
        state.bosses.clear();
        for (area, boss) in bosses {
            state.bosses.entry(area).or_default().push(boss);
        }
    }

    /// Targets whose step the log completed (not a hand tick), as (area, label). Call
    /// before `set_completed`: a newly completed one is an anchor for the navigator.
    pub fn set_log_completed(&self, done: Vec<(String, String)>) {
        crate::relock(&self.0).log_completed = done.into_iter().collect();
    }

    /// Replacing the snapshot also handles unticking/resetting a checklist.
    pub fn set_completed(&self, objectives: Vec<(String, String)>) -> Option<LayoutView> {
        let mut state = crate::relock(&self.0);
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
        // Only a navigator that got data has something new to keep (a log catch-up
        // replays old zone entries without any).
        if let (Some(nav), Some(path)) = (session.navigator.as_ref().filter(|n| n.changed()), session.nav_file()) {
            let memory = nav.memory();
            write_json(&path, &memory);
            if let Some(log) = session.obs.borrow_mut().as_mut() {
                log.history(obs::now_ms(), &memory);
            }
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
    /// Instances are told apart by area and seed; towns share seed 1 but have no
    /// graph, so no two instances share a `.nav.json` while that holds.
    /// With it, how it was made (the flight recorder's start).
    fn navigator(&self, session: &Session) -> Option<(nav::Navigator, obs::Start)> {
        let area = self.graphs.get(&session.recording.area)?;
        let memory: Option<nav::Memory> = session
            .nav_file()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let arrival = match &session.arrived_from {
            Some(from) => nav::Arrival::From(from),
            None => nav::Arrival::Unknown,
        };
        let scale = graph::SCALE;
        // As `Navigator::new` decides (a position at most `RESUME`, 600 s, old).
        let kept = memory.as_ref().filter(|m| m.version == nav::MEMORY_VERSION);
        let resumed = kept.is_some_and(|m| {
            matches!(arrival, nav::Arrival::Unknown) && !m.positions.is_empty() && now().saturating_sub(m.at) <= 600
        });
        let start = obs::Start {
            area: session.recording.area.clone(),
            seed: session.recording.seed,
            from: session.arrived_from.clone(),
            scale,
            memory: kept.cloned(),
            age: kept.map(|m| now().saturating_sub(m.at)),
            resumed,
        };
        Some((nav::Navigator::new(&area, arrival, memory.as_ref(), scale, session.recording.seed.unwrap_or(0)), start))
    }

    /// A checklist step of this area that the log just completed (the tracker
    /// marks it so; a hand tick is not) puts the player at its target.
    fn anchor_log_completions(&mut self, before: &HashMap<String, Vec<String>>) {
        let Some(session) = self.session.as_mut() else { return };
        let area = session.recording.area.as_str();
        let Some(nav) = session.navigator.as_mut() else { return };
        let was = before.get(area).cloned().unwrap_or_default();
        let known = self.known_targets.get(area).map(Vec::as_slice).unwrap_or_default();
        let by_log = |l: &String| self.log_completed.contains(&(area.to_string(), l.clone()));
        for label in self.completed.get(area).into_iter().flatten().filter(|l| !was.contains(l) && by_log(l)) {
            let Some(target) = known.iter().find(|t| &t.label == label) else { continue };
            let seen = match (&target.exit, &target.tag) {
                (Some(exit), _) => nav::Seen::Exit(exit),
                (None, Some(tag)) => nav::Seen::Tag(tag),
                (None, None) => continue,
            };
            if let Some(log) = session.obs.get_mut() {
                log.seen(obs::now_ms(), seen);
            }
            nav.seen(seen);
        }
    }

    /// Saves and closes the current recording.
    fn finish(&mut self) {
        self.save();
        let Some(mut session) = self.session.take() else { return };
        if let Some(log) = session.obs.get_mut() {
            log.end(obs::now_ms());
        }
    }

    /// The arrow, kept on screen: when it cannot be worked out for a moment (tracking
    /// lost, a piece without enough to go on), the last one stays, marked unconfirmed.
    fn guide(&self, session: &Session) -> Option<guide::GuideView> {
        let fresh = self.fresh_guide(session);
        let kept = fresh.is_none();
        let view = match fresh {
            Some(view) => {
                *session.last_guide.borrow_mut() = Some(view.clone());
                Some(view)
            }
            // Where there is no navigator (a town) a kept needle must still be for the
            // step now first: a needle to the exit is not kept once the waypoint is next.
            None => {
                let area = session.recording.area.as_str();
                let wanted = self.targets.get(area).and_then(|targets| targets.first()).map(|t| t.label.as_str());
                session
                    .last_guide
                    .borrow()
                    .clone()
                    .filter(|view| session.navigator.is_some() || Some(view.target.as_str()) == wanted)
                    .map(|view| guide::GuideView { source: "unconfirmed".into(), ..view })
            }
        };
        if let (Some(view), Some(log)) = (&view, session.obs.borrow_mut().as_mut()) {
            let stale = session.fed_at.is_none_or(|t| t.elapsed() > STALE);
            log.needle(obs::now_ms(), &self.nav_targets(session), view, kept, stale);
        }
        view
    }

    /// The targets the navigator's needle is asked for: the checklist's open targets (not
    /// icons), then the onward exit (zones without a checklist, or targets no layout has).
    fn nav_targets(&self, session: &Session) -> Vec<guide::NavTarget> {
        let area = session.recording.area.as_str();
        let completed = self.completed.get(area).map(Vec::as_slice).unwrap_or_default();
        let mut targets = self.targets.get(area).cloned().unwrap_or_default();
        targets.retain(|t| !is_icon_target(t));
        if let Some(graph) = self.graphs.get(area) {
            targets.extend(graph::target(&graph, &session.came_from, completed));
        }
        targets
    }

    fn fresh_guide(&self, session: &Session) -> Option<guide::GuideView> {
        let area = session.recording.area.as_str();
        let live = &session.live;
        if live.lost_streak > 0 {
            return None;
        }
        // A step that leads to the waypoint: straight to its icon once the map showed it.
        let first = self.targets.get(area).and_then(|targets| targets.first());
        if let Some(target) = first.filter(|t| t.tag.as_deref() == Some(WAYPOINT_TARGET)) {
            return icon_guide(target, session.waypoint_at, live.pos);
        }
        if let Some(nav) = &session.navigator {
            return nav.guide(&self.nav_targets(session)).map(|view| not_stale(view, session.fed_at));
        }
        // No layout graphs here (a town): the exit the map shows by name is where to go.
        if let Some(view) = first.and_then(|target| named_exit_guide(target, &session.named_exits, live.pos)) {
            return Some(view);
        }
        // Its name not read yet (the label cut off at the minimap's edge): the only exit
        // icon seen is likely the one, shown as a likely direction.
        first.and_then(|target| only_exit_guide(target, &session.exit_places, live.pos))
    }

    /// The view with only the needle (`LayoutView::light`).
    fn needle_view(&self) -> Option<LayoutView> {
        let session = self.session.as_ref()?;
        let live = &session.live;
        Some(LayoutView {
            area: session.recording.area.clone(),
            seed: session.recording.seed,
            part: session.recording.parts.len() + 1,
            frames: live.part.frames,
            matched: live.part.matched,
            cols: VIEW_COLS,
            rows: VIEW_ROWS,
            walls: vec![],
            trail: vec![],
            player: [0.0, 0.0],
            pois: vec![],
            guide: self.guide(session),
            light: true,
            layout: None,
        })
    }

    fn view(&self) -> Option<LayoutView> {
        let session = self.session.as_ref()?;
        let live = &session.live;
        let solid: Vec<(i32, i32)> = live.kept().filter(|(_, &n)| n >= 2).map(|(&k, _)| k).collect();
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
            light: false,
            layout: session.navigator.as_ref().and_then(|n| n.picture()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exit_the_map_named_leads_the_needle_where_no_layout_is_known() {
        use crate::minimap::bigmap::Icon;
        let exit = |x: f32, y: f32, label: Option<&str>| Icon { kind: "exit", x, y, label: label.map(String::from), sprite: None };
        let mut named = Vec::new();
        // Seen from (100, 50): Clearfell's exit 30 cells right and 40 up; an unnamed exit and a checkpoint.
        let icons = [exit(30.0, -40.0, Some("Clearfell")), exit(-80.0, 5.0, None), Icon { kind: "checkpoint", x: 1.0, y: 1.0, label: None, sprite: None }];
        note_named_exits(&mut named, &icons, (100, 50));
        assert_eq!(named, vec![(130.0, 10.0, "Clearfell".to_string())]);
        let target = |label: &str, exit: Option<&str>| guide::NavTarget { label: label.into(), exit: exit.map(String::from), tag: None, optional: false };
        // From elsewhere on the same map the needle points at the remembered place, sure.
        let view = named_exit_guide(&target("Clearfell", Some("G1_2")), &named, (130, 110)).unwrap();
        assert_eq!((view.source.as_str(), view.target.as_str()), ("estimated", "Clearfell"));
        assert!((view.degrees.unwrap() + 90.0).abs() < 0.01, "straight up: {:?}", view.degrees);
        assert_eq!(view.distance, 100.0);
        // Another exit, or a target that is no exit: nothing to go by.
        assert!(named_exit_guide(&target("The Grelwood", Some("G1_4")), &named, (0, 0)).is_none());
        assert!(named_exit_guide(&target("Clearfell", None), &named, (0, 0)).is_none());
        // Seen again: the newer place replaces the older.
        note_named_exits(&mut named, &[exit(0.0, 0.0, Some("Clearfell"))], (128, 12));
        assert_eq!(named, vec![(128.0, 12.0, "Clearfell".to_string())]);
        // No name read: one exit icon seen is a likely direction, two are none.
        let mut places = Vec::new();
        note_exit_places(&mut places, &[exit(50.0, 0.0, None)], (0, 0));
        note_exit_places(&mut places, &[exit(40.0, 2.0, None)], (8, 0));
        assert_eq!(places, vec![(48.0, 2.0)], "the same exit seen again");
        let view = only_exit_guide(&target("Clearfell", Some("G1_2")), &places, (48, 102)).unwrap();
        assert_eq!(view.source, "rough");
        assert!((view.degrees.unwrap() + 90.0).abs() < 0.01);
        assert!(only_exit_guide(&target("Clearfell", None), &places, (0, 0)).is_none());
        note_exit_places(&mut places, &[exit(-200.0, 0.0, None)], (0, 0));
        assert!(only_exit_guide(&target("Clearfell", Some("G1_2")), &places, (0, 0)).is_none());
        // The exit come in by counts as a named icon beside another named exit.
        let came = CameIn { from: "G1_town".into(), at: (0, 0), delay: 2.0, waypoint: false, exits: vec![None] };
        let mud = [exit(54.0, -88.0, Some("Mud Burrow"))];
        let arrival = arrival_icon(Some(&came), &mud, (400, -610)).unwrap();
        assert_eq!((arrival.kind, arrival.x, arrival.y, arrival.label.as_deref()), ("exit", -400.0, 610.0, Some("G1_town")));
        // Its name read at the arrival is used; not alone, not beside an unread exit, not
        // after a late first frame, not by a waypoint, not without an exit icon there.
        let read = CameIn { exits: vec![Some("Clearfell Encampment".into())], ..CameIn { from: "G1_town".into(), at: (0, 0), delay: 2.0, waypoint: false, exits: vec![] } };
        assert_eq!(arrival_icon(Some(&read), &mud, (400, -610)).unwrap().label.as_deref(), Some("Clearfell Encampment"));
        assert!(arrival_icon(Some(&came), &[], (400, -610)).is_none());
        assert!(arrival_icon(Some(&came), &[exit(54.0, -88.0, None)], (400, -610)).is_none());
        assert!(arrival_icon(Some(&came), &[exit(10.0, -30.0, Some("Mud Burrow"))], (20, 0)).is_none(), "the other exit is by the arrival");
        for bad in [
            CameIn { delay: 30.0, from: "G1_town".into(), at: (0, 0), waypoint: false, exits: vec![None] },
            CameIn { waypoint: true, from: "G1_town".into(), at: (0, 0), delay: 2.0, exits: vec![None] },
            CameIn { exits: vec![], from: "G1_town".into(), at: (0, 0), delay: 2.0, waypoint: false },
            CameIn { from: String::new(), at: (0, 0), delay: 2.0, waypoint: false, exits: vec![None] },
        ] {
            assert!(arrival_icon(Some(&bad), &mud, (400, -610)).is_none());
        }
        // Exits named earlier come back as icons, unless the frame shows them.
        let named = vec![(130.0, 10.0, "Clearfell".to_string()), (-50.0, 40.0, "Mud Burrow".to_string())];
        let again = remembered_exits(&named, &[exit(30.0, -40.0, None)], (100, 50));
        assert_eq!(again.len(), 1, "the Clearfell exit is in view, unread");
        assert_eq!((again[0].x, again[0].y, again[0].label.as_deref()), (-150.0, -10.0, Some("Mud Burrow")));
        // A step that leads to the waypoint: its icon, once seen.
        let waypoint = guide::NavTarget { label: "Waypoint".into(), exit: None, tag: Some(WAYPOINT_TARGET.into()), optional: false };
        assert!(is_icon_target(&waypoint) && !is_icon_target(&target("Clearfell", Some("G1_2"))));
        assert!(icon_guide(&waypoint, None, (0, 0)).is_none());
        let view = icon_guide(&waypoint, Some((30.0, 40.0)), (0, 0)).unwrap();
        assert_eq!((view.source.as_str(), view.target.as_str(), view.distance), ("estimated", "Waypoint", 50.0));
    }

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

    /// A big-map line shows in every frame; lit twigs flicker: a few cells of a patch in
    /// each frame, never the same ones twice running.
    #[test]
    fn flickering_scene_pixels_are_dropped_and_lines_kept() {
        let mut live = Live::new();
        let line: Vec<(i32, i32)> = (-60..60).map(|x| (x, 20)).collect();
        let mut seed = 7u64;
        let mut rand = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as i32
        };
        // The player walks 4 cells a frame; the line stays on its map cells, the twigs (a
        // 30x30 patch in the world) show a few random cells of it in each frame.
        for step in 0..60 {
            let pos = (step * 4, 0);
            let mut rel: Vec<(i32, i32)> = line.iter().map(|&(x, y)| (x - pos.0, y)).filter(|&(x, _)| x.abs() < 70).collect();
            for _ in 0..40 {
                let (wx, wy) = (100 + rand().rem_euclid(30), -40 + rand().rem_euclid(30));
                if (wx - pos.0).abs() < 70 {
                    rel.push((wx - pos.0, wy));
                }
            }
            live.add_walls_in(&rel, pos, true, Some([pos.0 - 70, -65, pos.0 + 70, 65]));
        }
        let kept = |x0: i32, x1: i32, y0: i32, y1: i32| live.kept.keys().filter(|&&(x, y)| x >= x0 && x < x1 && y >= y0 && y < y1).count();
        let line_kept = kept(-60, 60, 20, 21);
        let twigs_kept = kept(100, 130, -40, -10);
        println!("line {line_kept} of 120 kept, twigs {twigs_kept} of 900 cells");
        assert!(line_kept >= 100, "{line_kept}");
        assert!(twigs_kept <= 20, "{twigs_kept}");
    }

    #[test]
    fn a_respawn_in_the_zone_starts_a_new_part() {
        let dir = scratch("respawn");
        let recorder = Recorder::new(Some(dir.clone()));
        enter(&recorder, "G1_town");
        enter(&recorder, "G1_2");
        for step in 0..40 {
            recorder.on_frame(&frame((step * 4, 0)), [300, 260]);
        }
        // Slain and generated again: the same area entered from itself.
        enter(&recorder, "G1_2");
        let state = recorder.0.lock().unwrap();
        let session = state.session.as_ref().unwrap();
        assert!(session.live.walls.is_empty(), "a new part");
        assert!(!session.live.unplaced);
        assert_eq!(session.recording.parts.len(), 1, "the old map waits to be joined");
        drop(state);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_guessed_boss_pair_stands_for_no_boss() {
        use crate::minimap::bigmap::Icon;
        let beira = [Boss { name: "Beira".into(), exit: None, tag: Some("boss".into()) }];
        let crone = |kind: &'static str, sprite: &'static str| Icon { kind, x: 0.0, y: 0.0, label: None, sprite: Some(sprite) };
        // Clearfell's "Crone" pair: no boss named, tied by guess, the place "Frostblood Ritual".
        assert!(boss_candidates(&crone("boss", "CroneActive"), "G1_2", &beira).is_empty());
        assert!(boss_candidates(&crone("boss-dead", "CroneInactive"), "G1_2", &beira).is_empty());
        // A pair the data ties to its boss still counts: the Miller's own.
        let miller = [Boss { name: "The Bloated Miller".into(), exit: Some("G1_town".into()), tag: None }];
        assert_eq!(boss_candidates(&crone("boss", "MillerActive"), "G1_1", &miller).len(), 1);
    }

    #[test]
    fn walls_seen_again_a_cell_off_add_nothing() {
        let rel: Vec<(i32, i32)> = (0..40).map(|x| (x, 5)).collect();
        let mut live = Live::new();
        live.add_walls(&rel, (0, 0), false);
        assert_eq!(live.walls.len(), 40);
        // The same line a cell lower and a cell to the side: it is the line seen again.
        live.add_walls(&rel, (1, 1), false);
        assert_eq!(live.walls.len(), 40, "even the end, next to the old end, is the old line");
        assert!(live.walls[&(10, 5)] >= 2);
        // Seen many times: the count stops growing.
        for _ in 0..100 {
            live.add_walls(&rel, (0, 0), false);
        }
        assert_eq!(live.walls[&(10, 5)], SEEN_MOST);
    }

    #[test]
    fn a_return_to_the_instance_goes_on_with_its_map() {
        let dir = scratch("return");
        let recorder = Recorder::new(Some(dir.clone()));
        enter(&recorder, "G1_town");
        enter(&recorder, "G1_2");
        for step in 0..40 {
            if step == 10 {
                let grelwood = crate::minimap::bigmap::Icon { kind: "exit", x: -100.0, y: 40.0, label: Some("The Grelwood".into()), sprite: None };
                recorder.on_icons(&[grelwood], true);
            }
            recorder.on_frame(&frame((step * 4, 0)), [300, 260]);
        }
        let first = recorder.0.lock().unwrap().session.as_ref().unwrap().live.walls.len();
        // To town and back by the same exit: the player appears where they came in before.
        enter(&recorder, "G1_town");
        recorder.on_frame(&frame((0, 0)), [300, 260]);
        enter(&recorder, "G1_2");
        for step in 0..40 {
            recorder.on_frame(&frame((step * 4, 0)), [300, 260]);
        }
        let state = recorder.0.lock().unwrap();
        let session = state.session.as_ref().unwrap();
        assert!(session.recording.parts.is_empty(), "no second part: {}", session.recording.parts.len());
        assert!(!session.live.unplaced);
        assert_eq!(session.live.pos, (156, 0), "the first visit's coordinates");
        assert!(session.live.walls.len() <= first + 2, "the same corridor drawn again: {} -> {}", first, session.live.walls.len());
        assert_eq!(session.named_exits.iter().map(|e| e.2.as_str()).collect::<Vec<_>>(), ["The Grelwood"], "named before, known again");
        drop(state);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_saved_map_not_found_again_leaves_its_named_exits_behind() {
        let dir = scratch("giveup");
        let recorder = Recorder::new(Some(dir.clone()));
        enter(&recorder, "G1_town");
        enter(&recorder, "G1_2");
        for step in 0..40 {
            if step == 10 {
                let grelwood = crate::minimap::bigmap::Icon { kind: "exit", x: -100.0, y: 40.0, label: Some("The Grelwood".into()), sprite: None };
                recorder.on_icons(&[grelwood], true);
            }
            recorder.on_frame(&frame((step * 4, 0)), [300, 260]);
        }
        recorder.on_icons(&[], true);
        // Back from town to a place the saved map does not have: a ring of walls.
        enter(&recorder, "G1_town");
        enter(&recorder, "G1_2");
        assert_eq!(recorder.0.lock().unwrap().session.as_ref().unwrap().named_exits.len(), 1, "known while it is looked for");
        let ring: Vec<[u16; 2]> = (0..360)
            .step_by(3)
            .map(|d| {
                let a = (d as f32).to_radians();
                [(75.0 + 40.0 * a.cos()) as u16, (65.0 + 25.0 * a.sin()) as u16]
            })
            .collect();
        let odd = MinimapView { walls: ring, ..frame((0, 0)) };
        for _ in 0..FIND_FRAMES + 2 {
            recorder.on_frame(&odd, [300, 260]);
        }
        let state = recorder.0.lock().unwrap();
        let session = state.session.as_ref().unwrap();
        assert!(!session.live.unplaced, "a new part");
        assert!(session.named_exits.is_empty(), "the old part's exits stay with it: {:?}", session.named_exits);
        assert!(!session.live.part.pois.iter().any(|p| p.kind == "exit"));
        assert!(session.recording.parts.iter().any(|p| p.pois.iter().any(|p| p.kind == "exit")), "kept in the saved part");
        drop(state);
        let _ = std::fs::remove_dir_all(dir);
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
        // From the encampment, a neighbour (from a zone that is none the arrival is the start).
        enter(&recorder, "G1_town");
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
        assert_eq!(summary, [("arrived", "G1_town", 0), ("npc", "Wounded Man", 8)]);
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
        // This visit has walked nothing yet; the total is kept.
        assert_eq!(nav.progress().0, 0.0);
        assert_eq!(nav.memory().walked, memory.walked);
        // The walk was weighed: the layouts are no longer equally likely.
        let (lo, hi) = memory.layouts.values().fold((1.0f64, 0.0f64), |(lo, hi), &p| (lo.min(p), hi.max(p)));
        assert!(hi > 1.5 * lo, "{lo} {hi}");
        let top = memory.layouts.iter().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert!(nav.ranking().iter().take(3).any(|(id, _)| id == top), "{:?}", &nav.ranking()[..3]);
        drop(state);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_log_catch_up_leaves_the_navigator_memory_untouched() {
        let dir = scratch("catchup");
        let file = dir.join("G1_2").join("7.nav.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let memory = nav::Memory { layouts: [("ClearFell_W_E_N".to_string(), 0.7)].into(), walked: 900.0, at: 12345, ..Default::default() };
        let bytes = serde_json::to_vec(&memory).unwrap();
        std::fs::write(&file, &bytes).unwrap();
        // History from the log at start: zone entries, no frames.
        let recorder = Recorder::new(Some(dir.clone()));
        for area in ["G1_2", "G1_town", "G1_2", "G1_3"] {
            recorder.on_log(&LogEvent::ZoneEntered { area: area.into(), level: 1, seed: Some(7) }, false);
        }
        recorder.save_now();
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exit_icons_reach_the_navigator_and_far_ones_last_between_scans() {
        use crate::minimap::bigmap::Icon;
        let recorder = Recorder::new(None);
        enter(&recorder, "G1_town");
        enter(&recorder, "G1_2");
        recorder.set_targets(vec![("G1_2".into(), guide::NavTarget { label: "The Grelwood".into(), exit: Some("G1_4".into()), ..Default::default() })]);
        recorder.on_frame(&frame((0, 0)), [300, 260]);
        // A whole-window scan: The Grelwood's exit, labelled, outside the wall frame.
        let grelwood = Icon { kind: "exit", x: -136.0, y: 83.0, label: Some("The Grelwood".into()), sprite: None };
        recorder.on_icons(&[grelwood], true);
        recorder.on_frame(&frame((4, 0)), [300, 260]);
        // The navigator's needle: whether the icon reached it.
        let needle = |r: &Recorder| {
            let state = r.0.lock().unwrap();
            assert_eq!(state.view().unwrap().guide.unwrap().target, "The Grelwood");
            let session = state.session.as_ref().unwrap();
            let target = guide::NavTarget { label: "The Grelwood".into(), exit: Some("G1_4".into()), ..Default::default() };
            session.navigator.as_ref().unwrap().guide(&[target]).unwrap()
        };
        let view = needle(&recorder);
        assert!(guide::angle_between(view.degrees.unwrap(), guide::screen_bearing(-136.0, 83.0).unwrap()) < 15.0, "{view:?}");
        // The next frame lists only icons inside the frame: the far one is kept, moved with the player.
        recorder.on_icons(&[], false);
        recorder.on_frame(&frame((8, 0)), [300, 260]);
        let later = needle(&recorder);
        assert!(guide::angle_between(later.degrees.unwrap(), guide::screen_bearing(-140.0, 83.0).unwrap()) < 15.0, "{later:?}");
        // The far icon was still handed over: the navigator still sees it.
        let state = recorder.0.lock().unwrap();
        assert_eq!(state.session.as_ref().unwrap().far_icons.len(), 1);
        assert_eq!(state.session.as_ref().unwrap().far_age, 1);
        drop(state);
        // A frame with an icon in the capture's margin is no scan: the kept far icon stays.
        let margin = Icon { kind: "exit", x: 90.0, y: 0.0, label: None, sprite: None };
        recorder.on_icons(&[margin], false);
        recorder.on_frame(&frame((8, 0)), [300, 260]);
        assert_eq!(recorder.0.lock().unwrap().session.as_ref().unwrap().far_icons.len(), 1);
        // A scan that sees no far exit clears them (the Grelwood's exit, read earlier, is
        // handed over every frame as a remembered exit instead, not as a far icon).
        recorder.on_icons(&[], true);
        recorder.on_frame(&frame((8, 0)), [300, 260]);
        assert!(recorder.0.lock().unwrap().session.as_ref().unwrap().far_icons.is_empty());
        recorder.on_icons(&[], false);
        recorder.on_frame(&frame((12, 0)), [300, 260]);
        assert!(recorder.0.lock().unwrap().session.as_ref().unwrap().far_icons.is_empty());
        let named = recorder.0.lock().unwrap().session.as_ref().unwrap().named_exits.clone();
        assert_eq!(named.len(), 1);
    }

    /// The owner's Clearfell of 2026-11-12 night (seed 2693856399, layout ClearFell_W_E_N):
    /// the Grelwood's exit 959 cells left of and 28 above the Mud Burrow's. Read one at a
    /// time, minutes apart, they still pick the layout.
    #[test]
    fn two_exits_read_at_different_times_pick_the_layout() {
        use crate::minimap::bigmap::Icon;
        let recorder = Recorder::new(None);
        recorder.on_log(&LogEvent::ZoneEntered { area: "G1_2".into(), level: 2, seed: Some(2693856399) }, false);
        recorder.on_frame(&frame((0, 0)), [300, 260]);
        let grelwood = Icon { kind: "exit", x: -136.0, y: 83.0, label: Some("The Grelwood".into()), sprite: None };
        recorder.on_icons(&[grelwood], true);
        recorder.on_frame(&frame((0, 0)), [300, 260]);
        // Walk on with no exit in view, then the Mud Burrow's exit is read.
        for step in 1..=10 {
            recorder.on_icons(&[], step % 8 == 0);
            recorder.on_frame(&frame((step * 4, 0)), [300, 260]);
        }
        let mud = Icon { kind: "exit", x: 823.0 - 40.0, y: 111.0, label: Some("Mud Burrow".into()), sprite: None };
        for _ in 0..3 {
            recorder.on_icons(&[mud.clone()], true);
            recorder.on_frame(&frame((40, 0)), [300, 260]);
        }
        let state = recorder.0.lock().unwrap();
        let picture = state.session.as_ref().unwrap().navigator.as_ref().unwrap().picture().unwrap();
        assert!(picture.known, "{picture:?}");
        assert_eq!(picture.id, "ClearFell_W_E_N");
    }

    #[test]
    fn a_needle_without_frames_for_two_seconds_is_not_gold() {
        let gold = guide::GuideView {
            variant: String::new(), target_kind: "entrance".into(), target: "X".into(), degrees: Some(10.0),
            source: "estimated".into(), distance: 5.0, confidence: 0.9,
        };
        assert_eq!(not_stale(gold.clone(), Some(Instant::now())).source, "estimated");
        assert_eq!(not_stale(gold.clone(), Instant::now().checked_sub(Duration::from_secs(3))).source, "unconfirmed");
        assert_eq!(not_stale(gold.clone(), None).source, "unconfirmed");
        let rough = guide::GuideView { source: "rough".into(), ..gold };
        assert_eq!(not_stale(rough, None).source, "rough");
    }

    #[test]
    fn a_pause_within_a_part_leaves_the_belief_as_it_was() {
        let recorder = Recorder::new(None);
        enter(&recorder, "G1_town");
        enter(&recorder, "G1_2");
        for step in 0..30 {
            recorder.on_frame(&frame((step * 4, 0)), [300, 260]);
        }
        let needle = |r: &Recorder| {
            let state = r.0.lock().unwrap();
            let nav = state.session.as_ref().unwrap().navigator.as_ref().unwrap();
            let view = nav.guide(&[guide::NavTarget { label: "Mud Burrow".into(), exit: Some("G1_3".into()), ..Default::default() }]).unwrap();
            (view.degrees, view.confidence)
        };
        let before = needle(&recorder);
        // Standing still for 10 s (inventory open), then the same place again.
        {
            let mut state = recorder.0.lock().unwrap();
            let session = state.session.as_mut().unwrap();
            session.fed_at = Instant::now().checked_sub(Duration::from_secs(10));
        }
        recorder.on_frame(&frame((29 * 4, 0)), [300, 260]);
        assert_eq!(needle(&recorder), before);
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
        // Ticked by hand half a second after an NPC line: not an anchor.
        recorder.on_log(&LogEvent::Dialogue { speaker: "Beira".into(), text: "…".into() }, true);
        recorder.set_log_completed(vec![]);
        recorder.set_completed(vec![("G1_2".into(), "Beira".into())]);
        assert!(!anchored(&recorder));
        recorder.set_completed(vec![]);
        // Completed by the log (the tracker says so): the player stands at a boss node.
        recorder.set_targets(vec![]);
        recorder.set_log_completed(vec![("G1_2".into(), "Beira".into())]);
        recorder.set_completed(vec![("G1_2".into(), "Beira".into())]);
        assert!(anchored(&recorder));
    }

    /// A visit with a whole-window scan every third frame (a far, named exit and a
    /// checkpoint), only the frame's own icons between, the needle asked after every
    /// frame, leaving through the exit: the flight recording replays to the same needles.
    fn obs_visit(recorder: &Recorder, frames: i32) {
        use crate::minimap::bigmap::Icon;
        enter(recorder, "G1_town");
        enter(recorder, "G1_2");
        recorder.set_targets(vec![("G1_2".into(), guide::NavTarget { label: "The Grelwood".into(), exit: Some("G1_4".into()), ..Default::default() })]);
        for step in 0..frames {
            let whole = step % 3 == 0;
            let x = step * 4;
            let icons = if whole {
                vec![
                    Icon { kind: "exit", x: -136.0 - x as f32, y: 83.0, label: Some("The Grelwood".into()), sprite: None },
                    Icon { kind: "checkpoint", x: 20.0 - x as f32, y: 10.0, label: None, sprite: None },
                ]
            } else {
                vec![]
            };
            recorder.on_icons(&icons, whole);
            recorder.on_frame(&frame((x, 0)), [300, 260]);
            recorder.view();
        }
    }

    /// Frames walking on, the needle to Mud Burrow (no icon of it: the layouts' needle).
    fn to_mud_burrow(recorder: &Recorder, steps: std::ops::Range<i32>) {
        recorder.set_targets(vec![("G1_2".into(), guide::NavTarget { label: "Mud Burrow".into(), exit: Some("G1_3".into()), ..Default::default() })]);
        for step in steps {
            recorder.on_icons(&[], false);
            recorder.on_frame(&frame((step * 4, 0)), [300, 260]);
            recorder.view();
        }
    }

    #[test]
    fn a_boss_of_the_zone_speaking_is_logged_once_as_near_its_place() {
        let dir = scratch("boss");
        let recorder = Recorder::new(Some(dir.clone()));
        recorder.set_bosses(vec![
            ("G1_2".into(), Boss { name: "Beira".into(), exit: None, tag: None }),
            ("G1_1".into(), Boss { name: "The Bloated Miller".into(), exit: Some("G1_town".into()), tag: None }),
        ]);
        obs_visit(&recorder, 10);
        let line = |speaker: &str| LogEvent::Dialogue { speaker: speaker.into(), text: "…".into() };
        // The log names her in full; twice in a row counts once; another zone's boss
        // and an ordinary speaker count for nothing.
        recorder.on_log(&line("Beira of the Rotten Pack"), true);
        recorder.on_log(&line("Beira of the Rotten Pack"), true);
        recorder.on_log(&line("The Bloated Miller"), true);
        recorder.on_log(&line("Renly"), true);
        // History replayed at start is no boss met now.
        recorder.on_log(&line("Beira of the Rotten Pack"), false);
        recorder.save_now();
        let text = std::fs::read_to_string(dir.join("G1_2").join("7.obs.jsonl")).unwrap();
        let near: Vec<&str> = text.lines().filter(|l| l.contains("\"near\":true")).collect();
        assert_eq!(near.len(), 1, "{text}");
        assert!(near[0].contains("\"tag\":\"boss\""), "{}", near[0]);
        // The replay hands it to the navigator the same way: its needles still agree.
        let visits = obs::replay::replay(&text, &graph::Graphs::default());
        assert!(visits[0].agree == visits[0].logged, "{:?}", visits[0].differ);
        assert!(Boss { name: "Zed".into(), exit: None, tag: None }.speaks_as("Zeda") == false, "short names never match");
    }

    fn boss(name: &str, exit: Option<&str>, tag: Option<&str>) -> Boss {
        Boss { name: name.into(), exit: exit.map(String::from), tag: tag.map(String::from) }
    }

    fn icon(kind: &'static str, sprite: Option<&'static str>, x: f32, y: f32) -> crate::minimap::bigmap::Icon {
        let sprite = sprite.map(|id| crate::minimap::bigmap::sprites().iter().find(|s| s.id == id).unwrap().id.as_str());
        crate::minimap::bigmap::Icon { kind, x, y, label: None, sprite }
    }

    #[test]
    fn a_boss_icon_counts_only_for_the_bosses_it_may_be() {
        let grelwood = [boss("Gerung", None, Some("bossman")), boss("Areagne", None, Some("witch_hut"))];
        let manor = [boss("Candlemass", None, None), boss("Count Geonor", None, None)];
        let miller = [boss("The Bloated Miller", Some("G1_town"), None)];
        let names = |icon: &crate::minimap::bigmap::Icon, area: &str, bosses: &[Boss]| -> Vec<String> {
            boss_candidates(icon, area, bosses).into_iter().map(|b| b.name.clone()).collect()
        };
        // An orange skull (an ordinary unique walks about; no campaign boss was seen as one)
        // and a grey skull (a dead rare too) are no boss.
        assert!(names(&icon("boss", Some("UniqueMonsterAlive"), 0.0, 0.0), "G1_4", &grelwood).is_empty());
        assert!(names(&icon("boss", Some("UniqueMonsterAlive"), 0.0, 0.0), "G1_1", &miller).is_empty());
        assert!(names(&icon("dead", Some("UniqueMonsterDead"), 0.0, 0.0), "G1_4", &grelwood).is_empty());
        assert!(names(&icon("rare", Some("RareMonsterAlive"), 0.0, 0.0), "G1_4", &grelwood).is_empty());
        // The Grelwood's carnivorous plant is no checklist boss.
        assert!(names(&icon("boss", Some("CarnivorousPlantActive"), 0.0, 0.0), "G1_4", &grelwood).is_empty());
        // A boss's own icon: by name, in its own area only.
        assert_eq!(names(&icon("boss", Some("MillerActive"), 0.0, 0.0), "G1_1", &miller), ["The Bloated Miller"]);
        assert!(names(&icon("boss", Some("MillerActive"), 0.0, 0.0), "G1_2", &[boss("Beira", None, None)]).is_empty());
        assert_eq!(names(&icon("boss-dead", Some("GargoyleInactive"), 0.0, 0.0), "G1_15", &manor), ["Candlemass"]);
        // An unnamed pair tied to its area by guess alone (the Count's): no boss, as the
        // Crone pair in Clearfell turned out to be the league's ritual altar.
        assert!(names(&icon("boss", Some("CountActive"), 0.0, 0.0), "G1_15", &manor).is_empty());
    }

    #[test]
    fn an_arrival_from_no_neighbour_is_the_zones_start() {
        // A new character spawns in the Riverbank after the old one left Clearfell: the log's
        // last zone is no neighbour of G1_1. From the encampment it is the exit's place.
        let recorder = Recorder::new(None);
        let arrived = |recorder: &Recorder| {
            recorder.on_frame(&frame((0, 0)), [300, 260]);
            let state = recorder.0.lock().unwrap();
            let session = state.session.as_ref().unwrap();
            (session.came_from.clone(), session.live.part.pois.iter().find(|p| p.kind == "arrived").map(|p| p.label.clone()))
        };
        enter(&recorder, "G1_2");
        enter(&recorder, "G1_1");
        assert_eq!(arrived(&recorder), (String::new(), Some(String::new())));
        enter(&recorder, "G1_town");
        enter(&recorder, "G1_1");
        assert_eq!(arrived(&recorder), ("G1_town".to_string(), Some("G1_town".to_string())));
    }

    #[test]
    fn the_map_names_a_boss_dead_only_when_the_grey_icon_is_its() {
        let recorder = Recorder::new(None);
        recorder.set_bosses(vec![
            ("G1_1".into(), boss("The Bloated Miller", Some("G1_town"), None)),
            ("G1_6".into(), boss("Ervig", None, Some("boss"))),
            ("G1_15".into(), boss("Candlemass", None, None)),
            ("G1_15".into(), boss("Count Geonor", None, None)),
        ]);
        // Frames walking along the corridor with these icons (cells from the player; the
        // places stay on the map as the player walks).
        let walk = |frames: std::ops::Range<i32>, icons: &dyn Fn(i32) -> Vec<crate::minimap::bigmap::Icon>| {
            for step in frames {
                recorder.on_icons(&icons(step * 4), false);
                recorder.on_frame(&frame((step * 4, 0)), [300, 260]);
            }
        };
        let at = |x: f32, y: f32| move |i: i32| (x - i as f32, y);
        // The Riverbank: the Miller alive by his own icon, a grey skull beside it (an escort,
        // a rare): nobody is dead.
        enter(&recorder, "G1_1");
        let (live, beside) = (at(60.0, 4.0), at(66.0, 8.0));
        walk(0..3, &|x| vec![icon("boss", Some("MillerActive"), live(x).0, live(x).1), icon("dead", Some("UniqueMonsterDead"), beside(x).0, beside(x).1)]);
        assert!(recorder.dead_bosses().is_empty(), "{:?}", recorder.dead_bosses());
        // His grey twin read once (one frame) is not enough; while his live icon is still
        // read, neither.
        walk(3..4, &|x| vec![icon("boss-dead", Some("MillerInactive"), live(x).0 + 1.0, live(x).1)]);
        assert!(recorder.dead_bosses().is_empty());
        walk(4..6, &|x| vec![icon("boss-dead", Some("MillerInactive"), live(x).0 + 1.0, live(x).1), icon("boss", Some("MillerActive"), live(x).0, live(x).1)]);
        assert!(recorder.dead_bosses().is_empty());
        // Frames 4 and 5 (grey beside the live icon) did not count; frame 6, grey and alone,
        // is the second such frame after frame 3: dead.
        walk(6..7, &|x| vec![icon("boss-dead", Some("MillerInactive"), live(x).0 + 1.0, live(x).1 - 1.0)]);
        assert_eq!(recorder.dead_bosses(), ["The Bloated Miller"]);
        // The recheck's case, from fresh: one grey frame beside the live icon, then one grey
        // frame alone: nobody.
        enter(&recorder, "G1_1");
        walk(0..3, &|x| vec![icon("boss", Some("MillerActive"), live(x).0, live(x).1)]);
        walk(3..4, &|x| vec![icon("boss-dead", Some("MillerInactive"), live(x).0, live(x).1), icon("boss", Some("MillerActive"), live(x).0 + 2.0, live(x).1)]);
        walk(4..5, &|x| vec![icon("boss-dead", Some("MillerInactive"), live(x).0, live(x).1)]);
        assert!(recorder.dead_bosses().is_empty());
        // A second grey frame alone: dead.
        walk(5..6, &|x| vec![icon("boss-dead", Some("MillerInactive"), live(x).0, live(x).1)]);
        assert_eq!(recorder.dead_bosses(), ["The Bloated Miller"]);
        // The Grim Tangle (one boss): a grey twin never seen alive names nobody.
        enter(&recorder, "G1_6");
        assert!(recorder.dead_bosses().is_empty());
        walk(0..4, &|x| vec![icon("boss-dead", Some("GrimTangleBossInactive"), 50.0 - x as f32, 0.0)]);
        assert!(recorder.dead_bosses().is_empty());
        // A grey skull where the boss's node is, no boss icon: nobody.
        walk(4..8, &|x| vec![icon("dead", Some("UniqueMonsterDead"), 20.0 - x as f32, 3.0)]);
        assert!(recorder.dead_bosses().is_empty());
        // Ogham Manor has two: the Count's pair names neither, so alive then grey, nobody;
        // the Gargoyle's names Candlemass.
        enter(&recorder, "G1_15");
        walk(0..3, &|x| vec![icon("boss", Some("CountActive"), 50.0 - x as f32, 0.0), icon("boss", Some("GargoyleActive"), -40.0 - x as f32, 10.0)]);
        walk(3..6, &|x| vec![icon("boss-dead", Some("CountInactive"), 50.0 - x as f32, 0.0), icon("boss-dead", Some("GargoyleInactive"), -40.0 - x as f32, 10.0)]);
        assert_eq!(recorder.dead_bosses(), ["Candlemass"]);
        enter(&recorder, "G1_town");
        assert!(recorder.dead_bosses().is_empty());
    }

    /// The Riverbank's real captures through the reader and the recorder: the Miller alive
    /// (his first line), then 4 minutes after his last line his grey twin at his place.
    /// The reader looks there because the recorder hints it: `LIVE_DIR=… cargo test
    /// the_real_miller -- --ignored --nocapture` (the captures are not fixtures: 4 MB each).
    #[test]
    #[ignore]
    fn the_real_miller_alive_then_dead() {
        use crate::minimap::bigmap;
        let dir = std::env::var("LIVE_DIR").unwrap();
        let recorder = Recorder::new(None);
        recorder.set_bosses(vec![("G1_1".into(), boss("The Bloated Miller", Some("G1_town"), None))]);
        enter(&recorder, "G1_1");
        let plan = bigmap::Plan { marker: [643.0, 561.0], radius: 16.0, frame: bigmap::Frame::wide(600, 518, [600, 518]) };
        for name in ["boss-bigmap-1794482345", "bigmap-1794482737"] {
            let image = crate::minimap::parse_ppm(&std::fs::read(format!("{dir}/{name}.ppm")).unwrap()).unwrap();
            // Three frames of each (the reader reads three a second): the first of the second
            // capture still hints from the old place, a name needs two frames.
            for _ in 0..3 {
                let (view, icons) = bigmap::read_with(&image, &plan, &[], false);
                println!("{name}: {:?}", icons.iter().map(|i| (i.kind, i.sprite)).collect::<Vec<_>>());
                recorder.on_icons(&icons, false);
                recorder.on_frame(&view, [600, 518]);
                let state = recorder.0.lock().unwrap();
                println!("  placed at {:?}", state.session.as_ref().unwrap().live.pos);
            }
        }
        assert_eq!(recorder.dead_bosses(), ["The Bloated Miller"]);
        // With FIXTURES set, the two wall frames are written for the fixture test below.
        if let Ok(out) = std::env::var("FIXTURES") {
            for name in ["boss-bigmap-1794482345", "bigmap-1794482737"] {
                let image = crate::minimap::parse_ppm(&std::fs::read(format!("{dir}/{name}.ppm")).unwrap()).unwrap();
                let (view, _) = bigmap::read_with(&image, &plan, &[], false);
                std::fs::write(format!("{out}/riverbank-frame-{name}.json"), serde_json::to_string(&view.walls).unwrap()).unwrap();
            }
        }
    }

    #[test]
    fn the_real_miller_alive_then_dead_through_the_recorder() {
        // The same with fixtures: each capture's wall frame (`riverbank-frame-*.json`) and the
        // crop around the cross its icons are read from (`bigmap-boss-miller.ppm`, alive, the
        // crop at 440, 480; `bigmap-miller-dead.ppm`, 4 minutes after his last line, at 440, 470).
        use crate::minimap::bigmap;
        let recorder = Recorder::new(None);
        recorder.set_bosses(vec![("G1_1".into(), boss("The Bloated Miller", Some("G1_town"), None))]);
        enter(&recorder, "G1_1");
        let fixtures = format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"));
        let mut kinds = Vec::new();
        for (walls, crop) in [("boss-bigmap-1794482345", "bigmap-boss-miller.ppm"), ("bigmap-1794482737", "bigmap-miller-dead.ppm")] {
            let cells: Vec<[u16; 2]> = serde_json::from_slice(&std::fs::read(format!("{fixtures}/riverbank-frame-{walls}.json")).unwrap()).unwrap();
            let image = crate::minimap::parse_ppm(&std::fs::read(format!("{fixtures}/{crop}")).unwrap()).unwrap();
            let cross = bigmap::marker_near(&image, [203.0, 86.0], 20.0).unwrap();
            let view = MinimapView { enabled: true, visible: true, message: "Reading big map".into(), cols: 600, rows: 518, walls: cells, player: Some([300.0, 259.0]) };
            for _ in 0..3 {
                let icons = bigmap::icons_in(&image, cross, bigmap::SCALE, 1.0, 2, &[], false);
                kinds.push(icons.iter().filter(|i| i.kind.starts_with("boss")).map(|i| i.sprite.unwrap_or("")).collect::<Vec<_>>());
                recorder.on_icons(&icons, false);
                recorder.on_frame(&view, [600, 518]);
            }
        }
        // Alive in all three frames; grey from the second frame after the move (the hint
        // follows the player's new place); dead once read grey twice.
        assert_eq!(kinds, [vec!["MillerActive"], vec!["MillerActive"], vec!["MillerActive"], vec![], vec!["MillerInactive"], vec!["MillerInactive"]]);
        assert_eq!(recorder.dead_bosses(), ["The Bloated Miller"]);
        crate::minimap::bigmap::set_icon_hints(vec![]);
        crate::minimap::bigmap::set_icon_area(None);
    }

    #[test]
    fn boss_icons_reach_neither_the_navigator_nor_the_flight_recorder() {
        use crate::minimap::bigmap::Icon;
        let dir = scratch("boss-icons");
        let recorder = Recorder::new(Some(dir.clone()));
        recorder.set_bosses(vec![("G1_2".into(), boss("Beira", None, None))]);
        enter(&recorder, "G1_town");
        enter(&recorder, "G1_2");
        recorder.set_targets(vec![("G1_2".into(), guide::NavTarget { label: "The Grelwood".into(), exit: Some("G1_4".into()), ..Default::default() })]);
        for step in 0..30 {
            let whole = step % 3 == 0;
            let x = step * 4;
            let mut icons = vec![Icon { kind: "exit", x: -136.0 - x as f32, y: 83.0, label: whole.then(|| "The Grelwood".into()), sprite: None }];
            // Beira's icon (the Crone's, Clearfell's pair) far east on the scans, and a grey skull.
            if whole {
                icons.push(icon("boss", Some("CroneActive"), 400.0 - x as f32, -60.0));
                icons.push(icon("dead", Some("RareMonsterDead"), 30.0 - x as f32, 40.0));
            }
            recorder.on_icons(&icons, whole);
            recorder.on_frame(&frame((x, 0)), [300, 260]);
            recorder.view();
        }
        recorder.save_now();
        let text = std::fs::read_to_string(dir.join("G1_2").join("7.obs.jsonl")).unwrap();
        assert!(!text.contains("bosses") && !text.contains(r#""kind":"boss""#), "{text}");
        // The grey skull is written once, as a mark.
        assert_eq!(text.matches(r#""kind":"dead""#).count(), 1, "{text}");
        let visits = obs::replay::replay(&text, &graph::Graphs::default());
        assert!(visits[0].agree == visits[0].logged && visits[0].logged > 0, "{:?}", visits[0].differ);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A walk built from a real big-map frame (`riverbank-walls-*.json`: its wall cells,
    /// map lines and scene by colour): the lines stay on their map cells, the scene scrolls
    /// 5 times as fast, what lies on the character (within 30 cells of the player) moves
    /// with the player. The player walks `steps` frames of 4 x 1 cells. Returns the scene
    /// cells the recorder keeps with the hold and without it, the line cells kept, and the
    /// line cells it saw.
    fn walk_real_walls(name: &str, steps: i32) -> (usize, usize, usize, usize) {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let cells = |key: &str| -> Vec<(i32, i32)> {
            json[key].as_array().unwrap().iter().map(|c| (c[0].as_i64().unwrap() as i32 - 300, c[1].as_i64().unwrap() as i32 - 259)).collect()
        };
        let (line, scene) = (cells("line"), cells("scene"));
        let on_player = |c: &(i32, i32)| c.0.abs() <= 30 && c.1.abs() <= 30;
        let recorder = Recorder::new(None);
        enter(&recorder, "G1_1");
        let mut seen = std::collections::HashSet::new();
        for i in 0..steps {
            let pos = (4 * i, i);
            let rel: Vec<(i32, i32)> = line
                .iter()
                .map(|&(x, y)| (x - pos.0, y - pos.1))
                .chain(scene.iter().map(|&(x, y)| if on_player(&(x, y)) { (x, y) } else { (x - 5 * pos.0, y - 5 * pos.1) }))
                .filter(|&(x, y)| x.abs() < 300 && y.abs() < 259)
                .collect();
            seen.extend(line.iter().map(|&(x, y)| (x, y)).filter(|&(x, y)| (x - pos.0).abs() < 300 && (y - pos.1).abs() < 259));
            let view = MinimapView {
                enabled: true,
                visible: true,
                message: "Reading big map".into(),
                cols: 600,
                rows: 518,
                walls: rel.iter().map(|&(x, y)| [(x + 300) as u16, (y + 259) as u16]).collect(),
                player: Some([300.0, 259.0]),
            };
            recorder.on_icons(&[], false);
            recorder.on_frame(&view, [600, 518]);
        }
        let state = recorder.0.lock().unwrap();
        let live = &state.session.as_ref().unwrap().live;
        assert_eq!(live.pos, (4 * (steps - 1), steps - 1), "the walk was not followed");
        let lines: std::collections::HashSet<(i32, i32)> = line.iter().copied().collect();
        let kept: Vec<(i32, i32)> = live.kept().map(|(&c, _)| c).collect();
        let scene_kept = kept.iter().filter(|c| !lines.contains(c)).count();
        let scene_all = live.walls.keys().filter(|c| !lines.contains(c)).count();
        let line_kept = kept.iter().filter(|c| lines.contains(c)).count();
        (scene_kept, scene_all, line_kept, seen.len())
    }

    #[test]
    fn scene_pixels_on_the_big_map_are_not_recorded() {
        // I9, from the Riverbank captures: foam, wet stones, the character's clothes and a
        // tutorial panel passed the line colour. Held until seen from two places 24 cells
        // apart, none of them is recorded; lines seen from such places all are.
        for name in ["riverbank-walls-1794481095.json", "riverbank-walls-1794481536.json", "riverbank-walls-1794482137.json"] {
            let (scene_kept, scene_all, line_kept, line_seen) = walk_real_walls(name, 20);
            println!("{name}: scene cells recorded {scene_all} without the hold, {scene_kept} with; line cells {line_kept} of {line_seen} seen");
            assert!(scene_kept * 50 <= scene_all.max(1), "{name}: {scene_kept} of {scene_all}");
            assert!(line_kept as f64 >= 0.9 * line_seen as f64, "{name}: {line_kept} of {line_seen}");
        }
    }

    #[test]
    fn a_big_map_cell_is_kept_once_seen_from_two_places_a_minimap_cell_at_once() {
        let recorder = Recorder::new(None);
        enter(&recorder, "G1_1");
        let big = |pos: (i32, i32)| MinimapView { message: "Reading big map".into(), ..frame(pos) };
        let kept = || recorder.0.lock().unwrap().session.as_ref().unwrap().live.kept().count();
        // Standing still with the big map open: nothing kept yet.
        for _ in 0..5 {
            recorder.on_frame(&big((0, 0)), [300, 260]);
        }
        assert_eq!(kept(), 0);
        // Walking on: the corridor seen from 24 cells further is kept.
        for step in 1..=7 {
            recorder.on_frame(&big((step * 4, 0)), [300, 260]);
        }
        assert!(kept() > 100, "{}", kept());
        // The corner minimap's cells are kept at once.
        let recorder = Recorder::new(None);
        enter(&recorder, "G1_1");
        recorder.on_frame(&frame((0, 0)), [300, 260]);
        assert!(recorder.0.lock().unwrap().session.as_ref().unwrap().live.kept().count() > 100);
    }

    /// Exploration: the Riverbank captures' big-map frames (`LIVE_DIR`, files in time order)
    /// fed to the recorder: where each is placed, what is kept:
    /// `LIVE_DIR=… cargo test live_frames_held -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_frames_held() {
        use crate::minimap::bigmap;
        let dir = std::env::var("LIVE_DIR").unwrap();
        let recorder = Recorder::new(None);
        enter(&recorder, "G1_1");
        let plan = bigmap::Plan { marker: [643.0, 561.0], radius: 16.0, frame: bigmap::Frame::wide(600, 518, [600, 518]) };
        for name in ["bigmap-1794481095", "bigmap-1794481536", "bigmap-1794482137", "boss-bigmap-1794482345"] {
            let image = crate::minimap::parse_ppm(&std::fs::read(format!("{dir}/{name}.ppm")).unwrap()).unwrap();
            let (view, _) = bigmap::read_with(&image, &plan, &[], false);
            recorder.on_icons(&[], false);
            recorder.on_frame(&view, [600, 518]);
            let state = recorder.0.lock().unwrap();
            let live = &state.session.as_ref().unwrap().live;
            println!("{name}: {} cells read; placed at {:?}; {} cells, {} held, parts {}", view.walls.len(), live.pos, live.walls.len(), live.held.len(), state.session.as_ref().unwrap().recording.parts.len());
        }
    }

    #[test]
    fn the_flight_recorder_logs_what_the_navigator_is_fed() {
        let dir = scratch("obs");
        let recorder = Recorder::new(Some(dir.clone()));
        obs_visit(&recorder, 40);
        // Then towards an exit not seen: the layouts' needle, which the replay gives too.
        recorder.set_targets(vec![("G1_2".into(), guide::NavTarget { label: "Mud Burrow".into(), exit: Some("G1_3".into()), ..Default::default() })]);
        for step in 40..46 {
            recorder.on_icons(&[], false);
            recorder.on_frame(&frame((step * 4, 0)), [300, 260]);
            recorder.view();
        }
        enter(&recorder, "G1_4");
        let text = std::fs::read_to_string(dir.join("G1_2").join("7.obs.jsonl")).unwrap();
        for kind in ["start", "targets", "needle", "move", "icons", "seen", "end"] {
            assert!(text.contains(&format!("\"k\":\"{kind}\"")), "{kind}: {text}");
        }
        // The town has no layout graphs: no flight recording.
        assert!(!dir.join("G1_town").join("7.obs.jsonl").exists());
        let visits = obs::replay::replay(&text, &graph::Graphs::default());
        assert_eq!(visits.len(), 1);
        let v = &visits[0];
        eprint!("{}", obs::replay::report(v));
        assert_eq!(v.frames, 46);
        assert!(v.logged > 0 && v.agree == v.logged, "{} of {}: {:?}", v.agree, v.logged, v.differ);
        assert_eq!(v.left.as_ref().unwrap().0, "G1_4");
        // A later visit of the same instance appends; its navigator starts from the
        // instance's memory, which the start line holds: the replay agrees again.
        obs_visit(&recorder, 3);
        to_mud_burrow(&recorder, 3..9);
        enter(&recorder, "G1_3");
        // And the app started mid-zone (the log's history): the position is resumed.
        recorder.on_log(&LogEvent::ZoneEntered { area: "G1_2".into(), level: 1, seed: Some(7) }, false);
        recorder.set_targets(vec![("G1_2".into(), guide::NavTarget { label: "Mud Burrow".into(), exit: Some("G1_3".into()), ..Default::default() })]);
        to_mud_burrow(&recorder, 9..15);
        enter(&recorder, "G1_3");
        let text = std::fs::read_to_string(dir.join("G1_2").join("7.obs.jsonl")).unwrap();
        assert_eq!(text.matches("\"k\":\"start\"").count(), 3);
        let with = obs::replay::replay(&text, &graph::Graphs::default());
        for (v, resumed) in with[1..].iter().zip([false, true]) {
            eprint!("{}", obs::replay::report(v));
            assert!(v.memory && v.memory_replayed && v.resumed == resumed, "{v:?}");
            assert!(v.logged > 0 && v.agree == v.logged, "{} of {}: {:?}", v.agree, v.logged, v.differ);
        }
        // Without the memory in the file (as before v0.1.48) the visit is left out of the agreement.
        let old: String = text
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| {
                let mut line: obs::Line = serde_json::from_str(l).unwrap();
                if let obs::Entry::Start { remembered, age, .. } = &mut line.entry {
                    (*remembered, *age) = (None, None);
                }
                serde_json::to_string(&line).unwrap() + "\n"
            })
            .collect();
        let visits = obs::replay::replay(&old, &graph::Graphs::default());
        assert!(visits[1].memory && !visits[1].memory_replayed);
        // The memory matters: from it the learnt layout starts ahead (0.13 against 0.04).
        assert!(with[1].belief[0].2 > 2.0 * visits[1].belief[0].2, "{:?} {:?}", with[1].belief, visits[1].belief);
        let line = obs::replay::summary(1, &visits);
        assert!(line.contains("2 visit(s) from a memory not in the file left out"), "{line}");
        // One exit read by name fixes no layout: no layout history.
        assert!(!dir.join(obs::HISTORY_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The frame loop's cost with and without the flight recorder:
    /// `cargo test --release obs_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn obs_cost() {
        let dir = scratch("obs-cost");
        let mut times = Vec::new();
        for round in 0..6 {
            let on = round % 2 == 0;
            let recorder = Recorder::new(Some(dir.clone()));
            enter(&recorder, "G1_town");
            enter(&recorder, "G1_2");
            if !on {
                *recorder.0.lock().unwrap().session.as_mut().unwrap().obs.get_mut() = None;
            }
            let started = Instant::now();
            obs_visit(&recorder, 300);
            times.push((on, started.elapsed().as_secs_f64() * 1e6 / 300.0));
            let _ = std::fs::remove_dir_all(&dir);
        }
        let mean = |on: bool| {
            let t: Vec<f64> = times.iter().filter(|x| x.0 == on).map(|x| x.1).collect();
            t.iter().sum::<f64>() / t.len() as f64
        };
        eprintln!("per frame (on_icons, on_frame, view): with the flight recorder {:.0} µs, without {:.0} µs; {times:?}", mean(true), mean(false));
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
    live.kept = live.walls.clone();
    live.part = Part { walls: vec![], ..first };
    let parts = recording.parts.len();
    let mut session = Session {
        recording, live, arrived_from: None, came_from: String::new(), started: true, file: None, navigator: None,
        fed: None, fed_at: None, far_icons: Vec::new(), far_age: FAR_FRAMES, named_exits: Vec::new(), exit_places: Vec::new(), waypoint_at: None,
        last_guide: Default::default(),
        obs: Default::default(), entered: Instant::now(), came_in: None,
        bosses_heard: Vec::new(), boss_places: Vec::new(), dead: Vec::new(), named_seen: Vec::new(), frame_no: 0, checkpoint_marks: Vec::new(),
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
            println!("   no layout graphs");
            continue;
        };
        let seed = recording.seed.unwrap_or(0);
        let mut nav = nav::Navigator::new(&area, nav::Arrival::Unknown, None, graph::SCALE, seed);
        // The truth from the big map (Clearfell 2693856399 is ClearFell_W_E_N): its
        // probability after each part, and the most any layout reached.
        let truth = (own == "G1_2" && seed == 2693856399).then_some("ClearFell_W_E_N");
        let mut truth_p: Vec<f64> = Vec::new();
        let mut most: f64 = 0.0;
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
                        // As in the app: leaving is soft evidence; an arrival starts a new
                        // visit at that exit from what the instance's memory holds.
                        "left" if poi.label != own && !poi.label.is_empty() => {
                            nav.seen(nav::Seen::Left(&poi.label));
                            anchors += &format!(" left {}", poi.label);
                        }
                        "arrived" if poi.label != own && !poi.label.is_empty() => {
                            let memory = nav.memory();
                            nav = nav::Navigator::new(&area, nav::Arrival::From(&poi.label), Some(&memory), graph::SCALE, seed);
                            anchors += &format!(" arrived {}", poi.label);
                        }
                        // Into the same zone again (a checkpoint after dying, a new instance): somewhere else.
                        "arrived" | "left" => nav.lost(f64::INFINITY),
                        _ => {}
                    }
                }
            }
            let (walked, _) = nav.progress();
            println!("   part {i:2}: {:4} trail points, walked {:5.0} |{anchors} | {}", part.trail.len(), walked, top(&nav));
            let ranking = nav.ranking();
            most = most.max(ranking[0].1);
            if let Some(id) = truth {
                truth_p.push(ranking.iter().find(|(l, _)| *l == id).map_or(0.0, |r| r.1));
            }
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
        println!("   most any layout reached: {most:.2}");
        if let Some(id) = truth {
            let least = truth_p.iter().copied().fold(1.0, f64::min);
            println!("   {id} (the truth): {:.3} after part 0, least {least:.3}", truth_p[0]);
            // The walked line alone must keep the true layout alive (no lower than
            // uniform after the first part, never under 2%) and claim no certainty.
            assert!(truth_p[0] >= 1.0 / area.layouts.len() as f64 && least >= 0.02, "{id}: {truth_p:?}");
        }
        if recording.parts.len() > 1 {
            assert!(most <= 0.6, "the path alone reached {most}");
        }
    }
}
