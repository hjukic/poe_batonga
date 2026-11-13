//! One screenshot per real user input. No timers that read the screen, no injected input.
use std::sync::{atomic::{AtomicBool, AtomicU64, Ordering}, mpsc::{self, SyncSender}, Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;
use std::collections::VecDeque;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use crate::minimap::Image;
use windows::{core::HSTRING, Globalization::Language, Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine, Storage::Streams::DataWriter, Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED}};
use windows_sys::Win32::{Foundation::{GetLastError, HWND, LPARAM, LRESULT, WPARAM, RECT}, System::LibraryLoader::GetModuleHandleW,
    Graphics::Gdi::ScreenToClient, UI::Input::KeyboardAndMouse::GetDoubleClickTime,
    UI::WindowsAndMessaging::{CallNextHookEx, GetForegroundWindow, GetMessageW, GetSystemMetrics, InternalGetWindowText,
        SetWindowsHookExW, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WH_MOUSE_LL,
        SM_CXDOUBLECLK, SM_CYDOUBLECLK,
        WM_KEYDOWN, WM_SYSKEYDOWN, WM_KEYUP, WM_SYSKEYUP, WM_LBUTTONDOWN, GetClientRect, WindowFromPoint, GetAncestor, GA_ROOT}};

static SEQ: AtomicU64 = AtomicU64::new(0);
static IN_ZONE: AtomicBool = AtomicBool::new(false);
static BOUND: AtomicBool = AtomicBool::new(false);
static ARMED: AtomicBool = AtomicBool::new(false);
static RECOVERY_PROBE: AtomicBool = AtomicBool::new(true);
static MODIFIERS: AtomicU64 = AtomicU64::new(0);
static REQUESTS: OnceLock<SyncSender<Trigger>> = OnceLock::new();
// Double-clicking a character row enters the game like Play does. The last
// row click (hook time in ms, and its screen point packed as two i32) is the
// first half of a possible double-click; `NO_CLICK` means there is none.
const NO_CLICK: u64 = u64::MAX;
static LAST_CLICK_MS: AtomicU64 = AtomicU64::new(0);
static LAST_CLICK_AT: AtomicU64 = AtomicU64::new(NO_CLICK);
/// Seq of the latest input that starts the game (Play/Continue, a row double-click).
static LAST_ACTIVATION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
/// `click`: a row double-click's point (fractions of the client area): the row it enters.
struct Trigger { seq: u64, delay: u64, activation: bool, focus_click: bool, click: Option<[f64; 2]> }
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

fn menu_input() -> bool {
    !IN_ZONE.load(Ordering::SeqCst) ||
        (!BOUND.load(Ordering::SeqCst) && RECOVERY_PROBE.load(Ordering::SeqCst))
}

fn game_foreground() -> bool {
    game_title(unsafe { GetForegroundWindow() })
}

fn game_title(window: HWND) -> bool {
    unsafe {
        if window.is_null() { return false; }
        let mut text = [0u16; 128];
        // Not GetWindowTextW: for the overlay's own windows that sends WM_GETTEXT
        // to a busy UI thread from inside the low-level hook (global input lag,
        // and past LowLevelHooksTimeout Windows silently removes the hook).
        let len = InternalGetWindowText(window, text.as_mut_ptr(), text.len() as i32);
        len > 0 && String::from_utf16_lossy(&text[..len as usize]) == "Path of Exile 2"
    }
}

fn request(delay: u64, activation: bool, focus_click: bool, click: Option<[f64; 2]>) {
    let seq = SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    // Before the worker sees it: the selection frame just before an activation
    // must not count as superseded (see `superseding_input`).
    if activation { LAST_ACTIVATION.store(seq, Ordering::SeqCst); }
    if let Some(sender) = REQUESTS.get() { let _ = sender.try_send(Trigger { seq, delay, activation, focus_click, click }); }
}

fn pack_point(x: i32, y: i32) -> u64 { ((x as u32 as u64) << 32) | y as u32 as u64 }

/// The second click of a double-click: soon enough after the previous row click
/// and within the system's double-click rectangle around it.
fn second_click(previous_ms: u64, previous_at: u64, now_ms: u64, x: i32, y: i32, max_ms: u64, slop: (i32, i32)) -> bool {
    if previous_at == NO_CLICK { return false; }
    let (px, py) = ((previous_at >> 32) as u32 as i32, previous_at as u32 as i32);
    // Hook times are 32-bit tick counts that wrap after 49.7 days.
    ((now_ms as u32).wrapping_sub(previous_ms as u32) as u64) <= max_ms &&
        (x - px).abs() <= slop.0 / 2 && (y - py).abs() <= slop.1 / 2
}

