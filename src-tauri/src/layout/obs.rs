//! Flight recorder: what the navigator is fed during a zone visit, as JSON lines
//! beside the recording (`layouts/<area>/<seed>.obs.jsonl`), so that every visit can
//! be replayed through the current navigator (`replay`, tests only).
//!
//! Every line has `t` (unix milliseconds) and `k` (its kind). Lines after `start`
//! have a frame `f`, the number of placed minimap frames of the visit so far:
//! `move`, `lost` and `icons` lines belong to frame `f`, the others came after it.
//! `f` is left out where it follows: a `move` or `lost` line starts the frame after
//! the last line's, any other line is in the last line's frame. A frame without a
//! line (no movement, the same icons as last written) is still counted, so the
//! replay hands the navigator exactly the calls the app made.

use super::guide::{GuideView, NavTarget};
use super::nav;
use crate::minimap::bigmap::Icon;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Writes reach the file at most once a second (and at the end of the visit).
const FLUSH_MS: u64 = 1000;
/// A needle is written when it changes, else at least this often while frames come.
const NEEDLE_EVERY_MS: u64 = 5000;
/// … where "changes" is another source, target or layout, or a turn of more than this.
const NEEDLE_TURN: f64 = 15.0;
/// Flight recordings are the only way to score the navigator on real play: they are
/// kept this long, and while all of them stay under this size (oldest go first).
const KEEP_DAYS: u64 = 180;
const KEEP_BYTES: u64 = 300 * 1000 * 1000;
/// More than this waiting (a file that cannot be written) is dropped.
const MAX_BUFFER: usize = 512 * 1024;
/// The layout history beside the area folders.
pub const HISTORY_FILE: &str = "layout-history.jsonl";
/// The history takes a layout that carries this share of the belief.
const HISTORY_SHARE: f64 = 0.9;
/// Checkpoints and waypoints don't move: one seen within this many cells of where
/// one of its kind was seen in this part is the same, and not written again.
const MARK_SAME: f64 = 4.0;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// The app's version (package.json; the crate's own stays at 0.1.0).
pub fn app_version() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| {
        serde_json::from_str::<serde_json::Value>(include_str!("../../../package.json"))
            .ok()
            .and_then(|v| v.get("version")?.as_str().map(str::to_string))
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
    })
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(f: &u64) -> bool {
    *f == 0
}

impl Entry {
    /// Its frame, and whether a line of this kind starts a frame.
    fn frame(&mut self) -> Option<(&mut u64, bool)> {
        match self {
            Entry::Move { f, .. } | Entry::Lost { f, .. } => Some((f, true)),
            Entry::Icons { f, .. }
            | Entry::Seen { f, .. }
            | Entry::Targets { f, .. }
            | Entry::Needle { f, .. }
            | Entry::End { f }
            | Entry::Gap { f } => Some((f, false)),
            Entry::Start { .. } => None,
        }
    }

    /// Its frame left out where it follows from the last line's (`last`), or filled in.
    pub fn implied(&mut self, last: &mut u64, writing: bool) {
        let gap = matches!(self, Entry::Gap { .. });
        let Some((f, starts)) = self.frame() else {
            *last = 0;
            return;
        };
        let implied = if starts { *last + 1 } else { *last };
        if writing {
            *last = *f;
            if *f == implied && !gap {
                *f = 0;
            }
        } else if gap {
            *last = *f;
        } else {
            if *f == 0 {
                *f = implied;
            }
            *last = *f;
        }
    }
}

fn exit_kind() -> String {
    "exit".into()
}

fn is_exit(kind: &str) -> bool {
    kind == "exit"
}

/// A map icon as the reader handed it over (minimap cells from the player); `kind`
/// is left out for exits.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LoggedIcon {
    #[serde(default = "exit_kind", skip_serializing_if = "is_exit")]
    pub kind: String,
    pub x: f32,
    pub y: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl LoggedIcon {
    fn of(icon: &Icon) -> Self {
        LoggedIcon { kind: icon.kind.to_string(), x: icon.x, y: icon.y, label: icon.label.clone() }
    }

    fn same(&self, icon: &Icon) -> bool {
        self.kind == icon.kind && self.x == icon.x && self.y == icon.y && self.label == icon.label
    }

    #[cfg(test)]
    fn icon(&self) -> Icon {
        let kind = match self.kind.as_str() {
            "exit" => "exit",
            "checkpoint" => "checkpoint",
            "waypoint" => "waypoint",
            _ => "other",
        };
        Icon { kind, x: self.x, y: self.y, label: self.label.clone() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct LoggedTarget {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub optional: bool,
}

impl LoggedTarget {
    fn of(t: &NavTarget) -> Self {
        LoggedTarget { label: t.label.clone(), exit: t.exit.clone(), tag: t.tag.clone(), optional: t.optional }
    }

    #[cfg(test)]
    fn target(&self) -> NavTarget {
        NavTarget { label: self.label.clone(), exit: self.exit.clone(), tag: self.tag.clone(), optional: self.optional }
    }
}

/// A needle that followed a way through the recorded map (`Navigator::last_way`):
/// where it looked, from the player (cells, screen axes), and the share of the way
/// over unrecorded ground.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Way {
    pub ahead: Option<[f32; 2]>,
    pub fog: Option<f32>,
}

/// What a way ahead in shadow would have shown (`Navigator::last_shadow`, ways ahead
/// not live): its bearing (screen degrees), length (cells), look-ahead from the
/// player (cells), the share over unrecorded ground, and whether it would be gold.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Shadow {
    pub degrees: Option<f32>,
    pub length: f32,
    pub ahead: [f32; 2],
    pub fog: f32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub gold: bool,
}

/// How the visit's navigator was made.
#[derive(Clone, Debug, PartialEq)]
pub struct Start {
    pub area: String,
    pub seed: Option<u64>,
    /// The arrival's area id ("" = a live arrival from no zone); None: unknown.
    pub from: Option<String>,
    pub scale: f64,
    /// The memory of the instance it started from (as `Navigator::new` takes it), how
    /// old it was then (seconds), and whether its position was resumed.
    pub memory: Option<nav::Memory>,
    pub age: Option<u64>,
    pub resumed: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "k", rename_all = "lowercase")]
pub enum Entry {
    /// `memory`: the navigator started from the instance's memory, written in full in
    /// `remembered` with its `age` (seconds) so the replay starts from it too (files
    /// before v0.1.48 have only the flag).
    Start {
        area: String,
        seed: Option<u64>,
        from: Option<String>,
        scale: f64,
        version: String,
        memory: bool,
        resumed: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        age: Option<u64>,
        /// A `nav::Memory` (kept as JSON: the line's flattened form cannot read its
        /// number-keyed maps directly).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remembered: Option<serde_json::Value>,
    },
    /// `Navigator::moved`, minimap cells (zero moves are not written).
    Move {
        #[serde(default, skip_serializing_if = "is_zero")]
        f: u64,
        dx: i32,
        dy: i32,
    },
    /// `Navigator::lost`: the first frame of a new part.
    Lost {
        #[serde(default, skip_serializing_if = "is_zero")]
        f: u64,
        seconds: f64,
    },
    /// The icons handed to the recorder with frame `f`; `size` (the frame's cells)
    /// when it changed since the last icons line.
    Icons {
        #[serde(default, skip_serializing_if = "is_zero")]
        f: u64,
        #[serde(default, skip_serializing_if = "is_false")]
        whole: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<[usize; 2]>,
        icons: Vec<LoggedIcon>,
    },
    /// `Navigator::seen`: an exit or a tag; `left`: the zone was left to `exit`.
    Seen {
        #[serde(default, skip_serializing_if = "is_zero")]
        f: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        left: bool,
    },
    /// The targets the needle was asked for, when they changed.
    Targets {
        #[serde(default, skip_serializing_if = "is_zero")]
        f: u64,
        targets: Vec<LoggedTarget>,
    },
    /// The needle shown. `kept`: tracking was lost and the last needle stayed (not the
    /// navigator's answer); `stale`: no placed frame for 2 s (gold shown gray); `way`:
    /// it followed a way through the recorded map (the log holds no walls, so a
    /// replay cannot give it).
    Needle {
        #[serde(default, skip_serializing_if = "is_zero")]
        f: u64,
        target: String,
        degrees: Option<f32>,
        source: String,
        confidence: f32,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        variant: String,
        distance: f32,
        #[serde(default, skip_serializing_if = "is_false")]
        kept: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        stale: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        way: bool,
        /// On `way` lines, once the navigator gives them: the look-ahead point the
        /// needle aims at, from the player (cells, screen axes), and the share of the
        /// way over unrecorded ground. Absent until fed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ahead: Option<[f32; 2]>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fog: Option<f32>,
        /// Ways ahead in shadow: what the map would have shown instead.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shadow: Option<Shadow>,
    },
    End {
        #[serde(default, skip_serializing_if = "is_zero")]
        f: u64,
    },
    /// Lines were lost here (a write failed): the visit is incomplete. Its `f` (the
    /// last frame before the next line) is always written, so the lines after it
    /// number their frames right.
    Gap { f: u64 },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Line {
    pub t: u64,
    #[serde(flatten)]
    pub entry: Entry,
}

/// Where the lines go: a file, or a string (tests).
enum Out {
    File(PathBuf),
    #[cfg(test)]
    Memory(String),
}

/// The flight recorder of one zone visit.
pub struct Log {
    out: Out,
    buf: String,
    /// A frame came: until then (a log catch-up, the app started in a town) only the
    /// start and log anchors wait, and nothing is written if none comes.
    active: bool,
    flushed_at: u64,
    failed: bool,
    /// Lines were dropped (a failed write): the next line is preceded by a `gap`.
    gap: bool,
    /// The file may not end in a newline (not written yet by this log, or a write
    /// failed half-way): the next write starts with one.
    newline: bool,
    /// The start line, until it is in the file (a failed first write drops it).
    start: Option<String>,
    frame: u64,
    /// The player's place in this part (cells, from the moves), and the checkpoints
    /// and waypoints written in it.
    pos: (i32, i32),
    marks: Vec<(String, f32, f32)>,
    /// The exits of the last icons line.
    last_icons: Vec<LoggedIcon>,
    last_size: Option<[usize; 2]>,
    last_targets: Option<Vec<LoggedTarget>>,
    /// The last needle written, when, and whether a frame came since.
    last_needle: Option<(u64, Entry)>,
    framed: bool,
    /// The frame of the last line written.
    last_f: u64,
    /// The layout history, and the layouts it took from this visit.
    history: Option<PathBuf>,
    recorded: Vec<String>,
    area: String,
    seed: Option<u64>,
}

impl Log {
    /// The log of a visit beginning now; `history`: the layout history file.
    pub fn new(path: PathBuf, history: Option<PathBuf>, start: &Start, t: u64) -> Self {
        Self::with(Out::File(path), history, start, t)
    }

