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
use super::guide::{screen_bearing, GuideView, NavTarget};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
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
/// Frames three a second see the same icons: a new set of icons counts in full;
/// the same set again only after the player walked this far (world units) since
/// the last update, and then for this much.
const ICON_EVERY: f64 = 60.0;
const ICON_TEMPER: f64 = 0.5;
/// Share of particles redrawn where the icons put the player, when the belief
/// does not explain them.
const ICON_INJECT: f64 = 0.3;
/// The needle points straight at the target's icon only when the route there is
/// at most this many times the straight distance.
const ICON_STRAIGHT: f64 = 1.3;
/// The needle at an icon is gold only this close (minimap cells): farther, walls
/// and dead ends between make the straight line a poor guide (simulation).
const ICON_GOLD_WITHIN: f64 = 200.0;
/// An icon update confirms the belief (makes gold possible) when the belief put
/// at least this share on the icons landing on exits.
const ICON_CONFIRM: f64 = 0.5;
/// The needle points at an exit icon once this share of the belief says it is the
/// target's exit (a labelled icon says so itself).
const ICON_SURE: f64 = 0.95;

/// Icons this close (world units) on the dead-reckoned track are one exit.
const SIGHT_MATCH: f64 = 40.0;

/// One exit seen on the map since the last loss.
#[derive(Clone, Debug)]
struct Sighting {
    at: (f64, f64),
    /// The label read most recently, and in how many scans in a row it was the same.
    name: Option<String>,
    scans: u32,
    /// Its label was counted in an update.
    labelled: bool,
}

/// An exit icon of a map frame: its offset from the player in minimap cells
/// (screen axes) and, when its label could be read, the area it leads to (an
/// area id or the zone's name). Unlabelled it is "some exit".
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ExitIcon {
    pub dx: f64,
    pub dy: f64,
    pub to: Option<String>,
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
}

