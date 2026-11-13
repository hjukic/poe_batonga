//! What Windows and Linux share in reading the character screens: one screenshot per real
//! user input (no timers that read the screen, no injected input), a small queue of frames
//! for OCR, and the frames handed to the shared recognizer (`src/character-screen.mjs`).
//! Each platform brings how it observes input, how it finds the game in front and its OCR.
use std::collections::VecDeque;
use std::sync::{atomic::{AtomicBool, AtomicU64, Ordering}, mpsc::{self, Receiver, SyncSender}, Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use crate::minimap::Image;

static SEQ: AtomicU64 = AtomicU64::new(0);
pub(super) static IN_ZONE: AtomicBool = AtomicBool::new(false);
pub(super) static BOUND: AtomicBool = AtomicBool::new(false);
pub(super) static ARMED: AtomicBool = AtomicBool::new(false);
static RECOVERY_PROBE: AtomicBool = AtomicBool::new(true);
static REQUESTS: OnceLock<SyncSender<Trigger>> = OnceLock::new();
/// No previous row click (see `second_click`).
pub(super) const NO_CLICK: u64 = u64::MAX;
/// Seq of the latest input that starts the game (Play/Continue, a row double-click).
static LAST_ACTIVATION: AtomicU64 = AtomicU64::new(0);

/// `click`: a row double-click's point (fractions of the client area): the row it enters.
#[derive(Clone, Copy)]
pub(super) struct Trigger { seq: u64, delay: u64, activation: bool, focus_click: bool, click: Option<[f64; 2]> }
pub fn latest_input() -> u64 { SEQ.load(Ordering::SeqCst) }
/// The newest input that may still supersede an OCR frame. An activation does
/// not: the frame just before it holds the name that is entering the game.
pub fn superseding_input() -> u64 {
    let latest = latest_input();
    if latest > 0 && LAST_ACTIVATION.load(Ordering::SeqCst) == latest { latest - 1 } else { latest }
}
pub fn set_context(in_zone: bool, bound: bool) {
    IN_ZONE.store(in_zone, Ordering::SeqCst);
    BOUND.store(bound, Ordering::SeqCst);
    if !in_zone {
        ARMED.store(false, Ordering::SeqCst);
        RECOVERY_PROBE.store(true, Ordering::SeqCst);
    } else if bound { RECOVERY_PROBE.store(false, Ordering::SeqCst); }
}

pub fn finish_unknown() {
    if IN_ZONE.load(Ordering::SeqCst) { RECOVERY_PROBE.store(false, Ordering::SeqCst); }
}

/// Whether an input may be on a character screen: in a menu, or just after an unbound
/// zone entry (one probe).
pub(super) fn menu_input() -> bool {
    !IN_ZONE.load(Ordering::SeqCst) ||
        (!BOUND.load(Ordering::SeqCst) && RECOVERY_PROBE.load(Ordering::SeqCst))
}

pub(super) fn request(delay: u64, activation: bool, focus_click: bool, click: Option<[f64; 2]>) {
    let seq = SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    // Before the worker sees it: the selection frame just before an activation
    // must not count as superseded (see `superseding_input`).
    if activation { LAST_ACTIVATION.store(seq, Ordering::SeqCst); }
    if let Some(sender) = REQUESTS.get() { let _ = sender.try_send(Trigger { seq, delay, activation, focus_click, click }); }
}

pub(super) fn pack_point(x: i32, y: i32) -> u64 { ((x as u32 as u64) << 32) | y as u32 as u64 }

/// The second click of a double-click: soon enough after the previous row click
/// and within the system's double-click rectangle around it.
pub(super) fn second_click(previous_ms: u64, previous_at: u64, now_ms: u64, x: i32, y: i32, max_ms: u64, slop: (i32, i32)) -> bool {
    if previous_at == NO_CLICK { return false; }
    let (px, py) = ((previous_at >> 32) as u32 as i32, previous_at as u32 as i32);
    // Hook times are 32-bit tick counts that wrap after 49.7 days.
    ((now_ms as u32).wrapping_sub(previous_ms as u32) as u64) <= max_ms &&
        (x - px).abs() <= slop.0 / 2 && (y - py).abs() <= slop.1 / 2
}

/// A click in the character list on the selection screen (client fractions).
pub(super) fn in_character_list(fx: f64, fy: f64) -> bool {
    (0.79..=0.99).contains(&fx) && (0.215..=0.73).contains(&fy)
}

/// A click on Play (selection) or Continue (creation), in client fractions.
pub(super) fn on_play(fx: f64, fy: f64) -> bool {
    (0.28..=0.62).contains(&fx) && (0.87..=0.95).contains(&fy)
}

/// What one click asks for, in client fractions: an activation, a row double-click's point,
/// and the request (`request`'s delay). `double`: it is a character row's second click.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) fn click_request(at: Option<(f64, f64)>, double: bool) {
    let in_zone = IN_ZONE.load(Ordering::SeqCst);
    let activation = at.is_some_and(|(fx, fy)| on_play(fx, fy)) || double;
    // Selection/edit clicks need a rendered frame; activation needs the
    // preceding field before the transition. OCR never blocks the input.
    request(if activation { 0 } else if in_zone { 180 } else { 80 }, activation, false,
        at.filter(|_| double).map(|(fx, fy)| [fx, fy]));
}