    #[cfg(test)]
    pub fn memory(start: &Start, t: u64) -> Self {
        Self::with(Out::Memory(String::new()), None, start, t)
    }

    fn with(out: Out, history: Option<PathBuf>, start: &Start, t: u64) -> Self {
        let mut log = Log {
            out,
            buf: String::new(),
            active: false,
            flushed_at: t,
            failed: false,
            gap: false,
            newline: true,
            start: None,
            frame: 0,
            pos: (0, 0),
            marks: Vec::new(),
            last_icons: Vec::new(),
            last_size: None,
            last_targets: None,
            last_needle: None,
            framed: false,
            last_f: 0,
            history,
            recorded: Vec::new(),
            area: start.area.clone(),
            seed: start.seed,
        };
        log.write_line(
            t,
            Entry::Start {
                area: start.area.clone(),
                seed: start.seed,
                from: start.from.clone(),
                scale: start.scale,
                version: app_version().to_string(),
                memory: start.memory.is_some(),
                age: start.age,
                remembered: start.memory.as_ref().and_then(|m| serde_json::to_value(m).ok()),
                resumed: start.resumed,
            },
        );
        log.start = Some(log.buf.clone());
        log
    }

    fn write(&mut self, t: u64, entry: Entry) {
        if self.gap {
            // The frame before this line: a frame's own line names its frame itself.
            let f = match &entry {
                Entry::Move { .. } | Entry::Lost { .. } | Entry::Icons { .. } => self.frame.saturating_sub(1),
                _ => self.frame,
            };
            self.gap = false;
            self.write_line(t, Entry::Gap { f });
        }
        self.write_line(t, entry);
    }

    fn write_line(&mut self, t: u64, mut entry: Entry) {
        if self.buf.len() > MAX_BUFFER {
            return;
        }
        entry.implied(&mut self.last_f, true);
        if let Ok(text) = serde_json::to_string(&Line { t, entry }) {
            self.buf.push_str(&text);
            self.buf.push('\n');
        }
        if self.active && t.saturating_sub(self.flushed_at) >= FLUSH_MS {
            self.flush(t);
        }
    }

    fn flush(&mut self, t: u64) {
        self.flushed_at = t;
        if self.buf.is_empty() {
            return;
        }
        // A newline first where the file may end in a torn line; the start line again
        // when it never reached the file.
        let mut text = String::new();
        if self.newline {
            text.push('\n');
        }
        if let Some(start) = self.start.as_ref().filter(|start| !self.buf.starts_with(start.as_str())) {
            text.push_str(start);
        }
        text.push_str(&self.buf);
        self.buf.clear();
        let written = match &mut self.out {
            Out::File(path) => {
                let result = (|| -> std::io::Result<()> {
                    use std::io::Write;
                    if let Some(dir) = path.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    std::fs::OpenOptions::new().create(true).append(true).open(&*path)?.write_all(text.as_bytes())
                })();
                if let Err(error) = &result {
                    if !self.failed {
                        eprintln!("Layout: could not write {}: {error}", path.display());
                    }
                    self.failed = true;
                }
                result.is_ok()
            }
            #[cfg(test)]
            Out::Memory(out) => {
                out.push_str(&text);
                true
            }
        };
        if written {
            self.newline = false;
            self.start = None;
        } else {
            // The lines are lost: the next one says so.
            self.gap = true;
            self.newline = true;
        }
    }

    /// A placed frame begins (before its move): returns its number.
    pub fn frame(&mut self) -> u64 {
        self.active = true;
        self.framed = true;
        self.frame += 1;
        self.frame
    }

    pub fn moved(&mut self, t: u64, dx: i32, dy: i32) {
        if dx != 0 || dy != 0 {
            self.pos = (self.pos.0 + dx, self.pos.1 + dy);
            self.write(t, Entry::Move { f: self.frame, dx, dy });
        }
    }

    pub fn lost(&mut self, t: u64, seconds: f64) {
        // A new part, with its own coordinates.
        self.pos = (0, 0);
        self.marks.clear();
        self.write(t, Entry::Lost { f: self.frame, seconds });
    }

    /// The frame's icons: every exit, exactly as handed over (the navigator's input),
    /// and the checkpoints and waypoints not yet written in this part (the navigator
    /// does not use them; they don't move). Written when the exits differ from the
    /// last written, on a whole-window scan with an exit or while far exits are kept
    /// (`far`: the recorder holds far exits that this frame uses or replaces), when
    /// the frame size changed while far exits are kept (it decides which are far),
    /// and for a new checkpoint or waypoint.
    pub fn icons(&mut self, t: u64, icons: &[Icon], whole: bool, size: (usize, usize), far: bool) {
        let size = [size.0, size.1];
        let exits: Vec<&Icon> = icons.iter().filter(|i| i.kind == "exit").collect();
        let same = exits.len() == self.last_icons.len() && self.last_icons.iter().zip(&exits).all(|(a, b)| a.same(b));
        let resized = self.last_size != Some(size);
        let (px, py) = (self.pos.0 as f32, self.pos.1 as f32);
        let mut new_marks = Vec::new();
        for icon in icons.iter().filter(|i| i.kind != "exit") {
            let at = (px + icon.x, py + icon.y);
            let known = self.marks.iter().chain(&new_marks).any(|(k, x, y)| k == icon.kind && ((x - at.0).hypot(y - at.1) as f64) <= MARK_SAME);
            if !known {
                new_marks.push((icon.kind.to_string(), at.0, at.1));
            }
        }
        if same && new_marks.is_empty() && !(whole && (!exits.is_empty() || far)) && !(far && resized) {
            return;
        }
        // The size matters for exits only: which are outside the frame.
        let size = (resized && (!exits.is_empty() || far)).then_some(size);
        if size.is_some() {
            self.last_size = size;
        }
        self.last_icons = exits.iter().map(|i| LoggedIcon::of(i)).collect();
        let mut listed = self.last_icons.clone();
        listed.extend(new_marks.iter().map(|(kind, x, y)| LoggedIcon { kind: kind.clone(), x: x - px, y: y - py, label: None }));
        self.marks.extend(new_marks);
        self.write(t, Entry::Icons { f: self.frame, whole, size, icons: listed });
    }

    pub fn seen(&mut self, t: u64, seen: nav::Seen) {
        let f = self.frame;
        let entry = match seen {
            nav::Seen::Exit(exit) => Entry::Seen { f, exit: Some(exit.into()), tag: None, left: false },
            nav::Seen::Tag(tag) => Entry::Seen { f, exit: None, tag: Some(tag.into()), left: false },
            nav::Seen::Left(exit) => Entry::Seen { f, exit: Some(exit.into()), tag: None, left: true },
        };
        self.write(t, entry);
    }

    /// The needle shown, asked for `targets`; written by the rules above.
    pub fn needle(&mut self, t: u64, targets: &[NavTarget], view: &GuideView, kept: bool, stale: bool, way: Option<Way>, shadow: Option<Shadow>) {
        if !self.active {
            return;
        }
        let f = self.frame;
        let list: Vec<LoggedTarget> = targets.iter().map(LoggedTarget::of).collect();
        if self.last_targets.as_ref() != Some(&list) {
            self.write(t, Entry::Targets { f, targets: list.clone() });
            self.last_targets = Some(list);
        }
        let round = |v: f32, k: f32| (v * k).round() / k;
        let entry = Entry::Needle {
            f,
            target: view.target.clone(),
            degrees: view.degrees.map(|d| round(d, 10.0)),
            source: view.source.clone(),
            confidence: round(view.confidence, 100.0),
            variant: view.variant.clone(),
            distance: view.distance.round(),
            kept,
            stale,
            way: way.is_some(),
            ahead: way.and_then(|w| w.ahead),
            fog: way.and_then(|w| w.fog),
            shadow: shadow.map(|s| Shadow {
                degrees: s.degrees.map(|d| round(d, 10.0)),
                length: s.length.round(),
                ahead: [round(s.ahead[0], 10.0), round(s.ahead[1], 10.0)],
                fog: round(s.fog, 100.0),
                gold: s.gold,
            }),
        };
        let due = match &self.last_needle {
            None => true,
            Some((at, last)) => changed(last, &entry) || (self.framed && t.saturating_sub(*at) >= NEEDLE_EVERY_MS),
        };
        if due {
            self.framed = false;
            self.last_needle = Some((t, entry.clone()));
            self.write(t, entry);
        }
    }

    /// The visit ended: everything goes to the file.
    pub fn end(&mut self, t: u64) {
        if !self.active {
            self.buf.clear();
            return;
        }
        self.write(t, Entry::End { f: self.frame });
        self.flush(t);
    }

    /// Writes what waits now (before a restart).
    pub fn flush_now(&mut self) {
        if self.active {
            self.flush(now_ms());
        }
    }

    /// Adds the visit's layout to the layout history when the named exits fixed it:
    /// the top layout at 0.9 or more, and the best of the named fit. Once per layout.
    pub fn history(&mut self, t: u64, memory: &nav::Memory) {
        let Some(path) = self.history.clone() else { return };
        if let Some(line) = history_line(memory, &self.recorded) {
            let line = HistoryLine { t, area: self.area.clone(), seed: self.seed, version: app_version().to_string(), ..line };
            self.recorded.push(line.layout.clone());
            append_line(&path, &line);
        }
    }

