//! Getting Started, the first-run guide, as the desktop app runs it.
//!
//! The steps themselves are decided in `oasis_core::onboarding`. This module owns the one copy of
//! the guide's progress, saves it, and carries out what a change of step means here: holding the
//! keyboard buttons back, opening and closing the welcome window, showing the caption panel over
//! other programs, and putting the demo website on button 2 and taking it off again.
//!
//! Every event goes through one worker thread (`send`), so the engine can report an event while
//! the main thread is in the middle of a settings change without the two waiting on each other.

use std::sync::mpsc::{self, Sender};
use std::sync::Mutex;
use std::time::Duration;

use oasis_core::actions::ActionKind;
use oasis_core::onboarding::{self as guide, Blocked, Event, Facts, Made, Progress, Status, StepId, Surface};
use oasis_engine::{EngineEvent, EngineHandle};
use oasis_settings::edit::ButtonEdit;
use oasis_settings::input::InputSettings;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

pub const WELCOME: &str = "welcome";
/// The caption panel shown over other programs.
pub const PANEL: &str = "guide";
const PANEL_SIZE: (f64, f64) = (520.0, 136.0);
/// The website button 2 opens during the first press, and only then.
const SITE: &str = "https://dhanushscience.github.io/PROJECT-OASIS/";
const GROQ_KEYS: &str = "https://console.groq.com/keys";
const PROFILE: &str = "Default";
/// Button 2, counted from 0.
const DEMO_BUTTON: usize = 1;
/// How long the guide waits for the person to come back from the browser before it comes forward itself.
const RETURN_AFTER: Duration = Duration::from_secs(60);

// ---- the saved file ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    status: String,
    step: String,
    #[serde(default)]
    skipped: Vec<String>,
    #[serde(default)]
    has_device: Option<bool>,
    #[serde(default)]
    made: Option<SavedMade>,
    #[serde(default)]
    voice_guide_ran: bool,
    #[serde(default)]
    notice_shown: bool,
}

#[derive(Serialize, Deserialize)]
struct SavedMade {
    kind: String,
    value: String,
}

fn to_saved(p: &Progress) -> Saved {
    Saved {
        version: 1,
        status: p.status.id().into(),
        step: p.step.id().into(),
        skipped: p.skipped.iter().map(|s| s.id().to_string()).collect(),
        has_device: p.has_device,
        made: p.made.as_ref().map(|m| SavedMade { kind: m.kind.clone(), value: m.value.clone() }),
        voice_guide_ran: p.voice_guide_ran,
        notice_shown: p.notice_shown,
    }
}

/// `None` for text that is not a progress file. A step name from a newer build starts that part again.
fn from_text(text: &str) -> Option<Progress> {
    let saved: Saved = serde_json::from_str(text).ok()?;
    Some(Progress {
        status: Status::parse(&saved.status)?,
        step: StepId::parse(&saved.step).unwrap_or(StepId::Welcome),
        has_device: saved.has_device,
        skipped: saved.skipped.iter().filter_map(|s| StepId::parse(s)).collect(),
        blocked: Vec::new(),
        made: saved.made.map(|m| Made { kind: m.kind, value: m.value }),
        voice_guide_ran: saved.voice_guide_ran,
        notice_shown: saved.notice_shown,
    })
}

/// What a start begins from. `file` is `None` when there is no progress file, and `Some(None)` when
/// there is one that cannot be read.
fn first_run(created_profiles: bool, file: Option<Option<Progress>>) -> Progress {
    match file {
        // No record: a first run, whether the profiles are new or left over from an earlier release.
        None => guide::fresh(),
        // Damaged: no guide, so the keys can never be held by a file nobody can read.
        Some(None) => guide::dismissed(),
        // Progress without the profiles it was about: the data folder was emptied by hand.
        Some(Some(_)) if created_profiles => guide::fresh(),
        Some(Some(progress)) => progress,
    }
}

// ---- state ------------------------------------------------------------------------------------

