//! The settings behind spoken commands: how a recording is used (typed, or run as a command), the
//! vocabulary built from the user's profiles, and the targets already approved by voice. Port of
//! the settings side of `VoiceSettings`, `VoiceLexicon.Rebuild` and `IntentTrustedTargetStore`.

use std::collections::HashSet;

use oasis_core::voice::command::safety::{is_first_use_sensitive, sensitive_target_keys, target_key};
use oasis_core::voice::command::{
    build_lexicon, ApprovalMode, LabeledAction, LexiconEntry, LexiconInputs, LexiconProfile, RouterSettings,
    RunningApp,
};
use serde_json::Value;

use crate::model::{Button, Settings};

/// What a push-to-talk recording becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    /// Typed when a text field is focused (or the app is on the dictate-first list), else a command.
    Auto,
    /// Always a command.
    Command,
    /// Always typed.
    Dictate,
}

/// Apps that dictate first even when no text field can be detected: chat apps built on WebView2
/// or Electron show UI Automation nothing editable.
const DEFAULT_DICTATE_FIRST: [&str; 9] =
    ["WhatsApp", "WhatsApp.Root", "Telegram", "Signal", "Slack", "Discord", "ms-teams", "Teams", "Messenger"];

fn voice(settings: &Settings) -> Option<&Value> {
    settings.extra.get("Voice").filter(|v| v.is_object())
}

pub fn capture_mode(settings: &Settings) -> CaptureMode {
    match voice(settings).and_then(|v| v.get("CaptureMode")).and_then(Value::as_i64) {
        Some(1) => CaptureMode::Command,
        Some(2) => CaptureMode::Dictate,
        _ => CaptureMode::Auto,
    }
}

