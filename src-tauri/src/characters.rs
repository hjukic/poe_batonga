//! Local character profiles. Unknown log sessions never inherit another character's ticks.
use crate::tracker::Progress;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Characters {
    pub active: Option<String>,
    pub active_source: Option<String>,
    pub session: Option<String>,
    pub profiles: BTreeMap<String, Progress>,
    /// Pre-profile progress has no trustworthy owner. Import only by explicit choice.
    pub legacy: Option<Progress>,
    pub known: Vec<crate::identity::KnownCharacter>,
}

#[derive(Serialize, Deserialize)]
pub struct SavedProgress {
    #[serde(flatten)]
    pub progress: Progress,
    #[serde(default)]
    pub characters: Characters,
}

impl Characters {
    pub fn remember(&mut self, progress: &Progress) {
        if let Some(name) = &self.active {
            self.profiles.insert(name.clone(), progress.clone());
        }
    }

    pub fn new_session(&mut self, progress: &mut Progress, session: Option<String>) -> bool {
        if self.session == session {
            return false;
        }
        self.session = session;
        self.clear_binding(progress);
        true
    }

    /// Stored progress survives a restart; a stored active name is not fresh identity.
    pub fn clear_binding(&mut self, progress: &mut Progress) {
        self.remember(progress);
        self.active = None;
        self.active_source = None;
        let old = std::mem::take(progress);
        progress.log_path = old.log_path;
        progress.log_offset = old.log_offset;
    }

    pub fn select(&mut self, progress: &mut Progress, name: &str, import_legacy: bool) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err("Enter a character name (1–80 characters)".into());
        }
        let matching: Vec<_> = self.profiles.keys().filter(|key| key.to_lowercase() == name.to_lowercase()).cloned().collect();
        if matching.len() > 1 { return Err("Several saved profiles differ only by letter case; select a distinct profile name".into()); }
        let name = matching.first().map(String::as_str).unwrap_or(name);
        if import_legacy && (self.legacy.is_none() || self.profiles.contains_key(name)) {
            return Err("Previous progress can only be assigned to a new character profile".into());
        }
        let unassigned = self.active.is_none().then(|| progress.clone());
        self.remember(progress);
        let mut next = if import_legacy {
            self.legacy.take().unwrap_or_default()
        } else {
            self.profiles.get(name).cloned().unwrap_or_default()
        };
        // Events observed in this login before choosing a character belong to the
        // character the owner now selects. Explicit corrections in its profile win.
        if let Some(pending) = unassigned {
            for (id, done) in pending.completed {
                if !next.dismissed.contains(&id) {
                    next.completed.entry(id).or_insert(done);
                }
            }
        }
        next.current_area = progress.current_area.clone();
        next.shown_zone = progress.shown_zone.clone();
        next.log_path = progress.log_path.clone();
        next.log_offset = progress.log_offset;
        *progress = next;
        self.active = Some(name.to_string());
        self.active_source = Some("manual".into());
        self.remember(progress);
        Ok(())
    }
}