#[derive(Default)]
pub struct OnboardingState {
    progress: Mutex<Option<Progress>>,
    events: Mutex<Option<Sender<Event>>>,
    /// The last thing the downloader was heard to say, so its many progress updates are sent on once.
    models: Mutex<Option<(bool, &'static str)>>,
}

/// What `begin` found. Which window opens is asked later, through `welcome_due`.
pub struct GuideStart {
    /// The engine starts with the keyboard buttons held back.
    pub hold: bool,
}

fn current(app: &AppHandle) -> Progress {
    let state = app.state::<OnboardingState>();
    let guard = state.progress.lock().unwrap_or_else(|p| p.into_inner());
    guard.clone().unwrap_or_else(guide::dismissed)
}

fn replace(app: &AppHandle, progress: &Progress) {
    let state = app.state::<OnboardingState>();
    *state.progress.lock().unwrap_or_else(|p| p.into_inner()) = Some(progress.clone());
    save(progress);
}

/// A failed write is logged and the guide carries on in memory.
fn save(progress: &Progress) {
    let Some(path) = oasis_engine::defaults::detect_paths().map(|p| p.onboarding()) else { return };
    let text = serde_json::to_string_pretty(&to_saved(progress)).unwrap_or_default();
    let temp = path.with_extension("json.tmp");
    if let Err(why) = std::fs::write(&temp, text).and_then(|()| std::fs::rename(&temp, &path)) {
        eprintln!("The guide's progress could not be saved: {why}");
    }
}

fn wants_welcome(p: &Progress) -> bool {
    p.status.active() && p.step.surface() == Surface::Welcome
}

/// Read the saved progress and decide how this start begins. Called once, before any window is shown.
pub fn begin(app: &AppHandle, created_profiles: bool) -> GuideStart {
    let file = oasis_engine::defaults::detect_paths()
        .map(|p| p.onboarding())
        .filter(|path| path.exists())
        .map(|path| std::fs::read_to_string(path).ok().and_then(|text| from_text(&text)));
    let progress = first_run(created_profiles, file);
    replace(app, &progress);

    let (tx, rx) = mpsc::channel();
    *app.state::<OnboardingState>().events.lock().unwrap_or_else(|p| p.into_inner()) = Some(tx);
    let worker = app.clone();
    std::thread::Builder::new()
        .name("oasis-guide".into())
        .spawn(move || {
            for event in rx {
                apply(&worker, event);
            }
        })
        .ok();
    GuideStart { hold: guide::holds_keys(&progress) }
}

/// Hand an event to the guide. Never blocks, so it is safe from any thread, the engine's included.
pub fn send(app: &AppHandle, event: Event) {
    if let Some(tx) = app.state::<OnboardingState>().events.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        let _ = tx.send(event);
    }
}

fn engine<R>(app: &AppHandle, f: impl FnOnce(&EngineHandle) -> R) -> Option<R> {
    let slot = app.state::<crate::EngineSlot>();
    let guard = slot.handle.lock().ok()?;
    guard.as_ref().map(f)
}

fn facts(app: &AppHandle) -> Facts {
    let (below_floor, voice_ready) = crate::models::guide_facts(app);
    Facts {
        browsers_installed: crate::installed_browsers().len(),
        below_floor,
        voice_ready,
        // A key is stored only after it passed its test on the Groq screen.
        groq_key_set: below_floor && crate::groq_key_status(),
        voice_steps: true,
    }
}

// ---- the demo button --------------------------------------------------------------------------

/// Button 2 of the Default profile: the profile's place, and the button's type and value. `None`
/// when there is no such profile.
fn demo_button(e: &EngineHandle) -> Option<(usize, String, String, bool)> {
    let settings = e.settings();
    let profile = settings.profiles.iter().position(|p| p.name == PROFILE)?;
    let button = settings.profiles[profile].config_profiles.first()?.buttons.get(DEMO_BUTTON)?;
    let empty = ActionKind::of(&button.kind) != ActionKind::Automation && button.value.trim().is_empty() && button.label.trim().is_empty();
    Some((profile, button.kind.clone(), button.value.clone(), empty))
}

