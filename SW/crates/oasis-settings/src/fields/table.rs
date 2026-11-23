//! The field table: every simple setting once, in the order the Settings screen shows them.

use super::*;

pub(super) const SECTIONS: [(&str, &str, &str); 7] = [
    ("general", "General", "How the OASIS interface and the overlay behave day to day."),
    ("youtube", "YouTube", "Hidden gestures and ad skipping, for the browser extension."),
    ("pad", "Pad gestures", "Gestures that work in every program."),
    ("overlay", "Overlay look", "Colours and sizes of the bars and labels on screen."),
    ("voice", "Voice", "Push-to-talk: talk to type, or give a command."),
    ("meetings", "Meetings", "Reminders and joining from your calendar."),
    ("preferences", "Preferences", "General choices OASIS uses when you do not name one."),
];

/// The colours the overlay can use, as in the C# window.
pub(super) const THEME_COLORS: [(&str, &str); 6] = [
    ("Black", "#161D1F"),
    ("Grey", "#A0A0A0"),
    ("White", "#FFFFFF"),
    ("Muted", "#6E8090"),
    ("Surface", "#F4FAFD"),
    ("Border", "#D0D8DC"),
];

pub(super) fn buttons(with_off: bool) -> Vec<(Value, String)> {
    let mut list = Vec::new();
    if with_off {
        list.push((json!(-1), "Off".to_string()));
    }
    list.extend((0..3).map(|i| (json!(i), format!("Button {}", i + 1))));
    list
}

pub(super) fn number(min: f64, max: f64, step: f64, integer: bool) -> Kind {
    Kind::Number { min, max, step, integer }
}

