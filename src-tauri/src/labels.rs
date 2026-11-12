//! Reading the big map's exit labels: the name of the zone an exit leads to, which the
//! game writes centred above the exit icon. Not OCR: each name the exit can have (the
//! zone's connections, set by the recorder) is drawn in the game's own font at the
//! label's size and compared with the label's pixels; a label is read only when one name
//! is clearly ahead of every other, "Checkpoint" and "Waypoint" included.
//!
//! Measured on the live big map (3840x2160): Fontin Regular at 17 px, light grey, the
//! text's top 25 px above the icon centre.

use super::Image;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// The game's label font, from its data (`art/2dart/fonts/fontin-regular.ttf`;
/// `scripts/fetch-fonts.sh`).
static FONT_BYTES: &[u8] = include_bytes!("../../data/fonts/fontin-regular.ttf");
/// Font sizes tried, in px at 2160 px height (17 fits the live labels best; a long name
/// on the minimap, "Clearfell Encampment", fit only at another size: 0.88 → 0.93).
const SIZES: [f32; 7] = [16.4, 16.6, 16.8, 17.0, 17.2, 17.4, 17.6];
/// The label's top lies this far above the icon centre, in px at 2160 px height.
const ABOVE: f64 = 25.0;
/// How far around that place the label is looked for (x, y), in px at 2160 px height.
const SEARCH: [f64; 2] = [6.0, 4.0];
/// A name is read with at least this score (0-1) and this lead over the next one. Live
/// labels: the right name 0.954-0.996, leading by 0.17-0.22; with another zone's names
/// the best wrong one reached 0.850, leading by 0.08.
pub const MIN_SCORE: f64 = 0.90;
/// A label cut off by the image's edge is read when at least this share of it is on screen.
const SHOWN_MOST: f64 = 0.6;
pub const MIN_LEAD: f64 = 0.10;
/// What an exit's label must not be: these labels belong to other icons nearby.
const OTHERS: [&str; 2] = ["Checkpoint", "Waypoint"];

struct State {
    names: Vec<String>,
    /// Counts the candidate lists set: a read begun under one list is not kept under the next.
    generation: u64,
    /// Rendered names: (name, size in hundredths of a px) -> mask.
    rendered: HashMap<(String, u32), Mask>,
    /// Names read, by what was drawn: the icon's place (image pixels / 4) and a signature
    /// of the label's pixels. Unread labels are not kept: a label hidden once (an effect,
    /// loot) is read again, and another exit landing on the same place has other pixels.
    read: HashMap<((i64, i64), u64), String>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// The names the exits of the current zone can have (its connections' zone names); until
/// this is called, or with an empty list, no label is read. To be called from the
/// recorder's zone entry.
pub fn set_label_candidates(names: Vec<String>) {
    if let Ok(mut state) = STATE.lock() {
        let (rendered, generation) = state.take().map(|s| (s.rendered, s.generation + 1)).unwrap_or_default();
        *state = Some(State { names, generation, rendered, read: HashMap::new() });
    }
}

fn font() -> &'static fontdue::Font {
    static FONT: OnceLock<fontdue::Font> = OnceLock::new();
    FONT.get_or_init(|| fontdue::Font::from_bytes(FONT_BYTES, fontdue::FontSettings::default()).expect("bundled font"))
}

/// Label text: light grey, nearly neutral.
fn is_text(r: u8, g: u8, b: u8) -> bool {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    r > 150 && g > 150 && b > 150 && (r - g).abs() < 20 && (b - r).abs() < 25
}

/// A drawn name: its inked pixels in their tight box.
#[derive(Clone)]
struct Mask {
    w: usize,
    h: usize,
    on: Vec<bool>,
}

