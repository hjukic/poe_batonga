//! Read-only Client.txt tracking for the campaign checklists (one per zone).
//!
//! The game's log is only read, never written. Lines become events (zone entered,
//! NPC dialogue), and events complete route steps whose trigger they match exactly.
//! Progress, including how far the log has been read, is saved locally so events
//! that happen while the overlay is closed are picked up on the next start.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter, Manager};

const PROGRESS_FILE: &str = "progress.json";
/// Progress saved before there was more than one checklist.
const OLD_PROGRESS_FILE: &str = "riverbank-progress.json";
/// On first run, look this far back for the latest game start to replay from.
const FIRST_RUN_LOOKBACK: u64 = 8 * 1024 * 1024;
/// Upper bound on bytes read per poll, so a huge backlog can't stall the UI.
const MAX_READ: u64 = 4 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(500);

// ---------- Route data ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Zone {
    pub id: String,
    pub name: String,
    pub level: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Part {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

/// One NPC line a trigger waits for. `text` is kept in its shown form (`shown_text`).
/// An unknown field fails to load, so a misspelt key is not silently ignored.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpcLine {
    pub speaker: String,
    pub text: String,
    /// The game-table row the line comes from (NPCTextAudio or NPCTalkDialogueTextAudio),
    /// for scripts/check-triggers.mjs. Not used to match.
    #[serde(default)]
    pub id: Option<String>,
    /// The boss of the next kill step rises on this line, though he may not speak: the
    /// line starts his fight (`boss_after_line`). Only lines marked so; any other line
    /// before a boss (the Wounded Man's "Find the Miller...", live v0.1.115) does not.
    #[serde(default)]
    pub emerges: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(try_from = "RawTrigger")]
pub enum Trigger {
    /// Any of `lines`, said while the player is in `area`. With `after`, the line
    /// only counts once that step is done (before this line, not by it).
    NpcLine { area: String, lines: Vec<NpcLine>, after: Option<String> },
    /// Entering `area`. With `after`, the entry only counts once that step is done
    /// (done before this entry, not by it): for moves back to a zone entered before.
    ZoneEntered { area: String, after: Option<String> },
}

/// A trigger as written in `data/routes/`. An `npcLine` names its line either as
/// `speaker` + `text` (one line) or as `lines` (several: random or class variants,
/// several speakers), not both.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum RawTrigger {
    NpcLine {
        area: String,
        #[serde(default)]
        speaker: Option<String>,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        lines: Vec<NpcLine>,
        #[serde(default)]
        after: Option<String>,
    },
    ZoneEntered {
        area: String,
        #[serde(default)]
        after: Option<String>,
    },
}

impl TryFrom<RawTrigger> for Trigger {
    type Error = String;

    fn try_from(raw: RawTrigger) -> Result<Self, String> {
        match raw {
            RawTrigger::ZoneEntered { area, after } => Ok(Trigger::ZoneEntered { area, after }),
            RawTrigger::NpcLine { area, speaker, text, lines, after } => {
                let lines = match (speaker, text, lines.is_empty()) {
                    (Some(speaker), Some(text), true) => vec![NpcLine { speaker, text, id: None, emerges: false }],
                    (None, None, false) => lines,
                    _ => return Err("npcLine needs either speaker and text, or lines".into()),
                };
                let lines: Vec<NpcLine> = lines
                    .into_iter()
                    .map(|l| NpcLine { speaker: shown_text(&l.speaker), text: shown_text(&l.text), id: l.id, emerges: l.emerges })
                    .collect();
                // An empty speaker or text would match nothing real, or an empty log line.
                if lines.iter().any(|l| l.speaker.is_empty() || l.text.is_empty()) {
                    return Err("npcLine: a line has an empty speaker or text".into());
                }
                Ok(Trigger::NpcLine { area, lines, after })
            }
        }
    }
}

/// A line's text as the log shows it, so table text and log text compare equal:
/// `<i>{word}` (and `<b>`, `<italic>`) becomes `word`, other tags (`<continue>`,
/// `<br/>`) and every run of whitespace (line breaks too) become one space.
/// The table texts carry this markup; no `[Tag|shown]` links occur in them (game
/// data, 2026-11-12). Whether the log keeps markup is not known: both sides are
/// normalised, so it matches either way.
pub fn shown_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        rest = &rest[open..];
        let Some(close) = rest.find('>') else { break };
        rest = &rest[close + 1..];
        match rest.strip_prefix('{').and_then(|r| r.split_once('}')) {
            Some((inner, after)) => {
                out.push_str(inner);
                rest = after;
            }
            None => out.push(' '),
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone, Deserialize)]
pub struct Step {
    pub id: String,
    pub kind: String,
    pub parts: Vec<Part>,
    /// Not needed to progress; shown dimmer.
    #[serde(default)]
    pub optional: bool,
    /// A detour that returns (a waypoint grab, a side zone the route comes back from): its
    /// zone entry ends no visit (`Route::visits`) and is never implied.
    #[serde(default)]
    pub returns: bool,
    /// Listed only until this step is done: a row for one order of play (the Grim Tangle's
    /// waypoint grab before the Red Vale). Once it is done the row is not listed at all.
    #[serde(default)]
    pub until: Option<String>,
    /// A boss that never speaks (the game's tables have no line for it: Areagne, Ervig,
    /// the Crowbell, Candlemass): the app cannot see its fight, so its row is not `tracked`.
    #[serde(default)]
    pub silent: bool,
    /// Other zones a zone-entry trigger also counts from, besides the list's own: the Red
    /// Vale's "Portal to town" when the player walks back and takes the Grelwood's waypoint.
    #[serde(default, rename = "alsoFrom")]
    pub also_from: Vec<String>,
    #[serde(default)]
    pub trigger: Option<Trigger>,
    #[serde(default, rename = "navigationTarget")]
    pub navigation_target: Option<NavigationTarget>,
    /// Quest-tracker states that prove the step done (`quest_screen::Condition`): kills
    /// and talks the log does not show.
    #[serde(default)]
    pub screen: Vec<crate::quest_screen::Condition>,
}

/// Where the compass heads for this step: a zone exit (`exit`: the area it leads
/// to) or a tagged point of the layout graph (`tag`, e.g. "boss", "campsite").
#[derive(Debug, Clone, Deserialize)]
pub struct NavigationTarget {
    pub area: String,
    pub label: String,
    #[serde(default)]
    pub exit: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    /// A map icon the step leads to instead ("waypoint": the zone's waypoint).
    #[serde(default)]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Route {
    /// Position in the campaign: checklists are kept in this order.
    #[serde(default)]
    pub order: u32,
    pub zone: Zone,
    pub steps: Vec<Step>,
}

/// Every checklist in `data/routes/`, one per zone, in campaign order: by the
/// route's `order`, so the first is where a new character starts.
pub fn load_routes() -> Vec<Route> {
    let mut routes: Vec<Route> = crate::bundle::ROUTES
        .iter()
        .map(|(name, json)| {
            let mut route: Route = serde_json::from_str(json).unwrap_or_else(|e| panic!("data/routes/{name}.json: {e}"));
            for step in &mut route.steps {
                for condition in &mut step.screen {
                    condition
                        .resolve(crate::quest_screen::quests())
                        .unwrap_or_else(|e| panic!("data/routes/{name}.json, {}: {e}", step.id));
                }
            }
            route
        })
        .collect();
    routes.sort_by_key(|route: &Route| route.order);
    routes
}

// ---------- Log parsing ----------

#[derive(Debug, Clone, PartialEq)]
pub enum LogEvent {
    ZoneEntered { area: String, level: u32, seed: Option<u64> },
    Dialogue { speaker: String, text: String },
}

/// Parses one Client.txt line, e.g.
/// `2026/11/11 22:41:55 547102231 3ef23348 [INFO Client 308] Wounded Man: Reach...`.
/// Returns the line's timestamp and the event, or None for anything else.
pub fn parse_line(line: &str) -> Option<(String, LogEvent)> {
    let timestamp = line.get(..19)?.to_string();
    let open = line.find(" [")?;
    let close = open + line[open..].find("] ")?;
    let tag = &line[open + 2..close];
    let message = line[close + 2..].trim_end();

    if let Some(rest) = message.strip_prefix("Generating level ") {
        let (level, rest) = rest.split_once(" area \"")?;
        let (area, rest) = rest.split_once('"')?;
        let level = level.trim().parse().ok()?;
        let seed = rest.trim().strip_prefix("with seed ").and_then(|s| s.trim().parse().ok());
        return Some((timestamp, LogEvent::ZoneEntered { area: area.to_string(), level, seed }));
    }

    // Dialogue is logged like chat, but without a channel prefix. Player chat
    // starts with #, &, @, %, $ (and system lines with [), so it is dropped here.
    // What keeps a player from ticking a step is that prefix and the exact text a
    // trigger needs, not the speaker: single-word speakers (Renly, Una, Alva) can
    // be player names too.
    if !tag.starts_with("INFO ") {
        return None;
    }
    let first = message.chars().next()?;
    if "#&@%$[".contains(first) {
        return None;
    }
    let (speaker, text) = message.split_once(": ")?;
    if speaker.is_empty() || speaker.contains(':') || is_system_line(speaker, text) {
        return None;
    }
    Some((
        timestamp,
        LogEvent::Dialogue { speaker: speaker.to_string(), text: text.trim().to_string() },
    ))
}

/// The game's own `INFO` lines of the form "X: Y" that are no one speaking. Seen in the
/// owner's Client.txt (2026-11-12, 1.5 MB, every "X: Y" line read): passive allocation
/// ("Successfully allocated passive skill id: projectiles15, name: …"), the graphics
/// adapter lines, tile and doodad hashes and a disconnect. "Successfully unallocated" is
/// assumed to be logged the same way. An NPC's name can have spaces ("The Bloated
/// Miller", "Beira of the Rotten Pack"), so it is these known starts, not a shape.
const SYSTEM_SPEAKERS: &[&str] = &["Successfully ", "Enumerated ", "Tile hash", "Doodad hash", "Abnormal disconnect"];

fn is_system_line(speaker: &str, text: &str) -> bool {
    // A number alone is a value being logged ("Tile hash: 1435737370"), not a line said.
    let number = !text.trim().is_empty() && text.trim().chars().all(|c| c.is_ascii_digit());
    number || SYSTEM_SPEAKERS.iter().any(|start| speaker.starts_with(start))
}

// ---------- Progress ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Completion {
    pub at: String,
    /// "log", "screen" (the quest tracker), "implied" (a later step was logged or read),
    /// "map" (a boss's grey icon) or "manual".
    pub source: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    #[serde(default)]
    pub current_area: Option<String>,
    /// The level of `current_area`, from its zone-generation line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area_level: Option<u32>,
    /// The character's level, from its last level-up line (`level_up_in_line`). Progress is
    /// kept per character, so this is that character's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    /// Zone of the checklist on show: the last zone entered that has one.
    #[serde(default)]
    pub shown_zone: Option<String>,
    #[serde(default)]
    pub completed: BTreeMap<String, Completion>,
    /// Steps the player unticked by hand; the log won't tick them again.
    #[serde(default)]
    pub dismissed: BTreeSet<String>,
    #[serde(default)]
    pub log_path: Option<String>,
    #[serde(default)]
    pub log_offset: Option<u64>,
}

/// Applies one event to progress. Returns true if anything visible changed.
///
/// A zone entry ticks only rows of the list of the zone the player came from (R-F1 of
/// 2026-11-13): "Portal to town" in Freythorn's list waits for an entry into town from
/// Freythorn, not from Ogham Village. Where the player came from is not known (the first
/// entry seen, or an area without a checklist): any list, as before.
pub fn apply(routes: &[Route], progress: &mut Progress, timestamp: &str, event: &LogEvent) -> bool {
    let mut changed = false;
    let origin = match event {
        LogEvent::ZoneEntered { .. } => progress.current_area.clone().filter(|area| routes.iter().any(|r| &r.zone.id == area)),
        LogEvent::Dialogue { .. } => None,
    };
    if let LogEvent::ZoneEntered { area, level, .. } = event {
        if progress.current_area.as_deref() != Some(area) || progress.area_level != Some(*level) {
            progress.current_area = Some(area.clone());
            progress.area_level = Some(*level);
            changed = true;
        }
        if routes.iter().any(|r| &r.zone.id == area) && progress.shown_zone.as_deref() != Some(area) {
            progress.shown_zone = Some(area.clone());
            changed = true;
        }
    }
    // What was done before this event: an `after` step ticked by this very event doesn't count.
    let done_before: BTreeSet<String> = progress.completed.keys().cloned().collect();
    // A line in the form the triggers keep (`shown_text`), worked out once per event.
    let heard = match event {
        LogEvent::Dialogue { speaker, text } => Some((shown_text(speaker), shown_text(text))),
        LogEvent::ZoneEntered { .. } => None,
    };
    // One event ticks at most one row of a list (R-F4 of 2026-11-13): when several match,
    // the last whose `after` holds. Skip the Trial's early waypoint grab, and the late entry
    // ticks "Waypoint to the Trial" alone.
    let mut ticks: BTreeMap<&str, (&Step, String, &str)> = BTreeMap::new();
    for (route, step) in routes.iter().flat_map(|r| r.steps.iter().map(move |s| (r, s))) {
        let Some(trigger) = &step.trigger else { continue };
        // A kill only "likely" is still open to its kill line.
        if !unconfirmed(progress, &step.id) || progress.dismissed.contains(&step.id) {
            continue;
        }
        let (evidence, source) = match (trigger, event) {
            (Trigger::ZoneEntered { area, after }, LogEvent::ZoneEntered { area: got, level, .. })
                if area == got && after.as_ref().is_none_or(|step| done_before.contains(step)) =>
            {
                match &origin {
                    Some(from) if *from == route.zone.id || step.also_from.contains(from) => {
                        (format!("Entered {got} (level {level})"), "log")
                    }
                    // A town's "Waypoint to X" says where to go next: X entered another way
                    // (on foot through the Grelwood) does that. Other lists' rows say how to
                    // leave their zone: from there only.
                    Some(from) if is_town(&route.zone.id) => (format!("Entered {got} from {from}"), "log"),
                    Some(_) => continue,
                    // Where from is not known: any list, but only as inferred (`Route::shown`
                    // does not take it for the player seen leaving).
                    None => (format!("Entered {got} (level {level}); where from is not known"), "implied"),
                }
            }
            (Trigger::NpcLine { area, lines, after }, LogEvent::Dialogue { speaker: who, text: said })
                if progress.current_area.as_deref() == Some(area.as_str())
                    && after.as_ref().is_none_or(|step| done_before.contains(step))
                    && heard.as_ref().is_some_and(|(s, t)| lines.iter().any(|l| &l.speaker == s && &l.text == t)) =>
            {
                (format!("{who}: {said}"), "log")
            }
            _ => continue,
        };
        ticks.insert(route.zone.id.as_str(), (step, evidence, source));
    }
    // Coming into Z from X proves Z's way to X was taken (R-F6): a start in the middle of an
    // act, or X reached another way (a waypoint from town). Z's open row into X, the last whose
    // `after` holds, is taken as done.
    // Not for a town on either side (council round 2): a town is reached from anywhere, so
    // arriving from X proves nothing about its row to X (Kingsmarch from Journey's End took
    // the later "Waypoint to Journey's End" as done), nor does arriving from town.
    if let (Some(from), LogEvent::ZoneEntered { area: got, .. }) = (&origin, event) {
        if let Some(route) = routes.iter().find(|r| &r.zone.id == got).filter(|_| !is_town(got) && !is_town(from)) {
            let back = route.steps.iter().rev().find(|step| {
                matches!(&step.trigger, Some(Trigger::ZoneEntered { area, after }) if area == from
                    && after.as_ref().is_none_or(|id| done_before.contains(id)))
                    && !progress.completed.contains_key(&step.id)
                    && !progress.dismissed.contains(&step.id)
            });
            if let Some(step) = back {
                ticks.entry(route.zone.id.as_str()).or_insert((step, format!("Came back from {from}"), "implied"));
            }
        }
    }
    for (step, evidence, source) in ticks.into_values() {
        progress.completed.insert(
            step.id.clone(),
            Completion { at: timestamp.to_string(), source: source.into(), evidence },
        );
        changed = true;
    }
    imply_earlier(routes, progress, timestamp, &done_before);
    changed
}

/// Judges the log events read while no character was bound against the progress of the one
/// now bound, each in the area it happened in. Returns true if anything visible changed.
pub fn replay_unbound(inner: &mut Inner) -> bool {
    if inner.characters.active.is_none() {
        return false;
    }
    let events = std::mem::take(&mut inner.characters.unbound);
    let Inner { routes, progress, characters, .. } = inner;
    let (area, shown) = (progress.current_area.clone(), progress.shown_zone.clone());
    let mut changed = false;
    for (timestamp, event, was_in) in &events {
        progress.current_area = was_in.clone();
        changed |= apply(routes, progress, timestamp, event);
    }
    if !events.is_empty() {
        progress.current_area = area;
        progress.shown_zone = shown;
        characters.remember(progress);
    }
    changed
}

/// The log shows zone entries and a few spoken lines; a talk in a dialogue panel, a
/// pick-up or a waypoint leaves no line (measured 2026-11-12: talks with Renly and Una
/// in the Clearfell Encampment wrote nothing). The lists are in walking order, so when
/// the log ticks a step, the steps before it in its list that the log cannot see are
/// taken as done too (source "implied"): only what the later step proves, talks and the like.
/// Not optional steps, boss kills, detours that return, steps no longer listed, and not what
/// a player may skip and still need (R-F5 of 2026-11-13): steps the compass leads to ("Take
/// the waypoint"), steps with a reward out in a zone (the campsite stash, the small ritual;
/// a town's hand-in of the visit being left is implied: Renly's gem when Clearfell is
/// entered) and zone entries (the log would have shown them: "Enter Freythorn" when the
/// Farmlands came first).
/// Those stay open until they are logged or ticked by hand; the panel keeps rows the app
/// cannot tick (`tracked`) from holding the arrow.
fn imply_earlier(routes: &[Route], progress: &mut Progress, timestamp: &str, done_before: &BTreeSet<String>) {
    for route in routes {
        let newly = |step: &Step| {
            !done_before.contains(&step.id)
                && progress.completed.get(&step.id).is_some_and(|c| c.source == "log" || c.source == "screen")
        };
        // A trip to town and back proves nothing about the visit's work (council round 2): a
        // portal from the Grelwood before the seals does not strike them. Such rows wait for
        // another step (`after`); the walk on into a town (the Riverbank's exit) does not.
        let to_town = |step: &Step| matches!(&step.trigger, Some(Trigger::ZoneEntered { area, after: Some(_) }) if is_town(area));
        let Some(last) = route.steps.iter().rposition(|step| newly(step) && !to_town(step)) else { continue };
        let visits = route.visits();
        for (at, step) in route.steps[..last].iter().enumerate() {
            // A town's hand-in (Renly's gem, the lute's points) is taken with the visit it
            // belongs to: the player stood by the NPC. A reward out in a zone is not.
            let hand_in = is_town(&route.zone.id) && visits[at] == visits[last];
            let unseen = step.kind == "objective"
                && !step.optional
                && !step.returns
                && listed(step, progress)
                && !is_boss(step)
                && step.navigation_target.is_none()
                && (hand_in || !has_reward(step))
                && !matches!(step.trigger, Some(Trigger::ZoneEntered { .. }));
            if !unseen || progress.completed.contains_key(&step.id) || progress.dismissed.contains(&step.id) {
                continue;
            }
            progress.completed.insert(
                step.id.clone(),
                Completion { at: timestamp.to_string(), source: "implied".into(), evidence: format!("Before {}", route.steps[last].id) },
            );
        }
    }
}

/// Ticks the steps a confirmed quest-tracker state proves done (`quest_screen`), and the
/// unseen steps before them as the log does. True when a step was ticked. A step unticked
/// by hand stays unticked; a "likely" kill becomes proven ("screen"), a confirmed one stays.
// The quest tracker is read on Windows only (`quest_screen.rs`).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn apply_screen(routes: &[Route], progress: &mut Progress, quest: &str, orders: &BTreeSet<u32>, evidence: &str) -> bool {
    let done_before: BTreeSet<String> = progress.completed.keys().cloned().collect();
    let at = now_label();
    let mut ticked = false;
    for step in routes.iter().flat_map(|r| &r.steps) {
        // A "probably done" kill is still open to proof (R-F7); a confirmed one stays.
        if !unconfirmed(progress, &step.id)
            || progress.dismissed.contains(&step.id)
            || !step.screen.iter().any(|c| c.proves(quest, orders))
        {
            continue;
        }
        progress.completed.insert(
            step.id.clone(),
            Completion { at: at.clone(), source: "screen".into(), evidence: evidence.to_string() },
        );
        ticked = true;
    }
    if ticked {
        imply_earlier(routes, progress, &at, &done_before);
    }
    ticked
}

