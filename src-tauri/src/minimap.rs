//! Live minimap reading: takes screenshots of the game's minimap corner and pulls out
//! the explored walls and the player marker by colour.
//!
//! Reads only pixels already on screen, through the OS screenshot tool. It never reads
//! game files or memory, and never sends input to the game.

use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

#[path = "bigmap.rs"]
pub mod bigmap;
#[path = "labels.rs"]
pub mod labels;
/// The zone names the current zone's exits lead to, for reading the big map's exit labels
/// (call on zone entry with the zone's `connectionNames`; nothing is read before).
pub use labels::set_label_candidates;
#[cfg(target_os = "windows")]
#[path = "minimap_windows.rs"]
mod windows_capture;
#[cfg(target_os = "windows")]
use windows_capture::{capture, capture_bigmap, capture_window, locate_minimap, Region as CaptureRegion};

/// About eight frames a second: the needle follows the player closely. A capture of
/// the map's area takes about 30 ms, of the whole window 50-60 ms (grim, 3840x2160).
const FRAME_INTERVAL: Duration = Duration::from_millis(120);
/// Where the game is checked again (window moved, workspace switched).
const LOCATE_INTERVAL: Duration = Duration::from_secs(2);

/// The top-right minimap capture, anchored to the game window's right edge and sized by
/// its height as the game sizes the minimap: 600x518 px at 3840x2160 (measured there).
const REGION_W: f64 = 600.0 / 2160.0;
const REGION_H: f64 = 0.24;

/// Native minimap pixels per cell of the wall grid.
pub const CELL: usize = 2;
/// Wall pixels a cell needs, so single stray pixels are ignored.
const CELL_MIN: u32 = 2;
/// How fast the learnt spot of the minimap's cross follows new frames.
const LEARN_RATE: f64 = 0.2;
/// Every this many big-map frames (about once a second) the whole game window is read,
/// for map icons far from the player (they move on screen only as fast as the player).
pub const SCAN_EVERY: u32 = 8;
/// Room left around this app's panel, in logical pixels (its shadow, rounding).
#[cfg(not(target_os = "windows"))]
const PANEL_MARGIN: f64 = 8.0;
/// Panels over the big map, left out of the whole-window icon search, as fractions of
/// the game window (x0, y0, x1, y1; measured at 3840x2160): this app's checklist panel
/// at its default place (first: replaced by where the panel really is, when known), the
/// zone and quest text top right, the life orb and flasks, the mana orb and skills, and
/// the experience bar. Between the orbs the map shows to the bottom: an exit there
/// ("Clearfell Encampment", the owner's Clearfell, 2026-11-12) was left out by a skipped
/// bottom band before.
const PANELS: [[f64; 4]; 5] = [
    [0.0, 0.0, 0.19, 0.40],
    [0.79, 0.0, 1.0, 0.46],
    [0.0, 0.79, 0.31, 1.0],
    [0.70, 0.79, 1.0, 1.0],
    [0.0, 0.975, 1.0, 1.0],
];
#[cfg(target_os = "linux")]
const GRIM_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Serialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MinimapView {
    pub enabled: bool,
    /// A map (minimap or big map) with its player marker was read in the last frame.
    pub visible: bool,
    pub message: String,
    /// Grid size in cells; `walls` lists occupied cells as [column, row].
    pub cols: usize,
    pub rows: usize,
    /// Used by the layout recorder; the overlay draws the stitched layout instead.
    #[serde(skip)]
    pub walls: Vec<[u16; 2]>,
    /// Player marker centre in grid cells.
    pub player: Option<[f32; 2]>,
}

impl MinimapView {
    fn status(enabled: bool, message: &str) -> Self {
        MinimapView { enabled, visible: false, message: message.into(), cols: 0, rows: 0, walls: vec![], player: None }
    }

    /// What the pages get is the same (`walls` is not sent): no news for them.
    fn same_for_pages(&self, other: &MinimapView) -> bool {
        let shown = |v: &MinimapView| (v.enabled, v.visible, v.message.clone(), v.cols, v.rows, v.player);
        shown(self) == shown(other)
    }
}

#[derive(Clone)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    /// RGB, row by row.
    pub pixels: Vec<u8>,
}

/// Lavender lines the game draws along explored walls (about RGB 91,91,121).
fn is_wall(r: u8, g: u8, b: u8) -> bool {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    (r - g).abs() < 9 && b - r > 18 && b - r < 51 && b > 84
}

/// The orange player cross (about RGB 169-255, 63-150, 21).
fn is_marker(r: u8, g: u8, b: u8) -> bool {
    r > 150 && g > 40 && g < 191 && b < 89 && (r as i32 - g as i32) > 60
}

/// Reads a minimap capture: its wall cells, and the player cross near `marker` (pixels, the
/// spot where the minimap draws it) within `radius`. Without the cross this is not a
/// minimap (with the big map open the corner shows big-map lines): not visible.
pub fn analyse(image: &Image, marker: [f64; 2], radius: f64) -> MinimapView {
    let cols = image.width.div_ceil(CELL);
    let rows = image.height.div_ceil(CELL);
    let player = bigmap::marker_near(image, marker, radius).map(|p| p.map(|v| (v / CELL as f64) as f32));
    if player.is_none() {
        return MinimapView { cols, rows, ..MinimapView::status(true, "Minimap not visible") };
    }
    let mut cells = vec![0u32; cols * rows];
    for y in 0..image.height {
        for x in 0..image.width {
            let i = (y * image.width + x) * 3;
            if is_wall(image.pixels[i], image.pixels[i + 1], image.pixels[i + 2]) {
                cells[(y / CELL) * cols + x / CELL] += 1;
            }
        }
    }
    let walls: Vec<[u16; 2]> = cells
        .iter()
        .enumerate()
        .filter(|(_, &n)| n >= CELL_MIN)
        .map(|(i, _)| [(i % cols) as u16, (i / cols) as u16])
        .collect();
    MinimapView { enabled: true, visible: true, message: "Reading minimap".into(), cols, rows, walls, player }
}

/// Parses binary PPM (P6, 8-bit), which is what `grim -t ppm` writes.
#[cfg(any(target_os = "linux", test))]
pub fn parse_ppm(bytes: &[u8]) -> Result<Image, String> {
    let mut fields = Vec::with_capacity(4);
    let mut pos = 0;
    while fields.len() < 4 {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if bytes.get(pos) == Some(&b'#') {
            while pos < bytes.len() && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        let start = pos;
        while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if start == pos {
            return Err("Screenshot header is incomplete".into());
        }
        fields.push(std::str::from_utf8(&bytes[start..pos]).map_err(|e| e.to_string())?);
    }
    pos += 1; // the single whitespace byte before the pixels
    let number = |s: &str| s.parse::<usize>().map_err(|_| format!("Bad screenshot header field {s}"));
    if fields[0] != "P6" || number(fields[3])? != 255 {
        return Err("Screenshot is not an 8-bit P6 PPM".into());
    }
    let (width, height) = (number(fields[1])?, number(fields[2])?);
    let pixels = bytes.get(pos..pos + width * height * 3).ok_or("Screenshot is truncated")?.to_vec();
    Ok(Image { width, height, pixels })
}

/// The big map is handed to the recorder in frames of this many cells (cols, rows), centred
/// on the player: more of the explored map per frame than the minimap's, so the recorder
/// finds its place again after a gap. It takes the new player cell when the frame size changes.
const BIG_FRAME: (usize, usize) = (600, 518);

/// Where the maps are read on screen. On Linux the areas are grim geometry in logical
/// compositor coordinates ("x,y wxh").
#[cfg(not(target_os = "windows"))]
#[derive(Clone, Debug, PartialEq)]
pub struct CaptureRegion {
    minimap: String,
    bigmap: String,
    /// The visible part of the whole game window: grim geometry, its top-left corner in
    /// logical pixels, its size in pixels, and pixels per logical pixel.
    window: String,
    window_origin: [f64; 2],
    window_size: [usize; 2],
    scale: f64,
    /// Where the minimap draws the player, inside the minimap capture, in its pixels.
    minimap_marker: [f64; 2],
    plan: bigmap::Plan,
    scan: Scan,
}

#[cfg(not(target_os = "windows"))]
impl CaptureRegion {
    /// A rectangle in global logical pixels (x, y, width, height), with `PANEL_MARGIN`
    /// around it, as a skip rectangle of the whole-window capture.
    fn skip_of(&self, [x, y, w, h]: [f64; 4]) -> [usize; 4] {
        let px = |v: f64, i: usize| (((v - self.window_origin[i]) * self.scale).max(0.0) as usize).min(self.window_size[i]);
        let m = PANEL_MARGIN;
        [px(x - m, 0), px(y - m, 1), px(x + w + m, 0), px(y + h + m, 1)]
    }
}

/// Reading the whole game window: the big map's frame and every icon on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Scan {
    /// The big map read from the whole-window capture (marker in its pixels).
    pub plan: bigmap::Plan,
    /// Panels left out of the icon search, in the capture's pixels (x0, y0, x1, y1).
    pub skip: Vec<[usize; 4]>,
}

/// The scan of a game window of `window_px` pixels whose captured part starts at
/// `origin` (window pixels) and is `size` pixels large.
fn scan_layout(window_px: [f64; 2], origin: [f64; 2], size: [usize; 2], layout: &BigLayout) -> Scan {
    let px = |f: f64, i: usize| ((f * window_px[i] - origin[i]).max(0.0) as usize).min(size[i]);
    Scan {
        plan: bigmap::Plan {
            marker: [layout.marker[0] - origin[0], layout.marker[1] - origin[1]],
            radius: bigmap::radius(window_px),
            frame: layout.frame,
        },
        skip: PANELS.iter().map(|&[x0, y0, x1, y1]| [px(x0, 0), px(y0, 1), px(x1, 0), px(y1, 1)]).collect(),
    }
}

