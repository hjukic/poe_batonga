//! Linux (Hyprland): the character screens are read as on Windows, one screenshot per real
//! input, with `grim` and `tesseract`.
//!
//! Wayland lets no program watch the input of others. Hyprland's non-consuming binds
//! (`bindn`) can: while the player is in the menus (not in a zone) and the game runs, a left
//! click, Enter and the keys that edit a name are bound to Hyprland's `event` dispatcher.
//! The game still gets every click and key; Hyprland only also writes `custom>>batonga-…`
//! to its event socket, which is read here. No process is started per input, nothing is
//! sent to the game. In a zone the binds are removed again, so gameplay input is not
//! watched at all.
use std::io::{BufRead, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use crate::minimap::Image;
use super::shared::{self, focus_of, report, request, Batch, Frame, Reader, NO_CLICK};
use super::tesseract::{self, Prep, AREAS};
pub use super::shared::{finish_unknown, latest_input, set_context, superseding_input};

/// The event names this app's binds send.
const CLICK: &str = "batonga-click";
const ENTER: &str = "batonga-enter";
const EDIT: &str = "batonga-edit";
/// Keys that edit a name in the creation screen (Hyprland key names), bound plain and with
/// Shift. Only that a key was pressed is used, never which.
const EDIT_KEYS: &[&str] = &["a","b","c","d","e","f","g","h","i","j","k","l","m","n","o","p","q","r","s","t","u","v","w","x","y","z",
    "0","1","2","3","4","5","6","7","8","9","minus","BackSpace","Delete","Left","Right","Home","End"];
/// Double-click time and distance: Hyprland has no setting for them; these are the usual.
const DOUBLE_CLICK_MS: u64 = 500;
const DOUBLE_CLICK_PX: i32 = 6;
/// The longest a `tesseract` run may take.
const OCR_TIMEOUT: Duration = Duration::from_secs(8);

static BOUND_KEYS: AtomicBool = AtomicBool::new(false);
static LAST_CLICK_MS: AtomicU64 = AtomicU64::new(0);
static LAST_CLICK_AT: AtomicU64 = AtomicU64::new(NO_CLICK);

/// Whether the character can be read here: Hyprland answers and `tesseract` runs.
/// `BATONGA_SCREEN_RECOGNITION=0` turns it off (the last chosen character is then kept).
pub fn available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| missing().is_none())
}

/// Why the character cannot be read here, if it cannot.
fn missing() -> Option<String> {
    if std::env::var("BATONGA_SCREEN_RECOGNITION").is_ok_and(|v| v == "0") {
        return Some("Screen recognition is switched off (BATONGA_SCREEN_RECOGNITION=0)".into());
    }
    if crate::layer_shell::hyprland_request("j/version").is_none() {
        return Some("Automatic character recognition on Linux needs Hyprland".into());
    }
    let tesseract = std::process::Command::new("tesseract").arg("--list-langs").output();
    match tesseract {
        Err(_) => Some("Automatic character recognition needs tesseract (with English data) installed".into()),
        Ok(out) if !String::from_utf8_lossy(&out.stdout).lines().any(|l| l.trim() == "eng") =>
            Some("tesseract has no English data (eng); install it for character recognition".into()),
        Ok(_) => None,
    }
}

fn hyprland(request: &str) -> Option<serde_json::Value> {
    serde_json::from_str(&crate::layer_shell::hyprland_request(request)?).ok()
}

fn is_game(window: &serde_json::Value) -> bool {
    window["class"].as_str() == Some("steam_app_2694490") || window["title"].as_str() == Some("Path of Exile 2")
}

/// The game is the focused window.
fn game_foreground() -> bool {
    hyprland("j/activewindow").is_some_and(|w| is_game(&w))
}

