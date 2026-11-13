//! The game's quest tracker, read off the screen: kills and talks the log never shows.
//!
//! The log has no line when a boss dies or an NPC is talked to (measured 2026-11-13:
//! nothing between the Bloated Miller's last bark and Mortimer's line at the gate two
//! minutes later; nothing during Renly's reward talk). The tracker under the minimap
//! changes at once: "Kill the Bloated Miller and end his rage" becomes "Enter the
//! logging encampment". Its lines are the `MapPinsText` of the quest's state in the
//! game's tables (`data/quests/states.json`, from `scripts/import-quest-states.mjs`).
//!
//! A route step names, in `screen`, the tracker states that prove it done (see
//! `Condition`). While the player is in a zone with such a step open, the right column
//! under the minimap is captured about once a second and read by OCR (Windows; no
//! game memory, no input). A quest's state counts when two reads in a row agree.
//! A quest that leaves the list after its last state (its reward was taken) counts as
//! complete when three reads in a row show other quests but not it.
//! See docs/quest-tracker.md.
#![cfg_attr(not(target_os = "windows"), allow(dead_code))]
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

/// The tracker's column, in client heights, anchored to the right edge: from below
/// the minimap down past the longest list seen. The Miller's tracker (2488x1336,
/// 2026-11-13) has its name at 0.28 and its line at 0.31, ending 0.28 heights left
/// of the edge; longer lines and more quests need more room.
pub const TRACKER_TOP: f64 = 0.22;
pub const TRACKER_HEIGHT: f64 = 0.63;
pub const TRACKER_WIDTH: f64 = 0.75;

/// Whether this platform reads the tracker (the Linux capture is not written yet).
pub const READS: bool = cfg!(target_os = "windows");

#[derive(Debug, Clone, Deserialize)]
pub struct Quest {
    pub name: String,
    #[allow(dead_code)]
    pub act: u32,
    pub states: Vec<State>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct State {
    pub order: u32,
    pub lines: Vec<String>,
    #[serde(default)]
    pub complete: bool,
}

#[derive(Deserialize)]
struct StatesFile {
    quests: BTreeMap<String, Quest>,
}

/// Every campaign quest by id (`TheMiller`), from `data/quests/states.json`.
pub fn quests() -> &'static BTreeMap<String, Quest> {
    static QUESTS: OnceLock<BTreeMap<String, Quest>> = OnceLock::new();
    QUESTS.get_or_init(|| {
        serde_json::from_str::<StatesFile>(include_str!("../../data/quests/states.json"))
            .unwrap_or_else(|e| panic!("data/quests/states.json: {e}"))
            .quests
    })
}

/// A step's `screen` entry: which states of `quest` prove the step done.
/// - `past`: every state further on than the first state showing this line (a lower
///   `order`), the quest's completion included. For quests that go one way only.
/// - `from`: the first state showing this line (the highest `order`) and every state
///   further on. For a step whose line before it is said again elsewhere in the quest
///   ("Speak to Alva"), or the n-th of several like steps (Act 4's islands).
/// - `lines`: the states showing any of these lines ("Quest Complete": the quest left
///   the list after its last state). For quests with branches (two bosses in either order).
/// - none of them: any state of the quest (it was given).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub quest: String,
    #[serde(default)]
    pub past: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub lines: Vec<String>,
    /// Filled by `resolve`.
    #[serde(skip)]
    proof: BTreeSet<u32>,
}

impl PartialEq for Condition {
    fn eq(&self, other: &Self) -> bool {
        self.quest == other.quest && self.past == other.past && self.from == other.from && self.lines == other.lines
    }
}