/// Ticks or unticks a step by hand; a click on a "likely" kill confirms it (it stays done).
pub fn toggle(routes: &[Route], progress: &mut Progress, id: &str) -> Result<(), String> {
    let step = routes.iter().flat_map(|r| &r.steps).find(|s| s.id == id).ok_or("Unknown step")?;
    if step.kind != "objective" {
        return Err("Only objectives can be ticked".into());
    }
    if progress.completed.get(id).is_some_and(|done| done.source == LIKELY) {
        // A click on a "probably done" kill confirms it.
        progress.completed.insert(id.to_string(), Completion { at: now_label(), source: "manual".into(), evidence: "Confirmed by hand".into() });
    } else if let Some(done) = progress.completed.remove(id) {
        // Unticking what the app ticked tells it not to tick that step again. Taking
        // back one's own click does not: the step is simply open again.
        if done.source != "manual" {
            progress.dismissed.insert(id.to_string());
        }
    } else {
        progress.dismissed.remove(id);
        progress.completed.insert(
            id.to_string(),
            Completion { at: now_label(), source: "manual".into(), evidence: "Ticked by hand".into() },
        );
    }
    Ok(())
}

fn now_label() -> String {
    let secs = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("unix:{secs}")
}

// ---------- View model ----------

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepView {
    id: String,
    kind: String,
    parts: Vec<Part>,
    optional: bool,
    /// The compass target this step stands for (its navigation target's label), if any.
    target: Option<String>,
    /// "done", "open" (watched in the log) or "manual" (no log trigger known).
    state: String,
    /// "log", "screen", "implied", "map", "manual", or "likely" (a boss quiet for 30 s).
    source: Option<String>,
    /// The app can tick this row by itself on this platform (`tracked`).
    tracked: bool,
    /// Which visit to the zone the step belongs to (see `Route::visits`).
    visit: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogStatus {
    pub connected: bool,
    pub path: Option<String>,
    pub message: String,
}

/// Whether a step is listed at all: a step with `until` is not, once that step is done.
pub fn listed(step: &Step, progress: &Progress) -> bool {
    step.until.as_ref().is_none_or(|id| !progress.completed.contains_key(id))
}

/// Whether a zone is an act's town (`G1_town`…): the hub its list's waypoint rows leave from.
fn is_town(zone: &str) -> bool {
    zone.ends_with("_town")
}

/// Whether a step shows a reward: the reward is the reason for the row (a pickup, a boss
/// with a permanent bonus, a hand-in).
fn has_reward(step: &Step) -> bool {
    step.parts.iter().any(|p| p.style.as_deref() == Some("reward"))
}

/// Whether a step is a boss kill (it has a boss part).
fn is_boss(step: &Step) -> bool {
    step.parts.iter().any(|p| p.style.as_deref() == Some("boss"))
}

/// Whether the app can tick a step by itself on this platform: a log trigger, a boss kill
/// (its fight starts when the boss speaks and ends with the kill line or as "likely" after
/// `FIGHT_QUIET`; not a boss marked `silent`: it never speaks, so no fight is seen), and on
/// Windows a quest-tracker rule. Otherwise a click or an implied tick only.
pub fn tracked(step: &Step) -> bool {
    step.kind == "objective"
        && (step.trigger.is_some() || (is_boss(step) && !step.silent) || (crate::quest_screen::READS && !step.screen.is_empty()))
}

/// Whether a step's trigger is a zone entry into another area: it leaves the zone.
fn leaves(step: &Step, zone: &str) -> bool {
    matches!(&step.trigger, Some(Trigger::ZoneEntered { area, .. }) if area != zone)
}

/// The visit on show (`Route::shown`).
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    /// The visit whose steps are listed. One past the last visit when no planned visit is
    /// due (a town trip the route does not plan): nothing is listed then.
    pub visit: usize,
    /// Open required steps of earlier visits the player left unfinished, listed with the
    /// visit on show (rule 5 of `Route::shown`): Finn's charm when the town trip after
    /// Freythorn was skipped, Crowbell when the Farmlands came before Freythorn.
    pub carried: Vec<usize>,
}

impl Route {
    /// Each step's visit to the zone, from 0. A zone's list covers every time the route
    /// passes through it (the town's: six times). A visit ends with the step that leaves
    /// the zone (a zone-entry trigger for another area); the steps after it belong to the
    /// next time the player is there. An optional step that leaves (a side trip such as
    /// the Mud Burrow) and a detour that returns (`returns`: a waypoint grab, the Cemetery's
    /// Mausoleum and Tomb) end nothing: what follows is still this visit.
    pub fn visits(&self) -> Vec<usize> {
        let mut visit = 0;
        self.steps
            .iter()
            .map(|step| {
                let this = visit;
                if leaves(step, &self.zone.id) && !step.optional && !step.returns {
                    visit += 1;
                }
                this
            })
            .collect()
    }

    /// The visit on show, from what is done, never from the order it was done in (R1 of
    /// 2026-11-13; `docs/architecture.md`, "Visits"):
    /// 1. A visit is finished when its leaving step is done or no longer listed. The
    ///    high-water mark is the visit after the last finished one: skipped detours and
    ///    earlier open visits do not hold the list back.
    /// 2. A later visit is due when the player has arrived for it: its leaving step's
    ///    `after` is done (the town's visits wait for the zone the route comes from), or,
    ///    with no `after`, it is the visit at the mark. The last due visit from the mark
    ///    on is shown.
    /// 3. Nothing due (a trip the route does not plan): the visit before the mark while it
    ///    has open required steps (back in the Grelwood for Areagne), else nothing. When the
    ///    player was not seen leaving that visit (its leaving step only inferred, R-F6), the
    ///    visit at the mark: first seen in the Red Vale, back in the Grelwood, the seals.
    /// 4. Every visit finished: the last one.
    /// 5. The open required steps of an earlier visit the player never finished (its leaving
    ///    step not done) come with the visit on show, its leaving step too unless that waits
    ///    for something not yet done (`after`) (R-F2): the Farmlands entered before
    ///    Freythorn, back in the Hunting Grounds Crowbell, the ritual and "Enter Freythorn"
    ///    are still listed; Finn's charm comes with the town visit after Ogham Village.
    /// 6. A boss with a reward left alive in a visit that was finished (the player went on
    ///    without it) comes with the visit on show too, until it is killed or ticked by
    ///    hand: back in the Hunting Grounds from town, Crowbell's points; with the seals,
    ///    Areagne's gem. The app knows whether a boss is dead; it does not know that of a
    ///    pickup, so other open steps of a finished visit do not come along (a pickup done
    ///    but never clicked would show again on every return).
    pub fn shown(&self, progress: &Progress) -> Shown {
        let visits = self.visits();
        let last = visits.last().copied().unwrap_or(0);
        let done = |i: usize| progress.completed.contains_key(&self.steps[i].id);
        let is_leaving = |i: usize| leaves(&self.steps[i], &self.zone.id) && !self.steps[i].optional && !self.steps[i].returns;
        // A visit's leaving step is its last step, when that step leaves.
        let leaving = |v: usize| visits.iter().rposition(|&x| x == v).filter(|&i| is_leaving(i));
        let finished = |v: usize| leaving(v).is_some_and(|i| done(i) || !listed(&self.steps[i], progress));
        let mark = (0..=last).rev().find(|&v| finished(v)).map_or(0, |v| v + 1);
        let waits = |i: usize| {
            matches!(&self.steps[i].trigger, Some(Trigger::ZoneEntered { after: Some(id), .. }) if !progress.completed.contains_key(id))
        };
        let left_open = |i: usize| {
            let step = &self.steps[i];
            step.kind == "objective" && !step.optional && !done(i) && listed(step, progress) && !waits(i)
        };
        // Rules 5 and 6: the steps of visits before `upto` to list with the visit on show:
        // everything open of a visit never finished, and the reward bosses left alive in one that was.
        // Nothing comes along on a list nothing was done in yet (a character first seen in
        // the middle of an act, or after a reset): its earlier visits were never seen.
        let touched = (0..self.steps.len()).any(done);
        let carry = |upto: usize| -> Vec<usize> {
            (0..self.steps.len())
                .filter(|&i| touched && visits[i] < upto && left_open(i))
                .filter(|&i| !finished(visits[i]) || (has_reward(&self.steps[i]) && is_boss(&self.steps[i])))
                .collect()
        };
        if mark > last {
            return Shown { visit: last, carried: carry(last) };
        }
        let after = |v: usize| match leaving(v).and_then(|i| self.steps[i].trigger.as_ref()) {
            Some(Trigger::ZoneEntered { after, .. }) => after.clone(),
            _ => None,
        };
        let due = |v: usize| match after(v) {
            Some(id) => progress.completed.contains_key(&id),
            None => v == mark,
        };
        let open = |i: usize| {
            let step = &self.steps[i];
            step.kind == "objective" && !done(i) && listed(step, progress) && !leaves(step, &self.zone.id)
        };
        if let Some(visit) = (mark..=last).rev().find(|&v| v == 0 || due(v)) {
            return Shown { visit, carried: carry(visit) };
        }
        let back = mark - 1;
        // The player was not seen leaving the visit before (its leaving step only inferred:
        // a start in the middle of an act, or back from where it leads): the next visit.
        let seen = leaving(back).and_then(|i| progress.completed.get(&self.steps[i].id)).is_none_or(|c| c.source != "implied");
        if !seen {
            return Shown { visit: mark, carried: carry(mark) };
        }
        let unfinished = (0..self.steps.len()).any(|i| visits[i] == back && open(i) && !self.steps[i].optional);
        Shown { visit: if unfinished { back } else { last + 1 }, carried: carry(mark) }
    }

    /// The visit on show (`shown`), without the carried steps.
    pub fn current_visit(&self, progress: &Progress) -> usize {
        self.shown(progress).visit
    }

    /// The steps listed now, each with the visit the page files it under: every listed step
    /// with its own visit, the carried ones with the visit on show.
    pub fn listed_steps<'a>(&'a self, progress: &Progress) -> Vec<(&'a Step, usize)> {
        let shown = self.shown(progress);
        self.steps
            .iter()
            .zip(self.visits())
            .enumerate()
            .filter(|(_, (step, _))| listed(step, progress))
            .map(|(i, (step, visit))| (step, if shown.carried.contains(&i) { shown.visit } else { visit }))
            .collect()
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackerView {
    /// The zone's level is the log's zone line's when known, else the route file's.
    zone: Option<Zone>,
    /// The character's level, from its last level-up line; None until one was seen.
    level: Option<u32>,
    current_area: Option<String>,
    in_zone: bool,
    character: Option<String>,
    recognized_character: Option<String>,
    characters: Vec<String>,
    legacy_available: bool,
    identity_note: String,
    steps: Vec<StepView>,
    /// The visit under way: the page lists its steps only (`StepView::visit`).
    visit: usize,
    log: LogStatus,
}

pub struct Inner {
    pub routes: Vec<Route>,
    pub progress: Progress,
    pub characters: crate::characters::Characters,
    pub identity: crate::identity::Recognition,
    pub log: LogStatus,
    pub save_path: Option<PathBuf>,
    /// The compass passes over optional steps (a setting of the page).
    pub skip_optional: bool,
    /// The last dialogue lines seen live: (timestamp, speaker, text), oldest first.
    pub recent: std::collections::VecDeque<(String, String, String)>,
    /// A boss fight under way (see `Fight`).
    pub fight: Option<Fight>,
    /// The player is in a zone: false from a game start (login, character selection)
    /// until the first zone is entered. The compass shows only in a zone.
    pub in_zone: bool,
    /// When `progress.json` was last written (`save`).
    pub saved_at: std::cell::Cell<Option<std::time::Instant>>,
}

/// With nothing else new, the log offset is written at most this often: the log grows
/// every few seconds with chat and system lines.
const SAVE_OFFSET_EVERY: Duration = Duration::from_secs(30);

/// Whether a log batch writes `progress.json`: when the progress changed, else when the
/// file's offset is `SAVE_OFFSET_EVERY` old (or was never written).
fn save_due(changed: bool, saved_at: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    changed || saved_at.is_none_or(|at| now.duration_since(at) >= SAVE_OFFSET_EVERY)
}

/// The log line of a game start.
const GAME_START: &str = "[STARTUP] Game Start";

/// Login/reconnect boundaries observed in Client.txt. They do not identify a character.
fn session_boundary(line: &str) -> bool {
    let Some((_, message)) = line.split_once("] ") else { return false };
    message == GAME_START
        || (message.starts_with("Async connecting to ") && message.contains(".login.pathofexile2.com"))
        || message.starts_with("Abnormal disconnect")
}

/// Whether the log's last game start already led into a zone (true too when the log
/// shows no game start at all).
fn zone_since_start(text: &str) -> bool {
    let last = text.lines().rev().find(|line| session_boundary(line));
    let after = last.and_then(|line| text.rfind(line)).map_or(text, |at| &text[at..]);
    after.contains("Generating level ") || last.is_none()
}

/// A boss fight under way: the overlay stays out of the way until it is over. It starts
/// when a boss of the zone whose kill step is open speaks (live lines only), and ends
/// when that step is ticked, the player dies or changes zone, or the boss has said
/// nothing for `FIGHT_QUIET` (bosses bark every few seconds while they fight; one that
/// never speaks starts no fight).
#[derive(Debug, Clone, PartialEq)]
pub struct Fight {
    /// The boss's kill step.
    pub step: String,
    /// When the boss last spoke.
    pub last_line: std::time::Instant,
    /// Other bosses of the zone heard before this one, each with its last line: their kills
    /// are settled on their own timers (`Inner::settle_fight`). Cleared with the fight.
    pub earlier: Vec<(String, std::time::Instant)>,
}

/// A boss silent for this long is taken to be dead or left behind. The Bloated Miller's
/// lines came at most 19 s apart in two logged fights.
pub const FIGHT_QUIET: Duration = Duration::from_secs(30);

/// The open kill step in `area` of the boss who is speaking. As in the layout recorder,
/// the speaker must contain the checklist's boss name (at least four letters), so
/// "Beira" matches "Beira of the Rotten Pack". Where the kill row has a line trigger whose
/// speakers carry the boss's name, only those speakers count (R-F3 of 2026-11-13): "Lachlann
/// the Lost", the man at the gate, is not "Lachlann of Endless Lament", the boss. A trigger
/// spoken by someone else (Mortimer for the Miller) leaves the name rule as it was.
pub fn boss_step<'a>(routes: &'a [Route], progress: &Progress, area: &str, speaker: &str) -> Option<&'a Step> {
    let speaker = speaker.to_lowercase();
    routes.iter().filter(|r| r.zone.id == area).flat_map(|r| &r.steps).find(|step| {
        let Some(name) = boss_name(step) else { return false };
        let own = own_speakers(step);
        unconfirmed(progress, &step.id)
            && name.len() >= 4
            && speaker.contains(&name)
            && (own.is_empty() || own.contains(&speaker))
    })
}

/// A kill row's boss name, lower case.
fn boss_name(step: &Step) -> Option<String> {
    step.parts.iter().find(|p| p.style.as_deref() == Some("boss")).map(|p| p.text.trim().to_lowercase())
}

/// The speakers of a kill row's own line trigger that carry the boss's name, lower case:
/// the boss's own forms. Empty when the row has no line trigger or others say it.
fn own_speakers(step: &Step) -> Vec<String> {
    match (&step.trigger, boss_name(step)) {
        (Some(Trigger::NpcLine { lines, .. }), Some(name)) => {
            lines.iter().map(|l| l.speaker.to_lowercase()).filter(|s| s.contains(&name)).collect()
        }
        _ => Vec::new(),
    }
}

/// A kill step not yet known done: open, or only "likely" (a boss quiet for `FIGHT_QUIET`).
fn unconfirmed(progress: &Progress, id: &str) -> bool {
    progress.completed.get(id).is_none_or(|done| done.source == LIKELY)
}

/// The source of a kill taken as probably done: the boss was quiet for `FIGHT_QUIET`.
pub const LIKELY: &str = "likely";

/// A live line in the current zone: a boss of the zone whose kill is not confirmed speaks
/// (or emerges): its fight starts, or resumes. A row ticked as "likely" opens again: the
/// boss is not dead. True when a row opened again.
pub fn hear_boss(routes: &[Route], progress: &mut Progress, fight: &mut Option<Fight>, speaker: &str, text: &str) -> bool {
    let area = progress.current_area.clone().unwrap_or_default();
    let Some(step) = boss_step(routes, progress, &area, speaker).or_else(|| boss_after_line(routes, progress, &area, text)) else {
        return false;
    };
    // Another boss heard before keeps its own timer (R-F8): Areagne, then Gerung.
    let mut earlier = Vec::new();
    if let Some(current) = fight.take() {
        earlier = current.earlier;
        if current.step != step.id {
            earlier.push((current.step, current.last_line));
        }
    }
    earlier.retain(|(id, _)| *id != step.id);
    *fight = Some(Fight { step: step.id.clone(), last_line: std::time::Instant::now(), earlier });
    let reopened = progress.completed.get(&step.id).is_some_and(|done| done.source == LIKELY);
    if reopened {
        progress.completed.remove(&step.id);
    }
    reopened
}

/// The open kill step that follows the step this line triggers, in `area`, when the line
/// is marked `emerges`: the line a boss emerges on starts his fight though he does not say it (the Rust King rises at the third
/// obelisk with a "Ghostly Voice" line, then may say nothing: the owner's fight of 2026-11-13).
pub fn boss_after_line<'a>(routes: &'a [Route], progress: &Progress, area: &str, text: &str) -> Option<&'a Step> {
    let route = routes.iter().find(|r| r.zone.id == area)?;
    let said = |step: &Step| matches!(&step.trigger, Some(Trigger::NpcLine { lines, .. }) if lines.iter().any(|l| l.emerges && l.text == text));
    let at = route.steps.iter().position(said)?;
    route.steps[at + 1..]
        .iter()
        .filter(|step| step.kind == "objective")
        .find(|step| step.parts.iter().any(|p| p.style.as_deref() == Some("boss")))
        // A boss with lines of its own starts its fight by them only (R-F3): Lachlann's
        // fight does not start at the Memorial Gate's line.
        .filter(|step| unconfirmed(progress, &step.id) && own_speakers(step).is_empty())
}

/// The character a log line names: a level-up (`: <name> (<Class>) is now level <n>`) or a
/// death (`: <name> has been slain.`). The game writes the character's name nowhere at login;
/// these lines are where it appears (the owner's Client.txt, 2026-11-13). Chat lines carry
/// their sender after `] ` with a channel mark (`#`, `@`, `$`, `%`, `&`), never `] : `.
pub fn character_in_line(line: &str) -> Option<String> {
    let (_, message) = line.split_once("] : ")?;
    let name = match message.trim_end().strip_suffix(" has been slain.") {
        Some(name) => name.to_string(),
        None => return level_up_in_line(line).map(|(name, _)| name),
    };
    valid_name(&name).then_some(name)
}

/// The character and its new level in a level-up line (`: <name> (<Class>) is now level <n>`).
pub fn level_up_in_line(line: &str) -> Option<(String, u32)> {
    let (_, message) = line.split_once("] : ")?;
    let (name, rest) = message.trim_end().split_once(" (")?;
    let (_, level) = rest.split_once(") is now level ")?;
    let level = level.trim().parse::<u32>().ok().filter(|l| (1..=100).contains(l))?;
    valid_name(name).then(|| (name.to_string(), level))
}

