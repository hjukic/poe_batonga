//! Session-safe character binding. Screen extraction runs separately from log tracking.
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use std::{path::PathBuf, sync::{Mutex, OnceLock}};

static TRACE: OnceLock<Option<(PathBuf, Mutex<()>)>> = OnceLock::new();

/// Opt-in, bounded local diagnostics. Never called from the input hook.
pub fn trace(stage: &str, details: serde_json::Value) {
    let Some(Some((path, lock))) = TRACE.get() else { return };
    let _guard = crate::relock(lock);
    let result = (|| -> std::io::Result<()> {
        use std::io::Write;
        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
        if std::fs::metadata(path).is_ok_and(|m| m.len() > 2_000_000) {
            let previous = path.with_extension("previous.log");
            if previous.exists() { std::fs::remove_file(&previous)?; }
            std::fs::rename(path, previous)?;
        }
        let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
        writeln!(std::fs::OpenOptions::new().create(true).append(true).open(path)?, "{}",
            serde_json::json!({"timeMs":time,"pid":std::process::id(),"stage":stage,"details":details}))
    })();
    if let Err(error) = result { eprintln!("Identity diagnostics: {error}"); }
}

// The character screens are read on Windows only. Linux read them with tesseract from
// v0.1.112 to v0.1.116; it was removed (owner, 2026-11-13: "log as main source, remove that
// linux tesseract"): the game's log names the character on every level-up and death
// (`tracker::character_in_line`, `Characters::bind_from_log`), on every platform.
#[cfg(target_os = "windows")]
#[path = "identity_native.rs"]
mod shared;
#[cfg(target_os = "windows")]
#[path = "identity_windows.rs"]
mod native;

/// On quit (Linux removed its input binds here while it read the screens).
pub fn stop() {}