pub fn load(bytes: &[u8]) -> Result<SavedProgress, serde_json::Error> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if value.get("characters").is_some() {
        serde_json::from_value(value)
    } else {
        let legacy: Progress = serde_json::from_value(value)?;
        let progress = Progress { log_path: legacy.log_path.clone(), log_offset: legacy.log_offset, ..Default::default() };
        Ok(SavedProgress { progress, characters: Characters { legacy: Some(legacy), ..Default::default() } })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::Completion;

    fn tick(progress: &mut Progress, id: &str) {
        progress.completed.insert(id.into(), Completion { at: "now".into(), source: "manual".into(), evidence: String::new() });
    }

    #[test]
    fn ocr_case_changes_reuse_the_profile_and_preserve_its_spelling() {
        let mut chars = Characters::default();
        let mut progress = Progress::default();
        chars.select(&mut progress, "Existing_Hero", false).unwrap();
        tick(&mut progress, "boss");
        chars.clear_binding(&mut progress);
        assert!(chars.active.is_none());
        assert!(progress.completed.is_empty());
        chars.select(&mut progress, "EXISTING_HERO", false).unwrap();
        assert_eq!(chars.active.as_deref(), Some("Existing_Hero"));
        assert_eq!(chars.profiles.len(), 1);
        assert!(progress.completed.contains_key("boss"));
    }

    #[test]
    fn conflicting_legacy_case_variants_are_not_arbitrarily_chosen() {
        let mut chars = Characters::default();
        chars.profiles.insert("Existing_Hero".into(), Progress::default());
        chars.profiles.insert("EXISTING_HERO".into(), Progress::default());
        let mut progress = Progress::default();
        assert!(chars.select(&mut progress, "existing_hero", false).is_err());
        assert!(chars.active.is_none());
    }

    #[test]
    fn characters_keep_separate_ticks_across_switches_and_restart() {
        let mut chars = Characters::default();
        let mut progress = Progress::default();
        chars.select(&mut progress, "First", false).unwrap();
        tick(&mut progress, "boss");
        chars.select(&mut progress, "Second", false).unwrap();
        assert!(progress.completed.is_empty());
        tick(&mut progress, "waypoint");
        chars.select(&mut progress, "First", false).unwrap();
        assert!(progress.completed.contains_key("boss"));
        assert!(!progress.completed.contains_key("waypoint"));
        chars.remember(&progress);
        let bytes = serde_json::to_vec(&SavedProgress { progress, characters: chars }).unwrap();
        let mut saved = load(&bytes).unwrap();
        saved.characters.select(&mut saved.progress, "Second", false).unwrap();
        assert!(saved.progress.completed.contains_key("waypoint"));
        assert!(!saved.progress.completed.contains_key("boss"));
    }

    #[test]
    fn a_new_login_requires_a_character_and_preserves_each_profile() {
        let mut chars = Characters::default();
        let mut progress = Progress::default();
        chars.new_session(&mut progress, Some("login-1".into()));
        chars.select(&mut progress, "First", false).unwrap();
        tick(&mut progress, "boss");
        assert!(!chars.new_session(&mut progress, Some("login-1".into())));
        assert!(chars.new_session(&mut progress, Some("login-2".into())));
        assert!(chars.active.is_none());
        assert!(progress.completed.is_empty());
        tick(&mut progress, "entry");
        chars.select(&mut progress, "Second", false).unwrap();
        assert!(progress.completed.contains_key("entry"));
        assert!(!progress.completed.contains_key("boss"));
        chars.select(&mut progress, "First", false).unwrap();
        assert!(progress.completed.contains_key("boss"));
    }

    #[test]
    fn old_shared_progress_is_preserved_without_guessing_its_character() {
        let mut old = Progress::default();
        tick(&mut old, "boss");
        let mut saved = load(&serde_json::to_vec(&old).unwrap()).unwrap();
        assert!(saved.progress.completed.is_empty());
        assert!(saved.characters.legacy.is_some());
        saved.characters.select(&mut saved.progress, "New", false).unwrap();
        assert!(saved.progress.completed.is_empty());
        saved.characters.select(&mut saved.progress, "Old", true).unwrap();
        assert!(saved.progress.completed.contains_key("boss"));
        assert!(saved.characters.legacy.is_none());
    }

    #[test]
    fn pending_log_events_respect_the_selected_characters_manual_corrections() {
        let mut chars = Characters::default();
        let mut progress = Progress::default();
        chars.new_session(&mut progress, Some("login-1".into()));
        chars.select(&mut progress, "First", false).unwrap();
        progress.dismissed.insert("boss".into());
        chars.new_session(&mut progress, Some("login-2".into()));
        tick(&mut progress, "boss");
        progress.log_path = Some("Client.txt".into());
        progress.log_offset = Some(1234);
        chars.select(&mut progress, "First", false).unwrap();
        assert!(!progress.completed.contains_key("boss"));
        assert!(progress.dismissed.contains("boss"));
        chars.select(&mut progress, "Second", false).unwrap();
        assert_eq!(progress.log_offset, Some(1234), "switching profiles must not rewind the shared log reader");
        assert_eq!(progress.log_path.as_deref(), Some("Client.txt"));
    }
}