fn set_demo_button(e: &EngineHandle, profile: usize, kind: &str, value: &str, label: &str) -> bool {
    let edit = ButtonEdit { kind: kind.into(), value: value.into(), label: label.into(), steps: Vec::new() };
    e.change(|s| s.set_button(profile, 0, DEMO_BUTTON, edit)).is_ok()
}

/// Put the website on button 2. `even_if_used` replaces what is there (the person agreed).
fn write_demo(app: &AppHandle, progress: &mut Progress, even_if_used: bool) {
    let made = engine(app, |e| {
        let (profile, _, _, empty) = demo_button(e)?;
        (empty || even_if_used).then(|| set_demo_button(e, profile, ActionKind::OpenUrl.as_str(), SITE, "OASIS")).filter(|ok| *ok)
    });
    if made.flatten().is_some() {
        progress.made = Some(Made { kind: ActionKind::OpenUrl.as_str().into(), value: SITE.into() });
    }
}

/// The demo is over: take the website off button 2, unless the person has changed the button since.
fn clear_demo(app: &AppHandle, progress: &mut Progress) {
    let Some(made) = progress.made.take() else { return };
    engine(app, |e| {
        if let Some((profile, kind, value, _)) = demo_button(e) {
            if guide::same_action(&made, &kind, &value) {
                set_demo_button(e, profile, ActionKind::Shortcut.as_str(), "", "");
            }
        }
    });
}

fn in_demo(p: &Progress) -> bool {
    p.status.active() && matches!(p.step, StepId::FirstPress | StepId::Return)
}

// ---- applying an event ------------------------------------------------------------------------

fn apply(app: &AppHandle, event: Event) {
    let before = current(app);
    let mut after = guide::step(&before, event, &facts(app));
    if after == before {
        return;
    }
    // The keys: held before the first press, let go from then on and on every way out.
    if guide::holds_keys(&after) != guide::holds_keys(&before) {
        engine(app, |e| e.hold_keys(guide::holds_keys(&after)));
    }
    // Button 2: the website goes on when the first press is asked for, and comes off after it.
    if in_demo(&after) && after.step == StepId::FirstPress && after.made.is_none() {
        write_demo(app, &mut after, false);
    }
    if !in_demo(&after) {
        clear_demo(app, &mut after);
    }
    replace(app, &after);

    let entered = after.step != before.step || after.status != before.status;
    if entered && after.status.active() {
        match after.step {
            // Already connected: nothing to wait for.
            StepId::Device if engine(app, |e| e.status().pad_name.is_some()) == Some(true) => send(app, Event::DeviceConnected),
            StepId::Return => {
                let later = app.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(RETURN_AFTER);
                    send(&later, Event::Timeout); // ignored unless the guide is still waiting there
                });
            }
            _ => {}
        }
    }
    show(app, &before, &after);
}

/// Bring the windows in line with the step.
fn show(app: &AppHandle, before: &Progress, after: &Progress) {
    let over_another_program = after.status.active() && after.step.surface() == Surface::Overlay;
    if let Some(panel) = app.get_webview_window(PANEL) {
        if over_another_program {
            let animation = if after.step == StepId::Return { "point" } else { "idle" };
            let _ = app.emit_to(PANEL, "guide-overlay", json!({ "animation": animation, "step": after.step.id(), "line": 0 }));
            let _ = panel.show();
        } else {
            let _ = app.emit_to(PANEL, "guide-overlay", json!({}));
            let _ = panel.hide();
        }
    }
    // After the voice steps the practice opens in the welcome window; closing it brings OASIS back in front.
    let _ = app.emit("onboarding-changed", view(app, after));

    let (welcome_was, welcome_now) = (wants_welcome(before), wants_welcome(after));
    let left_return = before.step == StepId::Return && after.step != StepId::Return;
    if welcome_was == welcome_now && !left_return {
        return;
    }
    // Windows are opened and closed on the main thread.
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if welcome_now {
            show_welcome(&handle);
        } else {
            if let Some(window) = handle.get_webview_window(WELCOME) {
                let _ = window.destroy();
            }
            crate::show_main(&handle);
        }
    });
}

// ---- what the pages see -----------------------------------------------------------------------

