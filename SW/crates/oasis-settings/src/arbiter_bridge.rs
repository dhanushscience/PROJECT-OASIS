//! Port of `Services/ArbiterSettingsBridge.cs`: the gesture settings in `profiles.json`
//! (`YouTube`, `Voice`, `Ai`, `PadGestures`, `ShowOverlayLabels`) become the arbiter's config.
//!
//! Those sections are read here by name from the raw JSON (`settings.extra`), with the C# defaults
//! for anything missing. Double-click for volume on YouTube is switched off whatever the file says.

use std::time::Duration;

use oasis_core::input::ArbiterConfig;
use oasis_core::{ButtonId, ButtonMask};
use serde_json::Value;

use crate::model::Settings;

/// `voice_model_present` is the answer to "is a speech model installed" (C# asked
/// `SpeechRecognizer.IsModelPresent`); voice push-to-talk only arms when it is.
pub fn arbiter_config(settings: &Settings, voice_model_present: bool) -> ArbiterConfig {
    let section = |name: &str| settings.extra.get(name).filter(|v| v.is_object());
    let (yt, voice, ai, pad) =
        (section("YouTube"), section("Voice"), section("Ai"), section("PadGestures"));

    let triggers = ai.and_then(|a| a.get("Triggers")).and_then(Value::as_array);
    let first_trigger = triggers.and_then(|t| t.first());
    let ai_enabled = flag(ai, "Enabled", false) && triggers.is_some_and(|t| !t.is_empty());

    let mut ai_hold = int(ai, "HoldDurationMs", 3000);
    if let Some(own) = first_trigger.and_then(|t| t.get("HoldDurationMs")).and_then(Value::as_i64) {
        if own > 0 {
            ai_hold = own;
        }
    }

    let push_to_talk = indices(voice.and_then(|v| v.get("PushToTalkMask")), &[0]);
    let voice_enabled = flag(voice, "Enabled", false) && !push_to_talk.is_empty() && voice_model_present;

    ArbiterConfig {
        double_click_window: millis(int(yt, "DoubleClickMs", 300)),
        volume_push_to_talk_floor: Duration::from_millis(700),
        hold2x_threshold: millis(int(yt, "Hold2xThresholdMs", 250)),
        push_to_talk_hold: millis(200),
        ai_hold_duration: millis(ai_hold),
        // Double-click for volume on YouTube is switched off (owner, 2026-11-18): a single tap on
        // those buttons waited for a second tap and, with labels hidden for the profile, was lost.
        // The stored `VolumeUpButtonIndex` / `VolumeDownButtonIndex` are kept but not used.
        volume_up: None,
        volume_down: None,
        hold2x: optional_button(int(yt, "Hold2xButtonIndex", 1)),
        push_to_talk_mask: mask(&push_to_talk),
        voice_enabled,
        ai_enabled,
        ai_trigger_mask: if ai_enabled {
            mask(&indices(first_trigger.and_then(|t| t.get("ButtonMask")), &[]))
        } else {
            ButtonMask::NONE
        },
        show_overlay_labels: settings.show_overlay_labels,
        show_desktop: optional_button(int(pad, "ShowDesktopButtonIndex", 2)),
        show_desktop_hold: millis(int(pad, "ShowDesktopHoldMs", 800)),
    }
}

fn int(section: Option<&Value>, key: &str, default: i64) -> i64 {
    section.and_then(|s| s.get(key)).and_then(Value::as_i64).unwrap_or(default)
}

fn flag(section: Option<&Value>, key: &str, default: bool) -> bool {
    section.and_then(|s| s.get(key)).and_then(Value::as_bool).unwrap_or(default)
}

fn millis(ms: i64) -> Duration {
    Duration::from_millis(ms.max(0) as u64)
}

fn optional_button(index: i64) -> Option<ButtonId> {
    (0..=2).contains(&index).then_some(ButtonId(index as i32))
}

/// A list of button indexes. A lone number is accepted as a one-item list.
fn indices(value: Option<&Value>, default: &[i64]) -> Vec<i64> {
    match value {
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_i64).collect(),
        Some(Value::Number(n)) => n.as_i64().into_iter().collect(),
        _ => default.to_vec(),
    }
}

