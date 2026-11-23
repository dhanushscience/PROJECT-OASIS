//! The voice settings this build uses (`Voice` in `profiles.json`): whether push-to-talk is on,
//! which microphone, which speech model, and how short a recording still counts. The rest of the
//! section (Groq, dictation chunking, vocabulary, …) is kept untouched.

use serde_json::Value;

use crate::model::Settings;
use crate::schema::DEFAULT_STT_MODEL_FOLDER;


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceSettings {
    pub enabled: bool,
    /// Pad buttons (0-based) that must be held together to talk.
    pub push_to_talk_buttons: Vec<i32>,
    /// A possibly shortened microphone name; empty means the default microphone.
    pub microphone: String,
    /// The speaker for the start and end sounds; empty means the system default.
    pub speaker: String,
    /// The folder under `Models\stt`.
    pub model_folder: String,
    /// Recordings shorter than this are treated as an accidental press.
    pub min_utterance_ms: u64,
    /// Whether dictation is cleaned up (fillers, numbers, punctuation words, capitals…) before it
    /// is typed. On by default, as in C#.
    pub dictation_cleanup: bool,
    /// The user's own spoken → written spellings, as `(spoken, canonical)`; blank rows are dropped.
    pub dictation_glossary: Vec<(String, String)>,
    /// What to type in place of a spoken phrase ("my mail" → an address), as `(spoken, written)`.
    /// Applied last, even with cleanup off, so the text is typed exactly as written.
    pub replacements: Vec<(String, String)>,
    /// Speech is sent to Groq to be turned into text (`SttProvider` 1). Off means on this computer.
    pub use_groq: bool,
    /// The Groq model; blank means the default.
    pub groq_model: String,
    /// When Groq fails, decode the recording on this computer instead of losing it.
    pub groq_fallback_to_local: bool,
    /// The spoken language code sent to Groq (`en`); `auto` lets Groq guess.
    pub language: String,
    /// Keep each dictation (its recording and text) on this PC, for the Insights history. Off by default.
    pub keep_history: bool,
    /// Write each dictation's raw and cleaned text to `dictation-trace.log`, for troubleshooting. Off by default.
    pub keep_trace: bool,
    /// Type the dictation where the cursor is. On by default; off only transcribes (and keeps it).
    pub paste_dictation: bool,
    /// A short sound when the mic opens and when the text is pasted. On by default.
    pub sounds: bool,
}

impl VoiceSettings {
    /// `Voice` as saved, with the C# default for anything missing.
    pub fn from(settings: &Settings) -> VoiceSettings {
        let voice = settings.extra.get("Voice").filter(|v| v.is_object());
        let text = |key: &str| voice.and_then(|v| v.get(key)).and_then(Value::as_str).map(|s| s.trim().to_string());
        VoiceSettings {
            enabled: voice.and_then(|v| v.get("Enabled")).and_then(Value::as_bool).unwrap_or(false),
            push_to_talk_buttons: voice
                .and_then(|v| v.get("PushToTalkMask"))
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_i64).filter_map(|i| i32::try_from(i).ok()).collect())
                .unwrap_or_else(|| vec![0]),
            microphone: text("MicrophoneDeviceName").unwrap_or_default(),
            speaker: text("SpeakerDeviceName").unwrap_or_default(),
            model_folder: text("ModelFileName")
                .filter(|m| !m.is_empty() && is_plain_folder_name(m))
                .unwrap_or_else(|| DEFAULT_STT_MODEL_FOLDER.to_string()),
            min_utterance_ms: voice
                .and_then(|v| v.get("MinUtteranceMs"))
                .and_then(Value::as_u64)
                .filter(|ms| *ms <= 5_000)
                .unwrap_or(300),
            dictation_cleanup: voice.and_then(|v| v.get("DictationCleanup")).and_then(Value::as_bool).unwrap_or(true),
            dictation_glossary: pairs(voice, "DictationGlossary"),
            replacements: pairs(voice, "Replacements"),
            use_groq: voice.and_then(|v| v.get("SttProvider")).and_then(Value::as_i64) == Some(1),
            groq_model: text("GroqSttModel").unwrap_or_default(),
            groq_fallback_to_local: voice.and_then(|v| v.get("GroqFallbackToLocal")).and_then(Value::as_bool).unwrap_or(true),
            language: text("Language").filter(|l| !l.is_empty()).unwrap_or_else(|| "en".to_string()),
            keep_history: voice.and_then(|v| v.get("KeepHistory")).and_then(Value::as_bool).unwrap_or(false),
            keep_trace: voice.and_then(|v| v.get("KeepDictationTrace")).and_then(Value::as_bool).unwrap_or(false),
            paste_dictation: voice.and_then(|v| v.get("PasteDictation")).and_then(Value::as_bool).unwrap_or(true),
            sounds: voice.and_then(|v| v.get("Sounds")).and_then(Value::as_bool).unwrap_or(true),
        }
    }
}

