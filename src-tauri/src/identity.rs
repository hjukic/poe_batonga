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

#[cfg(target_os = "windows")]
#[path = "identity_windows.rs"]
mod native;

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

pub fn spawn(app: AppHandle) {
    let _ = TRACE.set(std::env::var_os("BATONGA_IDENTITY_DIAGNOSTICS").map(|p| (PathBuf::from(p), Mutex::new(()))));
    trace("startup", serde_json::json!({"version":app.package_info().version.to_string()}));
    #[cfg(target_os = "windows")]
    native::spawn(app);
    #[cfg(not(target_os = "windows"))]
    {
        let tracker = app.state::<crate::tracker::Tracker>();
        crate::relock(&tracker.0).identity.note = "Automatic screen recognition is currently available on Windows".into();
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
                if let Some(league) = candidate.league.as_ref() {
                    let known = KnownCharacter { name: candidate.name.clone(), league: league.clone(), source: "complete-screen-read".into() };
                    remember_complete(&mut inner.characters, known);
                }
            }
            inner.identity.note = format!("Tracking: {}{}", inner.characters.active.as_deref().unwrap_or(&candidate.name),
                if from_history { " (name completed from history)" } else { "" });
            set_context(true, true);
            trace("bound", serde_json::json!({"name":inner.characters.active,"source":inner.characters.active_source,"seq":inner.identity.seq}));
            true
        }
        Err(error) => { inner.identity.note = error; false }
    }
}

fn remember_complete(characters: &mut crate::characters::Characters, known: KnownCharacter) {
    if !characters.known.iter().any(|k| k.name.to_lowercase() == known.name.to_lowercase() && k.league.to_lowercase() == known.league.to_lowercase()) {
        characters.known.push(known);
    }
}

#[tauri::command]
pub fn identity_observed(app: AppHandle, epoch: u64, seq: u64, result: ScreenResult, learned: Vec<KnownCharacter>, dry_run: bool) -> Result<(), String> {
    trace(if dry_run {"reference-result"} else {"screen-result"}, serde_json::json!({"epoch":epoch,"seq":seq,"result":result}));
    if dry_run { return Ok(()); } // Diagnostic references never select/save profiles.
    let tracker = app.state::<crate::tracker::Tracker>();
    let mut inner = crate::relock(&tracker.0);
    if !inner.identity.accepts(epoch, seq, latest_input()) {
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
        for known in learned.into_iter().filter(|k| valid_name(&k.name) && !k.league.trim().is_empty() && k.source == "complete-screen-read") {
            remember_complete(&mut inner.characters, known);
        }
        inner.identity.pending = if result.status == "candidate" {
            result.candidate.filter(|c| valid_name(&c.name) && c.league.as_ref().is_some_and(|l| !l.trim().is_empty()))
        } else { None };
        inner.identity.pending_from_history = result.resolution.as_deref() == Some("history-prefix");
        inner.identity.note = match &inner.identity.pending {
            Some(c) => format!("Recognized {}{}; waiting for zone entry", c.name,
                if result.resolution.as_deref() == Some("history-prefix") { " (from history)" } else { "" }),
            None => "Character name is not clear yet; uncover it or choose a profile in settings".into(),
        };
        bind_pending(&mut inner);
        inner.save();
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