impl Condition {
    /// Works out the proving states; an unknown quest or line is an error.
    pub fn resolve(&mut self, quests: &BTreeMap<String, Quest>) -> Result<(), String> {
        let quest = quests.get(&self.quest).ok_or_else(|| format!("screen: unknown quest {}", self.quest))?;
        let shows = |state: &State, text: &str| state.lines.iter().any(|l| normalize(l) == normalize(text));
        let orders_of = |text: &str| -> Result<Vec<u32>, String> {
            let found: Vec<u32> = quest.states.iter().filter(|s| shows(s, text)).map(|s| s.order).collect();
            if found.is_empty() {
                return Err(format!("screen ({}): no state shows {text:?}", self.quest));
            }
            Ok(found)
        };
        let forms = usize::from(self.past.is_some()) + usize::from(self.from.is_some()) + usize::from(!self.lines.is_empty());
        if forms > 1 {
            return Err(format!("screen ({}): one of past, from or lines", self.quest));
        }
        self.proof = match (&self.past, &self.from, self.lines.is_empty()) {
            (Some(past), _, _) => {
                let first = *orders_of(past)?.iter().min().unwrap();
                quest.states.iter().map(|s| s.order).filter(|&o| o < first).collect()
            }
            (_, Some(from), _) => {
                let earliest = *orders_of(from)?.iter().max().unwrap();
                quest.states.iter().map(|s| s.order).filter(|&o| o <= earliest).collect()
            }
            (None, None, false) => {
                let mut proof = BTreeSet::new();
                for line in &self.lines {
                    proof.extend(orders_of(line)?);
                }
                proof
            }
            (None, None, true) => quest.states.iter().map(|s| s.order).collect(),
        };
        if self.proof.is_empty() {
            return Err(format!("screen ({}): no state proves it", self.quest));
        }
        Ok(())
    }

    /// Whether the quest's state, read as one of `orders`, proves the step done: every
    /// state the read could be is a proving one.
    pub fn proves(&self, quest: &str, orders: &BTreeSet<u32>) -> bool {
        quest == self.quest && !orders.is_empty() && orders.is_subset(&self.proof)
    }
}

/// Text as compared: lower case ASCII letters and digits, apostrophes dropped
/// ("Renly's" and an OCR "Renlys" agree), anything else one space.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = true;
    for c in text.chars() {
        if matches!(c, '\'' | '\u{2019}' | '`') {
            continue;
        }
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            space = false;
        } else if !space {
            out.push(' ');
            space = true;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

fn distance(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, &ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let next = (row[j + 1] + 1).min(row[j] + 1).min(diagonal + usize::from(ca != cb));
            diagonal = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

/// The one entry `text` reads as: within `per` (one edit per that many letters; none
/// below eight letters). Two different entries equally close are no reading.
fn closest<'a, T>(entries: &'a [(String, T)], text: &str, per: usize) -> Option<&'a T> {
    let tolerance = if text.len() < 8 { 0 } else { text.len() / per };
    let mut best: Option<(usize, &(String, T))> = None;
    let mut tie = false;
    for entry in entries {
        if entry.0.len().abs_diff(text.len()) > tolerance {
            continue;
        }
        let d = distance(&entry.0, text);
        if d > tolerance {
            continue;
        }
        match best {
            Some((b, _)) if d > b => {}
            Some((b, _)) if d == b => tie = true,
            _ => {
                best = Some((d, entry));
                tie = false;
            }
        }
    }
    best.filter(|_| !tie).map(|(_, entry)| &entry.1)
}

struct Index {
    /// Normalized quest name, the quests of that name.
    names: Vec<(String, Vec<String>)>,
    /// Normalized tracker line, the (quest, state order) pairs showing it.
    lines: Vec<(String, Vec<(String, u32)>)>,
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut names: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut lines: BTreeMap<String, Vec<(String, u32)>> = BTreeMap::new();
        for (id, quest) in quests() {
            names.entry(normalize(&quest.name)).or_default().push(id.clone());
            for state in &quest.states {
                for line in &state.lines {
                    lines.entry(normalize(line)).or_default().push((id.clone(), state.order));
                }
            }
        }
        Index { names: names.into_iter().collect(), lines: lines.into_iter().collect() }
    })
}

/// A quest read in the tracker: the states its lines fit, and the lines.
#[derive(Debug, Clone, PartialEq)]
pub struct Seen {
    pub orders: BTreeSet<u32>,
    pub lines: Vec<String>,
}

/// One read of the tracker column: OCR lines, top to bottom.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reading {
    pub quests: BTreeMap<String, Seen>,
    /// Quest names recognized: a read with none shows no tracker (a panel covers it).
    pub headers: usize,
    /// The quests read, top to bottom.
    pub listed: Vec<String>,
}