#[derive(Serialize, Clone)]
pub struct View {
    status: &'static str,
    step: &'static str,
    has_device: Option<bool>,
    /// Why the step on screen cannot go on, if it cannot.
    blocked: Option<&'static str>,
    /// Before the first press, something of the person's must change first (button 2 is in use, or
    /// there is no way to press a button): the page asks.
    ask: bool,
    /// Button 2 holds something of the person's own.
    button_used: bool,
    keyboard: bool,
    /// The keyboard shortcut for button 2, to show beside "press button 2".
    chord: String,
}

fn view(app: &AppHandle, p: &Progress) -> View {
    let (button_used, keyboard, chord) = engine(app, |e| {
        let used = demo_button(e).is_some_and(|(_, kind, value, empty)| {
            !empty && !p.made.as_ref().is_some_and(|m| guide::same_action(m, &kind, &value))
        });
        let input = InputSettings::from(&e.settings());
        (used, input.keyboard, input.keys[DEMO_BUTTON].clone())
    })
    .unwrap_or((false, true, String::new()));
    let no_way_to_press = p.has_device != Some(true) && !keyboard;
    View {
        status: p.status.id(),
        step: p.step.id(),
        has_device: p.has_device,
        blocked: guide::blocking(p).map(Blocked::id),
        ask: p.status.active() && p.step == StepId::FirstPress && ((p.made.is_none() && button_used) || no_way_to_press),
        button_used,
        keyboard,
        chord,
    }
}

/// The OASIS interface's button 2 was pressed with the OASIS window in front. The engine runs nothing
/// then (the window navigates with the press), so during the first-press step the guide runs the
/// button itself, the way "Try it" does, and counts it as the press.
#[tauri::command]
pub fn onboarding_try_press(app: AppHandle) -> Result<(), String> {
    let ran = engine(&app, |e| {
        let (profile, ..) = demo_button(e).ok_or("There is no Default profile to press a button of.")?;
        e.try_button(profile, 0, DEMO_BUTTON)
    })
    .unwrap_or_else(|| Err("OASIS is not running.".into()));
    if ran.is_ok() {
        send(&app, Event::Press);
    }
    ran
}

#[tauri::command]
pub fn onboarding_state(app: AppHandle) -> View {
    view(&app, &current(&app))
}

/// An event as a page sends it: `{ "type": "device_answer", "value": true }`.
#[derive(Deserialize)]
pub struct PageEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    value: serde_json::Value,
}

/// The events the pages own. Anything else is refused: the rest come from the engine and the windows.
#[tauri::command]
pub fn onboarding_event(app: AppHandle, event: PageEvent) -> Result<(), String> {
    let event = match event.kind.as_str() {
        "next" => Event::Next,
        "skip_step" => Event::SkipStep,
        "skip_all" => Event::SkipAll,
        "back" => Event::Back,
        "device_answer" => Event::DeviceAnswer(event.value.as_bool().ok_or("device_answer needs true or false.")?),
        "browser_chosen" => Event::BrowserChosen,
        "groq_done" => Event::GroqDone,
        "groq_declined" => Event::GroqDeclined,
        "press" => Event::Press,
        "replay" => Event::Replay,
        other => return Err(format!("The guide has no event called '{other}'.")),
    };
    send(&app, event);
    Ok(())
}

/// The person agreed to what the first press needs: button 2 is given the website for the demo
/// (it is cleared afterwards), and keyboard keys are switched on when there is no device.
#[tauri::command]
pub fn onboarding_prepare_press(app: AppHandle) -> Result<(), String> {
    let mut progress = current(&app);
    if !(progress.status.active() && progress.step == StepId::FirstPress) {
        return Ok(());
    }
    if progress.made.is_none() {
        write_demo(&app, &mut progress, true);
    }
    if progress.has_device != Some(true) {
        engine(&app, |e| {
            let mut input = InputSettings::from(&e.settings());
            if !input.keyboard {
                input.keyboard = true;
                let _ = e.change(|s| s.set_input(&input).map_err(oasis_settings::edit::EditError::Invalid));
            }
        });
    }
    replace(&app, &progress);
    let _ = app.emit("onboarding-changed", view(&app, &progress));
    Ok(())
}

