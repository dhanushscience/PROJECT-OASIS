//! The simple settings, each described once: its label, where `profiles.json` keeps it, its
//! limits, and its default. The settings screen draws itself from [`sections`] and saves through
//! [`set_field`], so adding a setting is one line here and nothing else, and every value is
//! checked against the same limits the C# window's dials and lists had.
//!
//! Settings that are lists of things (profiles, buttons, sets) are edited by `edit.rs`; this is for
//! the switches, numbers and choices.

use serde::Serialize;
use serde_json::{json, Number, Value};

use crate::edit::EditError;
use crate::model::Settings;
use crate::overlay::css_color;

/// Things the registry cannot know by itself: the microphones this computer has.
#[derive(Debug, Clone, Default)]
pub struct FieldContext {
    pub microphones: Vec<String>,
    /// The speakers this computer has, for the start and end sounds.
    pub speakers: Vec<String>,
    /// Web browsers installed on this PC: `(process name, name to show)`.
    pub browsers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChoiceView {
    pub value: Value,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FieldView {
    pub id: String,
    pub label: String,
    pub hint: String,
    /// `toggle`, `number`, `choice` or `color`.
    pub kind: &'static str,
    pub value: Value,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub options: Vec<ChoiceView>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SectionView {
    pub id: String,
    pub title: String,
    pub hint: String,
    pub fields: Vec<FieldView>,
}

#[derive(Clone)]
enum Kind {
    Toggle,
    Number { min: f64, max: f64, step: f64, integer: bool },
    Choice(Vec<(Value, String)>),
    Color,
    /// Lines of `spoken = written`, each side at most `max` characters.
    Glossary { max: usize },
}

/// Where a setting lives.
#[derive(Clone, Copy)]
enum Place {
    /// `ShowOverlayLabels`, which the settings model keeps as a typed field.
    ShowLabels,
    /// `AllowRunScriptActions`, also a typed field.
    AllowScripts,
    /// A key at the top of the file.
    Root(&'static str),
    /// A key inside a section (`Voice.Enabled`).
    In(&'static str, &'static str),
    /// Push-to-talk buttons: `Voice.PushToTalkMask` and `Voice.Mode` together.
    TalkButtons,
    /// A list of `{ Spoken, Canonical }` under this key of `Voice`.
    Glossary(&'static str),
}

struct Spec {
    id: &'static str,
    section: &'static str,
    label: &'static str,
    hint: &'static str,
    place: Place,
    default: Value,
    kind: Kind,
}

// The field table and the glossary box, one file each; `use super::*` gives them the types above.
mod glossary;
mod table;

pub use glossary::add_glossary_word;
use self::{glossary::*, table::*};

/// A stored number as plain JSON: whole numbers are integers, so 350.0 equals 350.
fn tidy(value: &Value) -> Value {
    match value.as_f64() {
        Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => json!(f as i64),
        Some(f) => Number::from_f64((f * 1000.0).round() / 1000.0).map_or(value.clone(), Value::Number),
        None => value.clone(),
    }
}

fn stored(settings: &Settings, place: Place) -> Option<&Value> {
    match place {
        Place::Root(key) => settings.extra.get(key),
        Place::In(section, key) => settings.extra.get(section)?.get(key),
        Place::ShowLabels | Place::AllowScripts | Place::TalkButtons | Place::Glossary(_) => None,
    }
}

fn current(settings: &Settings, spec: &Spec) -> Value {
    match spec.place {
        Place::ShowLabels => json!(settings.show_overlay_labels),
        Place::AllowScripts => json!(settings.allow_run_script_actions),
        Place::TalkButtons => json!(mask_key(&read_mask(settings))),
        Place::Glossary(key) => json!(glossary_text(settings, key)),
        place => stored(settings, place).filter(|v| !v.is_null()).map_or_else(|| spec.default.clone(), tidy),
    }
}

/// The screen: every section and field with its current value.
pub fn sections(settings: &Settings, ctx: &FieldContext) -> Vec<SectionView> {
    let specs = specs(ctx, settings);
    SECTIONS
        .iter()
        .map(|(id, title, hint)| SectionView {
            id: (*id).to_string(),
            title: (*title).to_string(),
            hint: (*hint).to_string(),
            fields: specs.iter().filter(|s| s.section == *id).map(|s| view(settings, s)).collect(),
        })
        .collect()
}

fn view(settings: &Settings, spec: &Spec) -> FieldView {
    let value = current(settings, spec);
    let mut field = FieldView {
        id: spec.id.to_string(),
        label: spec.label.to_string(),
        hint: spec.hint.to_string(),
        kind: "toggle",
        value: value.clone(),
        min: None,
        max: None,
        step: None,
        options: Vec::new(),
    };
    match &spec.kind {
        Kind::Toggle => {}
        Kind::Number { min, max, step, .. } => {
            field.kind = "number";
            (field.min, field.max, field.step) = (Some(*min), Some(*max), Some(*step));
        }
        Kind::Choice(options) => {
            field.kind = "choice";
            field.options = options.iter().map(|(v, l)| ChoiceView { value: v.clone(), label: l.clone() }).collect();
            // A value saved by hand or by the C# app that is not in the list stays visible.
            if !field.options.iter().any(|o| o.value == value) {
                let label = if matches!(spec.place, Place::TalkButtons) {
                    format!("Custom ({})", mask_key(&read_mask(settings)))
                } else {
                    format!("Custom ({})", value.to_string().trim_matches('"'))
                };
                field.options.push(ChoiceView { value, label });
            }
        }
        Kind::Glossary { .. } => field.kind = "glossary",
        Kind::Color => {
            field.kind = "color";
            field.options = THEME_COLORS.iter().map(|(l, hex)| ChoiceView { value: json!(hex), label: (*l).to_string() }).collect();
            let held = value.as_str().map(|s| css_color(s, "#161D1F")).unwrap_or_default();
            match field.options.iter().find(|o| o.value.as_str().is_some_and(|h| h.eq_ignore_ascii_case(&held))) {
                Some(found) => field.value = found.value.clone(),
                None => field.options.push(ChoiceView { value: json!(held), label: format!("Custom ({held})") }),
            }
        }
    }
    field
}

fn bad<T>(why: impl Into<String>) -> Result<T, EditError> {
    Err(EditError::Invalid(why.into()))
}

fn put(settings: &mut Settings, place: Place, value: Value) {
    match place {
        Place::Root(key) => {
            settings.extra.insert(key.to_string(), value);
        }
        Place::In(section, key) => {
            let entry = settings.extra.entry(section.to_string()).or_insert_with(|| Value::Object(Default::default()));
            if !entry.is_object() {
                *entry = Value::Object(Default::default());
            }
            if let Some(map) = entry.as_object_mut() {
                map.insert(key.to_string(), value);
            }
        }
        Place::ShowLabels | Place::AllowScripts | Place::TalkButtons | Place::Glossary(_) => {}
    }
}

/// Change one setting. Checks the value against the limits of that setting first, and refuses
/// anything else with a sentence for the person editing; nothing changes on a refusal.
pub fn set_field(settings: &mut Settings, id: &str, value: Value, ctx: &FieldContext) -> Result<(), EditError> {
    let specs = specs(ctx, settings);
    let Some(spec) = specs.iter().find(|s| s.id == id) else { return bad(format!("There is no setting called \"{id}\".")) };

    let checked = match &spec.kind {
        Kind::Toggle => match value.as_bool() {
            Some(b) => json!(b),
            None => return bad(format!("{} must be on or off.", spec.label)),
        },
        Kind::Number { min, max, integer, .. } => {
            let Some(n) = value.as_f64().filter(|n| n.is_finite()) else {
                return bad(format!("{} must be a number.", spec.label));
            };
            if n < *min || n > *max {
                return bad(format!("{} must be between {} and {}.", spec.label, min, max));
            }
            if *integer { json!(n.round() as i64) } else { json!((n * 100.0).round() / 100.0) }
        }
        Kind::Choice(options) => {
            let wanted = tidy(&value);
            if !options.iter().any(|(v, _)| *v == wanted) {
                return bad(format!("That is not one of the choices for {}.", spec.label));
            }
            wanted
        }
        Kind::Color => match value.as_str() {
            Some(hex) if THEME_COLORS.iter().any(|(_, c)| c.eq_ignore_ascii_case(hex)) => json!(hex.to_ascii_uppercase()),
            _ => return bad(format!("{} must be one of the listed colours.", spec.label)),
        },
        Kind::Glossary { max } => match value.as_str() {
            Some(text) => glossary_rows(text, spec.label, *max, matches!(spec.place, Place::Glossary("DictationGlossary")))?,
            None => return bad(format!("{} must be text.", spec.label)),
        },
    };

    match spec.place {
        Place::ShowLabels => settings.show_overlay_labels = checked.as_bool().unwrap_or(true),
        Place::AllowScripts => settings.allow_run_script_actions = checked.as_bool().unwrap_or(false),
        Place::TalkButtons => {
            let key = checked.as_str().unwrap_or("0").to_string();
            let Some((_, mask)) = TALK_PRESETS.iter().find(|(_, m)| mask_key(m) == key) else {
                return bad("That is not one of the button choices.");
            };
            put(settings, Place::In("Voice", "PushToTalkMask"), json!(mask));
            // 0 is a single button held, 1 a chord of several.
            put(settings, Place::In("Voice", "Mode"), json!(i64::from(mask.len() > 1)));
        }
        Place::Glossary(key) => put(settings, Place::In("Voice", key), checked),
        place => put(settings, place, checked),
    }
    // Choosing where it sits (even the same place again) drops a spot it was dragged to.
    if id == "overlay.dock" {
        settings.overlay_x = (-1).into();
        settings.overlay_y = (-1).into();
    }
    Ok(())
}

/// Put the overlay's colours and sizes back to the defaults.
pub fn reset_overlay_look(settings: &mut Settings) {
    settings.extra.remove("OverlayAppearance");
}

/// Things worth saying about the current choices, without refusing them.
pub fn warnings(settings: &Settings) -> Vec<String> {
    let ctx = FieldContext::default();
    let value = |id: &str| -> Option<i64> {
        let specs = specs(&ctx, settings);
        let spec = specs.iter().find(|s| s.id == id)?;
        current(settings, spec).as_i64().filter(|i| *i >= 0)
    };
    let hold = value("youtube.hold2x_button");
    let mut notes = Vec::new();
    if let (Some(desktop), Some(hold)) = (value("pad.show_desktop_button"), hold) {
        if desktop == hold {
            notes.push("The show-desktop button is also the 2x-speed button: in a video it holds for 2x, elsewhere it shows the desktop.".to_string());
        }
    }
    notes
}


/// The settings and the read and write shortcuts the tests of both files use.
#[cfg(test)]
mod test_settings {
    use super::*;

    pub(super) fn real_like() -> Settings {
        serde_json::from_value(json!({
            "ShowOverlayLabels": true, "BrowserBridgeEnabled": true, "BrowserBridgePort": 17322,
            "YouTube": { "AutoSkipAds": true, "GesturesEnabled": true, "Hold2xButtonIndex": 2,
                         "VolumeUpButtonIndex": 1, "VolumeDownButtonIndex": 0 },
            "PadGestures": { "ShowDesktopButtonIndex": 2, "ShowDesktopHoldMs": 1000 },
            "OverlayAppearance": { "IndicatorHeight": 50, "LabelFontSize": 14, "IndicatorColor1": "#6E8090", "LabelTextColor": "#161d1f" },
            "Voice": { "Enabled": true, "PushToTalkMask": [0], "PushToTalkHoldMs": 350, "MicrophoneDeviceName": "Microphone (Logi C270 HD WebCam",
                       "IntentApprovalMode": 2, "MatchThreshold": 0.72 }
        }))
        .unwrap()
    }

    pub(super) fn field(settings: &Settings, id: &str) -> FieldView {
        let ctx = FieldContext { microphones: vec!["Microphone (Realtek(R) Audio)".into()], ..Default::default() };
        sections(settings, &ctx).into_iter().flat_map(|s| s.fields).find(|f| f.id == id).expect(id)
    }

    pub(super) fn set(settings: &mut Settings, id: &str, value: Value) -> Result<(), EditError> {
        set_field(settings, id, value, &FieldContext::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::test_settings::*;

    #[test]
    fn every_section_has_fields_and_every_field_a_unique_id() {
        let view = sections(&Settings::default(), &FieldContext::default());
        assert_eq!(view.len(), 7);
        let mut ids: Vec<&str> = view.iter().flat_map(|s| s.fields.iter().map(|f| f.id.as_str())).collect();
        assert!(view.iter().all(|s| !s.fields.is_empty()), "{view:?}");
        let before = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate ids");
    }

    #[test]
    fn only_installed_browsers_are_offered_and_chrome_is_the_default_when_there() {
        let browser = |ctx: &FieldContext| sections(&Settings::default(), ctx).into_iter().flat_map(|s| s.fields).find(|f| f.id == "preferences.browser").unwrap();
        let both = FieldContext { browsers: vec![("chrome".into(), "Google Chrome".into()), ("firefox".into(), "Firefox".into())], ..Default::default() };
        let view = serde_json::to_value(browser(&both)).unwrap();
        assert_eq!(view["value"], json!("chrome"), "{view}");
        assert_eq!(view["options"].as_array().map(Vec::len), Some(3), "system default + the two found: {view}");
        let no_chrome = FieldContext { browsers: vec![("firefox".into(), "Firefox".into())], ..Default::default() };
        assert_eq!(serde_json::to_value(browser(&no_chrome)).unwrap()["value"], json!(""), "the system's own browser");
    }

    #[test]
    fn a_blank_settings_file_shows_the_csharp_defaults() {
        let s = Settings::default();
        assert_eq!(field(&s, "youtube.hold2x_button").value, json!(1));
        assert_eq!(field(&s, "voice.enabled").value, json!(false));
        assert_eq!(field(&s, "overlay.indicator_height").value, json!(50));
        assert_eq!(field(&s, "general.show_labels").value, json!(true));
    }

    #[test]
    fn the_owners_real_values_are_shown() {
        let s = real_like();
        assert_eq!(field(&s, "youtube.hold2x_button").value, json!(2));
        assert_eq!(field(&s, "pad.show_desktop_hold_ms").value, json!(1000));
        assert_eq!(field(&s, "voice.approval_mode").value, json!(2));
        assert_eq!(field(&s, "voice.talk_buttons").value, json!("0"));
        assert_eq!(field(&s, "overlay.label_text").value, json!("#161D1F"), "matched without regard to case");
    }

    #[test]
    fn a_microphone_that_is_not_plugged_in_stays_visible() {
        let mic = field(&real_like(), "voice.microphone");
        assert_eq!(mic.value, json!("Microphone (Logi C270 HD WebCam"));
        assert!(mic.options.iter().any(|o| o.label.ends_with("(not connected)")), "{:?}", mic.options);
        assert!(mic.options.iter().any(|o| o.label == "System default"));
        assert!(mic.options.iter().any(|o| o.label == "Microphone (Realtek(R) Audio)"));
    }

    #[test]
    fn switches_and_choices_are_saved_where_the_csharp_app_keeps_them() {
        let mut s = real_like();
        set(&mut s, "youtube.gestures", json!(false)).unwrap();
        set(&mut s, "youtube.hold2x_button", json!(1)).unwrap();
        set(&mut s, "general.bridge_enabled", json!(false)).unwrap();
        set(&mut s, "voice.capture_mode", json!(2)).unwrap();
        assert_eq!(s.extra["YouTube"]["GesturesEnabled"], json!(false));
        assert_eq!(s.extra["YouTube"]["Hold2xButtonIndex"], json!(1));
        assert_eq!(s.extra["YouTube"]["AutoSkipAds"], json!(true), "the rest of the section is untouched");
        assert_eq!(s.extra["BrowserBridgeEnabled"], json!(false));
        assert_eq!(s.extra["Voice"]["CaptureMode"], json!(2));
    }

    #[test]
    fn show_labels_is_the_typed_field() {
        let mut s = Settings::default();
        set(&mut s, "general.show_labels", json!(false)).unwrap();
        assert!(!s.show_overlay_labels);
    }

    #[test]
    fn scripts_are_off_until_switched_on() {
        let mut s = Settings::default();
        assert_eq!(field(&s, "general.allow_scripts").value, json!(false));
        set(&mut s, "general.allow_scripts", json!(true)).unwrap();
        assert!(s.allow_run_script_actions);
    }

    #[test]
    fn numbers_are_held_to_the_limits_of_the_c_sharp_dials() {
        let mut s = Settings::default();
        assert!(set(&mut s, "overlay.indicator_width", json!(12)).is_ok());
        assert_eq!(s.extra["OverlayAppearance"]["IndicatorWidth"], json!(12));
        for (id, bad) in [("overlay.indicator_width", 3), ("overlay.indicator_width", 21), ("overlay.label_font_size", 11), ("general.bridge_port", 80), ("general.bridge_port", 70000)] {
            assert!(matches!(set(&mut s, id, json!(bad)), Err(EditError::Invalid(_))), "{id} {bad}");
        }
        assert!(set(&mut s, "overlay.indicator_opacity", json!(0.1)).is_err(), "never nearly invisible");
        assert!(set(&mut s, "overlay.indicator_opacity", json!("high")).is_err());
        assert!(set(&mut s, "overlay.indicator_opacity", json!(0.55)).is_ok());
    }

    #[test]
    fn a_refused_change_changes_nothing() {
        let mut s = real_like();
        let before = s.clone();
        assert!(set(&mut s, "youtube.hold2x_button", json!(7)).is_err());
        assert!(set(&mut s, "youtube.gestures", json!("yes")).is_err());
        assert!(set(&mut s, "no.such.setting", json!(1)).is_err());
        assert!(set(&mut s, "overlay.label_text", json!("red; background:url(x)")).is_err());
        assert_eq!(s, before);
    }

    #[test]
    fn colours_must_be_from_the_list() {
        let mut s = Settings::default();
        assert!(set(&mut s, "overlay.label_text", json!("#ffffff")).is_ok());
        assert_eq!(s.extra["OverlayAppearance"]["LabelTextColor"], json!("#FFFFFF"));
        assert!(set(&mut s, "overlay.label_text", json!("#123456")).is_err());
    }

    #[test]
    fn a_colour_set_by_hand_is_still_shown() {
        let s: Settings = serde_json::from_value(json!({ "OverlayAppearance": { "LabelBorderColor": "#80123456" } })).unwrap();
        let f = field(&s, "overlay.label_border");
        assert!(f.options.iter().any(|o| o.label.starts_with("Custom")), "{:?}", f.options);
    }

    #[test]
    fn the_talk_buttons_set_the_mask_and_the_mode_together() {
        let mut s = real_like();
        set(&mut s, "voice.talk_buttons", json!("0+2")).unwrap();
        assert_eq!(s.extra["Voice"]["PushToTalkMask"], json!([0, 2]));
        assert_eq!(s.extra["Voice"]["Mode"], json!(1));
        set(&mut s, "voice.talk_buttons", json!("1")).unwrap();
        assert_eq!(s.extra["Voice"]["PushToTalkMask"], json!([1]));
        assert_eq!(s.extra["Voice"]["Mode"], json!(0));
        assert!(set(&mut s, "voice.talk_buttons", json!("0+1+2")).is_err());
    }

    #[test]
    fn a_hand_made_talk_combination_stays_selectable() {
        let s: Settings = serde_json::from_value(json!({ "Voice": { "PushToTalkMask": [0, 1, 2] } })).unwrap();
        let f = field(&s, "voice.talk_buttons");
        assert_eq!(f.value, json!("0+1+2"));
        assert!(f.options.iter().any(|o| o.label == "Custom (0+1+2)"));
    }

    #[test]
    fn the_editing_commands_switch_is_on_by_default_and_read_by_the_router() {
        let mut s = Settings::default();
        let f = field(&s, "voice.edit_commands");
        assert_eq!((f.value, f.kind), (json!(true), "toggle"));
        set(&mut s, "voice.edit_commands", json!(false)).unwrap();
        assert_eq!(s.extra["Voice"]["EditCommands"], json!(false));
        assert!(!crate::voice_commands::router_settings(&s).edit_commands);
    }

    #[test]
    fn the_overlay_can_be_reset_to_the_defaults() {
        let mut s = real_like();
        reset_overlay_look(&mut s);
        assert!(!s.extra.contains_key("OverlayAppearance"));
        assert_eq!(field(&s, "overlay.indicator_height").value, json!(50));
    }

    #[test]
    fn the_overlay_module_reads_what_this_screen_writes() {
        let mut s = Settings::default();
        set(&mut s, "overlay.indicator_width", json!(11)).unwrap();
        set(&mut s, "overlay.indicator_color_1", json!("#a0a0a0")).unwrap();
        let look = crate::overlay::overlay_appearance(&s);
        assert_eq!(look.indicator_width, 11.0);
        assert_eq!(look.indicator_colors[0], "#A0A0A0");
    }

    #[test]
    fn clashing_buttons_get_a_note_but_are_allowed() {
        let mut s = real_like();
        assert!(warnings(&s).iter().any(|w| w.contains("show-desktop")), "your layout shares button 3: {:?}", warnings(&s));
        set(&mut s, "youtube.hold2x_button", json!(0)).unwrap();
        assert!(warnings(&s).is_empty(), "no clash once they differ: {:?}", warnings(&s));
    }
}
