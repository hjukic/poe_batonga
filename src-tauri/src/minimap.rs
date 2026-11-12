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

/// The top-right minimap, as a fraction of the game's monitor (measured at 3840x2160).
const REGION_X: f64 = 0.84375;
const REGION_W: f64 = 0.15625;
const REGION_H: f64 = 0.24;

/// Native minimap pixels per cell of the wall grid.
pub const CELL: usize = 2;
/// Wall pixels a cell needs, so single stray pixels are ignored.
const CELL_MIN: u32 = 2;
const MARKER_MIN: u32 = 4;
const WALLS_MIN: u32 = 40;

#[derive(Clone, Serialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MinimapView {
    pub enabled: bool,
    /// Walls or the player marker were found in the last frame.
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

pub fn analyse(image: &Image, enabled: bool) -> MinimapView {
    let cols = image.width.div_ceil(CELL);
    let rows = image.height.div_ceil(CELL);
    let mut cells = vec![0u32; cols * rows];
    let (mut wall_count, mut marker_count) = (0u32, 0u32);
    let (mut mx, mut my) = (0f64, 0f64);
    for y in 0..image.height {
        for x in 0..image.width {
            let i = (y * image.width + x) * 3;
            let (r, g, b) = (image.pixels[i], image.pixels[i + 1], image.pixels[i + 2]);
            if is_wall(r, g, b) {
                wall_count += 1;
                cells[(y / CELL) * cols + x / CELL] += 1;
            } else if is_marker(r, g, b) {
                marker_count += 1;
                mx += x as f64;
                my += y as f64;
            }
        }
    }
    let walls: Vec<[u16; 2]> = cells
        .iter()
        .enumerate()
        .filter(|(_, &n)| n >= CELL_MIN)
        .map(|(i, _)| [(i % cols) as u16, (i / cols) as u16])
        .collect();
    let player = (marker_count >= MARKER_MIN).then(|| {
        let n = marker_count as f64;
        [((mx / n + 0.5) / CELL as f64) as f32, ((my / n + 0.5) / CELL as f64) as f32]
    });
    let visible = wall_count >= WALLS_MIN || player.is_some();
    let message = match (visible, player.is_some()) {
        (false, _) => "Minimap not visible",
        (true, false) => "Reading minimap, player marker not found",
        (true, true) => "Reading minimap",
    };
    MinimapView { enabled, visible, message: message.into(), cols, rows, walls, player }
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
        (gx + gw * REGION_X).round(),
        gy.round(),
        (gx + gw * REGION_X).round() + (gw * REGION_W).round(),
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

/// One frame from whichever map is showing: the corner minimap while it is drawn, else the
/// big map (Tab), which hides the minimap; else the minimap's "not visible". Returns the
/// frame and the capture size the recorder stores (minimap pixels).
fn read(region: &CaptureRegion) -> Result<(MinimapView, [usize; 2]), String> {
    let image = capture(region)?;
    choose(&image, region.minimap_marker, &region.plan, || capture_bigmap(region))
}

fn choose(
    minimap: &Image,
    minimap_marker: [f64; 2],
    plan: &bigmap::Plan,
    bigmap: impl FnOnce() -> Result<Image, String>,
) -> Result<(MinimapView, [usize; 2]), String> {
    let view = analyse(minimap, true);
    let size = [minimap.width, minimap.height];
    // The player marker where the minimap draws it: the minimap is showing. Without it,
    // map lines in the corner may be the big map's, so the big map is checked first.
    if view.visible && bigmap::marker_near(minimap, minimap_marker, plan.radius * 1.5).is_some() {
        return Ok((view, size));
    }
    if let Ok(image) = bigmap() {
        let big = bigmap::analyse(&image, plan);
        if big.visible {
            return Ok((big, plan.frame.capture_px()));
        }
    }
    Ok((view, size))
}

pub struct Minimap {
    pub last: Arc<std::sync::Mutex<MinimapView>>,
}

pub fn init() -> Minimap {
    Minimap {
        last: Arc::new(std::sync::Mutex::new(MinimapView::status(true, "Starting"))),
    }
}

pub fn spawn(app: AppHandle, minimap: &Minimap) {
    let last = minimap.last.clone();
    std::thread::spawn(move || {
        let mut region: Option<(Result<CaptureRegion, String>, Instant)> = None;
        loop {
            let started = Instant::now();
            let view = {
                if region.as_ref().is_none_or(|(_, at)| at.elapsed() > LOCATE_INTERVAL) {
                    region = Some((locate_minimap(), Instant::now()));
                }
                match &region {
                    None => MinimapView::status(true, "Starting"),
                    Some((Err(error), _)) => MinimapView::status(true, error),
                    Some((Ok(area), _)) => match read(area) {
                        Ok((view, capture_px)) => {
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

    #[test]
    fn finds_walls_and_player_and_ignores_terrain() {
        let frame = image(64, 64, |x, y| match (x, y) {
            (_, 20 | 21) => [91, 91, 121],                     // wall line
            (30..=32, 40..=42) => [230, 110, 30],              // player cross
            _ if (x + y) % 3 == 0 => [120, 110, 95],           // brownish terrain
            _ => [10, 14, 20],
        });
        let view = analyse(&frame, true);
        assert!(view.visible);
        assert_eq!((view.cols, view.rows), (32, 32));
        assert_eq!(view.walls.len(), 32);
        assert!(view.walls.iter().all(|[_, row]| *row == 10));
        let [px, py] = view.player.unwrap();
        assert!((px - 15.75).abs() < 0.01 && (py - 20.75).abs() < 0.01, "{px},{py}");
    }

    #[test]
    fn dark_frame_is_not_a_minimap() {
        let view = analyse(&image(64, 64, |_, _| [12, 13, 18]), true);
        assert!(!view.visible);
        assert!(view.walls.is_empty() && view.player.is_none());
    }

    /// Needs the game on screen: `cargo test live_minimap -- --ignored --nocapture`.
    #[test]
    #[ignore]
    #[cfg(target_os = "linux")]
    fn live_minimap() {
        let region = locate_minimap().unwrap();
        let started = Instant::now();
        let (view, capture_px) = read(&region).unwrap();
        println!(
            "regions {} / {}: {capture_px:?} px in {:?}; {}; {} wall cells of {}x{}; player {:?}",
            region.minimap,
            region.bigmap,
            started.elapsed(),
            view.message,
            view.walls.len(),
            view.cols,
            view.rows,
            view.player
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
        let (mut minimap, mut big, mut analysis, mut record) = (vec![], vec![], vec![], vec![]);
        let mut message = String::new();
        for _ in 0..20 {
            let t = Instant::now();
            let image = capture(&region).unwrap();
            minimap.push(t.elapsed());
            let t = Instant::now();
            let image_big = capture_bigmap(&region).unwrap();
            big.push(t.elapsed());
            let t = Instant::now();
            let (view, size) = choose(&image, region.minimap_marker, &region.plan, || Ok(image_big)).unwrap();
            analysis.push(t.elapsed());
            let t = Instant::now();
            recorder.on_frame(&view, size);
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
        let (mut frames, mut slowest) = (0, Duration::ZERO);
        while started.elapsed() < Duration::from_secs(seconds) {
            let t = Instant::now();
            let Ok(region) = locate_minimap() else { continue };
            let Ok((view, capture_px)) = read(&region) else { continue };
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
        let frame = bigmap::Frame { cols: 190, rows: 150, player: [150.0, 30.0] };
        (fixture("bigmap-player.ppm"), bigmap::Plan { marker: [300.0, 60.0], radius: 16.0, frame })
    }

    /// A minimap frame: a wall line and the player cross at (60, 50).
    fn minimap() -> Image {
        image(120, 100, |x, y| match (x, y) {
            (_, 20 | 21) => [91, 91, 121],
            (57..=63, 47..=53) => [230, 110, 30],
            _ => [10, 14, 20],
        })
    }

    #[test]
    fn minimap_wins_while_it_shows_its_player() {
        let (_, plan) = big_map();
        let (view, size) = choose(&minimap(), [60.5, 50.5], &plan, || panic!("big map read")).unwrap();
        assert_eq!((view.message.as_str(), size), ("Reading minimap", [120, 100]));
    }

    #[test]
    fn big_map_is_read_when_the_corner_shows_zone_text() {
        // The corner while the big map is open: zone name and league text, no minimap.
        let corner = fixture("corner-bigmap-open.ppm");
        let (big, plan) = big_map();
        let (view, size) = choose(&corner, [100.0, 80.0], &plan, || Ok(big)).unwrap();
        assert_eq!(view.message, "Reading big map");
        assert_eq!(size, [380, 300]);
        assert_eq!((view.cols, view.rows, view.player), (190, 150, Some([150.0, 30.0])));
        assert!(view.walls.len() > 500);
    }

    #[test]
    fn minimap_without_its_player_still_counts_when_there_is_no_big_map() {
        // Walls but no player cross (as today: still read), and the big map shows nothing.
        let walls_only = image(120, 100, |_, y| if y == 20 || y == 21 { [91, 91, 121] } else { [10, 14, 20] });
        let (_, plan) = big_map();
        let scene = fixture("bigmap-scene.ppm");
        let (view, _) = choose(&walls_only, [60.5, 50.5], &plan, || Ok(scene)).unwrap();
        assert_eq!(view.message, "Reading minimap, player marker not found");
        // Neither map: the minimap's "not visible", also when the big map can't be captured.
        let dark = image(120, 100, |_, _| [12, 13, 18]);
        let (view, _) = choose(&dark, [60.5, 50.5], &plan, || Err("grim failed".into())).unwrap();
        assert_eq!(view.message, "Minimap not visible");
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
        assert_eq!(r.bigmap, "2702,379 319x278");
        assert_eq!(r.plan.frame.cols, 300);
        assert_eq!(r.plan.frame.rows, 259);
        let [px, py] = r.plan.frame.player;
        assert!((px - 157.9).abs() < 0.1 && (py - 142.0).abs() < 0.1, "{px},{py}");
        // The big-map marker (1919, 1059 window px) inside the big-map capture.
        let [mx, my] = r.plan.marker;
        assert!((mx - (1918.8 - (2702.0 - 1910.0) * 2.0)).abs() < 0.2, "{mx}");
        assert!((my - (1059.0 - (379.0 - 1.0) * 2.0)).abs() < 0.2, "{my}");
        assert_eq!(r.minimap_marker, [px as f64 * 2.0, py as f64 * 2.0]);
        // A window filling its monitor reads the same corner as before.
        let r = regions([1920.0, 0.0, 1920.0, 1080.0], [1920.0, 0.0, 1920.0, 1080.0], 2.0).unwrap();
        assert_eq!(r.minimap, "3540,0 300x259");
        assert!(regions([0.0, 0.0, 1920.0, 1080.0], [1920.0, 0.0, 1920.0, 1080.0], 2.0).is_err());
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
    /// minimap's scale, the player (world cell `pos`) at pixel `marker`.
    fn screen(width: usize, height: usize, pos: (i32, i32), marker: (i32, i32)) -> Image {
        let mut img = image(width, height, |_, _| [14, 16, 22]);
        let mut put = |x: i32, y: i32, rgb: [u8; 3]| {
            if x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height {
                img.pixels[(y as usize * width + x as usize) * 3..][..3].copy_from_slice(&rgb);
            }
        };
        for (wx, wy) in world() {
            let (x, y) = (marker.0 + (wx - pos.0) * 2, marker.1 + (wy - pos.1) * 2);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                put(x + dx, y + dy, [91, 91, 121]);
            }
        }
        for d in -3..=3 {
            put(marker.0 + d, marker.1, [230, 110, 30]);
            put(marker.0, marker.1 + d, [230, 110, 30]);
            put(marker.0 + d, marker.1 + d.abs() % 2, [230, 110, 30]);
        }
        img
    }

    /// Walks the player through `world` showing the minimap, then the big map, then the
    /// minimap again, and returns the recorder's overview as JSON.
    fn walk(minimap_marker: [f64; 2], plan: &bigmap::Plan, big_size: [usize; 2]) -> (serde_json::Value, Vec<&'static str>, (i32, i32)) {
        use crate::layout::Recorder;
        use crate::tracker::LogEvent;
        let recorder = Recorder::new(None);
        recorder.on_log(&LogEvent::ZoneEntered { area: "test".into(), level: 1, seed: Some(1) }, true);
        let mini_marker = (minimap_marker[0] as i32, minimap_marker[1] as i32);
        let big_marker = (plan.marker[0] as i32, plan.marker[1] as i32);
        let (start, mut pos) = ((60, 80), (60, 80));
        let mut seen = vec![];
        for step in 0..90 {
            pos = (60 + step * 2, 80 + (step as f32 / 9.0).sin() as i32 * 6 + step / 3);
            let big_open = (30..60).contains(&step);
            let corner = if big_open { image(600, 518, |_, _| [14, 16, 22]) } else { screen(600, 518, pos, mini_marker) };
            let big = screen(big_size[0], big_size[1], pos, big_marker);
            let (view, size) = choose(&corner, minimap_marker, plan, || Ok(big)).unwrap();
            seen.push(if view.message == "Reading big map" { "big" } else { "minimap" });
            recorder.on_frame(&view, size);
        }
        let json = serde_json::to_value(recorder.view().unwrap()).unwrap();
        (json, seen, (pos.0 - start.0, pos.1 - start.1))
    }

    /// The recorder's overview: one part, every frame placed, and the player where the walk ended.
    fn assert_one_track(json: &serde_json::Value, moved: (i32, i32)) {
        assert_eq!(json["part"], 1, "a new part was started");
        assert_eq!(json["frames"], json["matched"], "frames left unplaced");
        // The stitched area is under 300 x 260 cells, so the overview is unscaled.
        let at = |v: &serde_json::Value| (v[0].as_f64().unwrap(), v[1].as_f64().unwrap());
        let (first, player) = (at(&json["trail"][0]), at(&json["player"]));
        let (dx, dy) = (player.0 - first.0, player.1 - first.1);
        assert!((dx - moved.0 as f64).abs() <= 1.0 && (dy - moved.1 as f64).abs() <= 1.0, "moved {dx},{dy}, walked {moved:?}");
    }

    /// The owner's game window (3840x2160 px, minimap capture 600x518 px at 3240,0):
    /// where the minimap draws the player, how to read the big map, and its capture size.
    fn owner_plan(frame: Option<bigmap::Frame>) -> ([f64; 2], bigmap::Plan, [usize; 2]) {
        let window = [3840.0, 2160.0];
        let mut layout = big_layout(window, [3240.0, 0.0], [600, 518]);
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

    #[test]
    fn one_track_across_minimap_and_big_map() {
        let (minimap_marker, plan, size) = owner_plan(None);
        assert_eq!(size, [637, 555]);
        let (json, seen, moved) = walk(minimap_marker, &plan, size);
        assert_eq!(seen.iter().filter(|&&v| v == "big").count(), 30);
        assert_eq!((seen[29], seen[30], seen[59], seen[60]), ("minimap", "big", "big", "minimap"));
        assert_one_track(&json, moved);
    }

    /// Larger big-map frames (BigFrame::Wide) with the recorder as it is: it keeps the
    /// minimap's player cell when the frame size changes, so the track breaks. Passes
    /// once the recorder takes the new player cell (docs/map-reading.md, "Big map").
    #[test]
    #[ignore]
    fn one_track_with_wide_big_map_frames() {
        let (minimap_marker, plan, size) = owner_plan(Some(bigmap::Frame::wide(600, 518)));
        let (json, _, moved) = walk(minimap_marker, &plan, size);
        assert_one_track(&json, moved);
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