/// The answer to "a vertical or a horizontal pop-up?": the overlay, the voice pop-up and, unless the
/// person already chose keys of their own, the keyboard keys (F1 to F3, or ` Tab Caps Lock). The page
/// then sends `next`.
#[tauri::command]
pub fn onboarding_layout(app: AppHandle, vertical: bool) -> Result<(), String> {
    engine(&app, |e| {
        e.change(|s| {
            s.choose_layout(vertical);
            Ok(())
        })
        .map_err(|why| why.to_string())
    })
    .unwrap_or_else(|| Err("OASIS is not running.".into()))
}

/// What the microphone question needs to show.
#[derive(Serialize)]
pub struct MicInfo {
    /// Microphones Windows reports, by name.
    names: Vec<String>,
}

/// The microphones on this PC. None at all blocks the voice part (and says so); the welcome still goes on.
#[tauri::command]
pub fn onboarding_check_mic(app: AppHandle) -> MicInfo {
    let names = oasis_voice::device_names();
    send(&app, if names.is_empty() { Event::Blocked(Blocked::NoMicrophone) } else { Event::Unblocked(Blocked::NoMicrophone) });
    MicInfo { names }
}

/// Turns the Bluetooth radio on, so the pad can be found. Falls back to nothing on other systems.
#[tauri::command]
pub fn bluetooth_turn_on() -> Result<(), String> {
    #[cfg(windows)]
    {
        let paths = oasis_engine::defaults::detect_paths().ok_or("OASIS could not find its data folder.")?;
        oasis_platform::windows::actions::turn_on_bluetooth(&oasis_platform::security_log::SecurityLog::at(paths.security_log()));
        Ok(())
    }
    #[cfg(not(windows))]
    Err("Turn on Bluetooth in your system settings.".into())
}

/// Opens the Windows page where an app is allowed to use the microphone. The address is fixed here.
#[tauri::command]
pub fn onboarding_open_mic_settings() -> Result<(), String> {
    #[cfg(windows)]
    {
        oasis_win::shell::open("ms-settings:privacy-microphone", None).map_err(|code| format!("Windows could not open its settings (code {code})."))
    }
    #[cfg(not(windows))]
    Err("Open the microphone privacy settings of your system.".into())
}

#[derive(Serialize)]
pub struct GroqResult {
    ok: bool,
    text: String,
}

/// Try the stored Groq key with a quarter second of silence. Tells the guide when Groq could not be
/// reached at all (offline), and says so plainly; a rejected key is a different thing.
#[tauri::command]
pub async fn onboarding_test_groq(app: AppHandle) -> Result<GroqResult, String> {
    #[cfg(windows)]
    {
        let model = engine(&app, |e| oasis_settings::voice::VoiceSettings::from(&e.settings()).groq_model).ok_or("OASIS is not running.")?;
        let key = oasis_engine::defaults::windows::groq_key(&crate::groq_store()?).ok_or("No Groq key is stored yet.")?;
        let result = oasis_engine::groq::test_key(&key, &model);
        let offline = result.as_deref().is_some_and(|why| why.starts_with("Groq request failed"));
        send(&app, if offline { Event::Blocked(Blocked::Offline) } else { Event::Unblocked(Blocked::Offline) });
        Ok(match result {
            None => GroqResult { ok: true, text: "The key works.".into() },
            Some(why) if offline => GroqResult { ok: false, text: format!("Groq could not be reached. Check your internet. ({why})") },
            Some(why) => GroqResult { ok: false, text: why },
        })
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("Groq is only wired up on Windows.".into())
    }
}

/// Opens Groq's page for making a key. The address is fixed here; the page cannot name another.
#[tauri::command]
pub fn onboarding_open_groq_keys() -> Result<(), String> {
    #[cfg(windows)]
    {
        oasis_win::shell::open(GROQ_KEYS, None).map_err(|code| format!("Windows could not open the page (code {code})."))
    }
    #[cfg(not(windows))]
    Err(format!("Open {GROQ_KEYS} in a browser."))
}