    #[cfg(test)]
    pub fn text(&mut self) -> String {
        self.flush(self.flushed_at);
        match &self.out {
            Out::Memory(text) => text.clone(),
            Out::File(path) => std::fs::read_to_string(path).unwrap_or_default(),
        }
    }
}

/// Another needle than the last one written.
fn changed(last: &Entry, now: &Entry) -> bool {
    match (last, now) {
        (
            Entry::Needle { target: t0, degrees: d0, source: s0, variant: v0, kept: k0, shadow: h0, .. },
            Entry::Needle { target: t1, degrees: d1, source: s1, variant: v1, kept: k1, shadow: h1, .. },
        ) => {
            let shadow_moved = match (h0, h1) {
                (Some(a), Some(b)) => {
                    a.gold != b.gold
                        || match (a.degrees, b.degrees) {
                            (Some(x), Some(y)) => super::guide::angle_between(x, y) > NEEDLE_TURN,
                            (None, None) => false,
                            _ => true,
                        }
                }
                (None, None) => false,
                _ => true,
            };
            if shadow_moved {
                return true;
            }
            let turned = match (d0, d1) {
                (Some(a), Some(b)) => super::guide::angle_between(*a, *b) > NEEDLE_TURN,
                (None, None) => false,
                _ => true,
            };
            t0 != t1 || s0 != s1 || v0 != v1 || k0 != k1 || turned
        }
        _ => true,
    }
}

/// One line of `layout-history.jsonl`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct HistoryLine {
    pub t: u64,
    pub area: String,
    pub seed: Option<u64>,
    pub layout: String,
    /// The layout's share of the belief, and of the named fit alone.
    pub share: f64,
    pub named: f64,
    /// "named": the named exits alone give it 0.9; "named+belief": they favour it
    /// most, and with what else was seen it carries 0.9.
    pub by: String,
    pub version: String,
}

fn history_line(memory: &nav::Memory, recorded: &[String]) -> Option<HistoryLine> {
    let (top, &share) = memory.layouts.iter().max_by(|a, b| a.1.total_cmp(b.1))?;
    // No named fit yet: every layout's is 0 (log).
    if share < HISTORY_SHARE || memory.named.values().all(|&v| v == 0.0) || recorded.contains(top) {
        return None;
    }
    let best = memory.named.values().copied().fold(f64::MIN, f64::max);
    let own = memory.named.get(top).copied().unwrap_or(f64::MIN);
    if own < best - 1e-9 {
        return None;
    }
    let sum: f64 = memory.named.values().map(|v| (v - best).exp()).sum();
    let named = (own - best).exp() / sum;
    let by = if named >= HISTORY_SHARE { "named" } else { "named+belief" };
    Some(HistoryLine { layout: top.clone(), share, named, by: by.into(), ..Default::default() })
}

fn append_line(path: &Path, value: &impl Serialize) {
    let result = (|| -> std::io::Result<()> {
        use std::io::Write;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut text = serde_json::to_string(value)?;
        text.push('\n');
        std::fs::OpenOptions::new().create(true).append(true).open(path)?.write_all(text.as_bytes())
    })();
    if let Err(error) = result {
        eprintln!("Layout: could not write {}: {error}", path.display());
    }
}

/// Deletes `.obs.jsonl` files in the area folders of `dir` older than 180 days, and
/// the oldest while all of them come to more than 300 MB; says so when it does.
pub fn prune(dir: &Path) -> usize {
    let (deleted, bytes) = prune_with(dir, std::time::Duration::from_secs(KEEP_DAYS * 24 * 3600), KEEP_BYTES);
    if deleted > 0 {
        eprintln!(
            "Layout: deleted {deleted} flight recording(s), {:.1} MB: older than {KEEP_DAYS} days, or the oldest over {} MB in all",
            bytes as f64 / 1e6,
            KEEP_BYTES / 1_000_000
        );
    }
    deleted
}

/// Returns how many files it deleted, and their bytes.
fn prune_with(dir: &Path, max_age: std::time::Duration, max_bytes: u64) -> (usize, u64) {
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    for area in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        for file in std::fs::read_dir(area.path()).into_iter().flatten().flatten() {
            let path = file.path();
            if !path.to_string_lossy().ends_with(".obs.jsonl") {
                continue;
            }
            if let Ok(meta) = file.metadata() {
                files.push((meta.modified().unwrap_or(std::time::UNIX_EPOCH), meta.len(), path));
            }
        }
    }
    files.sort();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    let (mut deleted, mut bytes) = (0, 0);
    for (modified, len, path) in files {
        let old = modified.elapsed().is_ok_and(|age| age > max_age);
        if (old || total > max_bytes) && std::fs::remove_file(&path).is_ok() {
            total -= len;
            (deleted, bytes) = (deleted + 1, bytes + len);
        }
    }
    (deleted, bytes)
}


/// The replay: a flight recording's lines through a fresh navigator, as the app
/// fed them, and how good the needle was against what the file shows to be true.
#[cfg(test)]
pub(super) mod replay {
    use super::*;
    use crate::layout::graph::{self, Area, Graphs};
    use crate::layout::guide::{angle_between, from_screen, screen_bearing};
    use crate::layout::nav::{Arrival, Navigator, Seen};
    use std::collections::BTreeMap;

    /// An exit icon this close (cells) to where the player left confirms the exit.
    const AT_EXIT: f64 = 30.0;
    /// Icons of two scans this close (cells; the navigator's 40 world units) are one exit.
    const SIGHT: f64 = 31.0;
    /// Needle samples this close (cells) to the exit taken are not scored.
    const NEAR_EXIT: f64 = 10.0;
    /// A needle within this of the straight line to the exit is right.
    const RIGHT: f64 = 45.0;
    /// A logged and a replayed needle agree within this.
    const AGREE: f64 = 5.0;
    /// The named exits' fit, as the navigator's (`ICON_TOL`, `NAMED_BASE`).
    const TOL: f64 = 100.0;
    const FIT_FLOOR: f64 = -9.21;
    /// The named exits fix the layout at this share.
    const FIXED: f64 = 0.9;
    pub const SOURCES: [&str; 4] = ["gold", "rough", "gray", "none"];

    struct Sample {
        t: u64,
        part: usize,
        pos: (i32, i32),
        walked: f64,
        target_exit: Option<String>,
        degrees: Option<f32>,
        source: String,
        /// A shown needle that followed the way through the recorded map.
        way: bool,
    }

    /// Needle samples against the exit taken.
    #[derive(Default, Debug, Clone)]
    pub struct Score {
        /// Per source (`SOURCES`): (within 45°, all).
        pub by_source: [(usize, usize); 4],
        /// Samples of the last part pointing at another target.
        pub other_target: usize,
        /// Scored samples that followed the way through the recorded map.
        pub way: usize,
        /// When the needle was right for good: (ms since start, cells walked).
        pub right_for_good: Option<(u64, f64)>,
    }

    impl Score {
        pub fn count(&self) -> usize {
            self.by_source.iter().map(|s| s.1).sum()
        }

        fn add(&mut self, other: &Score) {
            for (s, a) in self.by_source.iter_mut().zip(&other.by_source) {
                (s.0, s.1) = (s.0 + a.0, s.1 + a.1);
            }
            self.way += other.way;
        }

        fn shares(&self) -> String {
            let parts: Vec<String> = SOURCES
                .iter()
                .zip(&self.by_source)
                .filter(|(_, s)| s.1 > 0)
                .map(|(n, s)| format!("{n} {} of {}", pct(s.0, s.1), s.1))
                .collect();
            if parts.is_empty() { "no samples".into() } else { parts.join(", ") }
        }
    }

    /// What one visit (a `start` line and what follows) gave.
    #[derive(Default, Debug)]
    pub struct Visit {
        pub area: String,
        pub seed: Option<u64>,
        pub from: Option<String>,
        pub version: String,
        pub memory: bool,
        /// The memory was in the file and the replay started from it; a visit that
        /// read one and has not is left out of the agreement.
        pub memory_replayed: bool,
        pub resumed: bool,
        pub start: u64,
        pub end: u64,
        pub ended: bool,
        pub frames: u64,
        pub parts: usize,
        /// Minimap cells walked (dead reckoning).
        pub walked: f64,
        pub scale: f64,
        /// When the top layout changed: (ms since start, layout, its share).
        pub belief: Vec<(u64, String, f64)>,
        /// The layouts the named exits fix (equal fits), their share, the names read.
        pub truth: Option<(Vec<String>, f64, usize)>,
        /// Where each name was read (world units, the last part with two or more).
        pub places: BTreeMap<String, (f64, f64)>,
        /// The exit left through, where (cells, in the last part), how it is confirmed.
        pub left: Option<(String, (i32, i32), &'static str)>,
        /// Left with no exit icon at the place (a portal, a waypoint, a death): the
        /// area left to is no exit taken, and the needle is not scored.
        pub unscored: bool,
        /// Against the exit taken, every frame: the needle the app showed (the last
        /// `needle` line, kept ones included) and the one the replay gives (no recorded map).
        pub shown: Score,
        pub replayed: Score,
        /// The map's ways ahead in shadow (not shown): gold or rough as they would have been.
        pub shadow: Score,
        /// The app's needles, those the current code gives alike, the first that differ.
        pub logged: usize,
        pub agree: usize,
        /// The app's needles that followed a way through the recorded map (not compared).
        pub mapped: usize,
        pub differ: Vec<String>,
        /// Lines were lost (a failed or torn write): not in the totals.
        pub incomplete: bool,
        /// Why the visit was not replayed.
        pub skipped: Option<String>,
    }

    struct Run {
        area: Area,
        nav: Navigator,
        scale: f64,
        targets: Vec<NavTarget>,
        raw: Vec<Icon>,
        whole: bool,
        size: (usize, usize),
        far: super::super::FarIcons,
        far_age: u32,
        pos: (i32, i32),
        part: usize,
        open: Option<u64>,
        done: u64,
        t: u64,
        /// Per part: label → world places of the exits read by name, each reading
        /// counted only when the same exit's scan before read the same name (as the
        /// navigator trusts a label).
        named: Vec<BTreeMap<String, Vec<(f64, f64)>>>,
        /// The exits seen by whole-window scans in this part: place (cells), last name read.
        sights: Vec<((f64, f64), Option<String>)>,
        /// This frame's icons line was a whole-window scan (labels are read on those).
        scan: bool,
        /// Exit icons seen: (part, place in cells, label).
        exits: Vec<(usize, (f64, f64), Option<String>)>,
        samples: Vec<Sample>,
        shown: Vec<Sample>,
        /// The needle on screen: the last `needle` line (target's exit, degrees, source, way).
        on_screen: Option<(Option<String>, Option<f32>, String, bool)>,
        /// What the map's way ahead would have shown then (ways ahead in shadow).
        shadow_on: Option<(Option<String>, Option<f32>, bool)>,
        shadows: Vec<Sample>,
        top: Option<String>,
        visit: Visit,
    }

