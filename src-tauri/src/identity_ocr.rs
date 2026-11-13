//! Reading the character screens on Linux: PaddleOCR's PP-OCRv6 models on ONNX Runtime, in
//! process (`paddle-ocr-rs`). One model finds the lines of text, another reads each line.
//! Each line found is cut to its letters' own pixels and split into words, so the shared
//! recognizer (`src/character-screen.mjs`) gets what Windows OCR gives it.
//!
//! tesseract read these screens from v0.1.112 to v0.1.116 and was removed: on the owner's
//! 3840x2160 selection screen it lost two of five names even one title row at a time, the
//! highlighted one among them (2026-11-13). These models read all five names, the level and
//! league lines and the three buttons on that screenshot, also made as small as 1920x1080
//! and 1280x720 once enlarged again (`scale_for`).
use std::collections::HashMap;
use paddle_ocr_rs::{base_net::BaseNet, crnn_net::CrnnNet, db_net::DbNet, ocr_utils::OcrUtils, scale_param::ScaleParam};
use crate::minimap::Image;
use super::shared::{style, Frame, Line, Word};

/// Finds the lines of text (1.8 MB). The larger detectors found the same lines more slowly
/// and, beside them, the row ornaments.
static DETECTOR: &[u8] = include_bytes!("../models/ocr/PP-OCRv6_det_tiny.onnx");
/// Reads a line (21 MB): the whole frame's read. Exact also on the small level and league lines.
static READER: &[u8] = include_bytes!("../models/ocr/PP-OCRv6_rec_small.onnx");
/// Reads a line (4.5 MB): the focused read. Another model, so that the two reads the
/// recognizer compares are independent, as Windows' whole frame and crop are. It read every
/// name too; on the small lines it slips ("LEVEL6 RANGER", "Standard Leagte").
static SECOND_READER: &[u8] = include_bytes!("../models/ocr/PP-OCRv6_rec_tiny.onnx");

/// Where the whole frame is read, as fractions of the game window: the selection screen's
/// right panel (character list, its tooltip, Create and Delete) and the bottom middle (Play,
/// Continue, the creation screen's name, league and class). Not all of the window: the rest
/// holds no menu text, and this app's own panel lies over the left.
pub const AREAS: [[f64; 4]; 2] = [[0.70, 0.10, 0.30, 0.72], [0.15, 0.64, 0.56, 0.34]];

/// One thread: the game comes first (tesseract's OpenMP build spread each run over every
/// core, v0.1.118). A read of the selection screen takes under a second on it.
const THREADS: usize = 1;
/// The white border the detector wants around an image, in pixels.
const PADDING: u32 = 50;
/// The detector looks at a crop no larger than this on its longer side: the lines are found
/// as well as at full size, in a third of the time. Each line is then read at full size.
const DETECT_SIDE: u32 = 960;
/// A line is kept when read at least this surely (0-1). Text read 0.89-1.0; ornaments the
/// detector took for text 0.51-0.59 or nothing at all.
const MIN_SCORE: f32 = 0.6;

/// Which of the two readers reads the lines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Read {
    Whole,
    Focused,
}

/// The loaded models: about 300 MB of memory while held (`identity_linux.rs` lets them go in
/// a zone).
pub struct Engine {
    detector: DbNet,
    reader: CrnnNet,
    second: CrnnNet,
}

/// How much a frame is enlarged before reading: menu text as high as on a 2160 px window,
/// where every line read right. At half that size one name lost a letter until enlarged.
pub fn scale_for(height: usize) -> f64 {
    (2160.0 / height.max(1) as f64).clamp(1.0, 3.0)
}

impl Engine {
    pub fn load() -> Result<Engine, String> {
        let failed = |what: &str, error: &dyn std::fmt::Display| format!("{what} model: {error}");
        let mut detector = DbNet::new();
        BaseNet::init_model_from_memory(&mut detector, DETECTOR, THREADS, None).map_err(|e| failed("text detection", &e))?;
        let mut reader = CrnnNet::new();
        reader.init_model_from_memory(READER, THREADS, None).map_err(|e| failed("text reading", &e))?;
        let mut second = CrnnNet::new();
        second.init_model_from_memory(SECOND_READER, THREADS, None).map_err(|e| failed("second text reading", &e))?;
        Ok(Engine { detector, reader, second })
    }

