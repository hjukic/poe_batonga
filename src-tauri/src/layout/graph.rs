//! The game's own layout graphs (`data/layouts/ggg/`, datamined).
//!
//! Each zone has a few possible layouts. A layout is a graph: fixed points (zone
//! exits, boss arenas, …) joined by corridors, in world units for outdoor zones
//! (x east, y north). Walls are generated around the corridors per instance, so
//! the player walks along the corridors of the layout being played. This file
//! loads the graphs (lazily, per area), measures how far a point is from the
//! corridors and works out, for a target, which way the corridors lead to it.
//! The navigator (`nav.rs`) decides which layout and where the player is.

use super::guide::{area_order, NavTarget};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};


/// World units per file unit of a dungeon room grid ("dgr") without a
/// `roomUnitSize` (with one: `roomUnitSize - 1`, derived from the room files, not
/// checked in play). The navigator gives dungeon particles a wide scale prior.
const DGR_UNIT: f64 = 8.0;
/// Grid cell of the nearest-corridor field, in world units.
const CELL: f64 = 8.0;
/// Room around the corridors in the field.
const MARGIN: f64 = 200.0;
/// The needle's distance counts at most this far from a corridor to it.
pub(super) const FAR: f64 = 150.0;
/// Spacing of the points sampled along the corridors, in world units.
const SPACING: f64 = 4.0;
/// Minimap cells per world unit, measured on a Clearfell Encampment visit.
pub const SCALE: f64 = 0.78;
/// How far along the corridors the needle looks (world units).
pub(super) const LOOK: f64 = 120.0;
/// Bosses that wait at a zone exit: (area, exit area, boss) until the boss is ticked.
const BOSS_AT_EXIT: [(&str, &str, &str); 1] = [("G1_1", "G1_town", "The Bloated Miller")];

// The files also carry `areaId`, `name`, `act`, `level` (area) and `file`, `kind`
// (layouts, edges); serde skips what is not read here.
#[derive(Deserialize)]
struct AreaFile {
    #[serde(default)]
    connections: Vec<String>,
    #[serde(rename = "connectionNames", default)]
    connection_names: Vec<String>,
    topologies: Vec<LayoutFile>,
}

#[derive(Deserialize)]
struct LayoutFile {
    id: String,
    /// "tgr" (outdoor, world units) or "dgr" (dungeon room grid); absent = "tgr".
    #[serde(default)]
    format: Option<String>,
    /// Dungeons: tiles per room cell.
    #[serde(rename = "roomUnitSize", default)]
    room_unit_size: Option<f64>,
    nodes: Vec<Node>,
    edges: Vec<EdgeFile>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub(super) struct Node {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub tag: Option<String>,
    /// More names for the node (the game's keys), matched like `tag`.
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub exit: Option<String>,
    #[serde(default)]
    pub start: bool,
}

impl Node {
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tag.as_deref() == Some(tag) || self.keys.iter().any(|k| k == tag)
    }
}

#[derive(Deserialize)]
struct EdgeFile {
    a: usize,
    b: usize,
    #[serde(default)]
    via: Vec<[f64; 2]>,
}

/// A corridor: points from node `a` to node `b`, with the arc length at each point.
pub(super) struct Edge {
    pub a: usize,
    pub b: usize,
    pub points: Vec<(f64, f64)>,
    along: Vec<f64>,
}

impl Edge {
    fn new(a: usize, b: usize, points: Vec<(f64, f64)>) -> Edge {
        let mut along = vec![0.0];
        for pair in points.windows(2) {
            along.push(along.last().unwrap() + (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1));
        }
        Edge { a, b, points, along }
    }

    pub fn length(&self) -> f64 {
        *self.along.last().unwrap_or(&0.0)
    }

    /// The point `s` units along the corridor from `a`.
    pub fn at(&self, s: f64) -> (f64, f64) {
        let s = s.clamp(0.0, self.length());
        let j = self.along.partition_point(|&t| t < s).clamp(1, self.points.len().max(2) - 1);
        let (p, q) = (self.points[j - 1], self.points[j]);
        let span = self.along[j] - self.along[j - 1];
        let t = if span > 0.0 { (s - self.along[j - 1]) / span } else { 0.0 };
        (p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1))
    }
}

/// A point on a corridor.
#[derive(Clone, Copy)]
pub(super) struct Sample {
    pub at: (f64, f64),
    pub edge: u32,
    /// Arc length from the edge's node `a`.
    pub s: f32,
}