pub fn dictate_first_apps(settings: &Settings) -> Vec<String> {
    voice(settings)
        .and_then(|v| v.get("DictateFirstApps"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_else(|| DEFAULT_DICTATE_FIRST.iter().map(|s| (*s).to_string()).collect())
}

/// Whether the process is on the dictate-first list (names match without regard to case).
pub fn is_dictate_first(settings: &Settings, process: &str) -> bool {
    dictate_first_apps(settings).iter().any(|app| app.eq_ignore_ascii_case(process.trim()))
}

pub fn router_settings(settings: &Settings) -> RouterSettings {
    let defaults = RouterSettings::default();
    let section = voice(settings);
    RouterSettings {
        match_threshold: section
            .and_then(|v| v.get("MatchThreshold"))
            .and_then(Value::as_f64)
            .filter(|t| (0.0..=1.0).contains(t))
            .unwrap_or(defaults.match_threshold),
        allow_launch_app: section.and_then(|v| v.get("AllowLaunchApp")).and_then(Value::as_bool).unwrap_or(true),
        allow_type_text: section.and_then(|v| v.get("AllowTypeText")).and_then(Value::as_bool).unwrap_or(true),
        now: None,
        quick_launch: crate::quick_launch::voice_refs(settings),
        spotify_running: false,
        running: Vec::new(),
        installed: None,
        edit_commands: section.and_then(|v| v.get("EditCommands")).and_then(Value::as_bool).unwrap_or(true),
        on_screen: Vec::new(),
        app_names: Vec::new(),
    }
}

pub fn approval_mode(settings: &Settings) -> ApprovalMode {
    ApprovalMode::from_number(voice(settings).and_then(|v| v.get("IntentApprovalMode")).and_then(Value::as_i64).unwrap_or(0))
}

fn labeled(button: &Button) -> LabeledAction {
    LabeledAction { label: button.label.clone(), action: button.to_action() }
}

/// The vocabulary for spoken commands: the buttons of the set in use, every profile's name and
/// buttons, well-known apps, and the programs that are running.
pub fn lexicon(settings: &Settings, active: Option<(usize, usize)>, running: &[RunningApp]) -> Vec<LexiconEntry> {
    let profiles: Vec<LexiconProfile> = settings
        .profiles
        .iter()
        .map(|p| LexiconProfile {
            name: p.name.clone(),
            buttons: p.config_profiles.iter().flat_map(|c| c.buttons.iter().map(labeled)).collect(),
        })
        .collect();
    let active_buttons: Option<(&str, Vec<LabeledAction>)> = active.and_then(|(profile, config)| {
        let p = settings.profiles.get(profile)?;
        let c = p.config_profiles.get(config)?;
        Some((p.name.as_str(), c.buttons.iter().map(labeled).collect()))
    });
    build_lexicon(&LexiconInputs {
        active: active_buttons.as_ref().map(|(name, buttons)| (*name, buttons.as_slice())),
        profiles: &profiles,
        running,
    })
}

/// Targets that need no approval: those approved by voice before, and everything the user put on
/// a pad button themselves.
pub fn trusted_targets(settings: &Settings) -> HashSet<String> {
    let mut set: HashSet<String> = voice(settings)
        .and_then(|v| v.get("IntentTrustedTargets"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    for profile in &settings.profiles {
        for config in &profile.config_profiles {
            for button in &config.buttons {
                if is_first_use_sensitive(&button.kind) {
                    set.insert(target_key(&button.kind, &button.value));
                }
                for step in button.automation_steps() {
                    if is_first_use_sensitive(&step.kind) {
                        set.insert(target_key(&step.kind, &step.value));
                    }
                }
            }
        }
    }
    set
}

impl Settings {
    /// Remember targets approved by voice, so the same app or link does not ask again. Returns
    /// whether anything new was added.
    pub fn trust_voice_targets(&mut self, steps: &[(String, String)]) -> bool {
        let keys = sensitive_target_keys(steps);
        let entry = self.extra.entry("Voice".to_string()).or_insert_with(|| Value::Object(Default::default()));
        let Some(section) = entry.as_object_mut() else { return false };
        let list = section.entry("IntentTrustedTargets").or_insert_with(|| Value::Array(Vec::new()));
        let Some(list) = list.as_array_mut() else { return false };
        let mut added = false;
        for key in keys {
            if list.iter().any(|existing| existing.as_str().is_some_and(|e| e.eq_ignore_ascii_case(&key))) {
                continue;
            }
            list.push(Value::String(key));
            added = true;
        }
        added
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings(json: Value) -> Settings {
        serde_json::from_value(json).unwrap()
    }

    fn with_profiles() -> Settings {
        settings(json!({
            "Voice": { "IntentApprovalMode": 2, "MatchThreshold": 0.8, "AllowLaunchApp": false,
                       "CaptureMode": 1, "DictateFirstApps": ["Slack"],
                       "IntentTrustedTargets": ["launchapp:spotify"] },
            "Profiles": [
                { "Name": "chrome", "ContextMatch": "chrome", "Kind": "app", "ConfigProfiles": [
                    { "Name": "chrome", "ContextMatch": "*", "Buttons": [
                        { "Type": "Shortcut", "Value": "Ctrl+T", "Label": "NEW TAB", "Steps": "" },
                        { "Type": "LaunchApp", "Value": "C:\\Apps\\Code.exe", "Label": "EDITOR", "Steps": "" },
                        { "Type": "OpenUrl", "Value": "https://example.com/", "Label": "", "Steps": "" } ] } ] }
            ]
        }))
    }

    #[test]
    fn the_defaults_match_csharp() {
        let s = Settings::default();
        assert_eq!(capture_mode(&s), CaptureMode::Auto);
        assert!(is_dictate_first(&s, "whatsapp") && is_dictate_first(&s, "Teams") && !is_dictate_first(&s, "notepad"));
        let r = router_settings(&s);
        assert_eq!((r.match_threshold, r.allow_launch_app, r.allow_type_text), (0.72, true, true));
        assert_eq!(approval_mode(&s), ApprovalMode::FirstUse);
    }

    #[test]
    fn saved_voice_settings_are_read() {
        let s = with_profiles();
        assert_eq!(capture_mode(&s), CaptureMode::Command);
        assert_eq!(dictate_first_apps(&s), ["Slack"]);
        let r = router_settings(&s);
        assert_eq!((r.match_threshold, r.allow_launch_app), (0.8, false));
        assert_eq!(approval_mode(&s), ApprovalMode::RiskyOnly);
    }

    #[test]
    fn editing_commands_are_on_unless_switched_off() {
        assert!(router_settings(&Settings::default()).edit_commands, "absent: on");
        assert!(router_settings(&settings(json!({ "Voice": { "AllowTypeText": false } }))).edit_commands);
        assert!(!router_settings(&settings(json!({ "Voice": { "EditCommands": false } }))).edit_commands);
        assert!(router_settings(&settings(json!({ "Voice": { "EditCommands": true } }))).edit_commands);
    }

    #[test]
    fn a_threshold_outside_zero_to_one_is_ignored() {
        assert_eq!(router_settings(&settings(json!({ "Voice": { "MatchThreshold": 7 } }))).match_threshold, 0.72);
        assert_eq!(router_settings(&settings(json!({ "Voice": { "MatchThreshold": -1 } }))).match_threshold, 0.72);
    }

    #[test]
    fn the_lexicon_holds_the_users_buttons_and_prefers_the_active_set() {
        let s = with_profiles();
        let entries = lexicon(&s, Some((0, 0)), &[]);
        let tab = entries.iter().find(|e| e.normalized == "new tab").expect("a button label");
        assert!(tab.prefer_active);
        assert_eq!(entries.iter().filter(|e| e.normalized == "new tab").count(), 1, "listed once, not once per pass");
        assert!(entries.iter().any(|e| e.normalized == "chrome"), "the profile name");
        assert!(entries.iter().all(|e| !e.normalized.is_empty()), "an unlabeled button adds nothing");
    }

    #[test]
    fn what_the_user_put_on_a_button_or_approved_before_is_trusted() {
        let trusted = trusted_targets(&with_profiles());
        assert!(trusted.contains("launchapp:code"), "{trusted:?}");
        assert!(trusted.contains("openurl:https://example.com"), "{trusted:?}");
        assert!(trusted.contains("launchapp:spotify"), "approved by voice before");
        assert!(!trusted.contains("launchapp:notepad"));
    }

    #[test]
    fn approving_a_target_is_remembered_once_and_the_rest_of_the_section_is_kept() {
        let mut s = with_profiles();
        let steps = vec![("LaunchApp".to_string(), "C:\\Apps\\Notepad.exe".to_string()), ("Shortcut".to_string(), "Ctrl+T".to_string())];
        assert!(s.trust_voice_targets(&steps));
        assert!(!s.trust_voice_targets(&steps), "the second time adds nothing");
        assert!(trusted_targets(&s).contains("launchapp:notepad"));
        let voice = s.extra.get("Voice").unwrap();
        assert_eq!(voice["MatchThreshold"], 0.8, "other voice settings are untouched");
        assert_eq!(voice["IntentTrustedTargets"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_settings_file_with_no_voice_section_gets_one_when_a_target_is_approved() {
        let mut s = Settings::default();
        assert!(s.trust_voice_targets(&[("OpenUrl".to_string(), "https://a.test".to_string())]));
        assert!(trusted_targets(&s).contains("openurl:https://a.test"));
    }
}