/// Reads the tracker's lines. A quest name starts its block; a line is taken for the
/// quest of its block, or, with no name read above it, only when one quest has it.
/// A line wrapped onto two OCR lines is read joined. A stray mark the OCR makes of the
/// line's bullet is dropped.
pub fn read(ocr: &[String]) -> Reading {
    let index = index();
    let texts: Vec<String> = ocr.iter().map(|l| normalize(l)).collect();
    let variants = |text: &str| -> Vec<String> {
        let mut out = vec![text.to_string()];
        if let Some((first, rest)) = text.split_once(' ') {
            if first.len() == 1 {
                out.push(rest.to_string());
            }
        }
        out
    };
    let line_of = |text: &str| variants(text).iter().find_map(|v| closest(&index.lines, v, 10));
    let mut reading = Reading::default();
    let mut block: Option<&Vec<String>> = None;
    let mut i = 0;
    while i < texts.len() {
        let text = &texts[i];
        if text.len() < 4 {
            i += 1;
            continue;
        }
        if let Some(ids) = variants(text).iter().find_map(|v| closest(&index.names, v, 8)) {
            block = Some(ids);
            reading.headers += 1;
            i += 1;
            continue;
        }
        let (found, used) = match line_of(text) {
            Some(found) => (Some(found), 1),
            None => match texts.get(i + 1) {
                Some(next) => (line_of(&format!("{text} {next}")), 2),
                None => (None, 1),
            },
        };
        let Some(found) = found else {
            i += 1;
            continue;
        };
        let mut pairs: Vec<&(String, u32)> = found.iter().filter(|(q, _)| block.is_some_and(|ids| ids.contains(q))).collect();
        if pairs.is_empty() {
            pairs = found.iter().collect();
        }
        let quest = &pairs[0].0;
        if pairs.iter().any(|(q, _)| q != quest) {
            i += used;
            continue;
        }
        let orders: BTreeSet<u32> = pairs.iter().map(|(_, o)| *o).collect();
        let shown = ocr[i..i + used].join(" ");
        if !reading.listed.contains(quest) {
            reading.listed.push(quest.clone());
        }
        let seen = reading.quests.entry(quest.clone()).or_insert_with(|| Seen { orders: orders.clone(), lines: vec![] });
        // A state with two lines: both read narrow it down.
        let both: BTreeSet<u32> = seen.orders.intersection(&orders).copied().collect();
        seen.orders = if both.is_empty() { orders } else { both };
        seen.lines.push(shown);
        i += used;
    }
    reading
}

/// A quest state confirmed by the reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Confirmed {
    pub quest: String,
    pub orders: BTreeSet<u32>,
    pub evidence: String,
}

/// Reads in a row that must show a quest gone before it counts as complete.
const GONE_READS: u32 = 3;
/// A read with at most this many quests shows the whole list: the column has room for
/// more, so a quest missing from it was not cut off at the bottom.
const SHORT_LIST: usize = 3;

/// Turns reads into confirmed states: the same state twice in a row. A quest that leaves
/// the list counts as complete when it was at one of its last two states, `GONE_READS`
/// reads in a row show other quests but not it, and the list cannot have cut it off: a
/// quest that was below it still shows, or the list is short.
#[derive(Debug, Default)]
pub struct Watcher {
    last: BTreeMap<String, BTreeSet<u32>>,
    confirmed: BTreeMap<String, BTreeSet<u32>>,
    missing: BTreeMap<String, u32>,
    /// The quests listed below each quest when it was last read.
    below: BTreeMap<String, Vec<String>>,
}

impl Watcher {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn frame(&mut self, reading: &Reading) -> Vec<Confirmed> {
        let quests = quests();
        let mut out = Vec::new();
        for (quest, seen) in &reading.quests {
            if self.last.get(quest) == Some(&seen.orders) {
                let name = quests.get(quest).map_or(quest.as_str(), |q| q.name.as_str());
                out.push(Confirmed {
                    quest: quest.clone(),
                    orders: seen.orders.clone(),
                    evidence: format!("Quest tracker: {name}: {}", seen.lines.join(" / ")),
                });
                self.confirmed.insert(quest.clone(), seen.orders.clone());
            }
            self.missing.remove(quest);
        }
        for (at, quest) in reading.listed.iter().enumerate() {
            self.below.insert(quest.clone(), reading.listed[at + 1..].to_vec());
        }
        // Only a read that shows the tracker (a name, another quest) can show a quest gone.
        if reading.headers > 0 && !reading.quests.is_empty() {
            let gone: Vec<String> = self.confirmed.keys().filter(|q| !reading.quests.contains_key(*q)).cloned().collect();
            for quest in gone {
                let Some(data) = quests.get(&quest) else { continue };
                // Its last two states (a reward talk, the step before it): a quest further
                // back that seems gone was misread, not finished.
                let mut open: Vec<u32> = data.states.iter().filter(|s| !s.complete).map(|s| s.order).collect();
                open.sort_unstable();
                let near_end = open.get(1).or(open.first()).is_some_and(|&end| self.confirmed[&quest].iter().all(|&o| o <= end));
                let below_shows = self.below.get(&quest).is_some_and(|b| b.iter().any(|q| reading.quests.contains_key(q)));
                if !(near_end && (below_shows || reading.quests.len() <= SHORT_LIST)) {
                    self.missing.remove(&quest);
                    continue;
                }
                let count = self.missing.entry(quest.clone()).or_default();
                *count += 1;
                if *count >= GONE_READS {
                    let orders: BTreeSet<u32> = data.states.iter().filter(|s| s.complete).map(|s| s.order).collect();
                    out.push(Confirmed {
                        quest: quest.clone(),
                        orders,
                        evidence: format!("Quest tracker: {} left the list after its last step", data.name),
                    });
                    self.confirmed.remove(&quest);
                    self.missing.remove(&quest);
                    self.below.remove(&quest);
                }
            }
        }
        self.last = reading.quests.iter().map(|(q, s)| (q.clone(), s.orders.clone())).collect();
        out
    }
}