    /// The lines of text in `area` (fractions of `image`), read `scale` times enlarged; their
    /// places in `image` pixels.
    pub fn read(&mut self, image: &Image, area: [f64; 4], scale: f64, read: Read) -> Result<Vec<Line>, String> {
        let (cut, origin) = crop(image, area, scale);
        let padded = OcrUtils::make_padding(&cut, PADDING).map_err(|e| e.to_string())?;
        let side = cut.width().max(cut.height()).min(DETECT_SIDE) + 2 * PADDING;
        let found = self.detector
            .get_text_boxes(&padded, &ScaleParam::get_scale_param(&padded, side), 0.5, 0.3, 1.6)
            .map_err(|e| format!("text detection: {e}"))?;
        let parts = OcrUtils::get_part_images(&padded, &found);
        let reader = if read == Read::Whole { &mut self.reader } else { &mut self.second };
        let texts = reader.get_text_lines(&parts, &HashMap::new(), 0.0).map_err(|e| format!("text reading: {e}"))?;
        Ok(found.iter().zip(texts).filter_map(|(found, read)| {
            let text = read.text.trim();
            // A score that is not a number (nothing read) fails this too.
            if text.is_empty() || !(read.text_score >= MIN_SCORE) { return None; }
            let back = |at: u32, origin: f64| (at as f64 - PADDING as f64) / scale + origin;
            let xs = || found.points.iter().map(|p| back(p.x, origin[0]));
            let ys = || found.points.iter().map(|p| back(p.y, origin[1]));
            let rect = [xs().fold(f64::MAX, f64::min), ys().fold(f64::MAX, f64::min), xs().fold(f64::MIN, f64::max), ys().fold(f64::MIN, f64::max)];
            let words = words(text, letters(image, rect));
            let color = words.first().and_then(|word| style(image, word));
            Some(Line { text: text.to_string(), words, style: color })
        }).collect())
    }
}

/// `area` of `image`, `scale` times enlarged (smoothly), and its origin in `image` pixels.
fn crop(image: &Image, area: [f64; 4], scale: f64) -> (image::RgbImage, [f64; 2]) {
    let x0 = ((image.width as f64 * area[0]).floor().max(0.0) as usize).min(image.width.saturating_sub(1));
    let y0 = ((image.height as f64 * area[1]).floor().max(0.0) as usize).min(image.height.saturating_sub(1));
    let w = ((image.width as f64 * area[2]) as usize).min(image.width - x0).max(1);
    let h = ((image.height as f64 * area[3]) as usize).min(image.height - y0).max(1);
    let mut pixels = Vec::with_capacity(w * h * 3);
    for y in y0..y0 + h {
        let at = (y * image.width + x0) * 3;
        pixels.extend_from_slice(&image.pixels[at..at + w * 3]);
    }
    let cut = image::RgbImage::from_raw(w as u32, h as u32, pixels).expect("the crop has its pixels");
    let cut = if scale > 1.0 {
        image::imageops::resize(&cut, (w as f64 * scale) as u32, (h as f64 * scale) as u32, image::imageops::FilterType::Triangle)
    } else { cut };
    (cut, [x0 as f64, y0 as f64])
}

