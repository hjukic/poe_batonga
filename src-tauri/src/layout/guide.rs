//! The arrow: where the next objective probably is, from Qutm's layout diagrams
//! (`data/layouts/qutm/layouts.json`) fitted to what we have recorded of this visit.
//!
//! Each diagram variant, in each of the 8 rotations/mirrors, is anchored at the
//! player's arrival point. A hypothesis is scored by how well the explored walls fit
//! inside the zone the diagram describes and, once a boss has spoken, by whether the
//! boss is where the diagram says. The scale (minimap cells per diagram width) comes
//! from that boss, from earlier visits, or from a default.

use super::Poi;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const LAYOUTS: &str = include_str!("../../../data/layouts/qutm/layouts.json");
const ZONES: &str = include_str!("../../../data/observations/campaign-zones.json");

/// Minimap cells per diagram width when nothing better is known. A rough guess,
/// replaced per zone once a visit has been measured.
pub const DEFAULT_SCALE: f64 = 1200.0;
/// How far outside the diagram's box walls may lie (the diagrams are approximate).
const MARGIN: f64 = 0.12;
/// Rotated or mirrored versions of a variant are less likely than the drawn one.
const TURNED_PRIOR: f64 = 0.35;
/// A boss seen this far (in degrees) from where the diagram puts it rules a hypothesis out.
const BOSS_ANGLE: f64 = 40.0;
/// Two directions closer than this count as agreeing.
const SAME_WAY: f64 = 45.0;
/// Within this share of the diagram width an objective counts as reached.
const REACH: f64 = 0.06;

#[derive(Deserialize, Debug, Clone)]
struct Icon {
    id: String,
    #[serde(default)]
    label: Option<String>,
    x: f64,
    y: f64,
}

#[derive(Deserialize, Debug, Clone)]
struct Line {
    #[serde(rename = "fromIconId")]
    from: usize,
    #[serde(rename = "toIconId")]
    to: usize,
}

#[derive(Deserialize, Debug, Clone)]
struct Variant {
    name: String,
    icons: Vec<Icon>,
    #[serde(default)]
    lines: Vec<Line>,
}

#[derive(Deserialize)]
struct ZoneFile {
    areas: Vec<Zone>,
}

#[derive(Deserialize)]
struct Zone {
    id: String,
    #[serde(default)]
    names: Vec<String>,
}

pub struct Catalog {
    layouts: HashMap<String, Vec<Variant>>,
    names: HashMap<String, Vec<String>>,
}

impl Catalog {
    pub fn load() -> Catalog {
        let layouts = serde_json::from_str(LAYOUTS).expect("bundled layouts are valid");
        let zones: ZoneFile = serde_json::from_str(ZONES).expect("bundled zone list is valid");
        Catalog { layouts, names: zones.areas.into_iter().map(|z| (z.id, z.names)).collect() }
    }
}

