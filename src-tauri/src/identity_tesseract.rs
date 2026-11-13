//! Reading the character screens with `tesseract` (Linux): the image prepared for it, its
//! TSV output turned into the words and lines the shared recognizer expects. No process is
//! started here (`identity_linux.rs` runs it), so all of it is tested on every platform.
use crate::minimap::Image;
use super::shared::{style, Frame, Line, Word};

/// How a crop is prepared. Two different preparations give two independent reads, as the
/// recognizer expects (Windows reads the whole frame and a crop).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Prep {
    /// The brightest channel, inverted: the gold and white text dark on light.
    Inverted,
    /// Black where any channel is bright, else white.
    Threshold,
}

/// Where tesseract reads, as fractions of the game window: the selection screen's right
/// panel (character list, its tooltip, Create and Delete) and the bottom middle (Play,
/// Continue, the creation screen's name, league and class).
pub const AREAS: [[f64; 4]; 2] = [[0.70, 0.10, 0.30, 0.72], [0.15, 0.64, 0.56, 0.34]];

/// Scale for tesseract: the game's menu text is about 1.5% of the window high; tesseract
/// reads best at about 40 px.
pub fn scale_for(height: usize) -> f64 {
    (40.0 / (height as f64 * 0.0153)).clamp(1.0, 3.0)
}

/// The crop `rect` (fractions) of `image`, prepared and scaled, as a binary PGM, and the
/// crop's origin in `image` pixels.
pub fn pgm(image: &Image, rect: [f64; 4], scale: f64, prep: Prep) -> (Vec<u8>, [f64; 2]) {
    let x0 = (image.width as f64 * rect[0]).floor().max(0.0) as usize;
    let y0 = (image.height as f64 * rect[1]).floor().max(0.0) as usize;
    let w = ((image.width as f64 * rect[2]) as usize).min(image.width.saturating_sub(x0)).max(1);
    let h = ((image.height as f64 * rect[3]) as usize).min(image.height.saturating_sub(y0)).max(1);
    let (sw, sh) = (((w as f64) * scale) as usize, ((h as f64) * scale) as usize);
    let mut out = format!("P5\n{sw} {sh}\n255\n").into_bytes();
    out.reserve(sw * sh);
    for y in 0..sh {
        let py = (y0 + ((y as f64 / scale) as usize).min(h - 1)).min(image.height - 1);
        for x in 0..sw {
            let px = (x0 + ((x as f64 / scale) as usize).min(w - 1)).min(image.width - 1);
            let at = (py * image.width + px) * 3;
            let bright = image.pixels[at].max(image.pixels[at + 1]).max(image.pixels[at + 2]);
            out.push(match prep {
                Prep::Inverted => 255 - bright,
                Prep::Threshold => if bright >= 100 { 0 } else { 255 },
            });
        }
    }
    (out, [x0 as f64, y0 as f64])
}

/// The words of tesseract's TSV output (`level` 5 rows), back in `image` pixels.
pub fn parse_tsv(tsv: &str, origin: [f64; 2], scale: f64) -> Vec<Word> {
    tsv.lines().skip(1).filter_map(|row| {
        let cells: Vec<&str> = row.split('\t').collect();
        if cells.len() < 12 || cells[0] != "5" { return None; }
        let text = cells[11..].join("\t").trim().to_string();
        let confidence: f64 = cells[10].parse().ok()?;
        if text.is_empty() || confidence < 0.0 { return None; }
        let number = |i: usize| cells[i].parse::<f64>().ok();
        Some(Word { text, x: origin[0] + number(6)? / scale, y: origin[1] + number(7)? / scale,
            width: number(8)? / scale, height: number(9)? / scale })
    }).collect()
}

