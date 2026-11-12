//! The arrow from the game's own layout graphs (`data/layouts/ggg/`, datamined).
//!
//! Each zone has a few possible layouts. A layout is a graph: fixed points (zone
//! exits, boss arenas, …) joined by corridors, in world units (x east, y north).
//! Walls are generated around the corridors per instance, so the player's walked
//! path follows the corridors of the layout being played. The path recorded since
//! arrival is laid on every layout (starting at the exit the player came through,
//! give or take a little, at about the measured minimap scale); the layout whose
//! corridors it follows best, clearly ahead of any layout pointing elsewhere, gives
//! the arrow. The arrow points a short way along the corridors to the target exit.

use super::guide::{area_order, from_screen, screen_bearing, to_screen, Evidence, GuideView};
use serde::Deserialize;
use std::collections::HashMap;

const AREAS: [(&str, &str); 2] = [
    ("G1_1", include_str!("../../../data/layouts/ggg/G1_1.json")),
    ("G1_2", include_str!("../../../data/layouts/ggg/G1_2.json")),
];

/// World units per layout tile.
const TILE: f64 = 23.0;
/// Grid cell of the distance-to-corridor field, in world units.
const CELL: f64 = 8.0;
/// Room around the layout's tiles in the field.
const MARGIN: f64 = 150.0;
/// Path points farther than this from any corridor all count the same.
const FAR: f64 = 150.0;
/// Minimap cells per world unit, measured on a Clearfell Encampment visit.
pub const SCALE: f64 = 0.78;
/// Scale factors tried around the measured scale.
const SCALE_FACTORS: [f64; 3] = [0.92, 1.0, 1.08];
/// The arrival point lies near, not on, the arrival exit's node: offsets tried (world units).
const OFFSET: i32 = 80;
const OFFSET_STEP: usize = 20;
/// At most this many path points are scored.
const PATH_POINTS: usize = 300;
/// The player must have walked this far (world units) from arrival before a layout is chosen.
const MIN_REACH: f64 = 150.0;
/// Mean distance (world units) of the walked path from the best layout's corridors.
const MAX_MEAN: f64 = 60.0;
/// Up to this mean, an unconfirmed best layout still gives a rough needle.
const ROUGH_MEAN: f64 = 80.0;
/// The best layout must beat the best layout pointing elsewhere by this share.
const MIN_CONFIDENCE: f32 = 0.2;
/// Two directions closer than this count as agreeing.
const SAME_WAY: f64 = 45.0;
/// How far along the corridors the arrow looks (world units).
const LOOK: f64 = 120.0;
/// Bosses that wait at a zone exit: (area, exit area, boss) until the boss is ticked.
const BOSS_AT_EXIT: [(&str, &str, &str); 1] = [("G1_1", "G1_town", "The Bloated Miller")];

#[derive(Deserialize)]
struct AreaFile {
    connections: Vec<String>,
    #[serde(rename = "connectionNames", default)]
    connection_names: Vec<String>,
    topologies: Vec<LayoutFile>,
}

#[derive(Deserialize)]
struct LayoutFile {
    id: String,
    size: [f64; 2],
    nodes: Vec<Node>,
    edges: Vec<EdgeFile>,
}

#[derive(Deserialize, Debug, Clone)]
struct Node {
    x: f64,
    y: f64,
    exit: Option<String>,
    #[serde(default)]
    start: bool,
}

#[derive(Deserialize)]
struct EdgeFile {
    a: usize,
    b: usize,
    via: Vec<[f64; 2]>,
}

pub struct Area {
    id: String,
    names: HashMap<String, String>,
    layouts: Vec<Layout>,
}

struct Layout {
    id: String,
    nodes: Vec<Node>,
    /// Corridors: (node a, node b, points from a to b).
    edges: Vec<(usize, usize, Vec<(f64, f64)>)>,
    field: Field,
}