/// `rect` (left, top, right, bottom, in image pixels) cut to the letters in it. The
/// detector's boxes leave a margin of about a quarter of the text's height around it; the
/// recognizer measures in title heights and compares the buttons' places with those Windows
/// OCR gave, whose word boxes are tight. Letters: the pixels in the brighter half between
/// the box's ground (its lower quarter of brightness) and its brightest.
pub fn letters(image: &Image, rect: [f64; 4]) -> [f64; 4] {
    let left = (rect[0].floor().max(0.0) as usize).min(image.width);
    let top = (rect[1].floor().max(0.0) as usize).min(image.height);
    let right = (rect[2].ceil().max(0.0) as usize).min(image.width);
    let bottom = (rect[3].ceil().max(0.0) as usize).min(image.height);
    if right < left + 3 || bottom < top + 3 { return rect; }
    let bright = |x: usize, y: usize| {
        let at = (y * image.width + x) * 3;
        image.pixels[at].max(image.pixels[at + 1]).max(image.pixels[at + 2])
    };
    let mut counts = [0usize; 256];
    for y in top..bottom { for x in left..right { counts[bright(x, y) as usize] += 1; } }
    let total = (right - left) * (bottom - top);
    let level = |share: f64| {
        let mut below = 0;
        counts.iter().position(|&count| { below += count; below as f64 >= total as f64 * share }).unwrap_or(255) as u32
    };
    let (ground, top_level) = (level(0.25), level(0.99));
    // No text that stands out: leave the box as found.
    if top_level < ground + 24 { return rect; }
    let cut = ((ground + top_level) / 2) as u8;
    // The letters' rows: bright along at least a twentieth of the box, one after another.
    // The frame lines of a row or a button above and below the text are apart from them,
    // and the tail of a J under the line is too little: the band is the capitals' height.
    let need = ((right - left) / 20).max(2);
    let inked: Vec<usize> = (top..bottom).map(|y| (left..right).filter(|&x| bright(x, y) >= cut).count()).collect();
    let mut band: Option<(usize, usize, usize)> = None; // rows from..to, their bright pixels
    let mut at = 0;
    while at < inked.len() {
        if inked[at] < need { at += 1; continue; }
        let from = at;
        while at < inked.len() && inked[at] >= need { at += 1; }
        let sum: usize = inked[from..at].iter().sum();
        if band.is_none_or(|(_, _, most)| sum > most) { band = Some((from, at, sum)); }
    }
    let Some((from, to, _)) = band else { return rect };
    let (y0, y1) = (top + from, top + to);
    // Their columns: more than a stray bright pixel within those rows.
    let columns: Vec<bool> = (left..right).map(|x| (y0..y1).filter(|&y| bright(x, y) >= cut).count() >= 2).collect();
    // A sliver at either end, one or two columns standing well apart from the text, is the
    // row's frame, not a letter (the owner's third row began 15 px early, at the frame).
    let apart = ((y1 - y0) / 4).max(2);
    let edge = |columns: &mut dyn Iterator<Item = (usize, &bool)>| -> Option<usize> {
        let inked: Vec<usize> = columns.filter(|(_, &on)| on).map(|(x, _)| x).collect();
        let mut first = 0;
        loop {
            let start = *inked.get(first)?;
            let run = inked[first..].iter().enumerate().take_while(|&(i, &x)| x.abs_diff(start) == i).count();
            let next = inked.get(first + run);
            if run <= 2 && next.is_some_and(|&x| x.abs_diff(start) >= run + apart) { first += run; } else { return Some(start); }
        }
    };
    let (Some(x0), Some(x1)) = (edge(&mut columns.iter().enumerate()), edge(&mut columns.iter().enumerate().rev())) else { return rect };
    [(left + x0) as f64, y0 as f64, (left + x1 + 1) as f64, y1 as f64]
}

/// The words of a line in `rect` (left, top, right, bottom), each given its share of the
/// line's width by its letters: these models read whole lines, the recognizer wants words.
pub fn words(text: &str, rect: [f64; 4]) -> Vec<Word> {
    let letters: Vec<char> = text.chars().collect();
    let width = rect[2] - rect[0];
    let total = letters.len().max(1) as f64;
    let mut words = Vec::new();
    let mut start = None;
    for at in 0..=letters.len() {
        let space = letters.get(at).is_none_or(|c| c.is_whitespace());
        match (space, start) {
            (false, None) => start = Some(at),
            (true, Some(from)) => {
                words.push(Word { text: letters[from..at].iter().collect(), x: rect[0] + width * from as f64 / total, y: rect[1],
                    width: width * (at - from) as f64 / total, height: rect[3] - rect[1] });
                start = None;
            }
            _ => {}
        }
    }
    words
}

