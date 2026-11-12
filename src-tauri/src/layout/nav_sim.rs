//! Simulated walks to check the navigator against (tests only).
//!
//! A walker follows one layout's corridors from an arrival node to a target with
//! a smooth sideways wander, wrong turns at junctions, a start offset, a fixed
//! scale error, small per-step noise and tracking losses. Each step yields the
//! movement in minimap cells (as the recorder measures it: whole cells) and the
//! true screen bearing of the route ahead. The navigator gets only the movement.
//!
//! Gate A (class check): of all "estimated" (gold) needles, at most 3% are more
//! than 60° off, and no layout has more than 10%. Gate B (coverage): at 60% of
//! the route, at least 85% of runs point within 45° (not reachable on these
//! graphs: see `ambiguity`; the test pins what is reached). Full numbers:
//! `cargo test --release nav_sim::tests::report -- --ignored --nocapture`.

use super::graph::{self, Area, Guidance, Layout, SCALE};
use super::guide::{angle_between, screen_bearing, to_screen, NavTarget};
use super::nav::{Arrival, Navigator};

/// World units the walker moves per step (a minimap frame at running speed).
const STEP: f64 = 20.0;
/// Walking speed, world units per second.
const SPEED: f64 = 60.0;
/// Sideways wander: spread and how far (world units) it takes to change.
const WANDER: f64 = 35.0;
const WANDER_LENGTH: f64 = 120.0;
/// Share of junctions where the walker takes a wrong turn, and how far it goes.
const WRONG_TURN: f64 = 1.0 / 3.0;
const WRONG_MIN: f64 = 150.0;
const WRONG_MAX: f64 = 300.0;
/// Start up to this far from the arrival node.
const START_OFFSET: f64 = 40.0;
/// Fixed scale error per run, up to this share.
const SCALE_ERROR: f64 = 0.06;
/// Per-step position noise (world units).
const STEP_NOISE: f64 = 2.0;
/// Chance per step that tracking is lost (about every 33 s of walking; the
/// Clearfell recording lost track every few minutes), and for how many steps.
const LOSS_CHANCE: f64 = 1.0 / 100.0;
const LOSS_MIN: usize = 5;
const LOSS_MAX: usize = 30;

pub(super) struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ 0xDEAD_BEEF)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn uniform(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.uniform()
    }

    fn gauss(&mut self) -> f64 {
        let r = (-2.0 * (1.0 - self.uniform()).ln()).sqrt();
        r * (std::f64::consts::TAU * self.uniform()).cos()
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Event {
    /// Movement in minimap cells; the true bearing (screen degrees) of the route
    /// ahead after it; the share of the walk done; world units walked so far.
    Moved { dx: f64, dy: f64, truth: Option<f32>, done: f64, walked: f64 },
    /// Tracking lost for this many seconds.
    Lost(f64),
}

/// Shortest node path from `from` to the nearest of `goals`, as (node, edge taken) pairs.
fn node_path(layout: &Layout, from: usize, goals: &[usize]) -> Vec<(usize, Option<usize>)> {
    let dist = layout.distances(goals);
    let mut path = vec![(from, None)];
    let mut at = from;
    while dist[at] > 0.0 && dist[at].is_finite() && path.len() <= layout.nodes.len() {
        let (e, next) = layout
            .edges
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                if e.a == at {
                    Some((i, e.b))
                } else if e.b == at {
                    Some((i, e.a))
                } else {
                    None
                }
            })
            .min_by(|x, y| {
                let c = |&(i, n): &(usize, usize)| layout.edges[i].length() + dist[n];
                c(x).total_cmp(&c(y))
            })
            .unwrap();
        path.push((next, Some(e)));
        at = next;
    }
    path
}

/// The corridor points of edge `e`, walked away from node `from`.
fn edge_points(layout: &Layout, e: usize, from: usize) -> Vec<(f64, f64)> {
    let edge = &layout.edges[e];
    if edge.a == from { edge.points.clone() } else { edge.points.iter().rev().copied().collect() }
}