pub(super) fn report(app: &AppHandle, message: &str) {
    super::trace("status", serde_json::json!({"message":message}));
    let tracker = app.state::<crate::tracker::Tracker>();
    let mut inner = crate::relock(&tracker.0);
    inner.identity.note = message.into();
    let _ = app.emit("tracker-state", inner.view());
}

#[derive(Clone, Serialize)]
pub(super) struct Word { pub text: String, pub x: f64, pub y: f64, pub width: f64, pub height: f64 }
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Style { gold_pixels: usize, gold_mean: f64, bright_fraction: f64 }
#[derive(Clone, Serialize)]
pub(super) struct Line { pub text: String, pub words: Vec<Word>, pub style: Option<Style> }
#[derive(Clone, Serialize)]
pub(super) struct Frame { pub width: usize, pub height: usize, pub lines: Vec<Line> }
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Batch { pub epoch: u64, pub seq: u64, pub full: Frame, pub focused: Frame, pub known: Vec<super::KnownCharacter>, pub dry_run: bool, pub click: Option<[f64; 2]> }
struct Job { epoch: u64, seq: u64, image: Image, click: Option<[f64; 2]> }

pub(super) fn frame_summary(frame: &Frame) -> serde_json::Value { frame_summary_in(frame, false) }

/// `menu`: the full frame already showed a menu (the focused crop has no anchors).
pub(super) fn frame_summary_in(frame: &Frame, menu: bool) -> serde_json::Value {
    let anchors: Vec<_> = frame.lines.iter().filter(|l| matches!(l.text.trim().to_uppercase().as_str(),
        "PLAY"|"CONTINUE"|"CREATE"|"DELETE"))
        .map(|l| serde_json::json!({"text":l.text,"at":l.words.first().map(|w|[w.x,w.y])})).collect();
    // Keep text only from the character name/league fields on a known menu.
    let menu = menu || anchors.iter().any(|a| a["text"].as_str().is_some_and(|t| matches!(t.to_uppercase().as_str(), "PLAY"|"CONTINUE")));
    let fields: Vec<_> = if menu { frame.lines.iter().filter(|l| l.words.first().is_some_and(|w| {
        let (x,y) = (w.x / frame.width as f64,w.y / frame.height as f64);
        ((0.39..0.68).contains(&x) && (0.72..0.84).contains(&y)) || ((0.74..0.99).contains(&x) && (0.12..0.82).contains(&y))
    })).map(|l| serde_json::json!({"text":l.text,"at":l.words.first().map(|w|[w.x,w.y]),"w":l.words.first().map(|w|w.width),"words":l.words.len(),"style":l.style})).collect() } else { vec![] };
    serde_json::json!({"width":frame.width,"height":frame.height,"lines":frame.lines.len(),"anchors":anchors,"fields":fields})
}

/// How gold a title is: the selected row's title is bright gold, the others dull.
pub(super) fn style(image: &Image, word: &Word) -> Option<Style> {
    let x = word.x / image.width as f64;
    let y = word.y / image.height as f64;
    // Wider than the name column: a turned screen (up to ~10°) moves the titles.
    if !(0.74..=0.88).contains(&x) || !(0.12..=0.82).contains(&y) { return None; }
    let (mut count, mut total, mut bright) = (0, 0.0, 0);
    for py in word.y.max(0.0) as usize..((word.y + word.height) as usize).min(image.height) {
        for px in word.x.max(0.0) as usize..((word.x + word.width) as usize).min(image.width) {
            let at = (py * image.width + px) * 3;
            let (r, g, b) = (image.pixels[at] as f64, image.pixels[at + 1] as f64, image.pixels[at + 2] as f64);
            if r >= 70.0 && r >= g * 1.08 && g >= b * 1.15 {
                count += 1; total += (r + g) / 2.0;
                if r >= 170.0 && g >= 120.0 { bright += 1; }
            }
        }
    }
    Some(Style { gold_pixels: count, gold_mean: if count > 0 { total / count as f64 } else { 0.0 },
        bright_fraction: if count > 0 { bright as f64 / count as f64 } else { 0.0 } })
}

