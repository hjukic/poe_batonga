//! The arrow: where the next objective probably is, from Qutm's layout diagrams
//! (`data/layouts/qutm/layouts.json`) fitted to what we have recorded of this visit.
//!
//! Coordinates keep the minimap's screen axes: x right, y down. Schematics are
//! hypotheses, not measured maps. Competing bearings suppress the arrow. Dialogue
//! records the PLAYER's encounter location, never the moving boss's coordinates,
//! and never proves a kill or completion.

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
/// Two directions closer than this count as agreeing.
const SAME_WAY: f64 = 45.0;
/// Minimum lead supported by evidence, not by a preferred diagram orientation.
const MIN_CONFIDENCE: f32 = 0.3;
/// Dialogue does not locate a boss precisely. Inside the encounter area there is
/// no defensible bearing to a moving enemy; show the objective without an arrow.
const ENCOUNTER_RADIUS: f64 = 32.0;

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

/// CSS/SVG and the minimap share these axes; no camera/isometric correction is
/// appropriate. A coincident target has no direction (atan2(0, 0) is misleading).
fn screen_bearing(dx: f64, dy: f64) -> Option<f32> {
    (dx.hypot(dy) > 1e-6).then(|| dy.atan2(dx).to_degrees() as f32)
}

/// Fit a continuous rotation, keeping both handedness hypotheses. Snapping to
/// quarter turns left up to 40 degrees of error even after observing a landmark.
fn align(drawn: (f64, f64), seen: (f64, f64), mirror: bool) -> ([f64; 4], f64) {
    let sign = if mirror { -1.0 } else { 1.0 };
    let angle = seen.1.atan2(seen.0) - drawn.1.atan2(sign * drawn.0);
    let (s, c) = angle.sin_cos();
    ([c * sign, -s, s * sign, c], seen.0.hypot(seen.1) / drawn.0.hypot(drawn.1))
}