/// `text` drawn at `px` pixels (pen moved by advance widths and kerning), pixels at
/// least 43% covered.
fn render(text: &str, px: f32) -> Mask {
    let font = font();
    let line = font.horizontal_line_metrics(px).map(|l| l.ascent).unwrap_or(px);
    let width = text.chars().map(|c| font.metrics(c, px).advance_width).sum::<f32>().ceil() as usize + 8;
    let height = (px * 2.0).ceil() as usize + 4;
    let mut canvas = vec![0u8; width * height];
    let (mut pen, mut previous) = (2.0f32, None);
    for c in text.chars() {
        if let Some(p) = previous {
            pen += font.horizontal_kern(p, c, px).unwrap_or(0.0);
        }
        let (metrics, bitmap) = font.rasterize(c, px);
        let left = (pen + metrics.xmin as f32).round() as i64;
        let top = (2.0 + line - (metrics.ymin as f32 + metrics.height as f32)).round() as i64;
        for y in 0..metrics.height {
            for x in 0..metrics.width {
                let (cx, cy) = (left + x as i64, top + y as i64);
                if cx >= 0 && cy >= 0 && (cx as usize) < width && (cy as usize) < height {
                    let cell = &mut canvas[cy as usize * width + cx as usize];
                    *cell = (*cell).max(bitmap[y * metrics.width + x]);
                }
            }
        }
        pen += metrics.advance_width;
        previous = Some(c);
    }
    let on: Vec<bool> = canvas.iter().map(|&v| v > 110).collect();
    let (mut x0, mut y0, mut x1, mut y1) = (width, height, 0, 0);
    for (i, _) in on.iter().enumerate().filter(|(_, &v)| v) {
        let (x, y) = (i % width, i / width);
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    if x0 > x1 {
        return Mask { w: 0, h: 0, on: vec![] };
    }
    let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
    let on = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| on[(y + y0) * width + x + x0]).collect();
    Mask { w, h, on }
}

