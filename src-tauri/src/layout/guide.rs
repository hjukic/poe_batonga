//! What the compass needle is made of: the targets it may head for, the needle it
//! shows, and the turn between the world (the layout graphs) and the in-game map.

use serde::Serialize;

/// The in-game map shows the world grid turned 45° with its y axis pointing up, seen at a
/// slant: the up-down axis is drawn `FLAT` times as long as the left-right one.
/// screen = ((x - y), -(x + y) * FLAT) / √2, times `graph::SCALE` cells per world unit.
/// Measured 2026-11-12 on the owner's two Clearfell instances against the game's own layout
/// graphs: three exits read by name fit ClearFell_W_E_N within 25-37 cells at 0.85 across and
/// 0.46 up-down (the next layout 300-400 cells off); a square map (0.78 both ways, fitted
/// once on poe2way's outlines) fitted no layout. Review by a second agent found it first.
pub(super) const FLAT: f64 = 0.54;

pub(super) fn to_screen((x, y): (f64, f64)) -> (f64, f64) {
    let c = std::f64::consts::FRAC_1_SQRT_2;
    (c * (x - y), -c * (x + y) * FLAT)
}

/// The inverse of `to_screen`: screen-oriented units back to world (x east, y north).
pub(super) fn from_screen((u, v): (f64, f64)) -> (f64, f64) {
    let c = std::f64::consts::FRAC_1_SQRT_2;
    let v = v / FLAT;
    (c * (u - v), -c * (u + v))
}

#[cfg(test)]
#[test]
fn from_screen_undoes_to_screen() {
    for p in [(0.0, 0.0), (100.0, 0.0), (0.0, 100.0), (-37.0, 412.5)] {
        let (x, y) = from_screen(to_screen(p));
        assert!((x - p.0).abs() < 1e-9 && (y - p.1).abs() < 1e-9, "{p:?} -> {:?}", (x, y));
    }
}

pub(super) fn angle_between(a: f32, b: f32) -> f64 {
    let d = ((a - b) as f64).rem_euclid(360.0);
    d.min(360.0 - d)
}

/// Screen direction in degrees (0 = right, 90 = down). A coincident target has none.
pub(super) fn screen_bearing(dx: f64, dy: f64) -> Option<f32> {
    (dx.hypot(dy) > 1e-6).then(|| dy.atan2(dx).to_degrees() as f32)
}

/// "G1_2" < "G1_11": compare the numbers in area ids as numbers.
pub(super) fn area_order(id: &str) -> Vec<(u32, String)> {
    id.split('_').map(|part| (part.parse().unwrap_or(u32::MAX), part.to_string())).collect()
}

/// Something the compass may head for, from an open checklist step: a zone exit
/// (`exit`: the area id it leads to) or a tagged point of the layout graph (`tag`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct NavTarget {
    pub label: String,
    pub exit: Option<String>,
    pub tag: Option<String>,
    /// The step is optional.
    pub optional: bool,
}

#[derive(Clone, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GuideView {
    pub variant: String,
    /// "entrance" for zone exits.
    pub target_kind: String,
    pub target: String,
    /// Screen direction in degrees: 0 = right, 90 = down.
    pub degrees: Option<f32>,
    /// "estimated" (layout confirmed), "rough" (likely layout) or "unconfirmed"
    /// (layout not recognised; `degrees` is then a best guess, if any, shown gray).
    pub source: String,
    /// Walking distance in minimap cells.
    pub distance: f32,
    /// 0-1: how sure the needle is.
    pub confidence: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn area_ids_sort_by_number() {
        assert!(area_order("G1_3") < area_order("G1_11"));
        assert!(area_order("G1_2") < area_order("G1_3"));
    }
}