/// Cuts a polyline after `length` units.
fn cut(points: &[(f64, f64)], length: f64) -> Vec<(f64, f64)> {
    let mut out = vec![points[0]];
    let mut left = length;
    for pair in points.windows(2) {
        let l = (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1);
        if l >= left {
            let t = if l > 0.0 { left / l } else { 0.0 };
            out.push((pair[0].0 + t * (pair[1].0 - pair[0].0), pair[0].1 + t * (pair[1].1 - pair[0].1)));
            return out;
        }
        out.push(pair[1]);
        left -= l;
    }
    out
}

fn incident(layout: &Layout, node: usize) -> Vec<usize> {
    (0..layout.edges.len()).filter(|&i| layout.edges[i].a == node || layout.edges[i].b == node).collect()
}

/// The centre line the walker follows: the route with wrong turns at junctions.
fn centre_line(layout: &Layout, from: usize, goals: &[usize], rng: &mut Rng) -> Vec<(f64, f64)> {
    let path = node_path(layout, from, goals);
    let mut line = vec![(layout.nodes[from].x, layout.nodes[from].y)];
    for (i, &(node, edge)) in path.iter().enumerate() {
        if let Some(e) = edge {
            let prev = path[i - 1].0;
            line.extend(edge_points(layout, e, prev).into_iter().skip(1));
        }
        // A wrong turn at a junction on the way (not at the goal).
        let Some(&(_, Some(onward))) = path.get(i + 1) else { continue };
        let branches: Vec<usize> = incident(layout, node).into_iter().filter(|&e| Some(e) != edge && e != onward).collect();
        if branches.is_empty() || rng.uniform() >= WRONG_TURN {
            continue;
        }
        let mut detour = vec![(layout.nodes[node].x, layout.nodes[node].y)];
        let (mut at, mut e) = (node, branches[(rng.uniform() * branches.len() as f64) as usize % branches.len()]);
        let mut left = rng.range(WRONG_MIN, WRONG_MAX);
        loop {
            let points = edge_points(layout, e, at);
            let l = layout.edges[e].length();
            detour.extend(cut(&points, left).into_iter().skip(1));
            if l >= left {
                break;
            }
            left -= l;
            at = if layout.edges[e].a == at { layout.edges[e].b } else { layout.edges[e].a };
            let next: Vec<usize> = incident(layout, at).into_iter().filter(|&x| x != e).collect();
            if next.is_empty() {
                break;
            }
            e = next[(rng.uniform() * next.len() as f64) as usize % next.len()];
        }
        let back: Vec<(f64, f64)> = detour.iter().rev().skip(1).copied().collect();
        line.extend(detour.into_iter().skip(1));
        line.extend(back);
    }
    line
}

/// Points every `STEP` units along a polyline, with the direction there.
fn resample(line: &[(f64, f64)]) -> Vec<((f64, f64), (f64, f64))> {
    let mut out = Vec::new();
    let mut carry = 0.0;
    for pair in line.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let l = (b.0 - a.0).hypot(b.1 - a.1);
        if l <= 0.0 {
            continue;
        }
        let dir = ((b.0 - a.0) / l, (b.1 - a.1) / l);
        let mut s = carry;
        while s < l {
            out.push(((a.0 + dir.0 * s, a.1 + dir.1 * s), dir));
            s += STEP;
        }
        carry = s - l;
    }
    if let Some(&last) = line.last() {
        let dir = out.last().map_or((1.0, 0.0), |o| o.1);
        out.push((last, dir));
    }
    out
}

pub(super) struct Options {
    /// Movement turned by this many degrees (a planted fault).
    pub turn: f64,
    /// Random tracking losses on.
    pub losses: bool,
    /// One loss of this many seconds from 40% of the walk.
    pub long_loss: Option<f64>,
}

impl Default for Options {
    fn default() -> Self {
        Options { turn: 0.0, losses: true, long_loss: None }
    }
}

