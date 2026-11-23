//! A spoken phrase: typed, run, asked about on the pad, or reported; spoken alarms, timers and reminders.

use super::*;

/// A spoken command waiting for the pad: button 1 runs it, button 2 cancels.
pub(super) struct Pending {
    pub(super) action: ButtonAction,
    pub(super) steps: Vec<(String, String)>,
    pub(super) summary: String,
    pub(super) heard: String,
    pub(super) label: String,
    pub(super) until: Instant,
    /// A plugged-in microphone this question offers; approving switches to it instead of running `action`.
    pub(super) mic: Option<String>,
}

/// How long a new-microphone offer stays up.
pub(super) const MIC_OFFER_TIMEOUT: Duration = Duration::from_secs(12);

/// How long an unanswered question stays on the pad.
pub(super) const CONFIRM_TIMEOUT: Duration = Duration::from_secs(30);

/// What the handlers of a spoken phrase need.
pub(super) struct SpokenContext<'a> {
    pub(super) shared: &'a Arc<Shared>,
    pub(super) settings: &'a Settings,
    pub(super) verified: bool,
    pub(super) jobs: &'a Jobs,
    pub(super) watch_token: &'a str,
}

/// Something was said. `decide` (`docs/VOICE_DECISIONS.md` V3, V4) says whether it is typed, run,
/// asked about or reported; this gathers what it needs from the PC and carries it out. Returns a
/// question for the pad when the command needs approval or was a near miss.
pub(super) fn handle_heard(
    ctx: &SpokenContext<'_>,
    foreground: &mut dyn ForegroundProbe,
    running: Vec<RunningApp>,
    installed: &dyn Fn(&str) -> bool,
    on_screen: &dyn Fn() -> Vec<oasis_core::voice::command::on_screen::ScreenItem>,
    app_names: &dyn Fn() -> Vec<String>,
    heard: Heard,
    now: Instant,
) -> Option<Pending> {
    let in_text_field = heard.dictate;
    let voice = VoiceSettings::from(ctx.settings);
    let router = voice_router_settings(ctx.settings);
    let glossary: Vec<GlossaryEntry> = voice.dictation_glossary.iter().map(|(s, c)| GlossaryEntry::new(s, c)).collect();
    let trusted = trusted_targets(ctx.settings);
    let facts = Facts {
        transcript: &heard.text,
        in_text_field,
        fragments: &voice.replacements,
        glossary: &glossary,
        allow_type_text: router.allow_type_text,
        edit_commands: router.edit_commands,
        verified: ctx.verified,
        live: ctx.shared.live.load(Ordering::SeqCst),
        trusted: &trusted,
        approval: approval_mode(ctx.settings),
    };
    // The Desktop and open Explorer folders, read only for an open-style phrase (a few ms).
    let lower = heard.text.trim().to_lowercase();
    let opening = ["open", "launch", "start", "run", "show", "go to", "take me", "bring up", "pull up"].iter().any(|w| lower.starts_with(w));
    let on_screen = if opening { on_screen() } else { Vec::new() };
    let app_names = if opening { app_names() } else { Vec::new() };
    let mut route_text = |text: &str| {
        let active = foreground.current_for_press().ok().and_then(|info| {
            let found = resolve_active_profile(&ctx.settings.profiles, &ForegroundContext::from(&info), ctx.watch_token)?;
            Some((found.profile, found.config?))
        });
        let lexicon = voice_lexicon(ctx.settings, active, &running);
        route(
            text,
            &lexicon,
            &RouterSettings {
                now: Some(local_now()),
                spotify_running: running.iter().any(|a| a.process.eq_ignore_ascii_case("spotify")),
                running: running.clone(),
                // "cloud" means Claude only on a PC that has it.
                installed: Some(["claude", "whatsapp", "kicad"].iter().filter(|a| installed(a)).map(|a| a.to_string()).collect()),
                on_screen: on_screen.clone(),
                app_names: app_names.clone(),
                ..voice_router_settings(ctx.settings)
            },
        )
    };
    // A folder on screen exists though it is no program.
    let exists = |app: &str| launch_target_exists(app, &running, installed) || (app.ends_with('\\') && std::path::Path::new(app).is_dir());
    let decided = decide(&facts, &mut route_text, &exists, &starts_a_shell);

    let say = |outcome: &str, label: &str, detail: &str| {
        ctx.shared.emit(EngineEvent::VoiceCommand {
            heard: heard.text.clone(),
            label: label.to_string(),
            outcome: outcome.to_string(),
            detail: detail.to_string(),
        });
    };
    let log = SecurityLog::at(ctx.shared.paths.security_log());
    for text in decided.notes {
        ctx.shared.emit(EngineEvent::Note { text });
    }
    let decision = match decided.decision {
        Decision::Run { action, label } => Decision::Run { label: launched_name(&action, label, &heard.text), action },
        Decision::Ask { action, label, summary } => Decision::Ask { label: launched_name(&action, label, &heard.text), action, summary },
        other => other,
    };
    let typed = match decision {
        Decision::PasteFragment { shown, value } => {
            paste_typed(ctx.shared, ctx.settings, ctx.verified, ctx.jobs, &shown, value);
            false
        }
        Decision::Type => true,
        Decision::Report { outcome, label, detail, log: line } => {
            say(outcome, &label, &detail);
            if let Some(line) = line {
                log.write(&line);
            }
            false
        }
        Decision::Run { action, label } => {
            run_voice_action(ctx.shared, ctx.jobs, &action, &label, &heard.text);
            false
        }
        Decision::Ask { action, label, summary } => {
            say("asking", &label, &summary);
            let steps = steps_of(&action);
            return Some(Pending { action, steps, summary, heard: heard.text.clone(), label, until: now.plus(CONFIRM_TIMEOUT), mic: None });
        }
        Decision::NoMatch { detail, log: line, copy_to_clipboard } => {
            say("no_match", "", &detail);
            log.write(&line);
            if copy_to_clipboard {
                let _ = oasis_win::clipboard::set_text(&heard.text);
                ctx.shared.emit(EngineEvent::Note { text: "Didn't understand that as a command, so nothing was typed. It is on the clipboard in case it was meant as text.".into() });
            }
            false
        }
    };
    if typed {
        let style = dictation_style(foreground);
        type_dictation(ctx, heard, style, !in_text_field);
    }
    None
}