/// Distance from each grid cell to the nearest corridor, in world units.
struct Field {
    origin: (f64, f64),
    cols: usize,
    rows: usize,
    dist: Vec<f32>,
}

impl Field {
    fn new(size: [f64; 2], edges: &[(usize, usize, Vec<(f64, f64)>)]) -> Field {
        let origin = (-MARGIN, -MARGIN);
        let cols = ((size[0] * TILE + 2.0 * MARGIN) / CELL).ceil() as usize;
        let rows = ((size[1] * TILE + 2.0 * MARGIN) / CELL).ceil() as usize;
        let mut dist = vec![f32::MAX; cols * rows];
        for (_, _, points) in edges {
            for pair in points.windows(2) {
                let ((ax, ay), (bx, by)) = (pair[0], pair[1]);
                let steps = ((bx - ax).hypot(by - ay) / (CELL / 2.0)).ceil().max(1.0) as usize;
                for s in 0..=steps {
                    let t = s as f64 / steps as f64;
                    let col = ((ax + t * (bx - ax) - origin.0) / CELL).floor();
                    let row = ((ay + t * (by - ay) - origin.1) / CELL).floor();
                    if col >= 0.0 && row >= 0.0 && (col as usize) < cols && (row as usize) < rows {
                        dist[row as usize * cols + col as usize] = 0.0;
                    }
                }
            }
        }
        // Two-pass chamfer distance.
        let (straight, diagonal) = (CELL as f32, (CELL * std::f64::consts::SQRT_2) as f32);
        let pass = |dist: &mut Vec<f32>, order: &mut dyn Iterator<Item = (usize, usize)>, back: bool| {
            let sign: i64 = if back { 1 } else { -1 };
            for (row, col) in order {
                let i = row * cols + col;
                let mut best = dist[i];
                for (dr, dc, cost) in [(0, sign, straight), (sign, 0, straight), (sign, sign, diagonal), (sign, -sign, diagonal)] {
                    let (r, c) = (row as i64 + dr, col as i64 + dc);
                    if r >= 0 && c >= 0 && (r as usize) < rows && (c as usize) < cols {
                        best = best.min(dist[r as usize * cols + c as usize] + cost);
                    }
                }
                dist[i] = best;
            }
        };
        pass(&mut dist, &mut (0..rows).flat_map(|r| (0..cols).map(move |c| (r, c))), false);
        pass(&mut dist, &mut (0..rows).rev().flat_map(|r| (0..cols).rev().map(move |c| (r, c))), true);
        Field { origin, cols, rows, dist }
    }

    fn at(&self, (x, y): (f64, f64)) -> f64 {
        let (col, row) = ((x - self.origin.0) / CELL, (y - self.origin.1) / CELL);
        if col < 0.0 || row < 0.0 || col as usize >= self.cols || row as usize >= self.rows {
            return FAR;
        }
        (self.dist[row as usize * self.cols + col as usize] as f64).min(FAR)
    }
}

pub fn load() -> HashMap<String, Area> {
    AREAS
        .iter()
        .map(|&(id, json)| {
            let file: AreaFile = serde_json::from_str(json).expect("bundled layout graphs are valid");
            let names = file.connections.iter().cloned().zip(file.connection_names.iter().cloned()).collect();
            let layouts = file
                .topologies
                .into_iter()
                .map(|t| {
                    let edges: Vec<(usize, usize, Vec<(f64, f64)>)> = t
                        .edges
                        .iter()
                        .map(|e| {
                            let mut points = vec![(t.nodes[e.a].x, t.nodes[e.a].y)];
                            points.extend(e.via.iter().map(|p| (p[0], p[1])));
                            points.push((t.nodes[e.b].x, t.nodes[e.b].y));
                            (e.a, e.b, points)
                        })
                        .collect();
                    let field = Field::new(t.size, &edges);
                    Layout { id: t.id, nodes: t.nodes, edges, field }
                })
                .collect();
            (id.to_string(), Area { id: id.to_string(), names, layouts })
        })
        .collect()
}

