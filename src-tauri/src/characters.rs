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
    /// The character chosen last, by hand or kept (`keep_chosen`).
    pub last: Option<String>,
    /// Log events read while no character was bound, each with the area the player was in,
    /// for `tracker::replay_unbound` to judge against the profile then chosen. Before, they
    /// were judged against an empty progress: an `after` step stayed open (review of
    /// 2026-11-13, item 4). Not saved.
    #[serde(skip)]
    pub unbound: Vec<(String, crate::tracker::LogEvent, Option<String>)>,
}

/// Events kept while unbound; a login's own entry is the one that matters.
pub const UNBOUND_EVENTS: usize = 64;

/// The profile progress is kept in where no character can be recognised and none was chosen.
pub const DEFAULT_PROFILE: &str = "Default";

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

    /// Everywhere but on Windows (`identity::keeps_last`), the last character chosen stays
    /// chosen until the screen or the log names another; before any was chosen, a "Default"
    /// one holds the progress kept before profiles. Each restart, login or game exit cleared
    /// the binding, and with nothing to bind it again the ticks went into an empty, unsaved
    /// progress (the owner's 40 ticks were moved aside on 2026-11-13). Call after anything
    /// that clears the binding.
    pub fn keep_chosen(&mut self, progress: &mut Progress) {
        if crate::identity::keeps_last() {
            self.keep_chosen_anyway(progress);
        }
    }

    fn keep_chosen_anyway(&mut self, progress: &mut Progress) {
        if self.active.is_some() {
            return;
        }
        let only = (self.profiles.len() == 1).then(|| self.profiles.keys().next().cloned()).flatten();
        let name = self.last.clone().or(only).unwrap_or_else(|| DEFAULT_PROFILE.to_string());
        let import = self.legacy.is_some() && !self.profiles.keys().any(|key| key.to_lowercase() == name.to_lowercase());
        let kept = if import { self.legacy.clone() } else { self.profiles.get(&name).cloned() };
        if self.select(progress, &name, import).is_err() {
            return;
        }
        self.active_source = Some("kept".into());
        // Where the player was is the profile's too: a restart in a zone is no new visit.
        if let Some(kept) = kept.filter(|_| progress.current_area.is_none()) {
            progress.current_area = kept.current_area;
            progress.area_level = kept.area_level;
            progress.shown_zone = kept.shown_zone;
            self.remember(progress);
        }
    }

    /// The character the game's log names (a level-up or a death: `tracker::character_in_line`),
    /// the main source of who is playing on every platform (owner, 2026-11-13). It becomes the
    /// active profile. The "Default" profile (progress kept before any name was known) is taken
    /// over by the first character the log names that has no profile of its own; where the
    /// last character is kept (`identity::keeps_last`), the progress from before profiles too.
    /// True when the profile changed.
    pub fn bind_from_log(&mut self, progress: &mut Progress, name: &str) -> bool {
        let same = |other: &str| other.to_lowercase() == name.to_lowercase();
        if self.active.as_deref().is_some_and(same) {
            self.active_source = Some("log".into());
            return false;
        }
        if !self.profiles.keys().any(|key| same(key)) {
            if let Some(kept) = self.profiles.remove(DEFAULT_PROFILE) {
                self.profiles.insert(name.to_string(), kept);
                if self.last.as_deref() == Some(DEFAULT_PROFILE) {
                    self.last = Some(name.to_string());
                }
                if self.active.as_deref() == Some(DEFAULT_PROFILE) {
                    self.active = Some(name.to_string());
                    self.active_source = Some("log".into());
                    self.last = Some(name.to_string());
                    return true;
                }
            }
        }
        let import = crate::identity::keeps_last() && self.legacy.is_some() && !self.profiles.keys().any(|key| same(key));
        if self.select(progress, name, import).is_err() {
            return false;
        }
        self.active_source = Some("log".into());
        true
    }

    /// Stored progress survives a restart; a stored active name is not fresh identity.
    /// Where the player was belongs to the log, like its path: kept, so that restoring an
    /// already-read entry on start is not taken for another visit (review of 2026-11-13,
    /// item 5). `select` carries it into the chosen profile.
    pub fn clear_binding(&mut self, progress: &mut Progress) {
        self.remember(progress);
        self.active = None;
        self.active_source = None;
        self.unbound.clear();
        let old = std::mem::take(progress);
        progress.log_path = old.log_path;
        progress.log_offset = old.log_offset;
        progress.current_area = old.current_area;
        progress.area_level = old.area_level;
        progress.shown_zone = old.shown_zone;
    }

    pub fn select(&mut self, progress: &mut Progress, name: &str, import_legacy: bool) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err("Enter a character name (1–80 characters)".into());
        }
        // The same spelling up to case first; else the same up to OCR misreads.
        let same = |fold: fn(&str) -> String| -> Vec<String> {
            self.profiles.keys().filter(|key| fold(key) == fold(name)).cloned().collect()
        };
        let mut matching = same(|s| s.to_lowercase());
        if matching.is_empty() { matching = same(match_key); }
        if matching.len() > 1 { return Err("Several saved profiles match this name (letter case or similar letters); select a distinct profile name".into()); }
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
        next.area_level = progress.area_level;
        next.shown_zone = progress.shown_zone.clone();
        next.log_path = progress.log_path.clone();
        next.log_offset = progress.log_offset;
        *progress = next;
        self.active = Some(name.to_string());
        self.last = Some(name.to_string());
        self.active_source = Some("manual".into());
        self.remember(progress);
        Ok(())
    }
}