/// For every grid cell, the corridor sample nearest to its centre.
struct Field {
    origin: (f64, f64),
    /// Cell size in file units.
    cell: f64,
    cols: usize,
    rows: usize,
    near: Vec<u32>,
}

impl Field {
    /// `unit`: world units per file unit.
    fn new(lo: (f64, f64), hi: (f64, f64), samples: &[Sample], unit: f64) -> Field {
        let (cell, margin) = (CELL / unit, MARGIN / unit);
        let origin = (lo.0 - margin, lo.1 - margin);
        let cols = ((hi.0 - lo.0 + 2.0 * margin) / cell).ceil() as usize + 1;
        let rows = ((hi.1 - lo.1 + 2.0 * margin) / cell).ceil() as usize + 1;
        let centre = |c: usize, r: usize| (origin.0 + (c as f64 + 0.5) * cell, origin.1 + (r as f64 + 0.5) * cell);
        let gap = |cell: (f64, f64), j: u32| {
            let p = samples[j as usize].at;
            (cell.0 - p.0).hypot(cell.1 - p.1)
        };
        let mut near = vec![u32::MAX; cols * rows];
        for (j, sample) in samples.iter().enumerate() {
            let c = ((sample.at.0 - origin.0) / cell).floor() as usize;
            let r = ((sample.at.1 - origin.1) / cell).floor() as usize;
            let i = r * cols + c;
            if near[i] == u32::MAX || gap(centre(c, r), j as u32) < gap(centre(c, r), near[i]) {
                near[i] = j as u32;
            }
        }
        // Two sweeps that hand each cell its neighbours' nearest sample when closer.
        let mut sweep = |order: &mut dyn Iterator<Item = (usize, usize)>, from: [(i64, i64); 4]| {
            for (r, c) in order {
                let here = centre(c, r);
                let mut best = near[r * cols + c];
                let mut best_gap = if best == u32::MAX { f64::MAX } else { gap(here, best) };
                for (dr, dc) in from {
                    let (nr, nc) = (r as i64 + dr, c as i64 + dc);
                    if nr < 0 || nc < 0 || nr as usize >= rows || nc as usize >= cols {
                        continue;
                    }
                    let j = near[nr as usize * cols + nc as usize];
                    if j != u32::MAX && gap(here, j) < best_gap {
                        (best, best_gap) = (j, gap(here, j));
                    }
                }
                near[r * cols + c] = best;
            }
        };
        sweep(&mut (0..rows).flat_map(|r| (0..cols).map(move |c| (r, c))), [(0, -1), (-1, -1), (-1, 0), (-1, 1)]);
        sweep(&mut (0..rows).rev().flat_map(|r| (0..cols).rev().map(move |c| (r, c))), [(0, 1), (1, 1), (1, 0), (1, -1)]);
        Field { origin, cell, cols, rows, near }
    }

    fn near(&self, (x, y): (f64, f64)) -> Option<usize> {
        let (col, row) = ((x - self.origin.0) / self.cell, (y - self.origin.1) / self.cell);
        if !(col >= 0.0 && row >= 0.0 && (col as usize) < self.cols && (row as usize) < self.rows) {
            return None;
        }
        let j = self.near[row as usize * self.cols + col as usize];
        (j != u32::MAX).then_some(j as usize)
    }
}

/// A layout's points are in its file's units: world units for "tgr", room-grid
/// units for "dgr". Distances it reports are in world units.
pub(super) struct Layout {
    pub id: String,
    /// A dungeon room grid ("dgr"): its units are not world units.
    pub dungeon: bool,
    /// World units per file unit (a guess for dungeons).
    pub unit: f64,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub samples: Vec<Sample>,
    field: Field,
}

/// Which way the corridors lead to one target, for every corridor sample.
pub(super) struct Guidance {
    /// The point `LOOK` along the route from the sample (the goal itself when nearer).
    pub ahead: Vec<(f32, f32)>,
    /// Route length (world units) from the sample to the goal; the straight distance
    /// when no corridor leads there, infinite without a goal.
    pub togo: Vec<f32>,
}