/// A click in the character list on the selection screen (client fractions).
fn in_character_list(fx: f64, fy: f64) -> bool {
    (0.79..=0.99).contains(&fx) && (0.215..=0.73).contains(&fy)
}

unsafe extern "system" fn keyboard(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code >= 0 {
        let event = &*(data as *const KBDLLHOOKSTRUCT);
        if event.flags & 0x10 == 0 { // Ignore injected keys. Never retain the typed key/text.
            let down = message as u32 == WM_KEYDOWN || message as u32 == WM_SYSKEYDOWN;
            let up = message as u32 == WM_KEYUP || message as u32 == WM_SYSKEYUP;
            let modifier = match event.vkCode {
                0x5b => 1, 0x5c => 2, 0x11|0xa2 => 4, 0xa3 => 8,
                0x12|0xa4 => 16, 0xa5 => 32, _ => 0,
            };
            if modifier != 0 {
                if down { MODIFIERS.fetch_or(modifier,Ordering::SeqCst); }
                else if up { MODIFIERS.fetch_and(!modifier,Ordering::SeqCst); }
            } else if down && game_foreground() {
                let held = MODIFIERS.load(Ordering::SeqCst);
                if name_input_key(event.vkCode, held) {
                    let menu = menu_input();
                    match event.vkCode {
                        0x1b => { ARMED.fetch_xor(true, Ordering::SeqCst); request(120, false, false, None); }
                        0x43 if !BOUND.load(Ordering::SeqCst) => request(100, false, false, None),
                        0x0d if menu => request(0, true, false, None),
                        _ if menu => request(65, false, false, None),
                        _ => {}
                    }
                }
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, message, data)
}

fn name_input_key(key: u32, modifiers: u64) -> bool {
    // Taking a screenshot, Alt+Tab or pressing a modifier doesn't edit the name.
    if modifiers & (1|2|16|32) != 0 { return false; }
    if modifiers & (4|8) != 0 { return matches!(key,0x56|0x58|0x59|0x5a); } // paste/cut/redo/undo
    matches!(key,0x08|0x09|0x0d|0x1b|0x21..=0x28|0x2e|0x30..=0x39|0x41..=0x5a|0xbd)
}

unsafe extern "system" fn mouse(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code >= 0 && message as u32 == WM_LBUTTONDOWN {
        let event = &*(data as *const MSLLHOOKSTRUCT);
        let window = GetAncestor(WindowFromPoint(event.pt),GA_ROOT);
        if event.flags & 1 == 0 && game_title(window) && (menu_input() || ARMED.load(Ordering::SeqCst)) {
            // Mouse-down is observed before the game handles Continue/Play. A worker
            // takes the screenshot; this callback never waits for capture or OCR.
            let in_zone = IN_ZONE.load(Ordering::SeqCst);
            let mut point = event.pt;
            let mut rect = RECT::default();
            let focus_click = GetForegroundWindow() != window;
            let at = (ScreenToClient(window, &mut point) != 0 &&
                GetClientRect(window, &mut rect) != 0 && rect.right > 0 && rect.bottom > 0)
                .then(|| (point.x as f64 / rect.right as f64, point.y as f64 / rect.bottom as f64));
            let play = at.is_some_and(|(fx, fy)| (0.28..=0.62).contains(&fx) && (0.87..=0.95).contains(&fy));
            // A character row: its second click within the double-click time
            // enters the game. A third click is a new first click.
            let row = !in_zone && at.is_some_and(|(fx, fy)| in_character_list(fx, fy));
            let double = row && second_click(LAST_CLICK_MS.load(Ordering::SeqCst), LAST_CLICK_AT.load(Ordering::SeqCst),
                event.time as u64, event.pt.x, event.pt.y, GetDoubleClickTime() as u64,
                (GetSystemMetrics(SM_CXDOUBLECLK), GetSystemMetrics(SM_CYDOUBLECLK)));
            LAST_CLICK_MS.store(event.time as u64, Ordering::SeqCst);
            LAST_CLICK_AT.store(if row && !double { pack_point(event.pt.x, event.pt.y) } else { NO_CLICK }, Ordering::SeqCst);
            let activation = play || double;
            // Selection/edit clicks need a rendered frame; activation needs the
            // preceding field before the transition. OCR never blocks the hook.
            request(if activation { 0 } else if in_zone { 180 } else { 80 }, activation, focus_click,
                at.filter(|_| double).map(|(fx, fy)| [fx, fy]));
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, message, data)
}

fn hooks(app: AppHandle) {
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        let keys = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), module, 0);
        let clicks = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), module, 0);
        super::trace("hooks-ready", serde_json::json!({"keyboard":!keys.is_null(),"mouse":!clicks.is_null(),"error":if keys.is_null() || clicks.is_null() {GetLastError()} else {0}}));
        if keys.is_null() || clicks.is_null() {
            report(&app, "Character recognition could not observe game input; use Settings → Character");
            if !keys.is_null() { UnhookWindowsHookEx(keys); }
            if !clicks.is_null() { UnhookWindowsHookEx(clicks); }
            return;
        }
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {}
        UnhookWindowsHookEx(keys);
        UnhookWindowsHookEx(clicks);
    }
}

