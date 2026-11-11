//! Live minimap reading: takes screenshots of the game's minimap corner and pulls out
//! the explored walls and the player marker by colour.
//!
//! Reads only pixels already on screen, through the OS screenshot tool. It never reads
//! game files or memory, and never sends input to the game.

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// About three frames a second.
const FRAME_INTERVAL: Duration = Duration::from_millis(330);
const IDLE_INTERVAL: Duration = Duration::from_secs(2);
/// Where the game is checked again (window moved, workspace switched).
const LOCATE_INTERVAL: Duration = Duration::from_secs(2);

/// The top-right minimap, as a fraction of the game's monitor (measured at 3840x2160).
const REGION_X: f64 = 0.84375;
const REGION_W: f64 = 0.15625;
const REGION_H: f64 = 0.24;

/// Native pixels per cell of the wall grid sent to the UI.
const CELL: usize = 4;
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

/// Screen area of the minimap in logical compositor coordinates: "x,y wxh".
#[cfg(target_os = "linux")]
fn locate_minimap() -> Result<String, String> {
    use serde_json::Value;
    let hyprctl = |what: &str| -> Result<Value, String> {
        let output = std::process::Command::new("hyprctl")
            .args([what, "-j"])
            .output()
            .map_err(|e| format!("hyprctl unavailable ({e}); only Hyprland is supported so far"))?;
        serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())
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
    Ok(format!(
        "{},{} {}x{}",
        (x + w * REGION_X).round(),
        y.round(),
        (w * REGION_W).round(),
        (h * REGION_H).round()
    ))
}

#[cfg(target_os = "linux")]
fn capture(region: &str) -> Result<Image, String> {
    let output = std::process::Command::new("grim")
        .args(["-t", "ppm", "-g", region, "-"])
        .output()
        .map_err(|e| format!("grim unavailable: {e}"))?;
    if !output.status.success() {
        return Err(format!("grim failed: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    parse_ppm(&output.stdout)
}

#[cfg(not(target_os = "linux"))]
fn locate_minimap() -> Result<String, String> {
    Err("Live minimap reading is only implemented for Linux (Hyprland) so far".into())
}

#[cfg(not(target_os = "linux"))]
fn capture(_region: &str) -> Result<Image, String> {
    Err("Live minimap reading is only implemented for Linux (Hyprland) so far".into())
}

pub struct Minimap {
    pub enabled: Arc<AtomicBool>,
    pub last: Arc<std::sync::Mutex<MinimapView>>,
}

pub fn init(enabled: bool) -> Minimap {
    let message = if enabled { "Starting" } else { "Off" };
    Minimap {
        enabled: Arc::new(AtomicBool::new(enabled)),
        last: Arc::new(std::sync::Mutex::new(MinimapView::status(enabled, message))),
    }
}

pub fn spawn(app: AppHandle, minimap: &Minimap) {
    let enabled = minimap.enabled.clone();
    let last = minimap.last.clone();
    std::thread::spawn(move || {
        let mut region: Option<(Result<String, String>, Instant)> = None;
        loop {
            let on = enabled.load(Ordering::Relaxed);
            let started = Instant::now();
            let view = if !on {
                region = None;
                MinimapView::status(false, "Off")
            } else {
                if region.as_ref().is_none_or(|(_, at)| at.elapsed() > LOCATE_INTERVAL) {
                    region = Some((locate_minimap(), Instant::now()));
                }
                match &region {
                    None => MinimapView::status(true, "Starting"),
                    Some((Err(error), _)) => MinimapView::status(true, error),
                    Some((Ok(area), _)) => match capture(area) {
                        Ok(image) => analyse(&image, true),
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
            let wait = if on && view.visible { FRAME_INTERVAL } else if on { FRAME_INTERVAL * 2 } else { IDLE_INTERVAL };
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
        assert_eq!((view.cols, view.rows), (16, 16));
        assert_eq!(view.walls.len(), 16);
        assert!(view.walls.iter().all(|[_, row]| *row == 5));
        let [px, py] = view.player.unwrap();
        assert!((px - 7.875).abs() < 0.01 && (py - 10.375).abs() < 0.01, "{px},{py}");
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
        let image = capture(&region).unwrap();
        let view = analyse(&image, true);
        println!(
            "region {region}: {}x{} px in {:?}; {}; {} wall cells of {}x{}; player {:?}",
            image.width,
            image.height,
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
        for row in art.iter().step_by(2) {
            println!("{}", row.iter().step_by(1).collect::<String>().trim_end());
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
