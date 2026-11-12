//! Big map (Tab) reading. With the big map open the game hides the corner minimap and
//! draws the explored map across the screen, centred on the player. This reads the map
//! lines around the player and hands the recorder frames in the minimap's units, so one
//! recording continues across both views.
//!
//! Measured on Clearfell at 3840x2160 (2026-11-12, `docs/map-reading.md`, "Big map"):
//! the big map draws at the minimap's scale (1.00 big-map pixels per minimap pixel),
//! with the same orientation and line colour; its player marker sits 21 px above the
//! centre of the game window.

use super::{is_marker, is_wall, labels, Image, MinimapView, CELL, CELL_MIN};
use serde::Serialize;

/// Big-map pixels per minimap pixel. Fit of the big map's walls onto recorded minimap
/// walls of the same place: 1.00 (0.995-1.005 score the same), no turn.
pub const SCALE: f64 = 1.0;
/// The big map's player marker, as a fraction of the game window (1919, 1059 px of 3840x2160).
const PLAYER: [f64; 2] = [0.4997, 0.4903];
/// The minimap's player marker, from the window's right and top edges, as a fraction of
/// the window's height: the game sizes its minimap by height and anchors it to the right
/// edge (about 3556, 284 px of 3840x2160, from a quarter-size screenshot, so good to
/// about 4 px; the reader then learns the real spot from the minimap itself).
const MINIMAP_PLAYER: f64 = 0.1315;
/// How far from the expected point the marker is looked for, in pixels at 2160 px height.
const MARKER_RADIUS: f64 = 16.0;
/// The player cross: 58 px at 2160 px height (11x10 px). An orange blob counts as the
/// cross between 0.4 and 2.5 times that; an exit icon is 360 px, fire specks a few.
const CROSS_PX: f64 = 58.0;
const CROSS_RANGE: [f64; 2] = [0.4, 2.5];
/// Touching wall cells fewer than this are dropped: map lines are long, lit ground
/// behind the translucent map leaves specks (Clearfell, 600x518 cells around the player:
/// 226 groups of which 212 under 15 cells; every line cell kept at 12).
const MIN_GROUP: usize = 12;
/// A group with more than this share of inside cells (all eight neighbours walls too) is
/// a filled area showing through the map (bluish floor, ice), not a line: lines are at
/// most two cells thick and have none.
const MAX_INSIDE: f64 = 0.25;
/// Wall cells a frame needs to count as the big map, and the most of the frame they may
/// cover (Clearfell: 1.7%).
const WALLS_MIN: usize = 40;
const WALLS_MAX: f64 = 0.15;
/// Width of the map lines in pixels at 2160 px height (wall pixels per edge pixel, times
/// two). At the default zoom 15 places of a Clearfell screenshot gave 3.41-4.54; the
/// fixture drawn 0.8 times as large gives 3.08, 1.5 times 4.96 (1.25 times: 4.31, which
/// this cannot tell from the default). Outside the range the big map is taken to be
/// zoomed, and its frames would not match the minimap's.
const LINE_WIDTH: [f64; 2] = [3.25, 4.75];

/// The frame handed to the recorder: its size in cells and the player's cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub cols: usize,
    pub rows: usize,
    pub player: [f32; 2],
    /// The capture size the recorder stores for this frame, in minimap pixels.
    pub capture_px: [usize; 2],
    /// The player cell follows where the minimap's marker is seen.
    pub follows_minimap: bool,
}

impl Frame {
    /// The size of a minimap frame, with the player where the minimap draws it, so the
    /// recorder sees no difference when the view changes. `minimap_px` is the minimap
    /// capture's size, `window_px` the game window's, `minimap_origin` the capture's
    /// top-left corner inside the window (all in screen pixels).
    pub fn like_minimap(minimap_px: [usize; 2], window_px: [f64; 2], minimap_origin: [f64; 2]) -> Frame {
        let marker = minimap_marker(window_px);
        let at = |i: usize| ((marker[i] - minimap_origin[i]) / CELL as f64) as f32;
        Frame {
            cols: minimap_px[0].div_ceil(CELL),
            rows: minimap_px[1].div_ceil(CELL),
            player: [at(0), at(1)],
            capture_px: minimap_px,
            follows_minimap: true,
        }
    }

    /// A larger frame with the player in the middle (see `BigFrame::Wide` in minimap.rs);
    /// it stores the minimap's capture size `capture_px`, the same in both views.
    pub fn wide(cols: usize, rows: usize, capture_px: [usize; 2]) -> Frame {
        Frame { cols, rows, player: [cols as f32 / 2.0, rows as f32 / 2.0], capture_px, follows_minimap: false }
    }
}

/// Where the minimap draws the player in a game window of `window_px` pixels.
pub fn minimap_marker(window_px: [f64; 2]) -> [f64; 2] {
    let from_edge = MINIMAP_PLAYER * window_px[1];
    [window_px[0] - from_edge, from_edge]
}

/// How to read one big-map capture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    /// Where the player marker is expected inside the capture, in its pixels.
    pub marker: [f64; 2],
    /// How far from there the marker may be, in pixels.
    pub radius: f64,
    pub frame: Frame,
}

/// The part of the game window to capture for `frame`, in window pixels
/// (x, y, width, height, clipped to the window), and where the marker is expected in it.
pub fn area(window_px: [f64; 2], frame: &Frame) -> ([f64; 4], [f64; 2]) {
    let marker = [PLAYER[0] * window_px[0], PLAYER[1] * window_px[1]];
    // Room for the marker search and for the player cell following the minimap's
    // learnt marker (up to `LEARN_LIMIT` radii from the estimate).
    let margin = radius(window_px) * (1.0 + LEARN_LIMIT) + 2.0;
    let cell = CELL as f64 * SCALE;
    let x0 = (marker[0] - frame.player[0] as f64 * cell - margin).max(0.0);
    let y0 = (marker[1] - frame.player[1] as f64 * cell - margin).max(0.0);
    let x1 = (marker[0] + (frame.cols as f64 - frame.player[0] as f64) * cell + margin).min(window_px[0]);
    let y1 = (marker[1] + (frame.rows as f64 - frame.player[1] as f64) * cell + margin).min(window_px[1]);
    ([x0, y0, x1 - x0, y1 - y0], [marker[0] - x0, marker[1] - y0])
}

/// The learnt minimap marker stays within this many marker radii of the estimate.
pub const LEARN_LIMIT: f64 = 1.5;

pub fn radius(window_px: [f64; 2]) -> f64 {
    MARKER_RADIUS * window_px[1] / 2160.0
}

/// The player cross near `at` (pixel-centre coordinates): of the orange blobs whose centre
/// is within `radius`, the nearest one of about the cross's size. Fire, item labels or an
/// exit icon next to it don't move it; a blob that touches the cross is too large and
/// gives none.
pub fn marker_near(image: &Image, at: [f64; 2], radius: f64) -> Option<[f64; 2]> {
    let size = radius / MARKER_RADIUS;
    // The box holds every whole blob whose centre is within `radius`.
    let reach = radius + 10.0 * size;
    let clamp = |v: f64, n: usize| (v.max(0.0) as usize).min(n);
    let (x0, x1) = (clamp(at[0] - reach, image.width), clamp(at[0] + reach + 1.0, image.width));
    let (y0, y1) = (clamp(at[1] - reach, image.height), clamp(at[1] + reach + 1.0, image.height));
    let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
    let mut orange: Vec<u8> = Vec::with_capacity(w * h);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * image.width + x) * 3;
            orange.push(if is_marker(image.pixels[i], image.pixels[i + 1], image.pixels[i + 2]) { 0 } else { NONE });
        }
    }
    let area = CROSS_PX * size * size;
    let mut best: Option<(f64, [f64; 2])> = None;
    for (_, blob) in blobs(&mut orange, w, h) {
        // A blob cut by the box (not by the image edge) is part of something larger.
        let cut = (blob.x0 == 0 && x0 > 0)
            || (blob.y0 == 0 && y0 > 0)
            || (blob.x1 == w - 1 && x1 < image.width)
            || (blob.y1 == h - 1 && y1 < image.height);
        let n = blob.n as f64;
        if cut || n < CROSS_RANGE[0] * area || n > CROSS_RANGE[1] * area {
            continue;
        }
        let centre = [x0 as f64 + blob.sx / n, y0 as f64 + blob.sy / n];
        let distance = (centre[0] - at[0]).hypot(centre[1] - at[1]);
        if distance <= radius && best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, centre));
        }
    }
    best.map(|(_, centre)| centre)
}

/// A group of touching pixels: count, sums of pixel-centre coordinates, bounding box.
struct Blob {
    n: u32,
    sx: f64,
    sy: f64,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

/// No class in a `blobs` mask.
const NONE: u8 = u8::MAX;

/// The 8-connected groups of pixels of the same class in a `w` x `h` mask, with their
/// class (`NONE`: no class; the mask is cleared on the way).
fn blobs(mask: &mut [u8], w: usize, h: usize) -> Vec<(u8, Blob)> {
    let mut found = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        let class = mask[start];
        if class == NONE {
            continue;
        }
        mask[start] = NONE;
        stack.push(start);
        let mut blob = Blob { n: 0, sx: 0.0, sy: 0.0, x0: w, y0: h, x1: 0, y1: 0 };
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            blob.n += 1;
            blob.sx += x as f64 + 0.5;
            blob.sy += y as f64 + 0.5;
            (blob.x0, blob.y0, blob.x1, blob.y1) = (blob.x0.min(x), blob.y0.min(y), blob.x1.max(x), blob.y1.max(y));
            for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                    let j = ny as usize * w + nx as usize;
                    if mask[j] == class {
                        mask[j] = NONE;
                        stack.push(j);
                    }
                }
            }
        }
        found.push((class, blob));
    }
    found
}

/// Reads a big-map capture into a frame in minimap units: the walls around the player,
/// resampled by `SCALE`, with the player at `plan.frame.player`. Visible only with the
/// player cross where the big map draws it, enough map lines, not too much of the frame
/// covered, and lines as wide as at the default zoom; else the message says which failed.
/// Also returns the map icons around the player while the big map is read.
#[cfg(test)]
pub fn read(image: &Image, plan: &Plan) -> (MinimapView, Vec<Icon>) {
    read_with(image, plan, &[], false)
}

/// `read`, with the icons looked for over the whole image but the `skip` rectangles, and
/// exit labels read when `labels` (whole-window frames).
pub fn read_with(image: &Image, plan: &Plan, skip: &[[usize; 4]], labels: bool) -> (MinimapView, Vec<Icon>) {
    let Some(centre) = marker_near(image, plan.marker, plan.radius) else {
        return (not_visible(plan, "player marker not found".into()), vec![]);
    };
    let (walls, width) = walls_in(image, centre, &plan.frame, SCALE);
    let width = width / size(plan);
    let problem = if walls.len() < WALLS_MIN {
        Some("too few map lines".to_string())
    } else if walls.len() as f64 > WALLS_MAX * (plan.frame.cols * plan.frame.rows) as f64 {
        Some("too much of the view looks like map lines".to_string())
    } else if !(LINE_WIDTH[0]..=LINE_WIDTH[1]).contains(&width) {
        Some(format!("map lines {width:.1} px wide, not {:.2}-{:.2} (zoomed?)", LINE_WIDTH[0], LINE_WIDTH[1]))
    } else {
        None
    };
    if let Some(problem) = problem {
        return (not_visible(plan, problem), vec![]);
    }
    let view = MinimapView {
        enabled: true,
        visible: true,
        message: "Reading big map".into(),
        cols: plan.frame.cols,
        rows: plan.frame.rows,
        walls,
        player: Some(plan.frame.player),
    };
    (view, icons_in(image, centre, SCALE, size(plan), 2, skip, labels))
}

fn not_visible(plan: &Plan, why: String) -> MinimapView {
    MinimapView {
        enabled: true,
        visible: false,
        message: format!("Big map not read: {why}"),
        cols: plan.frame.cols,
        rows: plan.frame.rows,
        walls: vec![],
        player: None,
    }
}

/// The game window's height relative to 2160 px, from a plan.
pub fn size(plan: &Plan) -> f64 {
    plan.radius / MARKER_RADIUS
}