    impl Run {
        fn new(area: Area, start: &Line) -> Run {
            let Entry::Start { area: id, seed, from, scale, version, memory, resumed, age, remembered } = &start.entry else {
                unreachable!()
            };
            let arrival = match from {
                Some(f) => Arrival::From(f),
                None => Arrival::Unknown,
            };
            // The memory as old as it was then: `Navigator::new` resumes by its age.
            let remembered = remembered
                .clone()
                .and_then(|m| serde_json::from_value::<nav::Memory>(m).ok())
                .map(|m| nav::Memory { at: (now_ms() / 1000).saturating_sub(age.unwrap_or(u64::MAX)), ..m });
            let nav = Navigator::new(&area, arrival, remembered.as_ref(), *scale, seed.unwrap_or(0));
            // As the navigator takes it.
            let scale = if scale.is_finite() && (0.4..=1.6).contains(scale) { *scale } else { graph::SCALE };
            Run {
                area,
                nav,
                scale,
                targets: Vec::new(),
                raw: Vec::new(),
                whole: false,
                size: (0, 0),
                far: Vec::new(),
                far_age: super::super::FAR_FRAMES,
                pos: (0, 0),
                part: 0,
                open: None,
                done: 0,
                t: start.t,
                named: vec![BTreeMap::new()],
                sights: Vec::new(),
                scan: false,
                exits: Vec::new(),
                samples: Vec::new(),
                shown: Vec::new(),
                shadow_on: None,
                shadows: Vec::new(),
                on_screen: None,
                top: None,
                visit: Visit {
                    area: id.clone(),
                    seed: *seed,
                    from: from.clone(),
                    version: version.clone(),
                    memory: *memory,
                    memory_replayed: remembered.is_some(),
                    resumed: *resumed,
                    start: start.t,
                    end: start.t,
                    parts: 1,
                    scale,
                    ..Default::default()
                },
            }
        }

        /// The area ids an exit label names (as the navigator reads them).
        fn ids(&self, label: &str) -> Vec<String> {
            let mut ids: Vec<String> = self
                .area
                .names
                .iter()
                .filter(|(id, name)| id.as_str() == label || name.eq_ignore_ascii_case(label))
                .map(|(id, _)| id.clone())
                .collect();
            if ids.is_empty() {
                ids.push(label.to_string());
            }
            ids
        }

        /// One frame's icons to the navigator, then a needle sample.
        fn close(&mut self) {
            let list = super::super::exit_list(&self.raw, self.whole, self.pos, self.size, &mut self.far, &mut self.far_age);
            self.nav.icons(&list);
            self.whole = false;
            self.visit.frames += 1;
            let scan = std::mem::take(&mut self.scan);
            for icon in self.raw.iter().filter(|i| i.kind == "exit") {
                let cells = (self.pos.0 as f64 + icon.x as f64, self.pos.1 as f64 + icon.y as f64);
                let label = icon.label.clone().filter(|l| !l.is_empty());
                if let Some(label) = label.as_ref().filter(|_| scan) {
                    let near = self.sights.iter_mut().find(|(at, _)| (at.0 - cells.0).hypot(at.1 - cells.1) <= SIGHT);
                    match near {
                        Some((at, last)) => {
                            if last.as_ref() == Some(label) {
                                let (wx, wy) = from_screen((cells.0 / self.scale, cells.1 / self.scale));
                                self.named[self.part].entry(label.clone()).or_default().push((wx, wy));
                            }
                            *at = ((at.0 + cells.0) / 2.0, (at.1 + cells.1) / 2.0);
                            *last = Some(label.clone());
                        }
                        None => self.sights.push((cells, Some(label.clone()))),
                    }
                }
                self.exits.push((self.part, cells, label));
            }
            let view = self.nav.guide(&self.targets);
            let target_exit =
                view.as_ref().and_then(|v| self.targets.iter().find(|t| t.label == v.target)).and_then(|t| t.exit.clone());
            self.samples.push(Sample {
                t: self.t,
                part: self.part,
                pos: self.pos,
                walked: self.visit.walked,
                target_exit,
                degrees: view.as_ref().and_then(|v| v.degrees),
                source: view.map_or("none".into(), |v| v.source),
                way: false,
            });
            // And the needle the player saw on this frame.
            if let Some((target_exit, degrees, source, way)) = self.on_screen.clone() {
                self.shown.push(Sample { t: self.t, part: self.part, pos: self.pos, walked: self.visit.walked, target_exit, degrees, source, way });
            }
            // And what the map would have shown in its place.
            if let Some((target_exit, degrees, gold)) = self.shadow_on.clone() {
                let source = if gold { "estimated" } else { "rough" }.to_string();
                self.shadows.push(Sample { t: self.t, part: self.part, pos: self.pos, walked: self.visit.walked, target_exit, degrees, source, way: true });
            }
            let ranking = self.nav.ranking();
            let (id, p) = ranking[0];
            if self.top.as_deref() != Some(id) {
                self.visit.belief.push((self.t - self.visit.start, id.to_string(), p));
                self.top = Some(id.to_string());
            }
        }

        /// Every frame up to `n` handed over: the open one, then those without a line
        /// (no movement, the icons as last written).
        fn advance_to(&mut self, n: u64) {
            if let Some(f) = self.open.filter(|&f| f <= n) {
                self.close();
                self.done = f;
                self.open = None;
            }
            while self.done < n {
                self.done += 1;
                self.close();
            }
        }

        fn line(&mut self, line: &Line) {
            self.t = line.t;
            self.visit.end = line.t;
            match &line.entry {
                Entry::Move { f, .. } | Entry::Lost { f, .. } | Entry::Icons { f, .. } => {
                    if self.open != Some(*f) {
                        self.advance_to(f.saturating_sub(1));
                        self.open = Some(*f);
                    }
                }
                Entry::Seen { f, .. } | Entry::Targets { f, .. } | Entry::Needle { f, .. } | Entry::End { f } | Entry::Gap { f } => {
                    self.advance_to(*f)
                }
                Entry::Start { .. } => {}
            }
            match &line.entry {
                Entry::Move { dx, dy, .. } => {
                    self.nav.moved(*dx as f64, *dy as f64);
                    self.pos = (self.pos.0 + dx, self.pos.1 + dy);
                    self.visit.walked += (*dx as f64).hypot(*dy as f64);
                }
                Entry::Lost { seconds, .. } => {
                    self.nav.lost(*seconds);
                    // A new part, with its own coordinates (the recorder drops far icons).
                    self.far.clear();
                    self.pos = (0, 0);
                    self.part += 1;
                    self.visit.parts += 1;
                    self.named.push(BTreeMap::new());
                    self.sights.clear();
                }
                Entry::Icons { whole, size, icons, .. } => {
                    self.raw = icons.iter().map(LoggedIcon::icon).collect();
                    self.whole = *whole;
                    self.scan = *whole;
                    if let Some([c, r]) = size {
                        self.size = (*c, *r);
                    }
                }
                Entry::Seen { exit, tag, left, .. } => match (exit, tag, left) {
                    (Some(exit), _, true) => {
                        self.nav.seen(Seen::Left(exit));
                        self.visit.left = Some((exit.clone(), self.pos, ""));
                    }
                    (Some(exit), _, false) => self.nav.seen(Seen::Exit(exit)),
                    (None, Some(tag), _) => self.nav.seen(Seen::Tag(tag)),
                    _ => {}
                },
                Entry::Targets { targets, .. } => self.targets = targets.iter().map(LoggedTarget::target).collect(),
                Entry::Needle { target, degrees, source, variant, kept, stale, way, shadow, .. } => {
                    // What the player sees from now on (a kept needle too: gray).
                    let target_exit = self.targets.iter().find(|t| t.label == *target).and_then(|t| t.exit.clone());
                    self.shadow_on = shadow.filter(|_| !*kept).map(|s| (target_exit.clone(), s.degrees, s.gold));
                    self.on_screen = Some((target_exit, *degrees, source.clone(), *way));
                    if *kept {
                        return;
                    }
                    // Through the recorded map: without its walls the replay cannot give it.
                    if *way {
                        self.visit.mapped += 1;
                        return;
                    }
                    self.visit.logged += 1;
                    let now = self.nav.guide(&self.targets);
                    let now_source = now.as_ref().map(|v| {
                        if *stale && v.source == "estimated" { "unconfirmed".to_string() } else { v.source.clone() }
                    });
                    let same = now.as_ref().is_some_and(|v| {
                        let turned = match (v.degrees, degrees) {
                            (Some(a), Some(b)) => angle_between(a, *b) > AGREE,
                            (None, None) => false,
                            _ => true,
                        };
                        v.target == *target && now_source.as_deref() == Some(source.as_str()) && v.variant == *variant && !turned
                    });
                    if same {
                        self.visit.agree += 1;
                    } else if self.visit.differ.len() < 5 {
                        let shown = |t: &str, d: Option<f32>, s: &str, v: &str| {
                            format!("{t} {}° {s}{}", d.map_or("-".into(), |d| format!("{d:.0}")), if v.is_empty() { String::new() } else { format!(" {v}") })
                        };
                        self.visit.differ.push(format!(
                            "{:.1}s app {} / now {}",
                            (line.t - self.visit.start) as f64 / 1000.0,
                            shown(target, *degrees, source, variant),
                            now.as_ref().map_or("none".into(), |v| shown(&v.target, v.degrees, now_source.as_deref().unwrap_or(""), &v.variant))
                        ));
                    }
                }
                Entry::End { .. } => self.visit.ended = true,
                Entry::Gap { .. } => self.visit.incomplete = true,
                Entry::Start { .. } => {}
            }
        }

        fn finish(mut self) -> Visit {
            self.advance_to(self.open.unwrap_or(self.done));
            self.truth();
            self.score();
            self.visit
        }

