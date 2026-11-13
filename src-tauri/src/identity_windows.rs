//! Windows: passive low-level input hooks observe real input; Windows.Media.Ocr reads the
//! frames. The shared part (`identity_native.rs`) captures and queues them.
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Emitter};
use crate::minimap::Image;
use super::shared::{self, focus_of, report, request, style, Batch, Frame, Line, Reader, Word, ARMED, BOUND, IN_ZONE, NO_CLICK, CREATION_FOCUS};
pub use super::shared::{finish_unknown, latest_input, set_context, superseding_input};
use windows::{core::HSTRING, Globalization::Language, Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine, Storage::Streams::DataWriter, Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED}};
use windows_sys::Win32::{Foundation::{GetLastError, HWND, LPARAM, LRESULT, WPARAM, RECT}, System::LibraryLoader::GetModuleHandleW,
    Graphics::Gdi::ScreenToClient, UI::Input::KeyboardAndMouse::GetDoubleClickTime,
    UI::WindowsAndMessaging::{CallNextHookEx, GetForegroundWindow, GetMessageW, GetSystemMetrics, InternalGetWindowText,
        SetWindowsHookExW, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WH_MOUSE_LL,
        SM_CXDOUBLECLK, SM_CYDOUBLECLK,
        WM_KEYDOWN, WM_SYSKEYDOWN, WM_KEYUP, WM_SYSKEYUP, WM_LBUTTONDOWN, GetClientRect, WindowFromPoint, GetAncestor, GA_ROOT}};

static MODIFIERS: AtomicU64 = AtomicU64::new(0);
// Double-clicking a character row enters the game like Play does. The last
// row click (hook time in ms, and its screen point packed as two i32) is the
// first half of a possible double-click; `NO_CLICK` means there is none.
static LAST_CLICK_MS: AtomicU64 = AtomicU64::new(0);
static LAST_CLICK_AT: AtomicU64 = AtomicU64::new(NO_CLICK);

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
                    let menu = shared::menu_input();
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
        if event.flags & 1 == 0 && game_title(window) && (shared::menu_input() || ARMED.load(Ordering::SeqCst)) {
            // Mouse-down is observed before the game handles Continue/Play. A worker
            // takes the screenshot; this callback never waits for capture or OCR.
            let in_zone = IN_ZONE.load(Ordering::SeqCst);
            let mut point = event.pt;
            let mut rect = RECT::default();
            let focus_click = GetForegroundWindow() != window;
            let at = (ScreenToClient(window, &mut point) != 0 &&
                GetClientRect(window, &mut rect) != 0 && rect.right > 0 && rect.bottom > 0)
                .then(|| (point.x as f64 / rect.right as f64, point.y as f64 / rect.bottom as f64));
            let play = at.is_some_and(|(fx, fy)| shared::on_play(fx, fy));
            // A character row: its second click within the double-click time
            // enters the game. A third click is a new first click.
            let row = !in_zone && at.is_some_and(|(fx, fy)| shared::in_character_list(fx, fy));
            let double = row && shared::second_click(LAST_CLICK_MS.load(Ordering::SeqCst), LAST_CLICK_AT.load(Ordering::SeqCst),
                event.time as u64, event.pt.x, event.pt.y, GetDoubleClickTime() as u64,
                (GetSystemMetrics(SM_CXDOUBLECLK), GetSystemMetrics(SM_CYDOUBLECLK)));
            LAST_CLICK_MS.store(event.time as u64, Ordering::SeqCst);
            LAST_CLICK_AT.store(if row && !double { shared::pack_point(event.pt.x, event.pt.y) } else { NO_CLICK }, Ordering::SeqCst);
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

fn ocr_engine() -> windows::core::Result<OcrEngine> {
    unsafe { RoInitialize(RO_INIT_MULTITHREADED)?; }
    OcrEngine::TryCreateFromLanguage(&Language::CreateLanguage(&HSTRING::from("en-US"))?)
}

/// Exercise native OCR -> actual webview listener -> IPC without touching profiles.
pub fn reference_probe(app: AppHandle) {
    let Some(path) = std::env::var_os("BATONGA_IDENTITY_REFERENCE") else { return };
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let engine = WindowsOcr(ocr_engine().map_err(|e|e.to_string())?);
            let image = crate::minimap::parse_ppm(&std::fs::read(path).map_err(|e|e.to_string())?)?;
            let (full,focused) = engine.frames(&image)?;
            super::trace("reference-emitted", shared::frame_summary(&full));
            app.emit("identity-frame", Batch {epoch:0,seq:0,full,focused,known:vec![],dry_run:true,click:None}).map_err(|e|e.to_string())
        })();
        if let Err(error) = result {super::trace("reference-failed",serde_json::json!({"error":error}));}
    });
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

struct WindowsOcr(OcrEngine);

impl Reader for WindowsOcr {
    fn frames(&self, image: &Image) -> Result<(Frame, Frame), String> {
        frames(&self.0, image).map_err(|e| e.to_string())
    }
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
    let crop = focus_of(&full);
    let cleaned;
    let image = if crop==Some(CREATION_FOCUS) {
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
        let engine = match ocr_engine() { Ok(engine) => WindowsOcr(engine), Err(error) => {
            eprintln!("Character OCR unavailable: {error}");
            report(&app, "Character recognition is unavailable; choose your character in settings"); return;
        }};
        super::trace("ocr-ready", serde_json::json!({"language":"en-US"}));
        report(&app, "Ready to read the character creation or selection screen");
        let hook_app = app.clone();
        // The hooks' requests only queue once `run` has set up the channel; an input in
        // these first microseconds is not read, as before.
        std::thread::spawn(move || hooks(hook_app));
        shared::run(app, &engine, game_foreground, crate::minimap::capture_identity_frame);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::shared::anchor;
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
        let engine = ocr_engine().unwrap();
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