/// How well each name matches the label above the exit icon at `icon` (image pixels),
/// best over the sizes and small shifts: 2PR/(P+R) with P the share of label pixels in the
/// name's box (and about a character to its left and right) that lie within a pixel of the
/// drawn name, R the share of the drawn name's pixels within a pixel of label pixels. `size` is the window's height / 2160.
pub fn scores(image: &Image, icon: [f64; 2], size: f64, names: &[String]) -> Vec<(String, f64)> {
    let mut state = STATE.lock().ok();
    let mut cache = state.as_mut().and_then(|s| s.as_mut()).map(|s| &mut s.rendered);
    // The label's neighbourhood as a text mask, and the mask grown by a pixel.
    let reach = (200.0 * size) as i64;
    let (ix, iy) = (icon[0].round() as i64, icon[1].round() as i64);
    let (ax, ay) = ((ix - reach).max(0) as usize, (iy - (ABOVE + 40.0) as i64).max(0) as usize);
    let (bx, by) = (((ix + reach) as usize).min(image.width), (iy.max(0) as usize).min(image.height));
    let (w, h) = (bx.saturating_sub(ax), by.saturating_sub(ay));
    let mut text = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = ((ay + y) * image.width + ax + x) * 3;
            text[y * w + x] = is_text(image.pixels[i], image.pixels[i + 1], image.pixels[i + 2]);
        }
    }
    let grown = grow(&text, w, h);
    let at = |mask: &[bool], x: i64, y: i64| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && mask[y as usize * w + x as usize];
    names
        .iter()
        .map(|name| {
            let mut best = 0.0f64;
            for base in SIZES {
                let px = base * size as f32;
                let key = (name.clone(), (px * 100.0) as u32);
                let mask = match cache.as_mut().and_then(|c| c.get(&key)) {
                    Some(mask) => mask.clone(),
                    None => {
                        let mask = render(name, px);
                        if let Some(c) = cache.as_mut() {
                            c.insert(key, mask.clone());
                        }
                        mask
                    }
                };
                if mask.on.is_empty() {
                    continue;
                }
                let drawn = mask.on.iter().filter(|&&v| v).count() as f64;
                let mask_grown = grow(&mask.on, mask.w, mask.h);
                let (sx, sy) = ((SEARCH[0] * size).ceil() as i64, (SEARCH[1] * size).ceil() as i64);
                let top = (icon[1] - ABOVE * size).round() as i64 - ay as i64;
                let left = (icon[0] - mask.w as f64 / 2.0).round() as i64 - ax as i64;
                for dy in -sy..=sy {
                    for dx in -sx..=sx {
                        let (x0, y0) = (left + dx, top + dy);
                        // A label cut off by the image's edge is scored on its part on screen,
                        // when that is most of it.
                        let (mut near, mut shown) = (0.0, 0.0);
                        for y in 0..mask.h {
                            for x in 0..mask.w {
                                if !mask.on[y * mask.w + x] {
                                    continue;
                                }
                                let (gx, gy) = (ax as i64 + x0 + x as i64, ay as i64 + y0 + y as i64);
                                if gx >= 0 && gy >= 0 && (gx as usize) < image.width && (gy as usize) < image.height {
                                    shown += 1.0;
                                }
                                if at(&grown, x0 + x as i64, y0 + y as i64) {
                                    near += 1.0;
                                }
                            }
                        }
                        if shown < SHOWN_MOST * drawn {
                            continue;
                        }
                        let recall = near / shown;
                        // Text beside the name's box on its rows counts against it: a short
                        // name must not fit inside a longer label ("Mud Burrow" scored 0.83
                        // on "Clearfell Encampment", the owner's minimap, 2026-11-12).
                        let side = (mask.h as f64 * 0.8).round() as i64;
                        let (mut seen, mut matched) = (0.0, 0.0);
                        for y in -1..=mask.h as i64 {
                            for x in -side..=mask.w as i64 - 1 + side {
                                if at(&text, x0 + x, y0 + y) {
                                    seen += 1.0;
                                    let inside = x >= 0 && y >= 0 && (x as usize) < mask.w && (y as usize) < mask.h;
                                    if inside && mask_grown[y as usize * mask.w + x as usize] {
                                        matched += 1.0;
                                    }
                                }
                            }
                        }
                        let precision = if seen > 0.0 { matched / seen } else { 0.0 };
                        if recall + precision > 0.0 {
                            best = best.max(2.0 * recall * precision / (recall + precision));
                        }
                    }
                }
            }
            (name.clone(), best)
        })
        .collect()
}

/// A mask grown by one pixel in every direction.
fn grow(mask: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut out = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            if mask[y * w + x] {
                for ny in y.saturating_sub(1)..(y + 2).min(h) {
                    for nx in x.saturating_sub(1)..(x + 2).min(w) {
                        out[ny * w + nx] = true;
                    }
                }
            }
        }
    }
    out
}

/// The zone name on the label of the exit icon at `icon` (image pixels), when one of the
/// current candidates is clearly ahead; None otherwise or before candidates are set.
pub fn read(image: &Image, icon: [f64; 2], size: f64) -> Option<String> {
    let key = (((icon[0] / 4.0).round() as i64, (icon[1] / 4.0).round() as i64), signature(image, icon, size));
    let (names, generation) = {
        let state = STATE.lock().ok()?;
        let state = state.as_ref()?;
        if let Some(known) = state.read.get(&key) {
            return Some(known.clone());
        }
        (state.names.clone(), state.generation)
    };
    if names.is_empty() {
        return None;
    }
    let found = decide(&names, scores(image, icon, size, &with_others(&names)));
    if let Some(name) = &found {
        keep(key, name, generation);
    }
    found
}

/// Keeps a name read under the candidate list `generation`, unless the list changed since.
fn keep(key: ((i64, i64), u64), name: &str, generation: u64) {
    if let Ok(mut state) = STATE.lock() {
        if let Some(state) = state.as_mut().filter(|s| s.generation == generation) {
            if state.read.len() >= 64 {
                state.read.clear();
            }
            state.read.insert(key, name.to_string());
        }
    }
}