/// Wall cells of `frame`, with the image pixel `centre` (the player) on the frame's player
/// cell and `scale` image pixels per minimap pixel; and the mean width in image pixels of
/// the lines kept (wall pixels per edge pixel, times two).
fn walls_in(image: &Image, centre: [f64; 2], frame: &Frame, scale: f64) -> (Vec<[u16; 2]>, f64) {
    const OUTSIDE: u32 = u32::MAX;
    let Frame { cols, rows, player, .. } = *frame;
    let (w, h) = (image.width, image.height);
    let cell = CELL as f64;
    // The frame column of each image column and the frame row of each image row:
    // image pixel -> minimap pixel is (p - centre) / scale + player * CELL.
    let along = |n: usize, c: f64, at: f32, cells: usize| -> Vec<u32> {
        (0..n)
            .map(|p| {
                let v = (p as f64 + 0.5 - c) / scale + at as f64 * cell;
                if v >= 0.0 && v < (cells * CELL) as f64 { (v / cell) as u32 } else { OUTSIDE }
            })
            .collect()
    };
    let (col_of, row_of) = (along(w, centre[0], player[0], cols), along(h, centre[1], player[1], rows));
    // Wall pixels inside the frame, and wall pixels per cell.
    let mut wall = vec![false; w * h];
    let mut counts = vec![0u32; cols * rows];
    for y in 0..h {
        let row = row_of[y];
        if row == OUTSIDE {
            continue;
        }
        for x in 0..w {
            let (i, col) = (y * w + x, col_of[x]);
            if col != OUTSIDE && is_wall(image.pixels[i * 3], image.pixels[i * 3 + 1], image.pixels[i * 3 + 2]) {
                wall[i] = true;
                counts[row as usize * cols + col as usize] += 1;
            }
        }
    }
    // A minimap cell holds CELL x CELL pixels; scaled, a big-map cell holds scale² as many.
    let need = ((CELL_MIN as f64 * scale * scale).round() as u32).max(1);
    let occupied: Vec<bool> = counts.iter().map(|&n| n >= need).collect();
    let walls = lines(&occupied, cols, rows);
    let mut kept = vec![false; cols * rows];
    for &[x, y] in &walls {
        kept[y as usize * cols + x as usize] = true;
    }
    let (mut pixels, mut edges) = (0u32, 0u32);
    for y in 1..h.saturating_sub(1) {
        if row_of[y] == OUTSIDE {
            continue;
        }
        for x in 1..w.saturating_sub(1) {
            let i = y * w + x;
            if !wall[i] || !kept[row_of[y] as usize * cols + col_of[x] as usize] {
                continue;
            }
            pixels += 1;
            if !(wall[i - 1] && wall[i + 1] && wall[i - w] && wall[i + w]) {
                edges += 1;
            }
        }
    }
    (walls, 2.0 * pixels as f64 / edges.max(1) as f64)
}

/// A map icon, in minimap cells from the player (x right, y down).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Icon {
    /// Found by colour: "exit" (orange zone exit), "checkpoint" or "waypoint". Named by the
    /// sprite matcher (`identify`): "boss" (an orange skull, or a boss's own icon while it
    /// lives), "boss-dead" (a boss's own icon, grey), "dead" (a grey skull: a dead unique
    /// or a dead rare, the game draws both alike), "rare" (a yellow skull), "portal".
    pub kind: &'static str,
    pub x: f32,
    pub y: f32,
    /// An exit's label: the zone it leads to, as given to `set_label_candidates`, when
    /// read (whole-window frames only).
    pub label: Option<String>,
    /// The sprite that matched (an id of `data/icons/index.json`), when one did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sprite: Option<&'static str>,
}

/// Icon colours and least size in pixels at 2160 px height, measured on the big map
/// (Clearfell, 2026-11-12). Exits: orange, the player cross's colour, 359-373 px (the
/// cross is 58). Checkpoints: a cyan crystal over a cyan base (about 100,170,220),
/// 145-185 px together. Waypoint: a pale blue ring (about 170,180,225); labels are
/// neutral grey and don't pass.
const ICONS: [(&str, u32); 3] = [("exit", 150), ("checkpoint", 60), ("waypoint", 80)];
/// Parts of one icon (a checkpoint's crystal and base) are joined within this many pixels.
/// Not exits: an exit icon is one blob (Clearfell, both exits, at every pixel and every
/// second: 359-373 px, 18-19 x 26-27 px, 73-77% of its box filled), and joining would let
/// the player's cross (orange too) and fire or loot text next to it add up to one.
const ICON_JOIN: f64 = 20.0;
/// An exit blob is at most this many times the least exit size (the icon with the cross
/// on it is 431 px; lava, fire or a lit area are larger), its box between these widths
/// per height (the icon 0.67-0.77, with the cross beside it about 1.1; a line of loot
/// text is wider), and at least this share of its box filled (the icon 0.70-0.81 at
/// every second pixel; text and fire edges are thin).
const EXIT_MAX: f64 = 6.0;
const EXIT_ASPECT: [f64; 2] = [0.4, 1.3];
const EXIT_FILL: f64 = 0.45;

/// Whether an orange blob is shaped like an exit icon; `least` is the least size and
/// `cross` the player cross's size (both in samples), counted off when the blob covers
/// the player `at` (in samples): the cross itself never makes an exit grow to one.
fn exit_shaped(blob: &Blob, least: f64, cross: f64, at: [f64; 2]) -> bool {
    let (w, h) = ((blob.x1 - blob.x0 + 1) as f64, (blob.y1 - blob.y0 + 1) as f64);
    let covers = |v: f64, lo: usize, hi: usize| v >= lo as f64 - 1.0 && v <= hi as f64 + 2.0;
    let on_player = covers(at[0], blob.x0, blob.x1) && covers(at[1], blob.y0, blob.y1);
    let n = blob.n as f64 - if on_player { cross } else { 0.0 };
    n >= least
        && (blob.n as f64) <= EXIT_MAX * least
        && (EXIT_ASPECT[0]..=EXIT_ASPECT[1]).contains(&(w / h))
        && blob.n as f64 >= EXIT_FILL * w * h
}

/// The icon kind of a pixel, an index into `ICONS`, or `NONE`.
fn icon_kind(r: u8, g: u8, b: u8) -> u8 {
    let (ri, gi, bi) = (r as i32, g as i32, b as i32);
    if is_marker(r, g, b) {
        0
    } else if bi > 180 && gi - ri > 45 && bi - ri > 80 {
        1
    } else if bi > 190 && ri > 130 && bi - ri > 18 && bi - ri < 75 && gi - ri < 40 {
        2
    } else {
        NONE
    }
}

/// Pixel classes that only point the sprite matcher somewhere (no icon by themselves):
/// pale grey (a grey skull's light half, 42% of its pixels; label text) and warm (orange
/// and yellow that is not the cross's colour: a yellow skull 57%, the orange skull's other
/// 25%). The colour classes 0-2 are tested first and stay as they were.
const PALE: u8 = 3;
const WARM: u8 = 4;

fn hint_kind(r: u8, g: u8, b: u8) -> u8 {
    let (lo, hi) = (r.min(g).min(b), r.max(g).max(b));
    if lo >= 110 && hi - lo <= 45 {
        PALE
    } else if r >= 140 && r as i32 - b as i32 >= 60 {
        WARM
    } else {
        NONE
    }
}

/// Map icons in `image`, relative to the player at pixel `centre`, with `scale` image
/// pixels per minimap pixel. `size` is the image's pixel size relative to 2160 px height.
pub fn icons(image: &Image, centre: [f64; 2], scale: f64, size: f64) -> Vec<Icon> {
    icons_in(image, centre, scale, size, 1, &[], false)
}

/// `icons`, looking at every `stride`-th pixel across and down (icons are 20-30 px wide,
/// so 2 still finds them, at a quarter of the cost) and leaving out the `skip` rectangles
/// (image pixels: x0, y0, x1, y1), where the game's or this app's own panels are.
pub fn icons_in(image: &Image, centre: [f64; 2], scale: f64, size: f64, stride: usize, skip: &[[usize; 4]], read_labels: bool) -> Vec<Icon> {
    let window = size;
    let (w, h) = (image.width.div_ceil(stride), image.height.div_ceil(stride));
    // One byte a sample; most samples are no icon.
    let mut kinds = Vec::with_capacity(w * h);
    for gy in 0..h {
        let y = gy * stride;
        let skipped: Vec<&[usize; 4]> = skip.iter().filter(|r| y >= r[1] && y < r[3]).collect();
        for gx in 0..w {
            let x = gx * stride;
            if skipped.iter().any(|r| x >= r[0] && x < r[2]) {
                kinds.push(NONE);
            } else {
                let i = (y * image.width + x) * 3;
                let (r, g, b) = (image.pixels[i], image.pixels[i + 1], image.pixels[i + 2]);
                let kind = icon_kind(r, g, b);
                kinds.push(if kind == NONE { hint_kind(r, g, b) } else { kind });
            }
        }
    }
    let size = size / stride as f64;
    let found = blobs(&mut kinds, w, h);
    // The player in samples.
    let player = centre.map(|c| (c - 0.5) / stride as f64 + 0.5);
    // Sample centre (in samples) -> image pixel centre.
    let to_px = |v: f64| (v - 0.5) * stride as f64 + 0.5;
    let mut icons: Vec<(Icon, [f64; 2])> = Vec::new();
    for (kind, &(name, least)) in ICONS.iter().enumerate() {
        let exit_least = least as f64 * size * size;
        let shaped = |blob: &Blob| name != "exit" || exit_shaped(blob, exit_least, CROSS_PX * size * size, player);
        // Join blobs of this kind whose centres are close, largest first (exits: none).
        let mut parts: Vec<(u32, f64, f64)> = found
            .iter()
            .filter(|(k, b)| *k as usize == kind && shaped(b))
            .map(|(_, b)| (b.n, b.sx, b.sy))
            .collect();
        parts.sort_by(|a, b| b.0.cmp(&a.0));
        let mut joined: Vec<(u32, f64, f64)> = Vec::new();
        for (n, sx, sy) in parts {
            let at = [sx / n as f64, sy / n as f64];
            let near = joined.iter_mut().find(|(m, jx, jy)| {
                name != "exit" && (jx / *m as f64 - at[0]).hypot(jy / *m as f64 - at[1]) < ICON_JOIN * size
            });
            match near {
                Some(group) => *group = (group.0 + n, group.1 + sx, group.2 + sy),
                None => joined.push((n, sx, sy)),
            }
        }
        let least = (least as f64 * size * size) as u32;
        for (n, sx, sy) in joined.into_iter().filter(|&(n, _, _)| n >= least) {
            let at = [to_px(sx / n as f64), to_px(sy / n as f64)];
            icons.push((Icon { kind: name, ..Icon::default() }, at));
        }
    }
    #[cfg(test)]
    let matching = MATCHING.with(|m| m.get());
    #[cfg(not(test))]
    let matching = true;
    if matching {
        identify(image, &mut icons, &found, stride, window, centre, scale);
    }
    let to_cells = |p: f64, c: f64| ((p - c) / scale / CELL as f64) as f32;
    icons
        .into_iter()
        .map(|(icon, at)| {
            // Exit labels are read only for what is still an exit after the matcher.
            let label = (read_labels && icon.kind == "exit").then(|| labels::read(image, at, window)).flatten();
            Icon { x: to_cells(at[0], centre[0]), y: to_cells(at[1], centre[1]), label, ..icon }
        })
        .collect()
}

/// The game's map icon sprites (`data/icons/`, `scripts/update-icons.sh`): 32x32 RGBA
/// each, the sheet's 64 px cells reduced by half (the sheet's first mip level).
static SPRITE_INDEX: &str = include_str!("../../data/icons/index.json");
static SPRITE_PIXELS: &[u8] = include_bytes!("../../data/icons/sprites.rgba");
const SPRITE_PX: usize = 32;
/// Screen pixels per pixel of a 64 px sprite cell at 2160 px height: the best fit of each
/// sprite on real icons (`sprite_scores_on_the_screenshot`, `live_capture`): `Entrance` 0.48
/// (the exit icon 18-19 x 26-27 px, the sprite's opaque box 39 x 56; 0.46-0.50 score within
/// 0.01), `Checkpoint` 0.52-0.54, `Waypoint` 0.52 (Clearfell), the Bloated Miller's own icon
/// 0.52-0.53 (the Riverbank, his first line): bosses' own icons are tried at 0.53. Skulls,
/// portals and the rest, not seen on a real screen yet, at both.
fn sprite_scales(sprite: &Sprite) -> &'static [f64] {
    match sprite.kind {
        "exit" => &[0.48],
        "checkpoint" | "waypoint" => &[0.53],
        _ if sprite.pair.is_some() => &[0.53],
        _ => &[0.48, 0.53],
    }
}

/// One icon sprite of the game.
pub struct Sprite {
    /// The `MinimapIcons` id ("Entrance", "UniqueMonsterAlive", "GrimTangleBossActive").
    pub id: String,
    /// What the reader calls it: "exit", "checkpoint", "waypoint", "portal", "boss",
    /// "boss-dead", "dead", "rare", "npc", "quest".
    pub kind: &'static str,
    /// A boss pair: its stem ("GrimTangleBoss"), area id and checklist boss name, where
    /// `data/icons/index.json` has them.
    pub pair: Option<String>,
    pub area: Option<String>,
    pub boss: Option<String>,
    /// False for a pair known to be no checklist boss's (the Grelwood's carnivorous plant).
    pub checklist: bool,
    rgba: &'static [u8],
}

