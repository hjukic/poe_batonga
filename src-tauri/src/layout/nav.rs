//! The navigator: one belief over (layout, position) per zone visit, updated with
//! every step the minimap tracking measures and every log anchor, never thrown
//! away when tracking is lost.
//!
//! Each layout of the zone has a small particle filter ("map matching" on its
//! corridor graph): weighted guesses of where the player is and how far off the
//! minimap scale is. Movement moves them; every `WEIGH_EVERY` units walked they
//! are weighted by how close they are to a corridor. The mean weight adds to the
//! layout's log evidence, which gives the layout posterior. The needle is the
//! posterior-weighted mean of the directions the corridors lead each particle.

use super::graph::{Area, Guidance, Layout, FAR};
use super::guide::{screen_bearing, to_screen, GuideView, NavTarget};
use super::route;
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};

/// Particles per layout.
const PARTICLES: usize = 200;
/// Spread of the arrival guess around the arrival exit (world units).
const SPREAD: f64 = 40.0;
/// Share of particles drawn along all corridors at arrival (a waypoint or a
/// portal puts the player elsewhere than the exit the log names); more when
/// coming from a town, where portals and waypoints are the usual way back.
const STRAY: f64 = 0.1;
const STRAY_TOWN: f64 = 0.3;
/// World units walked between two weighings.
const WEIGH_EVERY: f64 = 15.0;
/// The walked line's evidence is tempered: successive weighings see the same
/// stretch of corridor, so each counts half.
const TEMPER: f64 = 0.5;
/// What the walked line alone can claim: the top layout at most this share of the
/// belief (the lead over every other layout is capped accordingly). The graph's
/// straight corridors differ from the real ones in ways that do not average out
/// (Clearfell 2693856399: the walk prefers a wrong layout), so walking on must not
/// turn a lead into certainty; anchors and exit icons do that.
const PATH_MOST: f64 = 0.6;

/// The most log evidence the walked line can give one layout over another, for `n` layouts.
fn path_cap(n: usize) -> f64 {
    (PATH_MOST / (1.0 - PATH_MOST) * (n as f64 - 1.0).max(1.0)).ln().max(0.5)
}

/// Closeness to a corridor: full weight within `FLAT` world units of its centre
/// line (the walkable width), then a Gaussian of width `SIGMA` …
/// Real walks stray far: on the recorded Clearfell and Riverbank visits the best
/// layout's corridors are 26-38 units away at the median, 90-120 for one point in ten.
const FLAT: f64 = 60.0;
const SIGMA: f64 = 60.0;
/// … over a floor, so one bad step cannot kill a layout.
const FLOOR: f64 = 0.1;
/// Movement noise: this share of the distance moved, plus a little per weighing.
const NOISE: f64 = 0.2;
const NOISE_MIN: f64 = 1.0;
/// Scale error of the particles: spread at start, and drift per weighing.
const SCALE_SPREAD: f64 = 0.04;
const SCALE_DRIFT: f64 = 0.003;
const SCALE_LIMIT: f64 = 0.15;
/// Dungeons ("dgr"): the room grid's scale is a guess, its error up to this factor.
const DUNGEON_SCALE: f64 = 2.5;
/// When the particles fit this badly on average, some are redrawn along the corridors.
const LOST_FIT: f64 = 0.15;
const RESEED: f64 = 0.1;
/// What a player walks per second (world units), and the most a loss spreads.
const WALK_SPEED: f64 = 60.0;
const MAX_REACH: f64 = 2400.0;
/// During a loss most particles walk on along their corridor, mostly the way the
/// player was heading, between this share of the reach and all of it; the rest
/// spread anywhere within reach.
const ALONG: f64 = 0.8;
const ALONG_MIN: f64 = 0.25;
const KEEP_HEADING: f64 = 0.8;
/// The heading is the movement of about the last this many world units.
const HEADING_LENGTH: f64 = 60.0;
/// After a loss this long (seconds) a share of the particles is redrawn along
/// all corridors: the player may have used a waypoint, checkpoint or portal.
const LONG_LOSS: f64 = 20.0;
const REDRAW: f64 = 0.3;
/// A log anchor: how close (world units) the particles must be to the node (a
/// tag is a boss arena's centre, the line may play at its entrance) …
const ANCHOR_SIGMA: f64 = 80.0;
const TAG_SIGMA: f64 = 150.0;
const ANCHOR_FLOOR: f64 = 0.02;
/// Leaving the zone: probably through the exit the log names, but a portal,
/// waypoint or death leaves from anywhere; the evidence never drops below this.
const LEFT_FLOOR: f64 = 0.3;
/// A boss spoke: how near its node the player is taken to be (world units), and what
/// a layout whose belief is elsewhere keeps.
const NEAR_SIGMA: f64 = 250.0;
const NEAR_FLOOR: f64 = 0.3;
/// … the log evidence a layout without such a node loses …
const MISSING: f64 = -7.0;
/// … and how far around the node the particles are placed.
const ANCHOR_SPREAD: f64 = 25.0;
/// Needle sources: `|V|` at least `GOLD` with absolute evidence this visit (a log
/// anchor or an exit icon) is "estimated", at least `ROUGH` is "rough". The walked
/// line alone is never gold (`GOLD_WALK`: never): in the simulation its rare gold
/// steps were wrong (4 of 4 at three seeds), and on Clearfell it preferred a wrong
/// layout.
const GOLD: f64 = 0.9;
const ROUGH: f64 = 0.5;
/// … and gold needs the top layout at this probability.
const GOLD_LAYOUT: f64 = 0.9;
const GOLD_WALK: f64 = f64::INFINITY;
/// Areas whose needle was checked against a replayed recording with a known
/// truth: only there can it be gold. Elsewhere it is at most "rough", and in
/// dungeons (room grids, scale not measured) "unconfirmed".
const MEASURED: [&str; 2] = ["G1_1", "G1_2"];
/// Frame-to-frame moves under this many minimap cells are registration jitter:
/// they move the particles but do not count as walked.
const JITTER: f64 = 2.0;
/// A minimap scale (cells per world unit) outside this range is a bad
/// measurement; the navigator then uses the default.
const SCALE_RANGE: (f64, f64) = (0.4, 1.6);
/// The top layout is named once it has at least this probability.
const SURE: f64 = 0.5;
/// Layouts below this probability are left out of the needle.
const NEGLIGIBLE: f64 = 1e-4;
/// A remembered posterior is mixed with this share of the uniform prior.
const MEMORY_MIX: f64 = 0.1;
/// A remembered position is resumed when it is at most this old (seconds); the
/// resumed posterior keeps only this share of the uniform prior.
const RESUME: u64 = 600;
const RESUME_MIX: f64 = 1e-3;
/// Particles kept per likely layout in `Memory`, and the least probability kept.
const KEEP: usize = 32;
const KEEP_FROM: f64 = 0.01;

/// Exit icons on the map: an icon at offset `d` from the player says the player
/// stands at (some exit of the layout) − d. An exit sits up to about this far
/// (world units) from its graph node (Clearfell's two exits: 130 units off the
/// graph's vector) …
const ICON_TOL: f64 = 100.0;
/// … a false or wrongly read icon costs at most this (per icon) …
const ICON_FLOOR: f64 = 0.05;
/// … and a layout that explains the icons neither by its belief nor by the
/// geometry of two icons gets this much (the same for every layout).
const ICON_BASE: f64 = 0.005;
/// Named exits that do not fit a layout at all cost it this much (the fit's floor).
const NAMED_BASE: f64 = 1e-4;
/// Frames three a second see the same icons: a new set of icons counts in full;
/// the same set again only after the player walked this far (world units) since
/// the last update, and then for this much.
const ICON_EVERY: f64 = 60.0;
const ICON_TEMPER: f64 = 0.5;
/// Share of particles redrawn where the icons put the player, when the belief
/// does not explain them.
const ICON_INJECT: f64 = 0.3;
/// The needle points straight at the target's icon once the route there (from
/// where the icon puts the player) is under `ICON_TO` times the straight distance,
/// and back along the route above `ICON_BACK`: a river or a wall between makes the
/// route bend, and pointing across it was wrong in the simulation.
const ICON_TO: f64 = 1.2;
const ICON_BACK: f64 = 1.5;

/// The hysteresis between the needle at the icon and along the route.
fn at_icon(before: bool, ratio: f64) -> bool {
    if ratio < ICON_TO {
        true
    } else if ratio > ICON_BACK {
        false
    } else {
        before
    }
}
/// The needle at an icon is gold only this close (minimap cells): farther, walls
/// and dead ends between make the straight line a poor guide (simulation).
const ICON_GOLD_WITHIN: f64 = 200.0;
/// A layout carrying at least this share of the belief is moved less by icons: no
/// particles redrawn, and its weights only half-follow the icons.
const ICON_FIRM: f64 = 0.5;
/// An icon update confirms the belief (makes gold possible) when the belief put
/// at least this share on the icons landing on exits.
const ICON_CONFIRM: f64 = 0.5;
/// The needle points at an exit icon once this share of the belief says it is the
/// target's exit (a labelled icon says so itself).
const ICON_SURE: f64 = 0.95;
/// A new boss sighting the belief predicted is no absolute evidence (gold stays with exits
/// and log anchors), though a new unlabelled exit sighting is. A boss's arena is wide and
/// Clearfell's layouts have up to two boss nodes.
const BOSS_CONFIRMS: bool = false;
/// Whether what bosses' icons add to a layout's share counts for the 90% gold needs. Off:
/// in the simulation (2 seeds, minimap view, `nav_sim::boss_icons_report`) counting it made
/// 5 of 22 gold needles point more than 60° off, against 2 of 10 without it and 0 of 14
/// without boss icons; confirming as well changed nothing.
const BOSS_GOLD: bool = false;
/// Bosses' icons move the particles (weigh them, redraw some where they put the player)
/// as exit icons do. Off (the review): they only weigh the layouts (`Bank::boss`), the
/// position comes from the walk, exits and anchors.
const BOSS_PARTICLES: bool = false;

/// A way through the recorded map is taken unless it is longer than this many
/// times the straight line (and this many minimap cells more): a longer one
/// probably goes round the outside of what is recorded.
const ABSURD: f64 = 4.0;
const ABSURD_SLACK: f64 = 150.0;
/// A way with less than this share by recorded ground (within 15 cells of a wall
/// seen twice or of the walk) is mostly a guess across fog: never gold, and when it
/// is also about the straight line it says nothing the map knows, so the needle is
/// as without the map (the layouts' route then still counts).
const RECORDED: f64 = 0.5;
/// Gold only for a way at most this many times the straight line: a longer one
/// leans on a door the recorder may have made (a gap, a grazed line) or on fog.
/// The way back is exempt where it keeps to the walk (`WALKED_GOLD` of it within
/// 15 cells of the trail): every door on it was walked through.
const GOLD_RATIO: f64 = 1.5;
const WALKED_GOLD: f64 = 0.8;

/// What the last `guide_on` needle took from the recorded map (for the flight
/// recorder): where it looked, in minimap cells from the player, and the share of
/// the way over unrecorded ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WayNote {
    pub ahead: (f64, f64),
    pub unrecorded: f64,
}

/// Ways ahead (to a target placed from an icon in view or a sighting) are shown only
/// when this is on. Off, they run in shadow: the needle is the one without the map,
/// and what the map would have shown goes to the flight recorder (`last_shadow`).
/// On the owner's only real recording the map needle matched the walk less often
/// than the straight line (16 against 21 of 35; docs/map-reading.md, "The way
/// through the recorded map"). The way back to a trusted arrival is always live.
pub const MAP_AHEAD_LIVE: bool = false;

/// What a way ahead in shadow would have shown: its bearing (screen degrees), its
/// length (cells), where it looked (cells from the player), the share of it over
/// unrecorded ground, and whether it would have been gold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub degrees: Option<f32>,
    pub length: f32,
    pub ahead: (f64, f64),
    pub unrecorded: f64,
    pub gold: bool,
}

/// The memory format; older files are ignored.
pub(super) const MEMORY_VERSION: u32 = 1;

/// Icons this close (world units) on the dead-reckoned track are one exit.
const SIGHT_MATCH: f64 = 40.0;
/// A checkpoint icon is a sighting of some landmark node (`landmark`), soft: on the one
/// instance whose layout is known (Clearfell 2693856399, `ClearFell_W_E_N`) the three
/// checkpoints lie 42, 50 and 94 cells (54-121 units) from the nearest landmark (Mud
/// Burrow's and the Grelwood's exits, the campsite) …
const CHECKPOINT_TOL: f64 = 120.0;
/// … it does not say which landmark, and some landmarks have none: a layout without one
/// near keeps this much, and a checkpoint counts half …
const CHECKPOINT_FLOOR: f64 = 0.3;
const CHECKPOINT_WEIGHT: f64 = 0.5;
/// … and next to an exit icon of the same frame (minimap cells) it adds nothing: the exit
/// counted already (Clearfell: 28 and 42 cells from their exits' icons). What checkpoints
/// can take from a layout is capped (log), at most this many count in a visit, and one
/// the belief puts within `CHECKPOINT_SAME` units of a counted one is that one again
/// (after a tracking loss its place on the track means nothing).
const CHECKPOINT_BY_EXIT: f64 = 60.0;
const CHECKPOINT_CAP: f64 = 1.5;
const CHECKPOINT_MOST: usize = 6;
const CHECKPOINT_SAME: f64 = 150.0;
/// Node tags that name a corridor's shape, not a place: a node with none of these (or an
/// exit) is a landmark. The most common tags of all the graphs' nodes, sorted by hand.
const PLAIN: [&str; 24] = [
    "", "end", "block", "Block", "source", "sink", "connection", "join", "endcap", "deadend", "fill", "fill2", "blank", "cc",
    "Hallcc", "dune_edge", "water_source", "water_sink", "source_slash", "sink_slash", "source_slow", "windowend", "nobridge", "blockage",
];

/// A node a checkpoint may stand by: an exit, or a node whose tag names a place.
pub(super) fn landmark(node: &super::graph::Node) -> bool {
    node.exit.is_some() || !PLAIN.contains(&node.tag.as_deref().unwrap_or(""))
}
/// A boss's icon stands where the boss is, in its arena: a tagged node is the arena's
/// centre (as for a log anchor at a tag, `TAG_SIGMA`); at an exit, as an exit icon.
const BOSS_TOL: f64 = 150.0;

/// One exit (or boss) seen on the map since the last loss.
#[derive(Clone, Debug)]
struct Sighting {
    at: (f64, f64),
    /// A boss's icon: the places it may be (see `ExitIcon::boss`).
    boss: Vec<BossAt>,
    /// The label read most recently, and in how many scans in a row it was the same.
    name: Option<String>,
    scans: u32,
    /// Its label was counted in an update.
    labelled: bool,
    /// Placement jumps of the track before it was first seen (see `JUMPED`).
    jumps: u32,
}

/// A frame placed this far (minimap cells) from the last is a jump: a wide re-find
/// that may be a wrong match. A sighting from before one is no target on the map
/// (its place against the walls may be off by the jump); it still counts as evidence.
const JUMPED: f64 = 20.0;

/// An exit icon of a map frame: its offset from the player in minimap cells
/// (screen axes) and, when its label could be read, the area it leads to (an
/// area id or the zone's name). Unlabelled it is "some exit".
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ExitIcon {
    pub dx: f64,
    pub dy: f64,
    pub to: Option<String>,
    /// Not an exit but a boss's icon (a skull, or the boss's own icon): the places of the
    /// zone's bosses it may be. One place is an exact sighting of it (as an exit icon
    /// is); several are weighed as an unlabelled exit icon is over the exits. It is never
    /// labelled: what makes it gold is what makes an unlabelled exit icon gold.
    pub boss: Vec<BossAt>,
}

/// Where a boss of the zone stands in the layout graphs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BossAt {
    /// At a node with this tag ("boss", "arena", …).
    Tag(String),
    /// At the exit to this area id (the Bloated Miller by the way to the encampment).
    Exit(String),
}

impl ExitIcon {
    /// How far its object may stand from its node (world units).
    fn tolerance(&self) -> f64 {
        if self.boss.iter().any(|b| matches!(b, BossAt::Tag(_))) {
            BOSS_TOL
        } else {
            ICON_TOL
        }
    }
}

/// Where the visit starts.
#[derive(Clone, Copy, Debug)]
pub enum Arrival<'a> {
    /// Came through the exit to this area id ("" = the zone's start).
    From(&'a str),
    Unknown,
}