fn valid_name(name: &str) -> bool {
    (2..=80).contains(&name.chars().count()) && name.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// A line that names the character (`character_in_line`) binds that character's profile;
/// a level-up line also keeps the new level in it, so another character's level-up changes
/// only its own. True when the profile or its level changed.
pub fn note_character(inner: &mut Inner, line: &str) -> bool {
    let Some(name) = character_in_line(line) else { return false };
    let Inner { progress, characters, .. } = inner;
    let mut changed = characters.bind_from_log(progress, &name);
    let bound = characters.active.as_deref().is_some_and(|active| active.to_lowercase() == name.to_lowercase());
    if let (true, Some((_, level))) = (bound, level_up_in_line(line)) {
        changed |= progress.level != Some(level);
        progress.level = Some(level);
    }
    changed
}

/// The log line of the player's death: `: <character> has been slain.`
fn is_death(line: &str) -> bool {
    line.trim_end().ends_with(" has been slain.") && line.contains("] : ")
}

/// Dialogue lines kept for `Inner::note_hand_tick`.
const RECENT_LINES: usize = 30;

impl Inner {
    /// Ends the fight when its step is ticked or the boss has been quiet too long. A boss
    /// quiet that long, with the player still alive in the zone (a death or a zone change
    /// ends the fight first), is taken as probably killed: ticked as "likely" (owner,
    /// decision 4 of 2026-11-13: the arrow moves on, a click confirms). The same boss
    /// speaking again opens it again (`hear_boss`). Unticking it by hand keeps it open.
    /// True when a fight ended.
    pub fn settle_fight(&mut self) -> bool {
        let Some(fight) = self.fight.take() else { return false };
        // Bosses heard earlier in the fight, each on its own timer (R-F8 of 2026-11-13).
        let mut earlier = Vec::new();
        for (step, last_line) in fight.earlier {
            if !unconfirmed(&self.progress, &step) {
                continue;
            }
            if last_line.elapsed() > FIGHT_QUIET {
                self.probably_done(&step);
                continue;
            }
            earlier.push((step, last_line));
        }
        let killed = !unconfirmed(&self.progress, &fight.step);
        if !killed && fight.last_line.elapsed() <= FIGHT_QUIET {
            self.fight = Some(Fight { earlier, ..fight });
            return false;
        }
        if !killed {
            self.probably_done(&fight.step);
        }
        // The kill came while another boss heard before is not yet quiet: its fight goes on.
        if let Some(at) = (0..earlier.len()).max_by_key(|&i| earlier[i].1) {
            let (step, last_line) = earlier.remove(at);
            self.fight = Some(Fight { step, last_line, earlier });
            return false;
        }
        true
    }

    /// A boss quiet for `FIGHT_QUIET`: its kill is ticked as "likely", unless unticked by hand.
    fn probably_done(&mut self, step: &str) {
        if !self.progress.dismissed.contains(step) {
            self.progress.completed.insert(
                step.to_string(),
                Completion { at: now_label(), source: LIKELY.into(), evidence: "The boss fell silent for 30 s: probably killed".into() },
            );
        }
    }

    /// Ticks the kill steps of the current zone's bosses the map shows dead
    /// (`Recorder::dead_bosses`: the boss's own icon seen alive, then grey at the same
    /// place). True when a step was ticked. A step unticked by hand stays unticked.
    pub fn tick_dead_bosses(&mut self, dead: &[String]) -> bool {
        let Inner { routes, progress, .. } = self;
        let Some(area) = progress.current_area.clone() else { return false };
        let mut ticked = false;
        for step in routes.iter().filter(|r| r.zone.id == area).flat_map(|r| &r.steps) {
            let Some(boss) = step.parts.iter().find(|p| p.style.as_deref() == Some("boss")).map(|p| p.text.trim()) else {
                continue;
            };
            if !dead.iter().any(|name| name == boss)
                || !unconfirmed(progress, &step.id)
                || progress.dismissed.contains(&step.id)
            {
                continue;
            }
            progress.completed.insert(
                step.id.clone(),
                Completion { at: now_label(), source: "map".into(), evidence: format!("The map shows {boss}'s icon grey") },
            );
            ticked = true;
        }
        ticked
    }

    /// The character whose quest tracker is worth reading now: in a zone, bound, with a
    /// step open that the tracker can tick.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub fn screen_watch(&self) -> Option<String> {
        let character = self.characters.active.clone()?;
        let open = self.routes.iter().flat_map(|r| &r.steps).any(|step| {
            // A "likely" kill is still open to the tracker's proof (`apply_screen`).
            !step.screen.is_empty()
                && unconfirmed(&self.progress, &step.id)
                && !self.progress.dismissed.contains(&step.id)
        });
        (self.in_zone && open).then_some(character)
    }

    /// See `apply_screen`.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub fn apply_screen(&mut self, quest: &str, orders: &BTreeSet<u32>, evidence: &str) -> bool {
        apply_screen(&self.routes, &mut self.progress, quest, orders, evidence)
    }

    pub fn sync_navigation(&self, app: &AppHandle) {
        let completed = self.routes.iter().flat_map(|r| &r.steps)
            .filter(|s| self.progress.completed.contains_key(&s.id))
            .filter_map(|s| s.navigation_target.as_ref())
            .map(|t| (t.area.clone(), t.label.clone())).collect();
        // Open steps with a target, in checklist order: what the compass may head for.
        // Only the steps of each zone's visit on show: what the list shows.
        let targets = self.routes.iter()
            .flat_map(|r| {
                let now = r.current_visit(&self.progress);
                r.listed_steps(&self.progress).into_iter().filter(move |(_, visit)| *visit == now).map(|(step, _)| step)
            })
            .filter(|s| !self.progress.completed.contains_key(&s.id))
            .filter(|s| !(self.skip_optional && s.optional))
            .filter_map(|s| s.navigation_target.as_ref().map(|t| (t.area.clone(), crate::layout::guide::NavTarget {
                label: t.label.clone(),
                exit: t.exit.clone(),
                // An icon target travels as a tag the layouts do not have ("icon:waypoint").
                tag: t.tag.clone().or_else(|| t.icon.as_ref().map(|icon| format!("icon:{icon}"))),
                optional: s.optional,
            })))
            .collect();
        // Which targets the log completed (the player was there at that moment).
        let by_log = self.routes.iter().flat_map(|r| &r.steps)
            .filter(|s| self.progress.completed.get(&s.id).is_some_and(|c| c.source == "log"))
            .filter_map(|s| s.navigation_target.as_ref())
            .map(|t| (t.area.clone(), t.label.clone())).collect();
        // The zones' bosses (steps with a boss part), with the step's target as their place.
        let bosses = self.routes.iter().flat_map(|r| r.steps.iter().map(move |s| (r, s)))
            .filter_map(|(route, step)| {
                let name = step.parts.iter().find(|p| p.style.as_deref() == Some("boss"))?.text.trim().to_string();
                let target = step.navigation_target.as_ref();
                Some((route.zone.id.clone(), crate::layout::Boss {
                    name,
                    exit: target.and_then(|t| t.exit.clone()),
                    tag: target.and_then(|t| t.tag.clone()),
                }))
            })
            .collect();
        if !crate::RADAR {
            return;
        }
        let recorder = app.state::<crate::layout::Recorder>();
        recorder.set_bosses(bosses);
        recorder.set_targets(targets);
        recorder.set_log_completed(by_log);
        if let Some(view) = recorder.set_completed(completed) {
            crate::minimap::send_layout(app, view);
        }
    }

    pub fn view(&self) -> TrackerView {
        let route = self.routes.iter().find(|r| self.in_zone && Some(r.zone.id.as_str()) == self.progress.current_area.as_deref());
        // Steps not listed at all (`listed`) are left out; carried ones come under the visit on show.
        let steps = route.into_iter().flat_map(|r| r.listed_steps(&self.progress))
            .map(|(step, visit)| {
                let done = self.progress.completed.get(&step.id);
                let state = if step.kind != "objective" {
                    "note"
                } else if done.is_some() {
                    "done"
                } else if step.trigger.is_some() || (crate::quest_screen::READS && !step.screen.is_empty()) {
                    "open"
                } else {
                    "manual"
                };
                StepView {
                    id: step.id.clone(),
                    kind: step.kind.clone(),
                    parts: step.parts.clone(),
                    optional: step.optional,
                    target: step.navigation_target.as_ref().map(|t| t.label.clone()),
                    state: state.into(),
                    source: done.map(|c| c.source.clone()),
                    tracked: tracked(step),
                    visit,
                }
            })
            .collect();
        TrackerView {
            zone: route.map(|r| Zone { level: self.progress.area_level.unwrap_or(r.zone.level), ..r.zone.clone() }),
            level: self.characters.active.as_ref().and(self.progress.level),
            current_area: self.in_zone.then(|| self.progress.current_area.clone()).flatten(),
            in_zone: self.in_zone,
            character: self.characters.active.clone(),
            recognized_character: self.identity.pending.as_ref().map(|c| c.name.clone()),
            characters: self.characters.profiles.keys().cloned().collect(),
            legacy_available: self.characters.legacy.is_some(),
            identity_note: self.identity.note.clone(),
            steps,
            visit: route.map_or(0, |r| r.current_visit(&self.progress)),
            log: self.log.clone(),
        }
    }

    /// A step was ticked by hand: keep the dialogue lines seen just before it, in
    /// `step-observations.jsonl` beside the progress file. Lines that turn up before
    /// the same step every time can become its log trigger. Player chat never gets
    /// here (`parse_line` drops it).
    pub fn note_hand_tick(&self, id: &str) {
        let Some(path) = self.save_path.as_ref().map(|p| p.with_file_name("step-observations.jsonl")) else { return };
        let lines: Vec<_> = self
            .recent
            .iter()
            .map(|(at, speaker, text)| serde_json::json!({ "at": at, "speaker": speaker, "text": text }))
            .collect();
        let at = self.progress.completed.get(id).map(|c| c.at.clone()).unwrap_or_default();
        let entry = serde_json::json!({ "step": id, "area": self.progress.current_area, "at": at, "lines": lines });
        let result = std::fs::OpenOptions::new().create(true).append(true).open(&path).and_then(|mut file| {
            use std::io::Write;
            writeln!(file, "{entry}")
        });
        if let Err(error) = result {
            eprintln!("Tracker: could not note the tick: {error}");
        }
    }

    pub fn save(&self) {
        let Some(path) = &self.save_path else { return };
        let result = (|| -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut characters = self.characters.clone();
            characters.remember(&self.progress);
            let saved = crate::characters::SavedProgress { progress: self.progress.clone(), characters };
            crate::write_through_temp(path, &serde_json::to_vec_pretty(&saved)?)
        })();
        match result {
            Ok(()) => self.saved_at.set(Some(std::time::Instant::now())),
            Err(error) => eprintln!("Tracker: could not save progress: {error}"),
        }
    }
}

pub struct Tracker(pub Mutex<Inner>);

pub fn init(app: &AppHandle) -> Tracker {
    let dir = app.path().app_data_dir().ok();
    let save_path = dir.as_ref().map(|dir| dir.join(PROGRESS_FILE));
    // A file that does not parse is kept as `.bad` (said in the terminal), not dropped.
    let read = |name: &str| {
        let path = dir.as_ref()?.join(name);
        let bytes = std::fs::read(&path).ok()?;
        crate::characters::load(&bytes).map_err(|error| crate::keep_broken(&path, &error.to_string())).ok()
    };
    let routes = load_routes();
    let saved = read(PROGRESS_FILE).or_else(|| read(OLD_PROGRESS_FILE));
    let (mut progress, mut characters) = saved.map(|s| (s.progress, s.characters)).unwrap_or_default();
    // A character may have changed while the overlay was closed, without a new
    // login-server line. Never promote a persisted active name to live identity.
    characters.clear_binding(&mut progress);
    characters.keep_chosen(&mut progress);
    if progress.shown_zone.is_none() {
        progress.shown_zone =
            progress.current_area.clone().filter(|area| routes.iter().any(|r| &r.zone.id == area));
    }
    // Saved location is history. Live log context plus OS presence must confirm it.
    Tracker(Mutex::new(Inner {
        routes,
        progress,
        characters,
        identity: Default::default(),
        log: LogStatus { connected: false, path: None, message: "Looking for Client.txt".into() },
        save_path,
        skip_optional: false,
        recent: Default::default(),
        fight: None,
        in_zone: false,
        saved_at: Default::default(),
    }))
}

// ---------- Log tailing ----------

fn candidate_logs() -> Vec<PathBuf> {
    if let Some(path) = std::env::var_os("POE2_CLIENT_LOG") {
        return vec![PathBuf::from(path)];
    }
    let mut paths = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        for steam in [".local/share/Steam", ".steam/steam", ".var/app/com.valvesoftware.Steam/data/Steam"] {
            paths.push(home.join(steam).join("steamapps/common/Path of Exile 2/logs/Client.txt"));
        }
    }
    paths.push(PathBuf::from(r"C:\Program Files (x86)\Steam\steamapps\common\Path of Exile 2\logs\Client.txt"));
    paths.push(PathBuf::from(r"C:\Program Files (x86)\Grinding Gear Games\Path of Exile 2\logs\Client.txt"));
    paths
}

/// The most recently written candidate log.
fn find_log() -> Option<PathBuf> {
    candidate_logs()
        .into_iter()
        .filter_map(|p| {
            let modified = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
            Some((modified, p))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, p)| p)
}

/// Where to start on first run: the latest game start in the last few MB,
/// so the current play session is replayed but older ones are not.
fn first_run_offset(file: &mut File, len: u64) -> std::io::Result<u64> {
    let start = len.saturating_sub(FIRST_RUN_LOOKBACK);
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    file.take(len - start).read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    let line_start = |i: usize| text[..i].rfind('\n').map_or(0, |n| n + 1);
    let offset = match text.rfind("[STARTUP] Loading") {
        Some(i) => line_start(i),
        None if start == 0 => 0,
        None => text.find('\n').map_or(text.len(), |n| n + 1),
    };
    Ok(start + offset as u64)
}

/// The zone the player is in now: the last zone generation in the last few MB.
fn current_zone(file: &mut File, len: u64) -> std::io::Result<(Option<LogEvent>, bool, Option<String>, Option<String>)> {
    let start = len.saturating_sub(FIRST_RUN_LOOKBACK);
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    file.take(len - start).read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    let session = text.lines().rev().find(|line| session_boundary(line)).map(str::to_string);
    let after = session.as_ref().and_then(|s| text.rfind(s)).map_or(text.as_ref(), |at| &text[at..]);
    let zone = after
        .lines()
        .rev()
        .filter(|line| line.contains("Generating level "))
        .find_map(|line| parse_line(line).map(|(_, event)| event));
    // Who is playing: the last character this session's log named.
    let named = session.as_ref().and_then(|_| after.lines().rev().find_map(character_in_line));
    Ok((zone, zone_since_start(&text), session, named))
}

fn set_log_status(app: &AppHandle, connected: bool, path: Option<&Path>, message: &str) {
    let tracker = app.state::<Tracker>();
    let mut inner = crate::relock(&tracker.0);
    let path = path.map(|p| p.display().to_string());
    if inner.log.connected == connected && inner.log.path == path && inner.log.message == message {
        return;
    }
    inner.log = LogStatus { connected, path, message: message.into() };
    let _ = app.emit("tracker-state", inner.view());
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || loop {
        let Some(path) = find_log() else {
            set_in_zone(&app, false);
            set_log_status(&app, false, None, "Client.txt not found. Set POE2_CLIENT_LOG to its path.");
            std::thread::sleep(Duration::from_secs(3));
            continue;
        };
        if let Err(error) = follow(&app, &path) {
            set_in_zone(&app, false);
            set_log_status(&app, false, Some(&path), &format!("Log read error: {error}"));
            std::thread::sleep(Duration::from_secs(3));
        }
    });
}

/// Follows one log file until an I/O error. Handles truncation by restarting at 0.
fn follow(app: &AppHandle, path: &Path) -> std::io::Result<()> {
    let mut file = File::open(path)?;
    let path_text = path.display().to_string();
    let mut pos = {
        let tracker = app.state::<Tracker>();
        let saved = {
            let inner = crate::relock(&tracker.0);
            (inner.progress.log_path.as_deref() == Some(path_text.as_str()))
                .then_some(inner.progress.log_offset)
                .flatten()
        };
        let len = file.metadata()?.len();
        match saved {
            Some(offset) if offset <= len => offset,
            _ => first_run_offset(&mut file, len)?,
        }
    };
    // Replay the saved cursor before publishing live context. The saved zone may
    // belong to an earlier login, even though a game process is running now.
    set_log_status(app, true, Some(path), "Reading game log (read-only)");

    let mut pending: Vec<u8> = Vec::new();
    // Lines read before first reaching the end of the file are history, not live play.
    let mut live = false;
    loop {
        let len = std::fs::metadata(path)?.len();
        if len < pos {
            // Truncated or replaced: start over from the beginning.
            pos = 0;
            pending.clear();
            set_in_zone(app, false);
        }
        if len > pos {
            file.seek(SeekFrom::Start(pos))?;
            let mut chunk = Vec::new();
            (&mut file).take((len - pos).min(MAX_READ)).read_to_end(&mut chunk)?;
            pos += chunk.len() as u64;
            pending.extend_from_slice(&chunk);
            let Some(last_newline) = pending.iter().rposition(|&b| b == b'\n') else {
                std::thread::sleep(POLL);
                continue;
            };
            let complete: Vec<u8> = pending.drain(..=last_newline).collect();
            let read_through = pos - pending.len() as u64;
            // A panic in one batch loses that batch, not the reading (the lock is taken
            // back with `relock`).
            crate::guarded("Tracker: a log batch", || process(app, &path_text, &complete, read_through, live));
            continue; // more may be waiting beyond MAX_READ
        }
        if !live {
            restore_live_context(app, &mut file, pos - pending.len() as u64)?;
            live = true;
        }
        crate::guarded("Tracker: between log lines", || {
            if crate::game::running() == Some(false) {
                set_in_zone(app, false);
            }
            if crate::relock(&app.state::<Tracker>().0).in_zone {
                if crate::RADAR {
                    tick_map_kills(app);
                }
            }
            settle_quiet_fight(app);
        });
        std::thread::sleep(POLL);
    }
}

fn restore_live_context(app: &AppHandle, file: &mut File, pos: u64) -> std::io::Result<()> {
    let (zone, in_zone, session, named) = current_zone(file, pos)?;
    let running = crate::game::running() == Some(true);
    let tracker = app.state::<Tracker>();
    let mut inner = crate::relock(&tracker.0);
    let Inner { progress, characters, .. } = &mut *inner;
    let new_session = characters.new_session(progress, session);
    characters.keep_chosen(progress);
    if let Some(name) = &named {
        characters.bind_from_log(progress, name);
    }
    if new_session { inner.identity.invalidate(); }
    if let Some(event @ LogEvent::ZoneEntered { ref area, .. }) = zone.as_ref() {
        let Inner { routes, progress, .. } = &mut *inner;
        // Restoring an already-read entry is not another visit. In particular,
        // do not satisfy a later town-return trigger merely by restarting the app.
        if progress.current_area.as_deref() != Some(area.as_str()) {
            apply(routes, progress, "on start", event);
        } else if let LogEvent::ZoneEntered { level, .. } = event {
            progress.area_level = Some(*level);
        }
        if crate::RADAR {
            crate::minimap::set_label_candidates(exit_names(area));
            app.state::<crate::layout::Recorder>().on_log(event, false);
        }
    }
    inner.in_zone = running && in_zone && zone.is_some();
    if !running {
        let Inner { progress, characters, .. } = &mut *inner;
        characters.new_session(progress, None);
        characters.keep_chosen(progress);
    }
    inner.save();
    crate::identity::set_context(inner.in_zone, inner.characters.active.is_some());
    inner.sync_navigation(app);
    let in_zone = inner.in_zone;
    let _ = app.emit("tracker-state", inner.view());
    drop(inner);
    let _ = app.emit("in-zone", in_zone);
    Ok(())
}

/// The names of the zones `area`'s exits lead to (its layout file's
/// `connectionNames`); empty when the area has no layout file.
fn exit_names(area: &str) -> Vec<String> {
    crate::bundle::LAYOUTS
        .iter()
        .find(|(id, _)| *id == area)
        .and_then(|(_, json)| serde_json::from_str::<serde_json::Value>(json).ok())
        .and_then(|file| file.get("connectionNames")?.as_array().cloned())
        .map(|names| {
            // Two exits to the same zone share a name: listed once, or no label could lead.
            let mut unique: Vec<String> = Vec::new();
            for name in names.iter().filter_map(|n| n.as_str()) {
                if !unique.iter().any(|u| u == name) {
                    unique.push(name.to_string());
                }
            }
            unique
        })
        .unwrap_or_default()
}