fn report(app: &AppHandle, message: &str) {
    super::trace("status", serde_json::json!({"message":message}));
    let tracker = app.state::<crate::tracker::Tracker>();
    let mut inner = crate::relock(&tracker.0);
    inner.identity.note = message.into();
    let _ = app.emit("tracker-state", inner.view());
}

#[derive(Clone, Serialize)]
struct Word { text: String, x: f64, y: f64, width: f64, height: f64 }
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Style { gold_pixels: usize, gold_mean: f64, bright_fraction: f64 }
#[derive(Clone, Serialize)]
struct Line { text: String, words: Vec<Word>, style: Option<Style> }
#[derive(Clone, Serialize)]
struct Frame { width: usize, height: usize, lines: Vec<Line> }
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Batch { epoch: u64, seq: u64, full: Frame, focused: Frame, known: Vec<super::KnownCharacter>, dry_run: bool, click: Option<[f64; 2]> }
struct Job { epoch: u64, seq: u64, image: Image, click: Option<[f64; 2]> }

fn frame_summary(frame: &Frame) -> serde_json::Value { frame_summary_in(frame, false) }

/// `menu`: the full frame already showed a menu (the focused crop has no anchors).
fn frame_summary_in(frame: &Frame, menu: bool) -> serde_json::Value {
    let anchors: Vec<_> = frame.lines.iter().filter(|l| matches!(l.text.trim().to_uppercase().as_str(),
        "PLAY"|"CONTINUE"|"CREATE"|"DELETE"|"DRUID"|"WARRIOR"|"WITCH"|"MONK"|"SORCERESS"|"RANGER"|"MERCENARY"|"HUNTRESS"))
        .map(|l| serde_json::json!({"text":l.text,"at":l.words.first().map(|w|[w.x,w.y])})).collect();
    // Keep text only from the character name/league fields on a known menu.
    let menu = menu || anchors.iter().any(|a| a["text"].as_str().is_some_and(|t| matches!(t.to_uppercase().as_str(), "PLAY"|"CONTINUE")));
    let fields: Vec<_> = if menu { frame.lines.iter().filter(|l| l.words.first().is_some_and(|w| {
        let (x,y) = (w.x / frame.width as f64,w.y / frame.height as f64);
        ((0.39..0.68).contains(&x) && (0.72..0.84).contains(&y)) || ((0.74..0.99).contains(&x) && (0.12..0.82).contains(&y))
    })).map(|l| serde_json::json!({"text":l.text,"at":l.words.first().map(|w|[w.x,w.y]),"w":l.words.first().map(|w|w.width),"words":l.words.len(),"style":l.style})).collect() } else { vec![] };
    serde_json::json!({"width":frame.width,"height":frame.height,"lines":frame.lines.len(),"anchors":anchors,"fields":fields})
}

/// Exercise native OCR -> actual webview listener -> IPC without touching profiles.
pub fn reference_probe(app: AppHandle) {
    let Some(path) = std::env::var_os("BATONGA_IDENTITY_REFERENCE") else { return };
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            unsafe { RoInitialize(RO_INIT_MULTITHREADED).map_err(|e|e.to_string())?; }
            let engine = OcrEngine::TryCreateFromLanguage(&Language::CreateLanguage(&HSTRING::from("en-US")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            let image = crate::minimap::parse_ppm(&std::fs::read(path).map_err(|e|e.to_string())?)?;
            let (full,focused) = frames(&engine,&image).map_err(|e|e.to_string())?;
            super::trace("reference-emitted", frame_summary(&full));
            app.emit("identity-frame", Batch {epoch:0,seq:0,full,focused,known:vec![],dry_run:true,click:None}).map_err(|e|e.to_string())
        })();
        if let Err(error) = result {super::trace("reference-failed",serde_json::json!({"error":error}));}
    });
}