/// Log anchors: the player is at such a node now.
#[derive(Clone, Copy, Debug)]
pub enum Seen<'a> {
    /// At the exit to this area id (a checklist step at it was completed by the log).
    Exit(&'a str),
    /// At a node with this tag (a boss arena, …).
    Tag(&'a str),
    /// Left the zone to this area id: probably through its exit (soft: no particle
    /// is moved, a layout without the exit loses nothing).
    Left(&'a str),
    /// A boss of the zone spoke: the player is somewhere near a node with this tag
    /// (soft, and wide: a boss may speak before the player reaches the arena).
    NearTag(&'a str),
    /// The same for a boss that stands at the exit to this area id.
    NearExit(&'a str),
}

/// What is worth keeping of a zone instance between visits.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Memory {
    /// Files of an older version (none: before v0.1.42) are not read: their
    /// anchors and exits came from icons counted the old way.
    #[serde(default)]
    pub version: u32,
    /// Layout id → probability.
    pub layouts: BTreeMap<String, f64>,
    /// World units walked with tracking, over all visits (gold counts only this visit's).
    #[serde(default)]
    pub walked: f64,
    /// Where the player was: layout id → equally weighted [x, y, scale error].
    #[serde(default)]
    pub positions: BTreeMap<String, Vec<[f32; 3]>>,
    /// The walked line's and the anchors' log evidence per layout (layouts above).
    #[serde(default)]
    pub path: BTreeMap<String, f64>,
    #[serde(default)]
    pub anchors: BTreeMap<String, f64>,
    /// The named exits' fit per layout (see `Bank::named`).
    #[serde(default)]
    pub named: BTreeMap<String, f64>,
    /// Exits seen on the map: layout id → node → where the icon put it (file units).
    #[serde(default)]
    pub exits: BTreeMap<String, BTreeMap<usize, [f32; 2]>>,
    /// Unix seconds of the last data it holds.
    #[serde(default)]
    pub at: u64,
}

#[derive(Clone, Copy, Debug)]
struct Particle {
    x: f64,
    y: f64,
    /// Scale error: the world moves `1 / s` times what the minimap says.
    s: f64,
    w: f64,
}

struct Bank {
    prior: f64,
    /// Log evidence of the walked line, relative to the best layout, in [-path_cap, 0].
    path: f64,
    /// Log evidence of anchors (and later exit icons).
    evidence: f64,
    /// How well this layout's exits fit the last whole scan's named exits (log),
    /// replaced at every such scan: position-free and absolute.
    named: f64,
    /// What bosses' icons added to the icon evidence (log): in the posterior, not in the
    /// share gold needs (`BOSS_GOLD`), not kept in the memory.
    boss: f64,
    /// What checkpoint icons added (log): in the posterior, never in gold's share.
    checkpoints: f64,
    particles: Vec<Particle>,
}

/// The constants a test may change, to check that the gates catch a broken navigator.
#[derive(Clone, Copy, Debug)]
pub(super) struct Tuning {
    pub gold: f64,
    pub gold_layout: f64,
    pub gold_walk: f64,
    pub missing: f64,
    pub flat: f64,
    /// A new boss sighting the belief predicted confirms it, as an exit's does.
    pub boss_confirms: bool,
    /// What bosses' icons add counts in the layout share gold needs.
    pub boss_gold: bool,
    /// Bosses' icons weigh and redraw particles (`BOSS_PARTICLES`).
    pub boss_particles: bool,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning { gold: GOLD, gold_layout: GOLD_LAYOUT, gold_walk: GOLD_WALK, missing: MISSING, flat: FLAT, boss_confirms: BOSS_CONFIRMS, boss_gold: BOSS_GOLD, boss_particles: BOSS_PARTICLES }
    }
}

/// SplitMix64: small, fast and repeatable.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn uniform(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        (self.uniform() * n as f64) as usize % n.max(1)
    }

    /// Two independent standard normal numbers.
    fn gauss2(&mut self) -> (f64, f64) {
        let r = (-2.0 * (1.0 - self.uniform()).ln()).sqrt();
        let a = std::f64::consts::TAU * self.uniform();
        (r * a.cos(), r * a.sin())
    }
}

pub struct Navigator {
    area: Area,
    /// Minimap cells per world unit.
    scale: f64,
    rng: Rng,
    banks: Vec<Bank>,
    /// World units walked with tracking this visit, and before it (remembered).
    walked: f64,
    walked_before: f64,
    /// Data came in since the start (movement, an anchor): only then is the
    /// memory worth writing; and when the last came (unix seconds).
    changed: bool,
    data_at: u64,
    since_weigh: f64,
    /// Movement not yet applied to the particles: world units at the nominal
    /// scale, and the sum of squared step lengths (for the noise).
    pending: (f64, f64),
    pending_sq: f64,
    /// Recent movement (world units, nominal scale), for where a loss leads.
    heading: (f64, f64),
    /// A log anchor was seen during this visit.
    anchored: bool,
    /// Per (layout, goal nodes): which way the corridors lead.
    guidance: RefCell<HashMap<(usize, Vec<usize>), Guidance>>,
    /// Gold allowed (the area is measured; tests may say so for made-up areas).
    trusted: bool,
    /// The latest frame's exit icons, each with its sighting's label and whether
    /// that label was read the same in two scans.
    icons_now: Vec<(ExitIcon, bool)>,
    /// Exits seen since the last loss, by their place (world units on `track`).
    sightings: Vec<Sighting>,
    /// Checkpoints counted this visit: where the belief put each, per layout (file units).
    checkpoints_counted: Vec<Vec<(f64, f64)>>,
    /// Where the player is by dead reckoning since the last loss (world units,
    /// nominal scale): gives each icon a place, so one sighting counts once.
    track: (f64, f64),
    /// Units walked at the last icon update.
    icon_walked: f64,
    /// Placement jumps of the track so far.
    jumps: u32,
    /// The needle points at the target's icon (else along the route from where
    /// the icon puts the player): switched with hysteresis.
    at_icon: Cell<bool>,
    /// Exits seen per (layout, node): where the icons put them (file units).
    exits_seen: HashMap<(usize, usize), (f64, f64)>,
    tuning: Tuning,
    /// The last way through the recorded map: its target's label and cells (the
    /// part's minimap cells), preferred on the next call (hysteresis).
    way: RefCell<Option<(String, Vec<(i32, i32)>)>>,
    /// The last `guide_on` needle followed that way (for the flight recorder).
    last_way: Cell<Option<WayNote>>,
    /// Ways ahead are shown (`MAP_AHEAD_LIVE`; tests may switch it), and what the
    /// last needle's way ahead would have shown when they are not.
    map_ahead: bool,
    last_shadow: Cell<Option<Shadow>>,
}

/// What the recorder knows of the part being recorded, lent at `guide_on` time.
pub struct Place<'a> {
    /// Its walls and walked trail.
    pub map: route::Map<'a>,
    /// The player's cell (minimap cells, the part's coordinates).
    pub player: (i32, i32),
    /// Where the player appeared in this part, and how.
    pub came_in: Option<CameIn<'a>>,
}

/// The arrival point and what the first placed frame showed there.
#[derive(Clone, Copy, Debug)]
pub struct CameIn<'a> {
    /// The area id the player came from.
    pub from: &'a str,
    pub at: (i32, i32),
    /// Seconds from the zone entry to the first placed frame.
    pub delay: f64,
    /// A waypoint icon by the arrival point.
    pub waypoint: bool,
    /// Exit icons by the arrival point: their labels (None = unread).
    pub exits: &'a [Option<String>],
}

/// The arrival point is the exit's place only when the first placed frame came
/// soon after the zone entry (later the player may have walked on) and no waypoint
/// was by it.
const ARRIVAL_DELAY: f64 = 5.0;