/// The voice demo's Notepad opens full screen, so the guide's bar at the bottom is all that shares the screen.
/// The window appears a moment after the command ran, so it is looked for for a few seconds.
fn maximize_notepad() {
    #[cfg(windows)]
    std::thread::spawn(|| {
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(250));
            if let Some(w) = oasis_win::desktop::app_windows().into_iter().find(|w| w.process.to_lowercase().contains("notepad")) {
                oasis_win::desktop::maximize(w.handle);
                return;
            }
        }
    });
}

// ---- what the engine and the downloader report --------------------------------------------------

pub fn on_engine_event(app: &AppHandle, event: &EngineEvent) {
    match event {
        EngineEvent::Press(press) if usize::from(press.button) == DEMO_BUTTON + 1 && press.runs && press.profile == PROFILE => {
            send(app, Event::Press);
        }
        EngineEvent::PadConnected { .. } => {
            send(app, Event::Unblocked(Blocked::DeviceLost));
            send(app, Event::Unblocked(Blocked::Bluetooth));
            send(app, Event::DeviceConnected);
        }
        EngineEvent::PadLost => send(app, Event::Blocked(Blocked::DeviceLost)),
        EngineEvent::PadUnavailable { .. } => send(app, Event::Blocked(Blocked::Bluetooth)),
        // The voice part opens Notepad and then types into it: any other command or window does not count.
        EngineEvent::VoiceCommand { heard, label, detail, outcome } if outcome == "ran" => {
            let parts = [heard, label, detail];
            let right = match current(app).step {
                StepId::VoiceOpen => mentions(&parts, "notepad") && !mentions(&parts, "close"),
                StepId::VoiceClose => mentions(&parts, "notepad") && mentions(&parts, "close"),
                _ => false,
            };
            if right {
                if current(app).step == StepId::VoiceOpen {
                    maximize_notepad();
                }
                send(app, Event::CommandRan);
            }
        }
        EngineEvent::VoiceCommand { outcome, .. } if outcome == "asking" => caption(app, LINE_ALLOW),
        // These two look at the engine, and an event is reported from inside an engine call that may hold
        // the engine's lock: asking for it here would wait on itself. A thread of their own asks later.
        EngineEvent::Transcript { typed: true, .. } => {
            let app = app.clone();
            std::thread::spawn(move || {
                let in_notepad = engine(&app, |e| e.status().front_app.is_some_and(|f| is_notepad(&f.process))) == Some(true);
                // Typed where a command was asked for means the command was not understood.
                if in_notepad && current(&app).step == StepId::VoiceType {
                    send(&app, Event::TranscriptTyped);
                } else {
                    caption(&app, LINE_NOTEPAD);
                }
            });
        }
        EngineEvent::VoiceDropped { reason } => match reason.as_str() {
            "too_short" => caption(app, LINE_SHORT),
            "no_speech" => caption(app, LINE_QUIET),
            _ => {
                send(app, Event::Blocked(Blocked::NoMicrophone));
                caption(app, LINE_MIC);
            }
        },
        // The integrity of the profiles file has no event of its own: look after each change.
        EngineEvent::SettingsChanged => {
            let app = app.clone();
            std::thread::spawn(move || sync_integrity(&app));
        }
        _ => {}
    }
}

/// Which line of the voice steps is shown (index into `LINES` in lines.js). Line 0 is the step's own.
const LINE_SHORT: u32 = 1;
const LINE_QUIET: u32 = 2;
const LINE_MIC: u32 = 3;
const LINE_ALLOW: u32 = 4;
const LINE_NOTEPAD: u32 = 5;

fn is_notepad(process: &str) -> bool {
    process.to_lowercase().contains("notepad")
}

fn mentions(parts: &[&String], word: &str) -> bool {
    parts.iter().any(|p| p.to_lowercase().contains(word))
}

/// Show another line of the current voice step on the caption panel, and nothing when the guide is elsewhere.
fn caption(app: &AppHandle, line: u32) {
    let p = current(app);
    if p.status.active() && matches!(p.step, StepId::VoiceOpen | StepId::VoiceType | StepId::VoiceClose) {
        let _ = app.emit_to(PANEL, "guide-overlay", json!({ "animation": "retry", "step": p.step.id(), "line": line }));
    }
}