/// "The Bloated Miller" vs "Miller", "Clearfell Encampent" (sic) vs "Clearfell Encampment".
fn normalise(text: &str) -> String {
    let lower = text.to_lowercase();
    let lower = lower.strip_prefix("the ").unwrap_or(&lower);
    lower.chars().filter(|c| c.is_alphanumeric()).collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for j in 0..b.len() {
            let next = (row[j + 1] + 1).min(row[j] + 1).min(prev + usize::from(ca != b[j]));
            prev = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

fn same_name(a: &str, b: &str) -> bool {
    let (a, b) = (normalise(a), normalise(b));
    !a.is_empty() && !b.is_empty() && (a.contains(&b) || b.contains(&a) || edit_distance(&a, &b) <= 2)
}

/// Rotations and mirrors as 2x2 matrices [a, b, c, d]: (x, y) -> (ax + by, cx + dy).
const TURNS: [[f64; 4]; 8] = [
    [1.0, 0.0, 0.0, 1.0],
    [0.0, -1.0, 1.0, 0.0],
    [-1.0, 0.0, 0.0, -1.0],
    [0.0, 1.0, -1.0, 0.0],
    [-1.0, 0.0, 0.0, 1.0],
    [1.0, 0.0, 0.0, -1.0],
    [0.0, 1.0, 1.0, 0.0],
    [0.0, -1.0, -1.0, 0.0],
];

fn turn(t: &[f64; 4], (x, y): (f64, f64)) -> (f64, f64) {
    (t[0] * x + t[1] * y, t[2] * x + t[3] * y)
}

/// The inverse of a rotation/mirror is its transpose.
fn unturn(t: &[f64; 4], (x, y): (f64, f64)) -> (f64, f64) {
    (t[0] * x + t[2] * y, t[1] * x + t[3] * y)
}

fn angle_between(a: (f64, f64), b: (f64, f64)) -> f64 {
    let d = (a.1.atan2(a.0) - b.1.atan2(b.0)).to_degrees().rem_euclid(360.0);
    d.min(360.0 - d)
}

/// What we know about the current visit, in minimap cells relative to the arrival point.
pub struct Evidence<'a> {
    pub area: &'a str,
    /// Area id the player came from; the arrival point is that zone's entrance.
    pub arrived_from: &'a str,
    pub walls: &'a [(i32, i32)],
    pub trail: &'a [[i32; 2]],
    pub player: (i32, i32),
    pub pois: &'a [Poi],
    /// Measured on earlier visits, if any.
    pub scale: Option<f64>,
}

#[derive(Clone, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GuideView {
    pub variant: String,
    /// Icon kind ("boss", "entrance", "waypoint", …) and its label.
    pub target_kind: String,
    pub target: String,
    /// Screen direction in degrees: 0 = right, 90 = down.
    pub degrees: f32,
    /// Estimated distance in minimap cells.
    pub distance: f32,
    /// 0-1: how far ahead this hypothesis is of the best one pointing elsewhere.
    pub confidence: f32,
    /// The scale was measured (boss seen or earlier visit), not the default guess.
    pub measured: bool,
    pub turned: bool,
}

struct Hypothesis {
    score: f64,
    view: GuideView,
}

fn start_icon(variant: &Variant, from_names: &[String]) -> Option<usize> {
    let icons = &variant.icons;
    // Coming through an entrance starts there; otherwise at the drawn start (a new character).
    icons
        .iter()
        .position(|i| {
            i.id == "entrance" && i.label.as_deref().is_some_and(|l| from_names.iter().any(|n| same_name(l, n)))
        })
        .or_else(|| icons.iter().position(|i| i.id == "myplayer"))
}

/// Objectives in order: the drawn route from the start, else bosses/rewards, then exits.
fn route(variant: &Variant, start: usize) -> Vec<usize> {
    let icons = &variant.icons;
    let valid = |i: usize| i < icons.len() && i != start;
    let mut order = Vec::new();
    let mut at = start;
    while let Some(next) =
        variant.lines.iter().find(|l| l.from == at && l.to != l.from && valid(l.to) && !order.contains(&l.to))
    {
        order.push(next.to);
        at = next.to;
    }
    if order.is_empty() {
        let rank = |id: &str| match id {
            "boss" | "quest" | "reward generic" | "lute" | "tools" => 0,
            "entrance" => 2,
            "waypoint" | "checkpoint" | "myplayer" | "empty" => 9,
            _ => 1,
        };
        order = (0..icons.len()).filter(|&i| valid(i) && rank(&icons[i].id) < 9).collect();
        order.sort_by_key(|&i| rank(&icons[i].id));
    }
    order
}

