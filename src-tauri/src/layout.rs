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
mod route;

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
/// Far icons from a whole-screen scan stand in for this many frames after it
/// (the scan comes every third big-map frame).
const FAR_FRAMES: u32 = 3;
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
/// A frame that cannot be placed near the player's last place (after a gap: frames not
/// read, a fast move) is looked for this far from it, in cells, the way pieces are joined.
const REFIND: i32 = 150;
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
    /// Wall cells and how often seen: all of them place frames (`register`).
    walls: HashMap<(i32, i32), u32>,
    /// Big-map wall cells not yet seen from two player positions `HOLD_MOVE` apart, with
    /// the first: left out of the recording, the overview and the map's ways …
    held: HashMap<(i32, i32), (i32, i32)>,
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
}

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
        }
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
    fn add_walls(&mut self, rel: &[(i32, i32)], pos: (i32, i32), big: bool) {
        for &(rx, ry) in rel {
            let cell = (pos.0 + rx, pos.1 + ry);
            let n = self.walls.entry(cell).or_insert(0);
            *n += 1;
            let n = *n;
            if !big {
                self.held.remove(&cell);
            } else if n == 1 {
                self.held.insert(cell, pos);
            } else if self.held.get(&cell).is_some_and(|first| (first.0 - pos.0).abs().max((first.1 - pos.1).abs()) >= HOLD_MOVE) {
                self.held.remove(&cell);
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
    /// Where the layout graphs put it.
    fn place(&self) -> nav::BossAt {
        match (&self.exit, &self.tag) {
            (Some(exit), _) => nav::BossAt::Exit(exit.clone()),
            (None, Some(tag)) => nav::BossAt::Tag(tag.clone()),
            (None, None) => nav::BossAt::Tag("boss".into()),
        }
    }

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
const NAMED_FRAMES: u32 = 12;
/// A grey twin this close (cells) to where this part saw the pair alive is that boss, dead.
const SAME_PLACE: f32 = 12.0;
/// Bosses' own icons are evidence for the navigator. Off (shadow: read, logged, used for
/// `dead_bosses` and the reader's hints): in the simulation, even only a pair's icon read
/// in two frames, kept off the particles and out of gold's share, made gold worse: 2 seeds,
/// more than 60° off, minimap view 0 of 14 → 2 of 6, wide view 1.7% → 4.3%
/// (`nav_sim::boss_icons_report`; the review allowed half a point).
const BOSS_EVIDENCE: bool = false;

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

/// The zone an exit icon's label names (as in the area's connectionNames), when the
/// reader could read it: a named anchor. None is "some exit".
fn icon_label(icon: &crate::minimap::bigmap::Icon) -> Option<String> {
    icon.label.clone().filter(|l| !l.is_empty())
}

/// Exits (and bosses) kept from the last whole-window scan: place in the part's cells,
/// label, the boss places of a boss's icon.
type FarIcons = Vec<(f32, f32, Option<String>, Vec<nav::BossAt>)>;

/// Which of the zone's bosses a map icon may be (see `nav::ExitIcon::boss`): only a boss
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

/// The checkpoints of a flight recorder's icons line (written once a part, as the recorder
/// hands them to the navigator: `Session::new_checkpoints`), for the replay.
#[cfg(test)]
fn checkpoint_list(icons: &[crate::minimap::bigmap::Icon]) -> Vec<(f64, f64)> {
    icons.iter().filter(|i| i.kind == "checkpoint").map(|i| (i.x as f64, i.y as f64)).collect()
}

/// The places of `boss_candidates`, each once.
fn boss_places(icon: &crate::minimap::bigmap::Icon, area: &str, bosses: &[Boss]) -> Vec<nav::BossAt> {
    let mut places: Vec<nav::BossAt> = Vec::new();
    for boss in boss_candidates(icon, area, bosses) {
        let place = boss.place();
        if !places.contains(&place) {
            places.push(place);
        }
    }
    places
}

/// The exits (and bosses' icons, with the places of the zone's `bosses` in `area` they
/// may be) handed to the navigator with one frame (`Session::feed_icons`; the replay of
/// a flight recording builds them the same way). Every third big-map frame lists every
/// icon on screen, the frames between only those inside the wall frame: icons outside it
/// are kept from the last whole scan for a few frames.
#[allow(clippy::too_many_arguments)]
fn exit_list(
    icons: &[crate::minimap::bigmap::Icon],
    whole: bool,
    pos: (i32, i32),
    size: (usize, usize),
    far_icons: &mut FarIcons,
    far_age: &mut u32,
    area: &str,
    bosses: &[Boss],
) -> Vec<nav::ExitIcon> {
    let (hw, hh) = (size.0 as f32 / 2.0, size.1 as f32 / 2.0);
    let outside = |x: f32, y: f32| x.abs() > hw || y.abs() > hh;
    let fed: Vec<(&crate::minimap::bigmap::Icon, Vec<nav::BossAt>)> = icons
        .iter()
        .filter_map(|i| match i.kind {
            "exit" => Some((i, vec![])),
            _ if BOSS_EVIDENCE => Some((i, boss_places(i, area, bosses))).filter(|(_, places)| !places.is_empty()),
            _ => None,
        })
        .collect();
    let mut list: Vec<nav::ExitIcon> =
        fed.iter().map(|(i, boss)| nav::ExitIcon { dx: i.x as f64, dy: i.y as f64, to: icon_label(i), boss: boss.clone() }).collect();
    if whole {
        // A whole-window scan (marked by the reader): its far icons replace the
        // kept ones, none if it saw none.
        *far_icons = fed
            .iter()
            .filter(|(i, _)| outside(i.x, i.y))
            .map(|(i, boss)| (pos.0 as f32 + i.x, pos.1 as f32 + i.y, icon_label(i), boss.clone()))
            .collect();
        *far_age = 0;
    } else if *far_age < FAR_FRAMES {
        *far_age += 1;
        for (x, y, to, boss) in far_icons.iter() {
            let (dx, dy) = (x - pos.0 as f32, y - pos.1 as f32);
            // Not when this frame shows it already (an icon in the capture's margin).
            let shown = fed.iter().any(|(i, b)| b == boss && (i.x - dx).abs() <= 10.0 && (i.y - dy).abs() <= 10.0);
            if outside(dx, dy) && !shown {
                list.push(nav::ExitIcon { dx: dx as f64, dy: dy as f64, to: to.clone(), boss: boss.clone() });
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

/// The needle straight to `target`'s exit when the map has shown that exit by name:
/// sure, as the icon is the exit. For zones the navigator does not cover.
fn named_exit_guide(target: &guide::NavTarget, named: &[(f32, f32, String)], pos: (i32, i32)) -> Option<guide::GuideView> {
    target.exit.as_ref()?;
    let (x, y, _) = named.iter().find(|(_, _, name)| name.eq_ignore_ascii_case(&target.label))?;
    let (dx, dy) = (x - pos.0 as f32, y - pos.1 as f32);
    Some(guide::GuideView {
        variant: "named exit".into(),
        target_kind: "entrance".into(),
        target: target.label.clone(),
        degrees: Some(dy.atan2(dx).to_degrees()),
        source: "estimated".into(),
        distance: dx.hypot(dy),
        confidence: 1.0,
        measured: true,
        turned: false,
    })
}

impl Session {
    /// The frame's icons with the named kinds that colour does not make (a boss's icon, a
    /// grey one, a skull) kept only once read at the same map place (`SAME_ICON` cells, the
    /// same sprite) in two frames: an icon stays where it is, the scene a sprite was matched
    /// on does not. Exits, checkpoints and waypoints pass as read.
    fn confirm_named(&mut self, icons: &[crate::minimap::bigmap::Icon], pos: (i32, i32)) -> Vec<crate::minimap::bigmap::Icon> {
        self.frame_no += 1;
        let frame = self.frame_no;
        self.named_seen.retain(|n| frame - n.last <= NAMED_FRAMES);
        let mut out = Vec::with_capacity(icons.len());
        for icon in icons {
            if matches!(icon.kind, "exit" | "checkpoint" | "waypoint") {
                out.push(icon.clone());
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
        let sprites = crate::minimap::bigmap::sprites();
        let pair_of = |icon: &crate::minimap::bigmap::Icon| icon.sprite.and_then(|id| sprites.iter().find(|s| s.id == id)).and_then(|s| s.pair.clone());
        let live: Vec<String> = icons.iter().filter(|i| i.kind == "boss").filter_map(pair_of).collect();
        for icon in icons.iter().filter(|i| matches!(i.kind, "boss" | "boss-dead")) {
            let Some(pair) = pair_of(icon) else { continue };
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

    /// Hands the navigator the frame's exit and boss icons (see `exit_list`) and its new
    /// checkpoints; the named kinds only once seen in two frames (`confirm_named`).
    fn feed_icons(&mut self, icons: &[crate::minimap::bigmap::Icon], whole: bool, pos: (i32, i32), size: (usize, usize), bosses: &[Boss]) {
        let icons = self.confirm_named(icons, pos);
        let icons = icons.as_slice();
        self.see_bosses(icons, pos, bosses);
        let checkpoints = self.new_checkpoints(icons, pos);
        let Some(nav) = self.navigator.as_mut() else { return };
        if let Some(log) = self.obs.get_mut() {
            // Far exits this frame uses or replaces: the replay needs this frame's icons.
            let far = !self.far_icons.is_empty() && (whole || self.far_age < FAR_FRAMES);
            log.icons(obs::now_ms(), icons, whole, size, far);
        }
        let list = exit_list(icons, whole, pos, size, &mut self.far_icons, &mut self.far_age, &self.recording.area, bosses);
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
    catalog: guide::Catalog,
    completed: HashMap<String, Vec<String>>,
    /// Open checklist targets per area, in checklist order.
    targets: HashMap<String, Vec<guide::NavTarget>>,
    /// Every checklist target seen, per area (to find a step the log just completed).
    known_targets: HashMap<String, Vec<guide::NavTarget>>,
    /// Per area: its bosses from the checklists, with where each stands.
    bosses: HashMap<String, Vec<Boss>>,
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
        for [x, y, n] in part.walls {
            // A kept piece's cells are kept.
            let cell = (x + dx, y + dy);
            let count = live.walls.entry(cell).or_insert(0);
            *count += n as u32;
            let count = *count;
            live.held.remove(&cell);
            live.kept.insert(cell, count);
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
        // Flight recordings are kept for a month.
        if let Some(dir) = &dir {
            obs::prune(dir);
        }
        Recorder(Mutex::new(State {
            catalog: guide::Catalog::load(),
            completed: HashMap::new(),
            targets: HashMap::new(),
            known_targets: HashMap::new(),
            bosses: HashMap::new(),
            icons: Vec::new(),
            icons_whole: false,
            log_completed: Default::default(),
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
                // From a zone that is no neighbour (no exit leads there): the zone's start, not
                // that zone's exit. A new character's spawn in the Riverbank was recorded as
                // "arrived from G1_2", the old character's last zone (the owner's run of
                // 2026-11-12); the navigator already started at the start node, the recording,
                // the flight recorder and the scale measure took the exit's name.
                let arrived_from = state.previous_area.clone().map(|from| {
                    let neighbour = from.is_empty() || state.catalog.graph(area).is_none_or(|g| g.names.contains_key(&from));
                    if neighbour { from } else { String::new() }
                });
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
                    far_icons: Vec::new(),
                    far_age: FAR_FRAMES,
                    named_exits: Vec::new(),
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
        let mut state = self.0.lock().ok()?;
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
        let placed = if live.walls.is_empty() {
            Some(live.pos)
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
        match placed {
            Some(pos) => {
                session.feed(pos);
                session.feed_icons(&icons, whole, pos, (frame.cols, frame.rows), bosses);
                note_named_exits(&mut session.named_exits, &icons, pos);
                let live = &mut session.live;
                live.velocity = ((pos.0 - live.pos.0).clamp(-NEAR, NEAR), (pos.1 - live.pos.1).clamp(-NEAR, NEAR));
                live.pos = pos;
                live.lost_streak = 0;
                live.part.matched += 1;
                live.add_walls(&rel, pos, frame.message.starts_with("Reading big map"));
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
                live.part.lost += 1;
                live.lost_streak += 1;
                if live.lost_streak >= LOST_LIMIT {
                    // Lost track (teleport, map overlay, big jump): keep what we have, start over.
                    // The navigator carries on; the new part's coordinates are new.
                    let finished = std::mem::replace(&mut session.live, Live::new());
                    session.live.centre = finished.centre;
                    session.live.frame_size = finished.frame_size;
                    session.fed = None;
                    session.far_icons.clear();
                    session.named_exits.clear();
                    session.came_in = None;
                    // Places of the old part mean nothing in the new one's cells.
                    session.boss_places.clear();
                    session.named_seen.clear();
                    session.checkpoint_marks.clear();
                    crate::minimap::bigmap::set_icon_hints(vec![]);
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
        // Every frame (about three a second): the needle should turn as the player does.
        let due_view = state.last_view.is_none_or(|t| t.elapsed() >= Duration::from_millis(300));
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
            if let Some(session) = &state.session {
                if let Some(log) = session.obs.borrow_mut().as_mut() {
                    log.flush_now();
                }
            }
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

    /// The map icons of the frame about to be handed to `on_frame` (zone exits,
    /// checkpoints, the waypoint), in minimap cells from the player; its exits go to
    /// the navigator with the frame. `whole`: a whole-window scan, listing every icon on
    /// screen (other frames list only those inside the wall frame): the far icons of the
    /// last scan are replaced by this one's, also when it has none.
    pub fn on_icons(&self, icons: &[crate::minimap::bigmap::Icon], whole: bool) {
        if let Ok(mut state) = self.0.lock() {
            state.icons = icons.to_vec();
            state.icons_whole = whole;
        }
    }

    /// The current zone's bosses (names as given to `set_bosses`) that the map showed
    /// dead in this visit (see `Session::see_bosses` for when a grey icon counts). Empty
    /// outside a zone, and whenever no boss icon was read: being wrong about the icons
    /// costs nothing.
    #[allow(dead_code)] // for the chair to wire to the checklist
    pub fn dead_bosses(&self) -> Vec<String> {
        self.0.lock().ok().and_then(|s| s.session.as_ref().map(|s| s.dead.clone())).unwrap_or_default()
    }

    /// The zones' bosses, from the checklists: (area, boss). Set once at start.
    pub fn set_bosses(&self, bosses: Vec<(String, Boss)>) {
        if let Ok(mut state) = self.0.lock() {
            state.bosses.clear();
            for (area, boss) in bosses {
                state.bosses.entry(area).or_default().push(boss);
            }
        }
    }

    /// Targets whose step the log completed (not a hand tick), as (area, label). Call
    /// before `set_completed`: a newly completed one is an anchor for the navigator.
    pub fn set_log_completed(&self, done: Vec<(String, String)>) {
        if let Ok(mut state) = self.0.lock() {
            state.log_completed = done.into_iter().collect();
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
            bosses: self.bosses.get(&session.recording.area).cloned().unwrap_or_default(),
        };
        Some((nav::Navigator::new(&area, arrival, memory.as_ref(), scale, session.recording.seed.unwrap_or(0)), start))
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

    /// Saves and closes the current recording, and learns the zone's scale from it.
    fn finish(&mut self) {
        self.save();
        let Some(session) = self.session.take() else { return };
        if let Some(log) = session.obs.borrow_mut().as_mut() {
            log.end(obs::now_ms());
        }
        let pois = &session.live.part.pois;
        let Some(arrival) = pois.iter().filter(|p| p.kind == "arrived").min_by_key(|p| p.at) else { return };
        let area = &session.recording.area;
        let walls: Vec<(i32, i32)> = session
            .live
            .kept()
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
        let fresh = self.fresh_guide(session);
        let kept = fresh.is_none();
        let view = match fresh {
            Some(view) => {
                *session.last_guide.borrow_mut() = Some(view.clone());
                Some(view)
            }
            None => session.last_guide.borrow().clone().map(|view| guide::GuideView { source: "unconfirmed".into(), ..view }),
        };
        if let (Some(view), Some(log)) = (&view, session.obs.borrow_mut().as_mut()) {
            let stale = session.fed_at.is_none_or(|t| t.elapsed() > STALE);
            // A needle through the recorded map: where it looked and how much fog the way crossed.
            let way = session.navigator.as_ref().and_then(|n| n.last_way()).filter(|_| !kept).map(|w| obs::Way {
                ahead: Some([w.ahead.0 as f32, w.ahead.1 as f32]),
                fog: Some(w.unrecorded as f32),
            });
            // A way ahead in shadow: what the map would have shown in its place.
            let shadow = session.navigator.as_ref().and_then(|n| n.last_shadow()).filter(|_| !kept).map(|h| obs::Shadow {
                degrees: h.degrees,
                length: h.length,
                ahead: [h.ahead.0 as f32, h.ahead.1 as f32],
                fog: h.unrecorded as f32,
                gold: h.gold,
            });
            log.needle(obs::now_ms(), &self.nav_targets(session), view, kept, stale, way, shadow);
        }
        view
    }

    /// The targets the navigator's needle is asked for, as `fresh_guide` builds them.
    fn nav_targets(&self, session: &Session) -> Vec<guide::NavTarget> {
        let area = session.recording.area.as_str();
        let completed = self.completed.get(area).map(Vec::as_slice).unwrap_or_default();
        let mut targets = self.targets.get(area).cloned().unwrap_or_default();
        if let Some(graph) = self.catalog.graph(area) {
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
        let completed = self.completed.get(area).map(Vec::as_slice).unwrap_or_default();
        let scale = self.scale(area);
        if let Some(nav) = &session.navigator {
            // The checklist's open targets, then the onward exit (zones without a
            // checklist, or targets no layout has).
            let mut targets = self.targets.get(area).cloned().unwrap_or_default();
            if let Some(graph) = self.catalog.graph(area) {
                targets.extend(graph::target(&graph, &session.came_from, completed));
            }
            // The part's recorded map, lent: the way to a target whose place on it is known.
            let place = nav::Place {
                map: route::Map { walls: &live.kept, trail: &live.part.trail },
                player: live.pos,
                came_in: session.came_in.as_ref().filter(|c| !c.from.is_empty()).map(|c| nav::CameIn {
                    from: &c.from,
                    at: c.at,
                    delay: c.delay,
                    waypoint: c.waypoint,
                    exits: &c.exits,
                }),
            };
            return nav
                .guide_on(&targets, Some(&place))
                .map(|view| not_stale(guide::GuideView { measured: scale.is_some(), ..view }, session.fed_at));
        }
        // No layout graphs here (a town): the exit the map shows by name is where to go.
        let first = self.targets.get(area).and_then(|targets| targets.first());
        if let Some(view) = first.and_then(|target| named_exit_guide(target, &session.named_exits, live.pos)) {
            return Some(view);
        }
        // Everything relative to the (first) arrival point, where the terrain match starts;
        // a piece without one (tracking lost, overlay restarted) is placed anywhere.
        let arrival = live.part.pois.iter().filter(|p| p.kind == "arrived").min_by_key(|p| p.at);
        let (ax, ay) = arrival.map_or((0, 0), |a| (a.x, a.y));
        let arrived_from = arrival.map_or("", |a| a.label.as_str());
        let mut walls: Vec<(i32, i32)> = live.kept().filter(|(_, &n)| n >= 2).map(|(&(x, y), _)| (x - ax, y - ay)).collect();
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
        // The navigator's needle without the recorded map: whether the icon reached
        // it. (The app's needle follows the way round the corridor's recorded wall
        // to the icon: `route.rs` and `nav.rs` test that.)
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
        // A scan that sees no far exit clears them.
        recorder.on_icons(&[], true);
        recorder.on_frame(&frame((8, 0)), [300, 260]);
        assert!(recorder.0.lock().unwrap().session.as_ref().unwrap().far_icons.is_empty());
        // So the frames after it hand over no old far icon.
        recorder.on_icons(&[], false);
        recorder.on_frame(&frame((12, 0)), [300, 260]);
        assert!(recorder.0.lock().unwrap().session.as_ref().unwrap().far_icons.is_empty());
    }

    #[test]
    fn a_needle_without_frames_for_two_seconds_is_not_gold() {
        let gold = guide::GuideView {
            variant: String::new(), target_kind: "entrance".into(), target: "X".into(), degrees: Some(10.0),
            source: "estimated".into(), distance: 5.0, confidence: 0.9, measured: false, turned: false,
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
        // An unnamed pair (the Count's, read by hand as Count Geonor): either of the zone's.
        assert_eq!(names(&icon("boss", Some("CountActive"), 0.0, 0.0), "G1_15", &manor), ["Candlemass", "Count Geonor"]);
        // Its places for the navigator, each once.
        assert_eq!(boss_places(&icon("boss", Some("CountActive"), 0.0, 0.0), "G1_15", &manor), [nav::BossAt::Tag("boss".into())]);
        assert_eq!(boss_places(&icon("boss", Some("MillerActive"), 0.0, 0.0), "G1_1", &miller), [nav::BossAt::Exit("G1_town".into())]);
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
        // Then grey and alone at his place, two frames: dead.
        walk(6..8, &|x| vec![icon("boss-dead", Some("MillerInactive"), live(x).0 + 1.0, live(x).1 - 1.0)]);
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
    fn boss_icons_are_logged_and_replayed_as_fed() {
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
        let first = text.lines().find(|l| !l.is_empty()).unwrap();
        assert!(first.contains(r#""bosses":[{"name":"Beira"}]"#), "{first}");
        // Read on the scans (every third frame): written from its second, at 400 - 12 cells.
        assert!(text.contains(r#""kind":"boss","x":388.0,"y":-60.0,"sprite":"CroneActive""#), "{text}");
        // The grey skull is no navigator input: written once, as a mark.
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
        // The first needles would follow the recorded corridor round to the far exit:
        // ways ahead run in shadow, so the needle shown is the one without the map (the
        // replay gives it), and the map's way goes on the line as `shadow`, scored apart.
        assert!(!nav::MAP_AHEAD_LIVE);
        assert!(text.contains("\"shadow\":{") && !text.contains("\"way\":true") && v.mapped == 0, "{text}");
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
    fn an_arrival_is_the_exits_place_only_by_its_exit_and_soon_after_the_entry() {
        use crate::minimap::bigmap::Icon;
        let back = guide::NavTarget { label: "Clearfell".into(), exit: Some("G1_2".into()), ..Default::default() };
        // Into Mud Burrow from Clearfell; the first frame's icons; then 19 frames east.
        let visit = |icons: Vec<Icon>, late: bool| {
            let recorder = Recorder::new(None);
            enter(&recorder, "G1_2");
            enter(&recorder, "G1_3");
            recorder.set_targets(vec![("G1_3".into(), back.clone())]);
            if late {
                let mut state = recorder.0.lock().unwrap();
                state.session.as_mut().unwrap().entered = Instant::now().checked_sub(Duration::from_secs(20)).unwrap();
            }
            recorder.on_icons(&icons, false);
            for step in 0..20 {
                recorder.on_frame(&frame((step * 4, 0)), [300, 260]);
            }
            let view = recorder.view().unwrap().guide.unwrap();
            let state = recorder.0.lock().unwrap();
            let on_map = state.session.as_ref().unwrap().navigator.as_ref().unwrap().on_map();
            (on_map, view)
        };
        let exit = Icon { kind: "exit", x: -6.0, y: 2.0, label: None, sprite: None };
        let waypoint = Icon { kind: "waypoint", x: 5.0, y: -4.0, label: None, sprite: None };
        let (on_map, view) = visit(vec![exit], false);
        assert!(on_map && view.source == "estimated", "{view:?}");
        // Back west along the corridor.
        assert!(guide::angle_between(view.degrees.unwrap(), 180.0) < 20.0, "{view:?}");
        // (Without the exit's icon, which would be a sighting of its own.)
        let (on_map, view) = visit(vec![waypoint], false);
        assert!(!on_map, "by a waypoint: {view:?}");
        let (on_map, view) = visit(vec![], true);
        assert!(!on_map, "20 s after the entry: {view:?}");
        let (on_map, view) = visit(vec![], false);
        assert!(on_map && view.source == "rough", "no exit icon by it: {view:?}");
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
        fed: None, fed_at: None, far_icons: Vec::new(), far_age: FAR_FRAMES, named_exits: Vec::new(),
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
            println!("   no layout graphs (terrain arrow)");
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
