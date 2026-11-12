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
#[cfg(target_os = "windows")]
#[path = "minimap_windows.rs"]
mod windows_capture;
#[cfg(target_os = "windows")]
use windows_capture::{capture, capture_bigmap, locate_minimap, Region as CaptureRegion};

/// About three frames a second.
const FRAME_INTERVAL: Duration = Duration::from_millis(330);
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
}

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

/// Which size of frame the big map is handed to the recorder in.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
enum BigFrame {
    /// The minimap's size, with the player on the minimap's player cell: today's recorder
    /// keeps its player cell when the frame changes, so this continues one track as is.
    LikeMinimap,
    /// A larger frame (cols, rows) centred on the player: more of the explored map per
    /// frame. Needs the recorder to take a new player cell when the frame size changes.
    Wide(usize, usize),
}
const BIG_FRAME: BigFrame = BigFrame::LikeMinimap;

/// Where the maps are read on screen. On Linux the areas are grim geometry in logical
/// compositor coordinates ("x,y wxh").
#[cfg(not(target_os = "windows"))]
#[derive(Clone, Debug, PartialEq)]
pub struct CaptureRegion {
    minimap: String,
    bigmap: String,
    /// Where the minimap draws the player, inside the minimap capture, in its pixels.
    minimap_marker: [f64; 2],
    plan: bigmap::Plan,
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
    Ok(CaptureRegion {
        minimap: geometry(minimap),
        bigmap: geometry(big),
        minimap_marker: layout.minimap_marker,
        plan: bigmap::Plan { marker, radius: bigmap::radius(window_px), frame: layout.frame },
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
    let frame = match BIG_FRAME {
        BigFrame::LikeMinimap => like_minimap,
        BigFrame::Wide(cols, rows) => bigmap::Frame::wide(cols, rows),
    };
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
    let output = std::process::Command::new("grim")
        .args(["-t", "ppm", "-g", geometry, "-"])
        .output()
        .map_err(|e| format!("grim unavailable: {e}"))?;
    if !output.status.success() {
        return Err(format!("grim failed: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    parse_ppm(&output.stdout)
}

#[cfg(target_os = "linux")]
fn capture(region: &CaptureRegion) -> Result<Image, String> {
    grim(&region.minimap)
}

#[cfg(target_os = "linux")]
fn capture_bigmap(region: &CaptureRegion) -> Result<Image, String> {
    grim(&region.bigmap)
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

/// One reading: the frame for the recorder, the capture size it stores (minimap pixels)
/// and the map icons around the player.
struct Reading {
    view: MinimapView,
    capture_px: [usize; 2],
    icons: Vec<bigmap::Icon>,
}

/// What the reader keeps from frame to frame.
#[derive(Default)]
struct Memory {
    /// Where accepted minimap frames had their player cross (minimap-capture pixels,
    /// averaged), and the estimate it was learnt from: a new estimate (the window moved or
    /// was resized) starts over.
    minimap_marker: Option<([f64; 2], [f64; 2])>,
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
/// big map was not read.
fn read(memory: &mut Memory, region: &CaptureRegion) -> Result<Reading, String> {
    choose(memory, capture(region), region.minimap_marker, &region.plan, || capture_bigmap(region))
}

/// `minimap` is the corner's capture; when it failed (on Windows: covered by a window) the
/// big map is still tried, and the error is returned only if that is not read either.
fn choose(
    memory: &mut Memory,
    minimap: Result<Image, String>,
    estimate: [f64; 2],
    plan: &bigmap::Plan,
    bigmap: impl FnOnce() -> Result<Image, String>,
) -> Result<Reading, String> {
    let search = plan.radius * bigmap::LEARN_LIMIT;
    if let Ok(image) = &minimap {
        let mut view = analyse(image, memory.minimap_marker(estimate), search);
        if let Some(seen) = view.player {
            memory.learn(estimate, seen.map(|v| v as f64 * CELL as f64), search);
            // Both views report the learnt spot, so the player cell does not step at a switch.
            let at = memory.minimap_marker(estimate);
            view.player = Some(at.map(|v| (v / CELL as f64) as f32));
            let icons = bigmap::icons(image, at, 1.0, bigmap::size(plan));
            return Ok(Reading { view, capture_px: [image.width, image.height], icons });
        }
    }
    let mut frame = plan.frame;
    if frame.follows_minimap {
        frame.player = memory.minimap_marker(estimate).map(|v| (v / CELL as f64) as f32);
    }
    let why = match bigmap() {
        Ok(image) => {
            let (view, icons) = bigmap::read(&image, &bigmap::Plan { frame, ..*plan });
            if view.visible {
                return Ok(Reading { view, capture_px: frame.capture_px(), icons });
            }
            view.message
        }
        Err(error) => format!("Big map not read: {error}"),
    };
    let image = minimap?;
    let view = MinimapView { cols: image.width.div_ceil(CELL), rows: image.height.div_ceil(CELL), ..MinimapView::status(true, &format!("Minimap not visible. {why}")) };
    Ok(Reading { view, capture_px: [image.width, image.height], icons: vec![] })
}

pub struct Minimap {
    pub last: Arc<std::sync::Mutex<MinimapView>>,
    /// Map icons of the last frame, in minimap cells from the player. Not used yet.
    #[allow(dead_code)]
    pub icons: Arc<std::sync::Mutex<Vec<bigmap::Icon>>>,
}

pub fn init() -> Minimap {
    Minimap {
        last: Arc::new(std::sync::Mutex::new(MinimapView::status(true, "Starting"))),
        icons: Arc::default(),
    }
}

pub fn spawn(app: AppHandle, minimap: &Minimap) {
    let last = minimap.last.clone();
    let last_icons = minimap.icons.clone();
    std::thread::spawn(move || {
        let mut region: Option<(Result<CaptureRegion, String>, Instant)> = None;
        let mut memory = Memory::default();
        loop {
            let started = Instant::now();
            let view = {
                if region.as_ref().is_none_or(|(_, at)| at.elapsed() > LOCATE_INTERVAL) {
                    region = Some((locate_minimap(), Instant::now()));
                }
                match &region {
                    None => MinimapView::status(true, "Starting"),
                    Some((Err(error), _)) => MinimapView::status(true, error),
                    Some((Ok(area), _)) => match read(&mut memory, area) {
                        Ok(Reading { view, capture_px, icons }) => {
                            if let Ok(mut last) = last_icons.lock() {
                                *last = icons;
                            }
                            let recorder = app.state::<crate::layout::Recorder>();
                            if let Some(layout) = recorder.on_frame(&view, capture_px) {
                                let _ = app.emit("layout-state", layout);
                            }
                            view
                        }
                        Err(error) => {
                            region = None;
                            MinimapView::status(true, &error)
                        }
                    },
                }
            };
            let changed = last.lock().map(|mut l| {
                let changed = *l != view;
                *l = view.clone();
                changed
            });
            if changed.unwrap_or(true) {
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
        let Reading { view, capture_px, icons } = read(&mut Memory::default(), &region).unwrap();
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

    /// Time per frame on the live game: `cargo test [--release] live_timing -- --ignored --nocapture`.
    #[test]
    #[ignore]
    #[cfg(target_os = "linux")]
    fn live_timing() {
        use crate::layout::Recorder;
        use crate::tracker::LogEvent;
        let region = locate_minimap().unwrap();
        let recorder = Recorder::new(None);
        recorder.on_log(&LogEvent::ZoneEntered { area: "live-test".into(), level: 0, seed: None }, false);
        let mut memory = Memory::default();
        let (mut minimap, mut big, mut analysis, mut record) = (vec![], vec![], vec![], vec![]);
        let mut message = String::new();
        for _ in 0..20 {
            let t = Instant::now();
            let image = capture(&region);
            minimap.push(t.elapsed());
            let t = Instant::now();
            let image_big = capture_bigmap(&region);
            big.push(t.elapsed());
            let t = Instant::now();
            let Reading { view, capture_px, .. } = choose(&mut memory, image, region.minimap_marker, &region.plan, || image_big).unwrap();
            analysis.push(t.elapsed());
            let t = Instant::now();
            recorder.on_frame(&view, capture_px);
            record.push(t.elapsed());
            message = format!("{}, {} walls", view.message, view.walls.len());
        }
        let stat = |v: &[Duration]| {
            let ms: Vec<f64> = v.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
            format!("mean {:.1} ms, max {:.1} ms", ms.iter().sum::<f64>() / ms.len() as f64, ms.iter().cloned().fold(0.0, f64::max))
        };
        println!("{message}; regions {} / {}", region.minimap, region.bigmap);
        println!("minimap capture {}; big-map capture {}; analysis {}; recorder {}", stat(&minimap), stat(&big), stat(&analysis), stat(&record));
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
            let Ok(Reading { view, capture_px, .. }) = read(&mut memory, &region) else { continue };
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
        let frame = bigmap::Frame { cols: 190, rows: 150, player: [150.0, 30.0], follows_minimap: false };
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
        let reading = choose(&mut Memory::default(), Ok(minimap()), [60.5, 50.5], &plan, || panic!("big map read")).unwrap();
        assert_eq!((reading.view.message.as_str(), reading.capture_px), ("Reading minimap", [120, 100]));
    }

    #[test]
    fn big_map_is_read_when_the_corner_shows_zone_text() {
        // The corner while the big map is open: zone name and league text, no minimap.
        let corner = fixture("corner-bigmap-open.ppm");
        let (big, plan) = big_map();
        let Reading { view, capture_px, icons } = choose(&mut Memory::default(), Ok(corner), [100.0, 80.0], &plan, || Ok(big)).unwrap();
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
        let reading = choose(&mut Memory::default(), Ok(corner_lines([120, 100])), [60.5, 50.5], &plan, || Err("grim failed".into())).unwrap();
        assert!(!reading.view.visible && reading.view.walls.is_empty());
        assert_eq!(reading.view.message, "Minimap not visible. Big map not read: grim failed");
        let scene = fixture("bigmap-scene.ppm");
        let reading = choose(&mut Memory::default(), Ok(corner_lines([120, 100])), [60.5, 50.5], &plan, || Ok(scene)).unwrap();
        assert!(!reading.view.visible);
        assert_eq!(reading.view.message, "Minimap not visible. Big map not read: player marker not found");
        // An exit icon (or fire) in the corner is no cross.
        let mut with_exit = corner_lines([120, 100]);
        for y in 41..61 {
            for x in 52..70 {
                with_exit.pixels[(y * 120 + x) * 3..][..3].copy_from_slice(&[205, 96, 38]);
            }
        }
        let reading = choose(&mut Memory::default(), Ok(with_exit), [60.5, 50.5], &plan, || Err("grim failed".into())).unwrap();
        assert!(!reading.view.visible && reading.view.player.is_none());
    }

    #[test]
    fn the_cross_is_looked_for_where_it_was_last_seen() {
        let (_, plan) = big_map();
        let mut memory = Memory::default();
        // Expected at (75, 50), seen at (60, 50): learnt.
        let reading = choose(&mut memory, Ok(minimap()), [75.5, 50.5], &plan, || panic!("big map read")).unwrap();
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
        let reading = choose(&mut Memory::default(), covered(), [60.5, 50.5], &plan, || Ok(big)).unwrap();
        assert_eq!(reading.view.message, "Reading big map");
        // Neither: the minimap's own error, so the reader locates the game again.
        let scene = fixture("bigmap-scene.ppm");
        let error = choose(&mut Memory::default(), covered(), [60.5, 50.5], &plan, || Ok(scene)).err().unwrap();
        assert!(error.starts_with("Minimap covered"), "{error}");
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
        assert_eq!(r.bigmap, "2690,367 343x302");
        assert_eq!((r.plan.frame.cols, r.plan.frame.rows), (300, 259));
        let [px, py] = r.plan.frame.player;
        assert!((px - 158.0).abs() < 0.1 && (py - 142.0).abs() < 0.1, "{px},{py}");
        // The big-map marker (1919, 1059 window px) inside the big-map capture.
        let [mx, my] = r.plan.marker;
        assert!((mx - (1918.8 - (2690.0 - 1910.0) * 2.0)).abs() < 0.2, "{mx}");
        assert!((my - (1059.0 - (367.0 - 1.0) * 2.0)).abs() < 0.2, "{my}");
        assert_eq!(r.minimap_marker, [px as f64 * 2.0, py as f64 * 2.0]);
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
            let reading = choose(&mut memory, Ok(corner), walk.estimate, &walk.plan, || Ok(big)).unwrap();
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

    /// The owner's window: 3840x2160 px, minimap capture 600x518 px at 3240,0.
    fn owner(painted_off: [f64; 2]) -> Walk {
        let (estimate, plan, big_size) = plan_for([3840.0, 2160.0], [3240.0, 0.0], [600, 518], None);
        let painted = [estimate[0] + painted_off[0], estimate[1] + painted_off[1]];
        Walk { estimate, painted, minimap_size: [600, 518], plan, big_size, line: 4 }
    }

    #[test]
    fn one_track_across_minimap_and_big_map() {
        let walk_spec = owner([0.0, 0.0]);
        assert_eq!(walk_spec.big_size, [685, 603]);
        let (json, seen, path) = walk(&walk_spec);
        assert_eq!(seen.iter().filter(|&&v| v == "big").count(), 30);
        assert_eq!((seen[29], seen[30], seen[59], seen[60]), ("minimap", "big", "big", "minimap"));
        assert_one_track(&json, &path);
    }

    #[test]
    fn one_track_when_the_minimap_cross_is_off_the_estimate() {
        // The cross 12 px right and 10 px up of where it was expected (6 and 5 cells, more
        // than the recorder's 4-cell rule): the reader learns its spot, and big-map frames
        // put the player there too. (Within 4 cells there is no step either way: each view
        // places its walls around the player cell it reports.)
        let (json, seen, path) = walk(&owner([12.0, -10.0]));
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

    /// Larger big-map frames (BigFrame::Wide) with the recorder as it is: it keeps the
    /// minimap's player cell when the frame size changes, so the track breaks. Passes
    /// once the recorder takes the new player cell (docs/map-reading.md, "Big map").
    #[test]
    #[ignore]
    fn one_track_with_wide_big_map_frames() {
        let (estimate, plan, big_size) = plan_for([3840.0, 2160.0], [3240.0, 0.0], [600, 518], Some(bigmap::Frame::wide(600, 518)));
        let (json, _, path) = walk(&Walk { estimate, painted: estimate, minimap_size: [600, 518], plan, big_size, line: 4 });
        assert_one_track(&json, &path);
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
