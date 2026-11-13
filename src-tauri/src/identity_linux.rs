//! Linux (Hyprland): the character screens are read as on Windows, one screenshot per real
//! input, taken with `grim` and read in process (`identity_ocr.rs`).
//!
//! Wayland lets no program watch the input of others. Hyprland's non-consuming binds
//! (`bindn`) can: while the player is in the menus (not in a zone) and the game runs, a left
//! click, Enter and the keys that edit a name are bound to Hyprland's `event` dispatcher.
//! The game still gets every click and key; Hyprland only also writes `custom>>batonga-…`
//! to its event socket, which is read here. No process is started per input, nothing is
//! sent to the game. In a zone the binds are removed again, so gameplay input is not
//! watched at all.
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use crate::minimap::Image;
use super::ocr::{self, Engine, Read, AREAS};
use super::shared::{self, focus_of, report, request, without_creation_caret, Batch, Frame, Reader, CREATION_FOCUS, NO_CLICK};
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

static BOUND_KEYS: AtomicBool = AtomicBool::new(false);
static LAST_CLICK_MS: AtomicU64 = AtomicU64::new(0);
static LAST_CLICK_AT: AtomicU64 = AtomicU64::new(NO_CLICK);
/// The loaded models, held only while the player is in the menus (`keep_binds`): they take
/// about 300 MB, which belongs to the game while it is played. Loading takes about 60 ms.
static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);

/// Why the character screens cannot be read here, if they cannot: Hyprland must answer (the
/// input binds and the capture are its). `BATONGA_SCREEN_RECOGNITION=0` turns the reading
/// off; the log and the last chosen character still tell who plays.
fn missing() -> Option<String> {
    if std::env::var("BATONGA_SCREEN_RECOGNITION").is_ok_and(|v| v == "0") {
        return Some("Screen recognition is switched off (BATONGA_SCREEN_RECOGNITION=0)".into());
    }
    if crate::layer_shell::hyprland_request("j/version").is_none() {
        return Some("Reading the character screens on Linux needs Hyprland".into());
    }
    None
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

/// Hyprland configured in Lua (0.55 on) refuses `keyword` ("keyword can't work with
/// non-legacy parsers. Use eval."; the owner's Hyprland 0.56.2, 2026-11-13): there the binds
/// go through `eval`. Each bind's handle is kept in a Lua global, so removing them takes
/// exactly these binds and none of the user's own on the same keys.
static LUA: AtomicBool = AtomicBool::new(false);

/// The binds as one Lua `eval`: added (any left from a run that did not end cleanly removed
/// first) or removed.
fn lua_binds(add: bool) -> String {
    let clear = "for _, b in ipairs(_G.batonga_binds or {}) do b:remove() end";
    if !add {
        return format!("eval {clear}; _G.batonga_binds = nil");
    }
    let mut keys: Vec<(String, &str)> = vec![("mouse:272".into(), CLICK), ("Return".into(), ENTER), ("KP_Enter".into(), ENTER)];
    for key in EDIT_KEYS {
        keys.push((key.to_string(), EDIT));
        keys.push((format!("SHIFT + {key}"), EDIT));
    }
    let calls: Vec<String> = keys
        .iter()
        .map(|(key, event)| format!("hl.bind(\"{key}\", hl.dsp.event(\"{event}\"), {{ non_consuming = true }})"))
        .collect();
    format!("eval {clear}; _G.batonga_binds = {{ {} }}", calls.join(", "))
}

/// Adds or removes the binds (once per change).
fn set_binds(on: bool) {
    if BOUND_KEYS.swap(on, Ordering::SeqCst) == on { return; }
    let lua = LUA.load(Ordering::SeqCst);
    let mut reply = crate::layer_shell::hyprland_request(&if lua { lua_binds(on) } else { binds(on) });
    if !lua && reply.as_deref().is_some_and(|r| r.contains("non-legacy")) {
        LUA.store(true, Ordering::SeqCst);
        reply = crate::layer_shell::hyprland_request(&lua_binds(on));
    }
    super::trace("hyprland-binds", serde_json::json!({"on":on,"lua":LUA.load(Ordering::SeqCst),"reply":reply.map(|r| r.chars().take(200).collect::<String>())}));
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
        // In a zone, or with the game closed, nothing is read: the models' memory is given
        // back. A read still running keeps them until it ends (`try_lock`).
        if !wanted {
            if let Ok(mut engine) = ENGINE.try_lock() {
                if engine.take().is_some() {
                    // SAFETY: a plain C library call; it hands the freed memory back to the system.
                    unsafe { libc::malloc_trim(0); }
                    super::trace("ocr-released", serde_json::json!({}));
                }
            }
        }
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

/// Reads the screens with the bundled models (`identity_ocr.rs`).
struct Models;

impl Models {
    /// The whole frame's read and the focused one.
    fn read(engine: &mut Engine, image: &Image) -> Result<(Frame, Frame), String> {
        let scale = ocr::scale_for(image.height);
        let whole = |engine: &mut Engine, image: &Image| -> Result<Frame, String> {
            let mut lines = Vec::new();
            for area in AREAS { lines.extend(engine.read(image, area, scale, Read::Whole)?); }
            Ok(ocr::frame(image, lines))
        };
        let mut full = whole(engine, image)?;
        let Some(area) = focus_of(&full) else { return Ok((full.clone(), full)) };
        // The creation screen's input caret would be read as a letter after the name: it is
        // masked and the frame read again, as on Windows.
        let cleaned = (area == CREATION_FOCUS).then(|| without_creation_caret(image)).flatten();
        if let Some(cleaned) = &cleaned { full = whole(engine, cleaned)?; }
        let image = cleaned.as_ref().unwrap_or(image);
        let focused = ocr::frame(image, engine.read(image, area, scale, Read::Focused)?);
        Ok((full, focused))
    }
}

impl Reader for Models {
    /// Two independent reads, as the recognizer expects: the menu areas of the whole frame
    /// with the larger reading model, then the name list or the name field alone with the
    /// smaller one. The models are loaded when first needed and kept while in the menus.
    fn frames(&self, image: &Image) -> Result<(Frame, Frame), String> {
        // Reading stays out of the game's way: one thread (`identity_ocr.rs`), at the lowest
        // CPU priority. Set here, not when the thread starts: the capture thread started
        // from it must stay prompt.
        // SAFETY: a plain system call; on Linux it changes the calling thread alone. A
        // refusal is ignored: normal priority then.
        unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, 19); }
        let mut held = crate::relock(&ENGINE);
        if held.is_none() {
            let started = Instant::now();
            *held = Some(Engine::load()?);
            super::trace("ocr-loaded", serde_json::json!({"elapsedMs":started.elapsed().as_millis()}));
        }
        let engine = held.as_mut().expect("loaded above");
        // A line the models' own code cannot take (a box without size) panics in it: that
        // costs this frame and the loaded models, not the reading thread.
        let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Self::read(engine, image)));
        read.unwrap_or_else(|_| {
            *held = None;
            Err("the text models failed on this frame".into())
        })
    }
}