pub(super) fn specs(ctx: &FieldContext, settings: &Settings) -> Vec<Spec> {
    let mut list = vec![
        Spec {
            id: "general.show_labels",
            section: "general",
            label: "Show button labels first",
            hint: "The first tap on a button shows what the three buttons do; the second tap runs it. A profile can override this.",
            place: Place::ShowLabels,
            default: json!(true),
            kind: Kind::Toggle,
        },
        Spec {
            id: "general.paste_dictation",
            section: "general",
            label: "Paste dictation after transcribing",
            hint: "On: what you say is typed where your cursor is. Off: it is only transcribed and kept in Insights (if history is on), and nothing is pasted.",
            place: Place::In("Voice", "PasteDictation"),
            default: json!(true),
            kind: Kind::Toggle,
        },
        Spec {
            id: "general.paste_tag",
            section: "general",
            label: "Add “[typed using project OASIS]” after pasted dictation",
            hint: "Off by default. When on, that note in square brackets is pasted right after the text you spoke. It is not added to Insights.",
            place: Place::In("Voice", "PasteTag"),
            default: json!(false),
            kind: Kind::Toggle,
        },
        Spec {
            id: "general.allow_scripts",
            section: "general",
            label: "Allow scripts",
            hint: "Lets a button run a PowerShell or AutoHotkey script you wrote. Off by default: a script can do anything a program can. Scripts never run from voice or the browser extension.",
            place: Place::AllowScripts,
            default: json!(false),
            kind: Kind::Toggle,
        },
        Spec {
            id: "general.bridge_enabled",
            section: "general",
            label: "Browser extension link",
            hint: "Lets the OASIS Bridge browser extension tell OASIS what YouTube is doing.",
            place: Place::Root("BrowserBridgeEnabled"),
            default: json!(true),
            kind: Kind::Toggle,
        },
        Spec {
            id: "general.bridge_port",
            section: "general",
            label: "Extension link port",
            hint: "Change only if another program uses 17322. The extension must use the same port.",
            place: Place::Root("BrowserBridgePort"),
            default: json!(17322),
            kind: number(1024.0, 65535.0, 1.0, true),
        },
        Spec {
            id: "youtube.auto_skip_ads",
            section: "youtube",
            label: "Skip ads automatically",
            hint: "The extension clicks Skip when it appears.",
            place: Place::In("YouTube", "AutoSkipAds"),
            default: json!(true),
            kind: Kind::Toggle,
        },
        Spec {
            id: "youtube.gestures",
            section: "youtube",
            label: "Hidden gestures",
            hint: "Hold a button for 2x speed while a video plays.",
            place: Place::In("YouTube", "GesturesEnabled"),
            default: json!(true),
            kind: Kind::Toggle,
        },
        Spec {
            id: "youtube.hold2x_button",
            section: "youtube",
            label: "Hold for 2x speed",
            hint: "The button to hold.",
            place: Place::In("YouTube", "Hold2xButtonIndex"),
            default: json!(1),
            kind: Kind::Choice(buttons(true)),
        },
        Spec {
            id: "pad.sounds",
            section: "pad",
            label: "Sound",
            hint: "A short sound when you press and hold to talk, and another when the words are pasted.",
            place: Place::In("Voice", "Sounds"),
            default: json!(true),
            kind: Kind::Toggle,
        },
        Spec {
            id: "pad.show_desktop_button",
            section: "pad",
            label: "Hold to show the desktop",
            hint: "Hold this button in any program to show the desktop; hold again to bring the windows back.",
            place: Place::In("PadGestures", "ShowDesktopButtonIndex"),
            default: json!(2),
            kind: Kind::Choice(buttons(true)),
        },
        Spec {
            id: "pad.show_desktop_hold_ms",
            section: "pad",
            label: "How long to hold",
            hint: "",
            place: Place::In("PadGestures", "ShowDesktopHoldMs"),
            default: json!(800),
            kind: Kind::Choice(
                [500, 800, 1000, 1500, 2000]
                    .iter()
                    .map(|ms| (json!(ms), format!("{:.1} s", f64::from(*ms) / 1000.0)))
                    .collect(),
            ),
        },
    ];

    // Where the overlay sits and how it rests. Root keys, so "Reset look" keeps them.
    list.push(Spec {
        id: "overlay.dock",
        section: "overlay",
        label: "Where the overlay sits",
        hint: "Left or right stacks the bars down that edge; top or bottom puts them in a row, centred.",
        place: Place::Root("OverlayDock"),
        default: json!("right"),
        kind: Kind::Choice(vec![
            (json!("right"), "Right edge".to_string()),
            (json!("left"), "Left edge".to_string()),
            (json!("bottom"), "Bottom, centred".to_string()),
            (json!("top"), "Top, centred".to_string()),
        ]),
    });
    list.push(Spec {
        id: "overlay.idle",
        section: "overlay",
        label: "When not in use",
        hint: "",
        place: Place::Root("OverlayIdle"),
        default: json!("faint"),
        kind: Kind::Choice(vec![
            (json!("faint"), "Faint".to_string()),
            (json!("hidden"), "Hidden".to_string()),
            (json!("always"), "Always full".to_string()),
        ]),
    });
    list.push(Spec {
        id: "overlay.popup",
        section: "overlay",
        label: "Voice pop-up",
        hint: "Across the bottom of the screen, or down its side.",
        place: Place::Root(crate::overlay::POPUP),
        default: json!("horizontal"),
        kind: Kind::Choice(vec![
            (json!("horizontal"), "Horizontal".to_string()),
            (json!("vertical"), "Vertical".to_string()),
        ]),
    });

    // The overlay look.
    /// One overlay number: id, key, label, min, max, step, default, whole numbers only.
    type Look = (&'static str, &'static str, &'static str, f64, f64, f64, f64, bool);
    let looks: [Look; 8] = [
        ("overlay.indicator_width", "IndicatorWidth", "Bar width", 4.0, 20.0, 1.0, 8.0, true),
        ("overlay.indicator_height", "IndicatorHeight", "Bar height", 16.0, 64.0, 1.0, 50.0, true),
        ("overlay.indicator_spacing", "IndicatorSpacing", "Space between bars", 2.0, 24.0, 1.0, 14.0, true),
        ("overlay.indicator_opacity", "IndicatorOpacity", "Bar opacity", 0.2, 1.0, 0.05, 1.0, false),
        ("overlay.label_font_size", "LabelFontSize", "Label text size", 12.0, 36.0, 1.0, 14.0, true),
        ("overlay.label_font_opacity", "LabelFontOpacity", "Label text opacity", 0.2, 1.0, 0.05, 1.0, false),
        ("overlay.label_bubble_opacity", "LabelBubbleOpacity", "Label bubble opacity", 0.2, 1.0, 0.05, 1.0, false),
        ("overlay.label_corner_radius", "LabelCornerRadius", "Label roundness", 0.0, 48.0, 1.0, 22.0, true),
    ];
    for (id, key, label, min, max, step, default, integer) in looks {
        list.push(Spec {
            id,
            section: "overlay",
            label,
            hint: "",
            place: Place::In("OverlayAppearance", key),
            default: if integer { json!(default as i64) } else { json!(default) },
            kind: number(min, max, step, integer),
        });
    }
    for (id, key, label, default) in [
        ("overlay.indicator_color_1", "IndicatorColor1", "Top bar colour", "#6E8090"),
        ("overlay.indicator_color_2", "IndicatorColor2", "Middle bar colour", "#D0D8DC"),
        ("overlay.indicator_color_3", "IndicatorColor3", "Bottom bar colour", "#F4FAFD"),
        ("overlay.label_background", "LabelBackgroundColor", "Label background", "#FFFFFF"),
        ("overlay.label_text", "LabelTextColor", "Label text colour", "#161D1F"),
        ("overlay.label_border", "LabelBorderColor", "Label border", "#D0D8DC"),
    ] {
        list.push(Spec {
            id,
            section: "overlay",
            label,
            hint: "",
            place: Place::In("OverlayAppearance", key),
            default: json!(default),
            kind: Kind::Color,
        });
    }

    // Preferences (W14): only browsers found on this PC; Chrome when it is there, else the system's.
    let mut browsers = vec![(json!(""), "System default".to_string())];
    browsers.extend(ctx.browsers.iter().map(|(name, shown)| (json!(name), shown.clone())));
    let has_chrome = ctx.browsers.iter().any(|(name, _)| name == "chrome");
    list.push(Spec {
        id: "preferences.browser",
        section: "preferences",
        label: "Preferred browser",
        hint: "Opens a spoken search, or \"open the browser\", when no browser is open. With one open, the page goes there as a new tab.",
        place: Place::In("Preferences", "Browser"),
        default: json!(if has_chrome { "chrome" } else { "" }),
        kind: Kind::Choice(browsers),
    });

    // Voice.
    let mut mics = vec![(json!(""), "System default".to_string())];
    mics.extend(ctx.microphones.iter().map(|m| (json!(m), m.clone())));
    let saved_mic = settings
        .extra
        .get("Voice")
        .and_then(|v| v.get("MicrophoneDeviceName"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty());
    if let Some(saved) = saved_mic {
        if !ctx.microphones.iter().any(|m| m == saved) {
            mics.push((json!(saved), format!("{saved} (not connected)")));
        }
    }
    let mut speakers = vec![(json!(""), "System default".to_string())];
    speakers.extend(ctx.speakers.iter().map(|m| (json!(m), m.clone())));
    let saved_speaker = settings
        .extra
        .get("Voice")
        .and_then(|v| v.get("SpeakerDeviceName"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty());
    if let Some(saved) = saved_speaker {
        if !ctx.speakers.iter().any(|m| m == saved) {
            speakers.push((json!(saved), format!("{saved} (not connected)")));
        }
    }
    list.extend([
        Spec {
            id: "voice.enabled",
            section: "voice",
            label: "Push-to-talk",
            hint: "Hold the talk button, speak, let go. Needs a speech model (see the Status screen).",
            place: Place::In("Voice", "Enabled"),
            default: json!(false),
            kind: Kind::Toggle,
        },
        Spec {
            id: "voice.microphone",
            section: "voice",
            label: "Microphone",
            hint: "",
            place: Place::In("Voice", "MicrophoneDeviceName"),
            default: json!(""),
            kind: Kind::Choice(mics),
        },
        Spec {
            id: "voice.speaker",
            section: "voice",
            label: "Sound output",
            hint: "Where the start and end sounds play. Pick the speaker you hear, if a USB microphone takes over the default.",
            place: Place::In("Voice", "SpeakerDeviceName"),
            default: json!(""),
            kind: Kind::Choice(speakers),
        },
        Spec {
            id: "voice.talk_buttons",
            section: "voice",
            label: "Buttons to hold",
            hint: "A short tap still runs the button's normal action.",
            place: Place::TalkButtons,
            default: json!("0"),
            kind: Kind::Choice(talk_presets()),
        },
        Spec {
            id: "voice.hold_ms",
            section: "voice",
            label: "Hold time",
            hint: "How long to hold before listening starts.",
            place: Place::In("Voice", "PushToTalkHoldMs"),
            default: json!(200),
            kind: Kind::Choice([200, 350, 500, 750, 1000].iter().map(|ms| (json!(ms), format!("{ms} ms"))).collect()),
        },
        Spec {
            id: "voice.capture_mode",
            section: "voice",
            label: "What a recording becomes",
            hint: "Auto types when a text field is focused and runs a command otherwise.",
            place: Place::In("Voice", "CaptureMode"),
            default: json!(0),
            kind: Kind::Choice(vec![
                (json!(0), "Auto".to_string()),
                (json!(1), "Always a command".to_string()),
                (json!(2), "Always typed".to_string()),
            ]),
        },
        Spec {
            id: "voice.approval_mode",
            section: "voice",
            label: "Voice commands that ask first",
            hint: "Risky commands always ask. Asking means button 1 runs it and button 2 cancels.",
            place: Place::In("Voice", "IntentApprovalMode"),
            default: json!(0),
            kind: Kind::Choice(vec![
                (json!(0), "A new app or link, the first time".to_string()),
                (json!(1), "Every command".to_string()),
                (json!(2), "Only risky ones".to_string()),
            ]),
        },
        Spec {
            id: "voice.allow_launch_app",
            section: "voice",
            label: "Let voice open apps",
            hint: "",
            place: Place::In("Voice", "AllowLaunchApp"),
            default: json!(true),
            kind: Kind::Toggle,
        },
        Spec {
            id: "voice.allow_type_text",
            section: "voice",
            label: "Let voice type text",
            hint: "For \"type …\" and \"paste …\" commands.",
            place: Place::In("Voice", "AllowTypeText"),
            default: json!(true),
            kind: Kind::Toggle,
        },
        Spec {
            id: "voice.edit_commands",
            section: "voice",
            label: "Editing commands",
            hint: "Said on their own, \"copy\", \"paste\", \"undo\", \"select all\", \"ctrl z\", \"go back\" and the like press their keys, also in a text field.",
            place: Place::In("Voice", "EditCommands"),
            default: json!(true),
            kind: Kind::Toggle,
        },
        Spec {
            id: "voice.dictation_cleanup",
            section: "voice",
            label: "Tidy up what you dictate",
            hint: "Spoken \"comma\" and \"full stop\" become symbols, number words become digits, \"um\" is dropped, \"no actually …\" keeps the correction, capitals are added.",
            place: Place::In("Voice", "DictationCleanup"),
            default: json!(true),
            kind: Kind::Toggle,
        },
        Spec {
            id: "voice.polish",
            section: "voice",
            label: "Polish with the local model",
            hint: "S1-mini tidies each sentence on the graphics card while you talk. Needs the Quality model set; switching it off also frees the card's memory.",
            place: Place::In("Voice", "PolishEnabled"),
            default: json!(true),
            kind: Kind::Toggle,
        },
        Spec {
            id: "voice.polish_tone",
            section: "voice",
            label: "Polish tone",
            hint: "How formal the polished text reads: casual is lower case with little punctuation, formal expands contractions.",
            place: Place::In("Voice", "PolishTone"),
            default: json!("balanced"),
            kind: Kind::Choice(
                [("casual", "Casual"), ("semi-casual", "Semi-casual"), ("balanced", "Balanced"), ("semi-formal", "Semi-formal"), ("formal", "Formal")]
                    .iter()
                    .map(|(v, l)| (json!(v), l.to_string()))
                    .collect(),
            ),
        },
        // For testing (V54); may go in the next update.
        Spec {
            id: "voice.polish_structure",
            section: "voice",
            label: "Polish structure",
            hint: "Prose keeps sentences; lists lets the model turn three or more items into bullets.",
            place: Place::In("Voice", "PolishStructure"),
            default: json!("lists"),
            kind: Kind::Choice([("prose", "Prose"), ("lists", "Lists")].iter().map(|(v, l)| (json!(v), l.to_string())).collect()),
        },
        Spec {
            id: "voice.polish_context",
            section: "voice",
            label: "Polish context",
            hint: "Email adds a greeting and a sign-off. Automatic uses email in mail programs and general everywhere else.",
            place: Place::In("Voice", "PolishContext"),
            default: json!("auto"),
            kind: Kind::Choice(
                [("auto", "Automatic"), ("general", "General"), ("email", "Email")].iter().map(|(v, l)| (json!(v), l.to_string())).collect(),
            ),
        },
        Spec {
            id: "voice.polish_wait_ms",
            section: "voice",
            label: "Polish wait limit",
            hint: "The longest the polish may add after you let go, while plugged in. On battery it waits 1.5 times as long, twice with Battery Saver on. Text not polished in time is typed as the rules wrote it.",
            place: Place::In("Voice", "PolishWaitMs"),
            default: json!(3000),
            kind: Kind::Choice([600, 800, 1000, 1500, 2000, 3000].iter().map(|ms| (json!(ms), format!("{ms} ms"))).collect()),
        },
        Spec {
            id: "voice.dictation_glossary",
            section: "voice",
            label: "Your own spellings",
            hint: "One per line, as: how it sounds = how to write it. For example: kay cad = KiCad",
            place: Place::Glossary("DictationGlossary"),
            default: json!(""),
            kind: Kind::Glossary { max: MAX_GLOSSARY_TEXT },
        },
        Spec {
            id: "voice.replacements",
            section: "voice",
            label: "Fragments",
            hint: "One per line, as: what you say = what to type. For example: my mail = you@example.com",
            place: Place::Glossary("Replacements"),
            default: json!(""),
            kind: Kind::Glossary { max: MAX_REPLACEMENT_TEXT },
        },
        Spec {
            id: "voice.keep_history",
            section: "voice",
            label: "Keep my dictation history",
            hint: "Saves each dictation, the recording and its text, in a folder on this PC for the Insights screen. Nothing is sent anywhere; you decide whether to share the folder.",
            place: Place::In("Voice", "KeepHistory"),
            default: json!(false),
            kind: Kind::Toggle,
        },
        Spec {
            id: "voice.keep_trace",
            section: "voice",
            label: "Keep a dictation trace (for troubleshooting)",
            hint: "Writes what you said, before and after cleanup, to a log file on this PC. Turn it off when you are done; nothing is sent anywhere.",
            place: Place::In("Voice", "KeepDictationTrace"),
            default: json!(false),
            kind: Kind::Toggle,
        },
        Spec {
            id: "meetings.reminders",
            section: "meetings",
            label: "Remind me before a meeting",
            hint: "A reminder with Join, Snooze and Dismiss appears in the OASIS window. Needs your calendar connected (below).",
            place: Place::In("Meetings", "RemindersEnabled"),
            default: json!(true),
            kind: Kind::Toggle,
        },
        Spec {
            id: "meetings.remind_minutes",
            section: "meetings",
            label: "Remind me this many minutes before",
            hint: "",
            place: Place::In("Meetings", "ReminderMinutesBefore"),
            default: json!(5),
            kind: number(1.0, 60.0, 1.0, true),
        },
        Spec {
            id: "meetings.join_minutes",
            section: "meetings",
            label: "Allow joining this many minutes before the start",
            hint: "The join button and \"join my meeting\" open a meeting from this long before it starts until it ends.",
            place: Place::In("Meetings", "JoinWindowMinutesBefore"),
            default: json!(15),
            kind: number(1.0, 120.0, 1.0, true),
        },
        Spec {
            id: "voice.stt_provider",
            section: "voice",
            label: "Where speech is turned into text",
            hint: "On this computer keeps everything private. Groq sends each recording to api.groq.com and needs an API key (below); it is faster on slow computers.",
            place: Place::In("Voice", "SttProvider"),
            default: json!(2),
            kind: Kind::Choice(vec![
                (json!(2), "On this computer (private)".to_string()),
                (json!(1), "Groq online (sends your speech to Groq)".to_string()),
            ]),
        },
        Spec {
            id: "voice.groq_fallback",
            section: "voice",
            label: "If Groq fails, use this computer",
            hint: "Offline, a wrong key or a rate limit then still gives you text. Needs the speech model installed.",
            place: Place::In("Voice", "GroqFallbackToLocal"),
            default: json!(true),
            kind: Kind::Toggle,
        },
        Spec {
            id: "voice.match_threshold",
            section: "voice",
            label: "How exact a command must be",
            hint: "Higher refuses more guesses.",
            place: Place::In("Voice", "MatchThreshold"),
            default: json!(0.72),
            kind: number(0.5, 0.95, 0.01, false),
        },
    ]);
    list
}

/// Button combinations for push-to-talk, as a list of 0-based buttons.
pub(super) const TALK_PRESETS: [(&str, &[i64]); 6] = [
    ("Button 1", &[0]),
    ("Button 2", &[1]),
    ("Button 3", &[2]),
    ("Buttons 1 and 3 together", &[0, 2]),
    ("Buttons 1 and 2 together", &[0, 1]),
    ("Buttons 2 and 3 together", &[1, 2]),
];

pub(super) fn mask_key(mask: &[i64]) -> String {
    mask.iter().map(i64::to_string).collect::<Vec<_>>().join("+")
}

pub(super) fn talk_presets() -> Vec<(Value, String)> {
    TALK_PRESETS.iter().map(|(label, mask)| (json!(mask_key(mask)), (*label).to_string())).collect()
}

pub(super) fn read_mask(settings: &Settings) -> Vec<i64> {
    settings
        .extra
        .get("Voice")
        .and_then(|v| v.get("PushToTalkMask"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_else(|| vec![0])
}