fn length(points: &[(f64, f64)]) -> f64 {
    points.windows(2).map(|p| (p[1].0 - p[0].0).hypot(p[1].1 - p[0].1)).sum()
}

impl Layout {
    /// Corridor distance from every node to the nearest of `goals`.
    fn distances(&self, goals: &[usize]) -> Vec<f64> {
        let mut dist = vec![f64::MAX; self.nodes.len()];
        for &g in goals {
            dist[g] = 0.0;
        }
        // Few nodes: plain Bellman-Ford style relaxation.
        for _ in 0..self.nodes.len() {
            let mut changed = false;
            for (a, b, points) in &self.edges {
                let l = length(points);
                for (u, v) in [(*a, *b), (*b, *a)] {
                    if dist[v] < f64::MAX && dist[v] + l < dist[u] {
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

    /// The walking route from `from` to the nearest goal along the corridors, and its length.
    fn route(&self, from: (f64, f64), goals: &[usize]) -> Option<(Vec<(f64, f64)>, f64)> {
        let dist = self.distances(goals);
        // Nearest corridor point.
        let mut best: Option<(f64, usize, usize, (f64, f64))> = None;
        for (e, (_, _, points)) in self.edges.iter().enumerate() {
            for (j, pair) in points.windows(2).enumerate() {
                let ((ax, ay), (bx, by)) = (pair[0], pair[1]);
                let (dx, dy) = (bx - ax, by - ay);
                let l2 = dx * dx + dy * dy;
                let t = if l2 > 0.0 { (((from.0 - ax) * dx + (from.1 - ay) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
                let q = (ax + t * dx, ay + t * dy);
                let d = (from.0 - q.0).hypot(from.1 - q.1);
                if best.is_none_or(|b| d < b.0) {
                    best = Some((d, e, j, q));
                }
            }
        }
        let (_, e, j, q) = best?;
        let (a, b, points) = &self.edges[e];
        let mut towards_a: Vec<(f64, f64)> = vec![from, q];
        towards_a.extend(points[..=j].iter().rev());
        let mut towards_b: Vec<(f64, f64)> = vec![from, q];
        towards_b.extend(points[j + 1..].iter());
        let cost_a = length(&towards_a) + dist[*a];
        let cost_b = length(&towards_b) + dist[*b];
        let (mut path, mut at, total) =
            if cost_a <= cost_b { (towards_a, *a, cost_a) } else { (towards_b, *b, cost_b) };
        if total >= f64::MAX / 2.0 {
            return None;
        }
        // Follow the corridors downhill to the goal.
        while dist[at] > 0.0 {
            let next = self
                .edges
                .iter()
                .filter_map(|(a, b, points)| {
                    if *a == at {
                        Some((*b, points.clone()))
                    } else if *b == at {
                        Some((*a, points.iter().rev().copied().collect()))
                    } else {
                        None
                    }
                })
                .min_by(|x, y| (length(&x.1) + dist[x.0]).total_cmp(&(length(&y.1) + dist[y.0])))?;
            path.extend(next.1.into_iter().skip(1));
            at = next.0;
        }
        Some((path, total))
    }
}

/// The point `look` along a path.
fn ahead(path: &[(f64, f64)], look: f64) -> (f64, f64) {
    let mut left = look;
    for pair in path.windows(2) {
        let l = (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1);
        if l >= left && l > 0.0 {
            let t = left / l;
            return (pair[0].0 + t * (pair[1].0 - pair[0].0), pair[0].1 + t * (pair[1].1 - pair[0].1));
        }
        left -= l;
    }
    *path.last().unwrap_or(&(0.0, 0.0))
}

/// The exit to head for: onward to the lowest-numbered zone not just come from or
/// ticked; a town only when there is nothing else. Returns (exit area, label, kind).
fn target(area: &Area, ev: &Evidence) -> Option<(String, String, String)> {
    let done = |id: &str| {
        let name = area.names.get(id).map(String::as_str).unwrap_or(id);
        ev.completed.iter().any(|c| c.eq_ignore_ascii_case(name))
    };
    let mut exits: Vec<&String> = area.names.keys().filter(|id| *id != ev.arrived_from && !done(id)).collect();
    exits.sort_by_key(|id| (id.ends_with("_town"), area_order(id)));
    let exit = (*exits.first()?).clone();
    let boss = BOSS_AT_EXIT.iter().find(|(a, e, b)| *a == area.id && *e == exit && !ev.completed.iter().any(|c| c == b));
    Some(match boss {
        Some((_, _, b)) => (exit, b.to_string(), "boss".into()),
        None => (exit.clone(), area.names[&exit].clone(), "entrance".into()),
    })
}

struct Placement {
    layout: usize,
    /// World position of the arrival point.
    origin: (f64, f64),
    scale: f64,
    mean: f64,
}

/// Each layout's best placement of the walked path, best layout first.
fn placements(area: &Area, ev: &Evidence, path: &[(f64, f64)], base: f64) -> Vec<Placement> {
    let mut out = Vec::new();
    for (index, layout) in area.layouts.iter().enumerate() {
        let mut anchors: Vec<(f64, f64)> = layout
            .nodes
            .iter()
            .filter(|n| n.exit.as_deref() == Some(ev.arrived_from))
            .map(|n| (n.x, n.y))
            .collect();
        if anchors.is_empty() {
            anchors = layout.nodes.iter().filter(|n| n.start).map(|n| (n.x, n.y)).collect();
        }
        let mut best: Option<Placement> = None;
        for factor in SCALE_FACTORS {
            let scale = base * factor;
            let rel: Vec<(f64, f64)> = path.iter().map(|&(x, y)| from_screen((x / scale, y / scale))).collect();
            for &(ax, ay) in &anchors {
                for dx in (-OFFSET..=OFFSET).step_by(OFFSET_STEP) {
                    for dy in (-OFFSET..=OFFSET).step_by(OFFSET_STEP) {
                        let origin = (ax + dx as f64, ay + dy as f64);
                        let mean = rel.iter().map(|&(x, y)| layout.field.at((origin.0 + x, origin.1 + y))).sum::<f64>()
                            / rel.len().max(1) as f64;
                        if best.as_ref().is_none_or(|b| mean < b.mean) {
                            best = Some(Placement { layout: index, origin, scale, mean });
                        }
                    }
                }
            }
        }
        out.extend(best);
    }
    out.sort_by(|a, b| a.mean.total_cmp(&b.mean));
    out
}

pub fn guide(area: &Area, ev: &Evidence) -> Option<GuideView> {
    let (exit, label, kind) = target(area, ev)?;
    let mut view = GuideView {
        variant: String::new(),
        target_kind: kind,
        target: label,
        degrees: None,
        source: "unconfirmed".into(),
        distance: 0.0,
        confidence: 0.0,
        measured: ev.scale.is_some(),
        turned: false,
    };
    let base = ev.scale.unwrap_or(SCALE);
    let step = ev.trail.len().div_ceil(PATH_POINTS).max(1);
    let mut path: Vec<(f64, f64)> = ev.trail.iter().step_by(step).map(|&(x, y)| (x as f64, y as f64)).collect();
    path.push((ev.player.0 as f64, ev.player.1 as f64));
    let reach = path.iter().map(|&(x, y)| x.hypot(y) / base).fold(0.0, f64::max);
    let all = placements(area, ev, &path, base);
    // Bearing (screen degrees) and walking distance (minimap cells) under one placement.
    let heading = |p: &Placement| -> Option<(f32, f64)> {
        let layout = &area.layouts[p.layout];
        let goals: Vec<usize> =
            (0..layout.nodes.len()).filter(|&i| layout.nodes[i].exit.as_deref() == Some(exit.as_str())).collect();
        let player = from_screen((ev.player.0 as f64 / p.scale, ev.player.1 as f64 / p.scale));
        let at = (p.origin.0 + player.0, p.origin.1 + player.1);
        let (route, total) = layout.route(at, &goals)?;
        let (x, y) = ahead(&route, LOOK);
        let (sx, sy) = to_screen((x - at.0, y - at.1));
        Some((screen_bearing(sx, sy)?, total * p.scale))
    };
    if reach < MIN_REACH {
        // Too little walked to tell layouts apart: a rough needle only if they all agree.
        let headings: Vec<(f32, f64)> = all.iter().filter_map(&heading).collect();
        if let Some(&(_, distance)) = headings.first() {
            let (sx, sy) = headings.iter().fold((0.0, 0.0), |(sx, sy), (d, _)| {
                let r = (*d as f64).to_radians();
                (sx + r.cos(), sy + r.sin())
            });
            let mean = sy.atan2(sx).to_degrees() as f32;
            if headings.len() == all.len() && headings.iter().all(|(d, _)| super::guide::angle_between(*d, mean) <= SAME_WAY) {
                view.degrees = Some(mean);
                view.distance = distance as f32;
                view.source = "rough".into();
            }
        }
        return Some(view);
    }
    let Some(best) = all.first() else { return Some(view) };
    view.variant = area.layouts[best.layout].id.clone();
    let Some((degrees, distance)) = heading(best) else { return Some(view) };
    view.distance = distance as f32;
    let rival = all
        .iter()
        .skip(1)
        .find(|p| heading(p).is_none_or(|(d, _)| super::guide::angle_between(d, degrees) > SAME_WAY))
        .map_or(f64::MAX, |p| p.mean);
    view.confidence = (1.0 - best.mean / rival.max(1e-9)) as f32;
    if best.mean <= MAX_MEAN && view.confidence >= MIN_CONFIDENCE {
        view.degrees = Some(degrees);
        view.source = "estimated".into();
    } else if best.mean <= ROUGH_MEAN {
        // Fits fairly well but without a clear lead: a fainter needle along it.
        view.degrees = Some(degrees);
        view.source = "rough".into();
    }
    Some(view)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev<'a>(area: &'a str, from: &'a str, trail: &'a [(i32, i32)], player: (i32, i32)) -> Evidence<'a> {
        Evidence { area, arrived_from: from, walls: &[], trail, player, completed: &[], scale: None }
    }

    /// The minimap path of a walk along `layout`'s corridors from `start` through `via` nodes.
    fn walk(layout: &Layout, start: (f64, f64), via: &[usize]) -> Vec<(i32, i32)> {
        let mut points = vec![start];
        for &n in via {
            points.push((layout.nodes[n].x, layout.nodes[n].y));
        }
        let mut out = Vec::new();
        for pair in points.windows(2) {
            let steps = ((pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1) / 10.0).ceil() as usize;
            for s in 0..=steps {
                let t = s as f64 / steps.max(1) as f64;
                let w = (pair[0].0 + t * (pair[1].0 - pair[0].0) - start.0, pair[0].1 + t * (pair[1].1 - pair[0].1) - start.1);
                let (x, y) = to_screen(w);
                out.push(((x * SCALE).round() as i32, (y * SCALE).round() as i32));
            }
        }
        out
    }

    #[test]
    fn graphs_load_with_named_exits() {
        let areas = load();
        assert_eq!(areas["G1_1"].layouts.len(), 5);
        assert_eq!(areas["G1_2"].layouts.len(), 33);
        assert_eq!(areas["G1_2"].names["G1_3"], "Mud Burrow");
        for layout in &areas["G1_2"].layouts {
            let exits: Vec<_> = layout.nodes.iter().filter_map(|n| n.exit.as_deref()).collect();
            assert!(exits.contains(&"G1_town") && exits.contains(&"G1_3") && exits.contains(&"G1_4"), "{}", layout.id);
        }
    }

    #[test]
    fn targets_follow_the_campaign() {
        let areas = load();
        let (exit, label, kind) = target(&areas["G1_1"], &ev("G1_1", "", &[], (0, 0))).unwrap();
        assert_eq!((exit.as_str(), label.as_str(), kind.as_str()), ("G1_town", "The Bloated Miller", "boss"));
        let done = ["The Bloated Miller".to_string()];
        let e = Evidence { completed: &done, ..ev("G1_1", "", &[], (0, 0)) };
        assert_eq!(target(&areas["G1_1"], &e).unwrap().1, "Clearfell Encampment");
        assert_eq!(target(&areas["G1_2"], &ev("G1_2", "G1_town", &[], (0, 0))).unwrap().1, "Mud Burrow");
        assert_eq!(target(&areas["G1_2"], &ev("G1_2", "G1_3", &[], (0, 0))).unwrap().1, "The Grelwood");
    }

    #[test]
    fn a_walk_along_one_layout_picks_it_and_points_along_its_corridors() {
        let areas = load();
        let area = &areas["G1_2"];
        let index = area.layouts.iter().position(|l| l.id == "ClearFell_W_S_N").unwrap();
        let layout = &area.layouts[index];
        let town = layout.nodes.iter().position(|n| n.exit.as_deref() == Some("G1_town")).unwrap();
        let start = (layout.nodes[town].x, layout.nodes[town].y);
        // Walk two corridors away from the town exit.
        let next: Vec<usize> = layout.edges.iter().filter_map(|(a, b, _)| {
            if *a == town { Some(*b) } else if *b == town { Some(*a) } else { None }
        }).collect();
        let far = layout.edges.iter().find_map(|(a, b, _)| {
            if *a == next[0] && *b != town { Some(*b) } else if *b == next[0] && *a != town { Some(*a) } else { None }
        }).unwrap();
        let trail = walk(layout, start, &[next[0], far]);
        let player = *trail.last().unwrap();
        let view = guide(area, &ev("G1_2", "G1_town", &trail, player)).unwrap();
        assert!(view.variant.contains("W_S_N"), "{view:?}");
        assert_eq!(view.target, "Mud Burrow");
    }

    #[test]
    fn no_walk_yet_names_the_target_without_a_direction() {
        let areas = load();
        // Clearfell's layouts put Mud Burrow on every side: no needle yet.
        let view = guide(&areas["G1_2"], &ev("G1_2", "G1_town", &[(0, 0), (5, 3)], (5, 3))).unwrap();
        assert_eq!(view.target, "Mud Burrow");
        assert_eq!(view.degrees, None);
    }

    #[test]
    fn the_route_bends_with_the_corridors() {
        // An L-shaped corridor: east, then north. Standing at the start, the route
        // first goes east (screen up-right), not straight to the goal.
        let nodes = vec![
            Node { x: 0.0, y: 0.0, exit: None, start: true },
            Node { x: 400.0, y: 0.0, exit: None, start: false },
            Node { x: 400.0, y: 400.0, exit: Some("X".into()), start: false },
        ];
        let edges = vec![(0, 1, vec![(0.0, 0.0), (400.0, 0.0)]), (1, 2, vec![(400.0, 0.0), (400.0, 400.0)])];
        let field = Field::new([20.0, 20.0], &edges);
        let layout = Layout { id: "L".into(), nodes, edges, field };
        let (route, total) = layout.route((0.0, 0.0), &[2]).unwrap();
        assert!((total - 800.0).abs() < 1e-6);
        let (x, y) = ahead(&route, LOOK);
        assert!((x - LOOK).abs() < 1e-6 && y.abs() < 1e-6);
    }
}