/// Every sprite of `data/icons/`, in its order.
pub fn sprites() -> &'static [Sprite] {
    static SPRITES: std::sync::OnceLock<Vec<Sprite>> = std::sync::OnceLock::new();
    SPRITES.get_or_init(|| {
        #[derive(serde::Deserialize)]
        struct Entry {
            id: String,
            kind: String,
            #[serde(default)]
            pair: Option<String>,
            #[serde(default)]
            area: Option<String>,
            #[serde(default)]
            boss: Option<String>,
            #[serde(default = "yes")]
            checklist: bool,
        }
        fn yes() -> bool {
            true
        }
        #[derive(serde::Deserialize)]
        struct Index {
            icons: Vec<Entry>,
        }
        const KINDS: [&str; 10] = ["exit", "checkpoint", "waypoint", "portal", "boss", "boss-dead", "dead", "rare", "npc", "quest"];
        let index: Index = serde_json::from_str(SPRITE_INDEX).expect("data/icons/index.json");
        let bytes = SPRITE_PX * SPRITE_PX * 4;
        index
            .icons
            .into_iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let kind = *KINDS.iter().find(|k| **k == e.kind)?;
                let rgba = SPRITE_PIXELS.get(i * bytes..(i + 1) * bytes)?;
                Some(Sprite { id: e.id, kind, pair: e.pair, area: e.area, boss: e.boss, checklist: e.checklist, rgba })
            })
            .collect()
    })
}

/// The sprites every candidate is compared with: the generic icons (not the boss pairs).
fn generic_sprites() -> &'static [usize] {
    static GENERIC: std::sync::OnceLock<Vec<usize>> = std::sync::OnceLock::new();
    GENERIC.get_or_init(|| (0..sprites().len()).filter(|&i| sprites()[i].pair.is_none()).collect())
}

/// The area being read: its boss pairs are compared too (others are left out: cost, and a
/// boss's icon is only looked for where it stands). Tests keep it per thread.
#[cfg(not(test))]
static ICON_AREA: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
/// Places where a boss pair's icon was seen in this visit, in minimap cells from the
/// player, set by the recorder before each frame: the matcher looks there for both of the
/// pair's sprites (a boss's grey icon is dark, colour alone does not find it).
#[cfg(not(test))]
static ICON_HINTS: std::sync::Mutex<Vec<(String, [f32; 2])>> = std::sync::Mutex::new(Vec::new());
#[cfg(test)]
thread_local! {
    static ICON_AREA_T: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    static ICON_HINTS_T: std::cell::RefCell<Vec<(String, [f32; 2])>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Sets the area the reader is in (the recorder's zone entry); None: no boss pairs.
pub fn set_icon_area(area: Option<&str>) {
    #[cfg(not(test))]
    if let Ok(mut current) = ICON_AREA.lock() {
        *current = area.map(String::from);
    }
    #[cfg(test)]
    ICON_AREA_T.with(|a| *a.borrow_mut() = area.map(String::from));
}

fn icon_area() -> Option<String> {
    #[cfg(not(test))]
    return ICON_AREA.lock().ok().and_then(|a| a.clone());
    #[cfg(test)]
    return ICON_AREA_T.with(|a| a.borrow().clone());
}

/// The current area's boss sprites (both of each pair).
fn area_sprites() -> Vec<usize> {
    let Some(area) = icon_area() else { return vec![] };
    (0..sprites().len()).filter(|&i| sprites()[i].area.as_deref() == Some(area.as_str())).collect()
}

/// `hints`: (pair stem, cells from the player), replacing the last ones.
pub fn set_icon_hints(hints: Vec<(String, [f32; 2])>) {
    #[cfg(not(test))]
    if let Ok(mut current) = ICON_HINTS.lock() {
        *current = hints;
    }
    #[cfg(test)]
    ICON_HINTS_T.with(|h| *h.borrow_mut() = hints);
}

fn icon_hints() -> Vec<(String, [f32; 2])> {
    #[cfg(not(test))]
    return ICON_HINTS.lock().map(|h| h.clone()).unwrap_or_default();
    #[cfg(test)]
    return ICON_HINTS_T.with(|h| h.borrow().clone());
}

/// A sprite drawn at screen scale with its centre `phase` pixels right and down of the
/// template's middle: straight colour and coverage per pixel, and the pixels it covers
/// fully (`core`: alpha at least `CORE`), the only ones compared, so the map lines and
/// ground behind its soft edges do not count.
struct Template {
    w: usize,
    h: usize,
    /// The sprite's centre in template pixels (continuous; pixel `i` spans i..i+1).
    centre: [f64; 2],
    /// Colour and coverage per pixel (drawing a sprite in tests uses them).
    #[cfg_attr(not(test), allow(dead_code))]
    rgb: Vec<[f32; 3]>,
    #[cfg_attr(not(test), allow(dead_code))]
    alpha: Vec<f32>,
    /// The core: template pixel indices, their offsets in an image row of `stride_px`
    /// pixels (set by `for_width`), and their colours.
    core: Vec<u32>,
    offsets: Vec<u32>,
    core_rgb: Vec<[f32; 3]>,
}

/// Alpha a template pixel needs to be compared.
const CORE: f32 = 0.9;

impl Template {
    /// `sprite` drawn `k` screen pixels per stored (32 px) sprite pixel, its centre at
    /// `phase` pixels from the middle of the template. Each screen pixel averages 4x4
    /// bilinear samples of the sprite (premultiplied), which serves any `k` near 1 or below.
    fn new(sprite: &Sprite, k: f64, phase: [f64; 2]) -> Template {
        let n = (SPRITE_PX as f64 * k).ceil() as usize + 3;
        let (w, h) = (n, n);
        let centre = [w as f64 / 2.0 + phase[0], h as f64 / 2.0 + phase[1]];
        let px = |x: i64, y: i64| -> [f32; 4] {
            if x < 0 || y < 0 || x >= SPRITE_PX as i64 || y >= SPRITE_PX as i64 {
                return [0.0; 4];
            }
            let i = (y as usize * SPRITE_PX + x as usize) * 4;
            let a = sprite.rgba[i + 3] as f32 / 255.0;
            [sprite.rgba[i] as f32 * a, sprite.rgba[i + 1] as f32 * a, sprite.rgba[i + 2] as f32 * a, a]
        };
        let sample = |u: f64, v: f64| -> [f32; 4] {
            // Bilinear between pixel centres (pixel i's centre at i + 0.5).
            let (fu, fv) = (u - 0.5, v - 0.5);
            let (x0, y0) = (fu.floor(), fv.floor());
            let (tx, ty) = ((fu - x0) as f32, (fv - y0) as f32);
            let (x0, y0) = (x0 as i64, y0 as i64);
            let (a, b, c, d) = (px(x0, y0), px(x0 + 1, y0), px(x0, y0 + 1), px(x0 + 1, y0 + 1));
            std::array::from_fn(|j| (a[j] * (1.0 - tx) + b[j] * tx) * (1.0 - ty) + (c[j] * (1.0 - tx) + d[j] * tx) * ty)
        };
        const SUB: usize = 4;
        let half = SPRITE_PX as f64 / 2.0;
        let mut rgb = Vec::with_capacity(w * h);
        let mut alpha = Vec::with_capacity(w * h);
        for ty in 0..h {
            for tx in 0..w {
                let mut acc = [0.0f32; 4];
                for sy in 0..SUB {
                    for sx in 0..SUB {
                        let x = tx as f64 + (sx as f64 + 0.5) / SUB as f64;
                        let y = ty as f64 + (sy as f64 + 0.5) / SUB as f64;
                        let s = sample((x - centre[0]) / k + half, (y - centre[1]) / k + half);
                        for j in 0..4 {
                            acc[j] += s[j];
                        }
                    }
                }
                let a = acc[3] / (SUB * SUB) as f32;
                let c = if a > 1e-4 { [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3]] } else { [0.0; 3] };
                rgb.push(c);
                alpha.push(a);
            }
        }
        let core: Vec<u32> = (0..w * h).filter(|&i| alpha[i] >= CORE).map(|i| i as u32).collect();
        let core_rgb = core.iter().map(|&i| rgb[i as usize]).collect();
        Template { w, h, centre, rgb, alpha, core, offsets: Vec::new(), core_rgb }
    }

    /// The template with its core's offsets for images `width` pixels wide.
    fn for_width(mut self, width: usize) -> Template {
        self.offsets = self.core.iter().map(|&i| ((i as usize / self.w) * width + i as usize % self.w) as u32).collect();
        self
    }

    /// How well the image matches the template with its top-left at pixel `at`: the
    /// normalised correlation of the core's colour values (all three channels in one
    /// vector, so colour counts as much as shape), and the distance of the mean colours
    /// (0-1 of 255). None when the template leaves the image.
    fn score(&self, image: &Image, at: [i64; 2]) -> Option<Match> {
        self.score_every(image, at, 1)
    }

    /// `score` over every `step`-th core pixel (a coarse look costs less).
    fn score_every(&self, image: &Image, at: [i64; 2], step: usize) -> Option<Match> {
        if at[0] < 0 || at[1] < 0 || at[0] as usize + self.w > image.width || at[1] as usize + self.h > image.height {
            return None;
        }
        let (x0, y0) = (at[0] as usize, at[1] as usize);
        let base = y0 * image.width + x0;
        let (mut n, mut channel, mut tsum) = (0usize, [0.0f32; 3], [0.0f32; 3]);
        let (mut ss, mut tt, mut st, mut chroma) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for k in (0..self.core.len()).step_by(step) {
            let (off, t) = (self.offsets[k] as usize, &self.core_rgb[k]);
            let p = (base + off) * 3;
            let c = [image.pixels[p] as f32, image.pixels[p + 1] as f32, image.pixels[p + 2] as f32];
            for j in 0..3 {
                channel[j] += c[j];
                tsum[j] += t[j];
                ss += c[j] * c[j];
                tt += t[j] * t[j];
                st += c[j] * t[j];
            }
            chroma += ((c[0] - c[1]) - (t[0] - t[1])).abs() + ((c[2] - c[1]) - (t[2] - t[1])).abs();
            n += 1;
        }
        if n == 0 {
            return None;
        }
        let count = (3 * n) as f32;
        let (s_all, t_all) = (channel.iter().sum::<f32>(), tsum.iter().sum::<f32>());
        let cross = st - s_all * t_all / count;
        let own = (ss - s_all * s_all / count).max(0.0);
        let norm = (tt - t_all * t_all / count).max(0.0);
        let ncc = if own > 0.0 && norm > 0.0 { cross / (own.sqrt() * norm.sqrt()) } else { 0.0 };
        let colour = (0..3).map(|j| ((channel[j] - tsum[j]) / n as f32).abs()).sum::<f32>() / (3.0 * 255.0);
        Some(Match { ncc: ncc as f64, colour: colour as f64, chroma: (chroma / (2.0 * 255.0 * n as f32)) as f64 })
    }
}

/// A template match: normalised correlation (-1..1), the distance of the mean colours
/// (0..1) and the mean per-pixel distance of hue (red and blue against green, 0..1: grey
/// against colour, which the correlation of a dark icon hardly sees).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Match {
    pub ncc: f64,
    pub colour: f64,
    pub chroma: f64,
}

impl Match {
    /// One number to rank by: the correlation, less the colour distances weighted.
    pub fn value(&self) -> f64 {
        self.ncc - COLOUR_WEIGHT * self.colour - CHROMA_WEIGHT * self.chroma
    }
}

/// How much a per-pixel hue distance (0-1) costs in the ranking value.
const CHROMA_WEIGHT: f64 = 2.0;

/// How much a mean-colour distance (0-1) costs in the ranking value.
const COLOUR_WEIGHT: f64 = 3.0;

/// Sub-pixel phases tried: the template's centre at 0 or half a pixel in each direction.
const PHASES: [[f64; 2]; 4] = [[0.0, 0.0], [0.5, 0.0], [0.0, 0.5], [0.5, 0.5]];

/// Templates per (sprite, scale, image width), 4 phases each, for one window size;
/// dropped when the size changes.
struct Templates {
    size: f64,
    built: std::collections::HashMap<(usize, u64, usize), std::sync::Arc<[Template; 4]>>,
}

static TEMPLATES: std::sync::Mutex<Option<Templates>> = std::sync::Mutex::new(None);