/// One simulated walk on `layout` from node `from` to the nearest of `goals`.
pub(super) fn walk(layout: &Layout, guidance: &Guidance, from: usize, goals: &[usize], seed: u64, options: &Options) -> Vec<Event> {
    let mut rng = Rng::new(seed);
    let line = centre_line(layout, from, goals, &mut rng);
    let points = resample(&line);
    let total = (points.len().max(2) - 1) as f64;
    let scale = SCALE * (1.0 + rng.range(-SCALE_ERROR, SCALE_ERROR));
    let (r, a) = (START_OFFSET * rng.uniform().sqrt(), std::f64::consts::TAU * rng.uniform());
    let start = (r * a.cos(), r * a.sin());
    let rho = (-STEP / WANDER_LENGTH).exp();
    let mut wander = rng.gauss() * WANDER;
    let (cos, sin) = (options.turn.to_radians().cos(), options.turn.to_radians().sin());
    let mut events = Vec::new();
    let mut last_cell: Option<(f64, f64)> = None;
    let mut lost_steps = 0usize;
    let mut lost_for = 0usize;
    let long_from = (0.4 * total) as usize;
    let long_steps = options.long_loss.map_or(0, |s| (s * SPEED / STEP) as usize);
    for (i, &(centre, dir)) in points.iter().enumerate() {
        wander = rho * wander + (1.0 - rho * rho).sqrt() * WANDER * rng.gauss();
        // The start offset fades over the first 100 units.
        let fade = (1.0 - i as f64 * STEP / 100.0).max(0.0);
        let normal = (-dir.1, dir.0);
        let at = (
            centre.0 + normal.0 * wander + start.0 * fade + rng.gauss() * STEP_NOISE,
            centre.1 + normal.1 * wander + start.1 * fade + rng.gauss() * STEP_NOISE,
        );
        let screen = to_screen(at);
        let cell = ((screen.0 * scale).round(), (screen.1 * scale).round());
        // Tracking losses: the movement during them is never reported.
        if i == long_from && long_steps > 0 {
            lost_for = long_steps.min(points.len().saturating_sub(i + 3));
            lost_steps = 0;
        } else if options.losses && lost_for == 0 && i > 2 && rng.uniform() < LOSS_CHANCE {
            lost_for = (rng.range(LOSS_MIN as f64, LOSS_MAX as f64 + 1.0) as usize).min(points.len().saturating_sub(i + 3));
            lost_steps = 0;
        }
        if lost_for > 0 {
            lost_steps += 1;
            if lost_steps >= lost_for {
                events.push(Event::Lost(lost_for as f64 * STEP / SPEED));
                lost_for = 0;
            }
            last_cell = None;
            continue;
        }
        let Some(prev) = last_cell.replace(cell) else { continue };
        let (dx, dy) = (cell.0 - prev.0, cell.1 - prev.1);
        let (dx, dy) = (dx * cos - dy * sin, dx * sin + dy * cos);
        // The truth: from where the walker stands to the point ahead on the route.
        let truth = layout.corridor(centre).1.and_then(|j| {
            let (ax, ay) = guidance.ahead[j];
            let (sx, sy) = to_screen((ax as f64 - at.0, ay as f64 - at.1));
            (guidance.togo[j] > 30.0).then(|| screen_bearing(sx, sy)).flatten()
        });
        events.push(Event::Moved { dx, dy, truth, done: i as f64 / total, walked: i as f64 * STEP });
    }
    events
}

/// What one run of the navigator on a simulated walk gave.
#[derive(Default)]
pub(super) struct Outcome {
    pub layout: String,
    /// Per tracked step: (source, error in degrees if both directions exist, share done).
    pub steps: Vec<(String, Option<f64>, f64)>,
    /// World units walked when the top layout became the true one for good.
    pub settled: Option<f64>,
    /// Error at 60% of the walk.
    pub at_60: Option<f64>,
}