/// Towns are left and entered by waypoints and portals: an arrival from one is
/// no exit's place.
fn town(area: &str) -> bool {
    area.ends_with("_town") || area.ends_with("_Town")
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Navigator {
    pub fn new(area: &Area, arrival: Arrival, memory: Option<&Memory>, scale: f64, seed: u64) -> Self {
        // A memory of an older version holds anchors and exits counted the old way.
        let memory = memory.filter(|m| m.version == MEMORY_VERSION);
        let mut rng = Rng(seed ^ 0x5EED_0F_BA70_4A);
        let scale = if scale.is_finite() && (SCALE_RANGE.0..=SCALE_RANGE.1).contains(&scale) { scale } else { super::graph::SCALE };
        let n = area.layouts.len();
        // Evidence kept per kind carries on; an older memory has only the posterior.
        let evidence = memory.filter(|m| !m.path.is_empty() || !m.anchors.is_empty());
        let remembered = memory.filter(|m| !m.layouts.is_empty() && evidence.is_none());
        let resume = memory.filter(|m| {
            matches!(arrival, Arrival::Unknown) && !m.positions.is_empty() && unix_now().saturating_sub(m.at) <= RESUME
        });
        // A resumed visit keeps its belief; a new visit stays open to other layouts.
        let mix = if resume.is_some() { RESUME_MIX } else { MEMORY_MIX };
        let banks = area
            .layouts
            .iter()
            .map(|layout| {
                let prior = match remembered {
                    Some(m) => ((1.0 - mix) * m.layouts.get(&layout.id).copied().unwrap_or(0.0) + mix / n as f64).ln(),
                    None => -(n as f64).ln(),
                };
                let kept = resume.and_then(|m| m.positions.get(&layout.id)).filter(|p| !p.is_empty());
                let particles = match kept {
                    Some(points) => (0..PARTICLES)
                        .map(|i| {
                            let [x, y, s] = points[i % points.len()];
                            let (gx, gy) = rng.gauss2();
                            let jitter = 10.0 / layout.unit;
                            Particle { x: x as f64 + gx * jitter, y: y as f64 + gy * jitter, s: s as f64, w: 1.0 }
                        })
                        .collect(),
                    None => start(layout, arrival, &mut rng),
                };
                let path = evidence.map_or(0.0, |m| m.path.get(&layout.id).copied().unwrap_or(-path_cap(n)).clamp(-path_cap(n), 0.0));
                // Kept anchor evidence is capped like the path's: a restart must not
                // count a whole visit's sightings in full again.
                let best = evidence.map_or(0.0, |m| m.anchors.values().copied().fold(f64::MIN, f64::max));
                let anchors = evidence.map_or(0.0, |m| {
                    (m.anchors.get(&layout.id).copied().unwrap_or(best - path_cap(n)) - best).clamp(-path_cap(n), 0.0)
                });
                let named = evidence.map_or(0.0, |m| m.named.get(&layout.id).copied().unwrap_or(0.0));
                let mut bank = Bank { prior, path, evidence: anchors, named, boss: 0.0, checkpoints: 0.0, particles };
                normalise(&mut bank.particles);
                bank
            })
            .collect();
        let mut nav = Navigator {
            area: area.clone(),
            scale,
            rng,
            banks,
            walked: 0.0,
            walked_before: memory.map_or(0.0, |m| m.walked),
            changed: false,
            data_at: memory.map_or(0, |m| m.at),
            since_weigh: 0.0,
            pending: (0.0, 0.0),
            pending_sq: 0.0,
            heading: (0.0, 0.0),
            anchored: false,
            guidance: RefCell::default(),
            trusted: MEASURED.contains(&area.id.as_str()),
            icons_now: Vec::new(),
            sightings: Vec::new(),
            checkpoints_counted: Vec::new(),
            track: (0.0, 0.0),
            icon_walked: 0.0,
            jumps: 0,
            at_icon: Cell::new(false),
            exits_seen: memory
                .map(|m| {
                    area.layouts
                        .iter()
                        .enumerate()
                        .flat_map(|(k, l)| m.exits.get(&l.id).into_iter().flatten().map(move |(&n, &[x, y])| ((k, n), (x as f64, y as f64))))
                        .filter(|&((k, n), _)| n < area.layouts[k].nodes.len())
                        .collect()
                })
                .unwrap_or_default(),
            tuning: Tuning::default(),
            way: RefCell::default(),
            last_way: Cell::new(None),
            map_ahead: MAP_AHEAD_LIVE,
            last_shadow: Cell::new(None),
        };
        // The player may have walked on since the position was saved.
        if let Some(m) = resume {
            let elapsed = unix_now().saturating_sub(m.at) as f64;
            if elapsed >= 1.0 {
                nav.lost(elapsed);
            }
        }
        nav
    }

    /// The exits a map icon can be in one layout: those leading to its label's area,
    /// or every exit when it has none; for a boss's icon, the nodes of its places.
    fn icon_nodes(&self, layout: &Layout, icon: &ExitIcon) -> Vec<usize> {
        if !icon.boss.is_empty() {
            return (0..layout.nodes.len())
                .filter(|&i| {
                    let n = &layout.nodes[i];
                    icon.boss.iter().any(|b| match b {
                        BossAt::Tag(tag) => !tag.is_empty() && n.has_tag(tag),
                        BossAt::Exit(area) => n.exit.as_deref() == Some(area.as_str()),
                    })
                })
                .collect();
        }
        let to = icon.to.as_deref().map(|t| self.exit_ids(t));
        (0..layout.nodes.len())
            .filter(|&i| match (&layout.nodes[i].exit, &to) {
                (Some(exit), Some(ids)) => ids.contains(&exit.as_str()),
                (Some(_), None) => true,
                (None, _) => false,
            })
            .collect()
    }

    /// The frame's exit icons (minimap cells from the player, screen axes).
    pub fn icons(&mut self, icons: &[ExitIcon]) {
        self.apply();
        // Each icon to its sighting: the same exit in the next frames (labelled on a
        // whole-window scan, unlabelled between) is one sighting; a label read
        // later upgrades it.
        let (mut new, mut upgraded, mut new_exit) = (false, false, false);
        let mut now = Vec::with_capacity(icons.len());
        for icon in icons {
            let (wx, wy) = super::guide::from_screen((icon.dx / self.scale, icon.dy / self.scale));
            let at = (self.track.0 + wx, self.track.1 + wy);
            // An exit and a boss by it (the Miller at his exit) are two sightings.
            let found = self.sightings.iter().position(|s| s.boss == icon.boss && (s.at.0 - at.0).hypot(s.at.1 - at.1) <= SIGHT_MATCH);
            let i = match found {
                Some(i) => i,
                None => {
                    new = true;
                    new_exit |= icon.boss.is_empty();
                    self.sightings.push(Sighting { at, boss: icon.boss.clone(), name: None, scans: 0, labelled: icon.to.is_some(), jumps: self.jumps });
                    self.sightings.len() - 1
                }
            };
            let sight = &mut self.sightings[i];
            sight.at = ((sight.at.0 + at.0) / 2.0, (sight.at.1 + at.1) / 2.0);
            if let Some(label) = &icon.to {
                if sight.name.as_ref() == Some(label) {
                    sight.scans += 1;
                } else {
                    (sight.name, sight.scans) = (Some(label.clone()), 1);
                }
                if !sight.labelled {
                    sight.labelled = true;
                    upgraded = true;
                }
            }
            let named = ExitIcon { to: sight.name.clone(), ..icon.clone() };
            now.push((named, sight.scans >= 2));
        }
        self.icons_now = now;
        if icons.is_empty() {
            return;
        }
        // A new sighting counts in full; a label read for one already counted, or
        // the same ones again after walking on, at half weight; else nothing new.
        let weight = if new {
            1.0
        } else if upgraded || self.walked - self.icon_walked >= ICON_EVERY {
            ICON_TEMPER
        } else {
            return;
        };
        self.icon_walked = self.walked;
        let icons: Vec<ExitIcon> = self.icons_now.iter().map(|(i, _)| i.clone()).collect();
        let icons = icons.as_slice();
        self.touched();
        let before = self.posterior();
        let mut confirmed = 0.0;
        let scale = self.scale;
        let candidates: Vec<Vec<Vec<usize>>> =
            self.area.layouts.iter().map(|l| icons.iter().map(|i| self.icon_nodes(l, i)).collect()).collect();
        let boss_still = !self.tuning.boss_particles;
        let Navigator { area, banks, rng, exits_seen, .. } = self;
        let mut learnt = Vec::new();
        for (k, (layout, bank)) in area.layouts.iter().zip(banks.iter_mut()).enumerate() {
            let cands = &candidates[k];
            // The evidence uses the graph's nodes only: a remembered place was learnt
            // under this layout's own belief, so it would make every layout agree with
            // its own past sightings, and icons could no longer tell layouts apart.
            let at = |n: usize| (layout.nodes[n].x, layout.nodes[n].y);
            // Where an icon is from a particle, in file units.
            let offset = |icon: &ExitIcon, s: f64| {
                let (x, y) = super::guide::from_screen((icon.dx / (scale * s), icon.dy / (scale * s)));
                (x / layout.unit, y / layout.unit)
            };
            // How well the icons (those `only` lets through) land on their nodes from a particle.
            let like_of = |p: &Particle, only: &dyn Fn(&ExitIcon) -> bool| -> f64 {
                icons
                    .iter()
                    .zip(cands)
                    .filter(|(icon, _)| only(icon))
                    .map(|(icon, nodes)| {
                        let (ox, oy) = offset(icon, p.s);
                        let d2 = nodes
                            .iter()
                            .map(|&n| {
                                let (x, y) = at(n);
                                ((p.x + ox - x).powi(2) + (p.y + oy - y).powi(2)) * layout.unit * layout.unit
                            })
                            .fold(f64::INFINITY, f64::min);
                        let tol = icon.tolerance();
                        ICON_FLOOR + (1.0 - ICON_FLOOR) * (-d2 / (2.0 * tol * tol)).exp()
                    })
                    .product()
            };
            let like = |p: &Particle| like_of(p, &|_| true);
            let agree: f64 = bank.particles.iter().map(|p| p.w * like(p)).sum();
            confirmed += before[k] * agree;
            // Two icons: does any pair of this layout's exits lie as far apart? This
            // needs no position belief at all.
            let mut geometry = 0.0;
            let labelled: Vec<usize> = (0..icons.len()).filter(|&j| icons[j].to.is_some()).collect();
            // Two or more exits read by name: how well this layout's named exits lie
            // as the icons do decides on its own, whatever was believed before; it
            // replaces the last such fit (one scan, however often seen, counts once).
            if labelled.len() >= 2 {
                let world = |i: &ExitIcon| super::guide::from_screen((i.dx / scale, i.dy / scale));
                let mut fit = 1.0;
                for &j in &labelled[1..] {
                    let (a, b) = (world(&icons[labelled[0]]), world(&icons[j]));
                    let want = (b.0 - a.0, b.1 - a.1);
                    let mut best = 0.0f64;
                    for &n1 in &cands[labelled[0]] {
                        for &n2 in &cands[j] {
                            if n1 == n2 {
                                continue;
                            }
                            let (p, q) = (at(n1), at(n2));
                            let got = ((q.0 - p.0) * layout.unit, (q.1 - p.1) * layout.unit);
                            let d2 = (got.0 - want.0).powi(2) + (got.1 - want.1).powi(2);
                            best = best.max((-d2 / (4.0 * ICON_TOL * ICON_TOL)).exp());
                        }
                    }
                    fit *= best;
                }
                bank.named = fit.max(NAMED_BASE).ln();
            }
            // How well the first of `ids` and each other lie as two of this layout's nodes do.
            let pairs = |ids: &[usize]| -> f64 {
                let world = |i: &ExitIcon| super::guide::from_screen((i.dx / scale, i.dy / scale));
                let mut fit = 1.0;
                for &j in &ids[1..] {
                    let (a, b) = (world(&icons[ids[0]]), world(&icons[j]));
                    let want = (b.0 - a.0, b.1 - a.1);
                    let mut best = 0.0f64;
                    for &n1 in &cands[ids[0]] {
                        for &n2 in &cands[j] {
                            if n1 == n2 {
                                continue;
                            }
                            let (p, q) = (at(n1), at(n2));
                            let got = ((q.0 - p.0) * layout.unit, (q.1 - p.1) * layout.unit);
                            let d2 = (got.0 - want.0).powi(2) + (got.1 - want.1).powi(2);
                            best = best.max((-d2 / (4.0 * ICON_TOL * ICON_TOL)).exp());
                        }
                    }
                    fit *= best;
                }
                fit
            };
            if labelled.len() < 2 && icons.len() >= 2 {
                geometry = pairs(&(0..icons.len()).collect::<Vec<_>>());
            }
            let total = weight * agree.max(geometry).max(ICON_BASE).ln();
            // A boss's icon moves the belief but is kept apart (`Bank::boss`): gold does not
            // lean on it (see `BOSS_GOLD`). The exits alone give `evidence` as before.
            let exits: Vec<usize> = (0..icons.len()).filter(|&j| icons[j].boss.is_empty()).collect();
            if exits.len() == icons.len() {
                bank.evidence += total;
            } else {
                let alone = if exits.is_empty() {
                    0.0
                } else {
                    let agree_exits: f64 = bank.particles.iter().map(|p| p.w * like_of(p, &|i| i.boss.is_empty())).sum();
                    let geometry_exits = if labelled.len() < 2 && exits.len() >= 2 { pairs(&exits) } else { 0.0 };
                    weight * agree_exits.max(geometry_exits).max(ICON_BASE).ln()
                };
                bank.evidence += alone;
                bank.boss += total - alone;
            }
            // The belief does not explain the icons: also try where they put the player,
            // unless this layout already carries the belief (a false or misread icon
            // must not move a confident bank much).
            let confident = before[k] >= ICON_FIRM;
            if agree < 0.3 && !confident {
                // Not where a boss's icon puts the player (`BOSS_PARTICLES`): an arena is
                // wide, and a layout may have two boss nodes.
                let soft = |j: usize| boss_still && !icons[j].boss.is_empty();
                let labelled: Vec<usize> = (0..icons.len()).filter(|&j| icons[j].to.is_some() && !cands[j].is_empty()).collect();
                let any: Vec<usize> = (0..icons.len()).filter(|&j| !cands[j].is_empty() && !soft(j)).collect();
                let pool = if labelled.is_empty() { any } else { labelled };
                if !pool.is_empty() {
                    for p in bank.particles.iter_mut() {
                        if rng.uniform() >= ICON_INJECT {
                            continue;
                        }
                        let j = pool[rng.below(pool.len())];
                        let n = cands[j][rng.below(cands[j].len())];
                        let s = scale_guess(layout, rng);
                        let (ox, oy) = offset(&icons[j], s);
                        let (x, y) = at(n);
                        let (gx, gy) = rng.gauss2();
                        let jitter = ICON_TOL / 2.0 / layout.unit;
                        *p = Particle { x: x - ox + gx * jitter, y: y - oy + gy * jitter, s, w: p.w };
                    }
                }
            }
            for p in bank.particles.iter_mut() {
                let l = if boss_still { like_of(p, &|i| i.boss.is_empty()) } else { like(p) };
                p.w *= if confident { 0.5 + 0.5 * l } else { l };
            }
            normalise(&mut bank.particles);
            let ess = 1.0 / bank.particles.iter().map(|p| p.w * p.w).sum::<f64>();
            if ess < PARTICLES as f64 / 2.0 {
                resample(&mut bank.particles, rng);
            }
            // Remember where each icon put its exit when the belief agrees on which it
            // is. (Only for layouts carrying half the belief, as the review asked, made
            // the gold needles worse in the simulation: 4.8% wrong against 2.9%, since
            // the route then starts from the node, up to 100 units off.)
            for (icon, nodes) in icons.iter().zip(cands) {
                let mut votes: HashMap<usize, (f64, f64, f64)> = HashMap::new();
                for p in &bank.particles {
                    let (ox, oy) = offset(icon, p.s);
                    let q = (p.x + ox, p.y + oy);
                    let near = nodes.iter().copied().min_by(|&a, &b| {
                        let d = |n: usize| {
                            let (x, y) = at(n);
                            (q.0 - x).powi(2) + (q.1 - y).powi(2)
                        };
                        d(a).total_cmp(&d(b))
                    });
                    if let Some(n) = near {
                        let v = votes.entry(n).or_default();
                        (v.0, v.1, v.2) = (v.0 + p.w, v.1 + p.w * q.0, v.2 + p.w * q.1);
                    }
                }
                if let Some((&n, &(w, x, y))) = votes.iter().max_by(|a, b| a.1 .0.total_cmp(&b.1 .0)) {
                    let node = &layout.nodes[n];
                    let close = ((x / w - node.x).hypot(y / w - node.y)) * layout.unit <= ICON_TOL;
                    if w >= ICON_SURE && close {
                        learnt.push(((k, n), (x / w, y / w)));
                    }
                }
            }
        }
        // Blended with what was seen before.
        for (key, (x, y)) in learnt {
            let blended = exits_seen.get(&key).map_or((x, y), |&(ox, oy)| ((ox + x) / 2.0, (oy + y) / 2.0));
            exits_seen.insert(key, blended);
        }
        // Absolute evidence (gold allowed) only when the belief held before a new
        // sighting predicted it: an icon that reshuffles a diffuse belief, or the
        // same sighting again, confirms nothing.
        if (new_exit || (new && self.tuning.boss_confirms)) && confirmed >= ICON_CONFIRM {
            self.anchored = true;
        }
    }

    /// New checkpoint icons (minimap cells from the player; the recorder hands each once a
    /// part), with the exit icons it handed to `icons`: soft evidence for the layouts whose
    /// landmarks they lie by. One not next to an exit icon, not where the belief puts a
    /// counted one (`CHECKPOINT_SAME`), within the visit's `CHECKPOINT_MOST`, weighs each
    /// layout by how much of its belief puts it within `CHECKPOINT_TOL` of a landmark (floor
    /// `CHECKPOINT_FLOOR`, counted at `CHECKPOINT_WEIGHT`, all of them together at most
    /// `CHECKPOINT_CAP`). Never an anchor, never in gold's share, no particle moved.
    pub fn checkpoints(&mut self, points: &[(f64, f64)], exits: &[ExitIcon]) {
        self.apply();
        for &(dx, dy) in points {
            if self.checkpoints_counted.len() >= CHECKPOINT_MOST {
                return;
            }
            if exits.iter().any(|e| e.boss.is_empty() && (e.dx - dx).hypot(e.dy - dy) <= CHECKPOINT_BY_EXIT) {
                continue;
            }
            let posterior = self.posterior();
            let scale = self.scale;
            // Where each layout's belief puts it (file units, the weighted mean).
            let places: Vec<(f64, f64)> = self
                .area
                .layouts
                .iter()
                .zip(&self.banks)
                .map(|(layout, bank)| {
                    let (mut x, mut y) = (0.0, 0.0);
                    for p in &bank.particles {
                        let (ox, oy) = super::guide::from_screen((dx / (scale * p.s), dy / (scale * p.s)));
                        (x, y) = (x + p.w * (p.x + ox / layout.unit), y + p.w * (p.y + oy / layout.unit));
                    }
                    (x, y)
                })
                .collect();
            let again: f64 = (0..places.len())
                .filter(|&k| {
                    let unit = self.area.layouts[k].unit;
                    self.checkpoints_counted.iter().any(|c| (c[k].0 - places[k].0).hypot(c[k].1 - places[k].1) * unit <= CHECKPOINT_SAME)
                })
                .map(|k| posterior[k])
                .sum();
            if again >= 0.5 {
                continue;
            }
            self.checkpoints_counted.push(places);
            self.touched();
            for (layout, bank) in self.area.layouts.iter().zip(self.banks.iter_mut()) {
                let marks: Vec<(f64, f64)> = layout.nodes.iter().filter(|n| landmark(n)).map(|n| (n.x, n.y)).collect();
                if marks.is_empty() {
                    continue;
                }
                let tol = CHECKPOINT_TOL / layout.unit;
                let agree: f64 = bank
                    .particles
                    .iter()
                    .map(|p| {
                        let (x, y) = super::guide::from_screen((dx / (scale * p.s), dy / (scale * p.s)));
                        let q = (p.x + x / layout.unit, p.y + y / layout.unit);
                        let d2 = marks.iter().map(|m| (q.0 - m.0).powi(2) + (q.1 - m.1).powi(2)).fold(f64::INFINITY, f64::min);
                        p.w * (CHECKPOINT_FLOOR + (1.0 - CHECKPOINT_FLOOR) * (-d2 / (2.0 * tol * tol)).exp())
                    })
                    .sum();
                bank.checkpoints = (bank.checkpoints + CHECKPOINT_WEIGHT * agree.max(CHECKPOINT_FLOOR).ln()).max(-CHECKPOINT_CAP);
            }
        }
    }

    /// The area ids an exit label names: every area whose name it is (two areas may
    /// share a name), else the label itself.
    fn exit_ids<'s>(&'s self, label: &'s str) -> Vec<&'s str> {
        let mut ids: Vec<&str> = self
            .area
            .names
            .iter()
            .filter(|(id, name)| id.as_str() == label || name.eq_ignore_ascii_case(label))
            .map(|(id, _)| id.as_str())
            .collect();
        if ids.is_empty() {
            ids.push(label);
        }
        ids
    }

    /// The share of the belief that puts icon `icon` on one of the target's exits.
    fn icon_is_target(&self, icon: &ExitIcon, target: &NavTarget, posterior: &[f64]) -> f64 {
        if let (Some(to), Some(exit)) = (&icon.to, &target.exit) {
            return if self.exit_ids(to).contains(&exit.as_str()) { 1.0 } else { 0.0 };
        }
        let mut share = 0.0;
        for (k, (layout, bank)) in self.area.layouts.iter().zip(&self.banks).enumerate() {
            if posterior[k] < NEGLIGIBLE {
                continue;
            }
            let goals = layout.goals(target);
            let nodes = self.icon_nodes(layout, icon);
            let at = |n: usize| (layout.nodes[n].x, layout.nodes[n].y);
            let mut hit = 0.0;
            for p in &bank.particles {
                let (x, y) = self.position(layout, p);
                let (ox, oy) = super::guide::from_screen((icon.dx / (self.scale * p.s), icon.dy / (self.scale * p.s)));
                let q = (x + ox / layout.unit, y + oy / layout.unit);
                let near = nodes.iter().copied().map(|n| {
                    let (nx, ny) = at(n);
                    (n, ((q.0 - nx).powi(2) + (q.1 - ny).powi(2)).sqrt() * layout.unit)
                }).min_by(|a, b| a.1.total_cmp(&b.1));
                if near.is_some_and(|(n, d)| d <= 2.0 * icon.tolerance() && goals.contains(&n)) {
                    hit += p.w;
                }
            }
            share += posterior[k] * hit;
        }
        share
    }

    /// Data came in since the start: the memory changed.
    pub fn changed(&self) -> bool {
        self.changed
    }

    fn touched(&mut self) {
        self.changed = true;
        self.data_at = unix_now();
    }

    /// Movement in minimap cells, screen axes (x right, y down).
    pub fn moved(&mut self, dx: f64, dy: f64) {
        let (wx, wy) = super::guide::from_screen((dx / self.scale, dy / self.scale));
        let step = wx.hypot(wy);
        // A cell of registration jitter is not walking.
        let walking = dx.hypot(dy) >= JITTER;
        if dx.hypot(dy) > JUMPED {
            self.jumps += 1;
        }
        self.pending = (self.pending.0 + wx, self.pending.1 + wy);
        self.pending_sq += step * step;
        self.track = (self.track.0 + wx, self.track.1 + wy);
        let keep = (-step / HEADING_LENGTH).exp();
        self.heading = (self.heading.0 * keep + wx, self.heading.1 * keep + wy);
        if walking {
            self.walked += step;
            self.touched();
        }
        self.since_weigh += step;
        if self.since_weigh >= WEIGH_EVERY {
            self.apply();
            self.weigh();
            self.since_weigh = 0.0;
        }
    }

    /// No movement data for this long (seconds); infinite when unknown.
    pub fn lost(&mut self, seconds: f64) {
        self.apply();
        // Icons in hand are stale, and their places on the track mean nothing now.
        self.icons_now.clear();
        self.sightings.clear();
        *self.way.get_mut() = None;
        self.since_weigh = 0.0;
        let reach = (WALK_SPEED * seconds.max(0.0)).min(MAX_REACH);
        let heading = self.heading;
        let Navigator { area, banks, rng, .. } = self;
        for (layout, bank) in area.layouts.iter().zip(banks.iter_mut()) {
            if reach > 1.0 {
                let r = reach / layout.unit;
                for p in &mut bank.particles {
                    let near = layout.corridor((p.x, p.y)).1;
                    if let Some(j) = near.filter(|_| rng.uniform() < ALONG) {
                        // On along the corridors, mostly the way the player was going.
                        let t = layout.tangent(j);
                        let ahead = t.0 * heading.0 + t.1 * heading.1 >= 0.0;
                        let forward = if rng.uniform() < KEEP_HEADING { ahead } else { !ahead };
                        let d = r * (ALONG_MIN + (1.0 - ALONG_MIN) * rng.uniform());
                        let (x, y) = layout.drift(j, forward, d, &mut |n| rng.below(n));
                        let (gx, gy) = rng.gauss2();
                        (p.x, p.y) = (x + gx * 15.0 / layout.unit, y + gy * 15.0 / layout.unit);
                        continue;
                    }
                    // Somewhere within reach, near a corridor if a few tries find one.
                    for _ in 0..8 {
                        let (d, a) = (r * rng.uniform().sqrt(), std::f64::consts::TAU * rng.uniform());
                        let at = (p.x + d * a.cos(), p.y + d * a.sin());
                        if layout.corridor(at).0 < FLAT {
                            (p.x, p.y) = at;
                            break;
                        }
                    }
                }
            }
            if seconds >= LONG_LOSS {
                for p in &mut bank.particles {
                    if rng.uniform() < REDRAW {
                        *p = Particle { w: p.w, ..on_corridor(layout, rng) };
                    }
                }
            }
        }
    }

    /// A log anchor: the player is at a matching node now.
    pub fn seen(&mut self, seen: Seen) {
        self.apply();
        let goals: Vec<Vec<usize>> = self.area.layouts.iter().map(|l| anchor_nodes(l, seen)).collect();
        // Something no layout knows (another area's id, an unknown tag) says nothing.
        if goals.iter().all(Vec::is_empty) {
            return;
        }
        // Soft evidence: no particle is moved, it is not an anchor, a layout without
        // the node loses nothing.
        let soft = match seen {
            Seen::Left(_) => Some((ANCHOR_SIGMA, LEFT_FLOOR)),
            Seen::NearTag(_) | Seen::NearExit(_) => Some((NEAR_SIGMA, NEAR_FLOOR)),
            Seen::Exit(_) | Seen::Tag(_) => None,
        };
        if let Some((sigma, floor)) = soft {
            self.touched();
            for ((layout, bank), goals) in self.area.layouts.iter().zip(self.banks.iter_mut()).zip(&goals) {
                if goals.is_empty() {
                    continue;
                }
                let agree = agreement(layout, &bank.particles, goals, sigma / layout.unit);
                bank.evidence += (floor + (1.0 - floor) * agree).ln();
            }
            return;
        }
        let sigma_world = if matches!(seen, Seen::Tag(_)) { TAG_SIGMA } else { ANCHOR_SIGMA };
        self.anchored = true;
        self.since_weigh = 0.0;
        self.touched();
        let missing = self.tuning.missing;
        let Navigator { area, banks, rng, .. } = self;
        for ((layout, bank), goals) in area.layouts.iter().zip(banks.iter_mut()).zip(&goals) {
            if goals.is_empty() {
                bank.evidence += missing;
                continue;
            }
            // How much of the belief sits at each matching node.
            let sigma = sigma_world / layout.unit;
            let mut mass = vec![0.0; goals.len()];
            let mut agree = 0.0;
            for p in &bank.particles {
                let mut best = 0.0f64;
                for (g, &node) in goals.iter().enumerate() {
                    let n = &layout.nodes[node];
                    let d2 = (p.x - n.x).powi(2) + (p.y - n.y).powi(2);
                    let k = (-d2 / (2.0 * sigma * sigma)).exp();
                    mass[g] += p.w * k;
                    best = best.max(k);
                }
                agree += p.w * best;
            }
            bank.evidence += (ANCHOR_FLOOR + (1.0 - ANCHOR_FLOOR) * agree).ln();
            // Everyone to the matching nodes, by how much belief each had; a few stay
            // where they were in case the log means a waypoint or portal.
            let total: f64 = mass.iter().sum::<f64>() + ANCHOR_FLOOR;
            let spread = ANCHOR_SPREAD / layout.unit;
            for p in &mut bank.particles {
                if rng.uniform() < STRAY {
                    continue;
                }
                let mut pick = rng.uniform() * total;
                let mut g = rng.below(goals.len());
                for (i, m) in mass.iter().enumerate() {
                    if pick < *m {
                        g = i;
                        break;
                    }
                    pick -= m;
                }
                let n = &layout.nodes[goals[g]];
                let (gx, gy) = rng.gauss2();
                (p.x, p.y) = (n.x + gx * spread, n.y + gy * spread);
            }
            bank.particles.iter_mut().for_each(|p| p.w = 1.0);
            normalise(&mut bank.particles);
        }
    }

    /// Other constants (mutation tests).
    #[cfg(test)]
    pub fn tuned(mut self, tuning: Tuning) -> Self {
        self.tuning = tuning;
        self
    }

    /// Shows ways ahead (tests of their live behaviour).
    #[cfg(test)]
    pub fn map_ahead_live(mut self) -> Self {
        self.map_ahead = true;
        self
    }

    /// Treats a made-up area as measured.
    #[cfg(test)]
    pub fn trusted(mut self) -> Self {
        self.trusted = true;
        self
    }

    /// World units walked with tracking, and whether a log anchor was seen.
    #[cfg(test)]
    pub fn progress(&self) -> (f64, bool) {
        (self.walked, self.anchored)
    }

    /// Layout probabilities, in layout order.
    fn posterior(&self) -> Vec<f64> {
        self.posterior_with(true, true)
    }

    /// The layout share gold needs: without what bosses' icons (`BOSS_GOLD`) and
    /// checkpoints added.
    fn gold_posterior(&self) -> Vec<f64> {
        self.posterior_with(self.tuning.boss_gold, false)
    }

    fn posterior_with(&self, bosses: bool, checkpoints: bool) -> Vec<f64> {
        let soft = |b: &Bank| if bosses { b.boss } else { 0.0 } + if checkpoints { b.checkpoints } else { 0.0 };
        let logs: Vec<f64> = self.banks.iter().map(|b| b.prior + b.path + b.evidence + b.named + soft(b)).collect();
        let top = logs.iter().copied().fold(f64::MIN, f64::max);
        let raw: Vec<f64> = logs.iter().map(|l| (l - top).exp()).collect();
        let sum: f64 = raw.iter().sum();
        raw.into_iter().map(|p| p / sum).collect()
    }

    /// The layout ids, most likely first, with their probabilities.
    #[cfg(test)]
    pub fn ranking(&self) -> Vec<(&str, f64)> {
        let mut out: Vec<(&str, f64)> =
            self.area.layouts.iter().map(|l| l.id.as_str()).zip(self.posterior()).collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
        out
    }

    /// The needle to the first of `targets` that a layout of this zone has,
    /// without the recorded map (tests and the simulation; the app lends it).
    #[cfg(test)]
    pub fn guide(&self, targets: &[NavTarget]) -> Option<GuideView> {
        self.guide_on(targets, None)
    }

    /// The needle, with the recorded map of the part when the recorder lends it:
    /// where the target's place on it is known, the needle follows the way there.
    pub fn guide_on(&self, targets: &[NavTarget], place: Option<&Place>) -> Option<GuideView> {
        let target = targets.iter().find(|t| self.area.layouts.iter().any(|l| !l.goals(t).is_empty()))?;
        let posterior = self.posterior();
        let (top, p_top) = posterior.iter().enumerate().fold((0, 0.0), |b, (i, &p)| if p > b.1 { (i, p) } else { b });
        let mapped = place.and_then(|place| self.on_the_map(target, &posterior, place));
        // A way ahead in shadow: worked out and kept for the flight recorder; the
        // needle is the one without the map.
        let (live, shadow) = match mapped {
            Some((view, note, back)) if back || self.map_ahead => (Some((view, note)), None),
            Some((view, note, _)) => {
                let shadow = Shadow {
                    degrees: view.degrees,
                    length: view.distance,
                    ahead: note.ahead,
                    unrecorded: note.unrecorded,
                    gold: view.source == "estimated",
                };
                (None, Some(shadow))
            }
            None => (None, None),
        };
        self.last_way.set(live.as_ref().map(|m| m.1));
        self.last_shadow.set(shadow);
        if let Some((view, _)) = live {
            return Some(view);
        }
        let mut cache = self.guidance.borrow_mut();
        let (mut vx, mut vy, mut togo, mut weight) = (0.0, 0.0, 0.0, 0.0);
        let mut bins = [0.0f64; BINS];
        for (k, (layout, bank)) in self.area.layouts.iter().zip(&self.banks).enumerate() {
            let pk = posterior[k];
            if pk < NEGLIGIBLE {
                continue;
            }
            let goals = layout.goals(target);
            if goals.is_empty() {
                continue;
            }
            let guidance = cache.entry((k, goals.clone())).or_insert_with(|| layout.guidance(&goals));
            for p in &bank.particles {
                let at = self.position(layout, p);
                let (off, near) = layout.corridor(at);
                let Some(j) = near else { continue };
                let rest = guidance.togo[j] as f64;
                if !rest.is_finite() {
                    continue;
                }
                let (ax, ay) = guidance.ahead[j];
                let (sx, sy) = super::guide::to_screen((ax as f64 - at.0, ay as f64 - at.1));
                let len = sx.hypot(sy);
                if len * layout.unit > 1.0 {
                    vx += pk * p.w * sx / len;
                    vy += pk * p.w * sy / len;
                    let bin = ((sy.atan2(sx) / std::f64::consts::TAU).rem_euclid(1.0) * BINS as f64) as usize % BINS;
                    bins[bin] += pk * p.w;
                }
                togo += pk * p.w * (rest + off.min(FAR));
                weight += pk * p.w;
            }
        }
        let confidence = vx.hypot(vy);
        let t = self.tuning;
        // Gold needs one layout to carry the belief, which only anchors and icons can
        // do (the walked line alone gives at most PATH_MOST): with the belief spread
        // over layouts that happen to point the same way the needle was often wrong.
        // Gold judges the layout by exits and anchors only: a boss's icon may have raised it.
        let p_gold = self.gold_posterior()[top];
        let source = if confidence >= t.gold && p_gold >= t.gold_layout && (self.walked >= t.gold_walk || self.anchored) {
            "estimated"
        } else if confidence >= ROUGH {
            "rough"
        } else {
            "unconfirmed"
        };
        let source = self.cap_source(source, &posterior);
        let route_degrees = mode(&bins).or_else(|| screen_bearing(vx, vy));
        // The target's own exit in view: the way there from where the icon puts the
        // player, not through the belief's mix of positions.
        let in_view = self
            .icons_now
            .iter()
            .map(|(icon, read_twice)| (icon, *read_twice, self.icon_is_target(icon, target, &posterior)))
            // Of equally good icons the nearest, then by place: the icons' order
            // changes from frame to frame.
            .min_by(|a, b| {
                let key = |i: &ExitIcon| (i.dx.hypot(i.dy), i.dx, i.dy);
                let (ka, kb) = (key(a.0), key(b.0));
                (b.2 > a.2 + 1e-9).cmp(&(a.2 > b.2 + 1e-9)).then(ka.0.total_cmp(&kb.0)).then(ka.1.total_cmp(&kb.1)).then(ka.2.total_cmp(&kb.2))
            });
        if let Some((icon, read_twice, share)) = in_view.filter(|(_, _, share)| *share >= ICON_SURE) {
            let (ox, oy) = super::guide::from_screen((icon.dx / self.scale, icon.dy / self.scale));
            let straight = ox.hypot(oy).max(1.0);
            let (mut rx, mut ry, mut ratio, mut sum) = (0.0, 0.0, 0.0, 0.0);
            for (k, layout) in self.area.layouts.iter().enumerate() {
                let pk = posterior[k];
                let goals = layout.goals(target);
                if pk < NEGLIGIBLE || goals.is_empty() {
                    continue;
                }
                let nodes: Vec<usize> = self.icon_nodes(layout, icon).into_iter().filter(|n| goals.contains(n)).collect();
                let Some(&n) = nodes.first().or(goals.first()) else { continue };
                // Where the exit was seen, else its node; the player is the icon's offset from it.
                let node = (layout.nodes[n].x, layout.nodes[n].y);
                let exit = self
                    .exits_seen
                    .get(&(k, n))
                    .copied()
                    .filter(|e| (e.0 - node.0).hypot(e.1 - node.1) * layout.unit <= ICON_TOL)
                    .unwrap_or(node);
                let implied = (exit.0 - ox / layout.unit, exit.1 - oy / layout.unit);
                let (off, near) = layout.corridor(implied);
                let Some(j) = near else { continue };
                let guidance = cache.entry((k, goals.clone())).or_insert_with(|| layout.guidance(&goals));
                let (ax, ay) = guidance.ahead[j];
                let (sx, sy) = super::guide::to_screen((ax as f64 - implied.0, ay as f64 - implied.1));
                let len = sx.hypot(sy).max(1e-9);
                (rx, ry) = (rx + pk * sx / len, ry + pk * sy / len);
                ratio += pk * (guidance.togo[j] as f64 + off.min(FAR)) / straight;
                sum += pk;
            }
            let ratio = if sum > 0.0 { ratio / sum } else { 1.0 };
            let pointing = at_icon(self.at_icon.get(), ratio);
            self.at_icon.set(pointing);
            // Gold when the label says it is the target's exit (read the same in two
            // scans, and not against a sure belief), or when one layout carries the
            // belief (an unlabelled icon could be another exit of a layout not ruled
            // out); within ICON_GOLD_WITHIN.
            let unlabelled = ExitIcon { to: None, ..icon.clone() };
            let against = icon.to.is_some() && p_top >= GOLD_LAYOUT && self.icon_is_target(&unlabelled, target, &posterior) < 0.5;
            let named = icon.to.is_some() && read_twice && !against;
            // Gold only while pointing at the icon: along the route from a position the
            // icon implies from the exit's node (up to 100 units off) it was wrong
            // more often (4.4% of gold, one seed).
            let near = icon.dx.hypot(icon.dy) <= ICON_GOLD_WITHIN && pointing;
            let sure = if near && !against && (named || p_gold >= GOLD_LAYOUT) { "estimated" } else { "rough" };
            return Some(GuideView {
                variant: if p_top >= SURE { self.area.layouts[top].id.clone() } else { String::new() },
                target_kind: if target.exit.is_some() { "entrance".into() } else { target.tag.clone().unwrap_or_default() },
                target: target.label.clone(),
                degrees: if pointing { screen_bearing(icon.dx, icon.dy) } else { screen_bearing(rx, ry) },
                // A labelled icon of the target in view is direct evidence, not layout
                // inference: gold in every area. An unlabelled one leans on the belief
                // and keeps its cap (rough in unmeasured areas, unconfirmed in dungeons).
                source: if named && near { "estimated".into() } else { self.cap_source(sure, &posterior).into() },
                distance: icon.dx.hypot(icon.dy) as f32,
                confidence: share.min(1.0) as f32,
                measured: false,
                turned: false,
            });
        }
        Some(GuideView {
            variant: if p_top >= SURE { self.area.layouts[top].id.clone() } else { String::new() },
            target_kind: if target.exit.is_some() { "entrance".into() } else { target.tag.clone().unwrap_or_default() },
            target: target.label.clone(),
            degrees: route_degrees,
            source: source.into(),
            distance: if weight > 0.0 { (togo / weight * self.scale) as f32 } else { 0.0 },
            confidence: confidence.min(1.0) as f32,
            measured: false,
            turned: false,
        })
    }

    /// The last needle followed a way through the recorded map.
    #[cfg(test)]
    pub fn on_map(&self) -> bool {
        self.last_way.get().is_some()
    }

    /// What the last needle took from the recorded map, if it followed a way.
    pub fn last_way(&self) -> Option<WayNote> {
        self.last_way.get()
    }

    /// What the last needle's way ahead would have shown, when ways ahead are in shadow.
    pub fn last_shadow(&self) -> Option<Shadow> {
        self.last_shadow.get()
    }

    /// Where the target is on the part's map, when known: its icon in view (as
    /// `guide` finds it), else the place the player came in by it (when that was by
    /// the exit), else a sighting of it in this part (sightings are cleared at every
    /// loss, and a new part starts with one). Then the way there through the recorded map, if
    /// there is one and it is not absurdly long: the needle looks `route::LOOK`
    /// along it and the distance is its length (the flag: the way back to the
    /// arrival). A way that is about the straight
    /// line and lies mostly over unrecorded ground is not taken, and none over
    /// unrecorded ground is gold. None leaves the needle to `guide`.
    fn on_the_map(&self, target: &NavTarget, posterior: &[f64], place: &Place) -> Option<(GuideView, WayNote, bool)> {
        let (top, p_top) = posterior.iter().enumerate().fold((0, 0.0), |b, (i, &p)| if p > b.1 { (i, p) } else { b });
        // Of the icons the belief (or the label) says are the target's exit, the
        // most certain, then the nearest, then by place.
        let best = |list: Vec<(ExitIcon, bool)>| {
            list.into_iter()
                .map(|(icon, twice)| {
                    let share = self.icon_is_target(&icon, target, posterior);
                    (icon, twice, share)
                })
                .filter(|(_, _, share)| *share >= ICON_SURE)
                .min_by(|a, b| {
                    let key = |i: &ExitIcon| (i.dx.hypot(i.dy), i.dx, i.dy);
                    let (ka, kb) = (key(&a.0), key(&b.0));
                    (b.2 > a.2 + 1e-9).cmp(&(a.2 > b.2 + 1e-9)).then(ka.0.total_cmp(&kb.0)).then(ka.1.total_cmp(&kb.1)).then(ka.2.total_cmp(&kb.2))
                })
        };
        let sighted = || {
            let list = self
                .sightings
                .iter()
                .filter(|s| s.jumps == self.jumps)
                .map(|s| {
                    // The inverse of how `icons` placed it on the track.
                    let (sx, sy) = to_screen((s.at.0 - self.track.0, s.at.1 - self.track.1));
                    (ExitIcon { dx: sx * self.scale, dy: sy * self.scale, to: s.name.clone(), boss: s.boss.clone() }, s.scans >= 2)
                })
                .collect();
            best(list)
        };
        // The exit the player came in by, where they appeared, when that was by the
        // exit: soon after the zone entry, no waypoint by it. An exit icon by it (unread,
        // or read as that zone) makes it sure (`read_twice` carries that here).
        let arrived = || {
            let exit = target.exit.as_deref()?;
            let c = place.came_in.filter(|c| c.from == exit && !town(c.from) && c.delay <= ARRIVAL_DELAY && !c.waypoint)?;
            let sure = c.exits.iter().any(|l| l.as_deref().is_none_or(|l| self.exit_ids(l).contains(&exit)));
            let (dx, dy) = ((c.at.0 - place.player.0) as f64, (c.at.1 - place.player.1) as f64);
            Some((ExitIcon { dx, dy, to: None, boss: vec![] }, sure, 1.0))
        };
        // The icon in view, else the arrival point (on the map itself), else a
        // sighting (placed by dead reckoning).
        let (icon, read_twice, share, arrival) = match best(self.icons_now.clone()) {
            Some((icon, twice, share)) => (icon, twice, share, false),
            None => match arrived() {
                Some((icon, twice, share)) => (icon, twice, share, true),
                None => {
                    let (icon, twice, share) = sighted()?;
                    (icon, twice, share, false)
                }
            },
        };
        let from = (place.player.0 as f64, place.player.1 as f64);
        let to = (from.0 + icon.dx, from.1 + icon.dy);
        let straight = icon.dx.hypot(icon.dy);
        let way = {
            let kept = self.way.borrow();
            let keep = kept.as_ref().filter(|(label, _)| *label == target.label).map_or(&[][..], |(_, cells)| cells.as_slice());
            route::way(&place.map, from, to, keep)?
        };
        if way.length > (ABSURD * straight).max(straight + ABSURD_SLACK) {
            return None;
        }
        // The straight line across ground nobody has recorded: the map changes
        // nothing and knows nothing there.
        if way.straight() && way.recorded < RECORDED {
            return None;
        }
        *self.way.borrow_mut() = Some((target.label.clone(), way.cells.clone()));
        let source = if arrival {
            // The way back to where the player came in by the exit: walked and recorded;
            // sure with the exit's icon seen there.
            if read_twice { "estimated" } else { "rough" }
        } else {
            // As at the icon in `guide`, without "the route is about straight": the
            // way through the recorded map is the route.
            let unlabelled = ExitIcon { to: None, ..icon.clone() };
            let against = icon.to.is_some() && p_top >= GOLD_LAYOUT && self.icon_is_target(&unlabelled, target, posterior) < 0.5;
            let named = icon.to.is_some() && read_twice && !against;
            let near = straight <= ICON_GOLD_WITHIN;
            let p_gold = self.gold_posterior()[top];
            let sure = if near && !against && (named || p_gold >= GOLD_LAYOUT) { "estimated" } else { "rough" };
            if named && near { "estimated" } else { self.cap_source(sure, posterior) }
        };
        // Mostly across fog, or a long way round: a guess, however the target is known.
        let long = way.length > GOLD_RATIO * straight && !(arrival && way.walked >= WALKED_GOLD);
        let source = if source == "estimated" && (way.recorded < RECORDED || long) { "rough" } else { source };
        let note = WayNote { ahead: (way.ahead.0 - from.0, way.ahead.1 - from.1), unrecorded: 1.0 - way.recorded };
        let view = GuideView {
            variant: if p_top >= SURE { self.area.layouts[top].id.clone() } else { String::new() },
            target_kind: if target.exit.is_some() { "entrance".into() } else { target.tag.clone().unwrap_or_default() },
            target: target.label.clone(),
            degrees: screen_bearing(way.ahead.0 - from.0, way.ahead.1 - from.1),
            source: source.into(),
            distance: way.length as f32,
            confidence: share.min(1.0) as f32,
            measured: false,
            turned: false,
        };
        Some((view, note, arrival))
    }

    /// Gold only where it was measured: at most rough in other areas, unconfirmed
    /// wherever a dungeon layout carries belief.
    fn cap_source(&self, source: &'static str, posterior: &[f64]) -> &'static str {
        let dungeon = self.area.layouts.iter().zip(posterior).any(|(l, &p)| l.dungeon && p >= NEGLIGIBLE);
        if dungeon {
            "unconfirmed"
        } else if source == "estimated" && !self.trusted {
            "rough"
        } else {
            source
        }
    }

    /// What is worth keeping for this instance.
    pub fn memory(&self) -> Memory {
        let posterior = self.posterior();
        let mut positions = BTreeMap::new();
        for (k, (layout, bank)) in self.area.layouts.iter().zip(&self.banks).enumerate() {
            if posterior[k] < KEEP_FROM {
                continue;
            }
            // Equally weighted picks of the particles (systematic, no randomness).
            let mut kept = Vec::with_capacity(KEEP);
            let (mut sum, mut i) = (bank.particles[0].w, 0);
            for m in 0..KEEP {
                let u = (m as f64 + 0.5) / KEEP as f64;
                while sum < u && i + 1 < bank.particles.len() {
                    i += 1;
                    sum += bank.particles[i].w;
                }
                let p = &bank.particles[i];
                let (x, y) = self.position(layout, p);
                kept.push([(x * 10.0).round() as f32 / 10.0, (y * 10.0).round() as f32 / 10.0, (p.s * 1000.0).round() as f32 / 1000.0]);
            }
            positions.insert(layout.id.clone(), kept);
        }
        let ids = || self.area.layouts.iter().map(|l| l.id.clone());
        let mut exits: BTreeMap<String, BTreeMap<usize, [f32; 2]>> = BTreeMap::new();
        for (&(k, n), &(x, y)) in &self.exits_seen {
            exits.entry(self.area.layouts[k].id.clone()).or_default().insert(n, [x as f32, y as f32]);
        }
        Memory {
            version: MEMORY_VERSION,
            exits,
            layouts: ids().zip(posterior).collect(),
            path: ids().zip(self.banks.iter().map(|b| b.path)).collect(),
            anchors: ids().zip(self.banks.iter().map(|b| b.evidence)).collect(),
            named: ids().zip(self.banks.iter().map(|b| b.named)).collect(),
            walked: self.walked_before + self.walked,
            positions,
            at: self.data_at,
        }
    }

    /// A particle's position including movement not yet applied.
    fn position(&self, layout: &Layout, p: &Particle) -> (f64, f64) {
        let k = 1.0 / (layout.unit * p.s);
        (p.x + self.pending.0 * k, p.y + self.pending.1 * k)
    }

    /// Moves every particle by the pending movement, with noise.
    fn apply(&mut self) {
        if self.pending_sq <= 0.0 {
            return;
        }
        let (dx, dy) = self.pending;
        let noise = NOISE * self.pending_sq.sqrt() + NOISE_MIN;
        let Navigator { area, banks, rng, .. } = self;
        for (layout, bank) in area.layouts.iter().zip(banks.iter_mut()) {
            let (limit_lo, limit_hi) = scale_limits(layout);
            for p in &mut bank.particles {
                let k = 1.0 / (layout.unit * p.s);
                let (gx, gy) = rng.gauss2();
                p.x += dx * k + gx * noise / layout.unit;
                p.y += dy * k + gy * noise / layout.unit;
                let (gs, _) = rng.gauss2();
                p.s = (p.s * (1.0 + gs * SCALE_DRIFT)).clamp(limit_lo, limit_hi);
            }
        }
        self.pending = (0.0, 0.0);
        self.pending_sq = 0.0;
    }

    /// Weights the particles by their closeness to a corridor.
    fn weigh(&mut self) {
        let flat = self.tuning.flat;
        let Navigator { area, banks, rng, .. } = self;
        for (layout, bank) in area.layouts.iter().zip(banks.iter_mut()) {
            let mut fit = 0.0;
            for p in &mut bank.particles {
                let like = closeness(layout.corridor((p.x, p.y)).0, flat);
                fit += p.w * like;
                p.w *= like;
            }
            bank.path += TEMPER * fit.ln();
            normalise(&mut bank.particles);
            let ess = 1.0 / bank.particles.iter().map(|p| p.w * p.w).sum::<f64>();
            if ess < PARTICLES as f64 / 2.0 {
                resample(&mut bank.particles, rng);
            }
            // Nothing fits: the belief is probably somewhere else; try a few fresh guesses.
            if fit < LOST_FIT {
                for p in &mut bank.particles {
                    if rng.uniform() < RESEED {
                        *p = Particle { w: p.w, ..on_corridor(layout, rng) };
                    }
                }
            }
        }
        // Only the lead counts, and only up to the cap.
        let best = banks.iter().map(|b| b.path).fold(f64::MIN, f64::max);
        let cap = path_cap(banks.len());
        for bank in banks.iter_mut() {
            bank.path = (bank.path - best).max(-cap);
        }
    }
}

/// Directions are binned this finely to find where most of the belief points.
const BINS: usize = 72;

/// The direction most of the belief points (screen degrees): the bin with the
/// most weight within ±45°, refined by the weighted mean of those bins. With
/// layouts pointing different ways the mean of all directions falls between
/// them; this follows the largest group.
fn mode(bins: &[f64; BINS]) -> Option<f32> {
    let width = BINS / 8;
    let around = |b: usize| (0..=2 * width).map(|i| bins[(b + BINS - width + i) % BINS]).sum::<f64>();
    let best = (0..BINS).max_by(|&a, &b| around(a).total_cmp(&around(b)))?;
    if around(best) <= 0.0 {
        return None;
    }
    let (mut x, mut y) = (0.0, 0.0);
    for i in 0..=2 * width {
        let b = (best + BINS - width + i) % BINS;
        let a = (b as f64 + 0.5) / BINS as f64 * std::f64::consts::TAU;
        (x, y) = (x + bins[b] * a.cos(), y + bins[b] * a.sin());
    }
    screen_bearing(x, y)
}

/// How well a point `d` world units from the nearest corridor fits: a steady
/// sideways offset inside the corridor costs nothing, so weighing often does not
/// count the same offset again and again; far off costs the floor.
fn closeness(d: f64, flat: f64) -> f64 {
    let out = (d - flat).max(0.0);
    FLOOR + (1.0 - FLOOR) * (-out * out / (2.0 * SIGMA * SIGMA)).exp()
}

fn scale_limits(layout: &Layout) -> (f64, f64) {
    if layout.dungeon { (1.0 / DUNGEON_SCALE, DUNGEON_SCALE) } else { (1.0 - SCALE_LIMIT, 1.0 + SCALE_LIMIT) }
}

fn scale_guess(layout: &Layout, rng: &mut Rng) -> f64 {
    if layout.dungeon {
        // Log-uniform over the whole range: direction still works, distance does not.
        DUNGEON_SCALE.powf(2.0 * rng.uniform() - 1.0)
    } else {
        let (lo, hi) = scale_limits(layout);
        (1.0 + rng.gauss2().0 * SCALE_SPREAD).clamp(lo, hi)
    }
}

/// A particle somewhere along the corridors.
fn on_corridor(layout: &Layout, rng: &mut Rng) -> Particle {
    let sample = layout.samples[rng.below(layout.samples.len())];
    let (gx, gy) = rng.gauss2();
    let jitter = 15.0 / layout.unit;
    Particle { x: sample.at.0 + gx * jitter, y: sample.at.1 + gy * jitter, s: scale_guess(layout, rng), w: 1.0 }
}

/// The nodes an anchor stands for in one layout.
fn anchor_nodes(layout: &Layout, seen: Seen) -> Vec<usize> {
    (0..layout.nodes.len())
        .filter(|&i| match seen {
            Seen::Exit(area) | Seen::Left(area) | Seen::NearExit(area) => layout.nodes[i].exit.as_deref() == Some(area),
            Seen::Tag(tag) | Seen::NearTag(tag) => !tag.is_empty() && layout.nodes[i].has_tag(tag),
        })
        .collect()
}

/// The weighted share of particles near one of `nodes` (a Gaussian of `sigma`, file units).
fn agreement(layout: &Layout, particles: &[Particle], nodes: &[usize], sigma: f64) -> f64 {
    particles
        .iter()
        .map(|p| {
            let best = nodes
                .iter()
                .map(|&n| (p.x - layout.nodes[n].x).powi(2) + (p.y - layout.nodes[n].y).powi(2))
                .fold(f64::MAX, f64::min);
            p.w * (-best / (2.0 * sigma * sigma)).exp()
        })
        .sum()
}

/// The first particles of a layout.
fn start(layout: &Layout, arrival: Arrival, rng: &mut Rng) -> Vec<Particle> {
    let mut nodes: Vec<usize> = Vec::new();
    if let Arrival::From(area) = arrival {
        nodes = anchor_nodes(layout, Seen::Exit(area));
        if nodes.is_empty() || area.is_empty() {
            nodes = (0..layout.nodes.len()).filter(|&i| layout.nodes[i].start).collect();
        }
    }
    let spread = SPREAD / layout.unit;
    let stray = match arrival {
        Arrival::From(area) if area.ends_with("_town") || area.ends_with("_Town") => STRAY_TOWN,
        _ => STRAY,
    };
    (0..PARTICLES)
        .map(|_| {
            if nodes.is_empty() || rng.uniform() < stray {
                return on_corridor(layout, rng);
            }
            let n = &layout.nodes[nodes[rng.below(nodes.len())]];
            let (gx, gy) = rng.gauss2();
            Particle { x: n.x + gx * spread, y: n.y + gy * spread, s: scale_guess(layout, rng), w: 1.0 }
        })
        .collect()
}

fn normalise(particles: &mut [Particle]) {
    let sum: f64 = particles.iter().map(|p| p.w).sum();
    if sum > 0.0 && sum.is_finite() {
        particles.iter_mut().for_each(|p| p.w /= sum);
    } else {
        let w = 1.0 / particles.len() as f64;
        particles.iter_mut().for_each(|p| p.w = w);
    }
}

/// Systematic resampling: equal weights, copies by weight.
fn resample(particles: &mut Vec<Particle>, rng: &mut Rng) {
    let n = particles.len();
    let step = 1.0 / n as f64;
    let mut u = rng.uniform() * step;
    let (mut sum, mut i) = (particles[0].w, 0);
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        while sum < u && i + 1 < n {
            i += 1;
            sum += particles[i].w;
        }
        out.push(Particle { w: step, ..particles[i] });
        u += step;
    }
    *particles = out;
}

#[cfg(test)]
mod tests {
    use super::super::graph::{self, SCALE};
    use super::super::nav_sim::{walk, Event, Options};
    use super::*;

    /// A layout told apart early on a walk from the town exit to The Grelwood
    /// (some walks stay ambiguous to the end: `nav_sim::tests::ambiguity`).
    const WALKED: &str = "ClearFell_N_E_S_League";

    fn exit(id: &str, label: &str) -> NavTarget {
        NavTarget { label: label.into(), exit: Some(id.into()), ..Default::default() }
    }

    /// A walk on `layout` from the exit to `from` towards the exit to `to`, without losses.
    fn walk_on(area: &Area, layout: &str, from: &str, to: &str, seed: u64) -> Vec<Event> {
        let l = area.layouts.iter().find(|l| l.id == layout).unwrap();
        let goals = l.goals(&exit(to, to));
        let start = l.goals(&exit(from, from))[0];
        walk(l, start, &goals, seed, &Options { losses: false, ..Default::default() }).unwrap()
    }

    fn feed(nav: &mut Navigator, events: &[Event]) {
        for e in events {
            match *e {
                Event::Moved { dx, dy, .. } => nav.moved(dx, dy),
                Event::Lost(s) => nav.lost(s),
            }
        }
    }

    #[test]
    fn particles_far_from_every_corridor_cost_the_floor_and_are_reseeded() {
        let area = graph::load("G1_2");
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 2);
        for bank in &mut nav.banks {
            bank.particles.iter_mut().for_each(|p| (p.x, p.y) = (p.x - 5000.0, p.y + 5000.0));
        }
        // Far off costs the floor, not a capped distance.
        let far = nav.banks[0].particles[0];
        assert_eq!(closeness(area.layouts[0].corridor((far.x, far.y)).0, FLAT), FLOOR);
        assert_eq!(closeness(10.0, FLAT), 1.0);
        nav.weigh();
        // A share of every layout's particles is redrawn near its corridors.
        let near: usize = area.layouts.iter().zip(&nav.banks)
            .map(|(layout, bank)| bank.particles.iter().filter(|p| layout.corridor((p.x, p.y)).0 <= FLAT).count())
            .sum();
        let expected = RESEED * (PARTICLES * area.layouts.len()) as f64;
        assert!(near as f64 >= 0.8 * expected, "{near} of {expected}");
    }

    #[test]
    fn no_walk_yet_gives_only_a_gray_best_guess() {
        // Clearfell's layouts put Mud Burrow on every side.
        let area = graph::load("G1_2");
        let nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 1);
        let view = nav.guide(&[exit("G1_3", "Mud Burrow")]).unwrap();
        assert_eq!((view.target.as_str(), view.target_kind.as_str()), ("Mud Burrow", "entrance"));
        assert_eq!(view.source, "unconfirmed");
        assert!(view.degrees.is_some());
        assert!(view.variant.is_empty());
    }

    #[test]
    fn a_walk_along_one_layout_picks_it_and_the_needle_follows_its_corridors() {
        let area = graph::load("G1_2");
        let events = walk_on(&area, WALKED, "G1_town", "G1_4", 5);
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 5);
        let cut = events.len() * 3 / 4;
        feed(&mut nav, &events[..cut]);
        let ranking = nav.ranking();
        assert_eq!(ranking[0].0, WALKED, "{:?}", &ranking[..3]);
        // The walked line alone claims no certainty.
        assert!(ranking[0].1 <= PATH_MOST + 1e-9, "{:?}", &ranking[..3]);
        let view = nav.guide(&[exit("G1_4", "The Grelwood")]).unwrap();
        let Some(Event::Moved { truth: Some(truth), .. }) = events[..cut].iter().rev().find(|e| matches!(e, Event::Moved { truth: Some(_), .. })) else { panic!() };
        let off = super::super::guide::angle_between(view.degrees.unwrap(), *truth);
        assert!(off < 45.0, "{view:?} truth {truth}");
        assert_ne!(view.source, "unconfirmed", "{view:?}");
        assert!(view.distance > 0.0);
    }

    #[test]
    fn a_target_no_layout_has_gives_no_needle_and_the_first_resolvable_one_wins() {
        let area = graph::load("G1_2");
        let nav = Navigator::new(&area, Arrival::Unknown, None, SCALE, 1);
        let nowhere = exit("G9_9", "Nowhere");
        assert!(nav.guide(&[nowhere.clone()]).is_none());
        assert!(nav.guide(&[]).is_none());
        let tag = NavTarget { label: "Beira".into(), tag: Some("boss".into()), ..Default::default() };
        let view = nav.guide(&[nowhere, tag]).unwrap();
        assert_eq!((view.target.as_str(), view.target_kind.as_str()), ("Beira", "boss"));
    }

    #[test]
    fn an_anchor_moves_the_belief_and_a_layout_without_the_node_loses() {
        let json = r#"{"connections": ["A", "B"], "topologies": [
            {"id": "both", "nodes": [{"x": 0, "y": 0, "exit": "A"}, {"x": 600, "y": 0, "exit": "B"}], "edges": [{"a": 0, "b": 1}]},
            {"id": "one", "nodes": [{"x": 0, "y": 0, "exit": "A"}, {"x": 600, "y": 0}], "edges": [{"a": 0, "b": 1}]}]}"#;
        let area = Area::parse("T", json).unwrap();
        let mut nav = Navigator::new(&area, Arrival::Unknown, None, SCALE, 3).trusted();
        nav.seen(Seen::Exit("B"));
        let ranking = nav.ranking();
        assert_eq!(ranking[0].0, "both");
        assert!(ranking[0].1 > 0.99, "{ranking:?}");
        // Standing at B, A lies west: screen down-left (135°).
        let view = nav.guide(&[exit("A", "A")]).unwrap();
        let off = super::super::guide::angle_between(view.degrees.unwrap(), 135.0);
        assert!(off < 10.0, "{view:?}");
        assert_eq!(view.source, "estimated", "an anchor confirms: {view:?}");
        // Something no layout knows changes nothing.
        let before = nav.ranking()[0].1;
        nav.seen(Seen::Tag("nothing"));
        assert_eq!(nav.ranking()[0].1, before);
    }

    #[test]
    fn gold_only_where_measured() {
        let both = r#"{"connections": ["A", "B"], "topologies": [
            {"id": "both", "format": "FORMAT", "roomUnitSize": 9, "nodes": [{"x": 0, "y": 0, "exit": "A"}, {"x": 600, "y": 0, "exit": "B"}], "edges": [{"a": 0, "b": 1}]}]}"#;
        let source = |format: &str, trusted: bool| {
            let area = Area::parse("X", &both.replace("FORMAT", format)).unwrap();
            let mut nav = Navigator::new(&area, Arrival::Unknown, None, SCALE, 3);
            if trusted {
                nav = nav.trusted();
            }
            nav.seen(Seen::Exit("B"));
            nav.guide(&[exit("A", "A")]).unwrap().source
        };
        assert_eq!(source("tgr", true), "estimated");
        assert_eq!(source("tgr", false), "rough", "an area without a replayed recording");
        assert_eq!(source("dgr", false), "unconfirmed", "a dungeon");
        assert_eq!(source("dgr", true), "unconfirmed", "a dungeon, until its scale is measured");
        // The target's own labelled icon in view, the way there straight: gold anywhere.
        let near = r#"{"connections": ["A", "B"], "topologies": [
            {"id": "near", "format": "FORMAT", "roomUnitSize": 2, "nodes": [{"x": 0, "y": 0, "exit": "A"}, {"x": 180, "y": 0, "exit": "B"}], "edges": [{"a": 0, "b": 1}]}]}"#;
        for format in ["tgr", "dgr"] {
            let area = Area::parse("X", &near.replace("FORMAT", format)).unwrap();
            let l = &area.layouts[0];
            let (sx, sy) = super::super::guide::to_screen(((l.nodes[0].x - l.nodes[1].x) * l.unit, 0.0));
            let icon = |to: Option<&str>| ExitIcon { dx: sx * SCALE, dy: sy * SCALE, to: to.map(String::from), boss: vec![] };
            let mut nav = Navigator::new(&area, Arrival::From("B"), None, SCALE, 3);
            nav.icons(&[icon(Some("A"))]);
            // One scan's label is not enough for gold; the same name again is.
            assert_ne!(nav.guide(&[exit("A", "A")]).unwrap().source, "estimated", "{format}");
            nav.icons(&[icon(Some("A"))]);
            let view = nav.guide(&[exit("A", "A")]).unwrap();
            assert_eq!(view.source, "estimated", "{format}: {view:?}");
            // Unlabelled, the same icon is inferred: capped.
            let mut nav = Navigator::new(&area, Arrival::From("B"), None, SCALE, 3);
            nav.icons(&[icon(None)]);
            assert_ne!(nav.guide(&[exit("A", "A")]).unwrap().source, "estimated", "{format}");
        }
    }

    #[test]
    fn every_bundled_area_takes_every_call() {
        let graphs = graph::Graphs::default();
        for (id, _) in crate::bundle::LAYOUTS {
            let Some(area) = graphs.get(id) else { continue };
            let exits: Vec<NavTarget> = area.names.keys().map(|e| exit(e, e)).collect();
            for arrival in [Arrival::Unknown, Arrival::From(""), Arrival::From(exits.first().map_or("", |t| t.label.as_str()))] {
                let mut nav = Navigator::new(&area, arrival, None, SCALE, 1);
                for i in 0..40 {
                    nav.moved((i % 7) as f64 - 2.0, 5.0);
                }
                nav.lost(3.0);
                nav.lost(f64::INFINITY);
                for t in &exits {
                    nav.seen(Seen::Left(t.label.as_str()));
                    nav.seen(Seen::Exit(t.label.as_str()));
                }
                nav.seen(Seen::Tag("boss"));
                if let Some(view) = nav.guide(&exits) {
                    assert!(view.degrees.is_none_or(f32::is_finite) && view.distance.is_finite(), "{id}: {view:?}");
                }
                let memory = nav.memory();
                assert!(memory.layouts.values().all(|p| p.is_finite()), "{id}");
                Navigator::new(&area, Arrival::Unknown, Some(&memory), SCALE, 2);
            }
        }
    }

    #[test]
    fn a_bad_scale_falls_back_and_jitter_is_not_walking() {
        let area = graph::load("G1_2");
        for scale in [0.0, -1.0, f64::NAN, 40.0] {
            let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, scale, 1);
            nav.moved(10.0, 0.0);
            assert!(nav.guide(&[exit("G1_3", "Mud Burrow")]).unwrap().degrees.is_some_and(f32::is_finite), "{scale}");
        }
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 1);
        for i in 0..200 {
            nav.moved(if i % 2 == 0 { 1.0 } else { -1.0 }, 0.0);
        }
        assert_eq!(nav.progress().0, 0.0);
    }

    /// Screen offset (minimap cells) of `layout`'s exit to `to` seen from `at` (world units).
    fn icon_of(area: &Area, layout: &str, to: &str, at: (f64, f64)) -> ExitIcon {
        let l = area.layouts.iter().find(|l| l.id == layout).unwrap();
        let n = l.nodes.iter().find(|n| n.exit.as_deref() == Some(to)).unwrap();
        let (sx, sy) = super::super::guide::to_screen((n.x - at.0, n.y - at.1));
        ExitIcon { dx: sx * SCALE, dy: sy * SCALE, to: None, boss: vec![] }
    }

    #[test]
    fn two_named_exit_icons_pin_the_layout_and_the_needle_points_at_the_target() {
        let area = graph::load("G1_2");
        let l = area.layouts.iter().find(|l| l.id == "ClearFell_W_E_N").unwrap();
        // Standing on a corridor somewhere, app just restarted: nothing known.
        let at = l.samples[l.samples.len() / 3].at;
        let mut nav = Navigator::new(&area, Arrival::Unknown, None, SCALE, 4);
        let wood = ExitIcon { to: Some("The Grelwood".into()), ..icon_of(&area, "ClearFell_W_E_N", "G1_4", at) };
        let mud = ExitIcon { to: Some("G1_3".into()), ..icon_of(&area, "ClearFell_W_E_N", "G1_3", at) };
        nav.icons(&[wood.clone(), mud.clone()]);
        let ranking = nav.ranking();
        assert_eq!(ranking[0].0, "ClearFell_W_E_N", "{:?}", &ranking[..3]);
        assert!(ranking[0].1 > 0.6, "{:?}", &ranking[..3]);
        let view = nav.guide(&[exit("G1_4", "The Grelwood")]).unwrap();
        // The icon is near and the route there bends: the route's needle, from a
        // position the icons pinned, close to the icon's bearing.
        let bearing = screen_bearing(wood.dx, wood.dy).unwrap();
        assert!(super::super::guide::angle_between(view.degrees.unwrap(), bearing) < 45.0, "{view:?}");
        assert_ne!(view.source, "unconfirmed", "{view:?}");
        // The same frame again is the same sighting.
        let before = nav.ranking()[0].1;
        nav.icons(&[wood.clone(), mud.clone()]);
        assert_eq!(nav.ranking()[0].1, before);
        // The exits seen are kept, blended over updates.
        let first = nav.memory().exits["ClearFell_W_E_N"].clone();
        nav.icon_walked -= ICON_EVERY;
        nav.icons(&[wood.clone(), mud.clone()]);
        assert_eq!(nav.memory().exits["ClearFell_W_E_N"].len(), first.len());
    }

    #[test]
    fn one_sighting_counts_once_and_does_not_confirm_itself() {
        let area = graph::load("G1_2");
        let l = area.layouts.iter().find(|l| l.id == "ClearFell_W_E_N").unwrap();
        let at = l.samples[l.samples.len() / 3].at;
        let named = |to: &str, exit: &str| ExitIcon { to: Some(to.into()), ..icon_of(&area, "ClearFell_W_E_N", exit, at) };
        let pair = [named("The Grelwood", "G1_4"), named("Mud Burrow", "G1_3")];
        let bare: Vec<ExitIcon> = pair.iter().map(|i| ExitIcon { to: None, ..i.clone() }).collect();
        let mut once = Navigator::new(&area, Arrival::Unknown, None, SCALE, 4);
        once.icons(&pair);
        let mut often = Navigator::new(&area, Arrival::Unknown, None, SCALE, 4);
        // The live pattern: labels on the whole-window scan, every third frame.
        for frame in 0..12 {
            often.icons(if frame % 3 == 0 { &pair } else { &bare });
        }
        let (a, b) = (once.posterior(), often.posterior());
        assert!(a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-9), "{:?} vs {:?}", &once.ranking()[..2], &often.ranking()[..2]);
        assert!(!often.progress().1, "the sighting confirmed itself");
        // Seen unlabelled first, then labelled: the label is one upgrade, counted once.
        let mut late = Navigator::new(&area, Arrival::Unknown, None, SCALE, 4);
        late.icons(&bare);
        late.icons(&pair);
        let upgraded = late.posterior();
        for _ in 0..6 {
            late.icons(&pair);
            late.icons(&bare);
        }
        assert!(late.posterior().iter().zip(&upgraded).all(|(x, y)| (x - y).abs() < 1e-9));
    }

    #[test]
    fn a_label_is_gold_only_when_read_twice_and_not_against_a_sure_belief() {
        let area = graph::load("G1_2");
        let l = area.layouts.iter().find(|l| l.id == "ClearFell_W_E_N").unwrap();
        let at = (l.nodes.iter().find(|n| n.exit.as_deref() == Some("G1_3")).unwrap().x + 60.0, l.nodes.iter().find(|n| n.exit.as_deref() == Some("G1_3")).unwrap().y);
        let mud = icon_of(&area, "ClearFell_W_E_N", "G1_3", at);
        let label = |to: &str| ExitIcon { to: Some(to.into()), ..mud.clone() };
        let target = [exit("G1_4", "The Grelwood")];
        // Mud Burrow's exit, its label misread as The Grelwood in one scan in three: never gold.
        let mut nav = Navigator::new(&area, Arrival::Unknown, None, SCALE, 5);
        for scan in 0..12 {
            let read = if scan % 3 == 2 { label("The Grelwood") } else { label("Mud Burrow") };
            for frame in [read, mud.clone(), mud.clone()] {
                nav.icons(&[frame]);
                assert_ne!(nav.guide(&target).unwrap().source, "estimated", "scan {scan}");
            }
        }
        // A sure belief that this is Mud Burrow, and a label saying The Grelwood twice: rough at most.
        let mut nav = Navigator::new(&area, Arrival::Unknown, None, SCALE, 5).trusted();
        let k = area.layouts.iter().position(|l| l.id == "ClearFell_W_E_N").unwrap();
        nav.banks[k].evidence += 20.0;
        nav.icons(&[label("Mud Burrow")]);
        nav.icons(&[label("Mud Burrow")]);
        let mut wrong = Navigator::new(&area, Arrival::Unknown, None, SCALE, 5).trusted();
        wrong.banks[k].evidence += 20.0;
        for bank in &mut wrong.banks {
            bank.particles = nav.banks[k].particles.clone();
        }
        wrong.banks[k].particles = nav.banks[k].particles.clone();
        wrong.icons(&[label("The Grelwood")]);
        wrong.icons(&[label("The Grelwood")]);
        let view = wrong.guide(&target).unwrap();
        assert_ne!(view.source, "estimated", "{view:?}");
    }

    #[test]
    fn the_needle_switches_between_icon_and_route_with_hysteresis() {
        // A walk that sweeps the route-to-straight ratio from 1.1 to 1.6 and back.
        let mut ratios: Vec<f64> = (0..=50).map(|i| 1.1 + 0.01 * i as f64).collect();
        ratios.extend(ratios.clone().into_iter().rev());
        // With noise on top, as particles move.
        let noisy: Vec<f64> = ratios.iter().enumerate().map(|(i, r)| r + if i % 2 == 0 { 0.04 } else { -0.04 }).collect();
        // At 1.1 the needle is at the icon.
        let (mut state, mut switches) = (true, 0);
        for r in noisy {
            let next = at_icon(state, r);
            switches += usize::from(next != state);
            state = next;
        }
        assert!(switches <= 2, "{switches} switches");
    }

    #[test]
    fn of_two_exits_with_the_same_name_the_nearest_wins_in_any_order() {
        let json = r#"{"connections": ["A", "B"], "connectionNames": ["Aville", "Bee"], "topologies": [
            {"id": "two", "nodes": [{"x": 0, "y": 0, "exit": "A"}, {"x": 300, "y": 0, "exit": "B"}, {"x": 300, "y": 300, "exit": "A"}],
             "edges": [{"a": 0, "b": 1}, {"a": 1, "b": 2}]}]}"#;
        let area = Area::parse("T", json).unwrap();
        let at = (300.0, 0.0);
        let icon = |n: (f64, f64)| {
            let (sx, sy) = super::super::guide::to_screen((n.0 - at.0, n.1 - at.1));
            ExitIcon { dx: sx * SCALE, dy: sy * SCALE, to: Some("Aville".into()), boss: vec![] }
        };
        let (near, far) = (icon((300.0, 300.0)), icon((0.0, 0.0)));
        let bearing = |icons: &[ExitIcon]| {
            let mut nav = Navigator::new(&area, Arrival::From("B"), None, SCALE, 1);
            nav.icons(icons);
            nav.guide(&[exit("A", "Aville")]).unwrap().degrees
        };
        assert_eq!(bearing(&[near.clone(), far.clone()]), bearing(&[far, near]));
    }

    #[test]
    fn one_unlabelled_icon_and_a_known_arrival_pin_the_layout() {
        let area = graph::load("G1_2");
        let l = area.layouts.iter().find(|l| l.id == "ClearFell_W_E_N").unwrap();
        let town = l.nodes.iter().find(|n| n.exit.as_deref() == Some("G1_town")).unwrap();
        // Just arrived from the encampment; Mud Burrow's exit is in view.
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 4);
        nav.icons(&[icon_of(&area, "ClearFell_W_E_N", "G1_3", (town.x, town.y))]);
        let ranking = nav.ranking();
        assert!(ranking[..3].iter().any(|(id, _)| *id == "ClearFell_W_E_N"), "{:?}", &ranking[..3]);
        assert!(ranking.iter().find(|(id, _)| *id == "ClearFell_W_E_N").unwrap().1 > 0.15, "{:?}", &ranking[..3]);
    }

    #[test]
    fn a_labelled_icon_is_a_named_anchor_and_one_unlabelled_icon_is_not_enough() {
        let area = graph::load("G1_2");
        let l = area.layouts.iter().find(|l| l.id == "ClearFell_W_E_N").unwrap();
        let at = l.samples[l.samples.len() / 3].at;
        let wood = icon_of(&area, "ClearFell_W_E_N", "G1_4", at);
        let target = [exit("G1_4", "The Grelwood")];
        // One unlabelled icon, nothing else known: it could be any exit of any layout.
        let mut nav = Navigator::new(&area, Arrival::Unknown, None, SCALE, 4);
        nav.icons(&[wood.clone()]);
        let unsure = nav.guide(&target).unwrap();
        assert!(nav.ranking()[0].1 < 0.2, "{:?}", &nav.ranking()[..3]);
        assert_ne!(unsure.source, "estimated", "{unsure:?}");
        // Labelled, it is that exit: the needle points at it.
        let mut nav = Navigator::new(&area, Arrival::Unknown, None, SCALE, 4);
        let named = ExitIcon { to: Some("The Grelwood".into()), ..wood.clone() };
        nav.icons(&[named.clone()]);
        let view = nav.guide(&target).unwrap();
        assert!(super::super::guide::angle_between(view.degrees.unwrap(), screen_bearing(named.dx, named.dy).unwrap()) < 45.0, "{view:?}");
        assert_ne!(view.source, "unconfirmed", "{view:?}");
        // A label naming another exit is not the target.
        let mut nav = Navigator::new(&area, Arrival::Unknown, None, SCALE, 4);
        nav.icons(&[ExitIcon { to: Some("Mud Burrow".into()), ..wood }]);
        assert_ne!(nav.guide(&target).unwrap().degrees, view.degrees);
    }

    /// A boss's icon at Clearfell's boss node of `layout`, seen from `at` (world units).
    fn boss_icon_of(area: &Area, layout: &str, at: (f64, f64)) -> ExitIcon {
        let l = area.layouts.iter().find(|l| l.id == layout).unwrap();
        let n = l.nodes.iter().find(|n| n.has_tag("boss")).unwrap();
        let (sx, sy) = super::super::guide::to_screen((n.x - at.0, n.y - at.1));
        ExitIcon { dx: sx * SCALE, dy: sy * SCALE, to: None, boss: vec![BossAt::Tag("boss".into())] }
    }

    #[test]
    fn a_boss_icon_is_weighed_at_the_boss_places_not_the_exits() {
        let area = graph::load("G1_2");
        let k = area.layouts.iter().position(|l| l.id == "ClearFell_W_E_N").unwrap();
        let l = &area.layouts[k];
        let town = l.nodes.iter().find(|n| n.exit.as_deref() == Some("G1_town")).unwrap();
        let at = (town.x, town.y);
        let boss = boss_icon_of(&area, "ClearFell_W_E_N", at);
        // Its candidate nodes are the boss nodes; one place or several, as given.
        let nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 4);
        let tagged: Vec<usize> = (0..l.nodes.len()).filter(|&i| l.nodes[i].has_tag("boss")).collect();
        assert_eq!(nav.icon_nodes(l, &boss), tagged);
        let both = ExitIcon { boss: vec![BossAt::Tag("boss".into()), BossAt::Exit("G1_3".into())], ..boss.clone() };
        assert_eq!(nav.icon_nodes(l, &both).len(), tagged.len() + 1);
        // Just arrived from the encampment, Beira's icon in view: the layout whose boss
        // node lies there gains, more than from the walk alone.
        let share = |icons: &[ExitIcon]| {
            let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 4);
            nav.icons(icons);
            nav.posterior()[k]
        };
        let (none, seen) = (share(&[]), share(std::slice::from_ref(&boss)));
        assert!(seen > 1.5 * none, "W_E_N without {none:.3}, with Beira's icon {seen:.3}");
        // The same place taken for an exit says something else.
        let as_exit = share(&[ExitIcon { boss: vec![], ..boss.clone() }]);
        assert!((as_exit - seen).abs() > 0.01, "as an exit {as_exit:.3}, as a boss {seen:.3}");
        // A boss's icon and an exit's at one place are two sightings.
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 4);
        nav.icons(&[boss.clone(), ExitIcon { boss: vec![], ..boss.clone() }]);
        assert_eq!(nav.sightings.len(), 2);
    }

    #[test]
    fn a_boss_icon_never_confirms_the_belief() {
        let area = graph::load("G1_2");
        let l = area.layouts.iter().find(|l| l.id == "ClearFell_W_E_N").unwrap();
        let at = l.samples[l.samples.len() / 3].at;
        let target = [NavTarget { label: "Beira".into(), tag: Some("boss".into()), ..Default::default() }];
        // Nothing known: a boss's icon alone, seen again and again, is no anchor and no gold.
        let mut nav = Navigator::new(&area, Arrival::Unknown, None, SCALE, 4);
        for _ in 0..6 {
            nav.icons(&[boss_icon_of(&area, "ClearFell_W_E_N", at)]);
            assert_ne!(nav.guide(&target).unwrap().source, "estimated");
        }
        assert!(!nav.progress().1, "a boss icon alone confirmed itself");
        // Two exits read by name fix the layout and place; the boss's icon where the belief
        // expects it is a new sighting the belief predicted. An unlabelled exit icon would
        // confirm the belief then; a boss's does not (`BOSS_CONFIRMS`), unless switched on.
        let named = |to: &str, exit: &str| ExitIcon { to: Some(to.into()), ..icon_of(&area, "ClearFell_W_E_N", exit, at) };
        let pinned = |tuning: Tuning| {
            let mut nav = Navigator::new(&area, Arrival::Unknown, None, SCALE, 4).tuned(tuning);
            nav.icons(&[named("The Grelwood", "G1_4"), named("Mud Burrow", "G1_3")]);
            assert!(!nav.progress().1);
            nav.icons(&[named("The Grelwood", "G1_4"), named("Mud Burrow", "G1_3"), boss_icon_of(&area, "ClearFell_W_E_N", at)]);
            nav
        };
        assert!(!pinned(Tuning::default()).progress().1);
        let nav = pinned(Tuning { boss_confirms: true, ..Tuning::default() });
        assert!(nav.progress().1);
        // The needle to Beira points at her icon.
        let icon = boss_icon_of(&area, "ClearFell_W_E_N", at);
        let view = nav.guide(&target).unwrap();
        assert!(super::super::guide::angle_between(view.degrees.unwrap(), screen_bearing(icon.dx, icon.dy).unwrap()) < 30.0, "{view:?}");
    }

    /// Clearfell 2693856399 (`ClearFell_W_E_N`), measured on the big-map screenshot: the
    /// middle checkpoint seen from where the player came in, as cells on screen.
    fn middle_checkpoint_from_the_town(area: &Area) -> (f64, f64) {
        let l = area.layouts.iter().find(|l| l.id == "ClearFell_W_E_N").unwrap();
        let node = |exit: &str| l.nodes.iter().find(|n| n.exit.as_deref() == Some(exit)).unwrap();
        let to_file = |dx: f64, dy: f64| {
            let (x, y) = super::super::guide::from_screen((dx / SCALE, dy / SCALE));
            (x / l.unit, y / l.unit)
        };
        // The player: the Grelwood's exit icon at (-136, 83) cells (its node stands for it);
        // the middle checkpoint at (228.5, 199.5) cells from the player.
        let (ex, ey) = to_file(-136.0, 83.0);
        let player = (node("G1_4").x - ex, node("G1_4").y - ey);
        let (cx, cy) = to_file(228.5, 199.5);
        let checkpoint = (player.0 + cx, player.1 + cy);
        let town = node("G1_town");
        let (sx, sy) = super::super::guide::to_screen(((checkpoint.0 - town.x) * l.unit, (checkpoint.1 - town.y) * l.unit));
        (sx * SCALE, sy * SCALE)
    }

    #[test]
    fn a_checkpoint_is_soft_evidence_of_a_landmark() {
        let area = graph::load("G1_2");
        let k = area.layouts.iter().position(|l| l.id == "ClearFell_W_E_N").unwrap();
        let l = &area.layouts[k];
        // Which nodes count: exits and named places, not corridor shapes.
        let marks: Vec<String> = l.nodes.iter().filter(|n| landmark(n)).map(|n| n.exit.clone().or(n.tag.clone()).unwrap_or_default()).collect();
        assert_eq!(marks, ["G1_town", "bridge_small", "bridge_small", "boss", "G1_4", "campsite", "G1_3"]);
        // Arrived from the encampment, the middle checkpoint in view: the true layout gains,
        // or at least loses nothing.
        let seen = middle_checkpoint_from_the_town(&area);
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 4);
        let before = nav.posterior()[k];
        let particles: Vec<(f64, f64, f64)> = nav.banks[k].particles.iter().map(|p| (p.x, p.y, p.w)).collect();
        nav.checkpoints(&[seen], &[]);
        let after = nav.posterior()[k];
        println!("ClearFell_W_E_N {before:.4} -> {after:.4} (checkpoint at {:.0}, {:.0} cells); top {:?}", seen.0, seen.1, &nav.ranking()[..3]);
        assert!(after >= 0.99 * before, "{before} -> {after}");
        // Never an anchor, never in gold's share, no particle moved.
        assert!(!nav.progress().1);
        assert_eq!(nav.gold_posterior()[k], before);
        assert_eq!(nav.banks[k].particles.iter().map(|p| (p.x, p.y, p.w)).collect::<Vec<_>>(), particles);
        // The same checkpoint again counts once.
        nav.checkpoints(&[seen], &[]);
        assert_eq!(nav.posterior()[k], after);
        // Next to an exit icon of the same frame it adds nothing.
        let mut by_exit = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 4);
        let exit = ExitIcon { dx: seen.0 + 30.0, dy: seen.1 - 20.0, to: None, boss: vec![] };
        by_exit.checkpoints(&[seen], std::slice::from_ref(&exit));
        assert_eq!(by_exit.posterior()[k], before);
        // Where no layout has a landmark (far outside every graph) it changes little.
        let mut lost = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 4);
        let all_before = lost.posterior();
        lost.checkpoints(&[(9000.0, 9000.0)], &[]);
        let change = lost.posterior().iter().zip(&all_before).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(change < 1e-6, "{change}");
    }

    #[test]
    fn checkpoints_take_at_most_the_cap_and_count_once() {
        let area = graph::load("G1_2");
        // 50 checkpoints all over the zone (false ones, say): at most six count, and no
        // layout loses more than the cap against another.
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 4);
        let before = nav.posterior();
        let points: Vec<(f64, f64)> = (0..50).map(|i| ((i % 10) as f64 * 90.0 - 450.0, (i / 10) as f64 * 90.0 - 225.0)).collect();
        for p in &points {
            nav.checkpoints(&[*p], &[]);
        }
        assert_eq!(nav.checkpoints_counted.len(), CHECKPOINT_MOST);
        let soft: Vec<f64> = nav.banks.iter().map(|b| b.checkpoints).collect();
        let spread = soft.iter().copied().fold(f64::MIN, f64::max) - soft.iter().copied().fold(f64::MAX, f64::min);
        assert!(spread <= CHECKPOINT_CAP + 1e-9, "{spread}");
        let odds = |p: &[f64], i: usize, j: usize| (p[i] / p[j]).ln();
        let after = nav.posterior();
        let moved = (0..after.len()).flat_map(|i| (0..after.len()).map(move |j| (i, j))).map(|(i, j)| (odds(&after, i, j) - odds(&before, i, j)).abs()).fold(0.0, f64::max);
        assert!(moved <= CHECKPOINT_CAP + 1e-9, "{moved}");
        // One checkpoint, a tracking loss, the same checkpoint again from the same place:
        // the belief puts it where the counted one is, so it counts once.
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 4);
        let seen = middle_checkpoint_from_the_town(&area);
        nav.checkpoints(&[seen], &[]);
        let once = nav.posterior();
        nav.lost(1.0);
        nav.checkpoints(&[seen], &[]);
        assert_eq!(nav.checkpoints_counted.len(), 1);
        assert_eq!(nav.posterior(), once);
    }

    /// The owner's live case (N7): Clearfell 2693856399, app restarted (arrival
    /// unknown, no memory), big map open, the Grelwood exit icon at (−136, +83)
    /// cells; the wider capture also sees Mud Burrow at (823, 110).
    #[test]
    fn the_live_clearfell_case() {
        let area = graph::load("G1_2");
        let target = [exit("G1_4", "The Grelwood")];
        let grelwood = ExitIcon { dx: -136.0, dy: 83.0, to: None, boss: vec![] };
        let mud = ExitIcon { dx: 823.0, dy: 110.0, to: None, boss: vec![] };
        let mut nav = Navigator::new(&area, Arrival::Unknown, None, SCALE, 2693856399);
        nav.icons(&[grelwood.clone()]);
        let one = nav.guide(&target).unwrap();
        eprintln!("one icon: {one:?}; top {:?}", &nav.ranking()[..3]);
        assert_ne!(one.source, "estimated", "one unlabelled icon cannot say which exit it is");
        nav.icons(&[grelwood.clone(), mud.clone()]);
        let two = nav.guide(&target).unwrap();
        let truth = nav.ranking().iter().position(|(id, _)| *id == "ClearFell_W_E_N").unwrap();
        eprintln!("two unlabelled icons: {two:?}; top {:?}; ClearFell_W_E_N {:?} at rank {truth}", &nav.ranking()[..4], nav.ranking()[truth]);
        // Unlabelled, the pair fits more than one layout (any two exits may be them).
        assert_ne!(two.source, "estimated", "{two:?}");
        // Both labelled: the layout and the needle.
        let mut nav = Navigator::new(&area, Arrival::Unknown, None, SCALE, 2693856399);
        nav.icons(&[ExitIcon { to: Some("The Grelwood".into()), ..grelwood.clone() }, ExitIcon { to: Some("Mud Burrow".into()), ..mud }]);
        let named = nav.guide(&target).unwrap();
        eprintln!("two labelled icons: {named:?}; top {:?}", &nav.ranking()[..3]);
        assert_eq!(nav.ranking()[0].0, "ClearFell_W_E_N");
        assert!(super::super::guide::angle_between(named.degrees.unwrap(), screen_bearing(-136.0, 83.0).unwrap()) < 15.0, "{named:?}");
        assert_ne!(named.source, "unconfirmed", "{named:?}");
        // Labelled, one icon is enough to point at it.
        let mut nav = Navigator::new(&area, Arrival::Unknown, None, SCALE, 2693856399);
        nav.icons(&[ExitIcon { to: Some("The Grelwood".into()), ..grelwood }]);
        let one_named = nav.guide(&target).unwrap();
        eprintln!("one labelled icon: {one_named:?}");
        // The distance shown is the icon's (159 cells), not the belief's route.
        assert!((one_named.distance - 136f32.hypot(83.0)).abs() < 1.0, "{one_named:?}");
        assert!(super::super::guide::angle_between(one_named.degrees.unwrap(), screen_bearing(-136.0, 83.0).unwrap()) < 15.0, "{one_named:?}");
    }

    /// The live regression of v0.1.41 (S7): the app restarted by the player at the
    /// Grelwood exit, its memory from earlier builds, then the reader's frames: the
    /// near exit unlabelled, the whole-window scan with both named, unlabelled again.
    fn live_frames(nav: &mut Navigator) {
        let near = ExitIcon { dx: -136.0, dy: 83.0, to: None, boss: vec![] };
        let scan = [
            ExitIcon { to: Some("The Grelwood".into()), ..near.clone() },
            ExitIcon { dx: 823.0, dy: 110.0, to: Some("Mud Burrow".into()), boss: vec![] },
        ];
        for round in 0..4 {
            nav.icons(&[near.clone()]);
            nav.icons(&[near.clone()]);
            nav.icons(&scan);
            let _ = round;
        }
    }

    #[test]
    fn two_named_exits_win_whatever_was_believed_before() {
        let area = graph::load("G1_2");
        let target = [exit("G1_4", "The Grelwood")];
        let check = |nav: &Navigator, case: &str| {
            let r = nav.ranking();
            let p = r.iter().find(|(id, _)| *id == "ClearFell_W_E_N").unwrap().1;
            let view = nav.guide(&target).unwrap();
            eprintln!("{case}: W_E_N {p:.3}, top {:?}, needle {view:?}", &r[..3]);
            assert!(p >= 0.6, "{case}: W_E_N {p}, top {:?}", &r[..3]);
            assert!(super::super::guide::angle_between(view.degrees.unwrap(), screen_bearing(-136.0, 83.0).unwrap()) < 15.0, "{case}: {view:?}");
        };
        // The file the app wrote with v0.1.41.
        let old: Memory = serde_json::from_str(include_str!("live-v0.1.41.nav.json")).unwrap();
        let mut nav = Navigator::new(&area, Arrival::Unknown, Some(&old), SCALE, 2693856399);
        live_frames(&mut nav);
        check(&nav, "memory of v0.1.41");
        // The same beliefs in a memory of the current version: still overturned.
        let current = Memory { version: MEMORY_VERSION, at: unix_now(), ..old };
        let mut nav = Navigator::new(&area, Arrival::Unknown, Some(&current), SCALE, 2693856399);
        live_frames(&mut nav);
        check(&nav, "the same beliefs, current version");
        // No memory, the near exit unlabelled first, then the scan.
        let mut nav = Navigator::new(&area, Arrival::Unknown, None, SCALE, 2693856399);
        live_frames(&mut nav);
        check(&nav, "no memory");
    }

    #[test]
    fn leaving_is_soft_evidence() {
        let json = r#"{"connections": ["A", "B"], "topologies": [
            {"id": "both", "nodes": [{"x": 0, "y": 0, "exit": "A"}, {"x": 600, "y": 0, "exit": "B"}], "edges": [{"a": 0, "b": 1}]},
            {"id": "one", "nodes": [{"x": 0, "y": 0, "exit": "A"}, {"x": 600, "y": 0}], "edges": [{"a": 0, "b": 1}]}]}"#;
        let area = Area::parse("T", json).unwrap();
        let mut nav = Navigator::new(&area, Arrival::From("A"), None, SCALE, 3);
        let before: Vec<(f64, f64)> = nav.banks[0].particles.iter().map(|p| (p.x, p.y)).collect();
        nav.seen(Seen::Left("B"));
        // Nothing moved, the layout without exit B lost nothing, and the far exit costs at most the floor.
        assert_eq!(nav.banks[0].particles.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>(), before);
        assert_eq!(nav.banks[1].evidence, 0.0);
        assert!(nav.banks[0].evidence >= LEFT_FLOOR.ln() - 1e-9);
        assert!(!nav.progress().1, "leaving is not an anchor");
    }

    #[test]
    fn a_boss_speaking_is_soft_evidence_for_layouts_that_put_it_nearby() {
        // The player arrives at exit A. One layout has its boss right there, the other
        // 2,000 units off, a third has no boss node.
        let json = r#"{"connections": ["A", "B"], "topologies": [
            {"id": "near", "nodes": [{"x": 0, "y": 0, "exit": "A"}, {"x": 100, "y": 0, "tag": "boss"}, {"x": 2000, "y": 0, "exit": "B"}], "edges": [{"a": 0, "b": 1}, {"a": 1, "b": 2}]},
            {"id": "far", "nodes": [{"x": 0, "y": 0, "exit": "A"}, {"x": 2000, "y": 0, "tag": "boss"}, {"x": 2000, "y": 300, "exit": "B"}], "edges": [{"a": 0, "b": 1}, {"a": 1, "b": 2}]},
            {"id": "none", "nodes": [{"x": 0, "y": 0, "exit": "A"}, {"x": 2000, "y": 0, "exit": "B"}], "edges": [{"a": 0, "b": 1}]}]}"#;
        let area = Area::parse("T", json).unwrap();
        let mut nav = Navigator::new(&area, Arrival::From("A"), None, SCALE, 3);
        let before: Vec<(f64, f64)> = nav.banks[0].particles.iter().map(|p| (p.x, p.y)).collect();
        nav.seen(Seen::NearTag("boss"));
        // Nothing moved and it is no anchor; the layout with the boss nearby keeps almost
        // all, the one with it far away drops to the floor, the one without it loses nothing.
        assert_eq!(nav.banks[0].particles.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>(), before);
        assert!(!nav.progress().1, "a boss line is not an anchor");
        assert!(nav.banks[0].evidence > -0.25, "{}", nav.banks[0].evidence);
        assert!(nav.banks[1].evidence < NEAR_FLOOR.ln() + 0.15 && nav.banks[1].evidence >= NEAR_FLOOR.ln() - 1e-9);
        assert_eq!(nav.banks[2].evidence, 0.0);
        let p = nav.posterior();
        assert!(p[0] > p[1] * 2.0, "{p:?}");
        // A boss at an exit works the same way; a tag no layout has says nothing.
        let mut nav = Navigator::new(&area, Arrival::From("A"), None, SCALE, 3);
        nav.seen(Seen::NearExit("B"));
        assert!(nav.banks.iter().all(|b| b.evidence < 0.0), "exit B is far in every layout");
        let mut nav = Navigator::new(&area, Arrival::From("A"), None, SCALE, 3);
        nav.seen(Seen::NearTag("dragon"));
        assert!(nav.banks.iter().all(|b| b.evidence == 0.0));
    }

    #[test]
    fn tracking_lost_keeps_the_evidence_and_the_way() {
        let area = graph::load("G1_2");
        let events = walk_on(&area, WALKED, "G1_town", "G1_4", 5);
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 5);
        feed(&mut nav, &events[..events.len() * 3 / 4]);
        let target = [exit("G1_4", "The Grelwood")];
        let before = nav.ranking()[0].0.to_string();
        let p_before = nav.ranking()[0].1;
        let needle = nav.guide(&target).unwrap();
        nav.lost(3.0);
        assert_eq!((nav.ranking()[0].0, nav.ranking()[0].1), (before.as_str(), p_before));
        let after = nav.guide(&target).unwrap();
        assert!(super::super::guide::angle_between(after.degrees.unwrap(), needle.degrees.unwrap()) < 45.0, "{needle:?} {after:?}");
        // A long loss (waypoint, portal) keeps the layout too.
        nav.lost(f64::INFINITY);
        assert_eq!(nav.ranking()[0].0, before);
    }

    #[test]
    fn memory_carries_the_layout_and_a_fresh_position_to_the_next_start() {
        let area = graph::load("G1_2");
        let events = walk_on(&area, WALKED, "G1_town", "G1_4", 5);
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 5);
        feed(&mut nav, &events[..events.len() * 3 / 4]);
        let target = [exit("G1_4", "The Grelwood")];
        let needle = nav.guide(&target).unwrap();
        let memory: Memory = serde_json::from_str(&serde_json::to_string(&nav.memory()).unwrap()).unwrap();
        let (top, p_top) = (nav.ranking()[0].0.to_string(), nav.ranking()[0].1);
        let top = top.as_str();
        assert!((memory.layouts[top] - p_top).abs() < 1e-9);
        assert!(memory.positions.contains_key(top) && memory.positions[top].len() == KEEP);
        // Re-entering the instance through an exit: the layout is remembered, the position is the exit.
        let again = Navigator::new(&area, Arrival::From("G1_3"), Some(&memory), SCALE, 6);
        assert_eq!(again.ranking()[0].0, top);
        // A restart of the app where the player stands: the position too.
        let resumed = Navigator::new(&area, Arrival::Unknown, Some(&memory), SCALE, 7);
        let view = resumed.guide(&target).unwrap();
        assert!(super::super::guide::angle_between(view.degrees.unwrap(), needle.degrees.unwrap()) < 30.0, "{needle:?} {view:?}");
        assert_eq!(view.variant, needle.variant);
        // Saved 5 minutes ago: the player may be anywhere within reach, and nothing
        // is gold before this visit has data of its own.
        let aged = Memory { at: memory.at - 300, ..memory.clone() };
        let later = Navigator::new(&area, Arrival::Unknown, Some(&aged), SCALE, 7);
        let first = later.guide(&target).unwrap();
        assert_ne!(first.source, "estimated", "{first:?}");
        assert!(first.confidence < view.confidence, "{first:?} {view:?}");
        assert_eq!(later.progress().0, 0.0, "walked counts this visit only");
        assert_eq!(later.memory().walked, memory.walked);
        assert!(!later.changed());
        // Older than `RESUME`: the layout only.
        let stale = Memory { at: memory.at - RESUME - 1, ..memory.clone() };
        let cold = Navigator::new(&area, Arrival::Unknown, Some(&stale), SCALE, 7);
        assert_eq!(cold.ranking()[0].0, top);
        let unsure = cold.guide(&target).unwrap();
        assert!(unsure.confidence < view.confidence, "{unsure:?} {view:?}");
    }

    use super::super::guide::angle_between;
    use super::super::route::tests::{clearfell, line, outline, walls};

    fn lent<'a>(walls: &'a HashMap<(i32, i32), u32>, trail: &'a [[i32; 2]], player: (i32, i32), came_in: Option<CameIn<'a>>) -> Place<'a> {
        Place { map: route::Map { walls, trail }, player, came_in }
    }

    /// Came in from `from` at `at` a second after the zone entry, its exit's icon there.
    fn by_exit(from: &str, at: (i32, i32)) -> Option<CameIn<'_>> {
        Some(CameIn { from, at, delay: 1.0, waypoint: false, exits: &[None] })
    }

    fn bearing(dx: f64, dy: f64) -> f32 {
        screen_bearing(dx, dy).unwrap()
    }

    #[test]
    fn a_named_icon_behind_a_wall_is_reached_through_the_doorway() {
        let area = graph::load("G1_2");
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 3).map_ahead_live();
        // The Grelwood's exit 80 cells up, read by name in two scans; a wall across
        // halfway, with a 20-cell doorway to the right.
        let icon = ExitIcon { dx: 0.0, dy: -80.0, to: Some("The Grelwood".into()), boss: vec![] };
        nav.icons(std::slice::from_ref(&icon));
        nav.icons(std::slice::from_ref(&icon));
        let target = [exit("G1_4", "The Grelwood")];
        let map = walls([line((-200, -40), (59, -40)), line((80, -40), (200, -40))].concat());
        let before = nav.guide(&target).unwrap();
        let after = nav.guide_on(&target, Some(&lent(&map, &[], (0, 0), None))).unwrap();
        let doorway = bearing(70.0, -40.0);
        println!("before {:?}° {} {:.0} cells; after {:?}° {} {:.0} cells; doorway {doorway:.0}°",
            before.degrees, before.source, before.distance, after.degrees, after.source, after.distance);
        // Before: across the wall (along the layout's corridors from where the icon
        // puts the player, nearly straight), rough.
        assert!(angle_between(before.degrees.unwrap(), -90.0) < 5.0, "{before:?}");
        assert_eq!(before.source, "rough");
        // After: to the doorway first, the way's length, still gold.
        assert!(angle_between(after.degrees.unwrap(), doorway) < 10.0, "{after:?}");
        assert!((150.0..175.0).contains(&after.distance), "{after:?}");
        // Most of the way lies over ground nothing was recorded on (no walk here,
        // the wall alone): rough.
        assert_eq!(after.source, "rough");
        // The icon goes out of view and the player walks 30 cells right: the
        // sighting keeps its place, the way still goes through the doorway.
        nav.icons(&[]);
        for _ in 0..3 {
            nav.moved(10.0, 0.0);
        }
        let later = nav.guide_on(&target, Some(&lent(&map, &[], (30, 0), None))).unwrap();
        assert!(angle_between(later.degrees.unwrap(), bearing(40.0, -40.0)) < 10.0, "{later:?}");
        assert!((125.0..150.0).contains(&later.distance), "{later:?}");
        assert_eq!(later.source, "rough");
        // Without the map the needle is the layouts' (no icon in view).
        assert_ne!(nav.guide(&target).unwrap().distance, later.distance);
        // A loss ends the sighting and the kept way.
        nav.lost(5.0);
        assert!(nav.way.borrow().is_none());
        assert_eq!(nav.guide_on(&target, Some(&lent(&map, &[], (30, 0), None))), nav.guide(&target));
    }

    #[test]
    fn without_a_way_the_needle_is_as_before() {
        let area = graph::load("G1_2");
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 3).map_ahead_live();
        let icon = ExitIcon { dx: 0.0, dy: -80.0, to: Some("The Grelwood".into()), boss: vec![] };
        nav.icons(std::slice::from_ref(&icon));
        nav.icons(std::slice::from_ref(&icon));
        let target = [exit("G1_4", "The Grelwood")];
        // The exit walled in (as recorded): no way, the icon's needle as today.
        let map = walls(outline(-30, -110, 30, -50));
        let view = nav.guide_on(&target, Some(&lent(&map, &[], (0, 0), None)));
        assert_eq!(view, nav.guide(&target));
        assert!(angle_between(view.unwrap().degrees.unwrap(), -90.0) < 5.0);
    }

    #[test]
    fn the_way_back_follows_the_recorded_walk_and_is_gold() {
        // Mud Burrow (a dungeon: inferred needles are never gold there).
        let area = graph::load("G1_3");
        let nav = Navigator::new(&area, Arrival::From("G1_2"), None, SCALE, 5);
        // In at (0, 0), down 100, right 30, up 100 round a wall between; now 30
        // cells from the arrival, the wall between.
        let map = walls(line((15, -200), (15, 70)));
        let mut trail: Vec<[i32; 2]> = (0..=10).map(|k| [0, 10 * k]).collect();
        trail.extend((1..=3).map(|k| [10 * k, 100]));
        trail.extend((1..=10).map(|k| [30, 100 - 10 * k]));
        let target = [exit("G1_2", "Clearfell")];
        let view = nav.guide_on(&target, Some(&lent(&map, &trail, (30, 0), by_exit("G1_2", (0, 0))))).unwrap();
        // Round the wall's end, not across it: down, along the walk. Five times the
        // straight line, but every step of it by the walk: gold.
        assert!(angle_between(view.degrees.unwrap(), bearing(-15.0, 78.0)) < 10.0, "{view:?}");
        assert!((150.0..175.0).contains(&view.distance), "{view:?}");
        assert_eq!(view.source, "estimated");
        assert_eq!(view.target, "Clearfell");
        let inferred = nav.guide(&target).unwrap();
        assert_ne!(inferred.source, "estimated");
        // A wider U (100 cells): the way cuts across ground the walk did not cover,
        // twice the straight line: rough.
        let map = walls(line((50, -200), (50, 80)));
        let mut trail: Vec<[i32; 2]> = (0..=10).map(|k| [0, 10 * k]).collect();
        trail.extend((1..=10).map(|k| [10 * k, 100]));
        trail.extend((1..=10).map(|k| [100, 100 - 10 * k]));
        let view = nav.guide_on(&target, Some(&lent(&map, &trail, (100, 0), by_exit("G1_2", (0, 0))))).unwrap();
        println!("{view:?}");
        assert!(angle_between(view.degrees.unwrap(), bearing(-45.0, 87.0)) < 10.0, "{view:?}");
        assert_eq!(view.source, "rough");
    }

    #[test]
    fn a_long_way_round_is_rough() {
        // A recorded U-corridor: the named exit 64 cells across the block, the way
        // round it more than twice that, every cell of it by a recorded wall.
        let area = graph::load("G1_2");
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 3).map_ahead_live();
        named_icon(&mut nav, 64.0, 0.0, "The Grelwood");
        let target = [exit("G1_4", "The Grelwood")];
        let map = walls([outline(0, 0, 88, 74), outline(24, 0, 64, 50)].concat());
        let view = nav.guide_on(&target, Some(&lent(&map, &[], (12, 10), None))).unwrap();
        let note = nav.last_way().unwrap();
        println!("{view:?} {note:?}");
        assert!(view.distance > 2.0 * 64.0 && note.unrecorded < 0.1, "{view:?} {note:?}");
        assert!(angle_between(view.degrees.unwrap(), 90.0) < 15.0, "down the arm: {view:?}");
        assert_eq!(view.source, "rough");
    }

    #[test]
    fn an_arrival_from_a_town_is_no_exits_place() {
        // From town the player may have come by waypoint or portal.
        let area = graph::load("G1_2");
        let nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 5);
        let map = walls(line((30, -200), (30, 80)));
        let target = [exit("G1_town", "Clearfell Encampment")];
        let place = lent(&map, &[], (60, 0), by_exit("G1_town", (0, 0)));
        assert_eq!(nav.guide_on(&target, Some(&place)), nav.guide(&target));
    }

    #[test]
    fn a_needle_on_the_real_map_takes_a_few_milliseconds() {
        // The owner's Clearfell walls (21,500 cells seen twice) and trail; a named
        // exit icon 300 cells off. Time per `guide_on` call.
        let (map, trail) = clearfell();
        let area = graph::load("G1_2");
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 3).map_ahead_live();
        let target = [exit("G1_4", "The Grelwood")];
        let mut times = Vec::new();
        for (k, &[x, y]) in trail.iter().enumerate().step_by(25) {
            let icon = ExitIcon { dx: 240.0, dy: -180.0, to: Some("The Grelwood".into()), boss: vec![] };
            nav.icons(std::slice::from_ref(&icon));
            nav.icons(std::slice::from_ref(&icon));
            let started = std::time::Instant::now();
            let view = nav.guide_on(&target, Some(&lent(&map, &trail[..=k], (x, y), None))).unwrap();
            times.push(started.elapsed());
            assert!(view.degrees.is_some());
            nav.lost(1.0);
        }
        times.sort();
        println!("guide_on on the real map: {} calls, median {:?}, slowest {:?}", times.len(), times[times.len() / 2], times[times.len() - 1]);
        // The test profile is optimised less than the app; release: median 0.3 ms.
        assert!(times[times.len() - 1].as_millis() < 20 && times[times.len() / 2].as_millis() < 5, "{times:?}");
    }

    /// A named exit icon, read in two scans.
    fn named_icon(nav: &mut Navigator, dx: f64, dy: f64, to: &str) {
        let icon = ExitIcon { dx, dy, to: Some(to.into()), boss: vec![] };
        nav.icons(std::slice::from_ref(&icon));
        nav.icons(std::slice::from_ref(&icon));
    }

    #[test]
    fn a_straight_way_across_unrecorded_ground_leaves_the_layout_needle() {
        // A U-shaped layout: the exit to A lies 200 units east of B, the corridor
        // there goes 400 south, 200 east and 400 north (5 times the straight line).
        let json = r#"{"connections": ["A", "B"], "topologies": [
            {"id": "u", "nodes": [{"x": 0, "y": 0, "exit": "B"}, {"x": 0, "y": -400}, {"x": 200, "y": -400}, {"x": 200, "y": 0, "exit": "A"}],
             "edges": [{"a": 0, "b": 1}, {"a": 1, "b": 2}, {"a": 2, "b": 3}]}]}"#;
        let area = Area::parse("U", json).unwrap();
        let mut nav = Navigator::new(&area, Arrival::From("B"), None, SCALE, 3).trusted().map_ahead_live();
        let (dx, dy) = super::super::guide::to_screen((200.0 * SCALE, 0.0));
        named_icon(&mut nav, dx, dy, "A");
        let target = [exit("A", "A")];
        let before = nav.guide(&target).unwrap();
        // The layout says round the U, not straight at the icon.
        assert!(angle_between(before.degrees.unwrap(), bearing(dx, dy)) > 45.0, "{before:?}");
        // Nothing recorded: the way would be the straight line across fog. Not taken.
        let empty = HashMap::new();
        let view = nav.guide_on(&target, Some(&lent(&empty, &[], (0, 0), None)));
        assert_eq!(view.as_ref(), Some(&before));
        assert!(!nav.on_map() && nav.last_way().is_none());
    }

    #[test]
    fn a_way_mostly_across_fog_is_never_gold() {
        let area = graph::load("G1_2");
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 3).map_ahead_live();
        named_icon(&mut nav, 0.0, -120.0, "The Grelwood");
        let target = [exit("G1_4", "The Grelwood")];
        // One short recorded wall across the straight line, nothing else: the way
        // bends round its end, so the map changes the answer, but most of it lies
        // over ground nothing was recorded on.
        let stub = walls(line((-20, -60), (20, -60)));
        let view = nav.guide_on(&target, Some(&lent(&stub, &[], (0, 0), None))).unwrap();
        let note = nav.last_way().expect("the map's way");
        println!("{view:?} {note:?}");
        assert!(note.unrecorded > 0.5, "{note:?}");
        assert!(angle_between(view.degrees.unwrap(), -90.0) > 10.0, "{view:?}");
        assert_eq!(view.source, "rough");
        // The look-ahead, as the flight recorder gets it: from the player.
        assert!(angle_between(bearing(note.ahead.0, note.ahead.1), view.degrees.unwrap()) < 0.01);
        // Walked up a recorded corridor 50 cells wide, a pillar ahead: the way goes
        // round it over recorded ground, gold.
        let trail: Vec<[i32; 2]> = (0..=6).map(|k| [0, 60 - 10 * k]).collect();
        let corridor = walls([line((-25, 60), (-25, -160)), line((25, 60), (25, -160)), outline(-8, -50, 8, -34)].concat());
        let view = nav.guide_on(&target, Some(&lent(&corridor, &trail, (0, 0), None))).unwrap();
        let note = nav.last_way().unwrap();
        println!("{view:?} {note:?}");
        assert!(note.unrecorded < RECORDED, "{note:?}");
        assert!(angle_between(view.degrees.unwrap(), -90.0) > 5.0, "round the pillar: {view:?}");
        assert_eq!(view.source, "estimated");
    }

    #[test]
    fn an_arrival_counts_as_the_exit_only_soon_and_without_a_waypoint() {
        let area = graph::load("G1_3");
        let nav = Navigator::new(&area, Arrival::From("G1_2"), None, SCALE, 5);
        let map = walls(line((15, -200), (15, 70)));
        let mut trail: Vec<[i32; 2]> = (0..=10).map(|k| [0, 10 * k]).collect();
        trail.extend((1..=3).map(|k| [10 * k, 100]));
        trail.extend((1..=10).map(|k| [30, 100 - 10 * k]));
        let target = [exit("G1_2", "Clearfell")];
        let ask = |came_in: CameIn| nav.guide_on(&target, Some(&lent(&map, &trail, (30, 0), Some(came_in))));
        let base = CameIn { from: "G1_2", at: (0, 0), delay: 1.0, waypoint: false, exits: &[None] };
        assert_eq!(ask(base).unwrap().source, "estimated");
        // A waypoint by the arrival point, or the first frame 20 s after the zone entry:
        // not the exit's place, the needle as without it.
        assert_eq!(ask(CameIn { waypoint: true, ..base }), nav.guide(&target));
        assert_eq!(ask(CameIn { delay: 20.0, ..base }), nav.guide(&target));
        // No exit icon by it, or one read as another zone's: the way, rough.
        assert_eq!(ask(CameIn { exits: &[], ..base }).unwrap().source, "rough");
        let other = [Some("The Grelwood".to_string())];
        assert_eq!(ask(CameIn { exits: &other, ..base }).unwrap().source, "rough");
        let named = [Some("Clearfell".to_string())];
        assert_eq!(ask(CameIn { exits: &named, ..base }).unwrap().source, "estimated");
    }

    #[test]
    fn a_placement_jump_ends_the_sighting_as_a_target_on_the_map() {
        let area = graph::load("G1_2");
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 3).map_ahead_live();
        named_icon(&mut nav, 0.0, -80.0, "The Grelwood");
        let target = [exit("G1_4", "The Grelwood")];
        let map = walls([line((-200, -40), (59, -40)), line((80, -40), (200, -40))].concat());
        // Out of view, walked 15 cells right in one frame: still the sighting's way.
        nav.icons(&[]);
        nav.moved(15.0, 0.0);
        assert!(nav.guide_on(&target, Some(&lent(&map, &[], (15, 0), None))).is_some() && nav.on_map());
        // A frame placed 100 cells further (a wide re-find, maybe a wrong one): the
        // sighting's place against the walls is off by as much, so no way to it.
        nav.moved(100.0, 0.0);
        let view = nav.guide_on(&target, Some(&lent(&map, &[], (115, 0), None)));
        assert!(!nav.on_map(), "{view:?}");
        assert_eq!(view, nav.guide(&target));
    }

    #[test]
    fn ways_ahead_run_in_shadow() {
        assert!(!MAP_AHEAD_LIVE);
        let area = graph::load("G1_2");
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 3);
        named_icon(&mut nav, 0.0, -110.0, "The Grelwood");
        let target = [exit("G1_4", "The Grelwood")];
        // The pillar in a recorded corridor: live, the way round it would be gold.
        let trail: Vec<[i32; 2]> = (0..=6).map(|k| [0, 60 - 10 * k]).collect();
        let corridor = walls([line((-25, 60), (-25, -160)), line((25, 60), (25, -160)), outline(-8, -50, 8, -34)].concat());
        let view = nav.guide_on(&target, Some(&lent(&corridor, &trail, (0, 0), None)));
        let shadow = nav.last_shadow().expect("the way, in shadow");
        assert!(nav.last_way().is_none());
        // Shown: the needle without the map. Kept: what the map would have shown.
        assert_eq!(view, nav.guide(&target));
        assert!(shadow.gold && shadow.unrecorded < 0.5, "{shadow:?}");
        assert!(angle_between(shadow.degrees.unwrap(), -90.0) > 5.0, "round the pillar: {shadow:?}");
        assert!(shadow.length > 110.0, "{shadow:?}");
        // The way back to a trusted arrival is live (walked ground, no inferred door).
        let area = graph::load("G1_3");
        let nav = Navigator::new(&area, Arrival::From("G1_2"), None, SCALE, 5);
        let map = walls(line((15, -200), (15, 70)));
        let mut trail: Vec<[i32; 2]> = (0..=10).map(|k| [0, 10 * k]).collect();
        trail.extend((1..=3).map(|k| [10 * k, 100]));
        trail.extend((1..=10).map(|k| [30, 100 - 10 * k]));
        let view = nav.guide_on(&[exit("G1_2", "Clearfell")], Some(&lent(&map, &trail, (30, 0), by_exit("G1_2", (0, 0))))).unwrap();
        assert!(nav.last_way().is_some() && nav.last_shadow().is_none());
        assert_eq!(view.source, "estimated");
    }
}