/// The templates of sprite `index` at window size `size` (height / 2160) and `scale`
/// (screen px per 64 px sprite pixel at 2160 px), for images `width` pixels wide (each
/// capture size keeps its own: the reader's few capture sizes come in turn).
fn templates(index: usize, size: f64, scale: f64, width: usize) -> std::sync::Arc<[Template; 4]> {
    let mut cache = TEMPLATES.lock().unwrap_or_else(|e| e.into_inner());
    if cache.as_ref().is_none_or(|c| c.size != size) {
        *cache = Some(Templates { size, built: Default::default() });
    }
    let built = &mut cache.as_mut().expect("set above").built;
    built
        .entry((index, scale.to_bits(), width))
        .or_insert_with(|| {
            // Stored sprites are half the sheet's cells.
            let k = 2.0 * scale * size;
            std::sync::Arc::new(PHASES.map(|phase| Template::new(&sprites()[index], k, phase).for_width(width)))
        })
        .clone()
}

/// The best place for sprite `index` within `reach` pixels of `at` (where its centre is
/// guessed, image pixels): every second pixel at phase 0 on half the core pixels, the
/// pixels around the best, then the four half-pixel phases around that. Returns the match
/// and the sprite centre found.
fn best_match(image: &Image, at: [f64; 2], index: usize, size: f64, scale: f64, reach: i64) -> Option<(Match, [f64; 2])> {
    let set = templates(index, size, scale, image.width);
    let start = [(at[0] - set[0].centre[0]).round() as i64, (at[1] - set[0].centre[1]).round() as i64];
    // A sprite whose mean colour is far from what lies under it anywhere near cannot reach
    // `ACCEPT` (the colour alone costs more): nine coarse looks, no search.
    let step = (reach / 2).max(1);
    let mut coarse: Option<(f64, [i64; 2])> = None;
    let mut nearest = f64::INFINITY;
    for dy in [-step, 0, step] {
        for dx in [-step, 0, step] {
            let corner = [start[0] + dx, start[1] + dy];
            if let Some(m) = set[0].score_every(image, corner, 2) {
                nearest = nearest.min(m.colour);
                if coarse.is_none_or(|(v, _)| m.value() > v) {
                    coarse = Some((m.value(), corner));
                }
            }
        }
    }
    if nearest > PRUNE {
        let (_, corner) = coarse?;
        let m = set[0].score(image, corner)?;
        return Some((m, [corner[0] as f64 + set[0].centre[0], corner[1] as f64 + set[0].centre[1]]));
    }
    for dy in (-reach..=reach).step_by(2) {
        for dx in (-reach..=reach).step_by(2) {
            let corner = [start[0] + dx, start[1] + dy];
            if let Some(m) = set[0].score_every(image, corner, 2) {
                if coarse.is_none_or(|(v, _)| m.value() > v) {
                    coarse = Some((m.value(), corner));
                }
            }
        }
    }
    let (_, around) = coarse?;
    let mut best: Option<(Match, [i64; 2], usize)> = None;
    let try_at = |best: &mut Option<(Match, [i64; 2], usize)>, p: usize, corner: [i64; 2]| {
        if let Some(m) = set[p].score(image, corner) {
            if best.is_none_or(|(b, _, _)| m.value() > b.value()) {
                *best = Some((m, corner, p));
            }
        }
    };
    for dy in -1..=1 {
        for dx in -1..=1 {
            try_at(&mut best, 0, [around[0] + dx, around[1] + dy]);
        }
    }
    let (_, corner, _) = best?;
    for p in 1..PHASES.len() {
        for dy in -1..=0 {
            for dx in -1..=0 {
                try_at(&mut best, p, [corner[0] + dx, corner[1] + dy]);
            }
        }
    }
    let (m, c, p) = best?;
    let t = &set[p];
    Some((m, [c[0] as f64 + t.centre[0], c[1] as f64 + t.centre[1]]))
}

/// An icon is named by its sprite when that sprite's ranking value is at least `ACCEPT`
/// and leads the best sprite of any other kind by `LEAD`. Real icons: exits 0.95,
/// checkpoints 0.55-0.66, the waypoint 0.67 (Clearfell); the Bloated Miller's own icon
/// 0.78, the next sprite 0.45 (the Riverbank). The best sprite of a kind that is not there,
/// of all 108, at any place tried on the Clearfell screenshot: 0.64. A sprite drawn onto
/// real pixels scores 0.95-1.0.
const ACCEPT: f64 = 0.72;
const LEAD: f64 = 0.15;
/// A colour-found icon keeps the name of its own kind's best sprite from this value.
const OWN: f64 = 0.5;
/// A sprite whose mean colour differs this much (0-1) from the pixels under it where the
/// search starts is not searched further: at its best place it could not reach `ACCEPT`
/// (3 x 0.15 of colour alone; the real exits differ by 0.001-0.004 at their place, the thin waypoint ring up to 0.1 where its search starts).
const PRUNE: f64 = 0.15;
/// How far around a candidate's guessed centre the sprites are tried (px at 2160 px).
const REACH: f64 = 4.0;
/// … and around a colour group's centre: a grey skull's pale pixels lie in its upper
/// half, 6-7 px above its centre.
const GROUP_REACH: f64 = 8.0;
/// A hinted place is searched this far (px at 2160 px): the player moved since.
const HINT_REACH: f64 = 10.0;
/// The side of a drawn icon (px at 2160 px): 32 stored px at 0.96.
const ICON_SIDE: f64 = 30.0;
/// Candidate places tried per frame, at most (largest colour groups first).
const MAX_CANDIDATES: usize = 16;

/// `among`, each at its kind's scales and best place near `at`, best first.
fn rank(image: &Image, at: [f64; 2], among: &[usize], size: f64, reach: i64) -> Vec<(usize, Match, [f64; 2])> {
    let mut out = Vec::with_capacity(among.len());
    for &i in among {
        // The first scale searches; the others look only around where it found the sprite.
        let scales = sprite_scales(&sprites()[i]);
        let mut best = best_match(image, at, i, size, scales[0], reach);
        for &scale in &scales[1..] {
            let Some((m, c)) = best else { break };
            if m.colour > PRUNE {
                break;
            }
            if let Some(other) = best_match(image, c, i, size, scale, 1) {
                if other.0.value() > m.value() {
                    best = Some(other);
                }
            }
        }
        if let Some((m, c)) = best {
            out.push((i, m, c));
        }
    }
    out.sort_by(|a, b| b.1.value().total_cmp(&a.1.value()));
    out
}

/// The best of a ranking when it is sure: at least `ACCEPT`, `LEAD` ahead of other kinds.
fn verdict(ranked: &[(usize, Match, [f64; 2])]) -> Option<(usize, Match, [f64; 2])> {
    let &(best, m, c) = ranked.first()?;
    let kind = sprites()[best].kind;
    let other = ranked.iter().find(|(i, _, _)| sprites()[*i].kind != kind).map_or(f64::NEG_INFINITY, |(_, m, _)| m.value());
    (m.value() >= ACCEPT && m.value() - other >= LEAD).then_some((best, m, c))
}

/// Names icons by their sprites. Each icon colour found is compared with the sprites:
/// when another kind's sprite is clearly it (an orange skull read as an exit, a portal as
/// a checkpoint) it takes that kind, else it stays and keeps its own best sprite's id. Then
/// places colour does not make icons of are tried: groups of orange, warm and pale blobs
/// (skulls), the spot below a label that belongs to no icon found (a boss's place name), and
/// the hinted places of this visit's boss pairs; those name only kinds colour cannot find
/// (a boss, a dead one, a rare, a portal, …), never an exit, checkpoint or waypoint.
/// `found` are the frame's blobs (samples of `stride` px); `window` the size (height / 2160).
fn identify(image: &Image, icons: &mut Vec<(Icon, [f64; 2])>, found: &[(u8, Blob)], stride: usize, window: f64, centre: [f64; 2], scale: f64) {
    let area = area_sprites();
    let mut among: Vec<usize> = generic_sprites().to_vec();
    among.extend(&area);
    let reach = (REACH * window).round().max(1.0) as i64;
    let group_reach = (GROUP_REACH * window).round().max(1.0) as i64;
    let side = ICON_SIDE * window;
    // Colour-found icons: checked, relabelled when another kind is clearly it.
    for (icon, at) in icons.iter_mut() {
        let ranked = rank(image, *at, &among, window, reach);
        let own = ranked.iter().find(|(i, _, _)| sprites()[*i].kind == icon.kind);
        match verdict(&ranked) {
            Some((i, _, c)) if sprites()[i].kind != icon.kind => {
                let kind = sprites()[i].kind;
                if !matches!(kind, "exit" | "checkpoint" | "waypoint") {
                    (icon.kind, icon.sprite, *at) = (kind, Some(sprites()[i].id.as_str()), c);
                }
            }
            _ => icon.sprite = own.filter(|(_, m, _)| m.value() >= OWN).map(|(i, _, _)| sprites()[*i].id.as_str()),
        }
    }
    let px = |v: f64| (v - 0.5) * stride as f64 + 0.5;
    let taken = |at: [f64; 2], icons: &[(Icon, [f64; 2])]| {
        (at[0] - centre[0]).hypot(at[1] - centre[1]) < 0.5 * side || icons.iter().any(|(_, p)| (p[0] - at[0]).hypot(p[1] - at[1]) < 0.6 * side)
    };
    // (place, sprites, how far to search)
    let mut places: Vec<([f64; 2], Vec<usize>, i64)> = Vec::new();
    // Colour groups: orange that made no exit, warm, pale; joined within half an icon.
    let least = 0.08 * side * side / (stride * stride) as f64;
    let mut groups: Vec<(f64, f64, f64, [usize; 4])> = Vec::new();
    for (class, blob) in found {
        let fits = |a: usize, b: usize| ((b - a + 1) * stride) as f64 <= 1.5 * side;
        if !matches!(*class, 0 | PALE | WARM) || blob.n < 3 || !fits(blob.x0, blob.x1) || !fits(blob.y0, blob.y1) {
            continue;
        }
        let at = [blob.sx / blob.n as f64, blob.sy / blob.n as f64];
        let near = groups.iter_mut().find(|(n, sx, sy, _)| ((sx / n - at[0]).hypot(sy / n - at[1])) * stride as f64 <= 0.5 * side);
        match near {
            Some(g) => {
                (g.0, g.1, g.2) = (g.0 + blob.n as f64, g.1 + blob.sx, g.2 + blob.sy);
                g.3 = [g.3[0].min(blob.x0), g.3[1].min(blob.y0), g.3[2].max(blob.x1), g.3[3].max(blob.y1)];
            }
            None => groups.push((blob.n as f64, blob.sx, blob.sy, [blob.x0, blob.y0, blob.x1, blob.y1])),
        }
    }
    groups.retain(|(n, _, _, b)| *n >= least && ((b[2] - b[0] + 1).max(b[3] - b[1] + 1) * stride) as f64 <= 1.6 * side);
    groups.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (n, sx, sy, _) in groups {
        let at = [px(sx / n), px(sy / n)];
        if !taken(at, icons) {
            places.push((at, among.clone(), group_reach));
        }
    }
    places.truncate(MAX_CANDIDATES);
    // Below a label line no icon owns: a boss's place name (only with boss pairs here).
    let actives: Vec<usize> = area.iter().copied().filter(|&i| sprites()[i].kind == "boss").collect();
    if !actives.is_empty() {
        for at in label_spots(found, stride, window) {
            if !taken(at, icons) {
                places.push((at, among.clone(), (2.0 * REACH * window).round() as i64));
            }
        }
    }
    // This visit's boss pairs, where they were seen.
    for (pair, [dx, dy]) in icon_hints() {
        let at = [centre[0] + dx as f64 * CELL as f64 * scale, centre[1] + dy as f64 * CELL as f64 * scale];
        let mut set: Vec<usize> = (0..sprites().len()).filter(|&i| sprites()[i].pair.as_deref() == Some(pair.as_str())).collect();
        set.extend(generic_sprites());
        places.push((at, set, (HINT_REACH * window).round() as i64));
    }
    #[cfg(test)]
    TRIED.with(|t| t.borrow_mut().extend(icons.iter().map(|(_, at)| (*at, reach)).chain(places.iter().map(|(at, _, r)| (*at, *r)))));
    let mut new: Vec<(Icon, [f64; 2], f64)> = Vec::new();
    for (at, set, reach) in places {
        let ranked = rank(image, at, &set, window, reach);
        let Some((i, m, c)) = verdict(&ranked) else { continue };
        let kind = sprites()[i].kind;
        if matches!(kind, "exit" | "checkpoint" | "waypoint") || taken(c, icons) {
            continue;
        }
        let icon = Icon { kind, sprite: Some(sprites()[i].id.as_str()), ..Icon::default() };
        match new.iter_mut().find(|(_, p, _)| (p[0] - c[0]).hypot(p[1] - c[1]) < 0.6 * side) {
            Some(other) if other.2 >= m.value() => {}
            Some(other) => *other = (icon, c, m.value()),
            None => new.push((icon, c, m.value())),
        }
    }
    icons.extend(new.into_iter().map(|(icon, at, _)| (icon, at)));
    // One object read as two colour icons (a portal was both a checkpoint and a waypoint)
    // is one icon once named.
    let mut kept: Vec<(Icon, [f64; 2])> = Vec::with_capacity(icons.len());
    for (icon, at) in icons.drain(..) {
        let twin = icon.sprite.is_some() && kept.iter().any(|(k, p)| k.sprite == icon.sprite && k.kind == icon.kind && (p[0] - at[0]).hypot(p[1] - at[1]) < 0.6 * side);
        if !twin {
            kept.push((icon, at));
        }
    }
    *icons = kept;
}