/// `matchKey` of `src/character-screen.mjs`: matching only, never a display name. Windows
/// OCR reads the game's small-caps title font inconsistently (I/l/1, O/0, É/Æ/E), so
/// "BAT0NGA" must reach a hand-made "Batonga" (review of 2026-11-13, item 8). Accents are
/// dropped from the letters Unicode decomposes into letter + mark; ø, ł, đ stay as in JS.
pub fn match_key(name: &str) -> String {
    name.trim().to_lowercase().chars().map(|c| match c {
        'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' => 'd',
        'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' | 'æ' | 'œ' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' => 'h',
        'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | '1' | '|' | 'l' | 'ĺ' | 'ļ' | 'ľ' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò'..='ö' | 'ō' | 'ŏ' | 'ő' | '0' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' => 't',
        'ù'..='ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        other => other,
    }).collect()
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
    fn without_recognition_the_progress_is_kept_across_restarts() {
        // The owner's file after the profiles came: all ticks moved aside as legacy.
        let mut legacy = Progress { current_area: Some("G1_13_2".into()), shown_zone: Some("G1_13_2".into()), ..Default::default() };
        tick(&mut legacy, "beira");
        let mut chars = Characters { legacy: Some(legacy), ..Default::default() };
        let mut progress = Progress::default();
        chars.keep_chosen_anyway(&mut progress);
        assert_eq!(chars.active.as_deref(), Some(DEFAULT_PROFILE));
        assert!(progress.completed.contains_key("beira"), "the earlier ticks are back");
        assert_eq!(progress.current_area.as_deref(), Some("G1_13_2"));
        assert!(chars.legacy.is_none());
        // A restart, a login or the game closing clears the binding: kept again, with its ticks.
        tick(&mut progress, "miller");
        chars.clear_binding(&mut progress);
        assert!(progress.completed.is_empty());
        chars.keep_chosen_anyway(&mut progress);
        assert!(progress.completed.contains_key("beira") && progress.completed.contains_key("miller"));
        assert_eq!(progress.current_area.as_deref(), Some("G1_13_2"));
        // A character chosen by hand stays the one kept.
        chars.select(&mut progress, "Hero", false).unwrap();
        tick(&mut progress, "hero-step");
        chars.clear_binding(&mut progress);
        chars.keep_chosen_anyway(&mut progress);
        assert_eq!(chars.active.as_deref(), Some("Hero"));
        assert!(progress.completed.contains_key("hero-step"));
    }

    #[test]
    fn the_log_names_who_plays_and_takes_over_the_default_profile() {
        let mut chars = Characters::default();
        let mut progress = Progress::default();
        chars.select(&mut progress, DEFAULT_PROFILE, false).unwrap();
        tick(&mut progress, "beira");
        // The first level-up names the character: Default's ticks are now its own.
        assert!(chars.bind_from_log(&mut progress, "Ranger_One"));
        assert_eq!(chars.active.as_deref(), Some("Ranger_One"));
        assert_eq!(chars.active_source.as_deref(), Some("log"));
        assert!(progress.completed.contains_key("beira"));
        assert!(!chars.profiles.contains_key(DEFAULT_PROFILE));
        // Again (another level-up): nothing changes.
        assert!(!chars.bind_from_log(&mut progress, "RANGER_ONE"));
        // Another character logs in and levels: its own, empty profile; the first keeps its ticks.
        assert!(chars.bind_from_log(&mut progress, "Witch_Two"));
        assert!(progress.completed.is_empty());
        assert!(chars.bind_from_log(&mut progress, "Ranger_One"));
        assert!(progress.completed.contains_key("beira"));
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
    fn ocr_misreads_reuse_the_profile_and_an_exact_spelling_wins() {
        let mut chars = Characters::default();
        let mut progress = Progress::default();
        chars.select(&mut progress, "Batonga", false).unwrap();
        tick(&mut progress, "boss");
        chars.clear_binding(&mut progress);
        chars.select(&mut progress, "BAT0NGA", false).unwrap();
        assert_eq!(chars.active.as_deref(), Some("Batonga"));
        assert!(progress.completed.contains_key("boss"));
        assert_eq!(match_key("ÆLIÉN"), match_key("eii\u{e9}n"));
        assert_eq!(match_key("Bjørn") == match_key("Bjorn"), false, "ø has no mark to drop, as in JS");
        // Two profiles OCR cannot tell apart are fine while the read is one of them exactly.
        chars.profiles.insert("Anna1".into(), Progress::default());
        chars.profiles.insert("Annal".into(), Progress::default());
        chars.select(&mut progress, "ANNAL", false).unwrap();
        assert_eq!(chars.active.as_deref(), Some("Annal"));
        assert!(chars.select(&mut progress, "AnnaI", false).is_err(), "not arbitrarily one of them");
    }

    #[test]
    fn clearing_the_binding_keeps_where_the_player_is() {
        let mut chars = Characters::default();
        let mut progress = Progress { current_area: Some("G1_town".into()), shown_zone: Some("G1_town".into()), ..Default::default() };
        chars.select(&mut progress, "Hero", false).unwrap();
        tick(&mut progress, "boss");
        chars.clear_binding(&mut progress); // as on every start
        assert!(progress.completed.is_empty());
        assert_eq!(progress.current_area.as_deref(), Some("G1_town"), "a restored entry is no new visit");
        assert_eq!(progress.shown_zone.as_deref(), Some("G1_town"));
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