impl Layout {
    fn new(file: LayoutFile) -> Option<Layout> {
        let n = file.nodes.len();
        if n < 2 {
            return None;
        }
        let edges: Vec<Edge> = file
            .edges
            .iter()
            .filter(|e| e.a < n && e.b < n && e.a != e.b)
            .map(|e| {
                let mut points = vec![(file.nodes[e.a].x, file.nodes[e.a].y)];
                points.extend(e.via.iter().map(|p| (p[0], p[1])));
                points.push((file.nodes[e.b].x, file.nodes[e.b].y));
                Edge::new(e.a, e.b, points)
            })
            .collect();
        if edges.is_empty() {
            return None;
        }
        let dungeon = file.format.as_deref() == Some("dgr");
        let unit = match (dungeon, file.room_unit_size) {
            (false, _) => 1.0,
            (true, Some(size)) => (size - 1.0).max(1.0),
            (true, None) => DGR_UNIT,
        };
        let mut samples = Vec::new();
        for (index, edge) in edges.iter().enumerate() {
            let steps = (edge.length() * unit / SPACING).ceil().max(1.0) as usize;
            for k in 0..=steps {
                let s = edge.length() * k as f64 / steps as f64;
                samples.push(Sample { at: edge.at(s), edge: index as u32, s: s as f32 });
            }
        }
        let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
        for s in &samples {
            (lo, hi) = ((lo.0.min(s.at.0), lo.1.min(s.at.1)), (hi.0.max(s.at.0), hi.1.max(s.at.1)));
        }
        let field = Field::new(lo, hi, &samples, unit);
        Some(Layout { id: file.id, dungeon, unit, nodes: file.nodes, edges, samples, field })
    }

    /// The nearest corridor sample and how far away it is (world units); outside
    /// the field (more than its margin from every corridor) infinitely far.
    pub fn corridor(&self, at: (f64, f64)) -> (f64, Option<usize>) {
        let Some(j) = self.field.near(at) else { return (f64::INFINITY, None) };
        // The cell's sample is nearest to the cell's centre: its neighbours on the
        // same corridor may be nearer to the point itself.
        let gap = |k: usize| {
            let p = self.samples[k].at;
            (at.0 - p.0).hypot(at.1 - p.1)
        };
        let edge = self.samples[j].edge;
        let (lo, hi) = (j.saturating_sub(3), (j + 3).min(self.samples.len() - 1));
        let best = (lo..=hi).filter(|&k| self.samples[k].edge == edge).min_by(|&a, &b| gap(a).total_cmp(&gap(b))).unwrap_or(j);
        (gap(best) * self.unit, Some(best))
    }

    /// Walks `left` units along the corridors from sample `j` (forward = towards
    /// the edge's node `b`), choosing a branch with `pick(count)` at every node and
    /// turning back at dead ends. The point reached.
    pub fn drift(&self, j: usize, forward: bool, mut left: f64, pick: &mut dyn FnMut(usize) -> usize) -> (f64, f64) {
        let sample = self.samples[j];
        let (mut edge, mut at, mut forward) = (sample.edge as usize, sample.s as f64, forward);
        for _ in 0..64 {
            let e = &self.edges[edge];
            let room = if forward { e.length() - at } else { at };
            if left <= room {
                return e.at(if forward { at + left } else { at - left });
            }
            left -= room;
            let node = if forward { e.b } else { e.a };
            let next: Vec<usize> = (0..self.edges.len()).filter(|&i| i != edge && (self.edges[i].a == node || self.edges[i].b == node)).collect();
            if next.is_empty() {
                // A dead end: back the way it came.
                (at, forward) = (if forward { e.length() } else { 0.0 }, !forward);
                continue;
            }
            edge = next[pick(next.len()) % next.len()];
            forward = self.edges[edge].a == node;
            at = if forward { 0.0 } else { self.edges[edge].length() };
        }
        self.edges[edge].at(at)
    }

    /// The corridor's direction at sample `j` (towards node `b`), a unit vector.
    pub fn tangent(&self, j: usize) -> (f64, f64) {
        let sample = self.samples[j];
        let e = &self.edges[sample.edge as usize];
        let h = 2.0 / self.unit;
        let (p, q) = (e.at(sample.s as f64 - h), e.at(sample.s as f64 + h));
        let l = (q.0 - p.0).hypot(q.1 - p.1).max(1e-9);
        ((q.0 - p.0) / l, (q.1 - p.1) / l)
    }

    /// The nodes a target stands for: those with its exit, else those with its tag.
    pub fn goals(&self, target: &NavTarget) -> Vec<usize> {
        let matches = |n: &Node| match (&target.exit, &target.tag) {
            (Some(exit), _) => n.exit.as_deref() == Some(exit.as_str()),
            (None, Some(tag)) => n.has_tag(tag),
            (None, None) => false,
        };
        (0..self.nodes.len()).filter(|&i| matches(&self.nodes[i])).collect()
    }