/// A frame of `image` from the lines read, top to bottom.
pub fn frame(image: &Image, mut lines: Vec<Line>) -> Frame {
    let place = |line: &Line| line.words.first().map(|w| (w.y, w.x)).unwrap_or_default();
    lines.sort_by(|a, b| place(a).0.total_cmp(&place(b).0).then(place(a).1.total_cmp(&place(b).1)));
    Frame { width: image.width, height: image.height, lines }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_split_into_words_by_its_letters() {
        let found = words("LEVEL 97 INVOKER", [100.0, 50.0, 260.0, 70.0]);
        let texts: Vec<_> = found.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["LEVEL", "97", "INVOKER"]);
        assert_eq!((found[0].x, found[0].width, found[0].y, found[0].height), (100.0, 50.0, 50.0, 20.0));
        assert_eq!((found[1].x, found[1].width), (160.0, 20.0));
        assert_eq!(found[2].x + found[2].width, 260.0);
        let title = words("MUCKODUBRE", [10.0, 0.0, 110.0, 30.0]);
        assert_eq!((title.len(), title[0].x, title[0].width), (1, 10.0, 100.0), "a name is one word, the whole box");
        assert!(words("  ", [0.0, 0.0, 10.0, 10.0]).is_empty());
    }

    #[test]
    fn a_box_is_cut_to_its_letters() {
        // Dark, slightly uneven ground; gold "letters" in x 20..80, y 10..22; one stray bright pixel.
        let mut image = Image { width: 100, height: 40, pixels: Vec::with_capacity(100 * 40 * 3) };
        for at in 0..100 * 40 { let v = 14 + (at % 7) as u8; image.pixels.extend_from_slice(&[v, v, v]); }
        let mut paint = |x: usize, y: usize| { let at = (y * 100 + x) * 3; image.pixels[at..at + 3].copy_from_slice(&[200, 160, 80]); };
        for y in 10..22 { for x in (20..80).step_by(3) { paint(x, y); } }
        paint(90, 30);
        // A frame around the text: a line above it and a thin upright at the left.
        for x in 6..95 { paint(x, 5); }
        for y in 5..30 { paint(8, y); }
        assert_eq!(letters(&image, [5.0, 3.0, 96.0, 36.0]), [20.0, 10.0, 78.0, 22.0]);
        // Nothing stands out: the box stays as found.
        assert_eq!(letters(&image, [0.0, 30.0, 15.0, 40.0]), [0.0, 30.0, 15.0, 40.0]);
        assert!((scale_for(1080) - 2.0).abs() < 1e-9 && scale_for(2160) == 1.0 && scale_for(4320) == 1.0 && scale_for(600) == 3.0);
    }

    /// The game's own font, drawn as a selection screen: the models themselves are run.
    #[test]
    fn the_models_read_a_drawn_selection_screen() {
        let font = fontdue::Font::from_bytes(include_bytes!("../../data/fonts/fontin-regular.ttf") as &[u8], fontdue::FontSettings::default()).unwrap();
        let (width, height) = (1920usize, 1080usize);
        let mut image = Image { width, height, pixels: vec![12; width * height * 3] };
        let mut draw = |text: &str, x: f64, y: f64, px: f32, color: [u8; 3]| {
            let mut pen = (x * width as f64) as f32;
            let top = (y * height as f64) as f32;
            for c in text.chars() {
                let (metrics, bitmap) = font.rasterize(c, px);
                for gy in 0..metrics.height { for gx in 0..metrics.width {
                    let (ix, iy) = ((pen as i64 + metrics.xmin as i64 + gx as i64) as usize, (top + px - metrics.ymin as f32 - metrics.height as f32) as usize + gy);
                    let cover = bitmap[gy * metrics.width + gx] as u32;
                    let at = (iy * width + ix) * 3;
                    for channel in 0..3 { image.pixels[at + channel] = (12 + (color[channel] as u32 - 12) * cover / 255) as u8; }
                }}
                pen += metrics.advance_width;
            }
        };
        let (gold, dull, grey) = ([225, 185, 110], [150, 120, 70], [150, 150, 150]);
        draw("BATONGATESTER", 0.795, 0.42, 24.0, gold);
        draw("LEVEL 42 WITCH", 0.853, 0.445, 16.0, dull);
        draw("Act 2: Vastiri Outskirts", 0.853, 0.468, 16.0, grey);
        draw("Standard League", 0.853, 0.491, 16.0, grey);
        draw("CREATE", 0.822, 0.748, 20.0, gold);
        draw("DELETE", 0.930, 0.748, 20.0, gold);
        draw("PLAY", 0.368, 0.885, 30.0, gold);
        let mut engine = Engine::load().expect("the bundled models load");
        let scale = scale_for(height);
        let mut lines = engine.read(&image, AREAS[0], scale, Read::Whole).unwrap();
        lines.extend(engine.read(&image, AREAS[1], scale, Read::Whole).unwrap());
        let whole = frame(&image, lines);
        let texts: Vec<_> = whole.lines.iter().map(|l| l.text.to_uppercase()).collect();
        for expected in ["BATONGATESTER", "LEVEL 42 WITCH", "STANDARD LEAGUE", "CREATE", "DELETE", "PLAY"] {
            assert!(texts.iter().any(|t| t == expected), "{expected} not in {texts:?}");
        }
        let title = &whole.lines.iter().find(|l| l.text == "BATONGATESTER").unwrap().words[0];
        // Cut to its letters: where it was drawn, a capital's height.
        assert!((title.x / width as f64 - 0.795).abs() < 0.003 && (14.0..=26.0).contains(&title.height), "{} {}", title.x, title.height);
        assert_eq!(super::super::shared::focus_of(&whole), Some(super::super::shared::SELECTION_FOCUS));
        let focused = frame(&image, engine.read(&image, super::super::shared::SELECTION_FOCUS, scale, Read::Focused).unwrap());
        assert!(focused.lines.iter().any(|l| l.text == "BATONGATESTER"), "the second reader reads the name too");
    }
}