/// Whether `text` is a line whose first word lies in `box_` (frame fractions).
pub(super) fn anchor(frame: &Frame, text: &str, box_: [f64; 4]) -> bool {
    frame.lines.iter().any(|line| line.text.trim().eq_ignore_ascii_case(text) && line.words.first().is_some_and(|w| {
        let (x, y) = (w.x / frame.width as f64, w.y / frame.height as f64);
        x >= box_[0] && y >= box_[1] && x <= box_[2] && y <= box_[3]
    }))
}

/// The selection screen's name list, or the creation screen's name field, when `full`
/// shows that screen: the part read a second time (`Reader::frames`' focused frame).
pub(super) fn focus_of(full: &Frame) -> Option<[f64; 4]> {
    // Create and Delete side by side are the selection screen; Play, gold on an ornate button,
    // is the one tesseract misses (the owner's Linux frames, 2026-11-13).
    if anchor(full, "CREATE", [0.78, 0.70, 0.90, 0.85]) && anchor(full, "DELETE", [0.88, 0.70, 1.0, 0.85]) {
        Some(SELECTION_FOCUS)
    } else if anchor(full, "CONTINUE", [0.40, 0.82, 0.65, 0.98]) { Some(CREATION_FOCUS) } else { None }
}
pub(super) const SELECTION_FOCUS: [f64; 4] = [0.788, 0.205, 0.202, 0.520];
pub(super) const CREATION_FOCUS: [f64; 4] = [0.397, 0.723, 0.210, 0.110];

/// A platform's OCR: the whole frame and a second, focused read (see `focus_of`).
pub(super) trait Reader {
    fn frames(&self, image: &Image) -> Result<(Frame, Frame), String>;
}

/// Starts reading: inputs come through `request`, are captured by a worker (`foreground`
/// tells whether the game is in front; `capture` takes the game's frame) and read here, on
/// the calling thread, with `reader`. Returns only when the input channel closes.
pub(super) fn run(app: AppHandle, reader: &dyn Reader, foreground: fn() -> bool, capture: fn() -> Result<Image, String>) {
    let (sender, receiver) = mpsc::sync_channel::<Trigger>(64);
    if REQUESTS.set(sender).is_err() { return; }
    let pending: Arc<(Mutex<VecDeque<Job>>, Condvar)> = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
    let queue = pending.clone();
    let capture_app = app.clone();
    std::thread::spawn(move || capture_loop(capture_app, receiver, queue, foreground, capture));
    loop {
        let job = {
            let (lock, wake) = &*pending;
            let mut slot = crate::relock(lock);
            while slot.is_empty() { slot = wake.wait(slot).unwrap_or_else(|e| e.into_inner()); }
            slot.pop_front().unwrap()
        };
        {
            let tracker = app.state::<crate::tracker::Tracker>();
            if !crate::relock(&tracker.0).identity.current(job.epoch, job.seq) {
                super::trace("frame-rejected-before-ocr",serde_json::json!({"epoch":job.epoch,"seq":job.seq,"latest":latest_input()})); continue;
            }
        }
        let started = std::time::Instant::now();
        match reader.frames(&job.image) {
            Ok((full, focused)) => {
                let full_summary = frame_summary(&full);
                let menu = full_summary["fields"].as_array().is_some_and(|f| !f.is_empty());
                super::trace("ocr-result",serde_json::json!({"epoch":job.epoch,"seq":job.seq,"elapsedMs":started.elapsed().as_millis(),"full":full_summary,"focused":frame_summary_in(&focused, menu)}));
                let tracker = app.state::<crate::tracker::Tracker>();
                let inner = crate::relock(&tracker.0);
                if !inner.identity.current(job.epoch, job.seq) {
                    super::trace("frame-rejected-after-ocr",serde_json::json!({"epoch":job.epoch,"seq":job.seq,"latest":latest_input(),"currentEpoch":inner.identity.epoch,"currentSeq":inner.identity.seq})); continue;
                }
                let known = inner.characters.known.clone();
                drop(inner);
                let emitted = app.emit("identity-frame", Batch { epoch: job.epoch, seq: job.seq, full, focused, known, dry_run:false, click: job.click });
                super::trace("frame-emitted",serde_json::json!({"seq":job.seq,"error":emitted.err().map(|e|e.to_string())}));
            }
            Err(error) => {
                eprintln!("Character OCR: {error}");
                super::trace("ocr-failed",serde_json::json!({"seq":job.seq,"error":error}));
                report(&app, "Character could not be read; uncover the name or choose it in settings");
            }
        }
    }
}