/// An "open X" pill names what was said ("FRAME"), but the launch looks X up by fuzzy match and may
/// start another app (Brave). Name the app that will start. Other labels are left as they are.
pub(super) fn launched_name(action: &ButtonAction, label: String, heard: &str) -> String {
    #[cfg(windows)]
    {
        use oasis_core::actions::app_resolver::Resolved;
        use oasis_platform::windows::app_discovery::resolve;
        let spoken_target = heard.to_lowercase().contains(&label.to_lowercase());
        // A place (`shell:RecycleBinFolder`) keeps its own name; the lookup would guess a `.exe`.
        if action.kind() != ActionKind::LaunchApp || !action.steps.is_empty() || action.value.contains('|') || action.value.starts_with("shell:") || label.starts_with("SWITCH ") || !spoken_target {
            return label;
        }
        let lookup = oasis_core::voice::command::apps::launch_name(&action.value).unwrap_or(&action.value);
        let path = match resolve(lookup) {
            Some(Resolved::Shortcut(p) | Resolved::Exe(p)) => p,
            _ => return label, // a Store app has only an id; the spoken name stays
        };
        let stem = std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().to_uppercase());
        stem.filter(|s| !s.is_empty()).unwrap_or(label)
    }
    #[cfg(not(windows))]
    {
        let _ = (action, heard);
        label
    }
}

/// Whether the window in front is OASIS's own (the engine runs inside the desktop app).
pub(super) fn is_oasis(info: &ForegroundInfo) -> bool {
    info.pid == Some(std::process::id())
}

/// A spoken launch whose name looks harmless but whose real target is a shell: "command" is the
/// Command Prompt shortcut, "node.js command prompt" starts `cmd.exe`. `fast_path` checks the name.
pub(super) fn starts_a_shell(action: &ButtonAction) -> bool {
    #[cfg(windows)]
    {
        steps_of(action).iter().any(|(kind, value)| {
            ActionKind::of_ignore_case(kind) == ActionKind::LaunchApp
                && oasis_platform::windows::app_discovery::launch_target(value)
                    .is_some_and(|t| oasis_core::voice::command::safety::names_a_shell(&t))
        })
    }
    #[cfg(not(windows))]
    {
        let _ = action;
        false
    }
}

/// The wall clock now, for "in 10 minutes" and "at 7 pm".
pub(super) fn local_now() -> LocalTime {
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    #[cfg(windows)]
    let offset_secs = oasis_win::system::utc_offset_seconds();
    #[cfg(not(windows))]
    let offset_secs = 0;
    LocalTime { unix, offset_secs }
}