pub fn guide(catalog: &Catalog, ev: &Evidence) -> Option<GuideView> {
    let variants = catalog.layouts.get(ev.area)?;
    let from_names = catalog.names.get(ev.arrived_from).cloned().unwrap_or_default();
    let base_scale = ev.scale.unwrap_or(DEFAULT_SCALE);
    let player = (ev.player.0 as f64, ev.player.1 as f64);
    let mut hypotheses = Vec::new();

    for variant in variants {
        let Some(start) = start_icon(variant, &from_names) else { continue };
        let order = route(variant, start);
        if order.is_empty() {
            continue;
        }
        let origin = (variant.icons[start].x, variant.icons[start].y);
        let rel = |i: usize| (variant.icons[i].x - origin.0, variant.icons[i].y - origin.1);
        // A boss that has spoken pins one point of the diagram.
        let spoken = order.iter().find_map(|&i| {
            let icon = &variant.icons[i];
            let label = icon.label.as_deref().filter(|_| icon.id == "boss")?;
            let poi = ev.pois.iter().find(|p| p.kind == "npc" && same_name(&p.label, label))?;
            Some((i, (poi.x as f64, poi.y as f64)))
        });

        for (n, t) in TURNS.iter().enumerate() {
            let mut score = if n == 0 { 1.0 } else { TURNED_PRIOR };
            let mut scale = base_scale;
            let mut measured = ev.scale.is_some();
            if let Some((i, seen)) = spoken {
                let drawn = turn(t, rel(i));
                if angle_between(drawn, seen) > BOSS_ANGLE {
                    continue;
                }
                let length = drawn.0.hypot(drawn.1);
                if length > 1e-6 {
                    scale = seen.0.hypot(seen.1) / length;
                    measured = true;
                }
            }
            // Smallest scale at which the walls fit the diagram's box (95th percentile).
            let mut needed: Vec<f64> = ev
                .walls
                .iter()
                .map(|&(x, y)| {
                    let (u, v) = unturn(t, (x as f64, y as f64));
                    let room = |d: f64, at: f64| if d > 0.0 { d / (1.0 + MARGIN - at) } else { -d / (at + MARGIN) };
                    room(u, origin.0).max(room(v, origin.1))
                })
                .collect();
            if !needed.is_empty() {
                needed.sort_by(f64::total_cmp);
                let fit = needed[(needed.len() * 95 / 100).min(needed.len() - 1)];
                score *= (scale / fit.max(scale)).powi(2);
                if !measured {
                    scale = scale.max(fit);
                }
            }
            let at = |i: usize| {
                let (x, y) = turn(t, rel(i));
                (x * scale, y * scale)
            };
            let reached = |i: usize| {
                let spot = at(i);
                let near = REACH * scale;
                ev.trail.iter().any(|&[x, y]| (x as f64 - spot.0).hypot(y as f64 - spot.1) < near)
                    || spoken.is_some_and(|(s, _)| s == i)
            };
            let Some(&target) = order.iter().find(|&&i| !reached(i)).or(order.last()) else { continue };
            let spot = at(target);
            let (dx, dy) = (spot.0 - player.0, spot.1 - player.1);
            let icon = &variant.icons[target];
            hypotheses.push(Hypothesis {
                score,
                view: GuideView {
                    variant: variant.name.clone(),
                    target_kind: icon.id.clone(),
                    target: icon.label.clone().unwrap_or_default(),
                    degrees: dy.atan2(dx).to_degrees() as f32,
                    distance: dx.hypot(dy) as f32,
                    confidence: 0.0,
                    measured,
                    turned: n != 0,
                },
            });
        }
    }

    hypotheses.sort_by(|a, b| b.score.total_cmp(&a.score));
    let best = hypotheses.first()?;
    let direction = |v: &GuideView| {
        let r = (v.degrees as f64).to_radians();
        (r.cos(), r.sin())
    };
    let rival = hypotheses
        .iter()
        .skip(1)
        .find(|h| angle_between(direction(&h.view), direction(&best.view)) > SAME_WAY)
        .map_or(0.0, |h| h.score);
    let mut view = best.view.clone();
    view.confidence = (1.0 - rival / best.score.max(1e-9)) as f32;
    Some(view)
}