/// The profiles file failed its integrity check, or passes it again.
fn sync_integrity(app: &AppHandle) {
    if let Some(ok) = engine(app, |e| e.status().integrity_ok) {
        send(app, if ok { Event::Unblocked(Blocked::Integrity) } else { Event::Blocked(Blocked::Integrity) });
    }
}

/// The downloader's state, as the guide needs it: ready, still coming, or not coming.
pub fn on_models(app: &AppHandle, voice_ready: bool, phase: &str) {
    let class = if voice_ready {
        "ready"
    } else if matches!(phase, "failed" | "none") {
        "none"
    } else {
        "coming"
    };
    {
        let state = app.state::<OnboardingState>();
        let mut last = state.models.lock().unwrap_or_else(|p| p.into_inner());
        if *last == Some((voice_ready, class)) {
            return;
        }
        *last = Some((voice_ready, class));
    }
    let (set, clear) = match class {
        "ready" => (None, [Blocked::Download, Blocked::NoModel]),
        "none" => (Some(Blocked::NoModel), [Blocked::Download, Blocked::Download]),
        _ => (Some(Blocked::Download), [Blocked::NoModel, Blocked::NoModel]),
    };
    for reason in clear {
        send(app, Event::Unblocked(reason));
    }
    match set {
        Some(reason) => send(app, Event::Blocked(reason)),
        None => {
            send(app, Event::Unblocked(Blocked::Download));
            send(app, Event::ModelReady);
        }
    }
}

/// The engine has started. Settle what a quit left half done: a demo button whose demo is over is
/// cleared, and one that is due is written.
pub fn engine_ready(app: &AppHandle) {
    let mut progress = current(app);
    let before = progress.clone();
    if in_demo(&progress) {
        if progress.step == StepId::FirstPress && progress.made.is_none() {
            write_demo(app, &mut progress, false);
        }
    } else {
        clear_demo(app, &mut progress);
    }
    if progress != before {
        replace(app, &progress);
    }
    sync_integrity(app);
    play_test_events(app);
}

/// A test switch, like `OASIS_TEST_BELOW_FLOOR`: `OASIS_GUIDE_TEST=try_press,main_focused` plays the
/// named events a few seconds apart once the app is up, so the guide can be checked with no hands
/// on the keyboard. Nothing happens unless the variable is set.
fn play_test_events(app: &AppHandle) {
    let Ok(list) = std::env::var("OASIS_GUIDE_TEST") else { return };
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(7));
        for name in list.split(',').map(str::trim) {
            let event = match name {
                "try_press" => {
                    let _ = onboarding_try_press(app.clone());
                    None
                }
                "device_yes" => Some(Event::DeviceAnswer(true)),
                "device_no" => Some(Event::DeviceAnswer(false)),
                "browser_chosen" => Some(Event::BrowserChosen),
                "popup_closed" => Some(Event::PopupClosed),
                "main_focused" => Some(Event::MainFocused),
                "next" => Some(Event::Next),
                "command_ran" => Some(Event::CommandRan),
                "transcript_typed" => Some(Event::TranscriptTyped),
                "skip_step" => Some(Event::SkipStep),
                "skip_all" => Some(Event::SkipAll),
                "replay" => Some(Event::Replay),
                _ => None,
            };
            if let Some(event) = event {
                send(&app, event);
            }
            std::thread::sleep(Duration::from_secs(5));
        }
    });
}

// ---- windows ----------------------------------------------------------------------------------

/// Whether a request to show OASIS should open the welcome window (the guide is at one of its steps).
pub fn welcome_due(app: &AppHandle) -> bool {
    wants_welcome(&current(app))
}

/// Open the welcome window, or bring it forward if it is already there.
pub fn show_welcome(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WELCOME) {
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, WELCOME, WebviewUrl::App("welcome.html".into()))
        .title("Welcome to OASIS")
        .inner_size(700.0, 500.0)
        .decorations(false)
        .resizable(false)
        .center()
        .build();
    if let Err(why) = built {
        // Never leave the person with no window at all.
        eprintln!("The welcome window could not open: {why}");
        crate::show_main(app);
    }
}