/// Draws sprite `index` over `image`, its centre at pixel `at`, `scale` screen px per 64 px
/// sprite pixel (alpha blended, as the game is assumed to draw it): tests only.
#[cfg(test)]
pub fn draw_for_tests(image: &mut Image, index: usize, at: [f64; 2], scale: f64) {
    let k = 2.0 * scale;
    let n = (SPRITE_PX as f64 * k).ceil() as usize + 3;
    let corner = [(at[0] - n as f64 / 2.0).round(), (at[1] - n as f64 / 2.0).round()];
    let phase = [at[0] - corner[0] - n as f64 / 2.0, at[1] - corner[1] - n as f64 / 2.0];
    let t = Template::new(&sprites()[index], k, phase);
    for ty in 0..t.h {
        for tx in 0..t.w {
            let (x, y) = (corner[0] as i64 + tx as i64, corner[1] as i64 + ty as i64);
            if x < 0 || y < 0 || x as usize >= image.width || y as usize >= image.height {
                continue;
            }
            let (i, a) = ((y as usize * image.width + x as usize) * 3, t.alpha[ty * t.w + tx]);
            for j in 0..3 {
                let v = t.rgb[ty * t.w + tx][j] * a + image.pixels[i + j] as f32 * (1.0 - a);
                image.pixels[i + j] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Off: the reader as it was before the sprite matcher (for tests that compare).
    static MATCHING: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// Every place `identify` tried (colour-found icons first), and its reach: for tests.
    static TRIED: std::cell::RefCell<Vec<([f64; 2], i64)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Where an icon would be below each line of label text in `found` (pale letters side by
/// side, about the labels' 17 px font): the label's top lies 25 px above its icon's centre
/// (measured for exits; assumed for named boss icons). Image pixels.
fn label_spots(found: &[(u8, Blob)], stride: usize, window: f64) -> Vec<[f64; 2]> {
    let s = stride as f64;
    let mut letters: Vec<&Blob> = found
        .iter()
        .filter(|(c, b)| {
            let (w, h) = (((b.x1 - b.x0 + 1) as f64) * s, ((b.y1 - b.y0 + 1) as f64) * s);
            *c == PALE && (5.0 * window..=16.0 * window).contains(&h) && w <= 16.0 * window
        })
        .map(|(_, b)| b)
        .collect();
    letters.sort_by_key(|b| b.x0);
    let mut lines: Vec<[f64; 4]> = Vec::new();
    let mut used = vec![false; letters.len()];
    for i in 0..letters.len() {
        if used[i] {
            continue;
        }
        let b = letters[i];
        let mut line = [b.x0 as f64 * s, b.y0 as f64 * s, (b.x1 + 1) as f64 * s, (b.y1 + 1) as f64 * s];
        let mut count = 1;
        for j in i + 1..letters.len() {
            let c = letters[j];
            let mid = |b: &Blob| (b.y0 + b.y1) as f64 * s / 2.0;
            if used[j] || (mid(c) - (line[1] + line[3]) / 2.0).abs() > 5.0 * window {
                continue;
            }
            if c.x0 as f64 * s - line[2] > 9.0 * window {
                break;
            }
            used[j] = true;
            count += 1;
            line = [line[0], line[1].min(c.y0 as f64 * s), line[2].max((c.x1 + 1) as f64 * s), line[3].max((c.y1 + 1) as f64 * s)];
        }
        if count >= 3 && line[2] - line[0] >= 25.0 * window {
            lines.push(line);
        }
    }
    lines.iter().map(|l| [(l[0] + l[2]) / 2.0, l[1] + 25.0 * window]).collect()
}

/// Occupied cells in groups (touching, diagonals too) of at least `MIN_GROUP` that are
/// lines, not filled areas (at most `MAX_INSIDE` of their cells inside).
fn lines(occupied: &[bool], cols: usize, rows: usize) -> Vec<[u16; 2]> {
    let inside = |i: usize| {
        let (x, y) = (i % cols, i / cols);
        x > 0
            && y > 0
            && x + 1 < cols
            && y + 1 < rows
            && (-1..=1).all(|dy: i64| (-1..=1).all(|dx: i64| occupied[(y as i64 + dy) as usize * cols + (x as i64 + dx) as usize]))
    };
    let mut seen = vec![false; occupied.len()];
    let mut walls = Vec::new();
    let mut group = Vec::new();
    for start in 0..occupied.len() {
        if !occupied[start] || seen[start] {
            continue;
        }
        group.clear();
        group.push(start);
        seen[start] = true;
        let mut next = 0;
        while next < group.len() {
            let i = group[next];
            next += 1;
            let (x, y) = ((i % cols) as i64, (i / cols) as i64);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= cols as i64 || ny >= rows as i64 {
                        continue;
                    }
                    let j = ny as usize * cols + nx as usize;
                    if occupied[j] && !seen[j] {
                        seen[j] = true;
                        group.push(j);
                    }
                }
            }
        }
        let filled = group.iter().filter(|&&i| inside(i)).count() as f64 > MAX_INSIDE * group.len() as f64;
        if group.len() >= MIN_GROUP && !filled {
            walls.extend(group.iter().map(|&i| [(i % cols) as u16, (i / cols) as u16]));
        }
    }
    walls.sort_unstable_by_key(|&[x, y]| (y, x));
    walls
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::minimap::parse_ppm;
    use std::collections::HashSet;

    /// A frame of `cols` x `rows` cells with the player at `player`.
    fn frame(cols: usize, rows: usize, player: [f32; 2]) -> Frame {
        Frame { cols, rows, player, capture_px: [cols * CELL, rows * CELL], follows_minimap: false }
    }

    /// Paints `rgb` on the pixels of `image` where `inside` holds.
    fn paint(image: &mut Image, rgb: [u8; 3], inside: impl Fn(usize, usize) -> bool) {
        for y in 0..image.height {
            for x in 0..image.width {
                if inside(x, y) {
                    image.pixels[(y * image.width + x) * 3..][..3].copy_from_slice(&rgb);
                }
            }
        }
    }

    /// `image` drawn `factor` times as large (nearest pixel), as a zoomed big map would be.
    fn zoomed(image: &Image, factor: f64) -> Image {
        let (width, height) = ((image.width as f64 * factor) as usize, (image.height as f64 * factor) as usize);
        let mut pixels = Vec::with_capacity(width * height * 3);
        for y in 0..height {
            for x in 0..width {
                let (sx, sy) = ((x as f64 / factor) as usize, (y as f64 / factor) as usize);
                pixels.extend_from_slice(&image.pixels[(sy * image.width + sx) * 3..][..3]);
            }
        }
        Image { width, height, pixels }
    }

    fn fixture(name: &str) -> Image {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        parse_ppm(&std::fs::read(&path).unwrap()).unwrap()
    }

    /// bigmap-player.ppm: 380x300 px of the big map on Clearfell, the player marker at
    /// about (300.5, 60.7), a frame covering the whole crop.
    fn player_plan() -> Plan {
        Plan { marker: [300.0, 60.0], radius: 16.0, frame: frame(190, 150, [150.0, 30.0]) }
    }

    /// Recorded minimap walls of the same place, relative to the player.
    fn recorded() -> Vec<(i32, i32)> {
        let path = format!("{}/tests/fixtures/clearfell-part14-walls.json", env!("CARGO_MANIFEST_DIR"));
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        json["walls"].as_array().unwrap().iter().map(|w| (w[0].as_i64().unwrap() as i32, w[1].as_i64().unwrap() as i32)).collect()
    }

    /// Share of `of` cells with a `on` cell within one cell, after moving `of` by `shift`.
    fn share(of: &[(i32, i32)], on: &HashSet<(i32, i32)>, shift: (i32, i32)) -> f64 {
        let near = |&(x, y): &(i32, i32)| (-1..=1).any(|dy| (-1..=1).any(|dx| on.contains(&(x + shift.0 + dx, y + shift.1 + dy))));
        of.iter().filter(|c| near(c)).count() as f64 / of.len().max(1) as f64
    }

    fn relative(walls: &[[u16; 2]], player: [f32; 2]) -> Vec<(i32, i32)> {
        walls.iter().map(|&[x, y]| (x as i32 - player[0] as i32, y as i32 - player[1] as i32)).collect()
    }

    #[test]
    fn finds_the_player_marker_at_the_centre() {
        let image = fixture("bigmap-player.ppm");
        let [x, y] = marker_near(&image, [300.0, 60.0], 16.0).unwrap();
        assert!((x - 300.5).abs() < 1.0 && (y - 60.7).abs() < 1.0, "{x},{y}");
        // Nothing orange-cross-like where there is no marker.
        assert!(marker_near(&image, [200.0, 100.0], 16.0).is_none());
        let view = read(&image, &player_plan()).0;
        assert!(view.visible, "{}", view.message);
        assert_eq!(view.player, Some([150.0, 30.0]));
    }

    #[test]
    fn walls_follow_the_map_lines() {
        let image = fixture("bigmap-player.ppm");
        let view = read(&image, &player_plan()).0;
        let found = relative(&view.walls, view.player.unwrap());
        let found_set: HashSet<_> = found.iter().copied().collect();
        let recorded = recorded();
        let recorded_set: HashSet<_> = recorded.iter().copied().collect();
        // Where both show a line they agree to a cell. The rest (checked by eye): the
        // recording has the border-strip bars at the right edge and some stitching drift;
        // the big map has the walk up to the player that this part never recorded.
        let covered = share(&recorded, &found_set, (0, 0));
        let explained = share(&found, &recorded_set, (0, 0));
        println!("{} big-map walls, {} recorded; covered {covered:.3}, explained {explained:.3}", found.len(), recorded.len());
        assert!(covered > 0.85, "{covered}");
        assert!(explained > 0.75, "{explained}");
    }

    #[test]
    fn plain_scene_has_almost_no_walls() {
        let image = fixture("bigmap-scene.ppm");
        let raw = image.pixels.chunks_exact(3).filter(|p| is_wall(p[0], p[1], p[2])).count();
        let frame = frame(128, 80, [64.0, 40.0]);
        let (walls, _) = walls_in(&image, [128.0, 80.0], &frame, SCALE);
        println!("{raw} lavender pixels, {} wall cells", walls.len());
        // The lit ground behind the translucent map has lavender specks; none survive.
        assert!(raw > 100);
        assert!(walls.len() <= 4, "{}", walls.len());
        let plan = Plan { marker: [128.0, 80.0], radius: 16.0, frame };
        assert!(!read(&image, &plan).0.visible);
    }

    #[test]
    fn scale_one_lines_up_with_recorded_minimap_walls() {
        let image = fixture("bigmap-player.ppm");
        let plan = player_plan();
        let centre = marker_near(&image, plan.marker, plan.radius).unwrap();
        let recorded = recorded();
        let score = |scale: f64| {
            let walls = relative(&walls_in(&image, centre, &plan.frame, scale).0, plan.frame.player);
            let set: HashSet<_> = walls.into_iter().collect();
            // Best shift within 3 cells: the scale must be right across the whole crop.
            let mut best = (0.0, (0, 0));
            for dy in -3..=3 {
                for dx in -3..=3 {
                    let s = share(&recorded, &set, (dx, dy));
                    if s > best.0 {
                        best = (s, (dx, dy));
                    }
                }
            }
            best
        };
        let (at_one, shift) = score(SCALE);
        let (smaller, _) = score(0.94);
        let (larger, _) = score(1.06);
        println!("scale 1.00: {at_one:.3} at {shift:?}; 0.94: {smaller:.3}; 1.06: {larger:.3}");
        assert_eq!(shift, (0, 0));
        assert!(at_one > smaller + 0.1 && at_one > larger + 0.1);
    }

    #[test]
    fn resamples_by_the_scale() {
        // A big map drawn twice as large: a line 40 px right of the marker is 20 minimap
        // pixels, 10 cells, from the player.
        let (width, height) = (120, 60);
        let mut pixels = vec![10u8; width * height * 3];
        for y in 0..height {
            for x in [80, 81, 82, 83] {
                pixels[(y * width + x) * 3..][..3].copy_from_slice(&[91, 91, 121]);
            }
        }
        let image = Image { width, height, pixels };
        let frame = frame(40, 20, [10.0, 10.0]);
        let (walls, _) = walls_in(&image, [40.0, 30.0], &frame, 2.0);
        assert!(!walls.is_empty());
        assert!(walls.iter().all(|&[x, _]| x == 20 || x == 21), "{walls:?}");
    }

    /// Icons found in a fixture, as (kind, x, y) rounded to cells.
    fn found_icons(name: &str, centre: [f64; 2]) -> Vec<(&'static str, i32, i32)> {
        let mut found: Vec<_> = icons(&fixture(name), centre, SCALE, 1.0)
            .into_iter()
            .map(|i| (i.kind, i.x.round() as i32, i.y.round() as i32))
            .collect();
        found.sort();
        found
    }

    #[test]
    fn finds_exits_checkpoints_and_the_waypoint() {
        // Around the player: the Grelwood exit and a checkpoint below-left; the player's
        // own cross (also orange) is not an exit.
        let near = found_icons("bigmap-player.ppm", [300.5, 60.7]);
        assert_eq!(near, [("checkpoint", -97, 102), ("exit", -136, 83)]);
        // Further east: the Mud Burrow exit, a checkpoint and the waypoint (positions
        // relative to the crop's corner here).
        let east = found_icons("bigmap-icons.ppm", [0.0, 0.0]);
        let kinds: Vec<_> = east.iter().map(|i| i.0).collect();
        assert_eq!(kinds, ["checkpoint", "exit", "waypoint"]);
        // At 3492,1263 / 3546,1280 / 3381,1257 px of the screenshot, crop at 3340,1215.
        for ((_, x, y), (ex, ey)) in east.iter().zip([(76.0, 24.0), (103.0, 32.5), (20.5, 21.0)]) {
            assert!((*x as f64 - ex).abs() <= 1.5 && (*y as f64 - ey).abs() <= 1.5, "{east:?}");
        }
        // No icons on plain scene or in the corner's zone text.
        assert!(found_icons("bigmap-scene.ppm", [0.0, 0.0]).is_empty());
        assert!(found_icons("corner-bigmap-open.ppm", [0.0, 0.0]).is_empty());
    }

    /// Icons on a screenshot or crop (binary PPM), printed in its pixels; BIGMAP_SIZE is
    /// the game window's height / 2160 (default 1):
    /// `BIGMAP_SHOT=shot.ppm cargo test icons_on_a_screenshot -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn icons_on_a_screenshot() {
        let image = parse_ppm(&std::fs::read(std::env::var("BIGMAP_SHOT").unwrap()).unwrap()).unwrap();
        let size = std::env::var("BIGMAP_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
        for icon in icons(&image, [0.0, 0.0], SCALE, size) {
            println!("{} at {:.0}, {:.0}", icon.kind, icon.x * 2.0, icon.y * 2.0);
        }
    }

    fn sprite_index(id: &str) -> usize {
        sprites().iter().position(|s| s.id == id).unwrap_or_else(|| panic!("no sprite {id}"))
    }

    /// `image` with sprite `id` drawn over it, its centre at pixel `at` (continuous), at
    /// `scale` screen pixels per 64 px sprite pixel: the way the game would draw it (an
    /// assumption: alpha blended, the sheet's half-size level resampled).
    fn draw(image: &mut Image, id: &str, at: [f64; 2], scale: f64) {
        draw_for_tests(image, sprite_index(id), at, scale);
    }

    /// I3: the matcher on the whole Clearfell screenshot (`BIGMAP_FULL`, a PPM): what it
    /// reads, its time, and at every place it tried the best sprite of each kind among all
    /// 108 (so the highest score of anything that is no icon): `BIGMAP_FULL=full.ppm cargo
    /// test --release matcher_on_the_screenshot -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn matcher_on_the_screenshot() {
        let image = parse_ppm(&std::fs::read(std::env::var("BIGMAP_FULL").unwrap()).unwrap()).unwrap();
        let skip = [[0, 1987, 3840, 2160], [0, 0, 730, 864], [3034, 0, 3840, 994], [0, 1685, 1229, 2160], [2765, 1685, 3840, 2160]];
        set_icon_area(Some("G1_2"));
        set_icon_hints(vec![]);
        let read = |stride| icons_in(&image, [1919.0, 1059.0], SCALE, 1.0, stride, &skip, false);
        let _ = read(2);
        TRIED.with(|t| t.borrow_mut().clear());
        let t = std::time::Instant::now();
        let icons = read(2);
        let took = t.elapsed();
        let tried = TRIED.with(|t| t.take());
        println!("{} icons in {took:?}, {} places tried:", icons.len(), tried.len());
        for icon in &icons {
            println!("  {} {:?} at {:.0},{:.0}", icon.kind, icon.sprite, 1919.0 + icon.x * 2.0, 1059.0 + icon.y * 2.0);
        }
        let all: Vec<usize> = (0..sprites().len()).collect();
        let mut worst: Vec<(f64, String)> = vec![];
        for (at, reach) in tried {
            let ranked = rank(&image, at, &all, 1.0, reach);
            let real = icons.iter().find(|i| (1919.0 + i.x as f64 * 2.0 - at[0]).hypot(1059.0 + i.y as f64 * 2.0 - at[1]) < 12.0);
            let best_of = |kind: &str| ranked.iter().find(|(i, _, _)| sprites()[*i].kind == kind).map(|(i, m, _)| (sprites()[*i].id.as_str(), m.value()));
            let line = format!(
                "{:.0},{:.0} ({}): top {} {:.3}; boss {:?} boss-dead {:?} dead {:?} rare {:?}",
                at[0], at[1], real.map_or("none", |i| i.kind), sprites()[ranked[0].0].id, ranked[0].1.value(),
                best_of("boss"), best_of("boss-dead"), best_of("dead"), best_of("rare")
            );
            println!("  {line}");
            for kind in ["boss", "boss-dead", "dead", "rare", "portal", "npc", "quest"] {
                if let Some((id, v)) = best_of(kind) {
                    if real.is_none_or(|i| i.kind != kind) {
                        worst.push((v, format!("{id} at {:.0},{:.0}", at[0], at[1])));
                    }
                }
            }
        }
        worst.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!("highest scores of sprites where there is no such icon: {:?}", &worst[..worst.len().min(8)]);
        // Time per frame, with and without the matcher (best of 5): the whole window every
        // second pixel, and the area around the player (685 x 603 px) at every pixel... the
        // reader's two big-map captures.
        let time = |f: &dyn Fn() -> usize| (0..5).map(|_| { let t = std::time::Instant::now(); f(); t.elapsed() }).min().unwrap();
        let crop = |x0: usize, y0: usize, w: usize, h: usize| Image {
            width: w,
            height: h,
            pixels: (y0..y0 + h).flat_map(|y| image.pixels[(y * image.width + x0) * 3..(y * image.width + x0 + w) * 3].to_vec()).collect(),
        };
        let near = crop(1919 - 342, 1059 - 301, 685, 603);
        let whole_on = time(&|| icons_in(&image, [1919.0, 1059.0], SCALE, 1.0, 2, &skip, false).len());
        let whole_off = without_matching(|| time(&|| icons_in(&image, [1919.0, 1059.0], SCALE, 1.0, 2, &skip, false).len()));
        let near_on = time(&|| icons_in(&near, [342.0, 301.0], SCALE, 1.0, 2, &[], false).len());
        let near_off = without_matching(|| time(&|| icons_in(&near, [342.0, 301.0], SCALE, 1.0, 2, &[], false).len()));
        println!("whole window: {whole_off:?} without the matcher, {whole_on:?} with; around the player: {near_off:?}, {near_on:?}");
        assert_eq!(icons.len(), 6);
    }

    /// The icons read within 15 px of pixel `at` (kind, sprite), the player at `centre`.
    fn read_near(image: &Image, centre: [f64; 2], at: [f64; 2], stride: usize) -> Vec<(&'static str, Option<&'static str>)> {
        icons_in(image, centre, SCALE, 1.0, stride, &[], false)
            .into_iter()
            .filter(|i| (centre[0] + i.x as f64 * 2.0 - at[0]).hypot(centre[1] + i.y as f64 * 2.0 - at[1]) < 15.0)
            .map(|i| (i.kind, i.sprite))
            .collect()
    }

    fn without_matching<R>(f: impl FnOnce() -> R) -> R {
        MATCHING.with(|m| m.set(false));
        let r = f();
        MATCHING.with(|m| m.set(true));
        r
    }

    /// Places on the player fixture: dark ground, on a map line, on lit ground.
    const PLACES: [[f64; 2]; 3] = [[60.0, 60.0], [129.0, 115.0], [250.0, 120.0]];
    const PLAYER: [f64; 2] = [300.5, 60.7];

    #[test]
    fn an_orange_skull_was_an_exit_and_is_a_boss() {
        // I2: drawn onto real map pixels at 0.52 (the scale checkpoints and the waypoint
        // are drawn at), the colour reader took the orange skull for an exit on every second
        // pixel (the whole-window scans); at 0.48 it saw nothing there.
        set_icon_area(None);
        let mut taken = 0;
        for at in PLACES {
            for (scale, stride) in [(0.48, 1), (0.48, 2), (0.52, 1), (0.52, 2)] {
                let mut image = fixture("bigmap-player.ppm");
                draw(&mut image, "UniqueMonsterAlive", at, scale);
                let old = without_matching(|| read_near(&image, PLAYER, at, stride));
                taken += old.iter().filter(|i| i.0 == "exit").count();
                assert_eq!(read_near(&image, PLAYER, at, stride), [("boss", Some("UniqueMonsterAlive"))], "{at:?} at {scale}, every {stride} px; before: {old:?}");
            }
        }
        assert!(taken >= 2, "the old reader's exits: {taken}");
    }

    #[test]
    fn skulls_and_a_portal_are_named_and_no_exit_checkpoint_or_waypoint() {
        set_icon_area(None);
        // A yellow skull is a rare; both grey skulls are one picture: "dead". A town portal
        // (blue) was a checkpoint, from 0.52 also a waypoint.
        let cases = [("RareMonsterAlive", "rare"), ("UniqueMonsterDead", "dead"), ("RareMonsterDead", "dead"), ("Portal", "portal")];
        for (id, kind) in cases {
            for at in PLACES {
                for (scale, stride) in [(0.48, 2), (0.52, 1), (0.52, 2)] {
                    let mut image = fixture("bigmap-player.ppm");
                    draw(&mut image, id, at, scale);
                    let near = read_near(&image, PLAYER, at, stride);
                    assert_eq!(near.len(), 1, "{id} at {at:?}, {scale}, every {stride} px: {near:?}");
                    assert_eq!(near[0].0, kind, "{id} at {at:?}, {scale}, every {stride} px: {near:?}");
                }
            }
        }
        let mut image = fixture("bigmap-player.ppm");
        draw(&mut image, "Portal", PLACES[0], 0.48);
        assert_eq!(without_matching(|| read_near(&image, PLAYER, PLACES[0], 2)), [("checkpoint", None)]);
    }

    #[test]
    fn real_icons_keep_their_kind_and_are_named_by_their_sprite() {
        set_icon_area(Some("G1_2"));
        let named = |name: &str, centre: [f64; 2], stride: usize| -> Vec<(&str, Option<&str>)> {
            let mut found: Vec<_> = icons_in(&fixture(name), centre, SCALE, 1.0, stride, &[], false).into_iter().map(|i| (i.kind, i.sprite)).collect();
            found.sort();
            found
        };
        for stride in [1, 2] {
            assert_eq!(named("bigmap-player.ppm", PLAYER, stride), [("checkpoint", Some("Checkpoint")), ("exit", Some("Entrance"))]);
            assert_eq!(named("bigmap-icons.ppm", [0.0, 0.0], stride), [("checkpoint", Some("Checkpoint")), ("exit", Some("Entrance")), ("waypoint", Some("Waypoint"))]);
        }
        set_icon_area(None);
    }

    #[test]
    fn no_boss_or_monster_on_the_real_fixtures_in_any_area() {
        // Nothing of the kind is in view on the fixtures; every area's boss pairs are tried.
        let areas: std::collections::BTreeSet<&str> = sprites().iter().filter_map(|s| s.area.as_deref()).collect();
        let fixtures = [("bigmap-player.ppm", PLAYER), ("bigmap-icons.ppm", [0.0, 0.0]), ("bigmap-scene.ppm", [128.0, 80.0]), ("corner-bigmap-open.ppm", [0.0, 0.0]), ("labels-grelwood.ppm", [0.0, 0.0]), ("labels-mudburrow.ppm", [0.0, 0.0])];
        let images: Vec<(Image, [f64; 2])> = fixtures.iter().map(|(n, c)| (fixture(n), *c)).collect();
        for area in areas {
            set_icon_area(Some(area));
            for ((name, _), (image, centre)) in fixtures.iter().zip(&images) {
                for stride in [1, 2] {
                    let extra: Vec<_> = icons_in(image, *centre, SCALE, 1.0, stride, &[], false)
                        .into_iter()
                        .filter(|i| !matches!(i.kind, "exit" | "checkpoint" | "waypoint"))
                        .collect();
                    assert!(extra.is_empty(), "{area}, {name}, every {stride} px: {extra:?}");
                }
            }
        }
        set_icon_area(None);
    }

    #[test]
    fn the_first_real_boss_icon_is_the_millers_own() {
        // The Riverbank, the Bloated Miller's first line (a capture the app kept): right by
        // the player's cross his own icon under its place name, "Besieged Encampment", no
        // skull; a checkpoint to the left. The old reader saw only the checkpoint.
        let image = fixture("bigmap-boss-miller.ppm");
        let cross = marker_near(&image, [203.0, 82.0], 16.0).unwrap();
        let read = |stride: usize| -> Vec<(&'static str, Option<&'static str>, i64, i64)> {
            let mut found: Vec<_> = icons_in(&image, cross, SCALE, 1.0, stride, &[], false)
                .into_iter()
                .map(|i| (i.kind, i.sprite, (cross[0] + i.x as f64 * 2.0).round() as i64, (cross[1] + i.y as f64 * 2.0).round() as i64))
                .collect();
            found.sort();
            found
        };
        set_icon_area(Some("G1_1"));
        for stride in [1, 2] {
            let found = read(stride);
            let kinds: Vec<_> = found.iter().map(|i| (i.0, i.1)).collect();
            assert_eq!(kinds, [("boss", Some("MillerActive")), ("checkpoint", Some("Checkpoint"))], "every {stride} px: {found:?}");
            let (x, y) = (found[0].2, found[0].3);
            assert!((x - 229).abs() <= 2 && (y - 86).abs() <= 2, "{found:?}");
            assert_eq!(without_matching(|| read(stride)).len(), 1);
        }
        // How it scores: 0.78 at 0.53; its grey twin next, 0.58; every other sprite lower.
        let all: Vec<usize> = (0..sprites().len()).collect();
        let ranked = rank(&image, [228.5, 86.0], &all, 1.0, 4);
        let value = |id: &str| ranked.iter().find(|(i, _, _)| sprites()[*i].id == id).unwrap().1.value();
        println!("MillerActive {:.3}, next {} {:.3}, MillerInactive {:.3}", value("MillerActive"), sprites()[ranked[1].0].id, ranked[1].1.value(), value("MillerInactive"));
        assert_eq!(sprites()[ranked[0].0].id, "MillerActive");
        assert!(value("MillerActive") >= ACCEPT && value("MillerActive") - ranked[1].1.value() >= LEAD);
        // In another area the Miller's pair is not compared: no boss.
        set_icon_area(Some("G1_2"));
        assert!(read(2).iter().all(|i| i.0 != "boss"));
        set_icon_area(None);
    }

    #[test]
    fn a_boss_icon_is_known_by_name_only_in_its_area() {
        // The Grim Tangle's boss icon (tan and brown: warm pixels point the matcher at it).
        let at = PLACES[0];
        let mut image = fixture("bigmap-player.ppm");
        // Bosses' own icons are drawn at 0.53, as the Miller's is (`sprite_scales`).
        draw(&mut image, "GrimTangleBossActive", at, 0.53);
        set_icon_area(Some("G1_6"));
        assert_eq!(read_near(&image, PLAYER, at, 2), [("boss", Some("GrimTangleBossActive"))]);
        // In another area its sprite is not compared: nothing there is sure.
        set_icon_area(Some("G1_2"));
        assert_eq!(read_near(&image, PLAYER, at, 2), []);
        set_icon_area(None);
        assert_eq!(read_near(&image, PLAYER, at, 2), []);
    }

    #[test]
    fn a_dark_boss_icon_is_found_below_its_label_or_where_it_was_seen() {
        // The Bloated Miller's icon is dark (13% bright pixels): colour does not point at it.
        set_icon_area(Some("G1_1"));
        let at = PLACES[0];
        let mut image = fixture("bigmap-player.ppm");
        draw(&mut image, "MillerActive", at, 0.53);
        assert_eq!(read_near(&image, PLAYER, at, 2), []);
        // With its place name above it (pale letters, the label's top 25 px above the
        // icon's centre; drawn here as 9 letter-sized boxes).
        let mut labelled = image.clone();
        let top = at[1] as usize - 25;
        paint(&mut labelled, [200, 200, 200], |x, y| {
            let x0 = at[0] as usize - 45;
            (top..top + 11).contains(&y) && x >= x0 && x < x0 + 90 && (x - x0) % 10 < 7
        });
        assert_eq!(read_near(&labelled, PLAYER, at, 2), [("boss", Some("MillerActive"))]);
        // Grey (killed), with no label: found where this visit saw the pair, though the
        // player moved 4 px since.
        let mut dead = fixture("bigmap-player.ppm");
        draw(&mut dead, "MillerInactive", at, 0.53);
        let cells = |v: f64, c: f64| ((v - c + 4.0) / 2.0) as f32;
        set_icon_hints(vec![("Miller".into(), [cells(at[0], PLAYER[0]), cells(at[1], PLAYER[1])])]);
        assert_eq!(read_near(&dead, PLAYER, at, 2), [("boss-dead", Some("MillerInactive"))]);
        // A hint where nothing is gives nothing.
        assert_eq!(read_near(&fixture("bigmap-player.ppm"), PLAYER, at, 2), []);
        set_icon_hints(vec![]);
        set_icon_area(None);
    }

    #[test]
    #[ignore]
    fn debug_case() {
        let id = std::env::var("ID").unwrap();
        let scale: f64 = std::env::var("SC").unwrap().parse().unwrap();
        let stride: usize = std::env::var("ST").unwrap().parse().unwrap();
        set_icon_area(std::env::var("AREA").ok().as_deref());
        let at = PLACES[0];
        let mut image = fixture("bigmap-player.ppm");
        draw(&mut image, &id, at, scale);
        if std::env::var("LABEL").is_ok() {
            let top = at[1] as usize - 25;
            paint(&mut image, [200, 200, 200], |x, y| {
                let x0 = at[0] as usize - 45;
                (top..top + 11).contains(&y) && x >= x0 && x < x0 + 90 && (x - x0) % 10 < 7
            });
        }
        TRIED.with(|t| t.borrow_mut().clear());
        let icons = icons_in(&image, PLAYER, SCALE, 1.0, stride, &[], false);
        println!("{icons:?}");
        let all: Vec<usize> = (0..sprites().len()).collect();
        for (p, reach) in TRIED.with(|t| t.take()) {
            let ranked = rank(&image, p, &all, 1.0, reach);
            println!("{p:?} r{reach}: {:?}", ranked.iter().take(4).map(|(i, m, c)| (sprites()[*i].id.as_str(), (m.value() * 1000.0).round() / 1000.0, (m.chroma * 1000.0).round() / 1000.0, c)).collect::<Vec<_>>());
        }
    }

    /// What the reader makes of a real capture (`LIVE_SHOT`, a PPM; `LIVE_AREA` the area id;
    /// `LIVE_AT` "x,y" a place to rank every sprite at): `cargo test live_capture -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_capture() {
        let image = parse_ppm(&std::fs::read(std::env::var("LIVE_SHOT").unwrap()).unwrap()).unwrap();
        set_icon_area(std::env::var("LIVE_AREA").ok().as_deref());
        let centre = marker_near(&image, [image.width as f64 / 2.0, image.height as f64 / 2.0 - 21.0], 60.0).unwrap_or([0.0, 0.0]);
        println!("cross at {centre:?}");
        for stride in [1, 2] {
            let icons = icons_in(&image, centre, SCALE, 1.0, stride, &[], false);
            for i in &icons {
                println!("every {stride}: {} {:?} at {:.0},{:.0}", i.kind, i.sprite, centre[0] + i.x as f64 * 2.0, centre[1] + i.y as f64 * 2.0);
            }
            let old = without_matching(|| icons_in(&image, centre, SCALE, 1.0, stride, &[], false));
            println!("every {stride}, old reader: {:?}", old.iter().map(|i| (i.kind, (centre[0] + i.x as f64 * 2.0).round(), (centre[1] + i.y as f64 * 2.0).round())).collect::<Vec<_>>());
        }
        if let Ok(at) = std::env::var("LIVE_AT") {
            let v: Vec<f64> = at.split(',').map(|v| v.parse().unwrap()).collect();
            let all: Vec<usize> = (0..sprites().len()).collect();
            for (i, m, c) in rank(&image, [v[0], v[1]], &all, 1.0, 8).iter().take(6) {
                println!("  {} {:.3} (ncc {:.3}, colour {:.3}, hue {:.3}) at {:.1},{:.1}", sprites()[*i].id, m.value(), m.ncc, m.colour, m.chroma, c[0], c[1]);
            }
            for scale in [0.40, 0.44, 0.48, 0.52, 0.56] {
                let i = sprite_index("MillerActive");
                if let Some((m, c)) = best_match(&image, [v[0], v[1]], i, 1.0, scale, 8) {
                    println!("  MillerActive at {scale}: {:.3} (ncc {:.3}) at {:.1},{:.1}", m.value(), m.ncc, c[0], c[1]);
                }
            }
        }
    }

    /// I2, measured: what today's colour reader made of icons it was not built for, drawn
    /// at several scales onto real map pixels: `cargo test boss_icons_to_the_old_reader -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn boss_icons_to_the_old_reader() {
        let ids = ["UniqueMonsterAlive", "UniqueMonsterDead", "RareMonsterAlive", "RareMonsterDead", "GrimTangleBossActive", "GrimTangleBossInactive", "MillerActive", "MillerInactive", "GraveyardBossActive", "EntrancePortal", "BossCheckpointActive", "Portal", "NPC", "QuestObject", "QuestItem", "Entrance"];
        // Places on the player fixture: dark ground, on a map line, on lit ground by the player.
        let places = [("dark", [60.0, 60.0]), ("line", [129.0, 115.0]), ("lit", [250.0, 120.0]), ("lit2", [330.0, 200.0])];
        for id in ids {
            let mut row = vec![];
            for scale in [0.40, 0.44, 0.48, 0.52, 0.56, 0.60, 0.72, 0.96] {
                let mut seen = vec![];
                for (place, at) in places {
                    let mut image = fixture("bigmap-player.ppm");
                    draw(&mut image, id, at, scale);
                    for stride in [1, 2] {
                        for icon in icons_in(&image, [300.5, 60.7], SCALE, 1.0, stride, &[], false) {
                            let px = [300.5 + icon.x as f64 * 2.0, 60.7 + icon.y as f64 * 2.0];
                            if (px[0] - at[0]).hypot(px[1] - at[1]) < 20.0 * scale / 0.48 {
                                seen.push(format!("{place}/{stride}:{}", icon.kind));
                            }
                        }
                    }
                }
                row.push(format!("{scale}: [{}]", seen.join(" ")));
            }
            println!("{id}: {}", row.join("  "));
        }
    }

    /// Scores of every sprite at the real icons of the whole Clearfell screenshot, by scale:
    /// `BIGMAP_FULL=full.ppm cargo test sprite_scores_on_the_screenshot -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn sprite_scores_on_the_screenshot() {
        let image = parse_ppm(&std::fs::read(std::env::var("BIGMAP_FULL").unwrap()).unwrap()).unwrap();
        // Where today's reader finds them (every second pixel), in the area it reads (the
        // bottom 8% and the panels left out; the truth: 2 exits, 3 checkpoints, the waypoint).
        let skip = [[0, 1987, 3840, 2160], [0, 0, 730, 864], [3034, 0, 3840, 994], [0, 1685, 1229, 2160], [2765, 1685, 3840, 2160]];
        let real: Vec<(&str, [f64; 2])> = icons_in(&image, [1919.0, 1059.0], SCALE, 1.0, 2, &skip, false)
            .into_iter()
            .map(|icon| (icon.kind, [1919.0 + icon.x as f64 * 2.0, 1059.0 + icon.y as f64 * 2.0]))
            .collect();
        println!("reader: {real:?}");
        for scale in [0.44, 0.46, 0.48, 0.50, 0.52, 0.54, 0.56, 0.58, 0.60] {
            println!("scale {scale}");
            for &(name, at) in &real {
                let mut all: Vec<(f64, &str, Match, [f64; 2])> = (0..sprites().len())
                    .filter_map(|i| best_match(&image, at, i, 1.0, scale, 4).map(|(m, c)| (m.value(), sprites()[i].id.as_str(), m, c)))
                    .collect();
                all.sort_by(|a, b| b.0.total_cmp(&a.0));
                let top: Vec<String> = all.iter().take(4).map(|(v, id, m, c)| format!("{id} {v:.3} (ncc {:.3}, colour {:.3}, chroma {:.3}) at {:.1},{:.1}", m.ncc, m.colour, m.chroma, c[0], c[1])).collect();
                println!("  {name}: {}", top.join(" | "));
            }
        }
    }

    #[test]
    fn orange_next_to_the_cross_does_not_move_it() {
        let image = fixture("bigmap-player.ppm");
        let plan = player_plan();
        let (clean, _) = read(&image, &plan);
        // A 100 px orange smear (fire, an item label) 6 px right of the cross, which spans
        // 295-305 x 56-66 px.
        let mut smeared = fixture("bigmap-player.ppm");
        paint(&mut smeared, [230, 110, 30], |x, y| (312..322).contains(&x) && (55..65).contains(&y));
        let (view, _) = read(&smeared, &plan);
        assert!(view.visible, "{}", view.message);
        assert_eq!(view.walls, clean.walls);
        let [x, y] = marker_near(&smeared, plan.marker, plan.radius).unwrap();
        assert!((x - 300.5).abs() < 1.0 && (y - 60.7).abs() < 1.0, "{x},{y}");
        // An exit icon's size (360 px) at the spot is not the cross either.
        let mut exit = fixture("bigmap-scene.ppm");
        paint(&mut exit, [205, 96, 38], |x, y| (120..138).contains(&x) && (70..90).contains(&y));
        assert!(marker_near(&exit, [128.0, 80.0], 16.0).is_none());
    }

    #[test]
    fn the_cross_never_becomes_or_joins_an_exit() {
        // The cross spans 295-305 x 56-65 px; only the real icons are found, at every
        // pixel (the corner minimap) and every second (the big map).
        let real = [("checkpoint", -97, 102), ("exit", -136, 83)];
        let found = |image: &Image, stride: usize| -> Vec<(&str, i32, i32)> {
            let mut found: Vec<_> = icons_in(image, [300.5, 60.7], SCALE, 1.0, stride, &[], false)
                .into_iter()
                .map(|i| (i.kind, i.x.round() as i32, i.y.round() as i32))
                .collect();
            found.sort();
            found
        };
        let orange = [230, 110, 30];
        let cases: [(&str, Box<dyn Fn(usize, usize) -> bool>); 5] = [
            // 100 px of orange (fire, an ember trail), its centre 12 px right of the cross's.
            ("100 px 12 px away", Box::new(|x, y| (308..318).contains(&x) && (56..66).contains(&y))),
            // The same right under the player's feet, touching the cross.
            ("100 px touching", Box::new(|x, y| (295..305).contains(&x) && (66..76).contains(&y))),
            // Three such patches around the cross: each alone is too small.
            ("three patches", Box::new(|x, y| {
                ((317..327).contains(&x) && (56..66).contains(&y))
                    || ((273..283).contains(&x) && (56..66).contains(&y))
                    || ((295..305).contains(&x) && (78..88).contains(&y))
            })),
            // A line of orange loot text (exit-sized in pixels, 80 x 6 px) by the player.
            ("loot text", Box::new(|x, y| (310..390).contains(&x) && (70..76).contains(&y) && x % 7 != 0)),
            // A lava pool (40 x 40 px) next to the player.
            ("lava", Box::new(|x, y| (320..360).contains(&x) && (80..120).contains(&y))),
        ];
        for (what, inside) in cases {
            let mut image = fixture("bigmap-player.ppm");
            paint(&mut image, orange, inside);
            for stride in [1, 2] {
                assert_eq!(found(&image, stride), real, "{what}, every {stride} px");
            }
        }
        // A real exit stays one with the cross on it.
        let mut image = fixture("bigmap-scene.ppm");
        paint(&mut image, [205, 96, 38], |x, y| (120..138).contains(&x) && (70..96).contains(&y) && (x + y) % 4 != 0);
        paint(&mut image, orange, |x, y| (132..143).contains(&x) && (80..90).contains(&y));
        for stride in [1, 2] {
            let exits: Vec<_> = icons_in(&image, [137.5, 85.0], SCALE, 1.0, stride, &[], false);
            assert_eq!(exits.len(), 1, "{exits:?}");
        }
    }

    #[test]
    fn a_speck_of_orange_on_lines_is_not_the_big_map() {
        // Map lines but no cross, only a 5 px ember where the cross would be.
        let mut image = fixture("bigmap-player.ppm");
        paint(&mut image, [20, 18, 16], |x, y| (290..312).contains(&x) && (50..72).contains(&y));
        paint(&mut image, [230, 110, 30], |x, y| (299..302).contains(&x) && (60..62).contains(&y) && x + y != 360);
        let (view, icons) = read(&image, &player_plan());
        assert!(!view.visible);
        assert!(view.walls.is_empty() && view.player.is_none() && icons.is_empty());
        assert_eq!(view.message, "Big map not read: player marker not found");
    }

    #[test]
    fn filled_areas_are_not_walls() {
        // A 60x60 px area in the line colour (a bluish floor showing through the map).
        let mut image = fixture("bigmap-scene.ppm");
        paint(&mut image, [95, 96, 128], |x, y| (40..100).contains(&x) && (50..110).contains(&y));
        let (walls, _) = walls_in(&image, [128.0, 80.0], &frame(128, 80, [64.0, 40.0]), SCALE);
        assert!(walls.is_empty(), "{} cells", walls.len());
        // Its outline drawn as a line is kept.
        let mut outline = fixture("bigmap-scene.ppm");
        let ring = |x: usize, y: usize| (40..100).contains(&x) && (50..110).contains(&y) && !((43..97).contains(&x) && (53..107).contains(&y));
        paint(&mut outline, [95, 96, 128], ring);
        let (walls, width) = walls_in(&outline, [128.0, 80.0], &frame(128, 80, [64.0, 40.0]), SCALE);
        assert!(walls.len() > 100, "{}", walls.len());
        assert!((width - 3.0).abs() < 0.3, "{width}");
    }

    #[test]
    fn a_frame_mostly_of_lines_is_not_the_big_map() {
        // Stripes 3 px wide every 6 px over the lower two thirds: lines, not filled, but
        // no map looks like that. (Every 4 px they would fill whole cells and be dropped.)
        let mut image = fixture("bigmap-player.ppm");
        paint(&mut image, [95, 96, 128], |x, y| y % 6 < 3 && y > 100 && !(290..312).contains(&x));
        let (view, _) = read(&image, &player_plan());
        assert!(!view.visible);
        assert_eq!(view.message, "Big map not read: too much of the view looks like map lines");
    }

    #[test]
    fn a_zoomed_big_map_is_not_read() {
        let image = fixture("bigmap-player.ppm");
        let plan = player_plan();
        let centre = marker_near(&image, plan.marker, plan.radius).unwrap();
        let (_, width) = walls_in(&image, centre, &plan.frame, SCALE);
        // Drawn 1.5 and 0.8 times as large: lines wider or thinner than at the default zoom.
        let mut widths = vec![width];
        for factor in [1.5, 0.8] {
            let zoomed = zoomed(&image, factor);
            let plan = Plan { marker: plan.marker.map(|v| v * factor), radius: plan.radius * factor.max(1.0), ..plan };
            let centre = marker_near(&zoomed, plan.marker, plan.radius).unwrap();
            let (_, width) = walls_in(&zoomed, centre, &plan.frame, SCALE);
            widths.push(width);
            let (view, _) = read(&zoomed, &Plan { radius: 16.0, ..plan });
            assert!(!view.visible, "at {factor}: {}", view.message);
            assert!(view.message.contains("px wide"), "at {factor}: {}", view.message);
        }
        println!("line width at zoom 1, 1.5, 0.8: {widths:.2?}");
        assert!((LINE_WIDTH[0]..=LINE_WIDTH[1]).contains(&width));
    }

    #[test]
    #[ignore]
    fn line_widths_by_zoom() {
        let image = fixture("bigmap-player.ppm");
        let plan = player_plan();
        for factor in [0.67, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5] {
            let zoomed = zoomed(&image, factor);
            let marker = plan.marker.map(|v| v * factor);
            let Some(centre) = marker_near(&zoomed, marker, 16.0 * factor.max(1.0)) else { println!("{factor}: no marker"); continue };
            let (walls, width) = walls_in(&zoomed, centre, &plan.frame, SCALE);
            println!("{factor}: width {width:.2}, {} walls", walls.len());
        }
        for name in ["bigmap-icons.ppm", "bigmap-scene.ppm"] {
            let image = fixture(name);
            let f = frame(image.width / 2, image.height / 2, [0.0, 0.0]);
            let (walls, width) = walls_in(&image, [0.0, 0.0], &f, SCALE);
            println!("{name}: width {width:.2}, {} walls", walls.len());
        }
    }

    #[test]
    fn the_compass_passes_no_colour_test() {
        // The compass (src/compass.css): needle #ffd27a, fainter (70%) when rough, gray
        // #9a9a9a when unsure, ring 18% gold, label #ffe2a6 or #c4c4c4 with a black
        // shadow; each blended over dark ground, a map line, a checkpoint and an exit.
        let colours = [[255, 210, 122], [154, 154, 154], [255, 226, 166], [196, 196, 196], [20, 20, 20]];
        let grounds = [[0, 0, 0], [16, 15, 18], [60, 55, 50], [95, 96, 128], [100, 170, 220], [205, 96, 38]];
        let mut passed = vec![];
        // Over plain ground nothing of the compass passes at all.
        for colour in colours {
            for alpha in [0.18, 0.5, 0.7, 0.85, 1.0] {
                for ground in [[0, 0, 0], [16, 15, 18], [60, 55, 50]] {
                    let mix: Vec<u8> = (0..3).map(|i| (colour[i] as f64 * alpha + ground[i] as f64 * (1.0 - alpha)).round() as u8).collect();
                    assert!(!is_wall(mix[0], mix[1], mix[2]) && icon_kind(mix[0], mix[1], mix[2]) == NONE, "{colour:?} at {alpha}");
                }
            }
        }
        for colour in colours {
            for ground in grounds {
                for alpha in [0.18, 0.5, 0.7, 0.85, 1.0] {
                    let mix: Vec<u8> = (0..3).map(|i| (colour[i] as f64 * alpha + ground[i] as f64 * (1.0 - alpha)).round() as u8).collect();
                    let (r, g, b) = (mix[0], mix[1], mix[2]);
                    let (gr, gg, gb) = (ground[0], ground[1], ground[2]);
                    // A blend over a line or an icon may keep that line's or icon's class (it
                    // only widens what is there: the needle over an exit, the gray label over
                    // a checkpoint); it must never pass as anything new.
                    let new_wall = is_wall(r, g, b) && !is_wall(gr, gg, gb);
                    let new_icon = icon_kind(r, g, b) != NONE && icon_kind(r, g, b) != icon_kind(gr, gg, gb);
                    // Known: the gray "unsure" label half over a checkpoint's cyan blends
                    // into the waypoint's pale blue. Only label edge pixels over a crystal
                    // (under 80 px) do, too few for a waypoint; the compass is not left out,
                    // as that would lose the icons nearest the player.
                    let known = colour == [196, 196, 196] && ground == [100, 170, 220] && icon_kind(r, g, b) == 2;
                    if (new_wall || new_icon) && !known {
                        passed.push((colour, ground, alpha, mix));
                    }
                }
            }
        }
        assert!(passed.is_empty(), "{passed:?}");
    }

    #[test]
    fn capture_area_covers_the_frame_around_the_marker() {
        let frame = frame(300, 259, [158.0, 142.0]);
        let ([x, y, w, h], marker) = area([3840.0, 2160.0], &frame);
        // Marker at 1919, 1059 px; 316 px to its left, 284 above, plus a margin for the
        // marker search and the learnt player cell (2.5 radii + 2 px).
        assert!((x + marker[0] - 1918.8).abs() < 0.1 && (y + marker[1] - 1059.0).abs() < 0.2);
        assert!((marker[0] - 316.0 - 42.0).abs() < 0.1 && (marker[1] - 284.0 - 42.0).abs() < 0.1);
        assert!((w - (600.0 + 84.0)).abs() < 0.1 && (h - (518.0 + 84.0)).abs() < 0.1);
    }
}