/// Scale measured on a finished visit: the distance from arrival to a boss that spoke
/// or to the exit the player left by, divided by that distance on the matching diagram.
pub fn measure_scale(catalog: &Catalog, area: &str, arrived_from: &str, pois: &[Poi]) -> Option<f64> {
    let variants = catalog.layouts.get(area)?;
    let from_names = catalog.names.get(arrived_from).cloned().unwrap_or_default();
    let mut found = Vec::new();
    for variant in variants {
        let Some(start) = start_icon(variant, &from_names) else { continue };
        let origin = (variant.icons[start].x, variant.icons[start].y);
        for poi in pois.iter().filter(|p| p.kind == "npc" || p.kind == "left") {
            let names = if poi.kind == "left" { catalog.names.get(&poi.label).cloned().unwrap_or_default() } else { vec![poi.label.clone()] };
            let wanted = if poi.kind == "left" { "entrance" } else { "boss" };
            for icon in &variant.icons {
                let label = icon.label.as_deref().unwrap_or("");
                if icon.id != wanted || !names.iter().any(|n| same_name(n, label)) {
                    continue;
                }
                let drawn = (icon.x - origin.0, icon.y - origin.1);
                let seen = (poi.x as f64, poi.y as f64);
                if angle_between(drawn, seen) <= BOSS_ANGLE && drawn.0.hypot(drawn.1) > 0.1 {
                    found.push(seen.0.hypot(seen.1) / drawn.0.hypot(drawn.1));
                }
            }
        }
    }
    found.sort_by(f64::total_cmp);
    found.get(found.len() / 2).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poi(kind: &str, label: &str, x: i32, y: i32) -> Poi {
        Poi { kind: kind.into(), label: label.into(), x, y, at: 0 }
    }

    #[test]
    fn matches_names_loosely() {
        assert!(same_name("Clearfell Encampent", "Clearfell Encampment"));
        assert!(same_name("The Bloated Miller", "Miller"));
        assert!(!same_name("Clearfell", "The Grelwood"));
    }

    #[test]
    fn every_bundled_zone_loads() {
        let catalog = Catalog::load();
        assert!(catalog.layouts.len() >= 90);
        assert!(catalog.layouts.contains_key("G1_1"));
        assert_eq!(catalog.names.get("G1_1").unwrap()[0], "The Riverbank");
    }

    /// Riverbank v1 runs from bottom-left to the top-right; v2 mirrors it. Walls
    /// explored far to the right of the start only fit v1.
    #[test]
    fn explored_walls_pick_the_variant() {
        let catalog = Catalog::load();
        let walls: Vec<(i32, i32)> = (0..600).map(|x| (x, -x / 3)).collect();
        let ev = Evidence {
            area: "G1_1",
            arrived_from: "",
            walls: &walls,
            trail: &[[0, 0]],
            player: (0, 0),
            pois: &[],
            scale: None,
        };
        let view = guide(&catalog, &ev).unwrap();
        assert_eq!(view.variant, "1");
        assert!(!view.turned);
        assert_eq!(view.target, "Miller");
        // Miller is up and to the right: between 0 and -90 degrees on screen.
        assert!(view.degrees < 0.0 && view.degrees > -90.0, "{}", view.degrees);
        assert!(view.confidence > 0.5, "{}", view.confidence);
    }

    #[test]
    fn a_boss_that_spoke_sets_the_scale_and_the_next_target() {
        let catalog = Catalog::load();
        // v1: start (0.06, 0.76), Miller (0.66, 0.37): 0.60 right, 0.39 up.
        let miller = poi("npc", "The Bloated Miller", 1200, -780);
        let ev = Evidence {
            area: "G1_1",
            arrived_from: "",
            walls: &[],
            trail: &[[0, 0], [1200, -780]],
            player: (1200, -780),
            pois: std::slice::from_ref(&miller),
            scale: None,
        };
        let view = guide(&catalog, &ev).unwrap();
        assert_eq!((view.variant.as_str(), view.target.as_str()), ("1", "Clearfell Encampent"));
        assert!(view.measured);
        assert!(view.distance > 200.0 && view.distance < 500.0, "{}", view.distance);
        let scale = measure_scale(&catalog, "G1_1", "", &[miller]).unwrap();
        assert!((scale - 2000.0).abs() < 100.0, "{scale}");
    }

    #[test]
    fn needs_a_known_arrival_point() {
        let catalog = Catalog::load();
        // Clearfell's diagrams start at an entrance; coming from an unknown zone there is no anchor.
        let ev = Evidence { area: "G1_2", arrived_from: "", walls: &[], trail: &[], player: (0, 0), pois: &[], scale: None };
        assert!(guide(&catalog, &ev).is_none());
        let ev = Evidence { arrived_from: "G1_town", ..ev };
        assert!(guide(&catalog, &ev).is_some());
    }

    #[test]
    fn back_from_town_starts_at_the_town_entrance() {
        let catalog = Catalog::load();
        let ev = Evidence { area: "G1_1", arrived_from: "G1_town", walls: &[], trail: &[], player: (0, 0), pois: &[], scale: None };
        // No drawn route leads away from the encampment entrance, so the boss is next.
        let view = guide(&catalog, &ev).unwrap();
        assert_eq!(view.target, "Miller");
    }
}