/// Schedules or cancels what a spoken alarm, timer or reminder phrase asked for. `false` when the
/// action is not one of those, so the caller runs it as usual.
pub(super) fn handle_reminder_action(shared: &Shared, action: &ButtonAction) -> bool {
    let now = local_now();
    let note = |text: String| shared.emit(EngineEvent::Note { text });
    match action.kind() {
        ActionKind::Reminder => {
            let mut parts = action.value.splitn(3, '|');
            let (kind, due, label) = (parts.next().unwrap_or(""), parts.next().and_then(|d| d.parse::<i64>().ok()), parts.next().unwrap_or(""));
            match due.filter(|d| *d > 0 && matches!(kind, "alarm" | "timer" | "reminder")) {
                Some(due) if shared.reminders.lock().unwrap_or_else(|e| e.into_inner()).add(kind, due, label) => {
                    note(phrases::describe(kind, due, &phrases::flat(label), now));
                }
                Some(_) => note("Too many alarms, timers and reminders are waiting; cancel some first.".into()),
                None => note("That alarm, timer or reminder had no usable time.".into()),
            }
            true
        }
        ActionKind::CancelReminders => {
            let removed = shared.reminders.lock().unwrap_or_else(|e| e.into_inner()).cancel(&action.value);
            note(match removed {
                0 => "Nothing to cancel.".into(),
                1 => "Cancelled 1.".into(),
                n => format!("Cancelled {n}."),
            });
            true
        }
        _ => false,
    }
}

/// Everything that has come due: shown, and (when actions are on) sounded.
pub(super) fn fire_due_reminders(shared: &Shared, jobs: &Jobs) {
    let now = local_now().unix;
    let due = shared.reminders.lock().unwrap_or_else(|e| e.into_inner()).take_due(now);
    for item in due {
        let (title, body) = item.message(now);
        shared.emit(EngineEvent::Reminder { title, body });
        if shared.live.load(Ordering::SeqCst) {
            let action = ButtonAction { kind: ActionKind::Chime.as_str().into(), value: item.chimes().to_string(), steps: vec![] };
            let _ = jobs.send(Job { action, button: -1, what: "sound the reminder".into(), allow_run_script: false, prepare: None });
        }
    }
}

/// Start a spoken command's action. It never runs a script, whatever the setting says.
pub(super) fn run_voice_action(
    shared: &Shared,
    jobs: &Jobs,
    action: &ButtonAction,
    label: &str,
    heard: &str,
) {
    let runs = shared.live.load(Ordering::SeqCst);
    SecurityLog::at(shared.paths.security_log())
        .write(&format!("Voice action: \"{heard}\" → {} / {label} / {}", action.kind, action.value));
    shared.emit(EngineEvent::VoiceCommand {
        heard: heard.to_string(),
        label: label.to_string(),
        outcome: if runs { "ran" } else { "preview" }.into(),
        detail: if runs { String::new() } else { "Actions are off, so nothing happened.".into() },
    });
    if runs {
        let what = format!("{} {}", action.kind, action.value);
        let _ = jobs.send(Job { action: action.clone(), button: -1, what, allow_run_script: false, prepare: None });
    }
}

/// What a close command would shut: the named app, or the window in front (`front`) when none is named.
pub(super) fn close_target(action: &ButtonAction, front: &str) -> Option<String> {
    let named = if ActionKind::of_ignore_case(&action.kind) == ActionKind::CloseApp {
        action.value.trim()
    } else if ActionKind::of_ignore_case(&action.kind) == ActionKind::WindowCommand {
        action.value.to_lowercase().strip_prefix("close|").map(|_| action.value[6..].trim())?
    } else {
        return None;
    };
    Some(if !named.is_empty() { named } else if !front.is_empty() { front } else { "this window" }.to_string())
}

#[cfg(test)]
mod close_target_tests {
    use super::{close_target, ButtonAction};

    fn act(kind: &str, value: &str) -> ButtonAction {
        ButtonAction { kind: kind.into(), value: value.into(), steps: vec![] }
    }

    #[test]
    fn a_close_names_its_app_else_the_window_in_front_else_this_window() {
        assert_eq!(close_target(&act("CloseApp", "WhatsApp"), "x").as_deref(), Some("WhatsApp"));
        assert_eq!(close_target(&act("WindowCommand", "close|chrome"), "x").as_deref(), Some("chrome"));
        assert_eq!(close_target(&act("WindowCommand", "close|"), "Notepad").as_deref(), Some("Notepad"));
        assert_eq!(close_target(&act("WindowCommand", "close|"), "").as_deref(), Some("this window"));
        assert_eq!(close_target(&act("WindowCommand", "maximize|chrome"), "x"), None);
        assert_eq!(close_target(&act("LaunchApp", "chrome"), "x"), None);
    }
}