/// Whether the character is read from the creation and selection screens here (Windows).
/// Elsewhere the log alone names it, and the last character stays chosen in between.
pub fn recognises() -> bool {
    cfg!(target_os = "windows")
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KnownCharacter {
    pub name: String,
    pub league: String,
    #[serde(default)]
    pub source: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Candidate {
    pub name: String,
    pub league: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct ScreenResult {
    pub screen: String,
    pub status: String,
    pub candidate: Option<Candidate>,
    #[serde(default)]
    pub resolution: Option<String>,
    pub reason: Option<String>,
}

#[derive(Default)]
pub struct Recognition {
    pub epoch: u64,
    pub seq: u64,
    pub captured_in_zone: bool,
    pub submitted: bool,
    pub activation_parent: Option<u64>,
    pub pending: Option<Candidate>,
    pub pending_from_history: bool,
    pub note: String,
}

impl Recognition {
    pub fn invalidate(&mut self) {
        self.epoch += 1;
        self.seq = 0;
        self.submitted = false;
        self.activation_parent = None;
        self.pending = None;
        self.pending_from_history = false;
        self.note = "Waiting for the character creation or selection screen".into();
    }

    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub fn begin(&mut self, seq: u64, in_zone: bool, activation: bool) -> u64 {
        // Continue/Play may already have faded by the time the capture worker runs.
        // Only retain the read of the immediately preceding input, never a name
        // from before an unprocessed edit or selection change.
        let consecutive = self.seq.checked_add(1) == Some(seq);
        self.activation_parent = (activation && consecutive).then_some(self.seq);
        if !activation || !consecutive {
            self.pending = None;
            self.pending_from_history = false;
        }
        self.seq = seq;
        self.captured_in_zone = in_zone;
        self.submitted = activation;
        self.note = "Reading character screen…".into();
        self.epoch
    }

    /// Takes a menu frame's read as the name waiting for zone entry. The
    /// activation's own frame may show a fading highlight or a hover tooltip:
    /// without a name of its own it keeps the read of the input directly
    /// before it (`begin` cleared anything older). False when it kept it.
    pub fn observe(&mut self, read: Option<Candidate>, from_history: bool, preceding_frame: bool) -> bool {
        if read.is_none() && !preceding_frame && self.submitted && self.pending.is_some() { return false; }
        self.pending = read;
        self.pending_from_history = from_history;
        true
    }

    /// A frame is current for the newest input, or for the selection click
    /// directly before a just-requested activation that has not begun yet.
    pub fn current(&self, epoch: u64, seq: u64) -> bool {
        self.accepts(epoch, seq, latest_input()) || self.accepts(epoch, seq, superseding_input())
    }

    pub fn accepts(&self, epoch: u64, seq: u64, latest: u64) -> bool {
        self.epoch == epoch && self.seq == latest &&
            (self.seq == seq || self.activation_parent == Some(seq))
    }
}

pub fn set_context(in_zone: bool, bound: bool) {
    #[cfg(target_os = "windows")]
    native::set_context(in_zone, bound);
    #[cfg(not(target_os = "windows"))]
    let _ = (in_zone, bound);
}

pub fn latest_input() -> u64 {
    #[cfg(target_os = "windows")]
    { native::latest_input() }
    #[cfg(not(target_os = "windows"))]
    { 0 }
}

/// Like `latest_input`, except that a just-requested activation does not
/// supersede the selection frame preceding it (see `native::superseding_input`).
fn superseding_input() -> u64 {
    #[cfg(target_os = "windows")]
    { native::superseding_input() }
    #[cfg(not(target_os = "windows"))]
    { 0 }
}

pub fn spawn(app: AppHandle) {
    let _ = TRACE.set(std::env::var_os("BATONGA_IDENTITY_DIAGNOSTICS").map(|p| (PathBuf::from(p), Mutex::new(()))));
    trace("startup", serde_json::json!({"version":app.package_info().version.to_string()}));
    #[cfg(target_os = "windows")]
    native::spawn(app);
    #[cfg(not(target_os = "windows"))]
    {
        let tracker = app.state::<crate::tracker::Tracker>();
        crate::relock(&tracker.0).identity.note = "Your character is read from the game log: it is known from your first level-up or death after you log in".into();
    }
}

/// Called after a real zone entry, or after OCR for the pre-entry frame finishes.
pub fn bind_pending(inner: &mut crate::tracker::Inner) -> bool {
    bind_at(inner, latest_input())
}

fn bind_at(inner: &mut crate::tracker::Inner, latest: u64) -> bool {
    if !inner.in_zone || !inner.identity.submitted || inner.identity.seq != latest { return false; }
    let Some(candidate) = inner.identity.pending.take() else { return false };
    match inner.characters.select(&mut inner.progress, &candidate.name, false) {
        Ok(()) => {
            let from_history = inner.identity.pending_from_history;
            inner.characters.active_source = Some(if from_history { "screen-history-prefix" } else { "screen-complete" }.into());
            if !from_history {
                // An unread league is remembered empty: the name is what identifies.
                let league = candidate.league.clone().unwrap_or_default();
                let known = KnownCharacter { name: candidate.name.clone(), league, source: "complete-screen-read".into() };
                remember_complete(&mut inner.characters, known);
            }
            crate::tracker::replay_unbound(inner);
            inner.identity.note = format!("Tracking: {}{}", inner.characters.active.as_deref().unwrap_or(&candidate.name),
                if from_history { " (name completed from history)" } else { "" });
            set_context(true, true);
            trace("bound", serde_json::json!({"name":inner.characters.active,"source":inner.characters.active_source,"seq":inner.identity.seq}));
            true
        }
        Err(error) => { inner.identity.note = error; false }
    }
}

/// False when it was known already. A name remembered without its league gets it.
fn remember_complete(characters: &mut crate::characters::Characters, known: KnownCharacter) -> bool {
    let same = |k: &KnownCharacter| k.name.to_lowercase() == known.name.to_lowercase();
    if characters.known.iter().any(|k| same(k) && k.league.to_lowercase() == known.league.to_lowercase()) { return false; }
    if !known.league.trim().is_empty() {
        if let Some(blank) = characters.known.iter_mut().find(|k| same(k) && k.league.trim().is_empty()) {
            blank.league = known.league;
            return true;
        }
    } else if characters.known.iter().any(|k| same(k)) {
        return false;
    }
    characters.known.push(known);
    true
}

#[tauri::command]
pub fn identity_observed(app: AppHandle, epoch: u64, seq: u64, result: ScreenResult, learned: Vec<KnownCharacter>, dry_run: bool) -> Result<(), String> {
    trace(if dry_run {"reference-result"} else {"screen-result"}, serde_json::json!({"epoch":epoch,"seq":seq,"result":result}));
    if dry_run { return Ok(()); } // Diagnostic references never select/save profiles.
    let tracker = app.state::<crate::tracker::Tracker>();
    let mut inner = crate::relock(&tracker.0);
    if !inner.identity.current(epoch, seq) {
        trace("result-rejected", serde_json::json!({"epoch":epoch,"seq":seq,"currentEpoch":inner.identity.epoch,"currentSeq":inner.identity.seq,"latest":latest_input()}));
        return Ok(());
    }
    let preceding_frame = seq != inner.identity.seq;
    let menu = matches!(result.screen.as_str(), "creation" | "selection");
    let returned_to_menu = menu && inner.identity.captured_in_zone && inner.in_zone;
    if returned_to_menu {
        let submitted = inner.identity.submitted;
        inner.identity.invalidate();
        let boundary = Some(format!("screen-menu:{}", inner.identity.epoch));
        let crate::tracker::Inner { characters, progress, .. } = &mut *inner;
        characters.clear_binding(progress);
        characters.session = boundary;
        inner.identity.seq = seq;
        inner.identity.submitted = submitted;
        inner.in_zone = false;
        inner.fight = None;
        inner.recent.clear();
    }
    if menu {
        // A readable activation frame supersedes its predecessor. An unreadable
        // loading frame may still receive the immediately preceding name read.
        if !preceding_frame { inner.identity.activation_parent = None; }
        set_context(inner.in_zone, false);
        // Saved only when something changed, not on every menu read (review item 10).
        let mut changed = returned_to_menu;
        for known in learned.into_iter().filter(|k| valid_name(&k.name) && k.source == "complete-screen-read") {
            changed |= remember_complete(&mut inner.characters, known);
        }
        let read = if result.status == "candidate" {
            // The league is no condition: it changes every few months and the game
            // cuts long ones short. The screen code checks that this is a character row.
            result.candidate.filter(|c| valid_name(&c.name)).map(|mut c| {
                c.league = c.league.filter(|l| !l.trim().is_empty());
                c
            })
        } else { None };
        let from_history = result.resolution.as_deref().is_some_and(|r| r.starts_with("history-prefix"));
        if !inner.identity.observe(read, from_history, preceding_frame) {
            trace("activation-kept-preceding", serde_json::json!({"seq":seq,"status":result.status,"reason":result.reason}));
        }
        inner.identity.note = match &inner.identity.pending {
            Some(c) => format!("Recognized {}{}; waiting for zone entry", c.name,
                if inner.identity.pending_from_history { " (from history)" } else { "" }),
            None => "Character name is not clear yet; uncover it or choose a profile in settings".into(),
        };
        changed |= bind_pending(&mut inner);
        if changed { inner.save(); }
        inner.sync_navigation(&app);
    } else {
        #[cfg(target_os = "windows")]
        native::finish_unknown();
        if inner.characters.active.is_none() {
            inner.identity.note = "Waiting for the character creation or selection screen".into();
        }
    }
    let _ = app.emit("tracker-state", inner.view());
    trace("binding-state", serde_json::json!({"epoch":inner.identity.epoch,"seq":inner.identity.seq,"inZone":inner.in_zone,"submitted":inner.identity.submitted,"pending":inner.identity.pending,"active":inner.characters.active}));
    drop(inner);
    if returned_to_menu {
        let _ = app.emit("in-zone", false);
        crate::set_fight(&app, false);
    }
    Ok(())
}

#[tauri::command]
pub fn identity_ready(app: AppHandle) {
    trace("frontend-ready", serde_json::json!({}));
    #[cfg(target_os = "windows")]
    native::reference_probe(app);
    #[cfg(not(target_os = "windows"))]
    let _ = app;
}

fn valid_name(name: &str) -> bool {
    (2..=80).contains(&name.chars().count()) && name.chars().all(|c| c.is_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logout_and_newer_input_reject_older_ocr_results() {
        let mut state = Recognition::default();
        let epoch = state.begin(1, false, false);
        assert!(state.accepts(epoch, 1, 1));
        assert!(!state.accepts(epoch, 1, 2));
        state.invalidate();
        assert!(!state.accepts(epoch, 1, 1));
        let next = state.begin(2, true, false);
        assert!(state.accepts(next, 2, 2));
        assert!(state.captured_in_zone);
    }
    #[test]
    fn editing_clears_the_prior_name_before_a_new_read() {
        let mut state = Recognition::default();
        state.pending = Some(Candidate { name: "Earlier_Name".into(), league: Some("League".into()) });
        state.begin(4, false, false);
        assert!(state.pending.is_none());
    }
    #[test]
    fn activation_can_keep_only_the_immediately_preceding_read() {
        let mut state = Recognition::default();
        state.begin(1, false, false);
        state.pending = Some(Candidate { name: "Final_Name".into(), league: Some("League".into()) });
        state.begin(2, false, true);
        assert!(state.pending.is_some());
        assert!(state.submitted);
        assert!(state.accepts(state.epoch, 1, 2), "pre-transition OCR may finish after Continue");
        state.begin(4, false, true); // An unprocessed edit happened at input 3.
        assert!(state.pending.is_none());
        assert!(!state.accepts(state.epoch, 1, 4));
    }

    #[test]
    fn an_unreadable_activation_frame_keeps_the_selection_read_before_it() {
        let hero = || Some(Candidate { name: "Hero".into(), league: Some("League".into()) });
        let mut state = Recognition::default();
        state.begin(1, false, false);
        assert!(state.observe(hero(), true, false));
        state.begin(2, false, true); // Play, or the row's second click
        assert!(!state.observe(None, false, false), "ambiguous frame: no selected row");
        assert_eq!(state.pending.as_ref().map(|c| c.name.as_str()), Some("Hero"));
        assert!(state.pending_from_history);
        let other = Candidate { name: "Other".into(), league: Some("League".into()) };
        assert!(state.observe(Some(other), false, false), "a readable activation frame wins");
        assert_eq!(state.pending.as_ref().map(|c| c.name.as_str()), Some("Other"));
        state.begin(3, false, false);
        state.pending = hero();
        assert!(state.observe(None, false, false), "a selection click without a name clears it");
        assert!(state.pending.is_none());
    }

    #[test]
    fn a_name_is_remembered_without_its_league_and_gets_it_later() {
        let mut chars = crate::characters::Characters::default();
        let known = |name: &str, league: &str| KnownCharacter { name: name.into(), league: league.into(), source: "complete-screen-read".into() };
        assert!(remember_complete(&mut chars, known("Hero", "")));
        assert!(!remember_complete(&mut chars, known("HERO", "")));
        assert!(remember_complete(&mut chars, known("Hero", "Fate of the Vaal")), "the league fills the blank");
        assert_eq!(chars.known.len(), 1);
        assert_eq!(chars.known[0].league, "Fate of the Vaal");
        assert!(!remember_complete(&mut chars, known("Hero", "")), "known with its league already");
        assert!(remember_complete(&mut chars, known("Hero", "Standard")), "moved to Standard when its league ended");
        assert_eq!(chars.known.len(), 2);
    }

    #[test]
    fn a_candidate_without_a_league_still_binds() {
        let mut inner = tracker();
        inner.identity.begin(1, false, true);
        inner.identity.pending = Some(Candidate { name: "OGLEDALOGOTOVOOOOO".into(), league: None });
        inner.in_zone = true;
        assert!(bind_at(&mut inner, 1));
        assert_eq!(inner.characters.active.as_deref(), Some("OGLEDALOGOTOVOOOOO"));
        assert_eq!(inner.characters.known.len(), 1);
    }

    fn tracker() -> crate::tracker::Inner {
        crate::tracker::Inner {
            routes: vec![], progress: Default::default(), characters: Default::default(),
            identity: Default::default(), log: crate::tracker::LogStatus {
                connected: false, path: None, message: String::new(),
            }, save_path: None, skip_optional: false, recent: Default::default(),
            fight: None, in_zone: false, saved_at: Default::default(),
        }
    }

    #[test]
    fn a_name_without_activation_and_zone_entry_never_creates_a_profile() {
        let mut inner = tracker();
        inner.identity.begin(1, false, false);
        inner.identity.pending = Some(Candidate { name: "New_Hero".into(), league: Some("League".into()) });
        assert!(!bind_at(&mut inner, 1));
        inner.in_zone = true;
        assert!(!bind_at(&mut inner, 1), "a typed but unsubmitted name is not active identity");
        inner.identity.begin(2, false, true);
        inner.in_zone = false;
        assert!(!bind_at(&mut inner, 2));
        inner.in_zone = true;
        assert!(!bind_at(&mut inner, 3), "a newer edit invalidates the submitted frame");
        assert!(bind_at(&mut inner, 2));
        assert_eq!(inner.characters.active.as_deref(), Some("New_Hero"));
        assert_eq!(inner.characters.known.len(), 1, "a creation name is remembered only after entry");
        assert!(!bind_at(&mut inner, 2), "the candidate is consumed once");
    }

    #[test]
    fn entry_restores_only_the_identified_profiles_ticks_and_records_provenance() {
        let mut inner = tracker();
        inner.characters.select(&mut inner.progress, "Saved_Hero", false).unwrap();
        inner.progress.completed.insert("boss".into(), crate::tracker::Completion {
            at: "before".into(), source: "manual".into(), evidence: String::new(),
        });
        inner.characters.clear_binding(&mut inner.progress);
        inner.identity.begin(8, false, true);
        inner.identity.pending = Some(Candidate { name: "SAVED_HERO".into(), league: Some("League".into()) });
        inner.identity.pending_from_history = true;
        inner.in_zone = true;
        assert!(bind_at(&mut inner, 8));
        assert!(inner.progress.completed.contains_key("boss"));
        assert_eq!(inner.characters.active.as_deref(), Some("Saved_Hero"));
        assert_eq!(inner.characters.active_source.as_deref(), Some("screen-history-prefix"));
        assert!(inner.characters.known.is_empty(), "inferred names never become complete observations");
        inner.characters.new_session(&mut inner.progress, Some("next-login".into()));
        inner.identity.invalidate();
        inner.identity.pending = Some(Candidate { name: "Other_Hero".into(), league: Some("League".into()) });
        inner.identity.submitted = true;
        assert!(bind_at(&mut inner, 0));
        assert!(inner.progress.completed.is_empty());
    }
}
