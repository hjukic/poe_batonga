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
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Icon {
    /// "exit" (orange zone exit), "checkpoint" or "waypoint".
    pub kind: &'static str,
    pub x: f32,
    pub y: f32,
    /// An exit's label: the zone it leads to, as given to `set_label_candidates`, when
    /// read (whole-window frames only).
    pub label: Option<String>,
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
                kinds.push(icon_kind(image.pixels[i], image.pixels[i + 1], image.pixels[i + 2]));
            }
        }
    }
    let size = size / stride as f64;
    let found = blobs(&mut kinds, w, h);
    // The player in samples.
    let player = centre.map(|c| (c - 0.5) / stride as f64 + 0.5);
    let mut icons = Vec::new();
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
            // Sample centre (in samples) -> image pixel centre.
            let at = [(sx / n as f64 - 0.5) * stride as f64 + 0.5, (sy / n as f64 - 0.5) * stride as f64 + 0.5];
            let to_cells = |p: f64, c: f64| ((p - c) / scale / CELL as f64) as f32;
            let label = (read_labels && name == "exit").then(|| labels::read(image, at, window)).flatten();
            icons.push(Icon { kind: name, x: to_cells(at[0], centre[0]), y: to_cells(at[1], centre[1]), label });
        }
    }
    icons
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
