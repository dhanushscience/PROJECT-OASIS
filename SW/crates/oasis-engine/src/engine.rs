//! The loop. See the crate docs.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant as StdInstant};

use oasis_app::{ArbiterHost, PadLink};
use oasis_core::bridge::{ConfigWhen, YouTubeWatchState};
use oasis_core::voice::command::decide::{decide, launch_target_exists, Decision, Facts};
use oasis_core::voice::command::RouterSettings;
use oasis_core::voice::command::safety::steps_of;
use oasis_core::voice::command::{route, RunningApp};
use oasis_core::voice::VoiceEvent;
use oasis_core::speech::pipeline as cleanup_pipeline;
use oasis_core::speech::style::Style;
use oasis_core::speech::{CleanupContext, CleanupRequest, GlossaryEntry};
use oasis_core::actions::{run_action, ActionBackend, ActionKind, ActionOutcome, ButtonAction};
use oasis_core::input::{ArbiterContext, ArbiterEffect, GestureClaimId, LabelPolicy, PadEvent};
use oasis_core::matching::ForegroundContext;
use oasis_core::Instant;
use oasis_platform::browser_bridge::{write_token_files, BridgeEvent, BridgeServer};
use oasis_platform::extension_install;
use oasis_platform::foreground::{ForegroundTracker, WindowSource};
use oasis_platform::pad_connector::{ConnectorEvent, PadConnector};
use oasis_platform::paths::OasisPaths;
use oasis_platform::ports::{ButtonSource, ForegroundApp, ForegroundInfo, PadTransport, PlatformResult};
use oasis_platform::profile_store::{LoadOutcome, ProfileStore, SaveError, SettingsIntegrity};
use oasis_platform::security_log::SecurityLog;
use oasis_platform::trusted_device::TrustedDevice;
use oasis_settings::arbiter_bridge::arbiter_config;
use oasis_settings::edit::EditError;
use oasis_settings::matcher::{has_panel_profiles, has_url_based_profiles, matching_profile, resolve_active_profile};
use oasis_settings::youtube::{bridge_enabled, bridge_port, extension_settings, gestures_active};
use oasis_settings::input::InputSettings;
use oasis_settings::voice::VoiceSettings;
use oasis_settings::voice_commands::{
    approval_mode, lexicon as voice_lexicon, router_settings as voice_router_settings, trusted_targets,
};
use oasis_settings::Settings;

use crate::reminders::Reminders;
use crate::use_counts::UseCounts;
use oasis_core::voice::command::reminders::{self as phrases, LocalTime};
use crate::events::{EngineEvent, EngineStatus, FrontApp, Press};
use crate::inbox::{FrontAsk, FrontSnapshot, Input};
use crate::voice::{Heard, Voice, VoiceContext, VoiceParts};

// The engine's features, one file each in `engine/`. They share this module's names (`use super::*`).
mod bridge;
mod dictation;
mod dispatch;
mod handle;
mod layouts;
mod parts;
mod spoken;
mod worker;

pub use handle::{start, EngineHandle};
pub use parts::{
    BackendContext, BackendFactory, BridgeConfig, ChangeError, EngineConfig, EngineError, EngineParts, ForegroundProbe,
};
pub(crate) use dictation::dictation_style;
use self::{bridge::*, dictation::*, dispatch::*, handle::*, layouts::*, spoken::*, worker::*};

/// How often the loop looks at the voice session while it records or transcribes: the loudness
/// for the pill (every 50 ms), pieces of the recording (every 250 ms) and finished text.
const VOICE_TICK: Duration = Duration::from_millis(10);

/// How often the loop looks for an outside edit of `profiles.json`.
/// Used only when the folder watcher could not start.
const DISK_CHECK_EVERY: Duration = Duration::from_millis(1500);
/// With the watcher running, a look now and then in case Windows missed a change (a network drive).
const DISK_SAFETY_NET: Duration = Duration::from_secs(30);

// ---- the loop --------------------------------------------------------------------------------

/// The optional parts of the loop, bundled so it takes few arguments.
struct LoopExtras {
    bridge_config: Option<BridgeConfig>,
    voice_parts: Option<VoiceParts>,
    startup_setup_at: Option<StdInstant>,
    buttons: Option<Box<dyn ButtonSource>>,
}

/// A press or label peek waiting for the front thread's read of what is in front.
enum Need {
    Tap(Gesture),
    /// Show the labels for `duration`; `only_if_clear`: not if labels already show (the YouTube near-end peek).
    Peek { duration: Duration, watch_token: String, only_if_clear: bool },
    /// A finished recording: typed or run against what is in front when it was handed over.
    Heard(Heard),
}

/// A capture the gesture rules began, waiting for "typed or a command?" from the front thread. Its
/// voice requests wait with it, in order; the microphone is already open (the pre-roll).
struct HeldCapture {
    requests: Vec<VoiceEvent>,
    /// The start of the press line for the timing log; the focus time is added with the answer.
    press_line: Option<String>,
    /// When it began: the answer is not waited for past `CAPTURE_WAIT` (see the loop).
    since: StdInstant,
}

