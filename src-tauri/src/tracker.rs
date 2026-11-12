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
                    (Some(speaker), Some(text), true) => vec![NpcLine { speaker, text, id: None }],
                    (None, None, false) => lines,
                    _ => return Err("npcLine needs either speaker and text, or lines".into()),
                };
                let lines: Vec<NpcLine> = lines
                    .into_iter()
                    .map(|l| NpcLine { speaker: shown_text(&l.speaker), text: shown_text(&l.text), id: l.id })
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
    #[serde(default)]
    pub trigger: Option<Trigger>,
    #[serde(default, rename = "navigationTarget")]
    pub navigation_target: Option<NavigationTarget>,
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
        .map(|(name, json)| serde_json::from_str(json).unwrap_or_else(|e| panic!("data/routes/{name}.json: {e}")))
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
    if speaker.is_empty() || speaker.contains(':') {
        return None;
    }
    Some((
        timestamp,
        LogEvent::Dialogue { speaker: speaker.to_string(), text: text.trim().to_string() },
    ))
}

// ---------- Progress ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Completion {
    pub at: String,
    /// "log" or "manual".
    pub source: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    #[serde(default)]
    pub current_area: Option<String>,
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
pub fn apply(routes: &[Route], progress: &mut Progress, timestamp: &str, event: &LogEvent) -> bool {
    let mut changed = false;
    if let LogEvent::ZoneEntered { area, .. } = event {
        if progress.current_area.as_deref() != Some(area) {
            progress.current_area = Some(area.clone());
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
    for step in routes.iter().flat_map(|r| &r.steps) {
        let Some(trigger) = &step.trigger else { continue };
        if progress.completed.contains_key(&step.id) || progress.dismissed.contains(&step.id) {
            continue;
        }
        let evidence = match (trigger, event) {
            (Trigger::ZoneEntered { area, after }, LogEvent::ZoneEntered { area: got, level, .. })
                if area == got && after.as_ref().is_none_or(|step| done_before.contains(step)) =>
            {
                format!("Entered {got} (level {level})")
            }
            (Trigger::NpcLine { area, lines, after }, LogEvent::Dialogue { speaker: who, text: said })
                if progress.current_area.as_deref() == Some(area.as_str())
                    && after.as_ref().is_none_or(|step| done_before.contains(step))
                    && heard.as_ref().is_some_and(|(s, t)| lines.iter().any(|l| &l.speaker == s && &l.text == t)) =>
            {
                format!("{who}: {said}")
            }
            _ => continue,
        };
        progress.completed.insert(
            step.id.clone(),
            Completion { at: timestamp.to_string(), source: "log".into(), evidence },
        );
        changed = true;
    }
    imply_earlier(routes, progress, timestamp, &done_before);
    changed
}

/// The log shows zone entries and a few spoken lines; a talk in a dialogue panel, a
/// pick-up or a waypoint leaves no line (measured 2026-11-12: talks with Renly and Una
/// in the Clearfell Encampment wrote nothing). The lists are in walking order, so when
/// the log ticks a step, the steps before it in its list that the log cannot see are
/// taken as done too (source "implied"). Not optional steps, not boss kills and not
/// steps the compass leads to: those stay open until they are logged or ticked by hand.
fn imply_earlier(routes: &[Route], progress: &mut Progress, timestamp: &str, done_before: &BTreeSet<String>) {
    for route in routes {
        let newly = |step: &Step| {
            !done_before.contains(&step.id) && progress.completed.get(&step.id).is_some_and(|c| c.source == "log")
        };
        let Some(last) = route.steps.iter().rposition(newly) else { continue };
        for step in &route.steps[..last] {
            let unseen = step.kind == "objective"
                && !step.optional
                && step.navigation_target.is_none()
                && !step.parts.iter().any(|p| p.style.as_deref() == Some("boss"));
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

/// Ticks or unticks a step by hand.
pub fn toggle(routes: &[Route], progress: &mut Progress, id: &str) -> Result<(), String> {
    let step = routes.iter().flat_map(|r| &r.steps).find(|s| s.id == id).ok_or("Unknown step")?;
    if step.kind != "objective" {
        return Err("Only objectives can be ticked".into());
    }
    if let Some(done) = progress.completed.remove(id) {
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
    source: Option<String>,
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

impl Route {
    /// Each step's visit to the zone, from 0. A zone's list covers every time the route
    /// passes through it (the town's: five times). A visit ends with the step that leaves
    /// the zone (a zone-entry trigger for another area); the steps after it belong to the
    /// next time the player is there.
    pub fn visits(&self) -> Vec<usize> {
        let mut visit = 0;
        self.steps
            .iter()
            .map(|step| {
                let this = visit;
                if matches!(&step.trigger, Some(Trigger::ZoneEntered { area, .. }) if *area != self.zone.id) {
                    visit += 1;
                }
                this
            })
            .collect()
    }

    /// The visit under way: the first whose leaving step is not done (the last one when
    /// all are, or when the list ends without a leaving step).
    pub fn current_visit(&self, progress: &Progress) -> usize {
        let visits = self.visits();
        let last = visits.last().copied().unwrap_or(0);
        (0..last)
            .find(|&v| {
                // A visit's leaving step is its last step.
                let leaving = visits.iter().rposition(|&x| x == v).map(|i| &self.steps[i]);
                leaving.is_some_and(|step| !progress.completed.contains_key(&step.id))
            })
            .unwrap_or(last)
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackerView {
    zone: Zone,
    current_area: Option<String>,
    steps: Vec<StepView>,
    /// The visit under way: the page lists its steps only (`StepView::visit`).
    visit: usize,
    log: LogStatus,
}

pub struct Inner {
    pub routes: Vec<Route>,
    pub progress: Progress,
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
}

/// The log line of a game start.
const GAME_START: &str = "[STARTUP] Game Start";

/// Whether the log's last game start already led into a zone (true too when the log
/// shows no game start at all).
fn zone_since_start(text: &str) -> bool {
    match (text.rfind(GAME_START), text.rfind("Generating level ")) {
        (Some(start), Some(zone)) => zone > start,
        (Some(_), None) => false,
        (None, _) => true,
    }
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
}

/// A boss silent for this long is taken to be dead or left behind. The Bloated Miller's
/// lines came at most 19 s apart in two logged fights.
pub const FIGHT_QUIET: Duration = Duration::from_secs(30);

/// The open kill step in `area` of the boss who is speaking. As in the layout recorder,
/// the speaker must contain the checklist's boss name (at least four letters), so
/// "Beira" matches "Beira of the Rotten Pack".
pub fn boss_step<'a>(routes: &'a [Route], progress: &Progress, area: &str, speaker: &str) -> Option<&'a Step> {
    let speaker = speaker.to_lowercase();
    routes.iter().filter(|r| r.zone.id == area).flat_map(|r| &r.steps).find(|step| {
        !progress.completed.contains_key(&step.id)
            && step.parts.iter().any(|p| {
                let name = p.text.trim().to_lowercase();
                p.style.as_deref() == Some("boss") && name.len() >= 4 && speaker.contains(&name)
            })
    })
}

/// The log line of the player's death: `: <character> has been slain.`
fn is_death(line: &str) -> bool {
    line.trim_end().ends_with(" has been slain.") && line.contains("] : ")
}

/// Dialogue lines kept for `Inner::note_hand_tick`.
const RECENT_LINES: usize = 30;

impl Inner {
    /// Ends the fight when its step is ticked or the boss has been quiet too long.
    /// True when a fight ended.
    pub fn settle_fight(&mut self) -> bool {
        let over = self.fight.as_ref().is_some_and(|fight| {
            self.progress.completed.contains_key(&fight.step) || fight.last_line.elapsed() > FIGHT_QUIET
        });
        if over {
            self.fight = None;
        }
        over
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
                || progress.completed.contains_key(&step.id)
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

    pub fn sync_navigation(&self, app: &AppHandle) {
        let completed = self.routes.iter().flat_map(|r| &r.steps)
            .filter(|s| self.progress.completed.contains_key(&s.id))
            .filter_map(|s| s.navigation_target.as_ref())
            .map(|t| (t.area.clone(), t.label.clone())).collect();
        // Open steps with a target, in checklist order: what the compass may head for.
        // Only steps of each zone's visit under way: what the list shows.
        let targets = self.routes.iter()
            .flat_map(|r| {
                let now = r.current_visit(&self.progress);
                r.steps.iter().zip(r.visits()).filter(move |(_, visit)| *visit == now).map(|(step, _)| step)
            })
            .filter(|s| !self.progress.completed.contains_key(&s.id))
            .filter(|s| !(self.skip_optional && s.optional))
            .filter_map(|s| s.navigation_target.as_ref().map(|t| (t.area.clone(), crate::layout::guide::NavTarget {
                label: t.label.clone(),
                exit: t.exit.clone(),
                tag: t.tag.clone(),
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
        let recorder = app.state::<crate::layout::Recorder>();
        recorder.set_bosses(bosses);
        recorder.set_targets(targets);
        recorder.set_log_completed(by_log);
        if let Some(view) = recorder.set_completed(completed) {
            let _ = app.emit("layout-state", view);
        }
    }

    /// The checklist on show: the last zone entered that has one, else the first.
    fn shown(&self) -> &Route {
        let zone = self.progress.shown_zone.as_deref();
        self.routes.iter().find(|r| Some(r.zone.id.as_str()) == zone).unwrap_or(&self.routes[0])
    }

    pub fn view(&self) -> TrackerView {
        let route = self.shown();
        let visits = route.visits();
        let steps = route
            .steps
            .iter()
            .zip(visits)
            .map(|(step, visit)| {
                let done = self.progress.completed.get(&step.id);
                let state = if step.kind != "objective" {
                    "note"
                } else if done.is_some() {
                    "done"
                } else if step.trigger.is_some() {
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
                    visit,
                }
            })
            .collect();
        TrackerView {
            zone: route.zone.clone(),
            current_area: self.progress.current_area.clone(),
            steps,
            visit: route.current_visit(&self.progress),
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
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(&self.progress)?)?;
            std::fs::rename(tmp, path)
        })();
        if let Err(error) = result {
            eprintln!("Tracker: could not save progress: {error}");
        }
    }
}

pub struct Tracker(pub Mutex<Inner>);

pub fn init(app: &AppHandle) -> Tracker {
    let dir = app.path().app_data_dir().ok();
    let save_path = dir.as_ref().map(|dir| dir.join(PROGRESS_FILE));
    let read = |name: &str| {
        let bytes = std::fs::read(dir.as_ref()?.join(name)).ok()?;
        serde_json::from_slice::<Progress>(&bytes).ok()
    };
    let routes = load_routes();
    let mut progress = read(PROGRESS_FILE).or_else(|| read(OLD_PROGRESS_FILE)).unwrap_or_default();
    if progress.shown_zone.is_none() {
        progress.shown_zone =
            progress.current_area.clone().filter(|area| routes.iter().any(|r| &r.zone.id == area));
    }
    // Being in a zone proves it was entered, even if its entry line was read before
    // that zone's checklist existed.
    if let Some(area) = progress.current_area.clone() {
        // Started inside a zone: no zone line will come, so its exit names are set here.
        crate::minimap::set_label_candidates(exit_names(&area));
        apply(&routes, &mut progress, "on start", &LogEvent::ZoneEntered { area, level: 0, seed: None });
    }
    Tracker(Mutex::new(Inner {
        routes,
        progress,
        log: LogStatus { connected: false, path: None, message: "Looking for Client.txt".into() },
        save_path,
        skip_optional: false,
        recent: Default::default(),
        fight: None,
        in_zone: true,
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
fn current_zone(file: &mut File, len: u64) -> std::io::Result<(Option<LogEvent>, bool)> {
    let start = len.saturating_sub(FIRST_RUN_LOOKBACK);
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    file.take(len - start).read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    let zone = text
        .lines()
        .rev()
        .filter(|line| line.contains("Generating level "))
        .find_map(|line| parse_line(line).map(|(_, event)| event));
    Ok((zone, zone_since_start(&text)))
}

fn set_log_status(app: &AppHandle, connected: bool, path: Option<&Path>, message: &str) {
    let tracker = app.state::<Tracker>();
    let Ok(mut inner) = tracker.0.lock() else { return };
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
            set_log_status(&app, false, None, "Client.txt not found. Set POE2_CLIENT_LOG to its path.");
            std::thread::sleep(Duration::from_secs(3));
            continue;
        };
        if let Err(error) = follow(&app, &path) {
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
        let saved = tracker.0.lock().ok().and_then(|inner| {
            (inner.progress.log_path.as_deref() == Some(path_text.as_str()))
                .then_some(inner.progress.log_offset)
                .flatten()
        });
        let len = file.metadata()?.len();
        match saved {
            Some(offset) if offset <= len => offset,
            _ => first_run_offset(&mut file, len)?,
        }
    };
    // The layout recorder needs the current zone even when no new zone is entered.
    let (zone, in_zone) = current_zone(&mut file, pos)?;
    if let Some(zone) = zone {
        app.state::<crate::layout::Recorder>().on_log(&zone, false);
    }
    set_in_zone(app, in_zone);
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
            process(app, &path_text, &complete, read_through, live);
            continue; // more may be waiting beyond MAX_READ
        }
        live = true;
        tick_map_kills(app);
        settle_quiet_fight(app);
        std::thread::sleep(POLL);
    }
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
    let Ok(mut inner) = tracker.0.lock() else { return };
    let fighting = inner.fight.is_some();
    let was_in_zone = inner.in_zone;
    let Inner { routes, progress, recent, fight, in_zone, .. } = &mut *inner;
    let mut changed = false;
    // Every line is handled in order; one event never hides the rest of a batch.
    for line in String::from_utf8_lossy(bytes).lines() {
        if live && is_death(line) {
            *fight = None;
        }
        if line.contains(GAME_START) {
            *in_zone = false;
        }
        if let Some((timestamp, event)) = parse_line(line) {
            changed |= apply(routes, progress, &timestamp, &event);
            match &event {
                // A new zone: whatever fight there was is behind the player.
                LogEvent::ZoneEntered { .. } => {
                    *fight = None;
                    *in_zone = true;
                }
                LogEvent::Dialogue { speaker, .. } if live => {
                    let area = progress.current_area.as_deref().unwrap_or_default();
                    if let Some(step) = boss_step(routes, progress, area, speaker) {
                        *fight = Some(Fight { step: step.id.clone(), last_line: std::time::Instant::now() });
                    }
                }
                LogEvent::Dialogue { .. } => {}
            }
            if let LogEvent::ZoneEntered { area, .. } = &event {
                // The big map's exit labels are read against this zone's exit names.
                crate::minimap::set_label_candidates(exit_names(area));
            }
            recorder.on_log(&event, live);
            if let (true, LogEvent::Dialogue { speaker, text }) = (live, &event) {
                recent.push_back((timestamp, speaker.clone(), text.clone()));
                if recent.len() > RECENT_LINES {
                    recent.pop_front();
                }
            }
        }
    }
    progress.log_path = Some(path.to_string());
    progress.log_offset = Some(read_through);
    inner.save();
    if changed {
        inner.sync_navigation(app);
        let _ = app.emit("tracker-state", inner.view());
    }
    // After the checklist is up to date: a kill ticked in this batch ends the fight.
    inner.settle_fight();
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
    if let Ok(mut inner) = tracker.0.lock() {
        inner.in_zone = in_zone;
    }
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
    let Ok(mut inner) = tracker.0.lock() else { return };
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
    let ended = tracker.0.lock().is_ok_and(|mut inner| inner.settle_fight());
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
        // The waypoint detour and the final entry are the same zone line.
        assert!(progress.completed.contains_key("grelwood-red-vale-wp"));
        assert!(progress.completed.contains_key("enter-red-vale"));
        assert!(!progress.completed.contains_key("runic-seals"));
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
        // Finn says the same line when summoned in Freythorn; only the town talk is the hand-in.
        assert!(run("G1_12", "finn-charm").is_empty());
        assert_eq!(run("G1_town", "finn-charm"), ["finn-charm"]);
        // The Hooded One's Weapon-piece talk is the same on all three islands.
        assert_eq!(run("G4_7", "shrike-hooded-one"), ["shrike-hooded-one"]);
        assert_eq!(run("G4_3_2", "shrike-hooded-one"), ["caverns-hooded-one"]);
        assert_eq!(run("G4_5_2", "shrike-hooded-one"), ["prison-hooded-one"]);
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
        assert!(toggle(&routes, &mut progress, "directions-river").is_err());
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
        // Not a note, an optional step, a boss kill, a step the compass leads to,
        // one unticked by hand, or a step after the logged one.
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
        assert_eq!(progress.completed["talk-renly"].source, "implied");
        // Later talks in the town's list stay open.
        assert!(!progress.completed.contains_key("renly-runed-spikes"));
        assert!(!progress.completed.contains_key("town-una-roots"));
    }

    #[test]
    fn a_boss_the_map_shows_dead_ticks_its_kill_step_in_its_zone() {
        let mut inner = Inner {
            routes: load_routes(),
            progress: Progress::default(),
            log: LogStatus { connected: false, path: None, message: String::new() },
            save_path: None,
            skip_optional: false,
            recent: Default::default(),
            fight: None,
            in_zone: true,
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
        assert_eq!(visit_of("enter-vastiri-outskirts"), 4);
        let mut progress = Progress::default();
        assert_eq!(town.current_visit(&progress), 0);
        // The visit ends when its leaving step is done, whatever else is open.
        enter_zone(&routes, &mut progress, "G1_town");
        enter_zone(&routes, &mut progress, "G1_2");
        assert_eq!(town.current_visit(&progress), 1);
        // A zone left once: one visit.
        let riverbank = routes.iter().find(|r| r.zone.id == "G1_1").unwrap();
        assert!(riverbank.visits().iter().all(|&v| v == 0));
        assert_eq!(riverbank.current_visit(&progress), 0);
    }

    #[test]
    fn taking_back_a_click_leaves_the_step_open_for_the_log() {
        let routes = load_routes();
        let mut progress = Progress::default();
        // Ticked by a click, then unticked by a click: open again, not held back.
        toggle(&routes, &mut progress, "enter-clearfell").unwrap();
        toggle(&routes, &mut progress, "enter-clearfell").unwrap();
        assert!(progress.dismissed.is_empty());
        enter_zone(&routes, &mut progress, "G1_2");
        assert_eq!(progress.completed["enter-clearfell"].source, "log");
        // Unticking what the log ticked holds it back.
        toggle(&routes, &mut progress, "enter-clearfell").unwrap();
        assert!(progress.dismissed.contains("enter-clearfell"));
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
    fn a_fight_ends_with_the_kill_or_a_quiet_boss() {
        let mut inner = Inner {
            routes: load_routes(),
            progress: Progress::default(),
            log: LogStatus { connected: false, path: None, message: String::new() },
            save_path: None,
            skip_optional: false,
            recent: Default::default(),
            fight: None,
            in_zone: true,
        };
        let fight = |ago: Duration| Some(Fight { step: "kill-bloated-miller".into(), last_line: std::time::Instant::now() - ago });
        assert!(!inner.settle_fight(), "no fight");
        inner.fight = fight(Duration::from_secs(5));
        assert!(!inner.settle_fight(), "the boss spoke five seconds ago");
        assert!(inner.fight.is_some());
        inner.fight = fight(FIGHT_QUIET + Duration::from_secs(1));
        assert!(inner.settle_fight(), "quiet for too long");
        assert!(inner.fight.is_none());
        inner.fight = fight(Duration::from_secs(1));
        done(&mut inner.progress, "kill-bloated-miller");
        assert!(inner.settle_fight(), "the kill is ticked");
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
    }

    #[test]
    fn the_players_death_line_is_told_from_chat() {
        assert!(is_death(&line("INFO", ": Somebody has been slain.")));
        assert!(!is_death(&line("INFO", "#Somebody: the boss has been slain.")));
        assert!(!is_death(&line("INFO", "Somebody: he has been slain.")));
        assert!(!is_death(&line("INFO", ": Somebody (Ranger) is now level 2")));
    }
}