fn process(app: &AppHandle, path: &str, bytes: &[u8], read_through: u64, live: bool) {
    let recorder = app.state::<crate::layout::Recorder>();
    let tracker = app.state::<Tracker>();
    let mut inner = crate::relock(&tracker.0);
    let fighting = inner.fight.is_some();
    let was_in_zone = inner.in_zone;
    let mut changed = inner.progress.log_path.as_deref() != Some(path);
    let running = crate::game::running() == Some(true);
    // Every line is handled in order; one event never hides the rest of a batch.
    for line in String::from_utf8_lossy(bytes).lines() {
        if live && is_death(line) {
            inner.fight = None;
        }
        // Who is playing, from the log (the main source on every platform), and its level.
        changed |= note_character(&mut inner, line);
        if session_boundary(line) {
            crate::identity::trace("session-boundary",serde_json::json!({"epoch":inner.identity.epoch,"seq":inner.identity.seq,"pending":inner.identity.pending,"live":live}));
            inner.identity.invalidate();
            crate::identity::set_context(false, false);
            let Inner { progress, characters, .. } = &mut *inner;
            changed |= characters.new_session(progress, Some(line.to_string()));
            characters.keep_chosen(progress);
            inner.in_zone = false;
            inner.fight = None;
            inner.recent.clear();
        }
        if let Some((timestamp, event)) = parse_line(line) {
            let Inner { routes, progress, recent, fight, in_zone, characters, .. } = &mut *inner;
            // Not the backlog read on start: it may be another character's.
            if live && characters.active.is_none() {
                if characters.unbound.len() == crate::characters::UNBOUND_EVENTS {
                    characters.unbound.remove(0);
                }
                characters.unbound.push((timestamp.clone(), event.clone(), progress.current_area.clone()));
            }
            changed |= apply(routes, progress, &timestamp, &event);
            match &event {
                // A new zone: whatever fight there was is behind the player.
                LogEvent::ZoneEntered { .. } => {
                    *fight = None;
                    *in_zone = running && live;
                }
                LogEvent::Dialogue { speaker, text } if live && *in_zone => {
                    changed |= hear_boss(routes, progress, fight, speaker, text);
                }
                LogEvent::Dialogue { .. } => {}
            }
            if crate::RADAR {
                if let LogEvent::ZoneEntered { area, .. } = &event {
                    // The big map's exit labels are read against this zone's exit names.
                    crate::minimap::set_label_candidates(exit_names(area));
                }
                recorder.on_log(&event, live);
            }
            if let (true, LogEvent::Dialogue { speaker, text }) = (live, &event) {
                recent.push_back((timestamp, speaker.clone(), text.clone()));
                if recent.len() > RECENT_LINES {
                    recent.pop_front();
                }
            }
        }
    }
    inner.progress.log_path = Some(path.to_string());
    inner.progress.log_offset = Some(read_through);
    if inner.in_zone != was_in_zone {
        crate::identity::trace("zone-state",serde_json::json!({"inZone":inner.in_zone,"area":inner.progress.current_area,"epoch":inner.identity.epoch,"seq":inner.identity.seq,"latest":crate::identity::latest_input(),"submitted":inner.identity.submitted,"pending":inner.identity.pending}));
    }
    crate::identity::set_context(inner.in_zone, inner.characters.active.is_some());
    changed |= crate::identity::bind_pending(&mut inner);
    changed |= inner.in_zone != was_in_zone;
    if save_due(changed, inner.saved_at.get(), std::time::Instant::now()) {
        inner.save();
    }
    if changed {
        inner.sync_navigation(app);
        let _ = app.emit("tracker-state", inner.view());
    }
    // After the checklist is up to date: a kill ticked in this batch ends the fight. A boss
    // heard earlier and quiet since is ticked "likely" here: shown at once.
    let before = inner.progress.completed.clone();
    inner.settle_fight();
    if inner.progress.completed != before {
        inner.sync_navigation(app);
        inner.save();
        let _ = app.emit("tracker-state", inner.view());
    }
    let now = inner.fight.is_some();
    let in_zone = inner.in_zone;
    drop(inner);
    if now != fighting {
        crate::set_fight(app, now);
    }
    if in_zone != was_in_zone {
        let _ = app.emit("in-zone", in_zone);
    }
}

/// Sets whether the player is in a zone and tells the compass.
fn set_in_zone(app: &AppHandle, in_zone: bool) {
    let tracker = app.state::<Tracker>();
    let mut inner = crate::relock(&tracker.0);
    let changed = inner.in_zone != in_zone;
    inner.in_zone = in_zone;
    let mut profile_changed = false;
    if !in_zone {
        crate::identity::set_context(false, false);
        inner.fight = None;
        let Inner { characters, progress, .. } = &mut *inner;
        profile_changed = characters.new_session(progress, None);
        characters.keep_chosen(progress);
        if changed || profile_changed || inner.identity.pending.is_some() {
            inner.identity.invalidate();
        }
    }
    if profile_changed { inner.save(); }
    if changed || profile_changed {
        inner.sync_navigation(app);
        let _ = app.emit("tracker-state", inner.view());
    }
    drop(inner);
    if !changed && !profile_changed { return; }
    if !in_zone { crate::set_fight(app, false); }
    let _ = app.emit("in-zone", in_zone);
}

/// Between log lines: ticks the kill of a boss the map shows dead (the log has no line
/// for most kills). Asked twice a second; the recorder's answer is a short list.
fn tick_map_kills(app: &AppHandle) {
    let dead = app.state::<crate::layout::Recorder>().dead_bosses();
    if dead.is_empty() {
        return;
    }
    let tracker = app.state::<Tracker>();
    let mut inner = crate::relock(&tracker.0);
    if !inner.tick_dead_bosses(&dead) {
        return;
    }
    inner.sync_navigation(app);
    inner.save();
    let _ = app.emit("tracker-state", inner.view());
    let fight_over = inner.settle_fight();
    drop(inner);
    if fight_over {
        crate::set_fight(app, false);
    }
}