fn mask(indices: &[i64]) -> ButtonMask {
    indices
        .iter()
        .filter_map(|&i| optional_button(i))
        .fold(ButtonMask::NONE, |m, b| m.with(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings(extra: Value) -> Settings {
        Settings { extra: extra.as_object().cloned().unwrap_or_default(), ..Settings::default() }
    }

    #[test]
    fn missing_sections_give_the_product_defaults() {
        let config = arbiter_config(&Settings::default(), true);
        let defaults = ArbiterConfig::product_defaults();
        assert_eq!(config.hold2x, defaults.hold2x);
        assert_eq!((config.volume_up, config.volume_down), (None, None), "double-click volume is off");
        assert_eq!(config.show_desktop, defaults.show_desktop);
        assert_eq!(config.show_desktop_hold, defaults.show_desktop_hold);
        assert_eq!(config.double_click_window, defaults.double_click_window);
        assert!(!config.voice_enabled && !config.ai_enabled);
    }

    #[test]
    fn the_owners_real_gesture_settings_map_across() {
        let s = settings(json!({
            "YouTube": { "Hold2xButtonIndex": 2, "VolumeUpButtonIndex": 1, "VolumeDownButtonIndex": 0,
                         "Hold2xThresholdMs": 250, "DoubleClickMs": 300 },
            "PadGestures": { "ShowDesktopButtonIndex": 2, "ShowDesktopHoldMs": 1000 },
            "Voice": { "Enabled": true, "PushToTalkMask": [0], "PushToTalkHoldMs": 350 },
            "Ai": { "Enabled": true, "HoldDurationMs": 3000,
                    "Triggers": [ { "ButtonMask": [1], "HoldDurationMs": -1 } ] }
        }));
        let c = arbiter_config(&s, true);
        assert_eq!(c.hold2x, Some(ButtonId::THREE));
        assert_eq!((c.volume_up, c.volume_down), (None, None), "double-click volume is off even when set");
        assert_eq!(c.show_desktop_hold, Duration::from_millis(1000));
        assert!(c.voice_enabled);
        assert_eq!(c.push_to_talk_mask, ButtonMask::ONE);
        assert!(c.ai_enabled);
        assert_eq!(c.ai_trigger_mask, ButtonMask::TWO);
        assert_eq!(c.ai_hold_duration, Duration::from_millis(3000), "a trigger hold of -1 defers to the AI setting");
    }

    #[test]
    fn voice_needs_a_speech_model_and_a_mask() {
        let s = settings(json!({ "Voice": { "Enabled": true, "PushToTalkMask": [0] } }));
        assert!(!arbiter_config(&s, false).voice_enabled);
        assert!(arbiter_config(&s, true).voice_enabled);
        let none = settings(json!({ "Voice": { "Enabled": true, "PushToTalkMask": [] } }));
        assert!(!arbiter_config(&none, true).voice_enabled);
    }

    #[test]
    fn a_trigger_can_set_its_own_hold_time() {
        let s = settings(json!({ "Ai": { "Enabled": true, "HoldDurationMs": 3000,
            "Triggers": [ { "ButtonMask": [0], "HoldDurationMs": 1500 } ] } }));
        assert_eq!(arbiter_config(&s, false).ai_hold_duration, Duration::from_millis(1500));
    }

    #[test]
    fn ai_without_a_trigger_is_off_and_a_lone_number_is_a_list() {
        let off = settings(json!({ "Ai": { "Enabled": true, "Triggers": [] } }));
        assert!(!arbiter_config(&off, false).ai_enabled);
        let lone = settings(json!({ "Ai": { "Enabled": true, "Triggers": [ { "ButtonMask": 2 } ] } }));
        assert_eq!(arbiter_config(&lone, false).ai_trigger_mask, ButtonMask::THREE);
    }

    #[test]
    fn out_of_range_buttons_switch_a_gesture_off() {
        let s = settings(json!({
            "YouTube": { "Hold2xButtonIndex": -1, "VolumeUpButtonIndex": 3 },
            "PadGestures": { "ShowDesktopButtonIndex": -1 }
        }));
        let c = arbiter_config(&s, false);
        assert_eq!((c.hold2x, c.volume_up, c.show_desktop), (None, None, None));
    }
}