fn capture_loop(app: AppHandle, receiver: Receiver<Trigger>, queue: Arc<(Mutex<VecDeque<Job>>, Condvar)>,
                foreground: fn() -> bool, capture: fn() -> Result<Image, String>) {
    let mut carry: Option<Trigger> = None;
    // A selection click directly followed by its activation (a row
    // double-click, or a quick Play) is captured first: the game is
    // already loading by the time the activation's own frame exists.
    let take = |current: &mut Trigger, newer: Trigger, carry: &mut Option<Trigger>| -> bool {
        if newer.activation && !current.activation && newer.seq == current.seq + 1 {
            *carry = Some(newer); true
        } else { *current = newer; false }
    };
    loop {
        let Some(mut trigger) = carry.take().or_else(|| receiver.recv().ok()) else { break };
        if trigger.delay > 0 {
            while let Ok(newer) = receiver.recv_timeout(Duration::from_millis(trigger.delay)) {
                if take(&mut trigger, newer, &mut carry) || trigger.delay == 0 { break; }
            }
        }
        if carry.is_none() { while let Ok(newer) = receiver.try_recv() { if take(&mut trigger, newer, &mut carry) { break; } } }
        // A mouse-down can focus the game and activate Continue in one
        // action. Wait for OS focus metadata, without sending input or
        // taking extra screenshots, before its single requested capture.
        if trigger.focus_click {
            let deadline = std::time::Instant::now()+Duration::from_millis(150);
            while !foreground() && std::time::Instant::now()<deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let in_front = foreground();
        super::trace("input",serde_json::json!({"seq":trigger.seq,"latest":latest_input(),"activation":trigger.activation,"focusClick":trigger.focus_click,"click":trigger.click,"delayMs":trigger.delay,"foreground":in_front,"inZone":IN_ZONE.load(Ordering::SeqCst),"bound":BOUND.load(Ordering::SeqCst)}));
        if trigger.seq != (if trigger.activation { latest_input() } else { superseding_input() }) || !in_front { continue; }
        let epoch = {
            let tracker = app.state::<crate::tracker::Tracker>();
            let mut inner = crate::relock(&tracker.0);
            let in_zone = inner.in_zone;
            let epoch = inner.identity.begin(trigger.seq, in_zone, trigger.activation);
            let _ = app.emit("tracker-state", inner.view());
            epoch
        };
        match capture() {
            Ok(image) => {
                let nonblack = image.pixels.chunks_exact(3).filter(|p|p.iter().any(|&v|v>8)).count();
                super::trace("captured",serde_json::json!({"epoch":epoch,"seq":trigger.seq,"width":image.width,"height":image.height,"nonblackFraction":nonblack as f64/(image.width*image.height).max(1) as f64}));
                let (lock, wake) = &*queue;
                let mut jobs = crate::relock(lock);
                if !trigger.activation { jobs.clear(); }
                // Keep a final edit's frame while Continue begins to fade.
                jobs.push_back(Job { epoch, seq: trigger.seq, image, click: trigger.click });
                while jobs.len() > 2 { jobs.pop_front(); }
                wake.notify_one();
            }
            Err(error) => {
                super::trace("capture-failed",serde_json::json!({"seq":trigger.seq,"error":error}));
                report(&app,&format!("Character screen capture paused: {error}"));
            }
        }
        // One Escape-menu click probes for a return to selection. Going
        // back to play must not leave OCR armed for every combat click.
        if IN_ZONE.load(Ordering::SeqCst) && trigger.delay == 180 {
            ARMED.store(false, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_second_row_click_soon_and_close_by_is_a_double_click() {
        let slop = (4, 4);
        let at = pack_point(-1700, 300); // a monitor left of the primary one
        assert!(second_click(1000, at, 1400, -1699, 301, 500, slop));
        assert!(!second_click(1000, at, 1600, -1699, 301, 500, slop), "too late");
        assert!(!second_click(1000, at, 1400, -1690, 301, 500, slop), "another row or spot");
        assert!(!second_click(1000, NO_CLICK, 1400, -1700, 300, 500, slop), "no first click");
        assert!(second_click(u32::MAX as u64 - 100, at, 200, -1700, 300, 500, slop), "tick count wrapped");
        assert!(in_character_list(0.85, 0.40));
        assert!(!in_character_list(0.45, 0.90), "Play is its own activation");
        assert!(on_play(0.45, 0.90));
    }
}