/// A signature of the label pixels above the exit icon at `icon`: the text mask over the
/// box any label there covers (names up to 320 px wide at 2160 px height; the text's top
/// within the search of its place, 17 px high).
fn signature(image: &Image, icon: [f64; 2], size: f64) -> u64 {
    use std::hash::{Hash, Hasher};
    let clamp = |v: f64, n: usize| (v.max(0.0) as usize).min(n);
    let (x0, x1) = (clamp(icon[0] - 160.0 * size, image.width), clamp(icon[0] + 160.0 * size, image.width));
    let (y0, y1) = (clamp(icon[1] - (ABOVE + SEARCH[1] + 2.0) * size, image.height), clamp(icon[1], image.height));
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (x0, x1, y0, y1).hash(&mut hasher);
    for y in y0..y1 {
        let mut bits = 0u64;
        for x in x0..x1 {
            let i = (y * image.width + x) * 3;
            bits = bits.rotate_left(1) ^ is_text(image.pixels[i], image.pixels[i + 1], image.pixels[i + 2]) as u64;
            if (x - x0) % 64 == 63 {
                bits.hash(&mut hasher);
                bits = 0;
            }
        }
        bits.hash(&mut hasher);
    }
    hasher.finish()
}

/// The candidates with the other labels an exit must not be.
pub fn with_others(names: &[String]) -> Vec<String> {
    names.iter().cloned().chain(OTHERS.iter().map(|s| s.to_string())).filter(|n| !n.is_empty()).collect()
}