/// The binds as Hyprland keywords: `bindn` to add, `unbind` to remove.
fn binds(add: bool) -> String {
    let mut keywords = Vec::new();
    let mut bind = |mods: &str, key: &str, event: &str| keywords.push(if add {
        format!("keyword bindn {mods},{key},event,{event}")
    } else {
        format!("keyword unbind {mods},{key}")
    });
    bind("", "mouse:272", CLICK);
    bind("", "Return", ENTER);
    bind("", "KP_Enter", ENTER);
    for key in EDIT_KEYS {
        bind("", key, EDIT);
        bind("SHIFT", key, EDIT);
    }
    format!("[[BATCH]]{}", keywords.join(";"))
}

/// Adds or removes the binds (once per change).
fn set_binds(on: bool) {
    if BOUND_KEYS.swap(on, Ordering::SeqCst) == on { return; }
    let reply = crate::layer_shell::hyprland_request(&binds(on));
    super::trace("hyprland-binds", serde_json::json!({"on":on,"reply":reply.map(|r| r.chars().take(200).collect::<String>())}));
}

/// Keeps the binds while they are wanted: the game runs and the player is not in a zone
/// (or just entered one unbound: the one probe for a missed selection).
fn keep_binds() {
    // Left over from an earlier run that did not end cleanly.
    BOUND_KEYS.store(true, Ordering::SeqCst);
    set_binds(false);
    loop {
        let wanted = shared::menu_input() && crate::game::running() == Some(true);
        set_binds(wanted);
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// Reads Hyprland's event socket for this app's custom events.
fn listen(app: AppHandle) {
    let Some(path) = socket2() else {
        report(&app, "Hyprland's event socket was not found; choose your character in settings");
        return;
    };
    loop {
        match std::os::unix::net::UnixStream::connect(&path) {
            Ok(stream) => {
                for line in std::io::BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    if let Some(event) = line.strip_prefix("custom>>") { on_event(event.trim()); }
                }
            }
            Err(error) => super::trace("hyprland-events", serde_json::json!({"error":error.to_string()})),
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn socket2() -> Option<std::path::PathBuf> {
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let instance = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok().or_else(|| {
        std::fs::read_dir(format!("{runtime}/hypr")).ok()?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().join(".socket2.sock").exists())
            .max_by_key(|entry| entry.metadata().and_then(|m| m.modified()).ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
    })?;
    Some(format!("{runtime}/hypr/{instance}/.socket2.sock").into())
}

fn on_event(event: &str) {
    if !shared::menu_input() { return; }
    match event {
        CLICK => on_click(),
        ENTER => if game_foreground() { request(0, true, false, None) },
        EDIT => if game_foreground() { request(65, false, false, None) },
        _ => {}
    }
}

/// A left click: where in the game window it was, as on Windows (Play, a row double-click).
fn on_click() {
    let Some(window) = hyprland("j/activewindow").filter(is_game) else { return };
    let Some(cursor) = hyprland("j/cursorpos") else { return };
    let number = |v: &serde_json::Value| v.as_f64().unwrap_or(0.0);
    let (x, y) = (number(&cursor["x"]), number(&cursor["y"]));
    let (left, top, width, height) = (number(&window["at"][0]), number(&window["at"][1]), number(&window["size"][0]), number(&window["size"][1]));
    let at = (width > 0.0 && height > 0.0).then(|| ((x - left) / width, (y - top) / height))
        .filter(|(fx, fy)| (0.0..=1.0).contains(fx) && (0.0..=1.0).contains(fy));
    let in_zone = shared::IN_ZONE.load(Ordering::SeqCst);
    let row = !in_zone && at.is_some_and(|(fx, fy)| shared::in_character_list(fx, fy));
    static START: OnceLock<Instant> = OnceLock::new();
    let now = START.get_or_init(Instant::now).elapsed().as_millis() as u64;
    let (px, py) = (x.round() as i32, y.round() as i32);
    let double = row && shared::second_click(LAST_CLICK_MS.load(Ordering::SeqCst), LAST_CLICK_AT.load(Ordering::SeqCst),
        now, px, py, DOUBLE_CLICK_MS, (DOUBLE_CLICK_PX * 2, DOUBLE_CLICK_PX * 2));
    LAST_CLICK_MS.store(now, Ordering::SeqCst);
    LAST_CLICK_AT.store(if row && !double { shared::pack_point(px, py) } else { NO_CLICK }, Ordering::SeqCst);
    shared::click_request(at, double);
}

/// Runs `tesseract` on one prepared crop and returns its TSV.
fn tesseract_tsv(pgm: Vec<u8>) -> Result<String, String> {
    use std::process::{Command, Stdio};
    let mut child = Command::new("tesseract")
        .args(["stdin", "stdout", "-l", "eng", "--psm", "11", "--dpi", "300", "tsv"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().map_err(|e| format!("tesseract unavailable: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("tesseract has no input")?;
    std::thread::spawn(move || { let _ = stdin.write_all(&pgm); });
    let mut stdout = child.stdout.take().ok_or("tesseract has no output")?;
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = send.send(stdout.read_to_string(&mut text).map(|_| text));
    });
    let Ok(read) = receive.recv_timeout(OCR_TIMEOUT) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("tesseract did not answer within {OCR_TIMEOUT:?}"));
    };
    let _ = child.wait();
    read.map_err(|e| e.to_string())
}

struct Tesseract;

impl Tesseract {
    fn read(&self, image: &Image, areas: &[[f64; 4]], prep: Prep) -> Result<Frame, String> {
        let scale = tesseract::scale_for(image.height);
        let mut words = Vec::new();
        for area in areas {
            let (pgm, origin) = tesseract::pgm(image, *area, scale, prep);
            words.extend(tesseract::parse_tsv(&tesseract_tsv(pgm)?, origin, scale));
        }
        Ok(tesseract::frame(image, words))
    }
}

impl Reader for Tesseract {
    /// The menu areas read once, then the name list or field a second time, prepared
    /// differently: two independent reads, as the recognizer expects.
    fn frames(&self, image: &Image) -> Result<(Frame, Frame), String> {
        let full = self.read(image, &AREAS, Prep::Inverted)?;
        let focused = match focus_of(&full) {
            Some(area) => self.read(image, &[area], Prep::Threshold)?,
            None => full.clone(),
        };
        Ok((full, focused))
    }
}

/// Exercise OCR -> the webview's recognizer -> IPC with a supplied PPM, never a profile.
pub fn reference_probe(app: AppHandle) {
    let Some(path) = std::env::var_os("BATONGA_IDENTITY_REFERENCE") else { return };
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let image = crate::minimap::parse_ppm(&std::fs::read(path).map_err(|e| e.to_string())?)?;
            let (full, focused) = Tesseract.frames(&image)?;
            super::trace("reference-emitted", shared::frame_summary(&full));
            app.emit("identity-frame", Batch { epoch: 0, seq: 0, full, focused, known: vec![], dry_run: true, click: None }).map_err(|e| e.to_string())
        })();
        if let Err(error) = result { super::trace("reference-failed", serde_json::json!({"error":error})); }
    });
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        if let Some(why) = missing() {
            super::trace("ocr-unavailable", serde_json::json!({"reason":why}));
            report(&app, &format!("{why}; choose your character in settings"));
            return;
        }
        super::trace("ocr-ready", serde_json::json!({"engine":"tesseract","input":"hyprland-binds"}));
        report(&app, "Ready to read the character creation or selection screen");
        std::thread::spawn(keep_binds);
        let events = app.clone();
        std::thread::spawn(move || listen(events));
        shared::run(app, &Tesseract, game_foreground, crate::minimap::capture_identity_frame);
    });
}

/// Removes the binds (on quit). Hyprland keeps them otherwise until it reloads.
pub fn stop() {
    set_binds(false);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binds_are_added_and_removed_as_one_batch() {
        let add = binds(true);
        assert!(add.starts_with("[[BATCH]]keyword bindn ,mouse:272,event,batonga-click;"));
        assert!(add.contains("keyword bindn SHIFT,a,event,batonga-edit"));
        assert!(binds(false).contains("keyword unbind ,Return"));
        assert_eq!(add.matches(';').count() + 1, 3 + EDIT_KEYS.len() * 2);
    }
}