fn style(image: &Image, word: &Word) -> Option<Style> {
    let x = word.x / image.width as f64;
    let y = word.y / image.height as f64;
    // Wider than the name column: a turned screen (up to ~10°) moves the titles.
    if !(0.74..=0.88).contains(&x) || !(0.12..=0.82).contains(&y) { return None; }
    let (mut count, mut total, mut bright) = (0, 0.0, 0);
    for py in word.y as usize..((word.y + word.height) as usize).min(image.height) {
        for px in word.x as usize..((word.x + word.width) as usize).min(image.width) {
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

fn read(engine: &OcrEngine, image: &Image, rect: [f64; 4]) -> windows::core::Result<Frame> {
    let x = (image.width as f64 * rect[0]).floor() as usize;
    let y = (image.height as f64 * rect[1]).floor() as usize;
    let width = ((image.width as f64 * rect[2]) as usize).min(image.width - x);
    let height = ((image.height as f64 * rect[3]) as usize).min(image.height - y);
    let mut bgra = Vec::with_capacity(width * height * 4);
    for py in y..y + height { for px in x..x + width {
        let at = (py * image.width + px) * 3;
        bgra.extend_from_slice(&[image.pixels[at + 2], image.pixels[at + 1], image.pixels[at], 255]);
    }}
    let writer = DataWriter::new()?;
    writer.WriteBytes(&bgra)?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(&writer.DetachBuffer()?, BitmapPixelFormat::Bgra8, width as i32, height as i32)?;
    let result = engine.RecognizeAsync(&bitmap)?.join()?;
    let mut lines = Vec::new();
    for line in result.Lines()? {
        let mut words = Vec::new();
        for word in line.Words()? {
            let bounds = word.BoundingRect()?;
            words.push(Word { text: word.Text()?.to_string(), x: x as f64 + bounds.X as f64, y: y as f64 + bounds.Y as f64,
                width: bounds.Width as f64, height: bounds.Height as f64 });
        }
        let color = words.first().and_then(|word| style(image, word));
        lines.push(Line { text: line.Text()?.to_string(), words, style: color });
    }
    Ok(Frame { width: image.width, height: image.height, lines })
}

fn anchor(frame: &Frame, text: &str, box_: [f64; 4]) -> bool {
    frame.lines.iter().any(|line| line.text.trim().eq_ignore_ascii_case(text) && line.words.first().is_some_and(|w| {
        let (x, y) = (w.x / frame.width as f64, w.y / frame.height as f64);
        x >= box_[0] && y >= box_[1] && x <= box_[2] && y <= box_[3]
    }))
}

/// The input caret rises above the font and is taller than a real I. Mask only
/// such narrow pixel bars, never remove a trailing letter from the OCR string.
fn without_creation_caret(image: &Image) -> Option<Image> {
    let x0 = (image.width as f64 * 0.410) as usize;
    let x1 = (image.width as f64 * 0.594) as usize;
    let y0 = (image.height as f64 * 0.738) as usize;
    let y1 = (image.height as f64 * 0.768) as usize;
    let min_height = (image.height as f64 * 0.022).ceil() as usize;
    let before_font = (image.height as f64 * 0.742) as usize;
    let max_width = (image.width as f64 * 0.0025).ceil() as usize;
    let bright = |x: usize,y: usize| {
        let at=(y*image.width+x)*3;
        image.pixels[at]>=140 && image.pixels[at+1]>=100
    };
    let mut columns = Vec::new();
    for x in x0..x1 {
        let mut start=y0;
        for y in y0..=y1 {
            if y<y1 && bright(x,y) {continue;}
            if y-start>=min_height && start<before_font {columns.push((x,start,y));break;}
            start=y+1;
        }
    }
    if columns.is_empty() {return None;}
    let mut cleaned=image.clone();
    let mut removed=0;
    let mut at=0;
    while at<columns.len() {
        let first=at;
        while at+1<columns.len() && columns[at+1].0==columns[at].0+1 {at+=1;}
        let (left,right)=(columns[first].0,columns[at].0);
        if right-left<max_width && left>x0 && right+2<x1 {
            let top=columns[first..=at].iter().map(|c|c.1).min().unwrap();
            let bottom=columns[first..=at].iter().map(|c|c.2).max().unwrap();
            for y in top..bottom {for x in left..=right {
                let l=(y*image.width+left-2)*3;
                let r=(y*image.width+right+2)*3;
                let source=if (image.pixels[l] as u32+image.pixels[l+1] as u32) < (image.pixels[r] as u32+image.pixels[r+1] as u32) {l} else {r};
                let dest=(y*image.width+x)*3;
                cleaned.pixels[dest..dest+3].copy_from_slice(&image.pixels[source..source+3]);
            }}
            removed+=1;
        }
        at+=1;
    }
    if removed>0 {
        super::trace("caret-masked",serde_json::json!({"bars":removed}));
        Some(cleaned)
    } else {None}
}

fn frames(engine: &OcrEngine, image: &Image) -> windows::core::Result<(Frame, Frame)> {
    let max = OcrEngine::MaxImageDimension()? as usize;
    let scaled;
    let image = if image.width.max(image.height) > max {
        let factor = max as f64 / image.width.max(image.height) as f64;
        let (w, h) = ((image.width as f64 * factor) as usize, (image.height as f64 * factor) as usize);
        let mut pixels = Vec::with_capacity(w * h * 3);
        for y in 0..h { for x in 0..w {
            let at = (((y as f64 / factor) as usize) * image.width + (x as f64 / factor) as usize) * 3;
            pixels.extend_from_slice(&image.pixels[at..at + 3]);
        }}
        scaled = Image { width: w, height: h, pixels };
        &scaled
    } else { image };
    let mut full = read(engine, image, [0.0, 0.0, 1.0, 1.0])?;
    let crop = if anchor(&full, "PLAY", [0.25, 0.82, 0.60, 0.98]) && anchor(&full, "CREATE", [0.78, 0.70, 0.90, 0.85]) && anchor(&full, "DELETE", [0.88, 0.70, 1.0, 0.85]) {
        Some([0.788, 0.205, 0.202, 0.520])
    } else if anchor(&full, "CONTINUE", [0.40, 0.82, 0.65, 0.98]) { Some([0.397, 0.723, 0.210, 0.110]) } else { None };
    let cleaned;
    let image = if crop==Some([0.397,0.723,0.210,0.110]) {
        if let Some(without_caret)=without_creation_caret(image) {
            cleaned=without_caret;
            full=read(engine,&cleaned,[0.0,0.0,1.0,1.0])?;
            &cleaned
        } else {image}
    } else {image};
    let focused = if let Some(crop) = crop { read(engine, image, crop)? } else { full.clone() };
    Ok((full, focused))
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let engine = (|| -> windows::core::Result<OcrEngine> {
            unsafe { RoInitialize(RO_INIT_MULTITHREADED)?; }
            OcrEngine::TryCreateFromLanguage(&Language::CreateLanguage(&HSTRING::from("en-US"))?)
        })();
        let engine = match engine { Ok(engine) => engine, Err(error) => {
            eprintln!("Character OCR unavailable: {error}");
            report(&app, "Character recognition is unavailable; choose your character in settings"); return;
        }};
        super::trace("ocr-ready", serde_json::json!({"language":"en-US"}));
        report(&app, "Ready to read the character creation or selection screen");
        let (sender, receiver) = mpsc::sync_channel::<Trigger>(64);
        if REQUESTS.set(sender).is_err() { return; }
        let pending: Arc<(Mutex<VecDeque<Job>>, Condvar)> = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
        let queue = pending.clone();
        let capture_app = app.clone();
        std::thread::spawn(move || {
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
                    while !game_foreground() && std::time::Instant::now()<deadline {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
                let foreground = game_foreground();
                super::trace("input",serde_json::json!({"seq":trigger.seq,"latest":latest_input(),"activation":trigger.activation,"focusClick":trigger.focus_click,"click":trigger.click,"delayMs":trigger.delay,"foreground":foreground,"inZone":IN_ZONE.load(Ordering::SeqCst),"bound":BOUND.load(Ordering::SeqCst)}));
                if trigger.seq != (if trigger.activation { latest_input() } else { superseding_input() }) || !foreground { continue; }
                let epoch = {
                    let tracker = capture_app.state::<crate::tracker::Tracker>();
                    let mut inner = crate::relock(&tracker.0);
                    let in_zone = inner.in_zone;
                    let epoch = inner.identity.begin(trigger.seq, in_zone, trigger.activation);
                    let _ = capture_app.emit("tracker-state", inner.view());
                    epoch
                };
                match crate::minimap::capture_identity_frame() {
                    Ok(image) => {
                        let nonblack = image.pixels.chunks_exact(3).filter(|p|p.iter().any(|&v|v>8)).count();
                        super::trace("captured",serde_json::json!({"epoch":epoch,"seq":trigger.seq,"width":image.width,"height":image.height,"nonblackFraction":nonblack as f64/(image.width*image.height) as f64}));
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
                        report(&capture_app,&format!("Character screen capture paused: {error}"));
                    }
                }
                // One Escape-menu click probes for a return to selection. Going
                // back to play must not leave OCR armed for every combat click.
                if IN_ZONE.load(Ordering::SeqCst) && trigger.delay == 180 {
                    ARMED.store(false, Ordering::SeqCst);
                }
            }
        });
        let hook_app = app.clone();
        std::thread::spawn(move || hooks(hook_app));
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
            match frames(&engine, &job.image) {
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
                Err(error) => { eprintln!("Character OCR: {error}"); report(&app, "Character could not be read; uncover the name or choose it in settings"); }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screenshot_shortcuts_and_focus_modifiers_do_not_invalidate_names() {
        assert!(!name_input_key(0x53,1)); // Win+Shift+S
        assert!(!name_input_key(0x09,16)); // Alt+Tab
        assert!(!name_input_key(0x2c,0)); // PrintScreen
        assert!(!name_input_key(0x10,0)); // Shift
        assert!(name_input_key(0x56,4)); // Ctrl+V edits the field
        assert!(name_input_key(0x0d,0)); // Enter activates
        assert!(name_input_key(0x49,0)); // A real I remains a letter
    }
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
    }
    #[test]
    fn caret_mask_preserves_real_letter_height_and_cleans_only_the_tall_bar() {
        let mut image=Image {width:2560,height:1440,pixels:vec![0;2560*1440*3]};
        for y in 1071..1095 {for x in 1100..1103 {let at=(y*2560+x)*3;image.pixels[at..at+3].copy_from_slice(&[230,200,140]);}}
        assert!(without_creation_caret(&image).is_none(),"a real I is not a caret");
        for y in 1066..1103 {for x in 1200..1202 {let at=(y*2560+x)*3;image.pixels[at..at+3].copy_from_slice(&[230,200,140]);}}
        let clean=without_creation_caret(&image).expect("tall caret detected");
        assert_eq!(clean.pixels[(1080*2560+1101)*3],230,"keep a legitimate I");
        assert_eq!(clean.pixels[(1080*2560+1201)*3],0,"remove only the cursor bar");
    }
    #[test]
    #[ignore = "Reads an explicitly supplied local PPM reference; no live capture"]
    fn supplied_reference() {
        unsafe { RoInitialize(RO_INIT_MULTITHREADED).unwrap(); }
        let engine = OcrEngine::TryCreateFromLanguage(&Language::CreateLanguage(&HSTRING::from("en-US")).unwrap()).unwrap();
        let path = std::env::var("BATONGA_IDENTITY_PPM").expect("Set BATONGA_IDENTITY_PPM to a reference PPM");
        let image = crate::minimap::parse_ppm(&std::fs::read(path).unwrap()).unwrap();
        let (full, focused) = frames(&engine, &image).unwrap();
        if anchor(&full,"CONTINUE",[0.40,0.82,0.65,0.98]) {
            let name = |frame: &Frame| frame.lines.iter().find(|l|l.words.first().is_some_and(|w|
                (0.405..0.60).contains(&(w.x/frame.width as f64)) && (0.73..0.78).contains(&(w.y/frame.height as f64)))).cloned();
            if let Some(original)=name(&focused) {
                let word=original.words.last().unwrap();
                let x=(word.x+word.width+5.0).ceil() as usize;
                let mut with_caret=image.clone();
                for y in (image.height as f64*0.7403) as usize..(image.height as f64*0.7660) as usize {
                    for px in x..x+2 {let at=(y*image.width+px)*3;with_caret.pixels[at..at+3].copy_from_slice(&[230,200,140]);}
                }
                assert!(without_creation_caret(&with_caret).is_some());
                let (new_full,new_focused)=frames(&engine,&with_caret).unwrap();
                assert_eq!(name(&new_full).unwrap().text,original.text,"full-frame OCR must not append the caret to the name");
                assert_eq!(name(&new_focused).unwrap().text,original.text,"crop OCR must agree after masking");
            }
        }
        println!("{}", serde_json::to_string(&Batch { epoch: 0, seq: 0, full, focused, known: vec![],dry_run:true,click:None }).unwrap());
    }
}