    /// Corridor distance (file units) from every node to the nearest of `goals`.
    pub fn distances(&self, goals: &[usize]) -> Vec<f64> {
        let mut dist = vec![f64::INFINITY; self.nodes.len()];
        for &g in goals {
            dist[g] = 0.0;
        }
        // Few nodes: plain Bellman-Ford style relaxation.
        for _ in 0..self.nodes.len() {
            let mut changed = false;
            for e in &self.edges {
                let l = e.length();
                for (u, v) in [(e.a, e.b), (e.b, e.a)] {
                    if dist[v] + l < dist[u] {
                        dist[u] = dist[v] + l;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        dist
    }

    /// Walks `left` units from node `at` downhill towards the goal; the point reached.
    fn walk_from_node(&self, dist: &[f64], mut at: usize, mut left: f64) -> (f64, f64) {
        for _ in 0..=self.nodes.len() {
            if dist[at] <= 0.0 {
                break;
            }
            let next = self
                .edges
                .iter()
                .filter(|e| e.a == at || e.b == at)
                .min_by(|x, y| {
                    let cost = |e: &Edge| e.length() + dist[if e.a == at { e.b } else { e.a }];
                    cost(x).total_cmp(&cost(y))
                });
            let Some(e) = next else { break };
            let forward = e.a == at;
            if left <= e.length() {
                return e.at(if forward { left } else { e.length() - left });
            }
            left -= e.length();
            at = if forward { e.b } else { e.a };
        }
        (self.nodes[at].x, self.nodes[at].y)
    }

    /// Which way the corridors lead to `goals` from every corridor sample.
    pub fn guidance(&self, goals: &[usize]) -> Guidance {
        let dist = self.distances(goals);
        let look = LOOK / self.unit;
        let mut ahead = Vec::with_capacity(self.samples.len());
        let mut togo = Vec::with_capacity(self.samples.len());
        for sample in &self.samples {
            let e = &self.edges[sample.edge as usize];
            let s = sample.s as f64;
            let (via_a, via_b) = (s + dist[e.a], e.length() - s + dist[e.b]);
            let mut total = via_a.min(via_b);
            let point = if !total.is_finite() {
                // Not joined by corridors (a few graphs leave an exit apart): straight to the nearest goal.
                let nearest = goals.iter().map(|&g| (self.nodes[g].x, self.nodes[g].y)).min_by(|p, q| {
                    let d = |n: &(f64, f64)| (n.0 - sample.at.0).hypot(n.1 - sample.at.1);
                    d(p).total_cmp(&d(q))
                });
                match nearest {
                    Some(goal) => {
                        total = (goal.0 - sample.at.0).hypot(goal.1 - sample.at.1);
                        goal
                    }
                    None => sample.at,
                }
            } else if via_a <= via_b {
                if s >= look { e.at(s - look) } else { self.walk_from_node(&dist, e.a, look - s) }
            } else if e.length() - s >= look {
                e.at(s + look)
            } else {
                self.walk_from_node(&dist, e.b, look - (e.length() - s))
            };
            ahead.push((point.0 as f32, point.1 as f32));
            togo.push((total * self.unit) as f32);
        }
        Guidance { ahead, togo }
    }
}

pub struct AreaData {
    pub(super) id: String,
    /// Exit area id → its name.
    pub(super) names: HashMap<String, String>,
    pub(super) layouts: Vec<Layout>,
}

/// One area's layout graphs; cheap to clone.
#[derive(Clone)]
pub struct Area(Arc<AreaData>);

impl std::ops::Deref for Area {
    type Target = AreaData;
    fn deref(&self) -> &AreaData {
        &self.0
    }
}

impl Area {
    /// The graphs of one bundled file; None when it has no usable layout.
    pub(super) fn parse(id: &str, json: &str) -> Option<Area> {
        let file: AreaFile = match serde_json::from_str(json) {
            Ok(file) => file,
            Err(error) => {
                eprintln!("Layout graphs for {id} not read: {error}");
                return None;
            }
        };
        let names = file.connections.iter().cloned().zip(file.connection_names.iter().cloned()).collect();
        let layouts: Vec<Layout> = file.topologies.into_iter().filter_map(Layout::new).collect();
        (!layouts.is_empty()).then(|| Area(Arc::new(AreaData { id: id.to_string(), names, layouts })))
    }
}

/// Areas whose graphs stay built (the current one and the last few).
const KEEP_AREAS: usize = 3;

/// The bundled graphs, built on first use per area (distance fields take a
/// moment) and dropped again once `KEEP_AREAS` newer areas were used.
#[derive(Default)]
pub struct Graphs(Mutex<Vec<(String, Option<Area>)>>);

impl Graphs {
    pub fn get(&self, id: &str) -> Option<Area> {
        let mut built = self.0.lock().ok()?;
        if let Some(i) = built.iter().position(|(a, _)| a == id) {
            let entry = built.remove(i);
            let area = entry.1.clone();
            built.push(entry);
            return area;
        }
        let area = crate::bundle::LAYOUTS.iter().find(|(stem, _)| *stem == id).and_then(|(_, json)| Area::parse(id, json));
        built.push((id.to_string(), area.clone()));
        if built.len() > KEEP_AREAS {
            built.remove(0);
        }
        area
    }
}

/// Without checklist targets: the onward exit, the lowest-numbered zone not just
/// come from or ticked; a town only when there is nothing else.
pub(super) fn target(area: &Area, arrived_from: &str, completed: &[String]) -> Option<NavTarget> {
    let done = |id: &str| {
        let name = area.names.get(id).map(String::as_str).unwrap_or(id);
        completed.iter().any(|c| c.eq_ignore_ascii_case(name))
    };
    // Onward first; else the zone just left (e.g. back from town), else any exit.
    let mut exits: Vec<&String> = area.names.keys().filter(|id| *id != arrived_from && !done(id)).collect();
    if exits.is_empty() {
        exits = area.names.keys().filter(|id| !done(id)).collect();
    }
    if exits.is_empty() {
        exits = area.names.keys().collect();
    }
    exits.sort_by_key(|id| (id.ends_with("_town"), area_order(id)));
    let exit = (*exits.first()?).clone();
    let boss = BOSS_AT_EXIT.iter().find(|(a, e, b)| *a == area.id && *e == exit && !completed.iter().any(|c| c == b));
    let label = match boss {
        Some((_, _, b)) => b.to_string(),
        None => area.names[&exit].clone(),
    };
    Some(NavTarget { label, exit: Some(exit), tag: None, optional: false })
}

#[cfg(test)]
pub(super) fn load(id: &str) -> Area {
    Graphs::default().get(id).expect("bundled layout graphs")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exit(id: &str) -> NavTarget {
        NavTarget { label: id.into(), exit: Some(id.into()), ..Default::default() }
    }

    #[test]
    fn graphs_load_with_named_exits() {
        assert_eq!(load("G1_1").layouts.len(), 5);
        let clearfell = load("G1_2");
        assert_eq!(clearfell.layouts.len(), 33);
        assert_eq!(clearfell.names["G1_3"], "Mud Burrow");
        for layout in &clearfell.layouts {
            for id in ["G1_town", "G1_3", "G1_4"] {
                assert!(!layout.goals(&exit(id)).is_empty(), "{} {id}", layout.id);
            }
        }
        // Tags stay on the nodes.
        assert!(clearfell.layouts.iter().any(|l| l.nodes.iter().any(|n| n.has_tag("boss"))));
        // The encampment's graph is a single node: no usable layout.
        assert!(Graphs::default().get("G1_town").is_none());
        assert!(Graphs::default().get("G9_none").is_none());
    }

    #[test]
    fn new_optional_fields_are_accepted() {
        let json = r#"{"areaId": "X", "name": "X", "act": 1, "level": 3, "connections": ["Y"], "connectionNames": ["Why"],
            "topologies": [
              {"id": "one", "file": "a.tgr", "format": "tgr", "size": [4, 4],
               "nodes": [{"x": 0, "y": 0, "tag": "start", "keys": ["k"], "start": true}, {"x": 50, "y": 0, "exit": "Y"}],
               "edges": [{"a": 0, "b": 1, "via": [], "kind": "road"}]},
              {"id": "lonely", "size": [1, 1], "nodes": [{"x": 0, "y": 0}], "edges": []},
              {"id": "room", "format": "dgr", "size": [3, 3], "roomUnitSize": 9,
               "nodes": [{"x": 0, "y": 0}, {"x": 2, "y": 0}], "edges": [{"a": 0, "b": 1}]}]}"#;
        let area = Area::parse("X", json).unwrap();
        let ids: Vec<&str> = area.layouts.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["one", "room"], "a layout with one node is skipped");
        assert!(area.layouts[0].nodes[0].has_tag("k"));
        assert!(!area.layouts[0].dungeon && area.layouts[1].dungeon);
        assert_eq!((area.layouts[0].unit, area.layouts[1].unit), (1.0, 8.0));
        // A broken file is skipped, not a crash.
        assert!(Area::parse("X", "{").is_none());
    }

    #[test]
    fn every_bundled_file_loads() {
        let graphs = Graphs::default();
        let mut usable = 0;
        for (id, _) in crate::bundle::LAYOUTS {
            if let Some(area) = graphs.get(id) {
                usable += 1;
                assert!(area.layouts.iter().all(|l| l.nodes.len() >= 2 && !l.samples.is_empty()), "{id}");
            }
        }
        assert!(usable >= 3, "{usable} areas with usable graphs");
    }

    #[test]
    fn targets_follow_the_campaign() {
        let label = |area: &str, from: &str, done: &[String]| target(&load(area), from, done).unwrap().label;
        assert_eq!(label("G1_1", "", &[]), "The Bloated Miller");
        assert_eq!(label("G1_1", "", &["The Bloated Miller".into()]), "Clearfell Encampment");
        assert_eq!(label("G1_2", "G1_town", &[]), "Mud Burrow");
        assert_eq!(label("G1_2", "G1_3", &[]), "The Grelwood");
        assert_eq!(target(&load("G1_2"), "G1_3", &[]).unwrap().exit.as_deref(), Some("G1_4"));
        // Back in The Riverbank from the encampment: still a target.
        assert!(target(&load("G1_1"), "G1_town", &[]).is_some());
    }

    #[test]
    fn the_corridor_distance_is_euclidean() {
        let area = load("G1_2");
        let layout = &area.layouts[0];
        let e = &layout.edges[0];
        let mid = e.at(e.length() / 2.0);
        assert!(layout.corridor(mid).0 < SPACING, "{:?}", layout.corridor(mid));
        // Far outside the layout: infinitely far.
        assert_eq!(layout.corridor((-1e5, -1e5)), (f64::INFINITY, None));
        // A brute-force check at random points near the corridors.
        let mut seed = 7u64;
        for _ in 0..200 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let j = (seed >> 33) as usize % layout.samples.len();
            let off = (((seed >> 20) & 0xff) as f64 - 128.0) * 0.6;
            let p = (layout.samples[j].at.0 + off, layout.samples[j].at.1 - off * 0.7);
            let exact = layout.samples.iter().map(|s| (s.at.0 - p.0).hypot(s.at.1 - p.1)).fold(f64::MAX, f64::min);
            let (got, _) = layout.corridor(p);
            assert!(got - exact < CELL / 2.0, "{got} vs {exact}");
        }
    }

    #[test]
    fn the_route_bends_with_the_corridors() {
        // An L-shaped corridor: east, then north. Standing at the start, the route
        // first goes east, not straight to the goal.
        let json = r#"{"topologies": [{"id": "L", "size": [20, 20],
            "nodes": [{"x": 0, "y": 0, "start": true}, {"x": 400, "y": 0}, {"x": 400, "y": 400, "exit": "X"}],
            "edges": [{"a": 0, "b": 1, "via": []}, {"a": 1, "b": 2, "via": []}]}]}"#;
        let area = Area::parse("L", json).unwrap();
        let layout = &area.layouts[0];
        let guidance = layout.guidance(&layout.goals(&exit("X")));
        let (_, near) = layout.corridor((0.0, 0.0));
        let j = near.unwrap();
        assert!((guidance.togo[j] - 800.0).abs() < 1e-3, "{}", guidance.togo[j]);
        let (x, y) = guidance.ahead[j];
        assert!((x as f64 - LOOK).abs() < 1e-3 && y.abs() < 1e-3, "{x} {y}");
        // Round the corner: 60 units before it, the point ahead is 60 units up the north arm.
        let (_, near) = layout.corridor((340.0, 0.0));
        let (x, y) = guidance.ahead[near.unwrap()];
        assert!((x - 400.0).abs() < 1e-3 && (y - 60.0).abs() < 1.0, "{x} {y}");
        // Near the goal the point ahead is the goal.
        let (_, near) = layout.corridor((400.0, 380.0));
        assert_eq!(guidance.ahead[near.unwrap()], (400.0, 400.0));
    }
}