/// Exercise OCR -> the webview's recognizer -> IPC with a supplied PPM, never a profile.
pub fn reference_probe(app: AppHandle) {
    let Some(path) = std::env::var_os("BATONGA_IDENTITY_REFERENCE") else { return };
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let image = crate::minimap::parse_ppm(&std::fs::read(path).map_err(|e| e.to_string())?)?;
            let (full, focused) = Models.frames(&image)?;
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
            report(&app, &format!("{why}; your character is taken from the game log, or chosen in settings"));
            return;
        }
        super::trace("ocr-ready", serde_json::json!({"engine":"pp-ocrv6","input":"hyprland-binds"}));
        report(&app, "Ready to read the character creation or selection screen");
        std::thread::spawn(keep_binds);
        let events = app.clone();
        std::thread::spawn(move || listen(events));
        shared::run(app, &Models, game_foreground, crate::minimap::capture_identity_frame);
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

    /// Reads a screenshot as the app does and writes both frames as JSON, for the
    /// recognizer to be checked offline: `BATONGA_IDENTITY_PPM=shot.ppm
    /// BATONGA_IDENTITY_OUT=frames.json cargo test read_a_screenshot -- --ignored --nocapture`.
    /// Prints how long the models take to load and to read, and the memory they hold.
    #[test]
    #[ignore]
    fn read_a_screenshot() {
        let path = std::env::var("BATONGA_IDENTITY_PPM").expect("BATONGA_IDENTITY_PPM");
        let image = crate::minimap::parse_ppm(&std::fs::read(path).unwrap()).unwrap();
        let memory = || std::fs::read_to_string("/proc/self/status").unwrap_or_default().lines()
            .find(|line| line.starts_with("VmRSS")).map(|line| line.split_whitespace().nth(1).unwrap_or("?").to_string()).unwrap_or_default();
        let before = memory();
        let started = Instant::now();
        Models.frames(&image).unwrap();
        let first = started.elapsed();
        let started = Instant::now();
        let (full, focused) = Models.frames(&image).unwrap();
        println!("first read (with loading) {first:?}, next read {:?}; memory {before} kB before, {} kB loaded", started.elapsed(), memory());
        *crate::relock(&ENGINE) = None;
        unsafe { libc::malloc_trim(0); }
        println!("memory {} kB after the models are let go", memory());
        let out = std::env::var("BATONGA_IDENTITY_OUT").expect("BATONGA_IDENTITY_OUT");
        std::fs::write(out, serde_json::to_string(&serde_json::json!({ "full": full, "focused": focused })).unwrap()).unwrap();
    }

    #[test]
    fn lua_binds_keep_their_handles_and_remove_only_those() {
        let add = lua_binds(true);
        assert!(add.starts_with("eval for _, b in ipairs(_G.batonga_binds or {}) do b:remove() end; _G.batonga_binds = { "));
        assert!(add.contains(r#"hl.bind("mouse:272", hl.dsp.event("batonga-click"), { non_consuming = true })"#));
        assert!(add.contains(r#"hl.bind("SHIFT + a", hl.dsp.event("batonga-edit"), { non_consuming = true })"#));
        assert_eq!(add.matches("hl.bind(").count(), 3 + EDIT_KEYS.len() * 2);
        assert_eq!(lua_binds(false), "eval for _, b in ipairs(_G.batonga_binds or {}) do b:remove() end; _G.batonga_binds = nil");
    }
}