/// Between log lines: ends a fight whose boss has gone quiet.
fn settle_quiet_fight(app: &AppHandle) {
    let tracker = app.state::<Tracker>();
    let mut inner = crate::relock(&tracker.0);
    let before = inner.progress.completed.len();
    let ended = inner.settle_fight();
    if inner.progress.completed.len() != before {
        inner.sync_navigation(app);
        inner.save();
        let _ = app.emit("tracker-state", inner.view());
    }
    drop(inner);
    if ended {
        crate::set_fight(app, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(tag: &str, message: &str) -> String {
        format!("2026/11/11 22:41:55 547102231 3ef23348 [{tag} Client 308] {message}")
    }

    #[test]
    fn parses_zone_generation() {
        let (ts, event) =
            parse_line(&line("DEBUG", r#"Generating level 1 area "G1_1" with seed 1256743153"#)).unwrap();
        assert_eq!(ts, "2026/11/11 22:41:55");
        assert_eq!(event, LogEvent::ZoneEntered { area: "G1_1".into(), level: 1, seed: Some(1256743153) });
    }

    /// The tracker reads of the owner's 2026-11-13 run, as `quest_screen` confirms them.
    #[test]
    fn the_quest_tracker_ticks_the_miller_and_renlys_reward() {
        use crate::quest_screen::{read, Watcher};
        let routes = load_routes();
        let mut progress = Progress::default();
        let mut watcher = Watcher::default();
        let mut feed = |progress: &mut Progress, lines: &[&str], times: usize| {
            let lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
            for _ in 0..times {
                for c in watcher.frame(&read(&lines)) {
                    apply_screen(&routes, progress, &c.quest, &c.orders, &c.evidence);
                }
            }
        };
        feed(&mut progress, &["Reaching Clearfell", "Kill the Bloated Miller and end his rage"], 2);
        assert!(progress.completed.contains_key("talk-wounded-man"));
        assert!(!progress.completed.contains_key("kill-bloated-miller"));
        // The kill: the tracker moves on at once, two minutes before Mortimer's line.
        feed(&mut progress, &["Reaching Clearfell", "Enter the logging encampment"], 2);
        let kill = &progress.completed["kill-bloated-miller"];
        assert_eq!(kill.source, "screen");
        assert!(kill.evidence.contains("Enter the logging encampment"));
        assert!(!progress.completed.contains_key("talk-renly"));
        // In town, then Renly's reward: the quest leaves the list, his next one shows.
        feed(&mut progress, &["Reaching Clearfell", "Talk to the Blacksmith"], 2);
        assert!(!progress.completed.contains_key("talk-renly"));
        feed(&mut progress, &["Treacherous Ground", "Search Clearfell to find the Mud Burrow entrance", "Slay the Devourer in its lair"], 3);
        assert_eq!(progress.completed["talk-renly"].source, "screen");
        // A step unticked by hand is not ticked again.
        let mut progress = Progress::default();
        progress.dismissed.insert("kill-bloated-miller".into());
        apply_screen(&routes, &mut progress, "TheMiller", &BTreeSet::from([4]), "test");
        assert!(!progress.completed.contains_key("kill-bloated-miller"));
    }

    #[test]
    fn every_screen_condition_names_real_tracker_lines() {
        // `load_routes` resolves each condition against data/quests/states.json and
        // panics on an unknown quest or line.
        let routes = load_routes();
        let count = routes.iter().flat_map(|r| &r.steps).filter(|s| !s.screen.is_empty()).count();
        assert!(count >= 100, "{count} steps read from the quest tracker");
    }

    #[test]
    fn miller_dialogue_is_not_completion_and_manual_completion_has_a_navigation_target() {
        let routes = load_routes();
        let mut progress = Progress::default();
        apply(&routes, &mut progress, "now", &LogEvent::ZoneEntered {
            area: "G1_1".into(), level: 1, seed: Some(7),
        });
        apply(&routes, &mut progress, "now", &LogEvent::Dialogue {
            speaker: "The Bloated Miller".into(), text: "Fight starts".into(),
        });
        assert!(!progress.completed.contains_key("kill-bloated-miller"));
        apply(&routes, &mut progress, "now", &LogEvent::Dialogue {
            speaker: "Mortimer".into(), text: "You there! Please, aid us!".into(),
        });
        assert!(!progress.completed.contains_key("kill-bloated-miller"));
        toggle(&routes, &mut progress, "kill-bloated-miller").unwrap();
        let step = routes[0].steps.iter().find(|s| s.id == "kill-bloated-miller").unwrap();
        let target = step.navigation_target.as_ref().unwrap();
        assert_eq!(target.area, "G1_1");
        assert_eq!(target.label, "The Bloated Miller");
        assert!(progress.completed.contains_key(&step.id));
    }

    #[test]
    fn parses_npc_dialogue_but_not_chat_or_system_lines() {
        let (_, event) = parse_line(&line("INFO", "Wounded Man: Reach... Clearfell... Find the Miller...")).unwrap();
        assert_eq!(
            event,
            LogEvent::Dialogue { speaker: "Wounded Man".into(), text: "Reach... Clearfell... Find the Miller...".into() }
        );
        assert!(parse_line(&line("INFO", "#Someone: Wounded Man: hi")).is_none());
        assert!(parse_line(&line("INFO", "&: GUILD UPDATE: hello")).is_none());
        assert!(parse_line(&line("INFO", "[SCENE] Set Source [The Riverbank]")).is_none());
        assert!(parse_line(&line("WARN", "Something: failed")).is_none());
        // The game's own "X: Y" lines, as in the owner's log: no one speaks there.
        for system in [
            "Successfully allocated passive skill id: projectiles15, name: Projectile Damage",
            "Successfully unallocated passive skill id: projectiles15, name: Projectile Damage",
            "Enumerated adapter: AMD Radeon RX 7900 XTX (RADV NAVI31)",
            "Enumerated device for adapter: AMD Radeon RX 7900 XTX (RADV NAVI31). Selected feature level: 49664.",
            "Tile hash: 1435737370",
            "Doodad hash: 1446108392",
            "Abnormal disconnect: An unexpected disconnection occurred.",
        ] {
            assert!(parse_line(&line("INFO", system)).is_none(), "{system}");
        }
        // NPCs of the owner's log, names with spaces too, still speak.
        for (speaker, said) in [("The Bloated Miller", "Saw you in half!"), ("Beira of the Rotten Pack", "Sleep in ice, wake in snow."), ("Una", "Please, take your time.")] {
            let (_, event) = parse_line(&line("INFO", &format!("{speaker}: {said}"))).unwrap();
            assert_eq!(event, LogEvent::Dialogue { speaker: speaker.into(), text: said.into() });
        }
        // No line a checklist waits for is taken for a system line.
        for step in load_routes().iter().flat_map(|r| &r.steps) {
            if let Some(Trigger::NpcLine { lines, .. }) = &step.trigger {
                assert!(lines.iter().all(|l| !is_system_line(&l.speaker, &l.text)), "{}", step.id);
            }
        }
    }

    fn feed(progress: &mut Progress, lines: &[String]) {
        let routes = load_routes();
        for l in lines {
            if let Some((ts, event)) = parse_line(l) {
                apply(&routes, progress, &ts, &event);
            }
        }
    }

    #[test]
    fn wounded_man_completes_only_on_his_last_line_in_the_riverbank() {
        let mut progress = Progress::default();
        feed(
            &mut progress,
            &[
                line("DEBUG", r#"Generating level 1 area "G1_1" with seed 1"#),
                line("INFO", "Wounded Man: By the First Ones! You're alive!"),
            ],
        );
        assert!(!progress.completed.contains_key("talk-wounded-man"));
        feed(&mut progress, &[line("INFO", "Wounded Man: Reach... Clearfell... Find the Miller...")]);
        assert_eq!(progress.completed["talk-wounded-man"].source, "log");
    }

    #[test]
    fn mortimer_thanking_you_completes_the_miller() {
        let mut progress = Progress::default();
        feed(
            &mut progress,
            &[
                line("DEBUG", r#"Generating level 1 area "G1_1" with seed 1"#),
                line("INFO", "Mortimer: You there! Please, aid us!"),
                line("INFO", "The Bloated Miller: Saw you in half!"),
                line("INFO", "The Bloated Miller: Die!"),
            ],
        );
        assert!(!progress.completed.contains_key("kill-bloated-miller"));
        feed(&mut progress, &[line("INFO", "Mortimer: Well done! Please come inside.")]);
        assert_eq!(progress.completed["kill-bloated-miller"].source, "log");
    }

    #[test]
    fn dialogue_outside_the_riverbank_does_not_count() {
        let mut progress = Progress::default();
        feed(
            &mut progress,
            &[
                line("DEBUG", r#"Generating level 1 area "G1_1" with seed 1"#),
                line("DEBUG", r#"Generating level 15 area "G1_town" with seed 1"#),
                line("INFO", "Wounded Man: Reach... Clearfell... Find the Miller..."),
            ],
        );
        // Not ticked by the line (the town entry takes the step before it as done).
        assert_eq!(progress.completed["talk-wounded-man"].source, "implied");
        assert!(progress.completed.contains_key("enter-clearfell-encampment"));
    }

    #[test]
    fn the_checklist_follows_the_zone_and_zone_entries_tick() {
        let routes = load_routes();
        let ids: Vec<&str> = routes.iter().flat_map(|r| &r.steps).map(|s| s.id.as_str()).collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "step ids are unique across checklists");

        let mut progress = Progress::default();
        feed(&mut progress, &[line("DEBUG", r#"Generating level 15 area "G1_town" with seed 1"#)]);
        assert_eq!(progress.shown_zone.as_deref(), Some("G1_town"));
        feed(&mut progress, &[line("DEBUG", r#"Generating level 2 area "G1_2" with seed 5"#)]);
        assert_eq!(progress.shown_zone.as_deref(), Some("G1_2"));
        assert!(progress.completed.contains_key("enter-clearfell"));
        feed(&mut progress, &[line("DEBUG", r#"Generating level 3 area "G1_3" with seed 6"#)]);
        // Mud Burrow has its own checklist.
        assert_eq!(progress.shown_zone.as_deref(), Some("G1_3"));
        assert_eq!(progress.current_area.as_deref(), Some("G1_3"));
        assert!(progress.completed.contains_key("enter-mud-burrow"));
        toggle(&routes, &mut progress, "kill-beira").unwrap();
        assert_eq!(progress.completed["kill-beira"].source, "manual");
    }

    #[test]
    fn every_act_one_zone_has_a_checklist_in_campaign_order() {
        let routes = load_routes();
        let zones: Vec<&str> = routes.iter().map(|r| r.zone.id.as_str()).collect();
        let act1 = [
            "G1_1", "G1_town", "G1_2", "G1_3", "G1_4", "G1_5", "G1_6", "G1_7", "G1_8", "G1_9", "G1_11", "G1_12",
            "G1_13_1", "G1_13_2", "G1_14", "G1_15", "G1_10",
        ];
        assert_eq!(zones[..act1.len()], act1);
        let mut progress = Progress::default();
        feed(&mut progress, &[line("DEBUG", r#"Generating level 4 area "G1_4" with seed 7"#)]);
        assert_eq!(progress.shown_zone.as_deref(), Some("G1_4"));
        feed(&mut progress, &[line("DEBUG", r#"Generating level 5 area "G1_5" with seed 8"#)]);
        // One step waits for the Red Vale's zone line (the waypoint detour there is gone).
        assert!(progress.completed.contains_key("enter-red-vale"));
        assert!(!progress.completed.contains_key("runic-seals"));
    }

    /// The Act 1 fast route (2026-11-13, R2): every pickup the fastest guide takes (GTD-video,
    /// GTD-build) is a route row, listed with "Optional content" off; what it skips is not a
    /// required row; a reward shows only where it is the reason for the row.
    #[test]
    fn act_one_is_the_fast_route() {
        let routes = load_routes();
        let step = |id: &str| routes.iter().flat_map(|r| &r.steps).find(|s| s.id == id);
        let text = |id: &str| step(id).unwrap().parts.iter().map(|p| p.text.as_str()).collect::<String>();
        let reward = |id: &str| step(id).unwrap().parts.iter().find(|p| p.style.as_deref() == Some("reward")).map(|p| p.text.clone());
        let pickups = [
            ("abandoned-stash", "Skill Gem"), ("kill-areagne", "Support Gem"), ("kill-ervig", "Support Gem"),
            ("ancient-ruins", "Ring"), ("haunted-treasure", "Support Gem"), ("kill-crowbell", "2 Passive Points"),
            ("small-ritual", "Support Gem"), ("kill-king-in-the-mists", "+30 Spirit"), ("finn-charm", "Charm"),
            ("crop-circle", "Skill Gem"), ("deliver-unas-lute", "2 Passive Points"), ("smithing-tools", "Salvage Bench"),
            ("blacksmiths-chest", "Rune + Orb"), ("leitis-gem", "Skill Gem"), ("gallows-secret", "Support Gem"),
            ("kill-candlemass", "+20 Max Life"),
        ];
        for (id, gives) in pickups {
            let row = step(id).unwrap_or_else(|| panic!("{id}"));
            assert!(!row.optional, "{id} is a route row");
            assert_eq!(reward(id).as_deref(), Some(gives), "{id}");
        }
        // Skipped: the Mud Burrow, side content; gone: the ring's town trip and the talks.
        for id in ["enter-mud-burrow", "kill-devourer", "kill-gerung", "talk-delwyn", "forgotten-riches", "renly-support-gem"] {
            assert!(step(id).unwrap().optional, "{id} stays optional");
        }
        for id in [
            "town-talk-finn", "town-una-cemetery", "town-hooded-one", "town-talk-all", "grelwood-red-vale-wp",
            "hunting-grounds-town", "freythorn-wp", "farmlands-wp", "freythorn-to-farmlands", "ominous-altars",
        ] {
            assert!(step(id).is_none(), "{id} is dropped");
        }
        assert_eq!(text("kill-lachlann"), "Kill Lachlann, leave his ring");
        // Bosses killed on the way carry no reward (decision 5).
        for id in ["kill-beira", "kill-rust-king", "kill-lachlann", "kill-draven", "kill-asinia", "kill-executioner", "kill-geonor"] {
            assert_eq!(reward(id), None, "{id}");
        }
        // Outside Act 1: the letter pays the points, and the heart for Aggorat is a route row.
        assert_eq!(reward("deliver-lihl-lima-letter").as_deref(), Some("2 Passive Points"));
        assert!(!step("deliver-lihl-lima-letter").unwrap().optional);
        assert_eq!(reward("fallen-dekhara"), None);
        assert!(!step("sacrificial-heart").unwrap().optional);
    }

    #[test]
    fn a_zone_entry_with_after_waits_for_that_step() {
        let route: Route = serde_json::from_str(
            r#"{"zone":{"id":"A","name":"A","level":1},"steps":[
                {"id":"detour","kind":"objective","parts":[],"trigger":{"type":"zoneEntered","area":"B"}},
                {"id":"move","kind":"objective","parts":[],"trigger":{"type":"zoneEntered","area":"B","after":"detour"}}
            ]}"#,
        )
        .unwrap();
        let routes = [route];
        let mut progress = Progress::default();
        let enter = |progress: &mut Progress, area: &str| {
            let (ts, event) = parse_line(&line("DEBUG", &format!(r#"Generating level 1 area "{area}" with seed 1"#))).unwrap();
            apply(&routes, progress, &ts, &event);
        };
        // The first entry ticks the detour only: `after` must be done before the entry.
        enter(&mut progress, "B");
        assert!(progress.completed.contains_key("detour"));
        assert!(!progress.completed.contains_key("move"));
        enter(&mut progress, "A");
        enter(&mut progress, "B");
        assert!(progress.completed.contains_key("move"));
    }

    #[test]
    fn events_read_unbound_are_judged_again_against_the_chosen_profile() {
        let route: Route = serde_json::from_str(
            r#"{"zone":{"id":"A","name":"A","level":1},"steps":[
                {"id":"detour","kind":"objective","parts":[],"trigger":{"type":"zoneEntered","area":"B"}},
                {"id":"move","kind":"objective","parts":[],"trigger":{"type":"zoneEntered","area":"B","after":"detour"}},
                {"id":"talk","kind":"objective","parts":[],"trigger":{"type":"npcLine","area":"B","after":"move","lines":[
                    {"speaker":"Una","text":"Hello."}]}}
            ]}"#,
        )
        .unwrap();
        let mut inner = Inner {
            routes: vec![route], progress: Progress::default(), characters: Default::default(),
            identity: Default::default(),
            log: LogStatus { connected: true, path: None, message: String::new() },
            save_path: None, skip_optional: false, recent: Default::default(), fight: None,
            in_zone: false, saved_at: Default::default(),
        };
        // The hero did the detour last login.
        inner.characters.select(&mut inner.progress, "Hero", false).unwrap();
        enter_zone(&inner.routes, &mut inner.progress, "B");
        enter_zone(&inner.routes, &mut inner.progress, "A");
        inner.characters.new_session(&mut inner.progress, Some("login-2".into()));
        // A new login enters B and hears Una before the character is known (as `process` does).
        let events = [
            LogEvent::ZoneEntered { area: "B".into(), level: 1, seed: None },
            LogEvent::Dialogue { speaker: "Una".into(), text: "Hello.".into() },
        ];
        for event in events {
            let area = inner.progress.current_area.clone();
            inner.characters.unbound.push(("t".into(), event.clone(), area));
            apply(&inner.routes, &mut inner.progress, "t", &event);
        }
        assert!(!inner.progress.completed.contains_key("move"), "judged against no earlier ticks");
        inner.characters.select(&mut inner.progress, "Hero", false).unwrap();
        assert!(replay_unbound(&mut inner));
        assert!(inner.progress.completed.contains_key("move"), "the hero's detour counts");
        assert!(inner.progress.completed.contains_key("talk"));
        assert_eq!(inner.progress.current_area.as_deref(), Some("B"));
        assert!(inner.characters.unbound.is_empty());
        assert!(inner.characters.profiles["Hero"].completed.contains_key("talk"), "saved in the profile");
    }

    fn one_route(steps: &str) -> [Route; 1] {
        [serde_json::from_str(&format!(r#"{{"zone":{{"id":"A","name":"A","level":1}},"steps":[{steps}]}}"#)).unwrap()]
    }

    fn say(routes: &[Route], progress: &mut Progress, speaker: &str, text: &str) {
        apply(routes, progress, "now", &LogEvent::Dialogue { speaker: speaker.into(), text: text.into() });
    }

    fn enter_zone(routes: &[Route], progress: &mut Progress, area: &str) {
        apply(routes, progress, "now", &LogEvent::ZoneEntered { area: area.into(), level: 1, seed: None });
    }

    #[test]
    fn an_npc_line_trigger_takes_any_of_several_lines() {
        let routes = one_route(
            r#"{"id":"kill","kind":"objective","parts":[],"trigger":{"type":"npcLine","area":"A","lines":[
                {"speaker":"Boss","text":"I fall..."},
                {"speaker":"Boss","text":"It ends..."},
                {"speaker":"Guard","text":"The beast is dead!"}]}}"#,
        );
        for (speaker, text) in [("Boss", "It ends..."), ("Guard", "The beast is dead!")] {
            let mut progress = Progress::default();
            enter_zone(&routes, &mut progress, "A");
            say(&routes, &mut progress, "Guard", "It ends...");
            assert!(!progress.completed.contains_key("kill"), "the text of one line with the speaker of another");
            say(&routes, &mut progress, speaker, text);
            assert_eq!(progress.completed["kill"].evidence, format!("{speaker}: {text}"));
        }
        // Bound to its zone like the one-line form.
        let mut progress = Progress::default();
        enter_zone(&routes, &mut progress, "B");
        say(&routes, &mut progress, "Boss", "It ends...");
        assert!(!progress.completed.contains_key("kill"));
    }

    #[test]
    fn an_npc_line_with_after_waits_for_that_step() {
        let routes = one_route(
            r#"{"id":"first","kind":"objective","parts":[],"trigger":{"type":"npcLine","area":"A","speaker":"Una","text":"Thank you."}},
               {"id":"second","kind":"objective","parts":[],"trigger":{"type":"npcLine","area":"A","speaker":"Una","text":"Thank you.","after":"first"}}"#,
        );
        let mut progress = Progress::default();
        enter_zone(&routes, &mut progress, "A");
        // The line that ticks `first` doesn't also tick `second`.
        say(&routes, &mut progress, "Una", "Thank you.");
        assert!(progress.completed.contains_key("first"));
        assert!(!progress.completed.contains_key("second"));
        say(&routes, &mut progress, "Una", "Thank you.");
        assert!(progress.completed.contains_key("second"));
    }

    #[test]
    fn npc_lines_compare_in_their_shown_form() {
        assert_eq!(shown_text("My home died with my <i>{akhara}. Already"), "My home died with my akhara. Already");
        assert_eq!(shown_text("Something's wrong. <continue>Something's off.\r\n\r\nGo."), "Something's wrong. Something's off. Go.");
        assert_eq!(shown_text("  All we wanted...\t"), "All we wanted...");
        assert_eq!(shown_text("\u{a0}Yep.<br/>All good."), "Yep. All good.");
        assert_eq!(shown_text("a < b"), "a < b");
        let routes = one_route(
            r#"{"id":"talk","kind":"objective","parts":[],"trigger":{"type":"npcLine","area":"A","speaker":"Jun","text":"My home died with my <i>{akhara}.\r\n<continue>Go."}}"#,
        );
        for logged in ["My home died with my akhara. Go.", "My home died with my <i>{akhara}.  Go."] {
            let mut progress = Progress::default();
            enter_zone(&routes, &mut progress, "A");
            say(&routes, &mut progress, "Jun", logged);
            assert!(progress.completed.contains_key("talk"), "{logged}");
        }
    }

    #[test]
    fn a_malformed_npc_line_trigger_does_not_load() {
        let parse = |trigger: &str| serde_json::from_str::<Trigger>(trigger).map(|_| ());
        assert!(parse(r#"{"type":"npcLine","area":"A","speaker":"S","text":"T"}"#).is_ok());
        assert!(parse(r#"{"type":"npcLine","area":"A","lines":[{"speaker":"S","text":"T"}],"after":"x"}"#).is_ok());
        assert!(parse(r#"{"type":"npcLine","area":"A"}"#).is_err(), "no line");
        assert!(parse(r#"{"type":"npcLine","area":"A","speaker":"S"}"#).is_err(), "no text");
        assert!(parse(r#"{"type":"npcLine","area":"A","lines":[]}"#).is_err(), "empty lines");
        assert!(
            parse(r#"{"type":"npcLine","area":"A","speaker":"S","text":"T","lines":[{"speaker":"S","text":"U"}]}"#).is_err(),
            "both forms"
        );
        assert!(parse(r#"{"type":"npcLine","area":"A","speaker":"S","texts":["T"]}"#).is_err(), "unknown field");
        assert!(
            parse(r#"{"type":"npcLine","area":"A","lines":[{"speaker":"S","text":"T","kind":"bark"}]}"#).is_err(),
            "unknown field in a line"
        );
        assert!(parse(r#"{"type":"npcLine","area":"A","lines":[{"speaker":"S","text":""}]}"#).is_err(), "empty text");
        assert!(parse(r#"{"type":"npcLine","area":"A","lines":[{"speaker":" ","text":"T"}]}"#).is_err(), "empty speaker");
        assert!(parse(r#"{"type":"npcLine","area":"A","speaker":"S","text":"<continue>"}"#).is_err(), "text empty when shown");
        assert!(parse(r#"{"type":"zoneEntered","area":"A","after":"x"}"#).is_ok());
    }

    #[test]
    fn game_data_triggers_tick_their_step_in_its_zone_only() {
        let routes = load_routes();
        // The first line of a shipped trigger, as the log would show it.
        let line_of = |id: &str| {
            let step = routes.iter().flat_map(|r| &r.steps).find(|s| s.id == id).unwrap();
            match &step.trigger {
                Some(Trigger::NpcLine { lines, .. }) => (lines[0].speaker.clone(), lines[0].text.clone()),
                other => panic!("{id}: {other:?}"),
            }
        };
        let run = |area: &str, id: &str| {
            let (speaker, text) = line_of(id);
            let mut progress = Progress::default();
            enter_zone(&routes, &mut progress, area);
            let before = progress.completed.clone();
            say(&routes, &mut progress, &speaker, &text);
            // What the line ticked (zone entries tick steps of their own).
            progress
                .completed
                .iter()
                .filter(|(k, c)| !before.contains_key(*k) && c.source == "log")
                .map(|(k, _)| k.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(line_of("kill-lachlann"), ("Lachlann of Endless Lament".into(), "Together... at last...".into()));
        assert_eq!(run("G1_7", "kill-lachlann"), ["kill-lachlann"]);
        assert!(run("G1_8", "kill-lachlann").is_empty());
        assert_eq!(run("G1_12", "kill-king-in-the-mists"), ["kill-king-in-the-mists"]);
        assert!(run("G1_town", "kill-king-in-the-mists").is_empty());
    }

    #[test]
    fn every_after_names_another_step() {
        let routes = load_routes();
        let ids: BTreeSet<&str> = routes.iter().flat_map(|r| &r.steps).map(|s| s.id.as_str()).collect();
        for step in routes.iter().flat_map(|r| &r.steps) {
            let after = match &step.trigger {
                Some(Trigger::NpcLine { after, .. }) | Some(Trigger::ZoneEntered { after, .. }) => after,
                None => continue,
            };
            if let Some(after) = after {
                assert!(ids.contains(after.as_str()), "{}: after {after}", step.id);
                assert_ne!(after, &step.id, "{}: after itself", step.id);
            }
        }
    }

    /// The pairs of rows one log event cannot tell apart (the class check below). A zone entry
    /// ticks only rows of the list of the zone the player left, and of one list only the last
    /// row whose `after` holds (`apply`). So two zone entries into the same area clash when
    /// their lists belong to the same zone (one list, or two lists of one zone) and the later
    /// has no `after`, or the earlier's: it would always win and the earlier never tick (the
    /// Grelwood's "Red Vale WP, return" on v0.1.118). A detour that returns is no exception
    /// (R-F4 of 2026-11-13). Two NPC-line triggers of one list sharing a line, the same.
    /// Read from the route files as written.
    fn clashes(files: &[(String, serde_json::Value)]) -> Vec<String> {
        use serde_json::Value;
        let steps = |file: &Value| file["steps"].as_array().cloned().unwrap_or_default();
        let lines = |step: &Value| -> BTreeSet<(String, String)> {
            let trigger = &step["trigger"];
            let mut set: BTreeSet<(String, String)> = trigger["lines"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|l| (shown_text(l["speaker"].as_str().unwrap()), shown_text(l["text"].as_str().unwrap())))
                .collect();
            if let (Some(speaker), Some(text)) = (trigger["speaker"].as_str(), trigger["text"].as_str()) {
                set.insert((shown_text(speaker), shown_text(text)));
            }
            set
        };
        // Every step with a trigger, with its file and the zone whose list holds it, in order.
        let all: Vec<(&str, &str, Value)> = files
            .iter()
            .flat_map(|(name, file)| {
                let zone = file["zone"]["id"].as_str().unwrap();
                steps(file).into_iter().map(move |s| (name.as_str(), zone, s))
            })
            .filter(|(_, _, s)| s["trigger"]["area"].is_string())
            .collect();
        let mut clashes = Vec::new();
        for (i, (file, zone, earlier)) in all.iter().enumerate() {
            let (kind, area) = (earlier["trigger"]["type"].as_str(), earlier["trigger"]["area"].as_str().unwrap());
            for (other, other_zone, later) in &all[i + 1..] {
                if other_zone != zone || later["trigger"]["type"].as_str() != kind || later["trigger"]["area"].as_str() != Some(area) {
                    continue;
                }
                // One event ticks the last row whose `after` holds: the later needs an `after`
                // of its own, or it always wins and the earlier never ticks.
                let (first, second) = (earlier["trigger"]["after"].as_str(), later["trigger"]["after"].as_str());
                let apart = second.is_some() && second != first;
                let same_event = match kind {
                    Some("zoneEntered") => !apart,
                    Some("npcLine") => !apart && file == other && !lines(earlier).is_disjoint(&lines(later)),
                    _ => false,
                };
                if same_event {
                    clashes.push(format!("{file}/{other}: {} and {} ({area})", earlier["id"], later["id"]));
                }
            }
        }
        clashes
    }

    fn route_files() -> Vec<(String, serde_json::Value)> {
        crate::bundle::ROUTES.iter().map(|(name, json)| (name.to_string(), serde_json::from_str(json).unwrap())).collect()
    }

    /// The class check of the levelling build (2026-11-13, R0): no log event may tick two
    /// steps of one zone's list. On v0.1.118 the Grelwood's "Red Vale WP, return" and "Enter
    /// The Red Vale" both ticked on the first Red Vale entry, so the visit between them could
    /// never show (Areagne with it). Planted pairs show the check catches the kind.
    #[test]
    fn no_log_event_ticks_two_steps_of_one_list() {
        let clashes_now = clashes(&route_files());
        assert!(clashes_now.is_empty(), "one event ticks both:\n{}", clashes_now.join("\n"));
        let file = |zone: &str, steps: &str| {
            serde_json::from_str::<serde_json::Value>(&format!(r#"{{"zone":{{"id":"{zone}"}},"steps":[{steps}]}}"#)).unwrap()
        };
        let entry = |id: &str, area: &str, after: Option<&str>| {
            let after = after.map_or(String::new(), |a| format!(r#","after":"{a}""#));
            format!(r#"{{"id":"{id}","trigger":{{"type":"zoneEntered","area":"{area}"{after}}}}}"#)
        };
        // One list: the Grelwood's old pair.
        let one = [("a".to_string(), file("A", &format!("{},{}", entry("wp", "B", None), entry("enter", "B", None))))];
        assert_eq!(clashes(&one).len(), 1);
        // The later waits for the earlier: fine.
        let ordered = [("a".to_string(), file("A", &format!("{},{}", entry("wp", "B", None), entry("enter", "B", Some("wp")))))];
        assert!(clashes(&ordered).is_empty());
        // Two lists of one zone, each with an entry into B that one entry from A matches.
        let two = [("a".to_string(), file("A", &entry("first", "B", None))), ("a2".to_string(), file("A", &entry("second", "B", None)))];
        assert_eq!(clashes(&two), ["a/a2: \"first\" and \"second\" (B)"]);
        // A detour that returns is no exception (R-F4): with no `after` the later always wins.
        let detour = format!(r#"{{"id":"wp","returns":true,"trigger":{{"type":"zoneEntered","area":"B"}}}},{}"#, entry("enter", "B", None));
        assert_eq!(clashes(&[("a".to_string(), file("A", &detour))]).len(), 1);
        // Two lists of different zones: an entry from one zone ticks only its own list.
        let apart = [("a".to_string(), file("A", &entry("from-a", "T", None))), ("c".to_string(), file("C", &entry("from-c", "T", None)))];
        assert!(clashes(&apart).is_empty());
    }

    /// R-F2: the Farmlands entered before Freythorn: back in the Hunting Grounds, Crowbell, the
    /// ritual and "Enter Freythorn" are still listed (nothing implies a zone entry).
    #[test]
    fn an_unfinished_visit_stays_listed_with_a_later_one() {
        let mut inner = playing();
        planned_up_to(&mut inner, "G1_11", 0);
        shows(&mut inner, "G1_11", &HUNTING_FIRST, &HUNTING_FIRST_ALL);
        play(&mut inner, &[enter("G1_13_1")]);
        assert!(!inner.progress.completed.contains_key("hunting-grounds-to-freythorn"));
        let mut back: Vec<&str> = HUNTING_FIRST.to_vec();
        back.push("enter-ogham-farmlands");
        shows(&mut inner, "G1_11", &back, &[]);
        assert_eq!(inner.view().steps.iter().find(|s| s.id == "kill-crowbell").unwrap().state, "manual", "open");
        // Then Freythorn and back: both visits finished, the last is listed.
        play(&mut inner, &[enter("G1_12"), enter("G1_11")]);
        assert_eq!(rows(&inner, false), ["kill-crowbell", "enter-ogham-farmlands"], "Crowbell lives: rule 6");
    }

    /// R-F2: the caravan's list out of order (Keth before the Mastodon Badlands): no required
    /// row is lost; the visit for the Valley of the Titans lists the skipped one's rows too.
    #[test]
    fn the_caravan_out_of_order_loses_no_required_row() {
        let mut inner = playing();
        let caravan = inner.routes.iter().find(|r| r.zone.id == "G2_town").unwrap().clone();
        for area in ["G2_1", "G2_town", "G2_3a", "G2_town", "G2_10_1", "G2_town", "G2_2", "G2_town", "G2_4_1", "G2_town"] {
            play(&mut inner, &[enter(area)]);
        }
        assert_eq!(inner.progress.completed["travel-keth"].source, "log");
        assert!(!inner.progress.completed.contains_key("travel-mastodon-badlands"));
        let listed: BTreeSet<String> = rows(&inner, false).into_iter().collect();
        let visits = caravan.visits();
        let shown = inner.view().visit;
        for (step, visit) in caravan.steps.iter().zip(visits) {
            let required = step.kind == "objective" && !step.optional;
            if required && visit < shown && !inner.progress.completed.contains_key(&step.id) {
                assert!(listed.contains(&step.id), "{} (visit {visit}) lost; listed {listed:?}", step.id);
            }
        }
        assert!(listed.contains("travel-mastodon-badlands") && listed.contains("travel-valley-titans"), "{listed:?}");
    }

    /// R-F6: a start in the middle of an act. First seen in the Red Vale (where from is not
    /// known), then back in the Grelwood: the seals visit, not the first visit's rows. And
    /// the Red Vale reached by waypoint from town, then the Grelwood on foot: coming in from
    /// the Red Vale proves the Grelwood's way there was taken.
    #[test]
    fn coming_back_from_a_zone_proves_its_exit() {
        let seals = ["kill-areagne", "runic-seals", "grelwood-una-seals", "grelwood-portal-town"]; // Areagne lives: rule 6
        let mut inner = playing();
        play(&mut inner, &[enter("G1_5")]);
        assert_eq!(inner.progress.completed["enter-red-vale"].source, "implied", "where from is not known");
        shows(&mut inner, "G1_4", &seals, &[]);
        let mut inner = playing();
        play(&mut inner, &[enter("G1_town"), enter("G1_5")]);
        assert!(!inner.progress.completed.contains_key("enter-red-vale"), "not from the Grelwood");
        shows(&mut inner, "G1_4", &seals, &[]);
        assert_eq!(inner.progress.completed["enter-red-vale"].evidence, "Came back from G1_5");
        // Seen leaving for the Red Vale and walking back (Areagne skipped): the first visit.
        let mut inner = playing();
        play(&mut inner, &[enter("G1_2"), enter("G1_4"), enter("G1_5")]);
        shows(&mut inner, "G1_4", &GRELWOOD_FIRST, &GRELWOOD_FIRST_ALL);
    }

    /// R-F3: Lachlann the Lost (the man at the gate) speaking starts no fight, so 30 quiet
    /// seconds leave the kill open; Lachlann of Endless Lament (the boss) starts it.
    #[test]
    fn a_namesake_starts_no_fight() {
        let mut inner = playing();
        play(&mut inner, &[enter("G1_7")]);
        let Inner { routes, progress, fight, .. } = &mut inner;
        hear_boss(routes, progress, fight, "Lachlann the Lost", "At last! It's open! Praise the First Ones!");
        assert!(fight.is_none());
        assert!(!inner.settle_fight());
        assert!(!inner.progress.completed.contains_key("kill-lachlann"));
        let Inner { routes, progress, fight, .. } = &mut inner;
        hear_boss(routes, progress, fight, "Lachlann of Endless Lament", "You will not have her!");
        assert_eq!(fight.as_ref().unwrap().step, "kill-lachlann");
        // A boss whose kill line someone else says (Mortimer) still starts by name.
        assert_eq!(boss_step(&inner.routes, &inner.progress, "G1_1", "The Bloated Miller").map(|s| s.id.as_str()), Some("kill-bloated-miller"));
        assert!(boss_step(&inner.routes, &inner.progress, "G1_1", "Mortimer").is_none());
    }

    /// R-F7: the quest tracker's proof turns a "probably done" kill into a proven one; a kill
    /// confirmed by hand stays as it is.
    #[test]
    fn a_screen_proof_upgrades_a_likely_kill() {
        let routes = load_routes();
        let mut progress = Progress::default();
        let likely = Completion { at: "now".into(), source: LIKELY.into(), evidence: "quiet".into() };
        progress.completed.insert("kill-bloated-miller".into(), likely);
        assert!(apply_screen(&routes, &mut progress, "TheMiller", &BTreeSet::from([3]), "tracker"));
        assert_eq!(progress.completed["kill-bloated-miller"].source, "screen");
        let mut progress = Progress::default();
        done(&mut progress, "kill-bloated-miller");
        apply_screen(&routes, &mut progress, "TheMiller", &BTreeSet::from([3]), "tracker");
        assert_eq!(progress.completed["kill-bloated-miller"].source, "manual");
    }

    /// R-F8: two bosses of one zone speaking in turn (Areagne, then Gerung) keep their own
    /// timers: neither's pending state is lost; quiet, both are probably done; the kill of one
    /// leaves the other's fight going. A death or a zone change clears the fight, and these.
    #[test]
    fn two_bosses_keep_their_own_timers() {
        let mut inner = playing();
        play(&mut inner, &[enter("G1_4")]);
        let hear = |inner: &mut Inner, speaker: &str| {
            let Inner { routes, progress, fight, .. } = inner;
            hear_boss(routes, progress, fight, speaker, "...");
        };
        let ago = |inner: &mut Inner, secs: u64| {
            let fight = inner.fight.as_mut().unwrap();
            let at = std::time::Instant::now() - Duration::from_secs(secs);
            fight.last_line = at;
            for (_, last) in &mut fight.earlier {
                *last = at;
            }
        };
        hear(&mut inner, "Areagne");
        hear(&mut inner, "Gerung, the Brambleghast");
        let fight = inner.fight.as_ref().unwrap();
        assert_eq!((fight.step.as_str(), fight.earlier.len()), ("kill-gerung", 1));
        hear(&mut inner, "Areagne");
        let fight = inner.fight.as_ref().unwrap();
        assert_eq!(fight.step, "kill-areagne");
        assert_eq!(fight.earlier.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["kill-gerung"]);
        ago(&mut inner, 5);
        assert!(!inner.settle_fight());
        ago(&mut inner, FIGHT_QUIET.as_secs() + 1);
        assert!(inner.settle_fight());
        for id in ["kill-areagne", "kill-gerung"] {
            assert_eq!(inner.progress.completed[id].source, LIKELY, "{id}");
        }
        // Gerung's kill confirmed while Areagne spoke a moment ago: her fight goes on.
        let mut inner = playing();
        play(&mut inner, &[enter("G1_4")]);
        hear(&mut inner, "Areagne");
        hear(&mut inner, "Gerung, the Brambleghast");
        toggle(&inner.routes.clone(), &mut inner.progress, "kill-gerung").unwrap();
        assert!(!inner.settle_fight(), "not over");
        assert_eq!(inner.fight.as_ref().unwrap().step, "kill-areagne");
        assert!(!inner.progress.completed.contains_key("kill-areagne"));
        // Leaving the zone or dying ends the fight (`process`): the earlier boss with it.
        inner.fight = None;
        assert!(!inner.settle_fight());
        assert!(!inner.progress.completed.contains_key("kill-areagne"));
    }

    /// R-F4: one event ticks at most one row of a list, the last whose `after` holds. The
    /// caravan's early Trial waypoint skipped, the Spires done: the late Trial entry ticks
    /// "Waypoint to the Trial" and leaves the waypoint grab open.
    #[test]
    fn one_entry_ticks_one_row_of_a_list() {
        let mut inner = playing();
        play(&mut inner, &[enter("G2_9_1"), enter("G2_9_2"), enter("G2_town"), enter("G2_13")]);
        assert_eq!(inner.progress.completed["enter-spires-of-deshar"].source, "log");
        assert_eq!(inner.progress.completed["caravan-to-trial"].source, "log");
        assert!(!inner.progress.completed.contains_key("travel-trial-waypoint"));
        // The early entry, before the Spires, ticks the waypoint grab only.
        let mut inner = playing();
        play(&mut inner, &[enter("G2_town"), enter("G2_13")]);
        assert!(inner.progress.completed.contains_key("travel-trial-waypoint"));
        assert!(!inner.progress.completed.contains_key("caravan-to-trial"));
    }

    /// R-F1: Freythorn left on foot (no town trip), on through the Hunting Grounds, the
    /// Farmlands and Ogham Village, then a portal to town: only the Village's portal row
    /// ticks, Freythorn's rows stay as they were.
    #[test]
    fn a_zone_entry_ticks_only_the_list_of_the_zone_left() {
        let mut inner = playing();
        planned_up_to(&mut inner, "G1_12", 0);
        play(&mut inner, &[enter("G1_12")]);
        let freythorn = |inner: &Inner| {
            ["ritual-altars", "kill-king-in-the-mists", "freythorn-portal-town"].map(|id| inner.progress.completed.contains_key(id))
        };
        let before = freythorn(&inner);
        assert_eq!(before, [false, false, false]);
        play(&mut inner, &[enter("G1_11"), enter("G1_13_1"), enter("G1_13_2"), enter("G1_town")]);
        assert_eq!(inner.progress.completed["village-portal-town"].source, "log");
        assert_eq!(freythorn(&inner), before, "Freythorn's rows as they were");
        assert!(!inner.progress.completed.contains_key("town-to-hunting-grounds"));
        // The town lists the visit the player came for, with Finn's charm carried.
        assert_eq!(rows(&inner, false), ["finn-charm", "deliver-unas-lute", "leitis-gem", "town-to-ogham-village"]);
    }

    #[test]
    fn exit_names_are_listed_once() {
        // The Cemetery has two exits each to the Mausoleum and the Tomb.
        let names = exit_names("G1_7");
        assert!(names.contains(&"Mausoleum of the Praetor".to_string()), "{names:?}");
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "{names:?}");
        assert!(exit_names("no such area").is_empty());
    }

    #[test]
    fn unticked_steps_stay_unticked() {
        let routes = load_routes();
        let mut progress = Progress::default();
        feed(&mut progress, &[line("DEBUG", r#"Generating level 15 area "G1_town" with seed 1"#)]);
        toggle(&routes, &mut progress, "enter-clearfell-encampment").unwrap();
        feed(&mut progress, &[line("DEBUG", r#"Generating level 15 area "G1_town" with seed 2"#)]);
        assert!(!progress.completed.contains_key("enter-clearfell-encampment"));
        toggle(&routes, &mut progress, "enter-clearfell-encampment").unwrap();
        assert_eq!(progress.completed["enter-clearfell-encampment"].source, "manual");
        assert!(toggle(&routes, &mut progress, "directions-aggorat").is_err(), "a note is no step to tick");
    }

    #[test]
    fn first_run_replays_from_latest_game_start() {
        let dir = std::env::temp_dir().join(format!("batonga-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Client.txt");
        let old = line("INFO", "[STARTUP] Loading in 0.01 seconds") + "\n" + &line("INFO", "Old: line") + "\n";
        let new = line("INFO", "[STARTUP] Loading in 0.02 seconds") + "\n" + &line("INFO", "New: line") + "\n";
        std::fs::write(&path, format!("{old}{new}")).unwrap();
        let mut file = File::open(&path).unwrap();
        let len = file.metadata().unwrap().len();
        assert_eq!(first_run_offset(&mut file, len).unwrap(), old.len() as u64);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn done(progress: &mut Progress, id: &str) {
        progress.completed.insert(id.into(), Completion { at: "now".into(), source: "manual".into(), evidence: String::new() });
    }

    #[test]
    fn a_logged_step_takes_the_unseen_steps_before_it_as_done() {
        let routes = one_route(
            r#"{"id":"talk","kind":"objective","parts":[{"text":"Talk to Una"}]},
               {"id":"note","kind":"note","parts":[]},
               {"id":"side","kind":"objective","optional":true,"parts":[]},
               {"id":"boss","kind":"objective","parts":[{"text":"Kill "},{"text":"Beira","style":"boss"}]},
               {"id":"gate","kind":"objective","navigationTarget":{"area":"A","label":"Gate","tag":"gate"},"parts":[]},
               {"id":"undone","kind":"objective","parts":[]},
               {"id":"leave","kind":"objective","parts":[],"trigger":{"type":"zoneEntered","area":"B"}},
               {"id":"later","kind":"objective","parts":[]}"#,
        );
        let mut progress = Progress::default();
        // Unticked by hand before: it stays unticked.
        progress.dismissed.insert("undone".into());
        enter_zone(&routes, &mut progress, "A");
        assert!(progress.completed.is_empty());
        enter_zone(&routes, &mut progress, "B");
        assert_eq!(progress.completed["leave"].source, "log");
        assert_eq!(progress.completed["talk"].source, "implied");
        assert_eq!(progress.completed["talk"].evidence, "Before leave");
        // Not a note, an optional step, a boss kill, a step the compass leads to, one
        // unticked by hand, or a step after the logged one.
        for id in ["note", "side", "boss", "gate", "undone", "later"] {
            assert!(!progress.completed.contains_key(id), "{id}");
        }
    }

    #[test]
    fn leaving_the_encampment_ticks_the_talk_with_renly() {
        let routes = load_routes();
        let mut progress = Progress::default();
        enter_zone(&routes, &mut progress, "G1_town");
        assert!(!progress.completed.contains_key("talk-renly"));
        enter_zone(&routes, &mut progress, "G1_2");
        assert_eq!(progress.completed["enter-clearfell"].source, "log");
        // Renly's gem is a town hand-in of the visit just left: taken with it.
        assert_eq!(progress.completed["talk-renly"].source, "implied");
        // A talk with nothing to collect is implied: Una's in the Grelwood by the Red Vale.
        enter_zone(&routes, &mut progress, "G1_4");
        enter_zone(&routes, &mut progress, "G1_5");
        assert_eq!(progress.completed["grelwood-talk-una"].source, "implied");
        // Later talks in the town's list stay open.
        assert!(!progress.completed.contains_key("renly-runed-spikes"));
        assert!(!progress.completed.contains_key("town-una-roots"));
    }

    #[test]
    fn a_boss_the_map_shows_dead_ticks_its_kill_step_in_its_zone() {
        let mut inner = Inner {
            routes: load_routes(),
            progress: Progress::default(),
            characters: Default::default(),
            identity: Default::default(),
            log: LogStatus { connected: false, path: None, message: String::new() },
            save_path: None,
            skip_optional: false,
            recent: Default::default(),
            fight: None,
            in_zone: true,
            saved_at: Default::default(),
        };
        let miller = ["The Bloated Miller".to_string()];
        assert!(!inner.tick_dead_bosses(&miller), "no zone yet");
        inner.progress.current_area = Some("G1_2".into());
        assert!(!inner.tick_dead_bosses(&miller), "not this zone's boss");
        inner.progress.current_area = Some("G1_1".into());
        assert!(inner.tick_dead_bosses(&miller));
        assert_eq!(inner.progress.completed["kill-bloated-miller"].source, "map");
        assert!(!inner.tick_dead_bosses(&miller), "ticked once");
        // Unticked by hand: the map does not tick it again.
        toggle(&inner.routes, &mut inner.progress, "kill-bloated-miller").unwrap();
        assert!(!inner.tick_dead_bosses(&miller));
    }

    #[test]
    fn a_list_is_split_into_visits_at_the_steps_that_leave_the_zone() {
        let routes = load_routes();
        let town = routes.iter().find(|r| r.zone.id == "G1_town").unwrap();
        let visit_of = |id: &str| town.visits()[town.steps.iter().position(|s| s.id == id).unwrap()];
        // First visit: Renly, then out to Clearfell. The spikes are for the second.
        assert_eq!((visit_of("talk-renly"), visit_of("enter-clearfell")), (0, 0));
        assert_eq!((visit_of("renly-runed-spikes"), visit_of("town-to-grelwood")), (1, 1));
        assert_eq!(visit_of("town-una-roots"), 2);
        assert_eq!((visit_of("finn-charm"), visit_of("town-to-hunting-grounds")), (3, 3));
        assert_eq!((visit_of("deliver-unas-lute"), visit_of("leitis-gem"), visit_of("town-to-ogham-village")), (4, 4, 4));
        assert_eq!(visit_of("enter-vastiri-outskirts"), 5);
        let mut progress = Progress::default();
        assert_eq!(town.current_visit(&progress), 0);
        // The visit ends when its leaving step is done, whatever else is open. The next is
        // due only once the player comes from the Red Vale: until then nothing (one past the
        // last visit); Renly's gem was a hand-in of the visit left.
        enter_zone(&routes, &mut progress, "G1_town");
        enter_zone(&routes, &mut progress, "G1_2");
        assert_eq!(town.current_visit(&progress), 6);
        for area in ["G1_4", "G1_5", "G1_town"] {
            enter_zone(&routes, &mut progress, area);
        }
        assert_eq!(town.current_visit(&progress), 1);
        // A zone left once: one visit.
        let riverbank = routes.iter().find(|r| r.zone.id == "G1_1").unwrap();
        assert!(riverbank.visits().iter().all(|&v| v == 0));
        assert_eq!(riverbank.current_visit(&progress), 0);
        // The optional trip into the Mud Burrow does not end Clearfell's visit: the way
        // on to the Grelwood stays in the list (and for the compass).
        let clearfell = routes.iter().find(|r| r.zone.id == "G1_2").unwrap();
        assert!(clearfell.visits().iter().all(|&v| v == 0), "{:?}", clearfell.visits());
    }

    #[test]
    fn taking_back_a_click_leaves_the_step_open_for_the_log() {
        let routes = load_routes();
        let mut progress = Progress::default();
        // Ticked by a click, then unticked by a click: open again, not held back.
        toggle(&routes, &mut progress, "enter-clearfell").unwrap();
        toggle(&routes, &mut progress, "enter-clearfell").unwrap();
        assert!(progress.dismissed.is_empty());
        enter_zone(&routes, &mut progress, "G1_town");
        enter_zone(&routes, &mut progress, "G1_2");
        assert_eq!(progress.completed["enter-clearfell"].source, "log");
        // Unticking what the log ticked holds it back.
        toggle(&routes, &mut progress, "enter-clearfell").unwrap();
        assert!(progress.dismissed.contains("enter-clearfell"));
        enter_zone(&routes, &mut progress, "G1_town");
        enter_zone(&routes, &mut progress, "G1_2");
        assert!(!progress.completed.contains_key("enter-clearfell"));
        // A click ticks it again and lifts the hold.
        toggle(&routes, &mut progress, "enter-clearfell").unwrap();
        assert!(progress.dismissed.is_empty());
    }

    #[test]
    fn a_zone_boss_speaking_names_its_open_kill_step() {
        let routes = load_routes();
        let mut progress = Progress::default();
        let step = |progress: &Progress, area: &str, speaker: &str| {
            boss_step(&routes, progress, area, speaker).map(|s| s.id.clone())
        };
        assert_eq!(step(&progress, "G1_1", "The Bloated Miller").as_deref(), Some("kill-bloated-miller"));
        // The game's longer name holds the checklist's.
        assert_eq!(step(&progress, "G1_2", "Beira of the Rotten Pack").as_deref(), Some("kill-beira"));
        // Not a boss, not this zone's boss, and not once the boss is dead.
        assert_eq!(step(&progress, "G1_1", "Mortimer"), None);
        assert_eq!(step(&progress, "G1_2", "The Bloated Miller"), None);
        done(&mut progress, "kill-bloated-miller");
        assert_eq!(step(&progress, "G1_1", "The Bloated Miller"), None);
    }

    #[test]
    fn the_log_offset_alone_is_saved_every_half_minute() {
        let now = std::time::Instant::now();
        assert!(save_due(false, None, now), "never written");
        assert!(!save_due(false, Some(now - Duration::from_secs(5)), now), "only chat since");
        assert!(save_due(true, Some(now - Duration::from_secs(5)), now), "a step ticked");
        assert!(save_due(false, Some(now - SAVE_OFFSET_EVERY), now));
    }

    #[test]
    fn a_fight_ends_with_the_kill_or_a_quiet_boss() {
        let mut inner = Inner {
            routes: load_routes(),
            progress: Progress::default(),
            characters: Default::default(),
            identity: Default::default(),
            log: LogStatus { connected: false, path: None, message: String::new() },
            save_path: None,
            skip_optional: false,
            recent: Default::default(),
            fight: None,
            in_zone: true,
            saved_at: Default::default(),
        };
        let fight = |ago: Duration| Some(Fight { step: "kill-bloated-miller".into(), last_line: std::time::Instant::now() - ago, earlier: Vec::new() });
        assert!(!inner.settle_fight(), "no fight");
        inner.fight = fight(Duration::from_secs(5));
        assert!(!inner.settle_fight(), "the boss spoke five seconds ago");
        assert!(inner.fight.is_some());
        inner.fight = fight(FIGHT_QUIET + Duration::from_secs(1));
        assert!(inner.settle_fight(), "quiet for too long");
        assert!(inner.fight.is_none());
        assert_eq!(inner.progress.completed["kill-bloated-miller"].source, LIKELY, "a quiet boss is probably killed");
        inner.progress.completed.remove("kill-bloated-miller");
        inner.fight = fight(Duration::from_secs(1));
        done(&mut inner.progress, "kill-bloated-miller");
        assert!(inner.settle_fight(), "the kill is ticked");
    }

    #[test]
    fn the_log_names_the_character_on_a_level_up_or_a_death() {
        let at = |message: &str| format!("2026/11/13 17:20:01 1 2 [INFO Client 308] {message}");
        assert_eq!(character_in_line(&at(": ajhbdfjkahuecibnauebfkl (Ranger) is now level 6")).as_deref(), Some("ajhbdfjkahuecibnauebfkl"));
        assert_eq!(character_in_line(&at(": Hollow_Sue has been slain.")).as_deref(), Some("Hollow_Sue"));
        for not in [": AFK mode is now ON. Autoreply \"This player is AFK.\"", "#Someone: I am now level 3",
            "@From Someone: Hollow has been slain.", ": You have entered Clearfell.", "Ghostly Voice: This is our land. We stand united!",
            ": Two Words (Ranger) is now level 2"] {
            assert_eq!(character_in_line(&at(not)), None, "{not}");
        }
    }

    #[test]
    fn the_rust_kings_emerge_line_ticks_the_obelisks_and_starts_his_fight() {
        let routes = load_routes();
        let mut progress = Progress::default();
        feed(&mut progress, &[line("DEBUG", r#"Generating level 5 area "G1_5" with seed 2001406961"#)]);
        // As the owner's log has it (2026-11-13): spoken by "Ghostly Voice".
        let emerge = line("INFO", "Ghostly Voice: This is our land. We stand united!");
        feed(&mut progress, &[emerge]);
        assert!(progress.completed.contains_key("obelisks-of-rust"), "by its text, whoever the log names");
        let kill = boss_after_line(&routes, &progress, "G1_5", "This is our land. We stand united!").map(|s| s.id.as_str());
        assert_eq!(kill, Some("kill-rust-king"));
        assert!(boss_after_line(&routes, &progress, "G1_5", "Something else").is_none());
    }

    #[test]
    fn a_line_before_a_boss_starts_no_fight_unless_he_emerges_on_it() {
        // Live v0.1.115: the Wounded Man's last line ticks his talk, the step before the
        // Miller; taken as the Miller's emerge line it hid the overlay and, 30 s later,
        // ticked the kill while the player stood by the Wounded Man.
        let routes = load_routes();
        let mut progress = Progress::default();
        feed(&mut progress, &[line("DEBUG", r#"Generating level 1 area "G1_1" with seed 1"#)]);
        feed(&mut progress, &[line("INFO", "Wounded Man: Reach... Clearfell... Find the Miller...")]);
        assert!(progress.completed.contains_key("talk-wounded-man"));
        assert!(boss_after_line(&routes, &progress, "G1_1", "Reach... Clearfell... Find the Miller...").is_none());
        assert!(boss_step(&routes, &progress, "G1_1", "Wounded Man").is_none());
    }

    #[test]
    fn the_player_is_in_a_zone_once_the_last_game_start_led_into_one() {
        let start = line("INFO", "[STARTUP] Game Start");
        let zone = line("DEBUG", r#"Generating level 2 area "G1_2" with seed 5"#);
        assert!(zone_since_start(""), "nothing known: as before");
        assert!(zone_since_start(&zone));
        assert!(zone_since_start(&format!("{start}\n{zone}")));
        assert!(!zone_since_start(&start), "at the login screen");
        assert!(!zone_since_start(&format!("{zone}\n{start}")), "started again: the last zone is not where the player is");
        let login = line("INFO", "Async connecting to mil.login.pathofexile2.com:21270");
        assert!(!zone_since_start(&format!("{zone}\n{login}")), "returning to login discards the old zone");
        assert!(zone_since_start(&format!("{login}\n{zone}")));
        assert!(!session_boundary(&line("INFO", "#Player: [STARTUP] Game Start")), "chat is not a login");
    }

    #[test]
    fn idle_and_unknown_zones_never_fall_back_to_the_saved_checklist() {
        let mut inner = Inner {
            routes: load_routes(), progress: Progress::default(), characters: Default::default(),
            identity: Default::default(),
            log: LogStatus { connected: true, path: None, message: String::new() },
            save_path: None, skip_optional: false, recent: Default::default(), fight: None,
            in_zone: false, saved_at: Default::default(),
        };
        inner.progress.current_area = Some("G1_1".into());
        inner.progress.shown_zone = Some("G1_1".into());
        assert!(inner.view().zone.is_none());
        assert!(inner.view().steps.is_empty());
        inner.in_zone = true;
        assert_eq!(inner.view().zone.unwrap().id, "G1_1");
        inner.progress.current_area = Some("UnknownArea".into());
        assert!(inner.view().zone.is_none());
        assert!(inner.view().steps.is_empty());
    }

    #[test]
    fn startup_context_uses_only_the_latest_login() {
        let path = std::env::temp_dir().join(format!("batonga-session-test-{}-{}.txt", std::process::id(),
            SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()));
        let old_zone = line("DEBUG", r#"Generating level 1 area "G1_1" with seed 1"#);
        let login = line("INFO", "Async connecting to mil.login.pathofexile2.com:21270");
        let text = format!("{old_zone}\n{login}\n");
        std::fs::write(&path, &text).unwrap();
        let mut file = File::open(&path).unwrap();
        let (zone, active, session, _) = current_zone(&mut file, text.len() as u64).unwrap();
        assert!(zone.is_none(), "the prior character's zone is history");
        assert!(!active);
        assert_eq!(session.as_deref(), Some(login.as_str()));
        drop(file);
        let zone_line = line("DEBUG", r#"Generating level 2 area "G1_2" with seed 2"#);
        let text = format!("{text}{zone_line}\n");
        std::fs::write(&path, &text).unwrap();
        let mut file = File::open(&path).unwrap();
        let (zone, active, session, _) = current_zone(&mut file, text.len() as u64).unwrap();
        assert!(matches!(zone, Some(LogEvent::ZoneEntered { area, .. }) if area == "G1_2"));
        assert!(active);
        assert_eq!(session.as_deref(), Some(login.as_str()));
        drop(file);
        std::fs::remove_file(path).unwrap();
    }

    // ---------- Replays of Act 1 (R1 of 2026-11-13: the visit follows the player) ----------

    /// The tracker as the app has it while the player is in a zone, with a chosen character.
    fn playing() -> Inner {
        let mut inner = Inner {
            routes: load_routes(), progress: Progress::default(), characters: Default::default(),
            identity: Default::default(),
            log: LogStatus { connected: true, path: None, message: String::new() },
            save_path: None, skip_optional: false, recent: Default::default(), fight: None,
            in_zone: true, saved_at: Default::default(),
        };
        inner.characters.select(&mut inner.progress, "Hero", false).unwrap();
        inner
    }

    fn enter(area: &str) -> String {
        line("DEBUG", &format!(r#"Generating level 5 area "{area}" with seed 1"#))
    }

    fn npc(speaker: &str, text: &str) -> String {
        line("INFO", &format!("{speaker}: {text}"))
    }

    /// Log lines as `process` applies them.
    fn play(inner: &mut Inner, lines: &[String]) {
        for l in lines {
            if let Some((ts, event)) = parse_line(l) {
                apply(&inner.routes, &mut inner.progress, &ts, &event);
            }
        }
    }

    /// The rows the panel lists: the steps of the visit on show; optional ones only with
    /// "Optional content" on.
    fn rows(inner: &Inner, optional: bool) -> Vec<String> {
        let view = inner.view();
        view.steps.iter().filter(|s| s.visit == view.visit && (optional || !s.optional)).map(|s| s.id.clone()).collect()
    }

    /// Enters `area` and checks the rows listed with "Optional content" off and on.
    fn shows(inner: &mut Inner, area: &str, off: &[&str], on: &[&str]) {
        play(inner, &[enter(area)]);
        assert_eq!(rows(inner, false), off, "{area}, optional content off");
        assert_eq!(rows(inner, true), if on.is_empty() { off } else { on }, "{area}, optional content on");
    }

    /// A restart of the app: the progress file written and read back, the profile kept.
    fn restart(inner: &Inner) -> Inner {
        let mut characters = inner.characters.clone();
        characters.remember(&inner.progress);
        let saved = crate::characters::SavedProgress { progress: inner.progress.clone(), characters };
        let saved = crate::characters::load(&serde_json::to_vec(&saved).unwrap()).unwrap();
        let (mut progress, mut characters) = (saved.progress, saved.characters);
        characters.clear_binding(&mut progress);
        characters.keep_chosen(&mut progress);
        let mut next = playing();
        next.progress = progress;
        next.characters = characters;
        next
    }

    const CEMETERY: [&str; 7] =
        ["talk-lachlann", "ancient-ruins", "enter-mausoleum", "enter-tomb", "memorial-gate", "kill-lachlann", "enter-hunting-grounds"];
    const GRELWOOD_FIRST: [&str; 5] = ["grelwood-talk-una", "grelwood-waypoint", "kill-areagne", "grelwood-grim-tangle-wp", "enter-red-vale"];
    const GRELWOOD_FIRST_ALL: [&str; 6] =
        ["grelwood-talk-una", "grelwood-waypoint", "kill-areagne", "grelwood-grim-tangle-wp", "kill-gerung", "enter-red-vale"];
    /// The Grelwood's seals visit in a replay that never kills Areagne: she comes along (rule 6).
    const SEALS: [&str; 4] = ["kill-areagne", "runic-seals", "grelwood-una-seals", "grelwood-portal-town"];
    const HUNTING_FIRST: [&str; 4] = ["hunting-grounds-waypoint", "kill-crowbell", "small-ritual", "hunting-grounds-to-freythorn"];
    const HUNTING_FIRST_ALL: [&str; 5] =
        ["hunting-grounds-waypoint", "talk-delwyn", "kill-crowbell", "small-ritual", "hunting-grounds-to-freythorn"];

    /// The zone entries of Act 1 in the planned order (contract 3).
    const PLANNED: [&str; 27] = [
        "G1_1", "G1_town", "G1_2", "G1_4", "G1_6", "G1_4", "G1_5", "G1_town", "G1_4", "G1_town", "G1_6", "G1_7", "G1_8",
        "G1_7", "G1_9", "G1_7", "G1_11", "G1_12", "G1_town", "G1_11", "G1_13_1", "G1_13_2", "G1_town", "G1_13_2", "G1_14",
        "G1_15", "G1_town",
    ];

    /// The planned entries before the first entry of `area` from `from` on.
    fn planned_up_to(inner: &mut Inner, area: &str, from: usize) {
        let stop = from + PLANNED[from..].iter().position(|a| *a == area).unwrap();
        play(inner, &PLANNED[..stop].iter().map(|a| enter(a)).collect::<Vec<_>>());
    }

    /// The start of Act 1, as planned, up to the first Grelwood entry.
    fn to_the_grelwood(inner: &mut Inner) {
        shows(inner, "G1_1", &["talk-wounded-man", "kill-bloated-miller", "enter-clearfell-encampment"], &[]);
        play(inner, &[npc("Wounded Man", "Reach... Clearfell... Find the Miller..."), npc("Mortimer", "Well done! Please come inside.")]);
        shows(inner, "G1_town", &["talk-renly", "enter-clearfell"], &[]);
        shows(inner, "G1_2", &["abandoned-stash", "kill-beira", "enter-grelwood"], &["abandoned-stash", "kill-beira", "enter-mud-burrow", "enter-grelwood"]);
        shows(inner, "G1_4", &GRELWOOD_FIRST, &GRELWOOD_FIRST_ALL);
    }

    /// S1: Act 1 in the planned order (contract 3): each zone entry lists its visit's rows.
    /// S3: the Grim Tangle's waypoint is grabbed before Areagne: she is still listed, open.
    /// S5: the Grim Tangle's first entry lists the waypoint grab only, the later one Ervig.
    #[test]
    fn act_one_in_the_planned_order_lists_each_visit() {
        let mut inner = playing();
        to_the_grelwood(&mut inner);
        shows(&mut inner, "G1_6", &["grim-tangle-waypoint"], &[]);
        shows(&mut inner, "G1_4", &GRELWOOD_FIRST, &GRELWOOD_FIRST_ALL);
        assert!(!inner.progress.completed.contains_key("kill-areagne"), "S3: Areagne is still to do");
        assert!(inner.progress.completed.contains_key("grelwood-grim-tangle-wp"));
        shows(&mut inner, "G1_5", &["obelisks-of-rust", "kill-rust-king", "red-vale-portal-town"], &[]);
        play(&mut inner, &[npc("Ghostly Voice", "This is our land. We stand united!")]);
        shows(&mut inner, "G1_town", &["renly-runed-spikes", "town-to-grelwood"], &["renly-support-gem", "renly-runed-spikes", "town-to-grelwood"]);
        shows(&mut inner, "G1_4", &SEALS, &[]);
        shows(&mut inner, "G1_town", &["town-una-roots", "town-to-grim-tangle"], &[]);
        shows(&mut inner, "G1_6", &["kill-ervig", "enter-cemetery"], &[]);
        shows(&mut inner, "G1_7", &CEMETERY, &[]);
        shows(&mut inner, "G1_8", &["kill-draven", "mausoleum-back"], &["kill-draven", "forgotten-riches", "mausoleum-back"]);
        shows(&mut inner, "G1_7", &CEMETERY, &[]);
        shows(&mut inner, "G1_9", &["haunted-treasure", "kill-asinia", "tomb-back"], &[]);
        shows(&mut inner, "G1_7", &CEMETERY, &[]);
        play(&mut inner, &[
            npc("Lachlann the Lost", "At last! It's open! Praise the First Ones!"),
            npc("Lachlann of Endless Lament", "Together... at last..."),
        ]);
        shows(&mut inner, "G1_11", &HUNTING_FIRST, &HUNTING_FIRST_ALL);
        shows(&mut inner, "G1_12", &["ritual-altars", "kill-king-in-the-mists", "freythorn-portal-town"], &[]);
        play(&mut inner, &[npc("The King in the Mists", "This is not the end...")]);
        shows(&mut inner, "G1_town", &["finn-charm", "town-to-hunting-grounds"], &[]);
        shows(&mut inner, "G1_11", &["kill-crowbell", "enter-ogham-farmlands"], &[]);
        shows(&mut inner, "G1_13_1", &["unas-lute", "crop-circle", "enter-ogham-village"], &[]);
        shows(&mut inner, "G1_13_2", &["smithing-tools", "blacksmiths-chest", "kill-executioner", "lever-leitis", "village-portal-town"], &[]);
        shows(&mut inner, "G1_town", &["deliver-unas-lute", "leitis-gem", "town-to-ogham-village"], &[]);
        shows(&mut inner, "G1_13_2", &["enter-manor-ramparts"], &[]);
        shows(&mut inner, "G1_14", &["gallows-secret", "enter-ogham-manor"], &[]);
        shows(&mut inner, "G1_15", &["kill-candlemass", "kill-geonor", "manor-portal-town"], &[]);
        play(&mut inner, &[npc("The Hooded One", "We must make haste. Let us return to Clearfell to make our preparations.")]);
        shows(&mut inner, "G1_town", &["town-beasts-trail", "enter-vastiri-outskirts"], &[]);
        play(&mut inner, &[enter("G2_1")]);
        assert_eq!(inner.progress.completed["enter-vastiri-outskirts"].source, "log");
        // Every leaving step of Act 1 was logged on the way.
        for id in ["town-to-grelwood", "grelwood-portal-town", "town-to-grim-tangle", "grim-tangle-waypoint", "freythorn-portal-town",
            "town-to-hunting-grounds", "village-portal-town", "town-to-ogham-village", "manor-portal-town"] {
            assert_eq!(inner.progress.completed[id].source, "log", "{id}");
        }
    }

    /// S2: the Red Vale before the Grim Tangle's waypoint. Back from the Red Vale the Grelwood
    /// lists the seals; the waypoint grab is listed while the roots still block the way (until
    /// the town trip after the seals), and a first entry after that trip is the real visit.
    #[test]
    fn the_red_vale_before_the_grim_tangle_waypoint() {
        for grab_with_the_seals in [false, true] {
            let mut inner = playing();
            to_the_grelwood(&mut inner);
            shows(&mut inner, "G1_5", &["obelisks-of-rust", "kill-rust-king", "red-vale-portal-town"], &[]);
            shows(&mut inner, "G1_town", &["renly-runed-spikes", "town-to-grelwood"], &["renly-support-gem", "renly-runed-spikes", "town-to-grelwood"]);
            shows(&mut inner, "G1_4", &SEALS, &[]);
            if grab_with_the_seals {
                shows(&mut inner, "G1_6", &["grim-tangle-waypoint"], &[]);
                assert!(!inner.progress.completed.contains_key("town-to-grim-tangle"), "a waypoint grab is not the town's step");
                shows(&mut inner, "G1_4", &SEALS, &[]);
            }
            shows(&mut inner, "G1_town", &["town-una-roots", "town-to-grim-tangle"], &[]);
            if !grab_with_the_seals {
                // No waypoint there yet: by the Grelwood, on foot (its visits are all done).
                shows(&mut inner, "G1_4", &SEALS, &[]);
            }
            shows(&mut inner, "G1_6", &["kill-ervig", "enter-cemetery"], &[]);
            assert!(inner.view().steps.iter().all(|s| s.id != "grim-tangle-waypoint"), "not listed at all");
            // Taken from town by waypoint, or on foot through the Grelwood.
            let from = if grab_with_the_seals { "(level 5)" } else { "from G1_4" };
            assert!(inner.progress.completed["town-to-grim-tangle"].evidence.contains(from));
            assert!(!inner.progress.completed.contains_key("grim-tangle-waypoint") || grab_with_the_seals, "never implied");
            shows(&mut inner, "G1_7", &CEMETERY, &[]);
        }
    }

    /// S4: a town trip from Clearfell or the Grelwood before the Red Vale lists no rows of a
    /// later town visit: nothing (the route plans no visit then; Renly's gem was a hand-in
    /// of the visit left); the zones left for it list their own visit again on return.
    #[test]
    fn an_early_town_trip_lists_nothing_stale() {
        let mut inner = playing();
        shows(&mut inner, "G1_town", &["talk-renly", "enter-clearfell"], &[]);
        shows(&mut inner, "G1_2", &["abandoned-stash", "kill-beira", "enter-grelwood"], &["abandoned-stash", "kill-beira", "enter-mud-burrow", "enter-grelwood"]);
        shows(&mut inner, "G1_town", &[], &[]);
        assert_eq!(inner.progress.completed["talk-renly"].source, "implied", "a hand-in of the visit left");
        assert!(!inner.view().steps.is_empty(), "the zone's later visits are still sent (errands key on them)");
        shows(&mut inner, "G1_2", &["abandoned-stash", "kill-beira", "enter-grelwood"], &["abandoned-stash", "kill-beira", "enter-mud-burrow", "enter-grelwood"]);
        shows(&mut inner, "G1_4", &GRELWOOD_FIRST, &GRELWOOD_FIRST_ALL);
        shows(&mut inner, "G1_town", &[], &[]);
        shows(&mut inner, "G1_4", &GRELWOOD_FIRST, &GRELWOOD_FIRST_ALL);
        assert!(!inner.progress.completed.contains_key("town-to-grelwood"));
        shows(&mut inner, "G1_5", &["obelisks-of-rust", "kill-rust-king", "red-vale-portal-town"], &[]);
        shows(&mut inner, "G1_town", &["renly-runed-spikes", "town-to-grelwood"], &["renly-support-gem", "renly-runed-spikes", "town-to-grelwood"]);
    }

    /// S6: the Mausoleum and the Tomb in either order: the Cemetery is one visit, the same rows
    /// each time, and each side zone lists its own.
    #[test]
    fn the_mausoleum_and_the_tomb_in_either_order() {
        for (first, second) in [("G1_9", "G1_8"), ("G1_8", "G1_9")] {
            let mut inner = playing();
            planned_up_to(&mut inner, "G1_7", 0);
            shows(&mut inner, "G1_7", &CEMETERY, &[]);
            for side in [first, second] {
                if side == "G1_8" {
                    shows(&mut inner, side, &["kill-draven", "mausoleum-back"], &["kill-draven", "forgotten-riches", "mausoleum-back"]);
                } else {
                    shows(&mut inner, side, &["haunted-treasure", "kill-asinia", "tomb-back"], &[]);
                }
                shows(&mut inner, "G1_7", &CEMETERY, &[]);
                assert_eq!(inner.view().visit, 0);
            }
            assert!(!inner.progress.completed.contains_key("memorial-gate"));
        }
    }

    /// S7: Lachlann's ring left on the ground: the Hunting Grounds list Crowbell, and a town
    /// trip before Freythorn lists no town rows.
    #[test]
    fn the_ring_left_lists_no_town_trip() {
        let mut inner = playing();
        planned_up_to(&mut inner, "G1_8", 0);
        play(&mut inner, &[enter("G1_8"), enter("G1_7"), enter("G1_9")]);
        shows(&mut inner, "G1_7", &CEMETERY, &[]);
        play(&mut inner, &[npc("Lachlann of Endless Lament", "Together... at last...")]);
        shows(&mut inner, "G1_11", &HUNTING_FIRST, &HUNTING_FIRST_ALL);
        shows(&mut inner, "G1_town", &[], &[]);
        shows(&mut inner, "G1_11", &HUNTING_FIRST, &HUNTING_FIRST_ALL);
        shows(&mut inner, "G1_12", &["ritual-altars", "kill-king-in-the-mists", "freythorn-portal-town"], &[]);
    }

    /// Skipped town trips: Freythorn left on foot, the town first reached from Ogham Village.
    /// The town lists the visit the player came for (the lute, Leitis), with Finn's charm
    /// from the skipped visit; its waypoint back to the Hunting Grounds is not carried.
    #[test]
    fn a_skipped_town_visit_carries_its_open_rows() {
        let mut inner = playing();
        planned_up_to(&mut inner, "G1_12", 0);
        play(&mut inner, &[enter("G1_12")]);
        shows(&mut inner, "G1_11", &["kill-crowbell", "enter-ogham-farmlands"], &[]);
        play(&mut inner, &[enter("G1_13_1"), enter("G1_13_2")]);
        shows(&mut inner, "G1_town", &["finn-charm", "deliver-unas-lute", "leitis-gem", "town-to-ogham-village"], &[]);
        assert_eq!(inner.view().visit, 4);
        shows(&mut inner, "G1_13_2", &["enter-manor-ramparts"], &[]);
        // Back in a zone whose visit is over, with a required row still open: Areagne.
        let mut inner = playing();
        play(&mut inner, &[enter("G1_4"), enter("G1_5")]);
        shows(&mut inner, "G1_4", &GRELWOOD_FIRST, &GRELWOOD_FIRST_ALL);
        for id in ["kill-areagne", "grelwood-waypoint"] {
            toggle(&inner.routes.clone(), &mut inner.progress, id).unwrap();
        }
        assert_eq!(rows(&inner, false), Vec::<String>::new(), "nothing left to do there now");
    }

    /// S8: a restart mid-zone lists the same rows (the saved progress decides, not the order
    /// the events were read in).
    #[test]
    fn a_restart_mid_zone_lists_the_same_rows() {
        let mut inner = playing();
        to_the_grelwood(&mut inner);
        let mut states = Vec::new();
        for area in ["G1_6", "G1_4", "G1_town", "G1_4", "G1_5", "G1_town", "G1_7", "G1_9", "G1_7", "G1_11"] {
            play(&mut inner, &[enter(area)]);
            let again = restart(&inner);
            assert_eq!(again.progress.current_area.as_deref(), Some(area));
            for optional in [false, true] {
                assert_eq!(rows(&again, optional), rows(&inner, optional), "{area}");
            }
            assert_eq!(again.view().visit, inner.view().visit, "{area}");
            states.push(rows(&inner, false));
        }
        assert!(states.contains(&Vec::new()), "an early town trip, with nothing listed, is among them");
    }

    /// R3: the character's level from its level-up lines, kept in its own profile; the zone's
    /// level from the zone line.
    #[test]
    fn the_level_comes_from_the_log_per_character() {
        let at = |message: &str| format!("2026/11/13 17:20:01 1 2 [INFO Client 308] {message}");
        assert_eq!(level_up_in_line(&at(": Hollow_Sue (Ranger) is now level 6")), Some(("Hollow_Sue".into(), 6)));
        assert_eq!(level_up_in_line(&at(": Hollow_Sue has been slain.")), None);
        assert_eq!(level_up_in_line(&at("#Hollow_Sue: (Ranger) is now level 6")), None, "chat");
        let mut inner = playing();
        play(&mut inner, &[enter("G1_4")]);
        assert_eq!(inner.view().level, None, "no level-up seen yet");
        assert!(note_character(&mut inner, &at(": Hollow_Sue (Ranger) is now level 6")));
        assert_eq!(inner.characters.active.as_deref(), Some("Hollow_Sue"), "the log names who plays");
        assert_eq!(inner.view().level, Some(6));
        assert!(!note_character(&mut inner, &at(": Hollow_Sue has been slain.")), "a death says no level");
        assert_eq!(inner.view().level, Some(6));
        // It survives a restart.
        let mut again = restart(&inner);
        assert_eq!(again.characters.active.as_deref(), Some("Hollow_Sue"));
        assert_eq!(again.view().level, Some(6));
        // Another character's level-up is its own; the first keeps its level.
        assert!(note_character(&mut again, &at(": Other_One (Witch) is now level 2")));
        assert_eq!(again.view().level, Some(2));
        assert!(note_character(&mut again, &at(": Hollow_Sue (Ranger) is now level 7")));
        assert_eq!(again.view().level, Some(7));
        assert_eq!(again.characters.profiles["Other_One"].level, Some(2));
        // The zone's level: the log's zone line, else the route file's.
        assert_eq!(again.view().zone.unwrap().level, 5, "the test's zone line says 5, the file 4");
        play(&mut again, &[line("DEBUG", r#"Generating level 7 area "G1_4" with seed 9"#)]);
        assert_eq!(again.view().zone.unwrap().level, 7);
        again.progress.area_level = None;
        assert_eq!(again.view().zone.unwrap().level, 4);
    }

    /// R4: which rows the app can tick by itself, and the waypoint row implied by a later entry.
    #[test]
    fn tracked_rows_and_the_implied_waypoint() {
        let mut inner = playing();
        play(&mut inner, &[enter("G1_town"), enter("G1_2"), enter("G1_4")]);
        let view = inner.view();
        let tracked = |id: &str| view.steps.iter().find(|s| s.id == id).unwrap().tracked;
        assert!(tracked("enter-red-vale"), "a log trigger");
        assert!(!tracked("kill-areagne"), "a boss that never speaks: no fight is seen");
        let clearfell = inner.routes.iter().find(|r| r.zone.id == "G1_2").unwrap();
        assert!(super::tracked(clearfell.steps.iter().find(|s| s.id == "kill-beira").unwrap()), "a boss: her fight starts when she speaks");
        assert!(!tracked("grelwood-waypoint"), "a hand row");
        assert_eq!(tracked("grelwood-talk-una"), crate::quest_screen::READS, "a quest-tracker rule, read on Windows only");
        play(&mut inner, &[enter("G1_6")]);
        assert!(!inner.progress.completed.contains_key("kill-areagne"), "boss rows stay out");
        // R-F5: the Grim Tangle entered without the Grelwood's waypoint: it stays open (the
        // seals trip needs it).
        assert!(!inner.progress.completed.contains_key("grelwood-waypoint"));
    }

    /// R-F5: implied ticks are what a later step proves. Pickups and rows the compass leads to
    /// stay open when the player leaves without them: the campsite stash, the small ritual.
    #[test]
    fn skipped_pickups_stay_open() {
        let mut inner = playing();
        play(&mut inner, &[enter("G1_town"), enter("G1_2"), enter("G1_4")]);
        assert!(!inner.progress.completed.contains_key("abandoned-stash"));
        assert_eq!(inner.progress.completed["enter-grelwood"].source, "log");
        let mut inner = playing();
        planned_up_to(&mut inner, "G1_11", 0);
        play(&mut inner, &[enter("G1_11"), enter("G1_12")]);
        for id in ["hunting-grounds-waypoint", "small-ritual", "kill-crowbell"] {
            assert!(!inner.progress.completed.contains_key(id), "{id}");
        }
        assert_eq!(inner.progress.completed["hunting-grounds-to-freythorn"].source, "log");
    }

    /// Rule 6 (council round 2): a boss with a reward left alive in a finished visit stays
    /// listed on a return; a pickup the app cannot see does not, nor a talk.
    #[test]
    fn a_reward_boss_left_alive_stays_listed_on_a_return() {
        let mut inner = playing();
        planned_up_to(&mut inner, "G1_11", 0);
        // Into Freythorn without Crowbell or the ritual, to town, back by the waypoint.
        play(&mut inner, &[enter("G1_11"), enter("G1_12"), enter("G1_town"), enter("G1_11")]);
        assert_eq!(rows(&inner, false), ["kill-crowbell", "enter-ogham-farmlands"]);
        // Ticked by hand (or killed): it is gone from the return visit.
        toggle(&inner.routes.clone(), &mut inner.progress, "kill-crowbell").unwrap();
        assert_eq!(rows(&inner, false), ["enter-ogham-farmlands"]);
        // The Grelwood's seals visit brings Areagne along while she lives.
        let mut inner = playing();
        to_the_grelwood(&mut inner);
        play(&mut inner, &[enter("G1_5"), enter("G1_town"), enter("G1_4")]);
        assert_eq!(rows(&inner, false), ["kill-areagne", "runic-seals", "grelwood-una-seals", "grelwood-portal-town"]);
        toggle(&inner.routes.clone(), &mut inner.progress, "kill-areagne").unwrap();
        assert_eq!(rows(&inner, false), ["runic-seals", "grelwood-una-seals", "grelwood-portal-town"]);
    }

    /// Council round 2 (Opus's replays). Act 4: Kingsmarch reached from Journey's End is the
    /// planned trip for the spikes: the town's later "Waypoint to Journey's End" is not taken
    /// as done by coming from there, nor Journey's End's "Portal to town" by coming from town.
    #[test]
    fn a_town_entry_proves_no_row_of_the_town() {
        let mut inner = playing();
        play(&mut inner, &[enter("G4_town"), enter("G4_2_1"), enter("G4_2_2"), enter("G4_town")]);
        assert!(!inner.progress.completed.contains_key("kingsmarch-to-journeys-end"));
        assert!(rows(&inner, false).contains(&"verisium-spikes".to_string()), "{:?}", rows(&inner, false));
        play(&mut inner, &[enter("G4_2_2")]);
        assert_eq!(inner.progress.completed["kingsmarch-to-journeys-end"].source, "log");
        assert!(!inner.progress.completed.contains_key("journeys-end-portal-town"));
    }

    /// The Red Vale done, the walk back into the Grelwood (for Areagne) and its waypoint to
    /// town: the town still lists the spikes, the Grelwood then the seals, the Grim Tangle
    /// Ervig (`alsoFrom` on the Red Vale's portal row).
    #[test]
    fn the_red_vale_left_on_foot_still_leads_to_the_spikes() {
        let mut inner = playing();
        to_the_grelwood(&mut inner);
        play(&mut inner, &[enter("G1_5"), npc("Ghostly Voice", "This is our land. We stand united!"), enter("G1_4"), enter("G1_town")]);
        assert_eq!(inner.progress.completed["red-vale-portal-town"].source, "log");
        assert_eq!(rows(&inner, false), ["renly-runed-spikes", "town-to-grelwood"]);
        shows(&mut inner, "G1_4", &SEALS, &[]);
        shows(&mut inner, "G1_town", &["town-una-roots", "town-to-grim-tangle"], &[]);
        shows(&mut inner, "G1_6", &["kill-ervig", "enter-cemetery"], &[]);
        // Freythorn left on foot, town by the Hunting Grounds' waypoint: Finn's visit.
        let mut inner = playing();
        planned_up_to(&mut inner, "G1_11", 0);
        play(&mut inner, &[enter("G1_11"), enter("G1_12"), npc("The King in the Mists", "This is not the end..."), enter("G1_11"), enter("G1_town")]);
        assert_eq!(rows(&inner, false), ["finn-charm", "town-to-hunting-grounds"]);
    }

    /// A trip to town in the middle of a visit strikes none of its work: the seals, the
    /// altars and the lever stay open.
    #[test]
    fn a_town_trip_mid_visit_strikes_nothing() {
        let mut inner = playing();
        to_the_grelwood(&mut inner);
        play(&mut inner, &[enter("G1_5"), enter("G1_town"), enter("G1_4"), enter("G1_town")]);
        for id in ["runic-seals", "grelwood-una-seals"] {
            assert!(!inner.progress.completed.contains_key(id), "{id}");
        }
        let mut inner = playing();
        planned_up_to(&mut inner, "G1_12", 0);
        play(&mut inner, &[enter("G1_12"), enter("G1_town"), enter("G1_12")]);
        assert!(!inner.progress.completed.contains_key("ritual-altars"));
        assert!(rows(&inner, false).contains(&"ritual-altars".to_string()));
        let mut inner = playing();
        planned_up_to(&mut inner, "G1_13_2", 0);
        play(&mut inner, &[enter("G1_13_2"), enter("G1_town")]);
        assert!(!inner.progress.completed.contains_key("lever-leitis"));
    }

    /// A character first seen in the middle of an act (or after a reset): a list nothing was
    /// done in brings no rows of its earlier visits along.
    #[test]
    fn a_list_never_seen_brings_nothing_along() {
        let mut inner = playing();
        play(&mut inner, &[enter("G1_13_2"), enter("G1_town")]);
        assert_eq!(rows(&inner, false), ["deliver-unas-lute", "leitis-gem", "town-to-ogham-village"]);
    }

    /// The Windows quest tracker keeps being read while a kill is only "likely": its proof
    /// can still confirm it (council round 2).
    #[test]
    fn the_quest_tracker_is_still_read_for_a_likely_kill() {
        let mut inner = playing();
        play(&mut inner, &[enter("G1_5")]);
        let with_screen: Vec<String> = inner.routes.iter().flat_map(|r| &r.steps).filter(|s| !s.screen.is_empty()).map(|s| s.id.clone()).collect();
        assert!(with_screen.contains(&"kill-rust-king".to_string()));
        for id in &with_screen {
            let source = if id == "kill-rust-king" { LIKELY } else { "screen" };
            inner.progress.completed.insert(id.clone(), Completion { at: "now".into(), source: source.into(), evidence: String::new() });
        }
        assert!(inner.screen_watch().is_some(), "a likely kill is still open to the tracker");
        inner.progress.completed.get_mut("kill-rust-king").unwrap().source = "manual".into();
        assert!(inner.screen_watch().is_none());
    }

    /// R5 (decision 4 of 2026-11-13): a boss quiet for 30 s is "likely" killed; it speaking
    /// again reopens the row and the fight; a click confirms; a click on a confirmed row
    /// reopens it.
    #[test]
    fn a_quiet_boss_is_probably_done() {
        let mut inner = playing();
        play(&mut inner, &[enter("G1_2")]);
        let quiet = |inner: &mut Inner| {
            inner.fight.as_mut().unwrap().last_line = std::time::Instant::now() - FIGHT_QUIET - Duration::from_secs(1);
            assert!(inner.settle_fight());
        };
        let state = |inner: &Inner| {
            let view = inner.view();
            let row = view.steps.iter().find(|s| s.id == "kill-beira").unwrap().clone();
            (row.state, row.source)
        };
        // 1. Quiet for 30 s: ticked, but only as likely.
        let Inner { routes, progress, fight, .. } = &mut inner;
        assert!(!hear_boss(routes, progress, fight, "Beira of the Rotten Pack", "Sleep in ice, wake in snow."));
        assert_eq!(fight.as_ref().unwrap().step, "kill-beira");
        quiet(&mut inner);
        assert_eq!(state(&inner), ("done".into(), Some(LIKELY.into())));
        assert!(inner.fight.is_none());
        // 2. The same boss speaks again: the row opens, the fight resumes.
        let Inner { routes, progress, fight, .. } = &mut inner;
        assert!(hear_boss(routes, progress, fight, "Beira of the Rotten Pack", "Cold..."));
        assert_eq!(fight.as_ref().unwrap().step, "kill-beira");
        assert_eq!(state(&inner), ("manual".into(), None), "open (no kill line known for her)");
        quiet(&mut inner);
        // 3. A click confirms it: done, by hand.
        toggle(&inner.routes.clone(), &mut inner.progress, "kill-beira").unwrap();
        assert_eq!(state(&inner), ("done".into(), Some("manual".into())));
        assert!(!inner.progress.dismissed.contains("kill-beira"));
        let Inner { routes, progress, fight, .. } = &mut inner;
        assert!(!hear_boss(routes, progress, fight, "Beira of the Rotten Pack", "Cold..."), "a confirmed kill stays");
        assert!(fight.is_none());
        // 4. A click on the confirmed row opens it again, as before.
        toggle(&inner.routes.clone(), &mut inner.progress, "kill-beira").unwrap();
        assert!(!inner.progress.completed.contains_key("kill-beira"));
        assert!(inner.progress.dismissed.is_empty());
        // A kill line that comes after the quiet tick makes it a logged kill.
        let mut inner = playing();
        play(&mut inner, &[enter("G1_7")]);
        let Inner { routes, progress, fight, .. } = &mut inner;
        hear_boss(routes, progress, fight, "Lachlann of Endless Lament", "You will not have her!");
        quiet(&mut inner);
        assert_eq!(inner.progress.completed["kill-lachlann"].source, LIKELY);
        play(&mut inner, &[npc("Lachlann of Endless Lament", "Together... at last...")]);
        assert_eq!(inner.progress.completed["kill-lachlann"].source, "log");
    }

    #[test]
    fn the_players_death_line_is_told_from_chat() {
        assert!(is_death(&line("INFO", ": Somebody has been slain.")));
        assert!(!is_death(&line("INFO", "#Somebody: the boss has been slain.")));
        assert!(!is_death(&line("INFO", "Somebody: he has been slain.")));
        assert!(!is_death(&line("INFO", ": Somebody (Ranger) is now level 2")));
    }

    /// Writes what the panel is sent at each step of Act 1 in the planned order, one JSON
    /// object per line (`{"name", "view"}`), to the file `BATONGA_VIEWS_OUT` names, for
    /// rendering the panel in a browser from a real run (`previewTracker`, the browser
    /// preview in `src/main.ts`). The character is one level under each zone, so the level
    /// targets show where they bite. Run:
    /// `BATONGA_VIEWS_OUT=<file> cargo test dump_act_one_views -- --ignored`.
    #[test]
    #[ignore = "writes a file for a render check: set BATONGA_VIEWS_OUT"]
    fn dump_act_one_views() {
        let Ok(out) = std::env::var("BATONGA_VIEWS_OUT") else { return };
        let mut inner = playing();
        let mut lines = Vec::new();
        let mut dump = |inner: &Inner, name: String| {
            lines.push(serde_json::json!({ "name": name, "view": inner.view() }).to_string());
        };
        // (zone to enter, lines spoken there afterwards)
        let said: &[(&str, &[(&str, &str)])] = &[
            ("G1_1", &[("Wounded Man", "Reach... Clearfell... Find the Miller..."), ("Mortimer", "Well done! Please come inside.")]),
            ("G1_town", &[]), ("G1_2", &[]), ("G1_4", &[]), ("G1_6", &[]), ("G1_4", &[]),
            ("G1_5", &[("Ghostly Voice", "This is our land. We stand united!")]),
            ("G1_town", &[]), ("G1_4", &[]), ("G1_town", &[]), ("G1_6", &[]), ("G1_7", &[]), ("G1_8", &[]), ("G1_7", &[]),
            ("G1_9", &[]),
            ("G1_7", &[("Lachlann the Lost", "At last! It's open! Praise the First Ones!"), ("Lachlann of Endless Lament", "Together... at last...")]),
            ("G1_11", &[]),
            ("G1_12", &[("The King in the Mists", "This is not the end...")]),
            ("G1_town", &[]), ("G1_11", &[]), ("G1_13_1", &[]), ("G1_13_2", &[]), ("G1_town", &[]), ("G1_13_2", &[]), ("G1_14", &[]),
            ("G1_15", &[("The Hooded One", "We must make haste. Let us return to Clearfell to make our preparations.")]),
            ("G1_town", &[]),
        ];
        // The reward bosses the log does not name are killed on the way: ticked as a click would.
        let kills: &[(usize, &str)] = &[(4, "kill-areagne"), (17, "kill-crowbell")];
        for (n, (area, spoken)) in said.iter().enumerate() {
            let zone = inner.routes.iter().find(|r| r.zone.id == *area).map_or(1, |r| r.zone.level);
            let own = if *area == "G1_town" { inner.progress.level.unwrap_or(1) } else { zone.saturating_sub(1).max(1) };
            note_character(&mut inner, &line("INFO", &format!(": Hero (Ranger) is now level {own}")));
            play(&mut inner, &[line("DEBUG", &format!(r#"Generating level {zone} area "{area}" with seed 1"#))]);
            dump(&inner, format!("{:02}-{area}", n + 1));
            for (k, (speaker, text)) in spoken.iter().enumerate() {
                play(&mut inner, &[npc(speaker, text)]);
                dump(&inner, format!("{:02}-{area}-said{}", n + 1, k + 1));
            }
            for (_, id) in kills.iter().filter(|(at, _)| *at == n + 1) {
                toggle(&inner.routes.clone(), &mut inner.progress, id).unwrap();
                dump(&inner, format!("{:02}-{area}-killed", n + 1));
            }
        }
        std::fs::write(&out, lines.join("\n") + "\n").unwrap();
    }
}