/// A saved list of `{ Spoken, Canonical }`; blank rows are dropped.
fn pairs(voice: Option<&Value>, key: &str) -> Vec<(String, String)> {
    voice
        .and_then(|v| v.get(key))
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let get = |k: &str| row.get(k).and_then(Value::as_str).map(|s| s.trim().to_string());
                    Some((get("Spoken")?, get("Canonical")?)).filter(|(s, c)| !s.is_empty() && !c.is_empty())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The model setting names a folder under `Models\stt`, never a path out of it.
fn is_plain_folder_name(name: &str) -> bool {
    !name.contains(['/', '\\', ':']) && name != "." && name != ".." && !name.contains("..")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings(json: Value) -> Settings {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn nothing_saved_means_off_with_the_defaults() {
        let v = VoiceSettings::from(&Settings::default());
        assert!(!v.enabled);
        assert_eq!(v.push_to_talk_buttons, [0]);
        assert_eq!((v.microphone.as_str(), v.model_folder.as_str(), v.min_utterance_ms), ("", "parakeet-tdt-0.6b-v2", 300));
    }

    #[test]
    fn keeping_the_history_is_off_until_the_user_turns_it_on() {
        assert!(!VoiceSettings::from(&Settings::default()).keep_history);
        assert!(VoiceSettings::from(&settings(json!({ "Voice": { "KeepHistory": true } }))).keep_history);
    }

    #[test]
    fn keeping_the_dictation_trace_is_off_until_the_user_turns_it_on() {
        assert!(!VoiceSettings::from(&Settings::default()).keep_trace);
        assert!(VoiceSettings::from(&settings(json!({ "Voice": { "KeepDictationTrace": true } }))).keep_trace);
    }

    #[test]
    fn dictation_cleanup_is_on_by_default_and_the_glossary_is_read() {
        let v = VoiceSettings::from(&Settings::default());
        assert!(v.dictation_cleanup && v.dictation_glossary.is_empty());

        let v = VoiceSettings::from(&settings(json!({ "Voice": {
            "DictationCleanup": false,
            "DictationGlossary": [
                { "Spoken": " kay cad ", "Canonical": "KiCad" },
                { "Spoken": "", "Canonical": "x" },
                { "Spoken": "y" }
            ] } })));
        assert!(!v.dictation_cleanup);
        assert_eq!(v.dictation_glossary, vec![("kay cad".to_string(), "KiCad".to_string())]);

        let v = VoiceSettings::from(&settings(json!({ "Voice": { "Replacements": [{ "Spoken": "my mail", "Canonical": "me@x.com" }] } })));
        assert_eq!(v.replacements, vec![("my mail".to_string(), "me@x.com".to_string())]);
    }

    #[test]
    fn the_owners_real_voice_section_is_read() {
        let v = VoiceSettings::from(&settings(json!({ "Voice": {
            "Enabled": true, "PushToTalkMask": [0], "MicrophoneDeviceName": "Microphone (Logi C270 HD WebCam",
            "ModelFileName": "parakeet-tdt-0.6b-v2", "MinUtteranceMs": 300, "MaxUtteranceMs": 15000 } })));
        assert!(v.enabled);
        assert_eq!(v.microphone, "Microphone (Logi C270 HD WebCam");
    }

    #[test]
    fn a_model_setting_cannot_point_out_of_the_models_folder() {
        for bad in ["..\\..\\Windows", "C:\\x", "a/b", "..", "x..y"] {
            let v = VoiceSettings::from(&settings(json!({ "Voice": { "ModelFileName": bad } })));
            assert_eq!(v.model_folder, DEFAULT_STT_MODEL_FOLDER, "{bad}");
        }
        assert_eq!(VoiceSettings::from(&settings(json!({ "Voice": { "ModelFileName": "my-model" } }))).model_folder, "my-model");
    }

    #[test]
    fn groq_is_off_unless_chosen_and_falls_back_by_default() {
        let off = VoiceSettings::from(&settings(json!({ "Voice": { "SttProvider": 2 } })));
        assert!(!off.use_groq && off.groq_fallback_to_local && off.language == "en" && off.groq_model.is_empty());
        assert!(!VoiceSettings::from(&settings(json!({}))).use_groq);
        assert!(!VoiceSettings::from(&settings(json!({ "Voice": { "SttProvider": 0 } }))).use_groq, "the retired value is the local engine");
        let on = VoiceSettings::from(&settings(json!({ "Voice": { "SttProvider": 1, "GroqSttModel": " m ", "GroqFallbackToLocal": false, "Language": "de" } })));
        assert!(on.use_groq && !on.groq_fallback_to_local && on.groq_model == "m" && on.language == "de");
    }

    #[test]
    fn junk_values_fall_back() {
        let v = VoiceSettings::from(&settings(json!({ "Voice": { "Enabled": "yes", "MinUtteranceMs": 999999, "PushToTalkMask": "x" } })));
        assert!(!v.enabled);
        assert_eq!(v.min_utterance_ms, 300);
        assert_eq!(v.push_to_talk_buttons, [0]);
    }
}