        /// The layout the named exits fix: how well each layout's exits lie as the
        /// exits read by name in one part do (relative places, as the navigator's
        /// named fit), summed over parts.
        fn truth(&mut self) {
            let mut fits = vec![0.0f64; self.area.layouts.len()];
            let mut read = 0;
            let mut places = BTreeMap::new();
            for named in &self.named {
                if named.len() < 2 {
                    continue;
                }
                read = read.max(named.len());
                // Each name's place: the median of its readings (a misread elsewhere moves it little).
                let median = |mut v: Vec<f64>| {
                    v.sort_by(f64::total_cmp);
                    let n = v.len();
                    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
                };
                let at: Vec<(Vec<String>, (f64, f64))> = named
                    .iter()
                    .map(|(l, read)| (l, (median(read.iter().map(|p| p.0).collect()), median(read.iter().map(|p| p.1).collect()))))
                    .map(|(l, at)| {
                        places.insert(l.clone(), at);
                        (self.ids(l), at)
                    })
                    .collect();
                for (k, layout) in self.area.layouts.iter().enumerate() {
                    let nodes = |ids: &[String]| -> Vec<usize> {
                        (0..layout.nodes.len()).filter(|&i| layout.nodes[i].exit.as_ref().is_some_and(|e| ids.contains(e))).collect()
                    };
                    let first = nodes(&at[0].0);
                    for (ids, place) in &at[1..] {
                        let want = (place.0 - at[0].1 .0, place.1 - at[0].1 .1);
                        let mut best = FIT_FLOOR;
                        for &n1 in &first {
                            for &n2 in &nodes(ids) {
                                if n1 == n2 {
                                    continue;
                                }
                                let (p, q) = (&layout.nodes[n1], &layout.nodes[n2]);
                                let got = ((q.x - p.x) * layout.unit, (q.y - p.y) * layout.unit);
                                let d2 = (got.0 - want.0).powi(2) + (got.1 - want.1).powi(2);
                                best = best.max(-d2 / (4.0 * TOL * TOL));
                            }
                        }
                        fits[k] += best;
                    }
                }
            }
            self.visit.places = places;
            if read < 2 {
                return;
            }
            let top = fits.iter().copied().fold(f64::MIN, f64::max);
            let sum: f64 = fits.iter().map(|f| (f - top).exp()).sum();
            let group: Vec<String> =
                self.area.layouts.iter().zip(&fits).filter(|(_, &f)| f >= top - 1e-6).map(|(l, _)| l.id.clone()).collect();
            let share = group.len() as f64 / sum;
            self.visit.truth = Some((group, share, read));
        }

        /// The needle against the straight line to the exit taken, in the part the
        /// player left from (dead reckoning back from the moment of leaving).
        fn score(&mut self) {
            let Some((exit, at, _)) = self.visit.left.clone() else { return };
            let ids_match = |label: &Option<String>| label.as_deref().is_some_and(|l| self.ids(l).contains(&exit));
            let near = |c: &(f64, f64)| (c.0 - at.0 as f64).hypot(c.1 - at.1 as f64) <= AT_EXIT;
            let mine: Vec<&(usize, (f64, f64), Option<String>)> = self.exits.iter().filter(|e| e.0 == self.part && near(&e.1)).collect();
            let how = if mine.iter().any(|e| ids_match(&e.2)) {
                "its labelled icon was there"
            } else if !mine.is_empty() {
                "an exit icon was there"
            } else {
                "no exit icon seen there: a portal, waypoint or death"
            };
            self.visit.left = Some((exit.clone(), at, how));
            if mine.is_empty() {
                self.visit.unscored = true;
                return;
            }
            self.visit.replayed = tally(&self.samples, self.part, &exit, at, self.visit.start);
            self.visit.shown = tally(&self.shown, self.part, &exit, at, self.visit.start);
            self.visit.shadow = tally(&self.shadows, self.part, &exit, at, self.visit.start);
        }
    }

    /// The samples of `part` aimed at `exit`, against the straight line to `at`.
    fn tally(samples: &[Sample], part: usize, exit: &str, at: (i32, i32), start: u64) -> Score {
        let mut score = Score::default();
        let mut right: Vec<(bool, u64, f64)> = Vec::new();
        {
            for s in samples.iter().filter(|s| s.part == part) {
                if s.target_exit.as_deref() != Some(exit) {
                    score.other_target += 1;
                    continue;
                }
                let (dx, dy) = ((at.0 - s.pos.0) as f64, (at.1 - s.pos.1) as f64);
                if dx.hypot(dy) < NEAR_EXIT {
                    continue;
                }
                let truth = screen_bearing(dx, dy);
                let source = match s.source.as_str() {
                    "estimated" => 0,
                    "rough" => 1,
                    _ if s.degrees.is_some() => 2,
                    _ => 3,
                };
                let ok = matches!((s.degrees, truth), (Some(d), Some(t)) if angle_between(d, t) <= RIGHT);
                score.by_source[source].1 += 1;
                score.by_source[source].0 += ok as usize;
                score.way += s.way as usize;
                right.push((ok, s.t - start, s.walked));
            }
        }
        // The first sample after which every one was right.
        if right.last().is_some_and(|r| r.0) {
            let from = right.iter().rposition(|r| !r.0).map_or(0, |i| i + 1);
            score.right_for_good = Some((right[from].1, right[from].2));
        }
        score
    }

    /// Replays a flight recording's text: one result per visit.
    pub fn replay(text: &str, graphs: &Graphs) -> Vec<Visit> {
        let mut out = Vec::new();
        let mut run: Option<Run> = None;
        let mut last_f = 0;
        // A line that could not be read (torn by a crash or a full disk) is skipped;
        // when the visit goes on after it, lines are missing.
        let mut torn = false;
        for raw in text.lines().filter(|l| !l.trim().is_empty()) {
            let Ok(mut line) = serde_json::from_str::<Line>(raw) else {
                torn = true;
                continue;
            };
            if std::mem::take(&mut torn) && !matches!(line.entry, Entry::Start { .. }) {
                if let Some(run) = run.as_mut() {
                    run.visit.incomplete = true;
                }
            }
            line.entry.implied(&mut last_f, false);
            if let Entry::Start { area, .. } = &line.entry {
                out.extend(run.take().map(Run::finish));
                match graphs.get(area) {
                    Some(graph) => run = Some(Run::new(graph, &line)),
                    None => out.push(Visit { area: area.clone(), skipped: Some("no layout graphs".into()), ..Default::default() }),
                }
                continue;
            }
            if let Some(run) = run.as_mut() {
                run.line(&line);
            }
        }
        out.extend(run.map(Run::finish));
        out
    }

    fn pct(a: usize, b: usize) -> String {
        if b == 0 { "-".into() } else { format!("{:.0}%", 100.0 * a as f64 / b as f64) }
    }

    /// The report of one visit, a few lines.
    pub fn report(v: &Visit) -> String {
        let mut out = format!("== {} seed {:?}", v.area, v.seed);
        if let Some(why) = &v.skipped {
            return out + &format!(": skipped, {why}\n");
        }
        let units = v.walked / v.scale;
        out += &format!(
            " (from {}, v{}, scale {:.3}{}{}): {} frames, {} part(s), {:.0} s, walked {:.0} cells ({:.0} units){}\n",
            v.from.as_deref().map_or("unknown", |f| if f.is_empty() { "no zone" } else { f }),
            v.version,
            v.scale,
            match (v.memory, v.memory_replayed) {
                (true, true) => ", from the instance's memory",
                (true, false) => ", from a memory not in the file (needles not compared)",
                _ => "",
            },
            if v.resumed { ", resumed" } else { "" },
            v.frames,
            v.parts,
            (v.end - v.start) as f64 / 1000.0,
            v.walked,
            units,
            if v.ended { "" } else { ", no end line" },
        );
        if v.incomplete {
            out += "   incomplete: lines were lost (a failed or torn write); not in the totals\n";
        }
        let belief: Vec<String> = v.belief.iter().map(|(t, id, p)| format!("{:.0}s {id} {p:.2}", *t as f64 / 1000.0)).collect();
        out += &format!("   top layout: {}\n", if belief.is_empty() { "-".into() } else { belief.join(" → ") });
        let last_top = v.belief.last().map(|b| b.1.clone());
        match &v.truth {
            Some((ids, share, read)) if *share >= FIXED => {
                let agrees = last_top.as_ref().is_some_and(|t| ids.contains(t));
                out += &format!(
                    "   truth layout ({read} exits read by name): {} {share:.2}; the replay's last top {}\n",
                    ids.join(" = "),
                    if agrees { "agrees" } else { "differs" }
                );
            }
            Some((ids, share, read)) => out += &format!("   named exits ({read}) do not fix the layout: best {} {share:.2}\n", ids.join(" = ")),
            None => out += "   truth layout: none (fewer than two exits read by name in one part)\n",
        }
        match &v.left {
            Some((exit, at, how)) if v.unscored => out += &format!("   left to {exit} at ({}, {}); {how}: not scored\n", at.0, at.1),
            Some((exit, at, how)) => {
                out += &format!("   left through {exit} at ({}, {}); {how}\n", at.0, at.1);
                out += "   needle within 45° of the exit taken (last part):\n";
                let good = |s: &Score| match s.right_for_good {
                    Some((t, w)) => format!("right for good after {:.0} s, {:.0} cells ({:.0} units)", t as f64 / 1000.0, w, w / v.scale),
                    None => "never right for good".into(),
                };
                out += &format!(
                    "     shown by the app: {} ({} through the recorded map); {}; {} to another target\n",
                    v.shown.shares(), v.shown.way, good(&v.shown), v.shown.other_target
                );
                out += &format!(
                    "     replayed, no map, every frame: {}; {}; {} to another target\n",
                    v.replayed.shares(), good(&v.replayed), v.replayed.other_target
                );
                out += &format!(
                    "     map (shadow), the way ahead the app did not show: {}; {}\n",
                    v.shadow.shares(), good(&v.shadow)
                );
            }
            None => out += "   exit taken: unknown (no live zone change)\n",
        }
        out += &format!(
            "   app's needles: {} logged, {} agree with the current code; {} through the recorded map (not replayed)\n",
            v.logged, v.agree, v.mapped
        );
        for d in &v.differ {
            out += &format!("     {d}\n");
        }
        out
    }