/// What is worth keeping of a zone instance between visits.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Memory {
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
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning { gold: GOLD, gold_layout: GOLD_LAYOUT, gold_walk: GOLD_WALK, missing: MISSING, flat: FLAT }
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
    /// Where the player is by dead reckoning since the last loss (world units,
    /// nominal scale): gives each icon a place, so one sighting counts once.
    track: (f64, f64),
    /// Units walked at the last icon update.
    icon_walked: f64,
    /// Exits seen per (layout, node): where the icons put them (file units).
    exits_seen: HashMap<(usize, usize), (f64, f64)>,
    tuning: Tuning,
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Navigator {
    pub fn new(area: &Area, arrival: Arrival, memory: Option<&Memory>, scale: f64, seed: u64) -> Self {
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
                let anchors = evidence.map_or(0.0, |m| m.anchors.get(&layout.id).copied().unwrap_or(0.0));
                let mut bank = Bank { prior, path, evidence: anchors, particles };
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
            track: (0.0, 0.0),
            icon_walked: 0.0,
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
    /// or every exit when it has none.
    fn icon_nodes(&self, layout: &Layout, icon: &ExitIcon) -> Vec<usize> {
        let to = icon.to.as_deref().map(|t| self.exit_id(t));
        (0..layout.nodes.len())
            .filter(|&i| match (&layout.nodes[i].exit, to) {
                (Some(exit), Some(to)) => exit == to,
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
        let (mut new, mut upgraded) = (false, false);
        let mut now = Vec::with_capacity(icons.len());
        for icon in icons {
            let (wx, wy) = super::guide::from_screen((icon.dx / self.scale, icon.dy / self.scale));
            let at = (self.track.0 + wx, self.track.1 + wy);
            let found = self.sightings.iter().position(|s| (s.at.0 - at.0).hypot(s.at.1 - at.1) <= SIGHT_MATCH);
            let i = match found {
                Some(i) => i,
                None => {
                    new = true;
                    self.sightings.push(Sighting { at, name: None, scans: 0, labelled: icon.to.is_some() });
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
        let Navigator { area, banks, rng, exits_seen, .. } = self;
        let mut learnt = Vec::new();
        for (k, (layout, bank)) in area.layouts.iter().zip(banks.iter_mut()).enumerate() {
            let cands = &candidates[k];
            let at = |n: usize| exits_seen.get(&(k, n)).copied().unwrap_or((layout.nodes[n].x, layout.nodes[n].y));
            // Where an icon is from a particle, in file units.
            let offset = |icon: &ExitIcon, s: f64| {
                let (x, y) = super::guide::from_screen((icon.dx / (scale * s), icon.dy / (scale * s)));
                (x / layout.unit, y / layout.unit)
            };
            let like = |p: &Particle| -> f64 {
                icons
                    .iter()
                    .zip(cands)
                    .map(|(icon, nodes)| {
                        let (ox, oy) = offset(icon, p.s);
                        let d2 = nodes
                            .iter()
                            .map(|&n| {
                                let (x, y) = at(n);
                                ((p.x + ox - x).powi(2) + (p.y + oy - y).powi(2)) * layout.unit * layout.unit
                            })
                            .fold(f64::INFINITY, f64::min);
                        ICON_FLOOR + (1.0 - ICON_FLOOR) * (-d2 / (2.0 * ICON_TOL * ICON_TOL)).exp()
                    })
                    .product()
            };
            let agree: f64 = bank.particles.iter().map(|p| p.w * like(p)).sum();
            confirmed += before[k] * agree;
            // Two icons: does any pair of this layout's exits lie as far apart? This
            // needs no position belief at all.
            let mut geometry = 0.0;
            if icons.len() >= 2 {
                geometry = 1.0;
                let world = |i: &ExitIcon| super::guide::from_screen((i.dx / scale, i.dy / scale));
                for j in 1..icons.len() {
                    let (a, b) = (world(&icons[0]), world(&icons[j]));
                    let want = (b.0 - a.0, b.1 - a.1);
                    let mut best = 0.0f64;
                    for &n1 in &cands[0] {
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
                    geometry *= best;
                }
            }
            bank.evidence += weight * agree.max(geometry).max(ICON_BASE).ln();
            // The belief does not explain the icons: also try where they put the player.
            if agree < 0.3 {
                let labelled: Vec<usize> = (0..icons.len()).filter(|&j| icons[j].to.is_some() && !cands[j].is_empty()).collect();
                let any: Vec<usize> = (0..icons.len()).filter(|&j| !cands[j].is_empty()).collect();
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
                p.w *= like(p);
            }
            normalise(&mut bank.particles);
            let ess = 1.0 / bank.particles.iter().map(|p| p.w * p.w).sum::<f64>();
            if ess < PARTICLES as f64 / 2.0 {
                resample(&mut bank.particles, rng);
            }
            // Remember where each icon put its exit when the belief agrees on which it is.
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
                    if w >= ICON_SURE {
                        learnt.push(((k, n), (x / w, y / w)));
                    }
                }
            }
        }
        exits_seen.extend(learnt);
        // Absolute evidence (gold allowed) only when the belief held before a new
        // sighting predicted it: an icon that reshuffles a diffuse belief, or the
        // same sighting again, confirms nothing.
        if new && confirmed >= ICON_CONFIRM {
            self.anchored = true;
        }
    }

    /// The area id an exit label names: the area whose name it is, else the label itself.
    fn exit_id<'s>(&'s self, label: &'s str) -> &'s str {
        self.area.names.iter().find(|(id, name)| id.as_str() == label || name.eq_ignore_ascii_case(label)).map_or(label, |(id, _)| id.as_str())
    }

    /// The share of the belief that puts icon `icon` on one of the target's exits.
    fn icon_is_target(&self, icon: &ExitIcon, target: &NavTarget, posterior: &[f64]) -> f64 {
        if let (Some(to), Some(exit)) = (&icon.to, &target.exit) {
            return if self.exit_id(to) == exit.as_str() { 1.0 } else { 0.0 };
        }
        let mut share = 0.0;
        for (k, (layout, bank)) in self.area.layouts.iter().zip(&self.banks).enumerate() {
            if posterior[k] < NEGLIGIBLE {
                continue;
            }
            let goals = layout.goals(target);
            let nodes = self.icon_nodes(layout, icon);
            let at = |n: usize| self.exits_seen.get(&(k, n)).copied().unwrap_or((layout.nodes[n].x, layout.nodes[n].y));
            let mut hit = 0.0;
            for p in &bank.particles {
                let (x, y) = self.position(layout, p);
                let (ox, oy) = super::guide::from_screen((icon.dx / (self.scale * p.s), icon.dy / (self.scale * p.s)));
                let q = (x + ox / layout.unit, y + oy / layout.unit);
                let near = nodes.iter().copied().map(|n| {
                    let (nx, ny) = at(n);
                    (n, ((q.0 - nx).powi(2) + (q.1 - ny).powi(2)).sqrt() * layout.unit)
                }).min_by(|a, b| a.1.total_cmp(&b.1));
                if near.is_some_and(|(n, d)| d <= 2.0 * ICON_TOL && goals.contains(&n)) {
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
        if let Seen::Left(_) = seen {
            self.touched();
            for ((layout, bank), goals) in self.area.layouts.iter().zip(self.banks.iter_mut()).zip(&goals) {
                if goals.is_empty() {
                    continue;
                }
                let agree = agreement(layout, &bank.particles, goals, ANCHOR_SIGMA / layout.unit);
                bank.evidence += (LEFT_FLOOR + (1.0 - LEFT_FLOOR) * agree).ln();
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
        let logs: Vec<f64> = self.banks.iter().map(|b| b.prior + b.path + b.evidence).collect();
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

    /// The needle to the first of `targets` that a layout of this zone has.
    pub fn guide(&self, targets: &[NavTarget]) -> Option<GuideView> {
        let target = targets.iter().find(|t| self.area.layouts.iter().any(|l| !l.goals(t).is_empty()))?;
        let posterior = self.posterior();
        let (top, p_top) = posterior.iter().enumerate().fold((0, 0.0), |b, (i, &p)| if p > b.1 { (i, p) } else { b });
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
        let source = if confidence >= t.gold && p_top >= t.gold_layout && (self.walked >= t.gold_walk || self.anchored) {
            "estimated"
        } else if confidence >= ROUGH {
            "rough"
        } else {
            "unconfirmed"
        };
        let source = self.cap_source(source, &posterior);
        let route_cells = if weight > 0.0 { togo / weight * self.scale } else { 0.0 };
        // The target's own exit in view: point at it.
        let in_view = self
            .icons_now
            .iter()
            .map(|(icon, read_twice)| (icon, *read_twice, self.icon_is_target(icon, target, &posterior)))
            .max_by(|a, b| a.2.total_cmp(&b.2));
        // … but only where the way there is about straight: a river or a wall between
        // makes the route bend, and pointing across it was wrong in the simulation,
        // labelled or not (8.6% of such gold needles over 60° off). Then the route's
        // needle stays; the icon has pinned the position, so it is good too.
        let route_degrees = mode(&bins).or_else(|| screen_bearing(vx, vy));
        let straight = |icon: &ExitIcon| route_cells <= ICON_STRAIGHT * icon.dx.hypot(icon.dy) || route_cells <= 0.0;
        if let Some((icon, read_twice, share)) = in_view.filter(|(icon, _, share)| *share >= ICON_SURE && straight(icon)) {
            // Gold when the label says it is the target's exit (read the same in two
            // scans, and not against a sure belief), or when one layout carries the
            // belief (an unlabelled icon could be another exit of a layout not ruled
            // out); and only within ICON_GOLD_WITHIN.
            let unlabelled = ExitIcon { to: None, ..icon.clone() };
            let against = icon.to.is_some() && p_top >= GOLD_LAYOUT && self.icon_is_target(&unlabelled, target, &posterior) < 0.5;
            let named = icon.to.is_some() && read_twice && !against;
            let near = icon.dx.hypot(icon.dy) <= ICON_GOLD_WITHIN;
            let sure = if near && !against && (named || p_top >= GOLD_LAYOUT) { "estimated" } else { "rough" };
            return Some(GuideView {
                variant: if p_top >= SURE { self.area.layouts[top].id.clone() } else { String::new() },
                target_kind: if target.exit.is_some() { "entrance".into() } else { target.tag.clone().unwrap_or_default() },
                target: target.label.clone(),
                degrees: screen_bearing(icon.dx, icon.dy),
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
            exits,
            layouts: ids().zip(posterior).collect(),
            path: ids().zip(self.banks.iter().map(|b| b.path)).collect(),
            anchors: ids().zip(self.banks.iter().map(|b| b.evidence)).collect(),
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
            Seen::Exit(area) | Seen::Left(area) => layout.nodes[i].exit.as_deref() == Some(area),
            Seen::Tag(tag) => !tag.is_empty() && layout.nodes[i].has_tag(tag),
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
            let icon = |to: Option<&str>| ExitIcon { dx: sx * SCALE, dy: sy * SCALE, to: to.map(String::from) };
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
        ExitIcon { dx: sx * SCALE, dy: sy * SCALE, to: None }
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
        nav.icons(&[wood, mud]);
        assert_eq!(nav.ranking()[0].1, before);
        // The exits seen are kept.
        assert!(nav.memory().exits.contains_key("ClearFell_W_E_N"));
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

    /// The owner's live case (N7): Clearfell 2693856399, app restarted (arrival
    /// unknown, no memory), big map open, the Grelwood exit icon at (−136, +83)
    /// cells; the wider capture also sees Mud Burrow at (823, 110).
    #[test]
    fn the_live_clearfell_case() {
        let area = graph::load("G1_2");
        let target = [exit("G1_4", "The Grelwood")];
        let grelwood = ExitIcon { dx: -136.0, dy: 83.0, to: None };
        let mud = ExitIcon { dx: 823.0, dy: 110.0, to: None };
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
        assert!(super::super::guide::angle_between(one_named.degrees.unwrap(), screen_bearing(-136.0, 83.0).unwrap()) < 15.0, "{one_named:?}");
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
}