/// The longest a recording's start waits for the front thread (UI Automation can hang on a
/// misbehaving program). Then it is typed, and the release that queued behind it goes through.
const CAPTURE_WAIT: Duration = Duration::from_millis(1500);

fn run_loop<T: PadTransport + 'static>(
    shared: &Arc<Shared>,
    connector: PadConnector<T>,
    foreground: Box<dyn ForegroundProbe>,
    jobs: &Jobs,
    inbox: Receiver<Input>,
    extras: LoopExtras,
) {
    let LoopExtras { bridge_config, mut voice_parts, mut startup_setup_at, buttons } = extras;
    let mut seen = shared.generation.load(Ordering::SeqCst);
    let mut settings = shared.settings().clone();
    let mut verified = shared.integrity_ok.load(Ordering::SeqCst);
    let start = StdInstant::now();

    // Looking for the pad holds a thread for up to 1.5 s, so the pad has a thread of its own.
    let (power, power_rx) = mpsc::channel::<bool>();
    let pad_thread = crate::inbox::spawn_pad(connector, start, power_rx, shared.inbox.clone());
    // Reading what is in front goes through UI Automation and can take a second: only the front
    // thread does it. The loop asks it and handles the answer (presses, peeks, spoken phrases).
    let focus_probe = voice_parts
        .as_mut()
        .map(|p| std::mem::replace(&mut p.editable_focus, Box::new(|| false)))
        .unwrap_or_else(|| Box::new(|| false));
    let (front, front_rx) = mpsc::channel::<FrontAsk>();
    let front_thread = crate::inbox::spawn_front(foreground, focus_probe, front_rx, shared.inbox.clone());

    // Which ways of pressing the buttons are on: the pad, and the keyboard.
    let mut buttons = buttons;
    if let Some(source) = buttons.as_mut() {
        let inbox = shared.inbox.clone();
        source.set_wake(Arc::new(move || {
            let _ = inbox.send(Input::Wake);
        }));
    }
    let mut input = InputSettings::from(&settings);
    let mut hardware_was_on = true;
    let mut held = shared.keys_held.load(Ordering::SeqCst);
    apply_input(&mut buttons, &input, shared);

    // Push-to-talk needs voice switched on and a speech model present; the gesture rules only
    // arm it then.
    let mut voice = voice_parts.map(Voice::new);
    let mut voice_ready = voice.as_ref().is_some_and(|v| v.ready(&settings));
    report_voice_state(shared, voice.as_ref(), &settings);
    // Load the speech model now, not on the first recording (VOICE_DECISIONS.md V9).
    if let Some(voice) = voice.as_mut() {
        voice.warm(&settings);
    }

    let mut link = PadLink::new();
    let mut arbiter = ArbiterHost::new(arbiter_config(&settings, voice_ready), true);
    let mut context = ArbiterContext::desktop();
    context.execution_allowed = verified;
    arbiter.set_context(context);
    let _ = front.send(FrontAsk::Profiles {
        url: has_url_based_profiles(&settings.profiles),
        panel: has_panel_profiles(&settings.profiles),
    });

    // When the labels now on screen go away, if any.
    let mut peek_until: Option<Instant> = None;
    // A pad button was let go during this pass (see the push-to-talk pre-roll).
    let mut pending_release = false;
    // When the talk button last went down and how long the pre-roll took to open the mic, for the
    // press line in the timing log.
    let mut talk_down: Option<(StdInstant, u64)> = None;
    // "Voice is getting ready: 48%" is on the overlay (the talk button was pressed during the download).
    let mut getting_ready_shown = false;
    // A spoken command waiting for the pad to say yes or no.
    let mut pending: Option<Pending> = None;
    // Presses and peeks asked of the front thread, oldest first; its answers come in this order.
    let mut asked: VecDeque<Need> = VecDeque::new();
    let mut capture: Option<HeldCapture> = None;
    // What is in front, as the front thread last saw it (at most `FRONT_WATCH_EVERY` old): only for
    // the YouTube gestures, which must be judged the moment a button goes down.
    let mut front_now = ForegroundInfo::default();

    // The browser extension: what YouTube is doing decides which set of buttons a profile shows,
    // and whether the hidden YouTube gestures are live.
    let mut bridge: Option<(BridgeServer, u16)> = None;
    sync_bridge(shared, bridge_config.as_ref(), &settings, &mut bridge);
    crate::polish::sync(&shared.paths.models_folder(), &settings);
    sync_preferences(&settings);
    let mut watch = YouTubeWatchState::default();
    let mut last_near_end = String::new();

    // profiles.json is checked for outside edits (the C# app, a text editor) when Windows reports a
    // write in its folder. Without the watcher the check falls back to a couple of times a second.
    #[cfg(windows)]
    let settings_watch = {
        let inbox = shared.inbox.clone();
        let file = shared.paths.profiles();
        let started = match file.parent() {
            Some(dir) => oasis_win::watch::FolderWatch::start(dir, move || {
                let _ = inbox.send(Input::SettingsFileChanged);
            })
            .map_err(|e| e.to_string()),
            None => Err("no folder".to_string()),
        };
        started
            .map_err(|why| {
                SecurityLog::at(shared.paths.security_log())
                    .write(&format!("The settings folder cannot be watched ({why}), so profiles.json is checked every 1.5 seconds."));
            })
            .ok()
    };
    #[cfg(not(windows))]
    let settings_watch: Option<()> = None;
    let disk_every = if settings_watch.is_some() { DISK_SAFETY_NET } else { DISK_CHECK_EVERY };
    let mut next_disk_check = StdInstant::now() + disk_every;
    let mut last_tried = None;
    let mut next_reminder_check = StdInstant::now();
    let mut last_front: Option<ForegroundInfo> = None;
    let mut next_meeting_check = StdInstant::now();
    let mut next_meeting_refresh = StdInstant::now();
    // The first memory line a minute after start, when the models have loaded.
    let mut next_memory_log = StdInstant::now() + Duration::from_secs(60);

    // A deadline as a clock time, rounded up to the whole millisecond `now` is counted in, so the
    // loop never wakes a moment early and finds nothing due.
    let at = |time: Instant| start + Duration::from_millis(u64::try_from(time.0).unwrap_or(0).div_ceil(1_000_000));
    while !shared.stop.load(Ordering::SeqCst) {
        // Sleep until something arrives or the next thing is due. The checks below run at least
        // once a second, so there is always a deadline.
        let voice_busy = voice.as_ref().is_some_and(|v| !v.is_idle());
        let due = [
            Some(next_meeting_check),
            Some(next_reminder_check),
            Some(next_disk_check),
            Some(next_memory_log),
            startup_setup_at,
            arbiter.timers().next_deadline().map(at),
            peek_until.map(at),
            pending.as_ref().map(|ask| at(ask.until)),
            voice_busy.then(|| StdInstant::now() + VOICE_TICK),
        ]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or_else(StdInstant::now);
        let mut inputs: Vec<Input> = Vec::new();
        match inbox.recv_timeout(due.saturating_duration_since(StdInstant::now())) {
            Ok(input) => inputs.push(input),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        inputs.extend(inbox.try_iter());
        if shared.stop.load(Ordering::SeqCst) {
            break;
        }

        if startup_setup_at.is_some_and(|at| StdInstant::now() >= at) {
            startup_setup_at = None;
            if verified && shared.live.load(Ordering::SeqCst) {
                if let Some(setup) = oasis_settings::quick_launch::startup_setup(&settings) {
                    let action = ButtonAction { kind: ActionKind::QuickLaunch.as_str().into(), value: setup.id, steps: vec![] };
                    let _ = jobs.send(Job { action, button: -1, what: "start-up setup".into(), allow_run_script: false, prepare: None });
                }
            }
        }
        if StdInstant::now() >= next_meeting_check {
            next_meeting_check = StdInstant::now() + Duration::from_secs(2);
            if let Some(meetings) = shared.meetings.clone() {
                let now = crate::spotify::now_unix();
                if StdInstant::now() >= next_meeting_refresh {
                    next_meeting_refresh = StdInstant::now() + Duration::from_secs(60);
                    // The network can be slow: ask on a thread of its own.
                    let background = Arc::clone(&meetings);
                    thread::spawn(move || background.refresh(now));
                }
                let section = oasis_settings::meetings::MeetingSettings::from(&settings);
                if let Some(due) = meetings.due_reminder(now, section.reminders_enabled, section.reminder_minutes_before) {
                    let offset = local_now().offset_secs;
                    shared.emit(EngineEvent::MeetingReminder {
                        title: due.title.clone(),
                        when: oasis_core::meetings::range_text(&due, offset),
                        message: oasis_core::meetings::reminder_text(&due, now),
                        can_join: !due.join_url.trim().is_empty(),
                    });
                }
            }
        }
        if StdInstant::now() >= next_reminder_check {
            next_reminder_check = StdInstant::now() + Duration::from_secs(1);
            fire_due_reminders(shared, jobs);
        }
        if StdInstant::now() >= next_disk_check {
            next_disk_check = StdInstant::now() + disk_every;
            let changed = shared.store().changed_on_disk();
            if changed.is_some() && changed != last_tried {
                last_tried = changed;
                match shared.reload_from_disk() {
                    Ok(()) => shared.emit(EngineEvent::Note {
                        text: "profiles.json was changed by another program, so OASIS loaded it again.".into(),
                    }),
                    Err(e) => shared.emit(EngineEvent::Note {
                        text: format!("profiles.json changed but could not be read, so the old settings stay: {e}"),
                    }),
                }
            }
        }
        // New settings were saved or reloaded: take them up before the next event.
        let generation = shared.generation.load(Ordering::SeqCst);
        if generation != seen {
            seen = generation;
            settings = shared.settings().clone();
            let chosen = InputSettings::from(&settings);
            let held_now = shared.keys_held.load(Ordering::SeqCst);
            if chosen != input || held_now != held {
                input = chosen;
                held = held_now;
                apply_input(&mut buttons, &input, shared);
            }
            verified = shared.integrity_ok.load(Ordering::SeqCst);
            voice_ready = voice.as_ref().is_some_and(|v| v.ready(&settings));
            report_voice_state(shared, voice.as_ref(), &settings);
            if let Some(voice) = voice.as_mut() {
                voice.warm(&settings);
            }
            arbiter.set_config(arbiter_config(&settings, voice_ready));
            context.execution_allowed = verified;
            arbiter.set_context(context);
            let _ = front.send(FrontAsk::Profiles {
                url: has_url_based_profiles(&settings.profiles),
                panel: has_panel_profiles(&settings.profiles),
            });
            // Matched again against the profiles in force, so a profile made while OASIS is in
            // front is the one for that app straight away.
            report_front(shared, &settings, last_front.as_ref());
            sync_bridge(shared, bridge_config.as_ref(), &settings, &mut bridge);
            crate::polish::sync(&shared.paths.models_folder(), &settings);
            sync_preferences(&settings);
        }
        // With the pad off the pad thread lets go of it and stops looking.
        if input.hardware != hardware_was_on {
            hardware_was_on = input.hardware;
            let _ = power.send(input.hardware);
            let status = if input.hardware { "Scanning" } else { "Off" };
            shared.update(|s| s.pad = status.into());
            shared.emit(EngineEvent::PadStatus { status: status.into() });
        }

        let now = Instant::from_millis(start.elapsed().as_millis() as i64);
        let mut connector_events = Vec::new();
        let mut answers = Vec::new();
        let mut captured = None;
        for input in inputs {
            match input {
                Input::Wake => {}
                // OASIS's own saves land here too: `changed_on_disk` knows them and says nothing.
                Input::SettingsFileChanged => next_disk_check = StdInstant::now(),
                Input::Pad(event) => connector_events.push(event),
                Input::Front(info) => {
                    // Remember the last app that was not OASIS: the window offers to make it a profile.
                    if !is_oasis(&info) && !info.process.is_empty() {
                        last_front = Some(info.clone());
                        report_front(shared, &settings, last_front.as_ref());
                    }
                    front_now = info;
                }
                Input::ForPress(info) => answers.push(info),
                Input::Capture { dictate, focus_ms } => captured = Some((dictate, focus_ms)),
                // Not while another question is up or a recording is going: the offer is simply dropped.
                Input::MicOffer(name) if pending.is_none() && !voice_busy => {
                    context.confirmation_pending = true;
                    arbiter.set_context(context);
                    shared.emit(EngineEvent::MicOffer { name: name.clone() });
                    pending = Some(Pending {
                        action: ButtonAction::default(),
                        steps: Vec::new(),
                        summary: String::new(),
                        heard: name.clone(),
                        label: String::new(),
                        until: now.plus(MIC_OFFER_TIMEOUT),
                        mic: Some(name),
                    });
                }
                Input::MicOffer(_) => {}
                Input::YouTube(event) => {
                    if oasis_core::bridge::youtube_watch::near_end_peek(&event, &mut last_near_end) && peek_until.is_none() && verified {
                        let _ = front.send(FrontAsk::Press { read: labels_possible(&settings) });
                        let watch_token = watch.current_when_token().to_string();
                        asked.push_back(Need::Peek { duration: Duration::from_secs(12), watch_token, only_if_clear: true });
                    }
                    if let Some(body) = oasis_core::bridge::youtube_watch::ended_notice(&event) {
                        shared.emit(EngineEvent::Notice { title: "YouTube ended".into(), body });
                    }
                    if watch.apply(&event) {
                        let (token, title) = (watch.current_when_token().to_string(), watch.title.clone());
                        shared.update(|s| {
                            s.youtube = token.clone();
                            s.youtube_title = title.clone();
                        });
                        shared.emit(EngineEvent::YouTube { token, title });
                    }
                }
            }
        }
        let watch_token = watch.current_when_token();

        let mut effects: Vec<ArbiterEffect> = Vec::new();

        if peek_until.is_some_and(|until| now >= until) {
            peek_until = None;
            context.labels_on_screen = false;
            arbiter.set_context(context);
            shared.emit(EngineEvent::HideLabels);
        }

        // What went down or up this turn, from the pad and from the keyboard, judged together.
        let mut pad_events: Vec<PadEvent> = Vec::new();
        for event in connector_events {
            match event {
                // A look that was under way when the pad was switched off must not say "Connected".
                ConnectorEvent::Status(_) | ConnectorEvent::UnavailableReason(_) if !input.hardware => {}
                ConnectorEvent::Status(status) => {
                    shared.update(|s| s.pad = status.as_str().to_string());
                    shared.emit(EngineEvent::PadStatus { status: status.as_str().to_string() });
                }
                ConnectorEvent::UnavailableReason(reason) => {
                    shared.update(|s| s.last_error = Some(reason.clone()));
                    shared.emit(EngineEvent::PadUnavailable { reason });
                }
                ConnectorEvent::Connected { name } => {
                    shared.update(|s| s.pad_name = Some(name.clone()));
                    shared.emit(EngineEvent::PadConnected { name });
                }
                ConnectorEvent::Rssi(_) => {}
                ConnectorEvent::Disconnected => {
                    shared.update(|s| {
                        s.pad_name = None;
                        s.battery_percent = None;
                        s.charging = false;
                        s.latency_ms = None;
                    });
                    shared.emit(EngineEvent::PadLost);
                    shared.low_battery_warned.store(false, Ordering::SeqCst);
                    pad_events.push(link.on_disconnected());
                }
                ConnectorEvent::Notification(bytes) => {
                    let (events, telemetry_changed) = link.on_notification(&bytes, now);
                    pad_events.extend(events);
                    if telemetry_changed {
                        report_telemetry(shared, &link);
                    }
                }
            }
        }
        // The keyboard's buttons join the pad's.
        if let Some(source) = buttons.as_mut() {
            for (button, down) in source.drain() {
                let id = oasis_core::ButtonId(i32::from(button));
                pad_events.push(if down { PadEvent::Down(id, now) } else { PadEvent::Up(id, now) });
            }
        }
        // The pad and the keyboard's buttons run their actions on every window, OASIS's own included.
        for pad_event in pad_events {
            match pad_event {
                PadEvent::Down(b, _) => {
                    shared.emit(EngineEvent::PadButton {
                        button: (b.0 + 1) as u8,
                        down: true,
                    });
                    // Hold-2× and double-click volume are live only on a playing
                    // YouTube watch page in front, so look before the press is judged.
                    let active = is_youtube_gesture_button(&settings, b.0)
                        && gestures_active(&settings, &watch, &ForegroundContext::from(&front_now));
                    if context.youtube_gestures_active != active {
                        context.youtube_gestures_active = active;
                        arbiter.set_context(context);
                    }
                    // Voice is on but its model is still downloading: say so instead of nothing.
                    if !voice_ready && is_talk_button(&settings, b.0) && VoiceSettings::from(&settings).enabled {
                        if let Some(percent) = crate::model_download::progress() {
                            shared.emit(EngineEvent::Voice { phase: format!("getting_ready:{percent}") });
                            getting_ready_shown = true;
                        }
                    }
                    // Start listening before the hold is recognised, so the first
                    // words are not lost.
                    if let (true, Some(voice)) = (voice_ready && is_talk_button(&settings, b.0), voice.as_mut()) {
                        // Wakes an idle Bluetooth speaker during the hold, so the start cue (which has
                        // no lead-in of its own any more) is heard the moment the hold is recognised.
                        cue(&settings, VOICE_WAKE);
                        let down = StdInstant::now();
                        voice.preroll(&settings);
                        talk_down = Some((down, down.elapsed().as_millis() as u64));
                    }
                }
                PadEvent::Up(b, _) => {
                    shared.emit(EngineEvent::PadButton {
                        button: (b.0 + 1) as u8,
                        down: false,
                    });
                    if std::mem::take(&mut getting_ready_shown) {
                        shared.emit(EngineEvent::Voice { phase: "idle".into() });
                    }
                    pending_release = true;
                }
                _ => {}
            }
            arbiter.apply(pad_event, now, None, None, &mut |e| effects.push(e.clone()));
        }
        arbiter.fire_due(now, &mut |e| effects.push(e.clone()));

        // Push-to-talk: begin or end a capture as the gesture rules say. Whether this recording is
        // typed or run as a command is decided once, from the program and focus in front, on the
        // front thread; the capture waits for that answer.
        let mut voice_requests: Vec<VoiceEvent> = Vec::new();
        let begins = effects.iter().any(|e| matches!(e, ArbiterEffect::StartVoiceCapture));
        if let (true, Some(voice)) = (begins, voice.as_ref()) {
            // The start sound and the pill come first, so the user knows at once they can talk:
            // the focus question can take up to 250 ms and pausing media longer still.
            // "listening" and "dictating" look the same; the session's own event follows.
            if voice_ready {
                cue(&settings, VOICE_IN);
            }
            let cue_at = StdInstant::now();
            if voice_ready && voice.is_idle() {
                shared.emit(EngineEvent::Voice { phase: "listening".into() });
            }
            let shown_at = StdInstant::now();
            let _ = front.send(FrontAsk::Capture(Box::new(settings.clone())));
            let press_line = talk_down.take().map(|(down, mic_open_ms)| {
                let since = |at: StdInstant| at.saturating_duration_since(down).as_millis();
                format!("press cue_ms={} hud_ms={} mic_open_ms={mic_open_ms}", since(cue_at), since(shown_at))
            });
            // Not replaced while one still waits: that would drop its queued release.
            if capture.is_none() {
                capture = Some(HeldCapture { requests: Vec::new(), press_line, since: StdInstant::now() });
            }
        }
        effects.retain(|effect| match effect {
            ArbiterEffect::StartVoiceCapture => {
                voice_requests.push(VoiceEvent::BeginRequested {
                    at: now,
                    enabled: voice_ready,
                    model_ready: voice_ready,
                    // Set when the front thread answers (see `HeldCapture`).
                    dictate: true,
                });
                false
            }
            ArbiterEffect::EndVoiceCapture => {
                voice_requests.push(VoiceEvent::EndRequested { at: now });
                false
            }
            ArbiterEffect::CancelVoiceCapture => {
                voice_requests.push(VoiceEvent::CancelRequested);
                false
            }
            _ => true,
        });
        if let Some(voice) = voice.as_mut() {
            let emit = |event: EngineEvent| shared.emit(event);
            let youtube_playing = watch_token == ConfigWhen::PLAYING;
            let ctx = VoiceContext {
                emit: &emit,
                settings: &settings,
                bridge: bridge.as_ref().map(|(server, _)| server),
                youtube_playing,
                log: &|line| SecurityLog::at(shared.paths.voice_timing_log()).write(line),
            };
            match capture.as_mut() {
                Some(waiting) => waiting.requests.append(&mut voice_requests),
                None => {
                    for request in voice_requests {
                        voice.request(request, now, &ctx);
                    }
                }
            }
            let timed_out = capture.as_ref().is_some_and(|c| c.since.elapsed() >= CAPTURE_WAIT);
            if captured.is_none() && timed_out {
                captured = Some((true, CAPTURE_WAIT.as_millis()));
            }
            if let Some((dictate, focus_ms, waiting)) = captured.and_then(|(d, f)| Some((d, f, capture.take()?))) {
                // One line per capture start: where the time between the press and the feedback went.
                if let Some(line) = waiting.press_line {
                    SecurityLog::at(shared.paths.voice_timing_log()).write(&format!("{line} focus_ms={focus_ms}"));
                }
                for mut request in waiting.requests {
                    if let VoiceEvent::BeginRequested { dictate: typed, .. } = &mut request {
                        *typed = dictate;
                    }
                    voice.request(request, now, &ctx);
                }
                // A double-click lock: tell the pill to show the tick and cross. It stays "locked" on
                // the page until the phase goes to thinking or idle.
                if matches!(arbiter.state(), oasis_core::input::PadState::VoiceLocked) {
                    shared.emit(EngineEvent::Voice { phase: "locked".into() });
                }
            }
            // A button was let go without the hold becoming a capture: drop the pre-roll. Not while a
            // capture waits for its answer: the pre-roll is its recording.
            if capture.is_none() && std::mem::take(&mut pending_release) {
                voice.discard_preroll();
            }
            crate::activity::set_voice_active(!voice.is_idle());
            for heard in voice.tick(now, &ctx) {
                let (timing, dictate, words) = (heard.timing, heard.dictate, heard.text.split_whitespace().count());
                let text_ms = timing.released.map(|r| r.elapsed().as_millis());
                log_voice_timing(shared, &settings, timing, dictate, words, text_ms);
                // The phrase is matched against what is in front, read fresh (a spoken command uses
                // the active profile's labels). That read can take a second, so the front thread does
                // it; the phrase waits in line with the presses, so their order is kept.
                let _ = front.send(FrontAsk::Press { read: true });
                asked.push_back(Need::Heard(heard));
            }
        }
        // The polish server is stopped after 10 idle minutes (frees ~1.2 GB) and started at the next press.
        for line in crate::polish::tick(voice.as_ref().is_some_and(|v| !v.is_idle())) {
            SecurityLog::at(shared.paths.voice_timing_log()).write(&line);
        }
        if StdInstant::now() >= next_memory_log {
            next_memory_log = StdInstant::now() + MEMORY_LOG_EVERY;
            let own = oasis_win::job::usage(std::process::id());
            let polish = crate::polish::server_pid().and_then(oasis_win::job::usage);
            SecurityLog::at(shared.paths.voice_timing_log()).write(&memory_line(own, polish));
        }

        // An unanswered question is dropped after a while, never left hanging over the pad.
        if pending.as_ref().is_some_and(|ask| now >= ask.until) {
            let timed_out = pending.take();
            context.confirmation_pending = false;
            arbiter.set_context(context);
            shared.emit(EngineEvent::ConfirmEnd { approved: false });
            if timed_out.is_some_and(|ask| ask.mic.is_none()) {
                shared.emit(EngineEvent::Note { text: "The voice command was not confirmed in time, so it was cancelled.".into() });
            }
        }

        for effect in effects {
            match effect {
                ArbiterEffect::DispatchCommand { button, claim: GestureClaimId::Confirmation } => {
                    // The pad answers a spoken command's question: button 1 runs it, button 2
                    // cancels, anything else is ignored.
                    if let Some(ask) = pending.take_if(|_| matches!(button.0, 0 | 1)) {
                        let approved = button.0 == 0;
                        context.confirmation_pending = false;
                        arbiter.set_context(context);
                        shared.emit(EngineEvent::ConfirmEnd { approved });
                        if let Some(name) = ask.mic {
                            if approved {
                                let ctx = oasis_settings::fields::FieldContext { microphones: vec![name.clone()], ..Default::default() };
                                let _ = shared.change(|s| oasis_settings::fields::set_field(s, "voice.microphone", serde_json::json!(name), &ctx));
                            }
                        } else if approved {
                            let _ = shared.change(|s| {
                                s.trust_voice_targets(&ask.steps);
                                Ok(())
                            });
                            run_voice_action(shared, jobs, &ask.action, &ask.label, &ask.heard);
                        } else {
                            shared.emit(EngineEvent::VoiceCommand {
                                heard: ask.heard,
                                label: ask.label,
                                outcome: "cancelled".into(),
                                detail: "Cancelled.".into(),
                            });
                        }
                    }
                }
                ArbiterEffect::DispatchCommand { button, claim } => {
                    // A tapped button's profile is the one for what is in front at the press, read
                    // fresh (the panel can change inside one window); the answer is handled below.
                    let _ = front.send(FrontAsk::Press { read: claim == GestureClaimId::BoundTap });
                    asked.push_back(Need::Tap(Gesture { button: button.0, claim, watch_token: watch_token.to_string() }));
                }
                ArbiterEffect::SendToDevice(command) => {
                    if let Some((server, _)) = &bridge {
                        server.send_command(command);
                    }
                }
                ArbiterEffect::ShowPeek(duration) => {
                    let _ = front.send(FrontAsk::Press { read: labels_possible(&settings) });
                    asked.push_back(Need::Peek { duration, watch_token: watch_token.to_string(), only_if_clear: false });
                }
                _ => {}
            }
        }

        // The front thread's reads, in the order the presses and peeks asked for them.
        for info in answers {
            let Some(need) = asked.pop_front() else { continue };
            match need {
                Need::Tap(gesture) => {
                    // First tap reveals the labels; the second one (while they show) runs.
                    let reveal = (gesture.claim == GestureClaimId::BoundTap && peek_until.is_none() && verified)
                        .then(|| info.as_ref().and_then(|info| peek_labels(&settings, info, Some(gesture.button), &gesture.watch_token)))
                        .flatten();
                    if let Some((profile, labels)) = reveal {
                        let peek = Duration::from_secs(4);
                        peek_until = Some(now.plus(peek));
                        context.labels_on_screen = true;
                        arbiter.set_context(context);
                        shared.emit(EngineEvent::ShowLabels {
                            profile,
                            labels,
                            millis: peek.as_millis() as u64,
                        });
                        continue;
                    }
                    // Running something ends the label peek.
                    if peek_until.take().is_some() {
                        context.labels_on_screen = false;
                        arbiter.set_context(context);
                        shared.emit(EngineEvent::HideLabels);
                    }
                    dispatch(shared, &settings, verified, info.as_ref(), jobs, gesture);
                }
                Need::Peek { duration, watch_token, only_if_clear } => {
                    if only_if_clear && (peek_until.is_some() || !verified) {
                        continue;
                    }
                    if let Some((profile, labels)) = info.as_ref().and_then(|info| peek_labels(&settings, info, None, &watch_token)) {
                        peek_until = Some(now.plus(duration));
                        context.labels_on_screen = true;
                        arbiter.set_context(context);
                        shared.emit(EngineEvent::ShowLabels { profile, labels, millis: duration.as_millis() as u64 });
                    }
                }
                Need::Heard(heard) => {
                    let Some(voice) = voice.as_ref() else { continue };
                    let spoken = SpokenContext { shared, settings: &settings, verified, jobs, watch_token };
                    let installed = |name: &str| voice.app_installed(name);
                    let front = info.as_ref().map(|i| ForegroundContext::from(i).app_name).unwrap_or_default();
                    let mut snapshot = FrontSnapshot(info);
                    if let Some(ask) = handle_heard(&spoken, &mut snapshot, voice.running_apps(), &installed, &|| voice.on_screen(), &|| voice.app_names(), heard, now) {
                        context.confirmation_pending = true;
                        arbiter.set_context(context);
                        shared.emit(EngineEvent::Confirm { summary: ask.summary.clone(), close: close_target(&ask.action, &front) });
                        pending = Some(ask);
                    }
                }
            }
        }
    }
    arbiter.shutdown();
    if let Some(mut voice) = voice.take() {
        voice.shutdown();
    }
    if let Some((mut server, _)) = bridge.take() {
        server.stop();
    }
    crate::polish::stop();
    // Ends the pad and front threads (each finishes the read it is in).
    drop((power, front, inbox, settings_watch));
    let _ = pad_thread.join();
    let _ = front_thread.join();
}

/// The app in front and the profile it matches, for the status (the window's "just used").
fn report_front(shared: &Shared, settings: &Settings, front: Option<&ForegroundInfo>) {
    let Some(info) = front else { return };
    let profile = matching_profile(&settings.profiles, &ForegroundContext::from(info));
    let front = FrontApp { process: info.process.clone(), title: info.window_title.clone() };
    shared.update(|s| {
        s.front_app = Some(front);
        s.front_profile = profile;
    });
}

/// Take up the choice of input methods: the keyboard listener on or off, with its shortcuts.
fn apply_input(buttons: &mut Option<Box<dyn ButtonSource>>, input: &InputSettings, shared: &Shared) {
    // A listener that sees every key is worth a line in the security log whenever it is on or off.
    // Held back (a guide is on screen) counts as off for the listener and the log.
    let held = shared.keys_held.load(Ordering::SeqCst);
    let keyboard = input.keyboard && !held;
    if let Some(source) = buttons.as_mut() {
        let log = SecurityLog::at(shared.paths.security_log());
        log.write(if keyboard {
            "Keyboard input switched on (listening for the chosen shortcuts only)."
        } else if input.keyboard {
            "Keyboard input is held back while the guide is on screen (not listening)."
        } else {
            "Keyboard input is off."
        });
        source.configure(keyboard, input.keys.clone(), input.exclusive);
        if let Some(text) = source.problem() {
            shared.emit(EngineEvent::Note { text });
        }
    }
}

/// One line per recording in `voice-timing.log`: where the wait after letting go of the button went.
/// Durations and counts only, never what was said. `text_ms`: release until the text was ready;
/// `handed_ms`: release until it was cleaned and handed to the paste (or the command decided).
/// The paste itself (clipboard, a fixed 120 ms wait, Ctrl+V) runs after that, on the worker.
fn log_voice_timing(
    shared: &Shared,
    settings: &Settings,
    timing: crate::voice::Timing,
    dictate: bool,
    words: usize,
    text_ms: Option<u128>,
) {
    let since = |ms: Option<u128>| ms.map_or_else(|| "-".to_string(), |m| m.to_string());
    let line =
        format!(
        "{} stt={} audio_ms={} segments={} load_ms={} stt_ms={} text_ms={} handed_ms={} words={}",
        if dictate { "dictate" } else { "command" },
        if VoiceSettings::from(settings).use_groq { "groq" } else { "local" },
        timing.audio_ms,
        timing.segments,
        timing.load_ms,
        timing.stt_ms,
        since(text_ms),
        since(timing.released.map(|r| r.elapsed().as_millis())),
        words,
    );
    SecurityLog::at(shared.paths.voice_timing_log()).write(&line);
}

/// How often a memory line goes to `voice-timing.log`, so a tester's log shows any growth.
const MEMORY_LOG_EVERY: Duration = Duration::from_secs(10 * 60);

/// The memory line: private MB and open handles of OASIS and of the polish server (`-` when it is
/// not running or could not be read).
fn memory_line(own: Option<(u64, u32)>, polish: Option<(u64, u32)>) -> String {
    let part = |u: Option<(u64, u32)>| u.map_or_else(|| "private_mb=- handles=-".to_string(), |(bytes, handles)| format!("private_mb={} handles={handles}", bytes / (1024 * 1024)));
    format!("memory oasis {} | polish-server {}", part(own), part(polish))
}

fn report_voice_state(shared: &Shared, voice: Option<&Voice>, settings: &Settings) {
    let word = voice.map_or("unavailable", |v| v.state_word(settings));
    shared.update(|s| s.voice = word.to_string());
}

/// Whether `button` is part of the push-to-talk combination.
fn is_talk_button(settings: &Settings, button: i32) -> bool {
    VoiceSettings::from(settings).push_to_talk_buttons.contains(&button)
}

fn report_telemetry(shared: &Shared, link: &PadLink) {
    let t = link.telemetry();
    shared.update(|s| {
        s.battery_percent = t.battery_percent;
        s.charging = t.is_charging;
        s.latency_ms = t.link_latency_ms;
    });
    if let Some(percent) = t.battery_percent {
        shared.emit(EngineEvent::Battery { percent, charging: t.is_charging, full: t.is_full });
        let (warn, warned) = oasis_core::battery::low_battery_check(percent, t.is_charging, shared.low_battery_warned.load(Ordering::SeqCst));
        shared.low_battery_warned.store(warned, Ordering::SeqCst);
        if warn {
            shared.emit(EngineEvent::Reminder { title: "OASIS — Low battery".into(), body: format!("Device battery is at {percent}%. Charge soon.") });
        }
    }
}

#[cfg(test)]
mod memory_line_tests {
    use super::memory_line;

    #[test]
    fn the_memory_line_shows_megabytes_and_handles_or_a_dash() {
        assert_eq!(
            memory_line(Some((785 * 1024 * 1024 + 5, 826)), Some((1179 * 1024 * 1024, 400))),
            "memory oasis private_mb=785 handles=826 | polish-server private_mb=1179 handles=400"
        );
        assert_eq!(memory_line(Some((2 * 1024 * 1024, 9)), None), "memory oasis private_mb=2 handles=9 | polish-server private_mb=- handles=-");
    }
}