/// The best name when it is one of `names`, scores at least `MIN_SCORE` and leads the
/// next by `MIN_LEAD`.
pub fn decide(names: &[String], mut scored: Vec<(String, f64)>) -> Option<String> {
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (best, score) = scored.first()?.clone();
    let next = scored.get(1).map_or(0.0, |s| s.1);
    (names.contains(&best) && score >= MIN_SCORE && score - next >= MIN_LEAD).then_some(best)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::minimap::{bigmap, parse_ppm};

    fn fixture(name: &str) -> Image {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        parse_ppm(&std::fs::read(&path).unwrap()).unwrap()
    }

    /// Icons of `kind` in a fixture, as image pixels.
    fn icons(image: &Image, kind: &str) -> Vec<[f64; 2]> {
        let mut found: Vec<[f64; 2]> = bigmap::icons(image, [0.0, 0.0], 1.0, 1.0)
            .into_iter()
            .filter(|i| i.kind == kind)
            .map(|i| [i.x as f64 * 2.0, i.y as f64 * 2.0])
            .collect();
        found.sort_by(|a, b| a[0].total_cmp(&b[0]));
        found
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// Clearfell's exits lead to these zones (data/layouts/ggg/G1_2.json).
    fn clearfell() -> Vec<String> {
        names(&["Clearfell Encampment", "Mud Burrow", "The Grelwood"])
    }

    /// The name read at `icon`, with the scores (best first) for the report.
    fn read_with(image: &Image, icon: [f64; 2], candidates: &[String]) -> (Option<String>, Vec<(String, f64)>) {
        let mut scored = scores(image, icon, 1.0, &with_others(candidates));
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        (decide(candidates, scored.clone()), scored)
    }

    #[test]
    fn both_exits_are_read() {
        let grelwood = fixture("labels-grelwood.ppm");
        let mud = fixture("labels-mudburrow.ppm");
        let (read, scored) = read_with(&grelwood, icons(&grelwood, "exit")[0], &clearfell());
        println!("The Grelwood exit: {scored:.3?}");
        assert_eq!(read.as_deref(), Some("The Grelwood"));
        let (read, scored) = read_with(&mud, icons(&mud, "exit")[0], &clearfell());
        println!("Mud Burrow exit: {scored:.3?}");
        assert_eq!(read.as_deref(), Some("Mud Burrow"));
    }

    /// labels-town-minimap.ppm: the corner minimap in the Clearfell Encampment (the
    /// owner's capture, 2026-11-12), the exit to Clearfell with its name above it.
    #[test]
    fn an_exits_name_is_read_on_the_corner_minimap_too() {
        let town = fixture("labels-town-minimap.ppm");
        let names: Vec<String> = ["The Riverbank", "Clearfell", "Vastiri Outskirts"].map(String::from).to_vec();
        let exits = icons(&town, "exit");
        assert_eq!(exits.len(), 1, "{exits:?}");
        let (read, scored) = read_with(&town, exits[0], &names);
        println!("The town's exit on the minimap: {scored:.3?}");
        assert_eq!(read.as_deref(), Some("Clearfell"));
    }

    #[test]
    fn checkpoint_and_waypoint_labels_are_no_zone() {
        let grelwood = fixture("labels-grelwood.ppm");
        let mud = fixture("labels-mudburrow.ppm");
        for (image, kind) in [(&grelwood, "checkpoint"), (&mud, "checkpoint"), (&mud, "waypoint")] {
            for icon in icons(image, kind) {
                let (read, scored) = read_with(image, icon, &clearfell());
                println!("{kind} at {icon:?}: {scored:.3?}");
                assert_eq!(read, None);
                assert!(OTHERS.contains(&scored[0].0.as_str()), "{scored:?}");
            }
        }
    }

    #[test]
    fn a_label_cut_by_the_edge_is_read_when_most_of_it_shows() {
        let grelwood = fixture("labels-grelwood.ppm");
        let icon = icons(&grelwood, "exit")[0];
        // The image starts `cut` pixels right of the icon's centre (the label is about 110 px).
        let left_of = |cut: f64| {
            let x0 = (icon[0] + cut) as usize;
            let width = grelwood.width - x0;
            let pixels = (0..grelwood.height).flat_map(|y| grelwood.pixels[(y * grelwood.width + x0) * 3..(y * grelwood.width + grelwood.width) * 3].to_vec()).collect();
            (Image { width, height: grelwood.height, pixels }, [icon[0] - x0 as f64, icon[1]])
        };
        let (image, at) = left_of(-30.0);
        let (read, scored) = read_with(&image, at, &clearfell());
        println!("a quarter cut off: {scored:.3?}");
        assert_eq!(read.as_deref(), Some("The Grelwood"));
        let (image, at) = left_of(15.0);
        let (read, scored) = read_with(&image, at, &clearfell());
        println!("more than half cut off: {scored:.3?}");
        assert_eq!(read, None);
    }

    #[test]
    fn a_covered_or_missing_label_is_not_read() {
        let mut grelwood = fixture("labels-grelwood.ppm");
        let icon = icons(&grelwood, "exit")[0];
        // Its left half covered (another label, an effect): unread.
        let (x0, y0) = (icon[0] as usize - 60, icon[1] as usize - 32);
        for y in y0..y0 + 20 {
            for x in x0..x0 + 55 {
                grelwood.pixels[(y * grelwood.width + x) * 3..][..3].copy_from_slice(&[30, 30, 34]);
            }
        }
        let (read, scored) = read_with(&grelwood, icon, &clearfell());
        println!("half covered: {scored:.3?}");
        assert_eq!(read, None);
        // No label at all.
        for y in y0..y0 + 20 {
            for x in x0..x0 + 120 {
                grelwood.pixels[(y * grelwood.width + x) * 3..][..3].copy_from_slice(&[30, 30, 34]);
            }
        }
        assert_eq!(read_with(&grelwood, icon, &clearfell()).0, None);
    }

    #[test]
    fn another_zones_names_read_nothing() {
        // The Grelwood's exits (G1_4): Clearfell, The Red Vale, The Grim Tangle, Lost Catacombs.
        let grelwood_zone = names(&["Clearfell", "The Red Vale", "The Grim Tangle", "Lost Catacombs"]);
        let grelwood = fixture("labels-grelwood.ppm");
        let mud = fixture("labels-mudburrow.ppm");
        for image in [&grelwood, &mud] {
            let (read, scored) = read_with(image, icons(image, "exit")[0], &grelwood_zone);
            println!("wrong list: {scored:.3?}");
            assert_eq!(read, None);
        }
    }

    /// `crop` pasted on a dark 600x200 canvas with the pixel `icon` at (300, 100).
    fn at_centre(crop: &Image, icon: [f64; 2]) -> Image {
        let (w, h) = (600, 200);
        let mut out = Image { width: w, height: h, pixels: [24, 22, 26].repeat(w * h) };
        let (ox, oy) = (300 - icon[0].round() as usize, 100 - icon[1].round() as usize);
        for y in 0..crop.height {
            for x in 0..crop.width {
                out.pixels[((y + oy) * w + x + ox) * 3..][..3].copy_from_slice(&crop.pixels[(y * crop.width + x) * 3..][..3]);
            }
        }
        out
    }

    /// The only test that sets the shared list (the store is shared by every test).
    #[test]
    fn the_store_keeps_what_was_drawn_and_only_names() {
        let mud = fixture("labels-mudburrow.ppm");
        let icon = icons(&mud, "exit")[0];
        let stored = || STATE.lock().unwrap().as_ref().map_or(0, |s| s.read.len());
        // Nothing is read before candidates are set, or with none.
        set_label_candidates(vec![]);
        assert_eq!(read(&mud, icon, 1.0), None);
        set_label_candidates(clearfell());
        let started = std::time::Instant::now();
        assert_eq!(read(&mud, icon, 1.0).as_deref(), Some("Mud Burrow"));
        println!("first read (names drawn, 5 names x 3 sizes x 117 places): {:?}", started.elapsed());
        // Kept for those pixels: read again from the store.
        assert_eq!(stored(), 1);
        let started = std::time::Instant::now();
        assert_eq!(read(&mud, icon, 1.0).as_deref(), Some("Mud Burrow"));
        println!("from the store, signature included: {:?}", started.elapsed());

        // A label hidden once (here covered by a dark effect) is not kept as unread: read
        // again once it shows.
        let grelwood = fixture("labels-grelwood.ppm");
        let g_icon = icons(&grelwood, "exit")[0];
        let mut hidden = grelwood.clone();
        for y in g_icon[1] as usize - 32..g_icon[1] as usize - 10 {
            for x in g_icon[0] as usize - 60..g_icon[0] as usize + 60 {
                hidden.pixels[(y * hidden.width + x) * 3..][..3].copy_from_slice(&[30, 30, 34]);
            }
        }
        assert_eq!(read(&hidden, g_icon, 1.0), None);
        assert_eq!(stored(), 1);
        assert_eq!(read(&grelwood, g_icon, 1.0).as_deref(), Some("The Grelwood"));

        // Another exit landing on the same screen place gets its own name, not the old one.
        let (first, second) = (at_centre(&grelwood, g_icon), at_centre(&mud, icon));
        assert_eq!(read(&first, [300.0, 100.0], 1.0).as_deref(), Some("The Grelwood"));
        let started = std::time::Instant::now();
        assert_eq!(read(&second, [300.0, 100.0], 1.0).as_deref(), Some("Mud Burrow"));
        println!("a read with the names drawn already: {:?}", started.elapsed());
        assert_eq!(read(&first, [300.0, 100.0], 1.0).as_deref(), Some("The Grelwood"));

        // A read begun before a zone change is not kept after it.
        let generation = STATE.lock().unwrap().as_ref().unwrap().generation;
        set_label_candidates(names(&["Clearfell", "The Red Vale", "The Grim Tangle", "Lost Catacombs"]));
        keep(((75, 25), 1), "The Grelwood", generation);
        assert_eq!(stored(), 0);
        // With the new zone's names that label reads nothing.
        assert_eq!(read(&first, [300.0, 100.0], 1.0), None);
        set_label_candidates(vec![]);
    }
}