/// Starts reading the tracker (Windows; elsewhere nothing).
pub fn spawn(app: tauri::AppHandle) {
    #[cfg(target_os = "windows")]
    windows::spawn(app);
    #[cfg(not(target_os = "windows"))]
    let _ = app;
}

#[cfg(target_os = "windows")]
#[path = "quest_screen_windows.rs"]
mod windows;

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|t| t.to_string()).collect()
    }

    fn condition(quest: &str, past: Option<&str>, lines: &[&str]) -> Condition {
        let mut c = Condition {
            quest: quest.into(),
            past: past.map(Into::into),
            from: None,
            lines: lines.iter().map(|l| l.to_string()).collect(),
            proof: BTreeSet::new(),
        };
        c.resolve(quests()).unwrap();
        c
    }

    #[test]
    fn the_millers_tracker_after_the_kill() {
        // OCR of the owner's screenshot (2026-11-13, Windows.Media.Ocr on the column).
        let reading = read(&lines(&["Reaching Clearfell", "Enter the logging encampment"]));
        assert_eq!(reading.headers, 1);
        assert_eq!(reading.quests["TheMiller"].orders, BTreeSet::from([4]));
        let kill = condition("TheMiller", Some("Kill the Bloated Miller and end his rage"), &[]);
        assert!(kill.proves("TheMiller", &reading.quests["TheMiller"].orders));
        let reward = condition("TheMiller", Some("Take Renly's reward"), &[]);
        assert!(!reward.proves("TheMiller", &reading.quests["TheMiller"].orders));
        // Before the kill the step stays open.
        let before = read(&lines(&["Reaching Clearfell", "Kill the Bloated Miller and end his rage"]));
        assert!(!kill.proves("TheMiller", &before.quests["TheMiller"].orders));
    }

    #[test]
    fn misreads_bullets_and_wrapped_lines() {
        let reading = read(&lines(&["Reachinq Clearfell", "• Enter the loqging encampment"]));
        assert_eq!(reading.quests["TheMiller"].orders, BTreeSet::from([4]));
        let wrapped = read(&lines(&["Secrets in the Dark", "Find the entrance to the Red Vale in the", "Grelwood"]));
        assert!(wrapped.quests["TreeOfSouls"].orders.contains(&28));
        // Short words and labels in the column match nothing.
        assert!(read(&lines(&["Renly", "Rusted Cuirass", "Una", "Checkpoint"])).quests.is_empty());
    }

    #[test]
    fn a_shared_line_needs_its_quest_name() {
        // "Pick up Draven's Memorial Key Piece" is in one quest only; "Speak to Una" in several.
        let shared: Vec<&String> = index().lines.iter().filter(|(_, p)| {
            let first = &p[0].0;
            p.iter().any(|(q, _)| q != first)
        }).map(|(l, _)| l).collect();
        assert!(!shared.is_empty());
        let line = shared[0].clone();
        assert!(read(&[line.clone()]).quests.is_empty());
    }

    #[test]
    fn two_reads_confirm_and_a_finished_quest_leaves() {
        let mut watcher = Watcher::default();
        let reward = read(&lines(&["Reaching Clearfell", "Take Renly's reward", "Treacherous Ground", "Kill the Devourer"]));
        assert!(watcher.frame(&reward).is_empty());
        let confirmed = watcher.frame(&reward);
        assert_eq!(confirmed.len(), 2);
        // Renly's talk: the Miller's quest is gone, the other one still shows.
        let after = read(&lines(&["Treacherous Ground", "Kill the Devourer"]));
        assert!(watcher.frame(&after).iter().all(|c| c.quest != "TheMiller"));
        assert!(watcher.frame(&after).iter().all(|c| c.quest != "TheMiller"));
        let done = watcher.frame(&after);
        let miller = done.iter().find(|c| c.quest == "TheMiller").expect("quest left the list");
        let talk = condition("TheMiller", Some("Take Renly's reward"), &[]);
        assert!(talk.proves("TheMiller", &miller.orders));
        // A covered tracker (nothing read) shows nothing gone.
        let mut watcher = Watcher::default();
        watcher.frame(&reward);
        watcher.frame(&reward);
        for _ in 0..5 {
            assert!(watcher.frame(&Reading::default()).is_empty());
        }
    }

    #[test]
    fn a_quest_gone_before_its_last_state_is_not_complete() {
        let mut watcher = Watcher::default();
        let kill = read(&lines(&["Reaching Clearfell", "Kill the Bloated Miller and end his rage", "Treacherous Ground", "Kill the Devourer"]));
        watcher.frame(&kill);
        watcher.frame(&kill);
        let other = read(&lines(&["Treacherous Ground", "Kill the Devourer"]));
        for _ in 0..5 {
            assert!(watcher.frame(&other).iter().all(|c| c.quest != "TheMiller"));
        }
    }

    #[test]
    fn a_reward_taken_at_once_still_finishes_the_quest() {
        // "Take Renly's reward" may show for less than two reads: the Miller's quest is
        // gone straight from "Talk to the Blacksmith", and Renly's next quest shows.
        let mut watcher = Watcher::default();
        let town = read(&lines(&["Reaching Clearfell", "Talk to the Blacksmith"]));
        watcher.frame(&town);
        watcher.frame(&town);
        let after = read(&lines(&["Treacherous Ground", "Search Clearfell to find the Mud Burrow entrance", "Slay the Devourer in its lair"]));
        let mut done = Vec::new();
        for _ in 0..3 {
            done.extend(watcher.frame(&after));
        }
        assert!(done.iter().any(|c| c.quest == "TheMiller" && c.orders == BTreeSet::from([0])));
    }

    #[test]
    fn a_long_list_may_have_cut_a_quest_off() {
        let mut watcher = Watcher::default();
        let long = read(&lines(&[
            "Secrets in the Dark", "Travel through Clearfell to the Grelwood",
            "Treacherous Ground", "Kill the Devourer",
            "The Lost Lute", "Retrieve Una's Lute",
            "Ominous Altars", "Defeat the King in the Mists",
            "Reaching Clearfell", "Talk to the Blacksmith",
        ]));
        watcher.frame(&long);
        watcher.frame(&long);
        let cut = read(&lines(&[
            "Secrets in the Dark", "Travel through Clearfell to the Grelwood",
            "Treacherous Ground", "Kill the Devourer",
            "The Lost Lute", "Retrieve Una's Lute",
            "Ominous Altars", "Defeat the King in the Mists",
        ]));
        for _ in 0..5 {
            assert!(watcher.frame(&cut).iter().all(|c| c.quest != "TheMiller"));
        }
    }

    #[test]
    fn branches_by_lines() {
        // Asinia first: her key piece picked up, then Draven: "Pick up Draven's Memorial
        // Key Piece" with Asinia's key held (order 14) proves Draven dead.
        let draven = condition("GraveyardFight", None, &["Pick up Draven's Memorial Key Piece"]);
        assert!(draven.proves("GraveyardFight", &BTreeSet::from([9, 14])));
        assert!(!draven.proves("GraveyardFight", &BTreeSet::from([15])));
    }

    #[test]
    fn bad_conditions_fail() {
        let mut c = Condition { quest: "Nope".into(), past: None, from: None, lines: vec![], proof: BTreeSet::new() };
        assert!(c.resolve(quests()).is_err());
        let mut c = Condition { quest: "TheMiller".into(), past: Some("Not a line".into()), from: None, lines: vec![], proof: BTreeSet::new() };
        assert!(c.resolve(quests()).is_err());
    }
}