/// One run: a walk on layout `k` from the exit to `from` ("" = the start node) to
/// `target`; the navigator is told where the walk starts when `told`.
pub(super) fn run(area: &Area, k: usize, from: &str, told: bool, target: &NavTarget, seed: u64, options: &Options) -> Option<Outcome> {
    let layout = &area.layouts[k];
    let goals = layout.goals(target);
    let node = (0..layout.nodes.len()).find(|&i| {
        let n = &layout.nodes[i];
        if from.is_empty() { n.start } else { n.exit.as_deref() == Some(from) }
    })?;
    // A walk needs a corridor to the goal (a few graphs leave an exit apart).
    if goals.is_empty() || goals.contains(&node) || !layout.distances(&goals)[node].is_finite() {
        return None;
    }
    let guidance = layout.guidance(&goals);
    let events = walk(layout, &guidance, node, &goals, seed, options);
    let arrival = if told { Arrival::From(from) } else { Arrival::Unknown };
    let mut nav = Navigator::new(area, arrival, None, SCALE, seed);
    let mut out = Outcome { layout: layout.id.clone(), ..Default::default() };
    let targets = [target.clone()];
    for event in events {
        match event {
            Event::Lost(seconds) => nav.lost(seconds),
            Event::Moved { dx, dy, truth, done, walked } => {
                nav.moved(dx, dy);
                let view = nav.guide(&targets)?;
                let error = match (view.degrees, truth) {
                    (Some(d), Some(t)) => Some(angle_between(d, t)),
                    _ => None,
                };
                if done >= 0.6 && out.at_60.is_none() {
                    out.at_60 = Some(error.unwrap_or(180.0));
                }
                let top = nav.ranking()[0].0 == layout.id;
                match (top, out.settled) {
                    (true, None) => out.settled = Some(walked),
                    (false, Some(_)) => out.settled = None,
                    _ => {}
                }
                out.steps.push((view.source, error, done));
            }
        }
    }
    Some(out)
}

/// Every simulated run of an area: each layout, each arrival exit (or the start
/// node), each other exit or tag as the target, `seeds` seeds.
pub(super) fn runs(id: &str, seeds: u64, told: bool, options: &Options) -> Vec<Outcome> {
    let area = graph::load(id);
    let mut arrivals: Vec<String> = area.names.keys().cloned().collect();
    arrivals.sort();
    let mut targets: Vec<NavTarget> =
        arrivals.iter().map(|e| NavTarget { label: e.clone(), exit: Some(e.clone()), ..Default::default() }).collect();
    if arrivals.len() < 2 {
        // One exit only (The Riverbank): also walk from the start, and back to it.
        arrivals.push(String::new());
        targets.push(NavTarget { label: "start".into(), tag: Some("entrance".into()), ..Default::default() });
    }
    let mut out = Vec::new();
    for k in 0..area.layouts.len() {
        for (a, from) in arrivals.iter().enumerate() {
            for (t, target) in targets.iter().enumerate() {
                for seed in 0..seeds {
                    let seed = seed * 7919 + k as u64 * 104_729 + a as u64 * 31 + t as u64 * 7;
                    out.extend(run(&area, k, from, told, target, seed, options));
                }
            }
        }
    }
    out
}

/// The class check's numbers.
#[derive(Debug)]
pub(super) struct Check {
    pub runs: usize,
    pub steps: usize,
    pub gold: usize,
    /// Gold needles more than 60° off.
    pub gold_off: usize,
    /// The worst layout: (id, share of its gold needles more than 60° off, its gold count).
    pub worst: (String, f64, usize),
    /// Share of runs pointing within 45° at 60% of the walk.
    pub coverage: f64,
    /// Median world units walked until the top layout is the true one for good (None: never in most runs).
    pub settled_median: Option<f64>,
    pub settled_share: f64,
}

impl Check {
    pub fn off_share(&self) -> f64 {
        self.gold_off as f64 / self.gold.max(1) as f64
    }

    pub fn gold_share(&self) -> f64 {
        self.gold as f64 / self.steps.max(1) as f64
    }

    /// Gate A: at most 3% of gold needles more than 60° off, no layout above 10%.
    pub fn class_ok(&self) -> bool {
        self.off_share() <= 0.03 && self.worst.1 <= 0.10
    }
}

