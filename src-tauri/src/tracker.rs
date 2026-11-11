//! Read-only Client.txt tracking for the Riverbank route.
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

const ROUTE_JSON: &str = include_str!("../../data/routes/riverbank.json");
const PROGRESS_FILE: &str = "riverbank-progress.json";
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Trigger {
    NpcLine { area: String, speaker: String, text: String },
    ZoneEntered { area: String },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Step {
    pub id: String,
    pub kind: String,
    pub parts: Vec<Part>,
    #[serde(default)]
    pub trigger: Option<Trigger>,
    #[serde(default, rename = "navigationTarget")]
    pub navigation_target: Option<NavigationTarget>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NavigationTarget {
    pub area: String,
    pub label: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Route {
    pub zone: Zone,
    pub steps: Vec<Step>,
}

pub fn load_route() -> Route {
    serde_json::from_str(ROUTE_JSON).expect("bundled route data is valid")
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
    // starts with #, &, @, %, $ and player names have no spaces, so an exact
    // speaker match on names like "Wounded Man" cannot come from a player.
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
pub fn apply(route: &Route, progress: &mut Progress, timestamp: &str, event: &LogEvent) -> bool {
    let mut changed = false;
    if let LogEvent::ZoneEntered { area, .. } = event {
        if progress.current_area.as_deref() != Some(area) {
            progress.current_area = Some(area.clone());
            changed = true;
        }
    }
    for step in &route.steps {
        let Some(trigger) = &step.trigger else { continue };
        if progress.completed.contains_key(&step.id) || progress.dismissed.contains(&step.id) {
            continue;
        }
        let evidence = match (trigger, event) {
            (Trigger::ZoneEntered { area }, LogEvent::ZoneEntered { area: got, level, .. }) if area == got => {
                format!("Entered {got} (level {level})")
            }
            (Trigger::NpcLine { area, speaker, text }, LogEvent::Dialogue { speaker: who, text: said })
                if progress.current_area.as_deref() == Some(area.as_str())
                    && speaker == who
                    && text == said =>
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
    changed
}

/// Ticks or unticks a step by hand.
pub fn toggle(route: &Route, progress: &mut Progress, id: &str) -> Result<(), String> {
    let step = route.steps.iter().find(|s| s.id == id).ok_or("Unknown step")?;
    if step.kind != "objective" {
        return Err("Only objectives can be ticked".into());
    }
    if progress.completed.remove(id).is_some() {
        progress.dismissed.insert(id.to_string());
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
    /// "done", "open" (watched in the log) or "manual" (no log trigger known).
    state: String,
    source: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogStatus {
    pub connected: bool,
    pub path: Option<String>,
    pub message: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackerView {
    zone: Zone,
    current_area: Option<String>,
    steps: Vec<StepView>,
    log: LogStatus,
}

pub struct Inner {
    pub route: Route,
    pub progress: Progress,
    pub log: LogStatus,
    pub save_path: Option<PathBuf>,
}

impl Inner {
    pub fn sync_navigation(&self, app: &AppHandle) {
        let completed = self.route.steps.iter().filter(|s| self.progress.completed.contains_key(&s.id))
            .filter_map(|s| s.navigation_target.as_ref())
            .map(|t| (t.area.clone(), t.label.clone())).collect();
        if let Some(view) = app.state::<crate::layout::Recorder>().set_completed(completed) {
            let _ = app.emit("layout-state", view);
        }
    }

    pub fn view(&self) -> TrackerView {
        let steps = self
            .route
            .steps
            .iter()
            .map(|step| {
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
                    state: state.into(),
                    source: done.map(|c| c.source.clone()),
                }
            })
            .collect();
        TrackerView {
            zone: self.route.zone.clone(),
            current_area: self.progress.current_area.clone(),
            steps,
            log: self.log.clone(),
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
    let save_path = app.path().app_data_dir().ok().map(|dir| dir.join(PROGRESS_FILE));
    let progress = save_path
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    Tracker(Mutex::new(Inner {
        route: load_route(),
        progress,
        log: LogStatus { connected: false, path: None, message: "Looking for Client.txt".into() },
        save_path,
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
fn current_zone(file: &mut File, len: u64) -> std::io::Result<Option<LogEvent>> {
    let start = len.saturating_sub(FIRST_RUN_LOOKBACK);
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    file.take(len - start).read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    Ok(text
        .lines()
        .rev()
        .filter(|line| line.contains("Generating level "))
        .find_map(|line| parse_line(line).map(|(_, event)| event)))
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
    if let Some(zone) = current_zone(&mut file, pos)? {
        app.state::<crate::layout::Recorder>().on_log(&zone, false);
    }
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
        std::thread::sleep(POLL);
    }
}

fn process(app: &AppHandle, path: &str, bytes: &[u8], read_through: u64, live: bool) {
    let recorder = app.state::<crate::layout::Recorder>();
    let tracker = app.state::<Tracker>();
    let Ok(mut inner) = tracker.0.lock() else { return };
    let Inner { route, progress, .. } = &mut *inner;
    let mut changed = false;
    // Every line is handled in order; one event never hides the rest of a batch.
    for line in String::from_utf8_lossy(bytes).lines() {
        if let Some((timestamp, event)) = parse_line(line) {
            changed |= apply(route, progress, &timestamp, &event);
            recorder.on_log(&event, live);
        }
    }
    progress.log_path = Some(path.to_string());
    progress.log_offset = Some(read_through);
    inner.save();
    if changed {
        inner.sync_navigation(app);
        let _ = app.emit("tracker-state", inner.view());
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
        let route = load_route();
        let mut progress = Progress::default();
        apply(&route, &mut progress, "now", &LogEvent::ZoneEntered {
            area: "G1_1".into(), level: 1, seed: Some(7),
        });
        apply(&route, &mut progress, "now", &LogEvent::Dialogue {
            speaker: "The Bloated Miller".into(), text: "Fight starts".into(),
        });
        assert!(!progress.completed.contains_key("kill-bloated-miller"));
        toggle(&route, &mut progress, "kill-bloated-miller").unwrap();
        let step = route.steps.iter().find(|s| s.id == "kill-bloated-miller").unwrap();
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
        let route = load_route();
        for l in lines {
            if let Some((ts, event)) = parse_line(l) {
                apply(&route, progress, &ts, &event);
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
    fn dialogue_outside_the_riverbank_does_not_count() {
        let mut progress = Progress::default();
        feed(
            &mut progress,
            &[
                line("DEBUG", r#"Generating level 15 area "G1_town" with seed 1"#),
                line("INFO", "Wounded Man: Reach... Clearfell... Find the Miller..."),
            ],
        );
        assert!(!progress.completed.contains_key("talk-wounded-man"));
        assert!(progress.completed.contains_key("enter-clearfell-encampment"));
    }

    #[test]
    fn unticked_steps_stay_unticked() {
        let route = load_route();
        let mut progress = Progress::default();
        feed(&mut progress, &[line("DEBUG", r#"Generating level 15 area "G1_town" with seed 1"#)]);
        toggle(&route, &mut progress, "enter-clearfell-encampment").unwrap();
        feed(&mut progress, &[line("DEBUG", r#"Generating level 15 area "G1_town" with seed 2"#)]);
        assert!(!progress.completed.contains_key("enter-clearfell-encampment"));
        toggle(&route, &mut progress, "enter-clearfell-encampment").unwrap();
        assert_eq!(progress.completed["enter-clearfell-encampment"].source, "manual");
        assert!(toggle(&route, &mut progress, "directions-river").is_err());
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
}