/// What we know about the current visit, in minimap cells relative to the arrival point.
pub struct Evidence<'a> {
    pub area: &'a str,
    /// Area id the player came from; the arrival point is that zone's entrance.
    pub arrived_from: &'a str,
    pub walls: &'a [(i32, i32)],
    pub player: (i32, i32),
    pub pois: &'a [Poi],
    /// Explicit objective completion from the checklist, never proximity/dialogue.
    pub completed: &'a [String],
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
    pub degrees: Option<f32>,
    /// "estimated", "unconfirmed", or "encounter" (player location at dialogue).
    pub source: String,
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
        let order: Vec<usize> = route(variant, start).into_iter().filter(|&i| {
            let label = variant.icons[i].label.as_deref().unwrap_or("");
            !ev.completed.iter().any(|done| same_name(label, done))
        }).collect();
        if order.is_empty() {
            continue;
        }
        let origin = (variant.icons[start].x, variant.icons[start].y);
        let rel = |i: usize| (variant.icons[i].x - origin.0, variant.icons[i].y - origin.1);
        let target = order[0];
        let target_icon = &variant.icons[target];
        // The observed encounter area takes precedence over any diagram guess.
        // Remain on this boss until completion is explicitly confirmed.
        if let Some(poi) = ev.pois.iter().find(|p| p.kind == "npc" && target_icon.id == "boss"
            && same_name(&p.label, target_icon.label.as_deref().unwrap_or(""))) {
            let (dx, dy) = (poi.x as f64 - player.0, poi.y as f64 - player.1);
            let distance = dx.hypot(dy);
            hypotheses.push(Hypothesis {
                score: 1.0,
                view: GuideView {
                    variant: variant.name.clone(), target_kind: target_icon.id.clone(),
                    target: target_icon.label.clone().unwrap_or_default(),
                    degrees: if distance > ENCOUNTER_RADIUS { screen_bearing(dx, dy) } else { None },
                    source: "encounter".into(), distance: distance as f32,
                    confidence: 0.0, measured: true, turned: false,
                },
            });
            continue;
        }
        // An encountered boss can calibrate a later objective, but its dialogue
        // is an approximate landmark, not evidence that the objective is done.
        let spoken = variant.icons.iter().enumerate().find_map(|(i, icon)| {
            let label = icon.label.as_deref().filter(|_| icon.id == "boss")?;
            let poi = ev.pois.iter().find(|p| p.kind == "npc" && same_name(&p.label, label))?;
            let seen = (poi.x as f64, poi.y as f64);
            (rel(i).0.hypot(rel(i).1) > 0.1 && seen.0.hypot(seen.1) > ENCOUNTER_RADIUS)
                .then_some((i, seen))
        });

        let transforms: Vec<([f64; 4], f64)> = if let Some((i, seen)) = spoken {
            [false, true].into_iter().map(|mirror| align(rel(i), seen, mirror)).collect()
        } else {
            TURNS.iter().map(|&t| (t, base_scale)).collect()
        };
        for (t, mut scale) in transforms {
            let mut score = 1.0;
            let measured = spoken.is_some() || ev.scale.is_some();
            // Smallest scale at which the walls fit the diagram's box (95th percentile).
            let mut needed: Vec<f64> = ev
                .walls
                .iter()
                .map(|&(x, y)| {
                    let (u, v) = unturn(&t, (x as f64, y as f64));
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
                let (x, y) = turn(&t, rel(i));
                (x * scale, y * scale)
            };
            let spot = at(target);
            let (dx, dy) = (spot.0 - player.0, spot.1 - player.1);
            let icon = &variant.icons[target];
            hypotheses.push(Hypothesis {
                score,
                view: GuideView {
                    variant: variant.name.clone(),
                    target_kind: icon.id.clone(),
                    target: icon.label.clone().unwrap_or_default(),
                    degrees: screen_bearing(dx, dy),
                    source: "estimated".into(),
                    distance: dx.hypot(dy) as f32,
                    confidence: 0.0,
                    measured,
                    turned: t != TURNS[0],
                },
            });
        }
    }

    hypotheses.sort_by(|a, b| b.score.total_cmp(&a.score));
    let best = hypotheses.first()?;
    let direction = |v: &GuideView| {
        let r = (v.degrees.unwrap_or(0.0) as f64).to_radians();
        (r.cos(), r.sin())
    };
    let rival = hypotheses
        .iter()
        .skip(1)
        .find(|h| h.view.target_kind != best.view.target_kind
            || !same_name(&h.view.target, &best.view.target)
            || angle_between(direction(&h.view), direction(&best.view)) > SAME_WAY)
        .map_or(0.0, |h| h.score);
    let mut view = best.view.clone();
    view.confidence = (1.0 - rival / best.score.max(1e-9)) as f32;
    if view.confidence < MIN_CONFIDENCE || (view.source == "estimated" && view.degrees.is_none()) {
        view.degrees = None;
        view.source = "unconfirmed".into();
    }
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
                // Length is invariant under rotation/reflection. Do not reject a
                // real observation because the schematic points another way.
                if seen.0.hypot(seen.1) > ENCOUNTER_RADIUS && drawn.0.hypot(drawn.1) > 0.1 {
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

    fn evidence() -> Evidence<'static> {
        Evidence { area: "G1_1", arrived_from: "", walls: &[], player: (0, 0), pois: &[], completed: &[], scale: None }
    }

    #[test]
    fn no_evidence_does_not_invent_a_confident_orientation() {
        let view = guide(&Catalog::load(), &evidence()).unwrap();
        assert_eq!(view.target, "Miller");
        assert_eq!(view.degrees, None);
        assert_eq!(view.source, "unconfirmed");
        assert_eq!(view.confidence, 0.0);
    }

    #[test]
    fn miller_dialogue_at_fight_start_does_not_skip_to_town() {
        let pois = [poi("npc", "The Bloated Miller", 1200, -780)];
        let ev = Evidence { player: (1200, -780), pois: &pois, ..evidence() };
        let view = guide(&Catalog::load(), &ev).unwrap();
        assert_eq!(view.target, "Miller");
        assert_eq!(view.source, "encounter");
        assert_eq!(view.degrees, None); // No known coordinates of the moving boss.
    }

    #[test]
    fn standing_on_a_guessed_boss_location_does_not_complete_it() {
        let ev = Evidence { player: (715, -470), scale: Some(1200.0), ..evidence() };
        assert_eq!(guide(&Catalog::load(), &ev).unwrap().target, "Miller");
    }

    #[test]
    fn encounter_bearing_uses_live_screen_axes_not_the_schematic() {
        let pois = [poi("npc", "The Bloated Miller", 100, -60)];
        let ev = Evidence { player: (60, -10), pois: &pois, ..evidence() };
        let view = guide(&Catalog::load(), &ev).unwrap();
        assert_eq!(view.target, "Miller");
        assert_eq!(view.source, "encounter");
        assert!((view.degrees.unwrap() - (-51.34019)).abs() < 0.001);
        // Passing the recorded encounter reverses the bearing.
        let ev = Evidence { player: (140, -110), ..ev };
        let view = guide(&Catalog::load(), &ev).unwrap();
        assert!((view.degrees.unwrap() - 128.65981).abs() < 0.001);
    }

    #[test]
    fn only_confirmed_completion_advances_to_the_exit() {
        let catalog = Catalog::load();
        let pois = [poi("npc", "The Bloated Miller", 1200, -780)];
        let completed = ["The Bloated Miller".into()];
        let ev = Evidence { player: (1200, -780), pois: &pois, completed: &completed, ..evidence() };
        let view = guide(&catalog, &ev).unwrap();
        assert_eq!(view.target, "Clearfell Encampent");
        assert!(view.measured);
        let completed = ["Miller".into(), "Clearfell Encampment".into()];
        assert!(guide(&catalog, &Evidence { completed: &completed, ..ev }).is_none());
    }

    #[test]
    fn screen_bearings_match_human_directions() {
        for (x, y, degrees) in [(10., 0., 0.), (10., 10., 45.), (0., 10., 90.),
            (-10., 10., 135.), (-10., 0., 180.), (-10., -10., -135.),
            (0., -10., -90.), (10., -10., -45.)] {
            assert_eq!(screen_bearing(x, y), Some(degrees));
        }
        assert_eq!(screen_bearing(0., 0.), None);
    }

    #[test]
    fn landmark_alignment_is_continuous_in_all_quadrants_and_mirrors() {
        for degrees in [-170.0_f64, -113., -37., 22., 68., 149.] {
            let seen = (degrees.to_radians().cos() * 600., degrees.to_radians().sin() * 600.);
            for mirror in [false, true] {
                let (t, scale) = align((0.6, -0.4), seen, mirror);
                let (x, y) = turn(&t, (0.6, -0.4));
                assert!((x * scale - seen.0).abs() < 1e-6);
                assert!((y * scale - seen.1).abs() < 1e-6);
            }
        }
    }

    fn schematic(exit: (f64, f64)) -> Catalog {
        Catalog {
            layouts: HashMap::from([("G1_1".into(), vec![Variant {
                name: "test".into(), icons: vec![
                    Icon { id: "myplayer".into(), label: None, x: 0.2, y: 0.5 },
                    Icon { id: "boss".into(), label: Some("Miller".into()), x: 0.5, y: 0.5 },
                    Icon { id: "entrance".into(), label: Some("Town".into()), x: exit.0, y: exit.1 },
                ], lines: vec![Line { from: 0, to: 1 }, Line { from: 1, to: 2 }],
            }])]), names: HashMap::new(),
        }
    }

    #[test]
    fn a_landmark_does_not_resolve_mirrored_exit_bearings() {
        let pois = [poi("npc", "Miller", 600, 0)];
        let completed = ["Miller".into()];
        let ev = Evidence { player: (600, 0), pois: &pois, completed: &completed, ..evidence() };
        let view = guide(&schematic((0.5, 0.8)), &ev).unwrap();
        assert_eq!(view.target, "Town");
        assert_eq!(view.source, "unconfirmed");
        assert_eq!(view.degrees, None);
    }

    #[test]
    fn an_aligned_exit_uses_the_observed_non_quarter_turn() {
        let pois = [poi("npc", "Miller", 600, -300)];
        let completed = ["Miller".into()];
        let ev = Evidence { player: (600, -300), pois: &pois, completed: &completed, ..evidence() };
        let view = guide(&schematic((0.8, 0.5)), &ev).unwrap();
        assert_eq!(view.source, "estimated");
        assert!((view.degrees.unwrap() + 26.56505).abs() < 0.001);
    }

    #[test]
    fn needs_a_known_arrival_point() {
        let catalog = Catalog::load();
        // Clearfell's diagrams start at an entrance; coming from an unknown zone there is no anchor.
        let ev = Evidence { area: "G1_2", ..evidence() };
        assert!(guide(&catalog, &ev).is_none());
        let ev = Evidence { arrived_from: "G1_town", ..ev };
        assert!(guide(&catalog, &ev).is_some());
    }

    #[test]
    fn back_from_town_starts_at_the_town_entrance() {
        let catalog = Catalog::load();
        let ev = Evidence { arrived_from: "G1_town", ..evidence() };
        // No drawn route leads away from the encampment entrance, so the boss is next.
        let view = guide(&catalog, &ev).unwrap();
        assert_eq!(view.target, "Miller");
    }

    #[test]
    fn scale_measurement_does_not_assume_screen_orientation() {
        let catalog = schematic((0.8, 0.5));
        for (x, y) in [(600, 0), (0, -600), (-600, 0), (0, 600)] {
            assert_eq!(measure_scale(&catalog, "G1_1", "", &[poi("npc", "Miller", x, y)]), Some(2000.0));
        }
    }
}