pub(super) fn check(outcomes: &[Outcome]) -> Check {
    use std::collections::BTreeMap;
    let mut per_layout: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let (mut steps, mut gold, mut off) = (0, 0, 0);
    for o in outcomes {
        for (source, error, _) in &o.steps {
            steps += 1;
            if source != "estimated" {
                continue;
            }
            let Some(error) = error else { continue };
            gold += 1;
            let entry = per_layout.entry(&o.layout).or_default();
            entry.0 += 1;
            if *error > 60.0 {
                off += 1;
                entry.1 += 1;
            }
        }
    }
    // A layout with only a handful of gold needles says little: judge it from 20 on.
    let worst = per_layout
        .iter()
        .filter(|(_, (g, _))| *g >= 20)
        .map(|(id, (g, o))| (id.to_string(), *o as f64 / *g as f64, *g))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap_or_default();
    let covered = outcomes.iter().filter(|o| o.at_60.is_some_and(|e| e <= 45.0)).count();
    let mut settled: Vec<f64> = outcomes.iter().map(|o| o.settled.unwrap_or(f64::INFINITY)).collect();
    settled.sort_by(f64::total_cmp);
    let settled_median = settled.get(settled.len() / 2).copied().filter(|d| d.is_finite());
    let settled_share = settled.iter().filter(|d| d.is_finite()).count() as f64 / settled.len().max(1) as f64;
    Check {
        runs: outcomes.len(),
        steps,
        gold,
        gold_off: off,
        worst,
        coverage: covered as f64 / outcomes.len().max(1) as f64,
        settled_median,
        settled_share,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(seeds: u64, told: bool, options: &Options) -> Vec<Outcome> {
        let mut out = runs("G1_1", seeds, told, options);
        out.extend(runs("G1_2", seeds, told, options));
        out
    }

    fn print(name: &str, c: &Check) {
        eprintln!(
            "{name}: {} runs, {} steps, gold {:.1}%, gold >60° off {:.2}% ({} of {}), worst layout {} {:.1}% of {}, \
             within 45° at 60%: {:.1}%, top layout true for good: {:.0}% of runs, median after {:?} units",
            c.runs, c.steps, 100.0 * c.gold_share(), 100.0 * c.off_share(), c.gold_off, c.gold, c.worst.0,
            100.0 * c.worst.1, c.worst.2, 100.0 * c.coverage, 100.0 * c.settled_share, c.settled_median.map(|d| d.round()),
        );
    }

    #[test]
    fn the_walker_follows_the_corridors_and_reports_the_truth() {
        let area = graph::load("G1_2");
        let layout = area.layouts.iter().find(|l| l.id == "ClearFell_W_S_N").unwrap();
        let target = NavTarget { exit: Some("G1_3".into()), ..Default::default() };
        let goals = layout.goals(&target);
        let guidance = layout.guidance(&goals);
        let from = layout.goals(&NavTarget { exit: Some("G1_town".into()), ..Default::default() })[0];
        let events = walk(layout, &guidance, from, &goals, 3, &Options { losses: false, ..Default::default() });
        // Summed movement turned back to world units ends near the goal.
        let (mut x, mut y) = (0.0, 0.0);
        for e in &events {
            if let Event::Moved { dx, dy, .. } = e {
                (x, y) = (x + dx, y + dy);
            }
        }
        let (wx, wy) = super::super::guide::from_screen((x / SCALE, y / SCALE));
        let (start, goal) = (&layout.nodes[from], &layout.nodes[goals[0]]);
        let (ex, ey) = (goal.x - start.x, goal.y - start.y);
        assert!((wx - ex).hypot(wy - ey) < 0.1 * ex.hypot(ey) + 80.0, "ended {wx:.0},{wy:.0}, goal {ex:.0},{ey:.0}");
        assert!(events.iter().filter(|e| matches!(e, Event::Moved { truth: Some(_), .. })).count() > 20);
    }

    /// Gate B asks for 85% within 45° at 60% of the walk; the graphs allow less:
    /// a rival layout still fits under the walk until a median 59% of it (see
    /// `ambiguity`), and the walked line alone may not claim more than 60% for one
    /// layout. This pins what is reached without exit icons (42%).
    const COVERAGE: f64 = 0.35;

    #[test]
    fn gates_hold_on_simulated_walks() {
        let outcomes = all(1, true, &Options::default());
        let c = check(&outcomes);
        print("arrival known", &c);
        assert!(c.class_ok(), "gate A: {c:?}");
        assert!(c.coverage >= COVERAGE, "gate B: {c:?}");
    }

    #[test]
    fn the_class_check_bites_on_turned_movement() {
        let outcomes = all(1, true, &Options { turn: 90.0, ..Default::default() });
        let c = check(&outcomes);
        print("movement turned 90°", &c);
        assert!(!c.class_ok(), "{c:?}");
    }

    #[test]
    #[ignore]
    fn report() {
        let seeds = std::env::var("SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(3);
        let c = check(&all(seeds, true, &Options::default()));
        print("arrival known", &c);
        print("arrival unknown", &check(&all(seeds, false, &Options::default())));
        print("30 s loss at 40%", &check(&all(seeds, true, &Options { long_loss: Some(30.0), ..Default::default() })));
        print("movement turned 90°", &check(&all(seeds, true, &Options { turn: 90.0, ..Default::default() })));
        print("no tracking losses", &check(&all(seeds, true, &Options { losses: false, ..Default::default() })));
        assert!(c.class_ok() && c.coverage >= COVERAGE);
    }

    /// How far a walk goes before each rival layout (entered at its own exit of the
    /// same name) no longer has corridors under it: the geometric limit of telling them apart.
    #[test]
    #[ignore]
    fn ambiguity() {
        let area = graph::load("G1_2");
        let mut lasts = Vec::new();
        let mut counts = Vec::new();
        let mut never_share = 0usize;
        let mut total = 0usize;
        for k in 0..area.layouts.len() {
            let layout = &area.layouts[k];
            for from in ["G1_town", "G1_3", "G1_4"] {
                for to in ["G1_town", "G1_3", "G1_4"] {
                    if from == to { continue; }
                    let target = NavTarget { exit: Some(to.into()), ..Default::default() };
                    let goals = layout.goals(&target);
                    let node = layout.goals(&NavTarget { exit: Some(from.into()), ..Default::default() })[0];
                    if !layout.distances(&goals)[node].is_finite() { continue; }
                    let line = centre_line(layout, node, &goals, &mut Rng::new(k as u64));
                    let pts = resample(&line);
                    let origin = (layout.nodes[node].x, layout.nodes[node].y);
                    let mut splits = Vec::new();
                    for (r, rival) in area.layouts.iter().enumerate() {
                        if r == k { continue; }
                        let starts = rival.goals(&NavTarget { exit: Some(from.into()), ..Default::default() });
                        // The longest a rival keeps the walk on its corridors, from any of its arrival nodes.
                        let keep = starts.iter().map(|&s| {
                            let o = (rival.nodes[s].x, rival.nodes[s].y);
                            pts.iter().position(|&((x, y), _)| rival.corridor((o.0 + x - origin.0, o.1 + y - origin.1)).0 > 80.0)
                                .map_or(f64::INFINITY, |i| i as f64 * STEP)
                        }).fold(0.0, f64::max);
                        splits.push(keep);
                    }
                    total += 1;
                    let walked = (pts.len() - 1) as f64 * STEP;
                    let finite: Vec<f64> = splits.iter().copied().filter(|d| d.is_finite()).collect();
                    never_share += splits.len() - finite.len();
                    let last = finite.iter().copied().fold(0.0, f64::max);
                    lasts.push(last / walked);
                    counts.push(splits.iter().filter(|d| !d.is_finite()).count());
                }
            }
        }
        lasts.sort_by(f64::total_cmp);
        eprintln!("{total} walks; rivals that fit the whole walk: {never_share} (per walk {:?})", counts);
        eprintln!("share of the walk until the last other-fitting rival drops out: median {:.2}, 75% {:.2}, 90% {:.2}",
            lasts[lasts.len() / 2], lasts[lasts.len() * 3 / 4], lasts[lasts.len() * 9 / 10]);
    }
}
