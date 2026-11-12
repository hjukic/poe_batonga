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
/// Needle sources: `|V|` at least `GOLD` (and `GOLD_WALK` units walked or an
/// anchor seen) is "estimated", at least `ROUGH` is "rough".
const GOLD: f64 = 0.8;
const ROUGH: f64 = 0.5;
const GOLD_WALK: f64 = 100.0;
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
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Navigator {
    pub fn new(area: &Area, arrival: Arrival, memory: Option<&Memory>, scale: f64, seed: u64) -> Self {
        let mut rng = Rng(seed ^ 0x5EED_0F_BA70_4A);
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
        self.pending = (self.pending.0 + wx, self.pending.1 + wy);
        self.pending_sq += step * step;
        let keep = (-step / HEADING_LENGTH).exp();
        self.heading = (self.heading.0 * keep + wx, self.heading.1 * keep + wy);
        self.walked += step;
        if step > 0.0 {
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
        let Navigator { area, banks, rng, .. } = self;
        for ((layout, bank), goals) in area.layouts.iter().zip(banks.iter_mut()).zip(&goals) {
            if goals.is_empty() {
                bank.evidence += MISSING;
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
        let (top, p_top) = posterior.iter().enumerate().fold((0, 0.0), |b, (i, &p)| if p > b.1 { (i, p) } else { b });
        let source = if confidence >= GOLD && (self.walked >= GOLD_WALK || self.anchored) {
            "estimated"
        } else if confidence >= ROUGH {
            "rough"
        } else {
            "unconfirmed"
        };
        Some(GuideView {
            variant: if p_top >= SURE { self.area.layouts[top].id.clone() } else { String::new() },
            target_kind: if target.exit.is_some() { "entrance".into() } else { target.tag.clone().unwrap_or_default() },
            target: target.label.clone(),
            degrees: mode(&bins).or_else(|| screen_bearing(vx, vy)),
            source: source.into(),
            distance: if weight > 0.0 { (togo / weight * self.scale) as f32 } else { 0.0 },
            confidence: confidence.min(1.0) as f32,
            measured: false,
            turned: false,
        })
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
        Memory {
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
        let Navigator { area, banks, rng, .. } = self;
        for (layout, bank) in area.layouts.iter().zip(banks.iter_mut()) {
            let mut fit = 0.0;
            for p in &mut bank.particles {
                let like = closeness(layout.corridor((p.x, p.y)).0);
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
fn closeness(d: f64) -> f64 {
    let out = (d - FLAT).max(0.0);
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
        walk(l, &l.guidance(&goals), start, &goals, seed, &Options { losses: false, ..Default::default() })
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
        assert_eq!(closeness(area.layouts[0].corridor((far.x, far.y)).0), FLOOR);
        assert_eq!(closeness(10.0), 1.0);
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
        let mut nav = Navigator::new(&area, Arrival::Unknown, None, SCALE, 3);
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