/// The capture areas for a game window at `game` (x, y, width, height) on a monitor at
/// `monitor`, both in logical coordinates, `scale` pixels per logical unit. Both maps are
/// placed relative to the game window and clipped to it and the monitor: a floating
/// window 10 px left of its monitor otherwise let a lavender window border into the
/// minimap crop, which the recorder took for walls (vertical bars in recordings).
#[cfg(not(target_os = "windows"))]
fn regions(game: [f64; 4], monitor: [f64; 4], scale: f64) -> Result<CaptureRegion, String> {
    let [gx, gy, gw, gh] = game;
    let left = gx.max(monitor[0]);
    let top = gy.max(monitor[1]);
    let right = (gx + gw).min(monitor[0] + monitor[2]);
    let bottom = (gy + gh).min(monitor[1] + monitor[3]);
    if right - left < 100.0 || bottom - top < 100.0 {
        return Err("Game window is not on its monitor".into());
    }
    // Whole logical units, kept inside the visible part of the window.
    let area = |x0: f64, y0: f64, x1: f64, y1: f64| {
        let (x0, y0) = (x0.max(left.ceil()), y0.max(top.ceil()));
        let (x1, y1) = (x1.min(right.floor()), y1.min(bottom.floor()));
        (x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0))
    };
    let geometry = |(x, y, w, h): (f64, f64, f64, f64)| format!("{x},{y} {w}x{h}");
    let minimap = area(
        (gx + gw).round() - (gh * REGION_W).round(),
        gy.round(),
        (gx + gw).round(),
        gy.round() + (gh * REGION_H).round(),
    );
    let window_px = [gw * scale, gh * scale];
    let origin_px = [(minimap.0 - gx) * scale, (minimap.1 - gy) * scale];
    let minimap_px = [(minimap.2 * scale).round() as usize, (minimap.3 * scale).round() as usize];
    let layout = big_layout(window_px, origin_px, minimap_px);
    let [ax, ay, aw, ah] = layout.area;
    let big = area(
        (gx + ax / scale).floor(),
        (gy + ay / scale).floor(),
        (gx + (ax + aw) / scale).ceil(),
        (gy + (ay + ah) / scale).ceil(),
    );
    let marker = [layout.marker[0] - (big.0 - gx) * scale, layout.marker[1] - (big.1 - gy) * scale];
    let window = area(left.ceil(), top.ceil(), right.floor(), bottom.floor());
    let window_origin = [(window.0 - gx) * scale, (window.1 - gy) * scale];
    let window_size = [(window.2 * scale).round() as usize, (window.3 * scale).round() as usize];
    Ok(CaptureRegion {
        minimap: geometry(minimap),
        bigmap: geometry(big),
        window: geometry(window),
        window_origin: [window.0, window.1],
        window_size,
        scale,
        minimap_marker: layout.minimap_marker,
        plan: bigmap::Plan { marker, radius: bigmap::radius(window_px), frame: layout.frame },
        scan: scan_layout(window_px, window_origin, window_size, &layout),
    })
}

/// Where the big map is read in a game window of `window_px` pixels whose minimap
/// capture starts at `minimap_origin` (pixels inside the window) and is `minimap_px` large.
struct BigLayout {
    frame: bigmap::Frame,
    /// The big-map capture inside the window, in pixels: x, y, width, height.
    area: [f64; 4],
    /// The big map's player marker inside the window, in pixels.
    marker: [f64; 2],
    /// Where the minimap draws the player, inside the minimap capture, in pixels.
    minimap_marker: [f64; 2],
}

fn big_layout(window_px: [f64; 2], minimap_origin: [f64; 2], minimap_px: [usize; 2]) -> BigLayout {
    let like_minimap = bigmap::Frame::like_minimap(minimap_px, window_px, minimap_origin);
    let frame = bigmap::Frame::wide(BIG_FRAME.0, BIG_FRAME.1, minimap_px);
    let (area, marker) = bigmap::area(window_px, &frame);
    BigLayout {
        frame,
        area,
        marker: [area[0] + marker[0], area[1] + marker[1]],
        minimap_marker: like_minimap.player.map(|v| v as f64 * CELL as f64),
    }
}

/// Finds the game window and where to read its maps.
#[cfg(target_os = "linux")]
fn locate_minimap() -> Result<CaptureRegion, String> {
    use serde_json::Value;
    let hyprctl = |what: &str| -> Result<Value, String> {
        let reply = crate::layer_shell::hyprland_request(&format!("j/{what}"))
            .ok_or("Hyprland not reachable; only Hyprland is supported on Linux so far")?;
        serde_json::from_str(&reply).map_err(|_| format!("Unexpected reply from Hyprland: {}", reply.trim()))
    };
    let clients = hyprctl("clients")?;
    let game = clients
        .as_array()
        .into_iter()
        .flatten()
        .find(|c| {
            c["class"].as_str().is_some_and(|s| s == "steam_app_2694490")
                || c["title"].as_str().is_some_and(|s| s == "Path of Exile 2")
        })
        .ok_or("Game window not found")?;
    let monitors = hyprctl("monitors")?;
    let monitor = monitors
        .as_array()
        .into_iter()
        .flatten()
        .find(|m| m["id"] == game["monitor"])
        .ok_or("Game monitor not found")?;
    // The screenshot shows whatever is on screen, so the game must be what is shown.
    if monitor["activeWorkspace"]["id"] != game["workspace"]["id"] {
        return Err("Game is not on screen".into());
    }
    let num = |v: &Value| v.as_f64().unwrap_or(0.0);
    let scale = num(&monitor["scale"]).max(0.1);
    let (x, y) = (num(&monitor["x"]), num(&monitor["y"]));
    let (w, h) = (num(&monitor["width"]) / scale, num(&monitor["height"]) / scale);
    let game = [num(&game["at"][0]), num(&game["at"][1]), num(&game["size"][0]), num(&game["size"][1])];
    // Without a usable window size, read the whole monitor as the game (the old behaviour).
    let game = if game[2] >= 100.0 && game[3] >= 100.0 { game } else { [x, y, w, h] };
    regions(game, [x, y, w, h], scale)
}

#[cfg(target_os = "linux")]
fn grim(geometry: &str) -> Result<Image, String> {
    let mut command = std::process::Command::new("grim");
    command.args(["-t", "ppm", "-g", geometry, "-"]);
    parse_ppm(&run(command, GRIM_TIMEOUT).map_err(|e| format!("grim {e}"))?)
}

/// Runs `command` and returns what it wrote, or an error when it fails or takes longer
/// than `timeout` (then it is killed): a hung screenshot tool must not stall the reader.
#[cfg(target_os = "linux")]
fn run(mut command: std::process::Command, timeout: Duration) -> Result<Vec<u8>, String> {
    use std::io::Read;
    use std::process::Stdio;
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("unavailable: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("has no output")?;
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = send.send(stdout.read_to_end(&mut bytes).map(|_| bytes));
    });
    let Ok(read) = receive.recv_timeout(timeout) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("did not answer within {timeout:?}"));
    };
    let status = child.wait().map_err(|e| e.to_string())?;
    let bytes = read.map_err(|e| e.to_string())?;
    if !status.success() {
        let mut error = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut error);
        }
        return Err(format!("failed: {}", error.trim()));
    }
    Ok(bytes)
}

#[cfg(target_os = "linux")]
fn capture(region: &CaptureRegion) -> Result<Image, String> {
    grim(&region.minimap)
}

#[cfg(target_os = "linux")]
fn capture_bigmap(region: &CaptureRegion) -> Result<Image, String> {
    grim(&region.bigmap)
}