/// The caption panel: made once, hidden, low in the middle of the main screen. It never takes a
/// click or the keyboard, so what is dictated cannot land in it.
pub fn create_panel(app: &AppHandle) -> tauri::Result<()> {
    let (w, h) = PANEL_SIZE;
    let panel = WebviewWindowBuilder::new(app, PANEL, WebviewUrl::App("guide.html".into()))
        .title("OASIS guide")
        .inner_size(w, h)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focusable(false)
        .focused(false)
        .visible(false)
        .build()?;
    panel.set_ignore_cursor_events(true)?;
    if let Some(screen) = app.primary_monitor()? {
        let scale = screen.scale_factor();
        let x = f64::from(screen.position().x) + (f64::from(screen.size().width) - w * scale) / 2.0;
        // clear of the taskbar
        let y = f64::from(screen.position().y) + f64::from(screen.size().height) - (h + 72.0) * scale;
        panel.set_position(PhysicalPosition::new(x.round(), y.round()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_run_starts_the_guide_whether_or_not_profiles_were_already_there() {
        assert_eq!(first_run(true, None), guide::fresh());
        assert_eq!(first_run(false, None), guide::fresh(), "left-over profiles, no record");
    }

    #[test]
    fn a_damaged_file_means_no_guide_so_the_keys_are_never_held() {
        let progress = first_run(false, Some(from_text("{ not json")));
        assert_eq!(progress.status, Status::Dismissed);
        assert!(!guide::holds_keys(&progress));
        assert!(from_text(r#"{"version":1,"status":"no_such_status","step":"welcome"}"#).is_none());
    }

    #[test]
    fn progress_without_its_profiles_starts_again() {
        let saved = Progress { status: Status::Complete, step: StepId::Done, ..guide::fresh() };
        assert_eq!(first_run(true, Some(Some(saved.clone()))), guide::fresh());
        assert_eq!(first_run(false, Some(Some(saved.clone()))), saved);
    }

    #[test]
    fn progress_survives_being_saved_and_read_and_never_keeps_what_blocks_it() {
        let progress = Progress {
            status: Status::WaitingForVoice,
            step: StepId::VoiceKey,
            has_device: Some(true),
            skipped: vec![StepId::Device],
            blocked: vec![Blocked::Download],
            made: Some(Made { kind: "OpenUrl".into(), value: SITE.into() }),
            voice_guide_ran: false,
            notice_shown: true,
        };
        let text = serde_json::to_string(&to_saved(&progress)).unwrap();
        assert!(!text.contains("download"), "live conditions are not saved: {text}");
        assert_eq!(from_text(&text), Some(Progress { blocked: Vec::new(), ..progress }));
    }

    #[test]
    fn only_notepad_counts_for_the_voice_steps() {
        let (open, other) = ("Open Notepad".to_string(), "open chrome".to_string());
        assert!(mentions(&[&other, &open], "notepad"));
        assert!(!mentions(&[&other, &other], "notepad"));
        let close = "CLOSE NOTEPAD".to_string();
        assert!(mentions(&[&close], "close") && !mentions(&[&open], "close"), "open and close are told apart");
        assert!(is_notepad("Notepad") && is_notepad("notepad++"));
        assert!(!is_notepad("chrome") && !is_notepad(""));
    }

    #[test]
    fn the_practice_step_is_saved_and_read_by_name() {
        let progress = Progress { status: Status::Running, step: StepId::TryCommands, ..guide::fresh() };
        let text = serde_json::to_string(&to_saved(&progress)).unwrap();
        assert!(text.contains("try_commands"), "{text}");
        assert_eq!(from_text(&text), Some(progress));
    }

    #[test]
    fn a_step_from_a_newer_build_starts_that_part_again() {
        let progress = from_text(r#"{"version":2,"status":"running","step":"a_new_step"}"#).unwrap();
        assert_eq!(progress.step, StepId::Welcome);
    }
}