    /// One line over all visits.
    pub fn summary(files: usize, visits: &[Visit]) -> String {
        let incomplete = visits.iter().filter(|v| v.skipped.is_none() && v.incomplete).count();
        let played: Vec<&Visit> = visits.iter().filter(|v| v.skipped.is_none() && !v.incomplete).collect();
        let truths = played.iter().filter(|v| v.truth.as_ref().is_some_and(|t| t.1 >= FIXED)).count();
        let exits = played.iter().filter(|v| v.left.is_some() && !v.unscored).count();
        let unscored = played.iter().filter(|v| v.unscored).count();
        let (mut shown, mut replayed, mut shadow) = (Score::default(), Score::default(), Score::default());
        for v in &played {
            shown.add(&v.shown);
            replayed.add(&v.replayed);
            shadow.add(&v.shadow);
        }
        // A visit whose navigator started from a memory the file does not hold cannot agree.
        let comparable: Vec<&&Visit> = played.iter().filter(|v| !v.memory || v.memory_replayed).collect();
        let (logged, agree): (usize, usize) = comparable.iter().fold((0, 0), |a, v| (a.0 + v.logged, a.1 + v.agree));
        let blind = played.len() - comparable.len();
        format!(
            "replay: {files} file(s), {} visit(s) replayed ({} skipped, {incomplete} incomplete), {truths} with a layout truth, {exits} with the exit taken, \
             {unscored} unscored exit(s) (no exit icon where they left); \
             needle within 45°, shown: {} ({} through the map); replayed: {}; map (shadow): {}; \
             app's needles agree {} ({agree} of {logged}; {blind} visit(s) from a memory not in the file left out)",
            played.len(),
            visits.len() - played.len() - incomplete,
            shown.shares(),
            shown.way,
            replayed.shares(),
            shadow.shares(),
            pct(agree, logged),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::replay::{replay, report, summary};
    use super::*;
    use crate::layout::graph::{self, Graphs, SCALE};
    use crate::layout::nav::{Arrival, ExitIcon, Navigator, Seen};
    use crate::layout::nav_sim::{walk, Event, Options};

    const T0: u64 = 1_762_945_000_000;

    fn start(area: &str, from: &str) -> Start {
        Start { area: area.into(), seed: Some(11), from: Some(from.into()), scale: SCALE, memory: None, age: None, resumed: false }
    }

    /// A simulated Clearfell visit written as the app writes it: the walker's moves and
    /// exit icons (every third frame a whole-window scan with names), the needle asked
    /// after every frame, and leaving through the target's exit. Returns the text and
    /// the needles given while it was written.
    fn simulated(losses: bool, view: f64) -> (String, Vec<GuideView>, String) {
        simulated_with(losses, view, None)
    }

    /// … with the Mud Burrow's icon read as "G1_4" on the first scan from frame `misread`.
    fn simulated_with(losses: bool, view: f64, misread: Option<usize>) -> (String, Vec<GuideView>, String) {
        let mut misread = misread;
        let frames_whole = 3;
        let area = graph::load("G1_2");
        let layout = area.layouts.iter().find(|l| l.id == "ClearFell_W_S_N").unwrap();
        let target = NavTarget { label: "Mud Burrow".into(), exit: Some("G1_3".into()), ..Default::default() };
        let goals = layout.goals(&target);
        let from = layout.goals(&NavTarget { exit: Some("G1_town".into()), ..Default::default() })[0];
        let options = Options { losses, view: Some(view), labels: true, ..Default::default() };
        let events = walk(layout, from, &goals, 5, &options).unwrap();
        let mut log = Log::memory(&start("G1_2", "G1_town"), T0);
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 11);
        let targets = [target];
        let mut needles = Vec::new();
        let size = (100_000, 100_000);
        let mut t = T0;
        // Where the player is in this part, and where the target's icon was seen.
        let (mut pos, mut exit_at) = ((0.0f64, 0.0f64), None);
        for (i, event) in events.iter().enumerate() {
            t += 333;
            log.frame();
            let mut icons: Vec<Icon> = Vec::new();
            match event {
                Event::Lost(seconds) => {
                    log.lost(t, *seconds);
                    nav.lost(*seconds);
                    (pos, exit_at) = ((0.0, 0.0), None);
                }
                Event::Moved { dx, dy, icons: seen, .. } => {
                    log.moved(t, *dx as i32, *dy as i32);
                    nav.moved(*dx as i32 as f64, *dy as i32 as f64);
                    pos = (pos.0 + *dx as i32 as f64, pos.1 + *dy as i32 as f64);
                    if let Some(e) = seen.iter().find(|e| e.to.as_deref() == Some("G1_3")) {
                        exit_at = Some((pos.0 + e.dx as f32 as f64, pos.1 + e.dy as f32 as f64));
                    }
                    let whole = i % frames_whole == 0;
                    icons = seen
                        .iter()
                        .map(|e| Icon { kind: "exit", x: e.dx as f32, y: e.dy as f32, label: e.to.clone().filter(|_| whole) })
                        .collect();
                    let due = misread.is_some_and(|m| i >= m);
                    if let Some(icon) = icons.iter_mut().find(|ic| due && ic.label.as_deref() == Some("G1_3")) {
                        icon.label = Some("G1_4".into());
                        misread = None;
                    }
                }
            }
            let whole = i % frames_whole == 0;
            log.icons(t, &icons, whole, size, false);
            let list: Vec<ExitIcon> =
                icons.iter().map(|i| ExitIcon { dx: i.x as f64, dy: i.y as f64, to: i.label.clone() }).collect();
            nav.icons(&list);
            if let Some(view) = nav.guide(&targets) {
                log.needle(t, &targets, &view, false, false, None, None);
                needles.push(view);
            }
        }
        // The walk ends at the exit's graph node, its icon lies up to 100 units off:
        // on to the icon (the player leaves standing on it), seeing it there.
        if let Some(at) = exit_at {
            let (dx, dy) = ((at.0 - pos.0).round() as i32, (at.1 - pos.1).round() as i32);
            t += 333;
            log.frame();
            log.moved(t, dx, dy);
            nav.moved(dx as f64, dy as f64);
            let icon = Icon { kind: "exit", x: (at.0 - pos.0 - dx as f64) as f32, y: (at.1 - pos.1 - dy as f64) as f32, label: Some("G1_3".into()) };
            log.icons(t, &[icon.clone()], true, size, false);
            nav.icons(&[ExitIcon { dx: icon.x as f64, dy: icon.y as f64, to: icon.label.clone() }]);
            if let Some(view) = nav.guide(&targets) {
                log.needle(t, &targets, &view, false, false, None, None);
                needles.push(view);
            }
        }
        log.seen(t, Seen::Left("G1_3"));
        nav.seen(Seen::Left("G1_3"));
        log.end(t + 100);
        (log.text(), needles, layout.id.clone())
    }

    #[test]
    fn a_simulated_visit_replays_as_it_was_fed() {
        let (text, needles, layout) = simulated(false, 8.0);
        // To look at it: OBS_DUMP=<file>.
        if let Ok(path) = std::env::var("OBS_DUMP") {
            std::fs::write(path, &text).unwrap();
        }
        let kinds: std::collections::BTreeSet<String> =
            text.lines().filter(|l| !l.is_empty()).map(|l| serde_json::from_str::<Line>(l).unwrap()).map(|l| format!("{:?}", l.entry).split([' ', '{']).next().unwrap().to_string()).collect();
        assert_eq!(kinds.into_iter().collect::<Vec<_>>(), ["End", "Icons", "Move", "Needle", "Seen", "Start", "Targets"]);
        let visits = replay(&text, &Graphs::default());
        assert_eq!(visits.len(), 1);
        let v = &visits[0];
        eprintln!("{}{}", report(v), summary(1, &visits));
        // Every frame was handed over, and the current code gives the needles it gave.
        assert_eq!(v.frames as usize, needles.len());
        assert!(v.logged >= 5 && v.agree == v.logged, "{} of {}: {:?}", v.agree, v.logged, v.differ);
        // The exits read by name fix the true layout, and the exit taken is the target's.
        let (ids, share, read) = v.truth.clone().expect("two exits read by name");
        assert!(ids.contains(&layout) && share >= 0.9 && read >= 2, "{ids:?} {share}");
        assert_eq!(v.left.as_ref().unwrap().0, "G1_3");
        assert!(!v.unscored);
        let scored = v.replayed.count();
        assert!(scored > 50, "{:?}", v.replayed);
        let right: usize = v.replayed.by_source.iter().map(|s| s.0).sum();
        assert!(right * 10 >= scored * 8, "{:?}", v.replayed);
        // What the app showed is scored too: its needle lines.
        assert!(v.shown.count() >= 5 && v.shown.way == 0, "{:?}", v.shown);
    }

    /// Needles that followed the way through the recorded map can't be replayed (the
    /// log holds no walls) but are scored as shown; `ahead` and `fog` are written when given.
    #[test]
    fn needles_through_the_map_are_scored_as_shown() {
        let (text, _, _) = simulated(false, 8.0);
        let way = r#""way":true,"ahead":[12.0,-5.0],"fog":0.25,"distance""#;
        let text = text.replace(r#""distance""#, way);
        let visits = replay(&text, &Graphs::default());
        let v = &visits[0];
        let report = report(v);
        eprint!("{report}");
        assert!(v.mapped > 0 && v.logged == 0, "{} {}", v.mapped, v.logged);
        assert!(v.shown.count() > 0 && v.shown.way == v.shown.count(), "{:?}", v.shown);
        assert!(report.contains("shown by the app: ") && !report.contains("shown by the app: no samples"), "{report}");
        let line = summary(1, &visits);
        assert!(line.contains("shown: gold") || line.contains("shown: rough"), "{line}");
        assert!(line.contains("replayed: "), "{line}");
        // The writer.
        let mut log = Log::memory(&start("G1_2", "G1_town"), T0);
        log.frame();
        let targets = [NavTarget { label: "Mud Burrow".into(), exit: Some("G1_3".into()), ..Default::default() }];
        let view = GuideView {
            variant: String::new(), target_kind: "entrance".into(), target: "Mud Burrow".into(), degrees: Some(10.0),
            source: "rough".into(), distance: 300.0, confidence: 0.7, measured: false, turned: false,
        };
        log.needle(T0 + 1, &targets, &view, false, false, Some(Way { ahead: Some([3.5, -1.0]), fog: Some(0.4) }), None);
        log.frame();
        log.needle(T0 + 2, &targets, &GuideView { degrees: Some(50.0), ..view }, false, false, Some(Way::default()), None);
        let text = log.text();
        assert!(text.contains(r#""way":true,"ahead":[3.5,-1.0],"fog":0.4"#), "{text}");
        assert_eq!(text.matches("ahead").count(), 1, "{text}");
    }

    /// Ways ahead in shadow: the needle shown is the one without the map, and what the
    /// map would have shown rides on the needle line; the replay scores it apart, by
    /// would-be gold and rough.
    #[test]
    fn the_map_in_shadow_is_scored_apart() {
        let (text, _, _) = simulated(false, 8.0);
        // Each needle line gets a shadow turned by `turn` from the needle shown.
        let shadowed = |turn: f32, gold: bool| -> String {
            text.lines()
                .map(|l| {
                    if !l.contains(r#""k":"needle""#) {
                        return l.to_string();
                    }
                    let degrees = l.split(r#""degrees":"#).nth(1).and_then(|r| r.split(',').next()).and_then(|d| d.parse::<f32>().ok());
                    let Some(d) = degrees else { return l.to_string() };
                    let shadow = Shadow { degrees: Some(d + turn), length: 100.0, ahead: [1.0, 2.0], fog: 0.2, gold };
                    l.replace(r#""distance""#, &format!(r#""shadow":{},"distance""#, serde_json::to_string(&shadow).unwrap()))
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let same = replay(&shadowed(0.0, true), &Graphs::default());
        let v = &same[0];
        let report = report(v);
        eprint!("{report}");
        // Shown as before (the replay agrees), the shadow scored on the same frames.
        assert!(v.logged > 0 && v.agree == v.logged, "{} of {}", v.agree, v.logged);
        assert_eq!(v.shadow.count(), v.shown.count());
        let shown_right: usize = v.shown.by_source.iter().map(|s| s.0).sum();
        assert_eq!(v.shadow.by_source[0], (shown_right, v.shown.count()), "{:?} {:?}", v.shadow, v.shown);
        assert!(report.contains("map (shadow), the way ahead the app did not show: gold"), "{report}");
        assert!(summary(1, &same).contains("map (shadow): gold"));
        // Turned round and rough: counted as rough, and hardly ever within 45° (only
        // where the needle shown was itself turned away).
        let turned = replay(&shadowed(180.0, false), &Graphs::default());
        let v = &turned[0];
        let (right, all) = v.shadow.by_source[1];
        assert!(all == v.shown.count() && right * 20 < all, "{:?}", v.shadow);
        // The writer: the shadow on the line, and a shadow that turns alone is a new line.
        let mut log = Log::memory(&start("G1_2", "G1_town"), T0);
        log.frame();
        let targets = [NavTarget { label: "Mud Burrow".into(), exit: Some("G1_3".into()), ..Default::default() }];
        let view = GuideView {
            variant: String::new(), target_kind: "entrance".into(), target: "Mud Burrow".into(), degrees: Some(10.0),
            source: "rough".into(), distance: 300.0, confidence: 0.7, measured: false, turned: false,
        };
        let shadow = Shadow { degrees: Some(40.04), length: 120.4, ahead: [3.46, -1.0], fog: 0.333, gold: true };
        log.needle(T0 + 1, &targets, &view, false, false, None, Some(shadow));
        log.frame();
        log.needle(T0 + 2, &targets, &view, false, false, None, Some(Shadow { degrees: Some(80.0), ..shadow }));
        let text = log.text();
        assert!(text.contains(r#""shadow":{"degrees":40.0,"length":120.0,"ahead":[3.5,-1.0],"fog":0.33,"gold":true}"#), "{text}");
        assert_eq!(text.matches(r#""k":"needle""#).count(), 2, "{text}");
    }

    /// A visit that ends with leaving to the town where no exit icon was (a portal
    /// in the middle of the map): the town is no exit taken, nothing is scored.
    #[test]
    fn a_portal_out_is_not_scored() {
        let mut log = Log::memory(&start("G1_2", "G1_town"), T0);
        let town = [NavTarget { label: "Clearfell Encampment".into(), exit: Some("G1_town".into()), ..Default::default() }];
        let area = graph::load("G1_2");
        let mut nav = Navigator::new(&area, Arrival::From("G1_town"), None, SCALE, 11);
        let mut t = T0;
        for step in 0..30 {
            t += 333;
            log.frame();
            log.moved(t, 6, -3);
            nav.moved(6.0, -3.0);
            // An exit far away, then nothing.
            let icons = if step < 3 { vec![Icon { kind: "exit", x: 200.0 - 6.0 * step as f32, y: 90.0, label: None }] } else { vec![] };
            log.icons(t, &icons, step % 3 == 0, (300, 260), false);
            nav.icons(&icons.iter().map(|i| ExitIcon { dx: i.x as f64, dy: i.y as f64, to: None }).collect::<Vec<_>>());
            if let Some(view) = nav.guide(&town) {
                log.needle(t, &town, &view, false, false, None, None);
            }
        }
        log.seen(t, Seen::Left("G1_town"));
        log.end(t + 100);
        let visits = replay(&log.text(), &Graphs::default());
        let v = &visits[0];
        eprint!("{}", report(v));
        assert!(v.unscored && v.replayed.count() == 0 && v.shown.count() == 0 && v.replayed.right_for_good.is_none(), "{:?}", v.replayed);
        let line = summary(1, &visits);
        assert!(line.contains("0 with the exit taken, 1 unscored exit(s)"), "{line}");
    }

    #[test]
    fn a_visit_with_losses_counts_its_parts_and_scores_the_last() {
        let (text, _, _) = simulated(true, 3.0);
        if let Ok(path) = std::env::var("OBS_DUMP_LOSSES") {
            std::fs::write(path, &text).unwrap();
        }
        let v = &replay(&text, &Graphs::default())[0];
        assert!(v.parts > 1, "{}", v.parts);
        assert_eq!(v.agree, v.logged);
        // Only the last part's samples are scored.
        let scored = v.replayed.count();
        assert!(scored > 0 && (scored as u64) < v.frames);
    }

    #[test]
    fn lines_round_trip_and_a_torn_line_is_skipped() {
        let mut log = Log::memory(&start("G1_2", ""), T0);
        log.frame();
        log.moved(T0 + 1, 0, 0);
        log.moved(T0 + 2, 3, -2);
        let exit = Icon { kind: "exit", x: -136.23457, y: 83.1, label: Some("The Grelwood".into()) };
        let cp = Icon { kind: "checkpoint", x: 12.5, y: -40.25, label: None };
        log.icons(T0 + 3, &[exit.clone(), cp.clone()], true, (300, 260), false);
        // The same icons again on a frame between scans: not written.
        log.frame();
        log.icons(T0 + 4, &[exit.clone(), cp.clone()], false, (300, 260), false);
        // A whole scan with an exit is written even when the same.
        log.frame();
        log.icons(T0 + 5, &[exit.clone(), cp], true, (300, 260), false);
        log.lost(T0 + 6, 4.25);
        log.seen(T0 + 7, Seen::Tag("boss"));
        log.end(T0 + 8);
        let text = log.text();
        let mut last = 0;
        let lines: Vec<Line> = text
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| {
                let mut line: Line = serde_json::from_str(l).unwrap();
                line.entry.implied(&mut last, false);
                line
            })
            .collect();
        assert_eq!(lines.len(), 7, "{text}");
        assert!(matches!(lines[1].entry, Entry::Move { f: 1, .. }) && matches!(lines[2].entry, Entry::Icons { f: 1, .. }));
        assert!(text.lines().nth(2).unwrap().starts_with(r#"{"t":1762945000002,"k":"move","dx":3,"dy":-2}"#), "{text}");
        let Entry::Icons { icons, size, whole, .. } = &lines[2].entry else { panic!() };
        assert_eq!((icons[0].x, icons[0].label.as_deref(), *size, *whole), (-136.23457, Some("The Grelwood"), Some([300, 260]), true));
        let Entry::Icons { size, f, .. } = &lines[3].entry else { panic!() };
        assert_eq!((*size, *f), (None, 3));
        // The icons of the move's frame leave their frame out.
        assert!(!text.lines().nth(3).unwrap().contains(r#""f":"#), "{text}");
        let Entry::Start { version, .. } = &lines[0].entry else { panic!() };
        assert_eq!(version, app_version());
        assert!(!app_version().starts_with("0.1.0"));
        // A line cut short by a crash does not stop the replay.
        let torn = format!("{text}{{\"t\":1,\"k\":\"mo");
        let v = &replay(&torn, &Graphs::default())[0];
        // Cut at the end: nothing after it is missing.
        assert!(v.frames == 3 && !v.incomplete);
        // The file begins with a newline: a torn line left by an earlier crash ends there.
        assert!(text.starts_with("\n{"), "{text}");
    }

    /// The first writes fail (the area's folder cannot be made), the later ones work.
    #[test]
    fn a_failed_write_leaves_a_gap_and_the_visit_is_incomplete() {
        let dir = std::env::temp_dir().join(format!("poe-batonga-test-gap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("G1_2");
        std::fs::write(&blocker, "a file where the folder should be").unwrap();
        let path = dir.join("G1_2").join("11.obs.jsonl");
        let mut log = Log::new(path.clone(), None, &start("G1_2", "G1_town"), T0);
        let mut t = T0;
        for step in 0..20 {
            if step == 10 {
                std::fs::remove_file(&blocker).unwrap();
            }
            t += 333;
            log.frame();
            log.moved(t, 3, 1);
            log.icons(t, &[], false, (300, 260), false);
        }
        log.seen(t, Seen::Left("G1_3"));
        log.end(t + 100);
        let text = std::fs::read_to_string(&path).unwrap();
        // The start line is written again, then the gap with its frame: the lines after
        // it number theirs from there.
        assert!(text.starts_with("\n{\"t\":1762945000000,\"k\":\"start\""), "{text}");
        assert!(text.lines().any(|l| l.contains("\"k\":\"gap\",\"f\":")), "{text}");
        let visits = replay(&text, &Graphs::default());
        let v = &visits[0];
        eprint!("{}", report(v));
        // Every frame is counted, also those whose lines were lost.
        assert!(v.incomplete && v.ended && v.frames == 20, "{v:?}");
        assert!(summary(1, &visits).contains("(0 skipped, 1 incomplete)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One scan in the middle reads the Mud Burrow's icon as another exit: the truth
    /// stays as it was (a name counts once read the same in two scans, by median).
    #[test]
    fn one_misread_scan_leaves_the_truth_as_it_was() {
        let (clean_text, misread_text) = (simulated_with(false, 8.0, None).0, simulated_with(false, 8.0, Some(30)).0);
        let named = |text: &str| text.matches(r#""label":"G1_4""#).count();
        assert_eq!(named(&misread_text), named(&clean_text) + 1, "one scan misread");
        let truth = |text: &str| replay(text, &Graphs::default())[0].truth.clone().unwrap();
        let (clean, misread) = (truth(&clean_text), truth(&misread_text));
        assert_eq!(clean.0, misread.0);
        // The misread name was read once there: it does not count (a mean of every
        // reading would move the Grelwood about 50 cells towards Mud Burrow).
        let place = |text: &str| replay(text, &Graphs::default())[0].places["G1_4"];
        assert_eq!(place(&clean_text), place(&misread_text));
        assert!((clean.1 - misread.1).abs() < 0.01 && clean.2 == misread.2, "{clean:?} {misread:?}");
    }

    /// A line torn in the middle of a file (a full disk) loses that line only.
    #[test]
    fn a_torn_middle_line_keeps_the_lines_after_it() {
        let (text, _, _) = simulated(false, 8.0);
        let lines: Vec<&str> = text.lines().collect();
        let k = lines.len() / 2;
        let torn: String = lines.iter().enumerate().map(|(i, l)| if i == k { format!("{}\n", &l[..l.len() / 2]) } else { format!("{l}\n") }).collect();
        let v = &replay(&torn, &Graphs::default())[0];
        assert!(v.incomplete && v.ended && v.left.is_some(), "{v:?}");
        assert!(!replay(&text, &Graphs::default())[0].incomplete);
    }

    #[test]
    fn nothing_is_written_without_a_frame() {
        // A log catch-up: a zone entered long ago, no frames, an anchor.
        let mut log = Log::memory(&start("G1_2", "G1_town"), T0);
        log.seen(T0 + 1, Seen::Exit("G1_3"));
        log.end(T0 + 2);
        assert_eq!(log.text(), "");
    }

    #[test]
    fn needles_are_written_when_they_change_or_every_five_seconds() {
        let mut log = Log::memory(&start("G1_2", "G1_town"), T0);
        let targets = [NavTarget { label: "Mud Burrow".into(), exit: Some("G1_3".into()), ..Default::default() }];
        let view = |deg: f32, source: &str| GuideView {
            variant: String::new(), target_kind: "entrance".into(), target: "Mud Burrow".into(), degrees: Some(deg),
            source: source.into(), distance: 300.0, confidence: 0.7, measured: false, turned: false,
        };
        let mut t = T0;
        for (deg, source) in [(10.0, "rough"), (20.0, "rough"), (30.0, "rough"), (31.0, "estimated"), (31.0, "estimated")] {
            t += 900;
            log.frame();
            log.needle(t, &targets, &view(deg, source), false, false, None, None);
        }
        // Then the same needle, frames still coming, for 6 s.
        for _ in 0..7 {
            t += 900;
            log.frame();
            log.needle(t, &targets, &view(31.0, "estimated"), false, false, None, None);
        }
        let text = log.text();
        let needles: Vec<u64> = text.lines().filter(|l| l.contains(r#""k":"needle""#)).map(|l| serde_json::from_str::<Line>(l).unwrap().t - T0).collect();
        // 10°, then 30° (20° from 10°), gold, and 5 s later the same again.
        assert_eq!(needles, [900, 2700, 3600, 9000], "{text}");
        assert_eq!(text.matches(r#""k":"targets""#).count(), 1);
    }

    #[test]
    fn the_history_takes_a_layout_the_named_exits_fixed_once() {
        let dir = std::env::temp_dir().join(format!("poe-batonga-test-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let history = dir.join(HISTORY_FILE);
        let mut log = Log::new(dir.join("G1_2").join("11.obs.jsonl"), Some(history.clone()), &start("G1_2", "G1_town"), T0);
        let mut memory = nav::Memory {
            layouts: [("A".to_string(), 0.95), ("B".to_string(), 0.05)].into(),
            named: [("A".to_string(), 0.0), ("B".to_string(), 0.0)].into(),
            ..Default::default()
        };
        // Sure, but not from named exits: nothing.
        log.history(T0, &memory);
        assert!(!history.exists());
        memory.named = [("A".to_string(), -0.1), ("B".to_string(), -6.0)].into();
        log.history(T0 + 1, &memory);
        log.history(T0 + 2, &memory);
        let text = std::fs::read_to_string(&history).unwrap();
        assert_eq!(text.lines().count(), 1, "{text}");
        let line: HistoryLine = serde_json::from_str(text.trim()).unwrap();
        assert_eq!((line.area.as_str(), line.seed, line.layout.as_str(), line.by.as_str()), ("G1_2", Some(11), "A", "named"));
        assert!(line.share == 0.95 && line.named > 0.99);
        // The named fit favours another layout than the belief's top: not fixed by names.
        memory.layouts = [("A".to_string(), 0.05), ("B".to_string(), 0.95)].into();
        log.history(T0 + 3, &memory);
        assert_eq!(std::fs::read_to_string(&history).unwrap().lines().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_recordings_are_deleted() {
        let dir = std::env::temp_dir().join(format!("poe-batonga-test-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("G1_2")).unwrap();
        let (old, new, other) = (dir.join("G1_2/1.obs.jsonl"), dir.join("G1_2/2.obs.jsonl"), dir.join("G1_2/1.json"));
        for path in [&old, &new, &other] {
            std::fs::write(path, "{}\n").unwrap();
        }
        let ago = |days: u64| std::time::SystemTime::now() - std::time::Duration::from_secs(days * 24 * 3600);
        let set = |path: &PathBuf, days: u64| std::fs::File::options().write(true).open(path).unwrap().set_modified(ago(days)).unwrap();
        // A month old is kept now (it was deleted at 30 days); half a year is not.
        let month = dir.join("G1_2/3.obs.jsonl");
        std::fs::write(&month, "{}\n").unwrap();
        set(&old, 181);
        set(&other, 181);
        set(&month, 31);
        assert_eq!(prune(&dir), 1);
        assert!(!old.exists() && new.exists() && month.exists() && other.exists());
        // Over the size limit: the oldest go until the rest fit.
        for (i, days) in [(4, 3), (5, 2), (6, 1)] {
            let path = dir.join(format!("G1_2/{i}.obs.jsonl"));
            std::fs::write(&path, vec![b'x'; 100]).unwrap();
            set(&path, days);
        }
        // 3 (31 days, 3 bytes), 4, 5, 6 (100 bytes each), 2 (3 bytes): 306 bytes in all.
        let year = std::time::Duration::from_secs(365 * 24 * 3600);
        assert_eq!(prune_with(&dir, year, 210), (2, 103));
        assert!(!month.exists() && !dir.join("G1_2/4.obs.jsonl").exists() && dir.join("G1_2/5.obs.jsonl").exists() && new.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An hour of play with the big map open: the walker's moves and exits, two
    /// checkpoints beside each exit, a whole-window scan every third frame (names,
    /// all icons) and only icons inside the wall frame between, a needle per second.
    /// The walker moves on 99% of the frames (the owner stands, fights and reads
    /// more; towns write nothing): this is the upper end of an hour.
    #[test]
    fn an_hour_of_play_stays_near_a_megabyte() {
        // Walking all the time with the big map open: the worst case.
        let worst = hour(1.0);
        // The big map open 70% of the frames (the minimap shows only exits near the player).
        let mixed = hour(0.7);
        assert!(mixed < 1_200_000 && worst < 1_350_000, "{mixed} {worst}");
    }

    fn hour(open: f64) -> usize {
        let area = graph::load("G1_2");
        let target = NavTarget { exit: Some("G1_3".into()), ..Default::default() };
        let mut log = Log::memory(&start("G1_2", "G1_town"), T0);
        let (mut t, mut frames, mut seed) = (T0, 0usize, 0u64);
        let options = Options { view: Some(3.0), labels: true, ..Default::default() };
        while frames < 3 * 3600 {
            let layout = &area.layouts[seed as usize % area.layouts.len()];
            seed += 1;
            let goals = layout.goals(&target);
            let Some(from) = layout.goals(&NavTarget { exit: Some("G1_town".into()), ..Default::default() }).first().copied() else { continue };
            let Some(events) = walk(layout, from, &goals, seed, &options) else { continue };
            for event in events {
                frames += 1;
                t += 333;
                log.frame();
                let big = (frames / 30) % 10 < (open * 10.0) as usize;
                let whole = big && frames % 3 == 0;
                let mut icons = Vec::new();
                match event {
                    Event::Lost(s) => log.lost(t, s),
                    Event::Moved { dx, dy, icons: seen, .. } => {
                        log.moved(t, dx as i32, dy as i32);
                        for e in seen.iter().filter(|e| whole || (e.dx.abs() <= 75.0 && e.dy.abs() <= 65.0)) {
                            let label = e.to.clone().filter(|_| whole).map(|id| area.names.get(&id).cloned().unwrap_or(id));
                            icons.push(Icon { kind: "exit", x: e.dx as f32, y: e.dy as f32, label });
                            for (ox, oy) in [(40.0, -25.0), (-60.0, 35.0)] {
                                let (x, y) = (e.dx + ox, e.dy + oy);
                                if whole || (x.abs() <= 75.0 && y.abs() <= 65.0) {
                                    icons.push(Icon { kind: "checkpoint", x: x as f32, y: y as f32, label: None });
                                }
                            }
                        }
                    }
                }
                log.icons(t, &icons, whole, (300, 260), false);
                if frames % 3 == 0 {
                    // A needle that turns a little every second, and changes source now and then.
                    let deg = (frames as f32 * 2.7) % 360.0;
                    let view = GuideView {
                        variant: String::new(), target_kind: "entrance".into(), target: "Mud Burrow".into(), degrees: Some(deg),
                        source: if frames % 30 < 15 { "rough".into() } else { "estimated".into() },
                        distance: 312.4, confidence: 0.734, measured: false, turned: false,
                    };
                    log.needle(t, &[NavTarget { label: "Mud Burrow".into(), ..target.clone() }], &view, false, false, None, None);
                }
            }
        }
        log.end(t);
        let text = log.text();
        let count = |k: &str| text.matches(&format!("\"k\":\"{k}\"")).count();
        let bytes = text.len() * 3 * 3600 / frames;
        eprintln!(
            "an hour, big map open {:.0}%: {bytes} bytes; {} move, {} icons, {} needle, {} lost lines in {frames} frames",
            100.0 * open, count("move"), count("icons"), count("needle"), count("lost")
        );
        bytes
    }

    /// Replays flight recordings:
    /// `OBS=<file or folder> cargo test --release replay_obs -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn replay_obs() {
        let root = std::path::PathBuf::from(std::env::var("OBS").expect("set OBS to a .obs.jsonl file or a folder"));
        let mut files = Vec::new();
        let mut stack = vec![root];
        while let Some(path) = stack.pop() {
            if path.is_dir() {
                stack.extend(std::fs::read_dir(&path).unwrap().flatten().map(|e| e.path()));
            } else if path.to_string_lossy().ends_with(".obs.jsonl") {
                files.push(path);
            }
        }
        files.sort();
        let graphs = Graphs::default();
        let mut all = Vec::new();
        for path in &files {
            println!("\n# {}", path.display());
            let visits = replay(&std::fs::read_to_string(path).unwrap(), &graphs);
            for v in &visits {
                print!("{}", report(v));
            }
            all.extend(visits);
        }
        println!("\n{}", summary(files.len(), &all));
    }
}