#[cfg(target_os = "linux")]
fn capture_window(region: &CaptureRegion) -> Result<Image, String> {
    grim(&region.window)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn locate_minimap() -> Result<CaptureRegion, String> {
    Err("Minimap reading supports Windows and Linux/Hyprland".into())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn capture(_region: &CaptureRegion) -> Result<Image, String> {
    Err("Minimap reading supports Windows and Linux/Hyprland".into())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn capture_bigmap(_region: &CaptureRegion) -> Result<Image, String> {
    Err("Minimap reading supports Windows and Linux/Hyprland".into())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn capture_window(_region: &CaptureRegion) -> Result<Image, String> {
    Err("Minimap reading supports Windows and Linux/Hyprland".into())
}

/// One reading: the frame for the recorder, the capture size it stores (minimap pixels,
/// the same in both views) and the map icons around the player: every icon on screen when
/// `whole` (a whole-window scan), else only those inside the wall frame.
struct Reading {
    view: MinimapView,
    capture_px: [usize; 2],
    icons: Vec<bigmap::Icon>,
    whole: bool,
}

/// What the reader keeps from frame to frame.
#[derive(Default)]
struct Memory {
    /// Where accepted minimap frames had their player cross (minimap-capture pixels,
    /// averaged), and the estimate it was learnt from: a new estimate (the window moved or
    /// was resized) starts over.
    minimap_marker: Option<([f64; 2], [f64; 2])>,
    /// Big-map frames read so far (every `SCAN_EVERY`-th reads the whole window).
    big_frames: u32,
}

impl Memory {
    /// Where the minimap's cross is expected: where it was seen, else the estimate.
    fn minimap_marker(&self, estimate: [f64; 2]) -> [f64; 2] {
        match self.minimap_marker {
            Some((at, from)) if from == estimate => at,
            _ => estimate,
        }
    }

    /// An accepted minimap frame had its cross at `seen`; the spot stays within `limit`
    /// pixels of the estimate.
    fn learn(&mut self, estimate: [f64; 2], seen: [f64; 2], limit: f64) {
        let at = match self.minimap_marker {
            Some((at, from)) if from == estimate => [0, 1].map(|i| at[i] + (seen[i] - at[i]) * LEARN_RATE),
            _ => seen,
        };
        let (dx, dy) = (at[0] - estimate[0], at[1] - estimate[1]);
        let far = dx.hypot(dy) / limit;
        let at = if far > 1.0 { [estimate[0] + dx / far, estimate[1] + dy / far] } else { at };
        self.minimap_marker = Some((at, estimate));
    }
}

/// One frame from whichever map is showing: the corner minimap while it shows its player
/// cross, else the big map (Tab), which hides the minimap; else not visible, with why the
/// big map was not read. `panel` gives where this app's checklist panel is (global
/// logical pixels), asked only for whole-window frames.
fn read(memory: &mut Memory, region: &CaptureRegion, panel: &dyn Fn() -> Option<[f64; 4]>) -> Result<Reading, String> {
    let big = |whole: bool| if whole { capture_window(region) } else { capture_bigmap(region) };
    let scan = || {
        let mut scan = region.scan.clone();
        if let Some(rect) = panel() {
            scan.skip[0] = region.skip_of(rect);
        }
        scan
    };
    choose(memory, capture(region), region.minimap_marker, &region.plan, Some(&scan), big)
}

/// `minimap` is the corner's capture; when it failed (on Windows: covered by a window) the
/// big map is still tried, and the error is returned only if that is not read either.
/// `bigmap(whole)` captures the big map around the player, or the whole game window,
/// which is read every `SCAN_EVERY`-th big-map frame (with `scan`) for icons far away.
fn choose(
    memory: &mut Memory,
    minimap: Result<Image, String>,
    estimate: [f64; 2],
    plan: &bigmap::Plan,
    scan: Option<&dyn Fn() -> Scan>,
    bigmap: impl Fn(bool) -> Result<Image, String>,
) -> Result<Reading, String> {
    let search = plan.radius * bigmap::LEARN_LIMIT;
    if let Ok(image) = &minimap {
        let mut view = analyse(image, memory.minimap_marker(estimate), search);
        if let Some(seen) = view.player {
            memory.learn(estimate, seen.map(|v| v as f64 * CELL as f64), search);
            // Both views report the learnt spot, so the player cell does not step at a switch.
            let at = memory.minimap_marker(estimate);
            view.player = Some(at.map(|v| (v / CELL as f64) as f32));
            // With the exits' names: the minimap draws them as the big map does (read
            // on the owner's capture of the Clearfell Encampment, 0.97 against 0.71).
            let icons = bigmap::icons_in(image, at, 1.0, bigmap::size(plan), 1, &[], true);
            return Ok(Reading { view, capture_px: [image.width, image.height], icons, whole: false });
        }
    }
    let mut frame = plan.frame;
    if frame.follows_minimap {
        frame.player = memory.minimap_marker(estimate).map(|v| (v / CELL as f64) as f32);
    }
    let scan = scan.filter(|_| memory.big_frames % SCAN_EVERY == 0).map(|make| make());
    memory.big_frames = memory.big_frames.wrapping_add(1);
    // The whole window when it is time (else, or if that capture fails, the area around the player).
    let whole = scan.as_ref().and_then(|scan| bigmap(true).ok().map(|image| (image, bigmap::Plan { frame, ..scan.plan }, &scan.skip[..])));
    let captured = match whole {
        Some(whole) => Ok(whole),
        None => bigmap(false).map(|image| (image, bigmap::Plan { frame, ..*plan }, &[][..])),
    };
    let why = match captured {
        Ok((image, plan, skip)) => {
            // Exit labels on whole-window frames only (`skip` is empty otherwise).
            let whole = !skip.is_empty();
            let (view, mut icons) = bigmap::read_with(&image, &plan, skip, whole);
            if view.visible {
                if !whole {
                    // The capture reaches past the wall frame (room for the marker search):
                    // icons there are left to the scans, which see them anyway.
                    let [px, py] = frame.player;
                    let inside = |v: f32, at: f32, n: usize| v >= -at && v <= n as f32 - at;
                    icons.retain(|i| inside(i.x, px, frame.cols) && inside(i.y, py, frame.rows));
                }
                return Ok(Reading { view, capture_px: frame.capture_px, icons, whole });
            }
            view.message
        }
        Err(error) => format!("Big map not read: {error}"),
    };
    let image = minimap?;
    let view = MinimapView { cols: image.width.div_ceil(CELL), rows: image.height.div_ceil(CELL), ..MinimapView::status(true, &format!("Minimap not visible. {why}")) };
    Ok(Reading { view, capture_px: [image.width, image.height], icons: vec![], whole: false })
}

pub struct Minimap {
    pub last: Arc<std::sync::Mutex<MinimapView>>,
}

pub fn init() -> Minimap {
    Minimap {
        last: Arc::new(std::sync::Mutex::new(MinimapView::status(true, "Starting"))),
    }
}

/// Set when something worth a picture happened (a boss spoke); the reader takes it.
static SAMPLE_WANTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Asks the reader to keep a capture of the minimap corner and of the big map's area
/// at its next frame (see `Samples`).
pub fn request_sample() {
    SAMPLE_WANTED.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Keeps a few real captures for later work on the reader (exit labels on the corner
/// minimap, wall colours in zones not seen yet), under `samples/` in the app's data
/// folder: a minimap that shows an exit at most once a minute (8 in all), the big map
/// once per zone (a zone instance keeps its layout: more captures of it while the player
/// stands in town added nothing); only the newest 40 files stay.
struct Samples {
    dir: Option<std::path::PathBuf>,
    minimap_exits: usize,
    last_minimap: Option<Instant>,
    last_big: Option<Instant>,
    /// The zone of the last big-map capture.
    big_area: Option<String>,
    /// Captures with an icon the sprite matcher named (a boss, a dead one, a rare): the
    /// first real ones check what is only drawn so far. Counted over restarts.
    named_icons: usize,
    last_icon: Option<Instant>,
}

impl Samples {
    const MINIMAP_EXITS: usize = 8;
    const NAMED_ICONS: usize = 12;
    const KEEP: usize = 40;
    /// Side of the piece of the whole window kept around a named icon (px).
    const ICON_CROP: usize = 400;

    fn new(dir: Option<std::path::PathBuf>) -> Samples {
        let count = |prefix: &str| {
            dir.as_ref()
                .and_then(|d| std::fs::read_dir(d).ok())
                .map_or(0, |files| files.flatten().filter(|f| f.file_name().to_string_lossy().starts_with(prefix)).count())
        };
        let (minimap_exits, named_icons) = (count("minimap-exit-"), count("icon-"));
        Samples { dir, minimap_exits, last_minimap: None, last_big: None, big_area: None, named_icons, last_icon: None }
    }

    /// A capture of a named icon (at most one a minute): the minimap corner it was read
    /// in, or a square of the whole window around it. Named `icon-<kind>-<sprite>-<view>-<time>.ppm`.
    fn named_icon(&mut self, view: &MinimapView, icons: &[bigmap::Icon], region: &CaptureRegion) {
        let Some(icon) = icons.iter().find(|i| matches!(i.kind, "boss" | "boss-dead" | "dead" | "rare")) else { return };
        if self.named_icons >= Self::NAMED_ICONS || self.last_icon.is_some_and(|t| t.elapsed() < Duration::from_secs(60)) {
            return;
        }
        self.last_icon = Some(Instant::now());
        let name = |view: &str| format!("icon-{}-{}-{view}", icon.kind, icon.sprite.unwrap_or("unknown"));
        let saved = if view.message == "Reading minimap" {
            self.save(&name("minimap"), capture(region))
        } else {
            // The big map draws at the minimap's scale: a cell is CELL pixels from the marker.
            let marker = region.scan.plan.marker;
            let at = [marker[0] + icon.x as f64 * CELL as f64, marker[1] + icon.y as f64 * CELL as f64];
            self.save(&name("bigmap"), capture_window(region).map(|image| crop_around(&image, at, Self::ICON_CROP)))
        };
        if saved {
            self.named_icons += 1;
        }
    }

    /// Called after a frame was read; captures again and saves when a sample is due.
    fn maybe(&mut self, view: &MinimapView, icons: &[bigmap::Icon], region: &CaptureRegion) {
        // A boss spoke: both views as they are now, whatever is being read.
        if self.dir.is_some() && SAMPLE_WANTED.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.save("boss-minimap", capture(region));
            self.save("boss-bigmap", capture_bigmap(region));
        }
        if self.dir.is_none() || !view.visible {
            return;
        }
        self.named_icon(view, icons, region);
        let due = |last: Option<Instant>, secs: u64| last.is_none_or(|t| t.elapsed() >= Duration::from_secs(secs));
        if view.message == "Reading minimap" {
            let exit = icons.iter().any(|icon| icon.kind == "exit");
            if exit && self.minimap_exits < Self::MINIMAP_EXITS && due(self.last_minimap, 60) {
                self.last_minimap = Some(Instant::now());
                if self.save("minimap-exit", capture(region)) {
                    self.minimap_exits += 1;
                }
            }
        } else {
            // Once per zone, half a minute in (the map is drawn by then), not in towns.
            let area = bigmap::icon_area();
            let new_zone = area.is_some() && area != self.big_area && !area.as_deref().is_some_and(|a| a.to_lowercase().contains("town"));
            if new_zone && due(self.last_big, 30) {
                self.last_big = Some(Instant::now());
                self.big_area = area;
                self.save("bigmap", capture_bigmap(region));
            }
        }
    }

    fn save(&self, name: &str, image: Result<Image, String>) -> bool {
        let (Some(dir), Ok(image)) = (&self.dir, image) else { return false };
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut bytes = format!("P6\n{} {}\n255\n", image.width, image.height).into_bytes();
        bytes.extend_from_slice(&image.pixels);
        if std::fs::create_dir_all(dir).and_then(|()| std::fs::write(dir.join(format!("{name}-{stamp}.ppm")), bytes)).is_err() {
            return false;
        }
        // Only the newest stay (the names sort by time within a kind; sort by modification).
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|f| Some((f.metadata().ok()?.modified().ok()?, f.path())))
            .collect();
        files.sort();
        let extra = files.len().saturating_sub(Self::KEEP);
        for (_, path) in files.into_iter().take(extra) {
            let _ = std::fs::remove_file(path);
        }
        true
    }
}

/// The `side` x `side` piece of `image` centred on `at` (clipped to the image).
fn crop_around(image: &Image, at: [f64; 2], side: usize) -> Image {
    let clamp = |v: f64, n: usize| (v.max(0.0) as usize).min(n);
    let half = side as f64 / 2.0;
    let (x0, y0) = (clamp(at[0] - half, image.width), clamp(at[1] - half, image.height));
    let (x1, y1) = (clamp(at[0] + half, image.width), clamp(at[1] + half, image.height));
    let mut pixels = Vec::with_capacity((x1 - x0) * (y1 - y0) * 3);
    for y in y0..y1 {
        pixels.extend_from_slice(&image.pixels[(y * image.width + x0) * 3..(y * image.width + x1) * 3]);
    }
    Image { width: x1 - x0, height: y1 - y0, pixels }
}

pub fn spawn(app: AppHandle, minimap: &Minimap) {
    let last = minimap.last.clone();
    std::thread::spawn(move || {
        let mut region: Option<(Result<CaptureRegion, String>, Instant)> = None;
        let mut memory = Memory::default();
        let mut samples = Samples::new(app.path().app_data_dir().ok().map(|dir| dir.join("samples")));
        loop {
            let started = Instant::now();
            // A panic in one frame (in the reader or the recorder) is printed and the next
            // frame is read as usual.
            let view = crate::guarded("Minimap: a frame", || {
                if region.as_ref().is_none_or(|(_, at)| at.elapsed() > LOCATE_INTERVAL) {
                    region = Some((locate_minimap(), Instant::now()));
                }
                match &region {
                    None => MinimapView::status(true, "Starting"),
                    Some((Err(error), _)) => MinimapView::status(true, error),
                    Some((Ok(area), _)) => match read(&mut memory, area, &|| crate::panel_rect(&app)) {
                        Ok(Reading { view, capture_px, icons, whole }) => {
                            let recorder = app.state::<crate::layout::Recorder>();
                            recorder.on_icons(&icons, whole);
                            if let Some(layout) = recorder.on_frame(&view, capture_px) {
                                let _ = app.emit("layout-state", layout);
                            }
                            samples.maybe(&view, &icons, area);
                            view
                        }
                        Err(error) => {
                            region = None;
                            MinimapView::status(true, &error)
                        }
                    },
                }
            })
            .unwrap_or_else(|| {
                region = None;
                MinimapView::status(true, "A frame failed (see the terminal); reading on")
            });
            let changed = {
                let mut l = crate::relock(&last);
                // The walls change every frame but are not sent.
                let changed = !l.same_for_pages(&view);
                *l = view.clone();
                changed
            };
            if changed {
                let _ = app.emit("minimap-state", view.clone());
            }
            let wait = if view.visible { FRAME_INTERVAL } else { FRAME_INTERVAL * 2 };
            std::thread::sleep(wait.saturating_sub(started.elapsed()));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_with_only_new_walls_is_no_news_for_the_pages() {
        let view = MinimapView { walls: vec![[1, 2]], player: Some([3.0, 4.0]), ..MinimapView::status(true, "Reading big map") };
        assert!(view.same_for_pages(&MinimapView { walls: vec![[5, 6], [7, 8]], ..view.clone() }));
        assert!(!view.same_for_pages(&MinimapView { player: Some([3.5, 4.0]), ..view.clone() }));
        assert!(!view.same_for_pages(&MinimapView { message: "Reading minimap".into(), ..view.clone() }));
    }

    fn image(width: usize, height: usize, paint: impl Fn(usize, usize) -> [u8; 3]) -> Image {
        let mut pixels = Vec::with_capacity(width * height * 3);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&paint(x, y));
            }
        }
        Image { width, height, pixels }
    }

    /// The player cross, about the game's size (61 px), centred on pixel (cx, cy).
    fn cross(x: usize, y: usize, cx: usize, cy: usize) -> bool {
        x.abs_diff(cx) + y.abs_diff(cy) <= 5
    }

    #[test]
    fn finds_walls_and_player_and_ignores_terrain() {
        let frame = image(64, 64, |x, y| match (x, y) {
            (_, 20 | 21) => [91, 91, 121],                     // wall line
            _ if cross(x, y, 31, 41) => [230, 110, 30],         // player cross
            _ if (x + y) % 3 == 0 => [120, 110, 95],           // brownish terrain
            _ => [10, 14, 20],
        });
        let view = analyse(&frame, [31.5, 41.5], 24.0);
        assert!(view.visible);
        assert_eq!((view.cols, view.rows), (32, 32));
        assert_eq!(view.walls.len(), 32);
        assert!(view.walls.iter().all(|[_, row]| *row == 10));
        let [px, py] = view.player.unwrap();
        assert!((px - 15.75).abs() < 0.01 && (py - 20.75).abs() < 0.01, "{px},{py}");
    }

    #[test]
    fn a_named_icon_is_kept_as_a_piece_around_it() {
        let image = image(100, 80, |x, y| [x as u8, y as u8, 0]);
        let piece = crop_around(&image, [90.0, 10.0], 40);
        // Clipped at the right and top edges.
        assert_eq!((piece.width, piece.height), (30, 30));
        assert_eq!(&piece.pixels[..3], &[70, 0, 0]);
    }

    #[test]
    fn the_corner_minimap_names_icons_too() {
        // The corner reader uses the same icon reading (every pixel); the minimap is assumed
        // to draw icons at the big map's size (the table's MinimapIconRadius equals
        // LargemapIconRadius for every icon taken). A synthetic minimap: lines, the cross,
        // an orange skull and a yellow one.
        let mut frame = image(240, 160, |x, y| match (x, y) {
            (_, 20 | 21) => [91, 91, 121],
            _ if cross(x, y, 120, 80) => [230, 110, 30],
            _ => [14, 16, 20],
        });
        let skull = |at: [f64; 2], id: &str| bigmap::sprites().iter().position(|s| s.id == id).map(|i| (at, i)).unwrap();
        for (at, i) in [skull([60.0, 110.0], "UniqueMonsterAlive"), skull([190.0, 110.0], "RareMonsterAlive")] {
            bigmap::draw_for_tests(&mut frame, i, at, 0.53);
        }
        let mut kinds: Vec<_> = bigmap::icons(&frame, [120.5, 80.5], 1.0, 1.0).into_iter().map(|i| (i.kind, i.sprite)).collect();
        kinds.sort();
        assert_eq!(kinds, [("boss", Some("UniqueMonsterAlive")), ("rare", Some("RareMonsterAlive"))]);
    }

    #[test]
    fn dark_frame_is_not_a_minimap() {
        let view = analyse(&image(64, 64, |_, _| [12, 13, 18]), [32.0, 32.0], 24.0);
        assert!(!view.visible);
        assert!(view.walls.is_empty() && view.player.is_none());
    }

    #[test]
    fn the_minimap_cross_is_not_pulled_by_exits_or_fire() {
        // The cross at (60, 50), an exit icon (360 px) 40 px away and an ember beside it:
        // the old whole-frame average moved the player towards the exit.
        let frame = image(120, 100, |x, y| match (x, y) {
            (_, 20 | 21) => [91, 91, 121],
            _ if cross(x, y, 60, 50) => [230, 110, 30],
            (92..=109, 40..=59) => [205, 96, 38],
            (68..=69, 50..=51) => [230, 110, 30],
            _ => [10, 14, 20],
        });
        let view = analyse(&frame, [62.0, 49.0], 24.0);
        let [px, py] = view.player.unwrap();
        assert!((px - 30.25).abs() < 0.01 && (py - 25.25).abs() < 0.01, "{px},{py}");
        // Only the exit icon at the spot: not a minimap.
        let exit_only = image(120, 100, |x, y| if (52..=69).contains(&x) && (40..=59).contains(&y) { [205, 96, 38] } else { [10, 14, 20] });
        assert!(!analyse(&exit_only, [60.5, 50.5], 24.0).visible);
    }

    /// Needs the game on screen: `cargo test live_minimap -- --ignored --nocapture`.
    #[test]
    #[ignore]
    #[cfg(target_os = "linux")]
    fn live_minimap() {
        let region = locate_minimap().unwrap();
        let started = Instant::now();
        let Reading { view, capture_px, icons, .. } = read(&mut Memory::default(), &region, &|| None).unwrap();
        println!(
            "regions {} / {}: {capture_px:?} px in {:?}; {}; {} wall cells of {}x{}; player {:?} (estimate {:?}); icons {icons:?}",
            region.minimap,
            region.bigmap,
            started.elapsed(),
            view.message,
            view.walls.len(),
            view.cols,
            view.rows,
            view.player,
            region.minimap_marker.map(|v| v / CELL as f64),
        );
        let mut art = vec![vec![' '; view.cols]; view.rows];
        for [c, r] in &view.walls {
            art[*r as usize][*c as usize] = '#';
        }
        if let Some([x, y]) = view.player {
            art[y as usize][x as usize] = '@';
        }
        for row in art.iter().step_by(4) {
            println!("{}", row.iter().step_by(2).collect::<String>().trim_end());
        }
    }

    /// Time per frame on the live game, whole-window frames apart (every third):
    /// `cargo test [--release] live_timing -- --ignored --nocapture`.
    #[test]
    #[ignore]
    #[cfg(target_os = "linux")]
    fn live_timing() {
        use crate::layout::Recorder;
        use crate::tracker::LogEvent;
        // LIVE_LABELS="Clearfell Encampment,Mud Burrow,The Grelwood": read exit labels too.
        if let Ok(names) = std::env::var("LIVE_LABELS") {
            set_label_candidates(names.split(',').map(String::from).collect());
        }
        let region = locate_minimap().unwrap();
        let recorder = Recorder::new(None);
        recorder.on_log(&LogEvent::ZoneEntered { area: "live-test".into(), level: 0, seed: None }, false);
        let mut memory = Memory::default();
        let (mut whole, mut near, mut record) = (vec![], vec![], vec![]);
        for frame in 0..21 {
            let t = Instant::now();
            let Reading { view, capture_px, icons, whole: scanned } = read(&mut memory, &region, &|| None).unwrap();
            let took = t.elapsed();
            let t = Instant::now();
            recorder.on_icons(&icons, scanned);
            recorder.on_frame(&view, capture_px);
            record.push(t.elapsed());
            if frame % SCAN_EVERY as usize == 0 {
                whole.push(took);
                println!("whole window: {}, {} walls, icons {:?}", view.message, view.walls.len(), icons.iter().map(|i| (i.kind, i.x.round(), i.y.round(), i.label.as_deref().unwrap_or("-"))).collect::<Vec<_>>());
            } else {
                near.push(took);
            }
        }
        let stat = |v: &[Duration]| {
            let ms: Vec<f64> = v.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
            format!("mean {:.1} ms, max {:.1} ms", ms.iter().sum::<f64>() / ms.len() as f64, ms.iter().cloned().fold(0.0, f64::max))
        };
        println!("regions {} / {} / {}", region.minimap, region.bigmap, region.window);
        println!("read, whole-window frames: {}; other frames: {}; recorder {}", stat(&whole), stat(&near), stat(&record));
    }

    /// Records the live game for LIVE_SECONDS (default 45) into LIVE_OUT (a layouts folder):
    /// `LIVE_OUT=/tmp/x cargo test --release live_record -- --ignored --nocapture`.
    #[test]
    #[ignore]
    #[cfg(target_os = "linux")]
    fn live_record() {
        use crate::layout::Recorder;
        use crate::tracker::LogEvent;
        let out = std::env::var("LIVE_OUT").expect("set LIVE_OUT");
        let seconds: u64 = std::env::var("LIVE_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(45);
        let area = std::env::var("LIVE_AREA").unwrap_or_else(|_| "live-test".into());
        let recorder = Recorder::new(Some(out.into()));
        recorder.on_log(&LogEvent::ZoneEntered { area, level: 0, seed: Some(seconds) }, false);
        let started = Instant::now();
        let mut memory = Memory::default();
        let (mut frames, mut slowest) = (0, Duration::ZERO);
        while started.elapsed() < Duration::from_secs(seconds) {
            let t = Instant::now();
            let Ok(region) = locate_minimap() else { continue };
            let Ok(Reading { view, capture_px, .. }) = read(&mut memory, &region, &|| None) else { continue };
            recorder.on_frame(&view, capture_px);
            frames += 1;
            slowest = slowest.max(t.elapsed());
            std::thread::sleep(FRAME_INTERVAL.saturating_sub(t.elapsed()));
        }
        // A zone change saves and closes the recording.
        recorder.on_log(&LogEvent::ZoneEntered { area: "end".into(), level: 0, seed: None }, false);
        println!("{frames} frames, slowest {slowest:?}");
    }

    fn fixture(name: &str) -> Image {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        parse_ppm(&std::fs::read(&path).unwrap()).unwrap()
    }

    /// The big map on Clearfell (bigmap-player.ppm) and how to read it.
    fn big_map() -> (Image, bigmap::Plan) {
        let frame = bigmap::Frame { cols: 190, rows: 150, player: [150.0, 30.0], capture_px: [380, 300], follows_minimap: false };
        (fixture("bigmap-player.ppm"), bigmap::Plan { marker: [300.0, 60.0], radius: 16.0, frame })
    }

    /// A minimap frame: a wall line and the player cross at (60, 50).
    fn minimap() -> Image {
        image(120, 100, |x, y| match (x, y) {
            (_, 20 | 21) => [91, 91, 121],
            _ if cross(x, y, 60, 50) => [230, 110, 30],
            _ => [10, 14, 20],
        })
    }

    #[test]
    fn minimap_wins_while_it_shows_its_player() {
        let (_, plan) = big_map();
        let reading = choose(&mut Memory::default(), Ok(minimap()), [60.5, 50.5], &plan, None, |_| panic!("big map read")).unwrap();
        assert_eq!((reading.view.message.as_str(), reading.capture_px), ("Reading minimap", [120, 100]));
    }

    #[test]
    fn big_map_is_read_when_the_corner_shows_zone_text() {
        // The corner while the big map is open: zone name and league text, no minimap.
        let corner = fixture("corner-bigmap-open.ppm");
        let (big, plan) = big_map();
        let Reading { view, capture_px, icons, .. } = choose(&mut Memory::default(), Ok(corner), [100.0, 80.0], &plan, None, |_| Ok(big.clone())).unwrap();
        assert_eq!(view.message, "Reading big map");
        // The Grelwood exit and its checkpoint, in cells from the player.
        let icons: Vec<_> = icons.iter().map(|i| (i.kind, i.x.round() as i32, i.y.round() as i32)).collect();
        assert_eq!(icons, [("exit", -136, 83), ("checkpoint", -97, 102)]);
        assert_eq!(capture_px, [380, 300]);
        assert_eq!((view.cols, view.rows, view.player), (190, 150, Some([150.0, 30.0])));
        assert!(view.walls.len() > 500);
    }

    /// Map lines in the corner, as the big map draws them there: no cross.
    fn corner_lines(size: [usize; 2]) -> Image {
        image(size[0], size[1], |x, y| if y % 15 < 2 || x % 23 < 2 { [91, 91, 121] } else { [14, 16, 22] })
    }

    #[test]
    fn corner_lines_without_the_cross_are_not_a_minimap() {
        let (_, plan) = big_map();
        let lines = corner_lines([120, 100]);
        assert!(analyse(&lines, [0.0, 0.0], 0.0).walls.is_empty());
        // The big map not readable (capture failed, or only scene): not visible, with why.
        let reading = choose(&mut Memory::default(), Ok(corner_lines([120, 100])), [60.5, 50.5], &plan, None, |_| Err("grim failed".into())).unwrap();
        assert!(!reading.view.visible && reading.view.walls.is_empty());
        assert_eq!(reading.view.message, "Minimap not visible. Big map not read: grim failed");
        let scene = fixture("bigmap-scene.ppm");
        let reading = choose(&mut Memory::default(), Ok(corner_lines([120, 100])), [60.5, 50.5], &plan, None, |_| Ok(scene.clone())).unwrap();
        assert!(!reading.view.visible);
        assert_eq!(reading.view.message, "Minimap not visible. Big map not read: player marker not found");
        // An exit icon (or fire) in the corner is no cross.
        let mut with_exit = corner_lines([120, 100]);
        for y in 41..61 {
            for x in 52..70 {
                with_exit.pixels[(y * 120 + x) * 3..][..3].copy_from_slice(&[205, 96, 38]);
            }
        }
        let reading = choose(&mut Memory::default(), Ok(with_exit), [60.5, 50.5], &plan, None, |_| Err("grim failed".into())).unwrap();
        assert!(!reading.view.visible && reading.view.player.is_none());
    }

    #[test]
    fn the_cross_is_looked_for_where_it_was_last_seen() {
        let (_, plan) = big_map();
        let mut memory = Memory::default();
        // Expected at (75, 50), seen at (60, 50): learnt.
        let reading = choose(&mut memory, Ok(minimap()), [75.5, 50.5], &plan, None, |_| panic!("big map read")).unwrap();
        assert_eq!(reading.view.player, Some([30.25, 25.25]));
        assert_eq!(memory.minimap_marker([75.5, 50.5]), [60.5, 50.5]);
        // A new estimate (the window moved) starts over; the spot stays near the estimate.
        assert_eq!(memory.minimap_marker([80.0, 50.0]), [80.0, 50.0]);
        memory.learn([0.0, 0.0], [100.0, 0.0], 24.0);
        assert_eq!(memory.minimap_marker([0.0, 0.0]), [24.0, 0.0]);
    }

    #[test]
    fn a_covered_minimap_does_not_stop_the_big_map() {
        let (big, plan) = big_map();
        let covered = || Err::<Image, String>("Minimap covered by another window; capture paused".into());
        let reading = choose(&mut Memory::default(), covered(), [60.5, 50.5], &plan, None, |_| Ok(big.clone())).unwrap();
        assert_eq!(reading.view.message, "Reading big map");
        // Neither: the minimap's own error, so the reader locates the game again.
        let scene = fixture("bigmap-scene.ppm");
        let error = choose(&mut Memory::default(), covered(), [60.5, 50.5], &plan, None, |_| Ok(scene.clone())).err().unwrap();
        assert!(error.starts_with("Minimap covered"), "{error}");
    }

    #[test]
    fn big_map_frames_report_the_minimap_capture_size() {
        // An odd minimap width: both views store the same capture size.
        let frame = bigmap::Frame::like_minimap([601, 518], [3842.0, 2160.0], [3241.0, 0.0]);
        assert_eq!((frame.cols, frame.capture_px), (301, [601, 518]));
        let (big, plan) = big_map();
        let plan = bigmap::Plan { frame: bigmap::Frame { capture_px: [601, 518], ..plan.frame }, ..plan };
        let reading = choose(&mut Memory::default(), Ok(corner_lines([120, 100])), [60.5, 50.5], &plan, None, |_| Ok(big.clone())).unwrap();
        assert_eq!(reading.capture_px, [601, 518]);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_hung_command_is_stopped() {
        let mut sleep = std::process::Command::new("sleep");
        sleep.arg("5");
        let started = Instant::now();
        let error = run(sleep, Duration::from_millis(200)).err().unwrap();
        assert!(error.starts_with("did not answer") && started.elapsed() < Duration::from_secs(2), "{error}");
        let mut echo = std::process::Command::new("printf");
        echo.arg("P6");
        assert_eq!(run(echo, Duration::from_secs(2)).unwrap(), b"P6");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn regions_follow_the_game_window() {
        // The owner's setup: DP-1 at 1920,0 (3840x2160 px, scale 2), the game window
        // floating 10 logical px left of it and 1 down.
        let r = regions([1910.0, 1.0, 1920.0, 1080.0], [1920.0, 0.0, 1920.0, 1080.0], 2.0).unwrap();
        // The minimap ends at the window's right edge, not the monitor's (whose last
        // 10 px show a lavender window border).
        assert_eq!(r.minimap, "3530,1 300x259");
        // Wide big-map frames: 600x518 cells around the player (1200x1036 px plus margins).
        assert_eq!(r.bigmap, "2548,250 643x561");
        assert_eq!((r.plan.frame.cols, r.plan.frame.rows, r.plan.frame.capture_px), (600, 518, [600, 518]));
        assert_eq!(r.plan.frame.player, [300.0, 259.0]);
        // The big-map marker (1919, 1059 window px) inside the big-map capture.
        let [mx, my] = r.plan.marker;
        assert!((mx - (1918.8 - (2548.0 - 1910.0) * 2.0)).abs() < 0.2, "{mx}");
        assert!((my - (1059.0 - (250.0 - 1.0) * 2.0)).abs() < 0.2, "{my}");
        // Where the minimap's cross is expected: cell 158, 142 of the minimap frame.
        let [ex, ey] = r.minimap_marker;
        assert!((ex - 316.0).abs() < 0.2 && (ey - 284.0).abs() < 0.2, "{ex},{ey}");
        // The whole window's visible part, its marker and the panels left out.
        assert_eq!(r.window, "1920,1 1910x1079");
        assert!((r.scan.plan.marker[0] - 1898.8).abs() < 0.2 && (r.scan.plan.marker[1] - 1059.0).abs() < 0.2);
        assert_eq!(r.scan.skip[0], [0, 0, 709, 864]);
        // The panel at 2000,600 logical (300x400) in the whole-window capture's pixels, with
        // the margin: the capture starts at 1920,1 logical, 2 px a logical pixel.
        assert_eq!(r.skip_of([2000.0, 600.0, 300.0, 400.0]), [144, 1182, 776, 2014]);
        // A window filling its monitor reads the same corner as before.
        let r = regions([1920.0, 0.0, 1920.0, 1080.0], [1920.0, 0.0, 1920.0, 1080.0], 2.0).unwrap();
        assert_eq!(r.minimap, "3540,0 300x259");
        assert!(regions([0.0, 0.0, 1920.0, 1080.0], [1920.0, 0.0, 1920.0, 1080.0], 2.0).is_err());
        // A 21:9 window (3440x1440 px): the minimap is sized by height at the right edge,
        // its cross expected at about 3251, 189 px of the window.
        let r = regions([0.0, 0.0, 1720.0, 720.0], [0.0, 0.0, 1720.0, 720.0], 2.0).unwrap();
        assert_eq!(r.minimap, "1520,0 200x173");
        let [ex, ey] = r.minimap_marker;
        assert!((ex + 3040.0 - 3250.6).abs() < 0.5 && (ey - 189.4).abs() < 0.5, "{ex},{ey}");
    }

    /// A small walled area (cells): room outlines and a wiggly wall, about 240 x 200.
    fn world() -> Vec<(i32, i32)> {
        let mut cells = Vec::new();
        let mut rect = |x0: i32, y0: i32, x1: i32, y1: i32| {
            for x in x0..=x1 {
                cells.extend([(x, y0), (x, y1)]);
            }
            for y in y0..=y1 {
                cells.extend([(x0, y), (x1, y)]);
            }
        };
        rect(0, 0, 240, 200);
        rect(30, 25, 70, 50);
        rect(150, 30, 175, 90);
        rect(60, 120, 120, 140);
        rect(190, 150, 215, 170);
        for x in 20..220 {
            cells.push((x, 100 + ((x as f32 / 11.0).sin() * 7.0) as i32));
        }
        cells
    }

    /// Paints wall cells and the player cross as the game would: 2 px a cell at the
    /// minimap's scale, lines `line` px wide, the player (world cell `pos`) on the pixel
    /// `marker`.
    fn screen(size: [usize; 2], pos: (i32, i32), marker: (i32, i32), line: i32) -> Image {
        let [width, height] = size;
        let mut img = image(width, height, |_, _| [14, 16, 22]);
        let mut put = |x: i32, y: i32, rgb: [u8; 3]| {
            if x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height {
                img.pixels[(y as usize * width + x as usize) * 3..][..3].copy_from_slice(&rgb);
            }
        };
        for (wx, wy) in world() {
            let (x, y) = (marker.0 + (wx - pos.0) * 2, marker.1 + (wy - pos.1) * 2);
            for dy in 0..line {
                for dx in 0..line {
                    put(x + dx - (line - 2) / 2, y + dy - (line - 2) / 2, [91, 91, 121]);
                }
            }
        }
        for dy in -5i32..=5 {
            for dx in -5i32..=5 {
                if dx.abs() + dy.abs() <= 5 {
                    put(marker.0 + dx, marker.1 + dy, [230, 110, 30]);
                }
            }
        }
        img
    }

    /// One walk through `world`: the minimap shown, then the big map, then the minimap.
    struct Walk {
        /// Where the reader expects the minimap's cross, and where it is painted.
        estimate: [f64; 2],
        painted: [f64; 2],
        minimap_size: [usize; 2],
        plan: bigmap::Plan,
        big_size: [usize; 2],
        /// Line width in pixels (about 4 at 2160 px height).
        line: i32,
    }

    /// The recorder's overview as JSON, which view each frame was read from, and the walk.
    fn walk(walk: &Walk) -> (serde_json::Value, Vec<&'static str>, Vec<(i32, i32)>) {
        use crate::layout::Recorder;
        use crate::tracker::LogEvent;
        let recorder = Recorder::new(None);
        recorder.on_log(&LogEvent::ZoneEntered { area: "test".into(), level: 1, seed: Some(1) }, true);
        let mini_marker = (walk.painted[0] as i32, walk.painted[1] as i32);
        let big_marker = (walk.plan.marker[0] as i32, walk.plan.marker[1] as i32);
        let mut memory = Memory::default();
        let (mut seen, mut path) = (vec![], vec![]);
        for step in 0..90 {
            let pos = (60 + step * 2, 80 + (step as f32 / 9.0).sin() as i32 * 6 + step / 3);
            path.push(pos);
            let big_open = (30..60).contains(&step);
            let corner = if big_open { corner_lines(walk.minimap_size) } else { screen(walk.minimap_size, pos, mini_marker, walk.line) };
            let big = screen(walk.big_size, pos, big_marker, walk.line);
            let reading = choose(&mut memory, Ok(corner), walk.estimate, &walk.plan, None, |_| Ok(big.clone())).unwrap();
            seen.push(if reading.view.message == "Reading big map" { "big" } else { "minimap" });
            recorder.on_frame(&reading.view, reading.capture_px);
        }
        let json = serde_json::to_value(recorder.view().unwrap()).unwrap();
        (json, seen, path)
    }

    /// One part, every frame placed, and the trail follows the walk step by step: no jump
    /// at the switches between views.
    fn assert_one_track(json: &serde_json::Value, path: &[(i32, i32)]) {
        assert_eq!(json["part"], 1, "a new part was started");
        assert_eq!(json["frames"], json["matched"], "frames left unplaced");
        // The stitched area is under 300 x 260 cells, so the overview is unscaled; every
        // frame moved at least 2 cells, so each left a trail point.
        let at = |v: &serde_json::Value| (v[0].as_f64().unwrap(), v[1].as_f64().unwrap());
        let trail: Vec<(f64, f64)> = json["trail"].as_array().unwrap().iter().map(at).collect();
        assert_eq!(trail.len(), path.len());
        for (i, (point, pos)) in trail.iter().zip(path).enumerate() {
            let (dx, dy) = (point.0 - trail[0].0, point.1 - trail[0].1);
            let (wx, wy) = (pos.0 - path[0].0, pos.1 - path[0].1);
            assert!((dx - wx as f64).abs() <= 1.0 && (dy - wy as f64).abs() <= 1.0, "frame {i}: moved {dx},{dy}, walked {wx},{wy}");
        }
    }

    /// A game window of `window` pixels whose minimap capture is `minimap_size` at
    /// `minimap_origin`: where the reader expects the minimap's cross, how it reads the big
    /// map, and the big-map capture size.
    fn plan_for(window: [f64; 2], minimap_origin: [f64; 2], minimap_size: [usize; 2], frame: Option<bigmap::Frame>) -> ([f64; 2], bigmap::Plan, [usize; 2]) {
        let mut layout = big_layout(window, minimap_origin, minimap_size);
        if let Some(frame) = frame {
            (layout.area, _) = bigmap::area(window, &frame);
            layout.frame = frame;
        }
        let [x, y, w, h] = layout.area;
        let (x0, y0) = (x.floor(), y.floor());
        let size = [((x + w).ceil() - x0) as usize, ((y + h).ceil() - y0) as usize];
        let marker = [layout.marker[0] - x0, layout.marker[1] - y0];
        (layout.minimap_marker, bigmap::Plan { marker, radius: bigmap::radius(window), frame: layout.frame }, size)
    }

    /// The owner's window: 3840x2160 px, minimap capture 600x518 px at 3240,0; big-map
    /// frames as `frame` (default: `BIG_FRAME`).
    fn owner(painted_off: [f64; 2], frame: Option<bigmap::Frame>) -> Walk {
        let (estimate, plan, big_size) = plan_for([3840.0, 2160.0], [3240.0, 0.0], [600, 518], frame);
        let painted = [estimate[0] + painted_off[0], estimate[1] + painted_off[1]];
        Walk { estimate, painted, minimap_size: [600, 518], plan, big_size, line: 4 }
    }

    /// Big-map frames the size of the minimap's, the player on the minimap's cell.
    fn minimap_sized() -> bigmap::Frame {
        bigmap::Frame::like_minimap([600, 518], [3840.0, 2160.0], [3240.0, 0.0])
    }

    #[test]
    fn one_track_across_minimap_and_big_map() {
        // Wide big-map frames (600x518 cells around the player): the recorder takes the
        // new player cell at each switch.
        let walk_spec = owner([0.0, 0.0], None);
        assert_eq!((walk_spec.plan.frame.cols, walk_spec.plan.frame.rows, walk_spec.plan.frame.capture_px), (600, 518, [600, 518]));
        assert_eq!(walk_spec.big_size, [1285, 1121]);
        let (json, seen, path) = walk(&walk_spec);
        assert_eq!(seen.iter().filter(|&&v| v == "big").count(), 30);
        assert_eq!((seen[29], seen[30], seen[59], seen[60]), ("minimap", "big", "big", "minimap"));
        assert_one_track(&json, &path);
    }

    #[test]
    fn one_track_with_minimap_sized_big_map_frames() {
        let walk_spec = owner([0.0, 0.0], Some(minimap_sized()));
        assert_eq!(walk_spec.big_size, [685, 603]);
        let (json, _, path) = walk(&walk_spec);
        assert_one_track(&json, &path);
    }

    #[test]
    fn one_track_when_the_minimap_cross_is_off_the_estimate() {
        // The cross 12 px right and 10 px up of where it was expected (6 and 5 cells, more
        // than the recorder's 4-cell rule): the reader learns its spot, and big-map frames
        // put the player there too. (Within 4 cells there is no step either way: each view
        // places its walls around the player cell it reports.)
        let (json, seen, path) = walk(&owner([12.0, -10.0], Some(minimap_sized())));
        assert_eq!(seen.iter().filter(|&&v| v == "big").count(), 30);
        assert_one_track(&json, &path);
    }

    #[test]
    fn one_track_in_a_21_9_window() {
        // 3440x1440 px: minimap capture 400x346 px at the right edge.
        let (estimate, plan, big_size) = plan_for([3440.0, 1440.0], [3040.0, 0.0], [400, 346], None);
        let painted = [estimate[0] + 2.0, estimate[1] + 2.0];
        let (json, seen, path) = walk(&Walk { estimate, painted, minimap_size: [400, 346], plan, big_size, line: 3 });
        assert_eq!(seen.iter().filter(|&&v| v == "big").count(), 30);
        assert_one_track(&json, &path);
    }

    /// The owner's whole game window while the big map is open: the two real big-map crops
    /// pasted where they were on screen (relative to the player marker), orange text in
    /// this app's panel and the bottom bar, the rest dark.
    fn whole_window(scan: &Scan) -> Image {
        let mut canvas = image(3840, 2160, |_, _| [16, 15, 18]);
        let mut paste = |crop: &Image, at: (usize, usize)| {
            for y in 0..crop.height {
                let (from, to) = ((y * crop.width) * 3, ((at.1 + y) * 3840 + at.0) * 3);
                canvas.pixels[to..to + crop.width * 3].copy_from_slice(&crop.pixels[from..from + crop.width * 3]);
            }
        };
        // bigmap-player.ppm has the marker at (300.5, 60.7); bigmap-icons.ppm was cut
        // 1741, 214 px right of and below it on the same screenshot.
        let [mx, my] = scan.plan.marker;
        let at = ((mx - 300.5).round() as usize, (my - 60.7).round() as usize);
        paste(&fixture("bigmap-player.ppm"), at);
        paste(&fixture("bigmap-icons.ppm"), (at.0 + 1741, at.1 + 214));
        // Orange words (a boss name in the checklist panel, the bottom bar's numbers).
        for (x0, y0) in [(200, 300), (1700, 2100)] {
            for letter in 0..8 {
                for y in y0..y0 + 14 {
                    for x in x0 + letter * 10..x0 + letter * 10 + 7 {
                        canvas.pixels[(y * 3840 + x) * 3..][..3].copy_from_slice(&[206, 120, 50]);
                    }
                }
            }
        }
        canvas
    }

    #[test]
    fn the_whole_window_is_read_for_icons_every_third_frame() {
        let (estimate, plan, _) = plan_for([3840.0, 2160.0], [3240.0, 0.0], [600, 518], None);
        let window = [3840.0, 2160.0];
        let layout = big_layout(window, [3240.0, 0.0], [600, 518]);
        let scan = scan_layout(window, [0.0, 0.0], [3840, 2160], &layout);
        let whole = whole_window(&scan);
        let near = fixture("bigmap-player.ppm");
        let near_plan = bigmap::Plan { marker: [300.0, 60.0], ..plan };
        let mut memory = Memory::default();
        let dark = || Ok(image(600, 518, |_, _| [14, 16, 22]));
        let cells = |icons: &[bigmap::Icon]| -> Vec<(&str, i32, i32)> {
            icons.iter().map(|i| (i.kind, i.x.round() as i32, i.y.round() as i32)).collect()
        };
        // Frame 1: the whole window. Every icon on the screenshot, in cells from the player:
        // the Grelwood exit and its checkpoint; 750-820 cells right, the waypoint, a
        // checkpoint and the Mud Burrow exit. Nothing from the panels.
        let reading = choose(&mut memory, dark(), estimate, &near_plan, Some(&|| scan.clone()), |whole_window| {
            if whole_window { Ok(whole.clone()) } else { panic!("the area around the player was read") }
        })
        .unwrap();
        assert_eq!(reading.view.message, "Reading big map");
        assert!(reading.whole);
        let found = cells(&reading.icons);
        println!("{found:?}");
        let expected = [("exit", -136, 83), ("checkpoint", -97, 102), ("waypoint", 741, 99), ("checkpoint", 796, 100), ("exit", 823, 110)];
        assert_eq!(found.len(), expected.len(), "{found:?}");
        for (kind, x, y) in expected {
            assert!(found.iter().any(|&(k, fx, fy)| k == kind && (fx - x).abs() <= 1 && (fy - y).abs() <= 1), "{kind} at {x},{y}: {found:?}");
        }
        // Frames 2 and 3: the area around the player only.
        for _ in 0..2 {
            let reading = choose(&mut memory, dark(), estimate, &near_plan, Some(&|| scan.clone()), |whole_window| {
                assert!(!whole_window);
                Ok(near.clone())
            })
            .unwrap();
            assert!(!reading.whole);
            assert_eq!(cells(&reading.icons), [("exit", -136, 83), ("checkpoint", -97, 102)]);
        }
        // A failed whole-window capture falls back to the area around the player.
        let reading = choose(&mut memory, dark(), estimate, &near_plan, Some(&|| scan.clone()), |whole_window| {
            if whole_window { Err("covered".into()) } else { Ok(near.clone()) }
        })
        .unwrap();
        assert_eq!(reading.view.message, "Reading big map");
    }

    #[test]
    fn icons_past_the_wall_frame_are_left_to_the_scans() {
        // A wall frame of 190x150 cells with the player at cell 130, 30: the Grelwood exit
        // (-136, 83 cells) lies in the capture but past the frame's left edge, its
        // checkpoint (-97, 102) inside. Frames between scans list only icons inside the
        // frame, so the recorder never takes one for a scan.
        let frame = bigmap::Frame { player: [130.0, 30.0], ..bigmap::Frame::wide(190, 150, [600, 518]) };
        let plan = bigmap::Plan { marker: [300.0, 60.0], radius: 16.0, frame };
        let near = fixture("bigmap-player.ppm");
        let mut memory = Memory { big_frames: 1, ..Memory::default() };
        let reading = choose(&mut memory, Err("big map open".into()), [316.0, 284.0], &plan, None, |_| Ok(near.clone())).unwrap();
        assert!(reading.view.visible, "{}", reading.view.message);
        assert!(!reading.whole);
        let found: Vec<_> = reading.icons.iter().map(|i| (i.kind, i.x.round() as i32, i.y.round() as i32)).collect();
        assert_eq!(found, [("checkpoint", -97, 102)]);
    }

    #[test]
    fn the_panel_is_left_out_where_it_really_is() {
        // The checklist panel dragged over the map: orange in it of an exit icon's size
        // and shape (an icon, a bold heading) reads as exits unless the panel's rectangle
        // (from `panel_rect`) is skipped. (Text letters alone are too small: exit blobs
        // are not joined.)
        let window = [3840.0, 2160.0];
        let layout = big_layout(window, [3240.0, 0.0], [600, 518]);
        let scan = scan_layout(window, [0.0, 0.0], [3840, 2160], &layout);
        let mut whole = whole_window(&scan);
        // Moved: no panel at its default place (top left), its words over the map instead.
        for y in 280..340 {
            for x in 180..300 {
                whole.pixels[(y * 3840 + x) * 3..][..3].copy_from_slice(&[16, 15, 18]);
            }
        }
        // (Drawn as the exit sprite: a plain orange block no longer reads as an exit.)
        let entrance = bigmap::sprites().iter().position(|s| s.id == "Entrance").unwrap();
        for blob in 0..3 {
            bigmap::draw_for_tests(&mut whole, entrance, [2409.0 + blob as f64 * 60.0, 1513.0], 0.48);
        }
        let read_with_skip = |skip0: [usize; 4]| {
            let mut scan = scan.clone();
            scan.skip[0] = skip0;
            let plan = scan.plan;
            let reading = choose(&mut Memory::default(), Ok(image(600, 518, |_, _| [14, 16, 22])), [316.0, 284.0], &plan, Some(&|| scan.clone()), |_| Ok(whole.clone())).unwrap();
            reading.icons.iter().filter(|i| i.kind == "exit").count()
        };
        // Fixed place only: the moved panel's three blobs read as more "exits".
        assert_eq!(read_with_skip(scan.skip[0]), 5);
        // Its real place (a 640x200 px panel there) left out: the two real exits.
        assert_eq!(read_with_skip([2380, 1480, 3020, 1680]), 2);
    }

    /// A larger walled area for gap tests: 900 x 700 cells of rooms of many sizes.
    fn rooms() -> Vec<(i32, i32)> {
        let mut cells = Vec::new();
        let mut seed = 7u32;
        let mut next = |n: u32| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) % n
        };
        let mut rect = |x0: i32, y0: i32, x1: i32, y1: i32| {
            for x in x0..=x1 {
                cells.extend([(x, y0), (x, y1)]);
            }
            for y in y0..=y1 {
                cells.extend([(x0, y), (x1, y)]);
            }
        };
        rect(0, 0, 900, 700);
        for _ in 0..110 {
            let (x, y) = (next(860) as i32, next(660) as i32);
            rect(x, y, x + 6 + next(40) as i32, y + 6 + next(40) as i32);
        }
        cells.sort_unstable();
        cells.dedup();
        cells
    }

    /// Walks along `path` showing the recorder frames of `cols` x `rows` cells with the
    /// player on `player`: the walls within the frame that the game has revealed (within
    /// 120 cells of anywhere the player has been, also where no frame was read). Frames
    /// are read only where `read` holds. Returns the overview and, per read frame, whether
    /// it was placed and where the recorder put the player.
    fn fog_walk(path: &[(i32, i32)], read: impl Fn(usize) -> bool, cols: usize, rows: usize, player: (i32, i32)) -> (serde_json::Value, Vec<(usize, bool, bool)>) {
        fog_walk_paced(path, read, cols, rows, player, Duration::ZERO)
    }

    /// `fog_walk` at `pace` a frame (the recorder joins pieces every 3 s of real time).
    fn fog_walk_paced(path: &[(i32, i32)], read: impl Fn(usize) -> bool, cols: usize, rows: usize, player: (i32, i32), pace: Duration) -> (serde_json::Value, Vec<(usize, bool, bool)>) {
        use crate::layout::Recorder;
        use crate::tracker::LogEvent;
        let world = rooms();
        let mut revealed = vec![false; world.len()];
        let recorder = Recorder::new(None);
        recorder.on_log(&LogEvent::ZoneEntered { area: "test".into(), level: 1, seed: Some(1) }, true);
        let mut log = vec![];
        let mut last = serde_json::Value::Null;
        for (step, &pos) in path.iter().enumerate() {
            for (i, &(x, y)) in world.iter().enumerate() {
                if (x - pos.0).pow(2) + (y - pos.1).pow(2) <= 120 * 120 {
                    revealed[i] = true;
                }
            }
            if !read(step) {
                continue;
            }
            let walls = world
                .iter()
                .zip(&revealed)
                .filter(|(_, &r)| r)
                .map(|(&(x, y), _)| (x - pos.0 + player.0, y - pos.1 + player.1))
                .filter(|&(x, y)| x >= 0 && y >= 0 && (x as usize) < cols && (y as usize) < rows)
                .map(|(x, y)| [x as u16, y as u16])
                .collect();
            let view = MinimapView { enabled: true, visible: true, message: String::new(), cols, rows, walls, player: Some([player.0 as f32, player.1 as f32]) };
            std::thread::sleep(pace);
            recorder.on_frame(&view, [600, 518]);
            let now = serde_json::to_value(recorder.view().unwrap()).unwrap();
            let placed = last.is_null() || now["matched"].as_u64() > last["matched"].as_u64() || now["part"] != last["part"];
            let new_part = !last.is_null() && now["part"].as_u64() > last["part"].as_u64();
            if !last.is_null() && now["part"].as_u64() < last["part"].as_u64() {
                println!("  step {step}: pieces joined, part {}", now["part"]);
            }
            log.push((step, placed, new_part));
            last = now;
        }
        (last, log)
    }

    /// A walk east through `rooms`, frames not read for `gap` steps from step 40 while the
    /// player runs on (4 cells a step).
    fn gap_path() -> Vec<(i32, i32)> {
        (0..110).map(|step| (120 + step * 4, 350 + ((step as f32 / 8.0).sin() * 25.0) as i32)).collect()
    }

    #[test]
    fn a_wide_frame_finds_its_place_again_after_a_gap() {
        // 12 steps unread: the player is 48 cells further on, beyond the recorder's
        // near and wide searches (12 and 32 cells around the last place).
        let path = gap_path();
        let (json, log) = fog_walk(&path, |step| !(40..52).contains(&step), 600, 518, (300, 259));
        let lost: Vec<usize> = log.iter().filter(|e| !e.1).map(|e| e.0).collect();
        println!("wide frames: lost at steps {lost:?}, part {}", json["part"]);
        assert_eq!(json["part"], 1, "a new part was started");
        assert!(lost.is_empty(), "lost at {lost:?}");
        // And it is placed where the player is.
        let at = |v: &serde_json::Value| (v[0].as_f64().unwrap(), v[1].as_f64().unwrap());
        let (first, end) = (at(&json["trail"][0]), at(&json["player"]));
        let (walked, start) = (path[path.len() - 1], path[0]);
        assert!(json["trail"].as_array().unwrap().len() > 50);
        let scale = (end.0 - first.0) / (walked.0 - start.0) as f64;
        assert!((scale - 1.0).abs() < 0.02 || scale < 1.0, "{scale}");
    }

    /// The same gap with minimap-sized frames, and joining pieces with both: prints what
    /// each does (`cargo test gaps_by_frame_size -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn gaps_by_frame_size() {
        let path = gap_path();
        for gap in [8, 12, 20, 30] {
            for (name, cols, rows, player) in [("minimap-sized", 300, 259, (158, 142)), ("wide", 600, 518, (300, 259))] {
                let started = Instant::now();
                let (json, log) = fog_walk(&path, |step| !(40..40 + gap).contains(&step), cols, rows, player);
                let lost = log.iter().filter(|e| !e.1).count();
                let parts = log.iter().filter(|e| e.2).count() + 1;
                println!("gap {gap} steps ({} cells), {name}: {lost} frames unplaced, {parts} parts, part now {}, {:?}", gap * 4, json["part"], started.elapsed());
            }
        }
    }

    /// Joining pieces of a visit: walk west, a waypoint jump 360 cells east (a new piece
    /// starts), then walk back west; at real pace (330 ms a frame) the recorder tries to
    /// join pieces every 3 s. `cargo test pieces_join_by_frame_size -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn pieces_join_by_frame_size() {
        let mut path: Vec<(i32, i32)> = (0..30).map(|step| (120 + step * 4, 350)).collect();
        path.extend((0..70).map(|step| (600 - step * 5, 350 + ((step as f32 / 8.0).sin() * 20.0) as i32)));
        for (name, cols, rows, player) in [("minimap-sized", 300, 259, (158, 142)), ("wide", 600, 518, (300, 259))] {
            println!("{name}:");
            let (json, log) = fog_walk_paced(&path, |_| true, cols, rows, player, FRAME_INTERVAL);
            let new_parts: Vec<usize> = log.iter().filter(|e| e.2).map(|e| e.0).collect();
            println!("  new pieces at steps {new_parts:?}; part at the end {}", json["part"]);
        }
    }

    #[test]
    fn parses_grim_ppm() {
        let mut bytes = b"P6\n# grim\n2 1\n255\n".to_vec();
        bytes.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let img = parse_ppm(&bytes).unwrap();
        assert_eq!((img.width, img.height, img.pixels), (2, 1, vec![1, 2, 3, 4, 5, 6]));
        assert!(parse_ppm(b"P6\n2 1\n255\n\x01").is_err());
    }
}
