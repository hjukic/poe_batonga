//! Windows: the tracker column captured with GDI (`minimap::capture_quest_tracker`, the
//! game in front and not covered) and read by Windows.Media.Ocr, about once a second,
//! only while a step that the tracker can tick is open.
use super::{read, Reading, Watcher};
use crate::minimap::Image;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use windows::{
    core::HSTRING,
    Globalization::Language,
    Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine,
    Storage::Streams::DataWriter,
    Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED},
};

const EVERY: Duration = Duration::from_millis(1000);

fn engine() -> windows::core::Result<OcrEngine> {
    unsafe {
        RoInitialize(RO_INIT_MULTITHREADED)?;
    }
    OcrEngine::TryCreateFromLanguage(&Language::CreateLanguage(&HSTRING::from("en-US"))?)
}

/// The OCR lines, top to bottom. A line the OCR runs across a wide gap (a ground label
/// beside the tracker) is split there.
fn ocr(engine: &OcrEngine, image: &Image) -> windows::core::Result<Vec<String>> {
    let max = OcrEngine::MaxImageDimension()? as usize;
    if image.width > max || image.height > max || image.width == 0 || image.height == 0 {
        return Ok(Vec::new());
    }
    let mut bgra = Vec::with_capacity(image.width * image.height * 4);
    for p in image.pixels.chunks_exact(3) {
        bgra.extend_from_slice(&[p[2], p[1], p[0], 255]);
    }
    let writer = DataWriter::new()?;
    writer.WriteBytes(&bgra)?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
        &writer.DetachBuffer()?,
        BitmapPixelFormat::Bgra8,
        image.width as i32,
        image.height as i32,
    )?;
    let result = engine.RecognizeAsync(&bitmap)?.join()?;
    let mut pieces: Vec<(f32, String)> = Vec::new();
    for line in result.Lines()? {
        let mut words: Vec<(f32, f32, f32, f32, String)> = Vec::new();
        for word in line.Words()? {
            let b = word.BoundingRect()?;
            words.push((b.X, b.Y, b.Width, b.Height, word.Text()?.to_string()));
        }
        let mut piece: Vec<&(f32, f32, f32, f32, String)> = Vec::new();
        for word in &words {
            if let Some(last) = piece.last() {
                let gap = word.0 - (last.0 + last.2);
                if gap > 2.5 * last.3.max(word.3) {
                    pieces.push((piece[0].1, piece.iter().map(|w| w.4.as_str()).collect::<Vec<_>>().join(" ")));
                    piece.clear();
                }
            }
            piece.push(word);
        }
        if !piece.is_empty() {
            pieces.push((piece[0].1, piece.iter().map(|w| w.4.as_str()).collect::<Vec<_>>().join(" ")));
        }
    }
    pieces.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(pieces.into_iter().map(|(_, text)| text).collect())
}

/// `BATONGA_QUEST_DIAGNOSTICS=<file>`: each new read (OCR lines, what they fit, what
/// was confirmed) as a JSON line, for a live test.
fn trace(lines: &[String], reading: &Reading, confirmed: &[super::Confirmed], last: &mut Vec<String>) {
    let Some(path) = std::env::var_os("BATONGA_QUEST_DIAGNOSTICS") else { return };
    if lines == last.as_slice() && confirmed.is_empty() {
        return;
    }
    *last = lines.to_vec();
    let quests: serde_json::Map<String, serde_json::Value> = reading
        .quests
        .iter()
        .map(|(q, s)| (q.clone(), serde_json::json!({ "orders": s.orders, "lines": s.lines })))
        .collect();
    let confirmed: Vec<_> = confirmed.iter().map(|c| serde_json::json!({ "quest": c.quest, "orders": c.orders })).collect();
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let entry = serde_json::json!({ "at": secs, "ocr": lines, "headers": reading.headers, "quests": quests, "confirmed": confirmed });
    let _ = std::fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut file| {
        use std::io::Write;
        writeln!(file, "{entry}")
    });
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let engine = match engine() {
            Ok(engine) => engine,
            Err(error) => {
                eprintln!("Quest tracker: no OCR engine: {error}");
                return;
            }
        };
        let mut watcher = Watcher::default();
        let mut character: Option<String> = None;
        let mut traced = Vec::new();
        loop {
            std::thread::sleep(EVERY);
            crate::guarded("Quest tracker: a read", || {
                let watching = crate::relock(&app.state::<crate::tracker::Tracker>().0).screen_watch();
                if watching != character {
                    watcher.reset();
                    character = watching.clone();
                }
                if watching.is_none() {
                    return;
                }
                // Not in front, covered or moved: no read this time.
                let Ok(image) = crate::minimap::capture_quest_tracker() else { return };
                let lines = match ocr(&engine, &image) {
                    Ok(lines) => lines,
                    Err(error) => {
                        eprintln!("Quest tracker: OCR failed: {error}");
                        return;
                    }
                };
                let reading = read(&lines);
                let confirmed = watcher.frame(&reading);
                trace(&lines, &reading, &confirmed, &mut traced);
                if confirmed.is_empty() {
                    return;
                }
                let tracker = app.state::<crate::tracker::Tracker>();
                let mut inner = crate::relock(&tracker.0);
                // The character may have changed while the frame was read.
                if inner.screen_watch() != character {
                    return;
                }
                let fighting = inner.fight.is_some();
                let mut changed = false;
                for c in &confirmed {
                    changed |= inner.apply_screen(&c.quest, &c.orders, &c.evidence);
                }
                if !changed {
                    return;
                }
                inner.save();
                inner.sync_navigation(&app);
                let _ = app.emit("tracker-state", inner.view());
                inner.settle_fight();
                let now = inner.fight.is_some();
                drop(inner);
                if now != fighting {
                    crate::set_fight(&app, now);
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    /// `BATONGA_QUEST_REFERENCE=<column.ppm> cargo test -- --ignored quest_reference --nocapture`:
    /// the OCR lines of a captured tracker column and what they read as.
    #[test]
    #[ignore]
    fn quest_reference() {
        let path = std::env::var_os("BATONGA_QUEST_REFERENCE").expect("BATONGA_QUEST_REFERENCE");
        let image = crate::minimap::parse_ppm(&std::fs::read(path).unwrap()).unwrap();
        let lines = super::ocr(&super::engine().unwrap(), &image).unwrap();
        println!("{lines:#?}");
        println!("{:#?}", super::read(&lines));
    }
}