/// Words on one baseline, close together, are one line (as Windows OCR returns them):
/// tesseract's own lines follow its page layout, which a game menu does not have.
pub fn group_lines(mut words: Vec<Word>) -> Vec<(String, Vec<Word>)> {
    words.sort_by(|a, b| a.x.total_cmp(&b.x));
    let mut lines: Vec<Vec<Word>> = Vec::new();
    for word in words {
        let centre = word.y + word.height / 2.0;
        let joins = lines.iter_mut().find(|line| {
            let last = line.last().unwrap();
            let height = last.height.max(word.height);
            (last.y + last.height / 2.0 - centre).abs() < height * 0.5 &&
                word.x - (last.x + last.width) < height * 1.2 && word.x >= last.x
        });
        match joins {
            Some(line) => line.push(word),
            None => lines.push(vec![word]),
        }
    }
    lines.sort_by(|a, b| a[0].y.total_cmp(&b[0].y).then(a[0].x.total_cmp(&b[0].x)));
    lines.into_iter().map(|words| (words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "), words)).collect()
}

/// A frame of `image` from the words read, with each line's gold measured as on Windows.
pub fn frame(image: &Image, words: Vec<Word>) -> Frame {
    let lines = group_lines(words).into_iter().map(|(text, words)| {
        let color = words.first().and_then(|word| style(image, word));
        Line { text, words, style: color }
    }).collect();
    Frame { width: image.width, height: image.height, lines }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TSV: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext
1\t1\t0\t0\t0\t0\t0\t0\t1400\t2300\t-1\t
5\t1\t1\t1\t1\t1\t100\t200\t330\t40\t91.2\tAAAABATONGA
5\t1\t2\t1\t1\t1\t600\t196\t150\t40\t88.0\tStandard
5\t1\t2\t1\t1\t2\t770\t197\t140\t40\t87.5\tLeague
5\t1\t3\t1\t1\t1\t380\t266\t110\t36\t90.0\tLEVEL
5\t1\t3\t1\t1\t2\t505\t266\t40\t36\t90.0\t17
5\t1\t4\t1\t1\t1\t560\t267\t100\t36\t90.0\tMONK
5\t1\t5\t1\t1\t1\t10\t10\t5\t5\t-1\t
";

    #[test]
    fn tesseract_words_become_the_lines_windows_ocr_gives() {
        let words = parse_tsv(TSV, [1792.0, 144.0], 2.0);
        assert_eq!(words.len(), 6);
        assert_eq!((words[0].x, words[0].y, words[0].height), (1842.0, 244.0, 20.0), "back in window pixels");
        let lines = group_lines(words);
        let mut texts: Vec<_> = lines.iter().map(|(text, _)| text.as_str()).collect();
        texts.sort();
        // The tooltip beside the title stays apart: it opens well to its right.
        assert_eq!(texts, ["AAAABATONGA", "LEVEL 17 MONK", "Standard League"]);
    }

    #[test]
    fn crops_are_prepared_two_ways() {
        let mut image = Image { width: 100, height: 50, pixels: vec![10; 100 * 50 * 3] };
        let at = (20 * 100 + 80) * 3;
        image.pixels[at..at + 3].copy_from_slice(&[220, 180, 90]); // a gold pixel at (80, 20)
        let (inverted, origin) = pgm(&image, [0.70, 0.20, 0.30, 0.60], 2.0, Prep::Inverted);
        assert_eq!(origin, [70.0, 10.0]);
        let header = b"P5\n60 60\n255\n";
        assert_eq!(&inverted[..header.len()], header);
        let pixel = |data: &[u8], x: usize, y: usize| data[header.len() + y * 60 + x];
        assert_eq!(pixel(&inverted, 20, 20), 255 - 220, "dark text");
        assert_eq!(pixel(&inverted, 0, 0), 245, "light ground");
        let (threshold, _) = pgm(&image, [0.70, 0.20, 0.30, 0.60], 2.0, Prep::Threshold);
        assert_eq!((pixel(&threshold, 21, 21), pixel(&threshold, 0, 0)), (0, 255));
        assert!((scale_for(1440) - 1.82).abs() < 0.01 && scale_for(4320) == 1.0);
    }
}
