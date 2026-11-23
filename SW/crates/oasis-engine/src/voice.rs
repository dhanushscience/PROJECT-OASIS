//! Push-to-talk dictation inside the engine. The rules (when a capture may begin, how long it may
//! run, what to show) are `oasis_core::voice`; this runs them: the microphone, the speech
//! engine on its own thread, pausing a playing YouTube video while you talk, and handing the
//! transcript back to be typed. Port of the dictation path of `Services/Voice/VoiceSessionService.cs`.
//!
//! With the local model, a recording is cut at pauses while the button is held and each piece is
//! decoded at once, so only the last piece is left when you let go (`docs/VOICE_DECISIONS.md`
//! V10). Groq gets the whole recording in one request.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant as StdInstant;

use oasis_app::VoiceSessionHost;
use oasis_core::voice::command::RunningApp;
use oasis_core::voice::{VoiceEffect, VoiceEvent, VoiceSessionConfig, VoiceState};
use oasis_settings::voice_commands::{capture_mode, is_dictate_first, CaptureMode};
use oasis_core::Instant;
use oasis_models::{AudioClip, SttEngine};
use oasis_platform::browser_bridge::BridgeServer;
use oasis_settings::voice::VoiceSettings;
use oasis_settings::Settings;

use crate::events::EngineEvent;

/// A microphone the engine can record from. `oasis_voice::Microphone` on Windows; a fake in tests.
pub trait VoiceMic: Send {
    fn start(&mut self) -> Result<(), String>;
    /// Stop and return what was heard (since the last `take`), as 16 kHz mono.
    fn stop(&mut self) -> AudioClip;
    /// What was heard so far, while recording goes on. A microphone that cannot do this returns
    /// nothing, and the whole recording comes from `stop`.
    fn take(&mut self) -> AudioClip {
        AudioClip { samples: Vec::new(), sample_rate: 16_000 }
    }
    /// How loud it is now, 0.0 to 1.0. A microphone without a meter says 0.
    fn level(&self) -> f32 {
        0.0
    }
}

/// How often the loudness is sent while recording, for the pill's bars to follow the voice.
const LEVEL_EVERY: std::time::Duration = std::time::Duration::from_millis(50);

/// How often the recording is looked at for a pause while the button is held.
const TAKE_EVERY: std::time::Duration = std::time::Duration::from_millis(250);

/// Makes a microphone for the current settings.
pub type MakeMic = Box<dyn Fn(&Settings) -> Box<dyn VoiceMic> + Send>;
/// Loads the speech engine for the current settings.
pub type LoadStt = Box<dyn Fn(&Settings) -> Result<Box<dyn SttEngine>, String> + Send>;

/// The real or pretend parts of voice, passed in like the pad and the foreground probe.
pub struct VoiceParts {
    /// A microphone for these settings (the device may be named in them).
    pub make_mic: MakeMic,
    /// Whether a speech model is usable for these settings. Cheap: only looks at files.
    pub model_ready: Box<dyn Fn(&Settings) -> bool + Send>,
    /// Load the speech engine. Slow; called on the speech thread, and only when needed.
    pub load_stt: LoadStt,
    /// Whether keyboard focus is in a text field of another program (the target for dictation).
    /// Asked once when a recording starts, on the engine's front thread (the loop does not wait).
    pub editable_focus: Box<dyn Fn() -> bool + Send>,
    /// The programs running now, for "open <a running app>" and "switch to …".
    pub running_apps: Box<dyn Fn() -> Vec<RunningApp> + Send>,
    /// Whether a name is a program really on this PC, for a spoken "open X" (`app_discovery::is_installed`).
    pub app_installed: Box<dyn Fn(&str) -> bool + Send>,
    /// The files and folders on the Desktop and in open Explorer windows, for "open project"
    /// (`app_discovery::on_screen_items`). Read only for an open-style phrase.
    pub on_screen: Arc<dyn Fn() -> Vec<oasis_core::voice::command::on_screen::ScreenItem> + Send + Sync>,
    /// Names of the installed apps already known (never starts a scan), for "did you mean …?".
    pub app_names: Box<dyn Fn() -> Vec<String> + Send>,
    /// Pauses whatever is playing on the PC (any player, like the keyboard's Pause key) and
    /// returns what it paused.
    pub pause_media: Box<dyn Fn() -> Vec<String> + Send>,
    /// Plays again what `pause_media` paused.
    pub resume_media: Box<dyn Fn(Vec<String>) + Send>,
}

/// Whether the recording about to start should be typed (true) or run as a command (false).
/// "Auto" types when the program in front is on the dictate-first list or a text field has
/// the focus. `process` is the program in front. A focus probe that times out reads as "no
/// field" (unknown); the gate in `engine.rs` then types any no-match phrase over 4 words (1.7).
pub fn wants_dictation(settings: &Settings, process: &str, editable_focus: &dyn Fn() -> bool) -> bool {
    match capture_mode(settings) {
        CaptureMode::Dictate => true,
        CaptureMode::Command => false,
        CaptureMode::Auto => is_dictate_first(settings, process) || editable_focus(),
    }
}

/// What was said, and whether the recording was meant to be typed.
#[derive(Debug, Clone, PartialEq)]
pub struct Heard {
    pub text: String,
    pub dictate: bool,
    pub timing: Timing,
    /// The first words after each long pause in dictation: hints for paragraph breaks, placed
    /// only after cleanup ([`oasis_core::speech::layout::place_paragraphs`]).
    pub paragraphs: Vec<String>,
    /// The whole recording, only while "Keep my dictation history" is on.
    pub audio: Option<AudioClip>,
}

/// Where one recording's time went, for the timing log. Durations in milliseconds; no text.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Timing {
    /// When the talk button was let go (the wait the user feels starts here).
    pub released: Option<StdInstant>,
    /// Length of the recording.
    pub audio_ms: u64,
    /// Loading the speech model, when this recording had to (0 when it was already loaded).
    pub load_ms: u64,
    /// Speech-to-text of the last piece (the part of the wait after release).
    pub stt_ms: u64,
    /// How many pieces the recording was decoded in (1 = whole).
    pub segments: u32,
}

struct SttJob {
    /// `None`: only load the engine for these settings, so the first recording does not wait for it.
    clip: Option<AudioClip>,
    settings: Settings,
    /// The last piece of a recording: its text completes what was heard.
    last: bool,
    /// With no clip: also decode a moment of silence, so a model left idle is fast again.
    wake: bool,
    /// Names the model should listen for in this recording (`docs/OPEN_ANYTHING.md` stage 2).
    hotwords: Arc<Vec<String>>,
}

/// What decides which speech engine is loaded. A change in any of these needs a fresh load.
fn engine_key(settings: &Settings) -> String {
    let v = VoiceSettings::from(settings);
    // The Dictionary is part of Groq's engine (its prompt); it does not touch the local one.
    let prompt = if v.use_groq { crate::groq::prompt_for(&v.dictation_glossary) } else { String::new() };
    format!("{}|{}|{}|{}|{}|{prompt}", v.model_folder, v.use_groq, v.groq_model, v.language, v.groq_fallback_to_local)
}

enum SttOutcome {
    Heard {
        text: String,
        load_ms: u64,
        stt_ms: u64,
        last: bool,
    },
    Failed { why: String, last: bool },
}

/// What the loop gives the voice session when it runs it.
pub struct VoiceContext<'a> {
    pub emit: &'a dyn Fn(EngineEvent),
    pub settings: &'a Settings,
    pub bridge: Option<&'a BridgeServer>,
    /// A YouTube video is playing in the browser: it is paused while talking, and only then.
    pub youtube_playing: bool,
    /// One line into `voice-timing.log` (numbers only, never what was said).
    pub log: &'a dyn Fn(&str),
}

pub struct Voice {
    parts: VoiceParts,
    host: VoiceSessionHost,
    mic: Option<Box<dyn VoiceMic>>,
    mic_device: String,
    mic_running: bool,
    clip: Option<AudioClip>,
    paused_media: bool,
    /// The media sessions paused for this recording, to play again afterwards.
    paused_sessions: Vec<String>,
    /// The recording now being made (or just made) was meant to be typed.
    dictate_this: bool,
    /// The timing of the recording being transcribed now.
    timing: Timing,
    jobs: Option<Sender<SttJob>>,
    results: Receiver<SttOutcome>,
    worker: Option<JoinHandle<()>>,
    /// The engine the speech thread was last asked to load ahead of time.
    warmed: String,
    /// When the speech model was last given work (a load, a piece or a wake).
    last_stt: StdInstant,
    /// Audio of this recording taken from the microphone but not yet sent as a piece.
    unsent: Vec<f32>,
    /// Samples of this recording already sent as pieces, and how many pieces.
    sent_samples: usize,
    sent_pieces: u32,
    last_take: StdInstant,
    last_level: StdInstant,
    /// For each piece sent and not yet decoded: whether it starts a paragraph (a long pause before it).
    paragraph_before: VecDeque<bool>,
    /// Text of the pieces decoded so far, and the first failure among them.
    texts: Vec<String>,
    /// The opening words of each decoded piece that followed a long pause.
    openings: Vec<String>,
    failed: Option<String>,
    /// The whole recording so far, kept only when the history is on (it is what Insights plays back).
    keep_audio: bool,
    recording: Vec<f32>,
    recording_rate: u32,
    /// The names of what is on screen, read off the engine thread when a command press starts.
    screen_names: Arc<Mutex<Vec<String>>>,
    /// This recording is a command (not dictation): places and on-screen names are listened for too.
    command_names: bool,
}

impl Voice {
    pub fn new(mut parts: VoiceParts) -> Voice {
        let (jobs, inbox) = mpsc::channel::<SttJob>();
        let (outcomes, results) = mpsc::channel::<SttOutcome>();
        let load_stt = std::mem::replace(&mut parts.load_stt, Box::new(|_| Err("unused".into())));
        let worker = thread::Builder::new()
            .name("oasis-speech".into())
            .spawn(move || speech_thread(&inbox, &outcomes, &load_stt))
            .ok();
        Voice {
            parts,
            host: VoiceSessionHost::new(VoiceSessionConfig::product_defaults(), true),
            mic: None,
            mic_device: String::new(),
            mic_running: false,
            clip: None,
            paused_media: false,
            paused_sessions: Vec::new(),
            dictate_this: true,
            timing: Timing::default(),
            jobs: Some(jobs),
            results,
            worker,
            warmed: String::new(),
            last_stt: StdInstant::now(),
            unsent: Vec::new(),
            sent_samples: 0,
            sent_pieces: 0,
            last_take: StdInstant::now(),
            last_level: StdInstant::now(),
            paragraph_before: VecDeque::new(),
            texts: Vec::new(),
            openings: Vec::new(),
            failed: None,
            keep_audio: false,
            recording: Vec::new(),
            recording_rate: 16_000,
            screen_names: Arc::default(),
            command_names: false,
        }
    }

    /// Load the speech engine now, in the background, when voice is on and these settings need a
    /// different one than was last loaded. Cheap to call often: it only compares the settings.
    pub fn warm(&mut self, settings: &Settings) {
        let key = engine_key(settings);
        if key == self.warmed || !self.ready(settings) {
            return;
        }
        if let Some(jobs) = &self.jobs {
            if jobs.send(SttJob { clip: None, settings: settings.clone(), last: false, wake: false, hotwords: Arc::default() }).is_ok() {
                self.warmed = key;
                self.last_stt = StdInstant::now();
            }
        }
    }

    /// Plugged in and idle: the local speech model decodes a moment of silence every few minutes,
    /// so Windows does not move it out of memory. One recording after two idle hours took 22 s to
    /// decode (the timing log, 2026-11-13). On battery the wake at button-down is the only one.
    fn keep_warm(&mut self, settings: &Settings) {
        if self.last_stt.elapsed() < KEEP_WARM_EVERY || !self.is_idle() || self.warmed.is_empty() {
            return;
        }
        self.last_stt = StdInstant::now();
        if VoiceSettings::from(settings).use_groq || oasis_win::power::status().on_battery {
            return;
        }
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(SttJob { clip: None, settings: settings.clone(), last: false, wake: true, hotwords: Arc::default() });
        }
    }

    /// The talk button went down. A local model left idle decodes its first piece up to a second
    /// slower (the timing log, 2026-11-13), so it is given a moment of silence to decode now, while
    /// the user is still talking. The same for the polish model.
    fn wake(&mut self, settings: &Settings) {
        if self.dictate_this {
            crate::polish::wake();
        }
        if self.last_stt.elapsed() < STT_IDLE || VoiceSettings::from(settings).use_groq {
            return;
        }
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(SttJob { clip: None, settings: settings.clone(), last: false, wake: true, hotwords: Arc::default() });
            self.last_stt = StdInstant::now();
        }
    }

    /// Push-to-talk is switched on and a speech model is there to use.
    pub fn ready(&self, settings: &Settings) -> bool {
        VoiceSettings::from(settings).enabled && (self.parts.model_ready)(settings)
    }

    /// For the status line: `off`, `no_model` or `ready`.
    pub fn state_word(&self, settings: &Settings) -> &'static str {
        if !VoiceSettings::from(settings).enabled {
            "off"
        } else if !(self.parts.model_ready)(settings) {
            "no_model"
        } else {
            "ready"
        }
    }

    pub fn is_idle(&self) -> bool {
        self.host.state() == VoiceState::Idle
    }

    /// The first button of the push-to-talk combination went down: start listening already, so the
    /// first words are not lost while the hold is still being recognised.
    pub fn preroll(&mut self, settings: &Settings) {
        if self.is_idle() && !self.mic_running {
            let _ = self.start_mic(settings);
        }
    }

    /// The hold did not become a capture (it was a tap): throw the pre-roll away.
    pub fn discard_preroll(&mut self) {
        if self.is_idle() && self.mic_running {
            self.stop_mic();
            self.clip = None;
        }
    }

    pub fn request(&mut self, event: VoiceEvent, now: Instant, ctx: &VoiceContext<'_>) {
        match event {
            VoiceEvent::BeginRequested { dictate, enabled, .. } if self.is_idle() => {
                self.dictate_this = dictate;
                self.command_names = !dictate;
                if enabled && !dictate {
                    // Read on a thread of its own (30-65 ms of Windows calls): the press never waits.
                    let (read, slot) = (Arc::clone(&self.parts.on_screen), Arc::clone(&self.screen_names));
                    thread::spawn(move || {
                        let names = read().iter().map(|i| i.stem().to_string()).collect();
                        *slot.lock().unwrap_or_else(|e| e.into_inner()) = names;
                    });
                }
                if enabled {
                    self.wake(ctx.settings);
                }
            }
            VoiceEvent::EndRequested { .. } if !self.is_idle() => {
                self.timing.released = Some(StdInstant::now())
            }
            _ => {}
        }
        self.run(VecDeque::from([event]), now, ctx, false);
    }

    /// The names the speech model listens for in this recording: the Dictionary always, and for a
    /// command the Windows places and what is on screen.
    fn hotwords(&self, settings: &Settings) -> Arc<Vec<String>> {
        // Dictation decodes greedily (faster); only a command listens for names.
        if !self.command_names {
            return Arc::default();
        }
        let dictionary: Vec<String> = VoiceSettings::from(settings).dictation_glossary.into_iter().map(|(_, written)| written).collect();
        // Each place once by its own name ("recycle bin", not all its spoken forms): on the owner's 60
        // clips a long list diluted every name and left fragments ("tra" from "trash").
        let (places, screen): (Vec<String>, Vec<String>) = if self.command_names {
            let mut seen = std::collections::HashSet::new();
            let places = oasis_core::actions::places::names().map(|(_, name, _)| name.to_lowercase()).filter(|n| seen.insert(n.clone())).collect();
            (places, self.screen_names.lock().unwrap_or_else(|e| e.into_inner()).clone())
        } else {
            (Vec::new(), Vec::new())
        };
        let places: Vec<&str> = places.iter().map(String::as_str).collect();
        Arc::new(oasis_core::speech::hotwords::phrases(&dictionary, &places, &screen))
    }

    /// The programs running now.
    pub fn running_apps(&self) -> Vec<RunningApp> {
        (self.parts.running_apps)()
    }

    /// The files and folders the user can see.
    pub fn on_screen(&self) -> Vec<oasis_core::voice::command::on_screen::ScreenItem> {
        (self.parts.on_screen)()
    }

    /// Installed app names already known, for "did you mean …?".
    pub fn app_names(&self) -> Vec<String> {
        (self.parts.app_names)()
    }

    /// Whether `name` is a program installed on this PC.
    pub fn app_installed(&self, name: &str) -> bool {
        (self.parts.app_installed)(name)
    }

    /// Timers and finished transcriptions. Returns what was heard.
    pub fn tick(&mut self, now: Instant, ctx: &VoiceContext<'_>) -> Vec<Heard> {
        self.keep_warm(ctx.settings);
        self.send_finished_pieces(ctx.settings);
        if !self.is_idle() && self.mic_running && self.last_level.elapsed() >= LEVEL_EVERY {
            self.last_level = StdInstant::now();
            if let Some(mic) = self.mic.as_ref() {
                (ctx.emit)(EngineEvent::VoiceLevel { level: (mic.level().clamp(0.0, 1.0) * 100.0).round() as u8 });
            }
        }
        let mut typed = Vec::new();
        let mut events = VecDeque::new();
        while let Ok(outcome) = self.results.try_recv() {
            let paragraph = self.paragraph_before.pop_front().unwrap_or(false);
            let last = match outcome {
                SttOutcome::Heard { text, load_ms, stt_ms, last } => {
                    let text = text.trim();
                    if !text.is_empty() {
                        // Dictation only: a command is one line whatever the pauses.
                        if paragraph && self.dictate_this && !self.texts.is_empty() {
                            self.openings.push(text.split_whitespace().take(3).collect::<Vec<_>>().join(" "));
                        }
                        self.texts.push(text.to_string());
                        // Sentences already finished are polished while the user keeps talking (V17).
                        if !last && self.dictate_this {
                            // Joined as the final text will be, so the cached sentences match it after release.
                            let so_far = join_dictation_pieces(&self.texts, &self.openings, &dictionary_names(ctx.settings));
                            crate::polish::prewarm(&so_far, ctx.settings, &self.openings);
                        }
                    }
                    if last {
                        self.timing = Timing { load_ms, stt_ms, ..self.timing };
                    }
                    last
                }
                SttOutcome::Failed { why, last } => {
                    self.failed.get_or_insert(why);
                    last
                }
            };
            if !last {
                continue;
            }
            let pieces = std::mem::take(&mut self.texts);
            // A command cut at its pauses is still one phrase: "Search for. Best laptops." would read
            // as two sentences, and in a text field only a one-sentence command may run.
            let paragraphs = std::mem::take(&mut self.openings);
            let text = if self.dictate_this {
                join_dictation_pieces(&pieces, &paragraphs, &dictionary_names(ctx.settings))
            } else {
                join_command_pieces(&pieces)
            };
            // A piece that failed would leave a hole in the middle: report it and type nothing.
            if let Some(why) = self.failed.take() {
                (ctx.emit)(EngineEvent::Note { text: format!("Voice: {why}") });
            } else if !text.is_empty() {
                let audio = (!self.recording.is_empty()).then(|| AudioClip { samples: std::mem::take(&mut self.recording), sample_rate: self.recording_rate });
                typed.push(Heard { text, dictate: self.dictate_this, timing: self.timing, paragraphs, audio });
            }
            events.push_back(VoiceEvent::ProcessingFinished);
        }
        self.run(events, now, ctx, true);
        typed
    }

    /// While the button is held, send each piece of the recording that ended at a pause to be
    /// decoded now. Local model only: Groq is one request per recording.
    fn send_finished_pieces(&mut self, settings: &Settings) {
        if self.is_idle() || !self.mic_running || self.last_take.elapsed() < TAKE_EVERY || VoiceSettings::from(settings).use_groq {
            return;
        }
        self.last_take = StdInstant::now();
        let (Some(mic), Some(jobs)) = (self.mic.as_mut(), &self.jobs) else { return };
        let taken = mic.take();
        self.unsent.extend_from_slice(&taken.samples);
        if self.keep_audio {
            self.recording.extend_from_slice(&taken.samples);
            self.recording_rate = taken.sample_rate;
        }
        let cut_for = if self.dictate_this { oasis_core::speech::audio::segment_cut } else { oasis_core::speech::audio::command_cut };
        while let Some(cut) = cut_for(&self.unsent, taken.sample_rate) {
            let rest = self.unsent.split_off(cut.min(self.unsent.len()));
            let piece = AudioClip { samples: std::mem::replace(&mut self.unsent, rest), sample_rate: taken.sample_rate };
            self.sent_samples += piece.samples.len();
            // A piece with no speech is not decoded: beam search answers silence with "Yeah." or
            // "Mm-hmm." (sherpa-onnx 2.0.2, length reward 0.5), as the last piece already is not.
            if !oasis_core::speech::audio::has_speech(&piece.samples, piece.sample_rate) {
                continue;
            }
            self.paragraph_before.push_back(self.sent_pieces > 0 && oasis_core::speech::audio::starts_paragraph(&piece.samples, piece.sample_rate));
            self.sent_pieces += 1;
            let _ = jobs.send(SttJob { clip: Some(piece), hotwords: self.hotwords(settings), settings: settings.clone(), last: false, wake: false });
            self.last_stt = StdInstant::now();
        }
    }

    fn run(&mut self, mut queue: VecDeque<VoiceEvent>, now: Instant, ctx: &VoiceContext<'_>, due: bool) {
        let mut effects: Vec<VoiceEffect> = Vec::new();
        if due {
            self.host.fire_due(now, &mut |e| effects.push(*e));
        }
        loop {
            for effect in std::mem::take(&mut effects) {
                self.effect(effect, now, ctx, &mut queue);
            }
            let Some(event) = queue.pop_front() else { break };
            self.host.apply(event, now, &mut |e| effects.push(*e));
        }
    }

    fn effect(&mut self, effect: VoiceEffect, _now: Instant, ctx: &VoiceContext<'_>, queue: &mut VecDeque<VoiceEvent>) {
        match effect {
            VoiceEffect::StartMic => {
                if let Err(why) = self.start_mic(ctx.settings) {
                    (ctx.emit)(EngineEvent::VoiceDropped { reason: "no_microphone".into() });
                    (ctx.emit)(EngineEvent::Note { text: why });
                    queue.push_back(VoiceEvent::CancelRequested);
                }
            }
            VoiceEffect::StopMic => {
                if self.mic_running {
                    self.stop_mic();
                }
            }
            VoiceEffect::DuckMedia => {
                // Only what is really playing is paused, so it is not resumed when you had paused
                // it yourself (the bug the C# ducker was rewritten for). Windows' own media
                // sessions cover Spotify, Apple Music and browsers; the extension is the fallback
                // for a YouTube tab that does not report one.
                self.paused_sessions = (self.parts.pause_media)();
                if self.paused_sessions.is_empty() {
                    if let (true, Some(bridge)) = (ctx.youtube_playing, ctx.bridge) {
                        bridge.send_command("playback.pause");
                        self.paused_media = true;
                    }
                }
            }
            VoiceEffect::RestoreMedia => {
                let sessions = std::mem::take(&mut self.paused_sessions);
                if !sessions.is_empty() {
                    (self.parts.resume_media)(sessions);
                }
                if std::mem::take(&mut self.paused_media) {
                    if let Some(bridge) = ctx.bridge {
                        bridge.send_command("playback.resume");
                    }
                }
            }
            VoiceEffect::ShowListening => {
                (ctx.emit)(EngineEvent::Voice { phase: "listening".into() });
            }
            VoiceEffect::ShowDictating => (ctx.emit)(EngineEvent::Voice { phase: "dictating".into() }),
            VoiceEffect::ShowThinking | VoiceEffect::ShowUnderstanding => {
                (ctx.emit)(EngineEvent::Voice { phase: "thinking".into() });
            }
            VoiceEffect::HideOverlay | VoiceEffect::ScheduleHideOverlay(_) => {
                (ctx.emit)(EngineEvent::Voice { phase: "idle".into() });
            }
            VoiceEffect::BeginProcessing { .. } => {
                if self.mic_running {
                    self.stop_mic();
                }
                let tail = self.clip.take().unwrap_or(AudioClip { samples: Vec::new(), sample_rate: 16_000 });
                let mut samples = std::mem::take(&mut self.unsent);
                samples.extend_from_slice(&tail.samples);
                let rate = tail.sample_rate.max(1);
                if self.keep_audio {
                    self.recording.extend_from_slice(&tail.samples);
                    self.recording_rate = rate;
                }
                let pieces = std::mem::take(&mut self.sent_pieces);
                let voice = VoiceSettings::from(ctx.settings);
                let heard_ms = (std::mem::take(&mut self.sent_samples) + samples.len()) as u64 * 1000 / u64::from(rate);
                self.timing.audio_ms = heard_ms;
                self.timing.segments = pieces + 1;
                // After earlier pieces, a last piece with no speech in it is not decoded.
                if pieces > 0 && !oasis_core::speech::audio::has_speech(&samples, rate) {
                    samples.clear();
                    self.timing.segments = pieces;
                }
                let paragraph = pieces > 0 && oasis_core::speech::audio::starts_paragraph(&samples, rate);
                let clip = AudioClip { samples, sample_rate: rate };
                // Too short, or only quiet: the model invents words ("Thank you.") from silence.
                if pieces == 0 && (heard_ms < voice.min_utterance_ms || !oasis_core::speech::audio::has_speech(&clip.samples, rate)) {
                    let reason = if heard_ms < voice.min_utterance_ms { "too_short" } else { "no_speech" };
                    (ctx.log)(&format!("dropped {reason} {}", oasis_core::speech::audio::speech_stats(&clip.samples, rate)));
                    (ctx.emit)(EngineEvent::VoiceDropped { reason: reason.into() });
                    (ctx.emit)(EngineEvent::Voice { phase: "idle".into() });
                    queue.push_back(VoiceEvent::ProcessingFinished);
                } else if let Some(jobs) = &self.jobs {
                    self.paragraph_before.push_back(paragraph);
                    self.last_stt = StdInstant::now();
                    if jobs.send(SttJob { clip: Some(clip), hotwords: self.hotwords(ctx.settings), settings: ctx.settings.clone(), last: true, wake: false }).is_err() {
                        self.texts.clear();
                        self.openings.clear();
                        self.paragraph_before.clear();
                        self.failed = None;
                        queue.push_back(VoiceEvent::ProcessingFinished);
                    }
                }
            }
            VoiceEffect::ClearStuckBusy | VoiceEffect::SetTimer { .. } | VoiceEffect::CancelTimer(_) => {}
        }
    }

    fn start_mic(&mut self, settings: &Settings) -> Result<(), String> {
        let device = VoiceSettings::from(settings).microphone;
        if self.mic.is_none() || self.mic_device != device {
            self.mic = Some((self.parts.make_mic)(settings));
            self.mic_device = device;
        }
        if self.mic_running {
            return Ok(());
        }
        let mic = self.mic.as_mut().expect("just created");
        mic.start()?;
        self.mic_running = true;
        self.unsent.clear();
        self.recording.clear();
        self.keep_audio = VoiceSettings::from(settings).keep_history;
        self.sent_samples = 0;
        self.sent_pieces = 0;
        self.last_take = StdInstant::now();
        Ok(())
    }

    fn stop_mic(&mut self) {
        if let Some(mic) = self.mic.as_mut() {
            self.clip = Some(mic.stop());
        }
        self.mic_running = false;
    }

    /// Stop everything: the recording, and the speech thread.
    pub fn shutdown(&mut self) {
        if self.mic_running {
            self.stop_mic();
        }
        self.jobs = None; // ends the speech thread
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for Voice {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Cut leading/trailing quiet before STT (spec 1.3). Off: on the 4 real DictationHistory clips
/// (2026-11-17, `trim_ab` example) the frame detector missed quiet speech and the trim dropped
/// real words on all 4 ("and continue with the work"); synthetic clips were unchanged.
const TRIM_BEFORE_STT: bool = false;

/// Loads the speech model on the first recording (it takes seconds) and keeps it.
fn speech_thread(inbox: &Receiver<SttJob>, outcomes: &Sender<SttOutcome>, load: &LoadStt) {
    let mut engine: Option<(String, Box<dyn SttEngine>)> = None;
    let mut last_hotwords: Vec<String> = Vec::new();
    for job in inbox {
        let wanted = engine_key(&job.settings);
        let started = StdInstant::now();
        if engine.as_ref().is_none_or(|(have, _)| *have != wanted) {
            engine = None; // free the old model before loading the next one
            last_hotwords.clear(); // a new engine has none
            match load(&job.settings) {
                Ok(loaded) => engine = Some((wanted, loaded)),
                // A failed preload is reported by the first recording that needs the engine.
                Err(why) if job.clip.is_some() => {
                    let _ = outcomes.send(SttOutcome::Failed { why, last: job.last });
                    continue;
                }
                Err(_) => continue,
            }
        }
        let (Some((_, stt)), Some(job_clip)) = (engine.as_mut(), job.clip) else {
            if let (true, Some((_, stt))) = (job.wake, engine.as_mut()) {
                let _ = stt.transcribe(&AudioClip { samples: vec![0.0; 8_000], sample_rate: 16_000 });
            }
            continue;
        };
        let load_ms = ms_since(started);
        if job_clip.samples.is_empty() {
            let _ = outcomes.send(SttOutcome::Heard { text: String::new(), load_ms, stt_ms: 0, last: job.last });
            continue;
        }
        // The recording goes to the speech model as it was heard (V44: audio cleaning cost accuracy).
        let job_clip = if TRIM_BEFORE_STT {
            AudioClip { samples: oasis_core::speech::audio::trim_to_speech(&job_clip.samples, job_clip.sample_rate).to_vec(), ..job_clip }
        } else {
            job_clip
        };
        let mut job_clip = job_clip;
        oasis_core::speech::audio::normalize(&mut job_clip.samples); // a quiet built-in mic decodes worse
        if *job.hotwords != last_hotwords {
            stt.set_hotwords(&job.hotwords);
            last_hotwords = job.hotwords.to_vec();
        }
        let decoding = StdInstant::now();
        let outcome = match stt.transcribe(&job_clip) {
            Ok(text) => SttOutcome::Heard {
                text,
                load_ms,
                stt_ms: ms_since(decoding),
                last: job.last,
            },
            Err(e) => SttOutcome::Failed { why: e.to_string(), last: job.last },
        };
        let _ = outcomes.send(outcome);
    }
}

/// A speech model not used for this long is woken when the talk button goes down.
const STT_IDLE: std::time::Duration = std::time::Duration::from_secs(20);
/// How often an idle speech model is kept in memory while the PC is plugged in.
const KEEP_WARM_EVERY: std::time::Duration = std::time::Duration::from_secs(240);

/// The capitalised words of the user's Dictionary, lower case: names, for [`join_dictation_pieces`].
fn dictionary_names(settings: &Settings) -> Vec<String> {
    VoiceSettings::from(settings)
        .dictation_glossary
        .iter()
        .flat_map(|(_, written)| written.split_whitespace())
        .filter(|w| w.starts_with(char::is_uppercase))
        .map(|w| w.to_lowercase())
        .collect()
}

/// A token without the punctuation around it.
fn bare(token: &str) -> &str {
    token.trim_matches(|c: char| !c.is_alphanumeric())
}

/// Words that keep their capital in the middle of a sentence.
const CAPITAL_WORDS: [&str; 19] = [
    "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday", "january", "february", "march",
    "april", "june", "july", "august", "september", "october", "november", "december", "oasis",
];

/// The pieces of one dictation as one text. The speech model writes every piece as if it stood
/// alone, so a pause in the middle of a sentence left "…it is Pasting even…" and "…providing me. a
/// good output": after a closing mark the next piece starts with a capital; with none, its capital
/// goes. A piece after a long pause (`paragraphs`: its opening words) starts a new sentence.
/// A name keeps its capital: a word of the Dictionary (`names`, lower case), or one the speech
/// model wrote with a capital in the middle of a sentence anywhere in this dictation.
// shortcut: a name said only once, right after a pause in mid-sentence, and not in the Dictionary
// still loses its capital; add it to the Dictionary.
fn join_dictation_pieces(pieces: &[String], paragraphs: &[String], names: &[String]) -> String {
    let mut names: Vec<String> = names.to_vec();
    for piece in pieces {
        let tokens: Vec<&str> = piece.split_whitespace().collect();
        for pair in tokens.windows(2) {
            if !pair[0].ends_with(['.', '!', '?']) && bare(pair[1]).starts_with(char::is_uppercase) {
                names.push(bare(pair[1]).to_lowercase());
            }
        }
    }
    let mut out = String::new();
    for piece in pieces.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
        if out.is_empty() {
            out.push_str(piece);
            continue;
        }
        let opens_paragraph = paragraphs.iter().any(|o| !o.is_empty() && piece.starts_with(o.as_str()));
        if !out.ends_with(['.', '!', '?']) && opens_paragraph {
            out.truncate(out.trim_end_matches(',').len());
            out.push('.');
        }
        let word: String = piece.chars().take_while(|c| c.is_alphanumeric() || *c == '\'').collect();
        // A full stop at a pause cut that English cannot have is the model's false stop: drop it so the
        // piece below is lowercased like any mid-sentence piece. Never after a word no sentence ends on
        // ("the.", "where."); after one few end on, before a function word ("to. Use"); before a word
        // that only continues ("so that", "than", "with"). Replayed on recordings: `replay_pauses` example.
        // shortcut: fixed word lists, no grammar; a rare real "…to. Use…" or "…. With that, …" loses its stop.
        const NEVER_LAST: [&str; 13] = ["the", "an", "and", "or", "of", "my", "your", "our", "their", "because", "than", "which", "where"];
        const RARELY_LAST: [&str; 13] = ["to", "a", "but", "so", "that", "with", "for", "in", "on", "at", "by", "if", "into"];
        const OPENERS: [&str; 12] = ["and", "so", "use", "when", "the", "but", "or", "then", "if", "because", "it", "this"];
        let next = word.to_lowercase();
        let last = out.split_whitespace().last().map(|w| bare(w).to_lowercase()).unwrap_or_default();
        let continues = ["than", "with", "into"].contains(&next.as_str())
            || (next == "so" && piece.split_whitespace().nth(1).is_some_and(|w| bare(w).eq_ignore_ascii_case("that")));
        if out.ends_with('.') && !out.ends_with("..") && !opens_paragraph
            && (NEVER_LAST.contains(&last.as_str()) || (RARELY_LAST.contains(&last.as_str()) && OPENERS.contains(&next.as_str())) || continues)
        {
            out.pop();
        }
        let keeps = word == "I" || word.starts_with("I'") || word.chars().skip(1).any(|c| c.is_uppercase() || c.is_ascii_digit()) || CAPITAL_WORDS.contains(&word.to_lowercase().as_str()) || names.contains(&word.to_lowercase());
        let mut chars = piece.chars();
        let first = chars.next().unwrap_or(' ');
        let ended = out.ends_with(['.', '!', '?']);
        out.push(' ');
        if ended {
            out.extend(first.to_uppercase());
        } else if keeps {
            out.push(first);
        } else {
            out.extend(first.to_lowercase());
        }
        out.push_str(chars.as_str());
    }
    out
}

/// The pieces of one command, without the full stops the speech model put where the user paused.
fn join_command_pieces(pieces: &[String]) -> String {
    let last = pieces.len().saturating_sub(1);
    pieces
        .iter()
        .enumerate()
        .map(|(i, p)| if i < last { p.trim_end_matches(['.', ',', '!', '?', ' ']) } else { p.as_str() })
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn ms_since(start: StdInstant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(voice: serde_json::Value) -> Settings {
        serde_json::from_value(serde_json::json!({ "Profiles": [], "Voice": voice })).expect("settings")
    }

    #[test]
    fn dictation_pieces_are_joined_with_the_casing_of_one_text() {
        let join_named = |pieces: &[&str], paragraphs: &[&str], names: &[&str]| {
            let owned = |list: &[&str]| list.iter().map(|p| p.to_string()).collect::<Vec<_>>();
            join_dictation_pieces(&owned(pieces), &owned(paragraphs), &owned(names))
        };
        let join = |pieces: &[&str], paragraphs: &[&str]| join_named(pieces, paragraphs, &[]);
        assert_eq!(join_named(&["then I asked", "Shashank about the board"], &[], &["shashank"]), "then I asked Shashank about the board", "a Dictionary name");
        assert_eq!(join(&["I met John today and", "John said", "Nothing is ready"], &[]), "I met John today and John said nothing is ready", "a name seen in mid-sentence");
        let words = settings(serde_json::json!({ "Enabled": true, "DictationGlossary": [{ "Spoken": "kay cad", "Canonical": "KiCad" }, { "Spoken": "dc", "Canonical": "pc" }, { "Spoken": "Sheshank", "Canonical": "Shashank" }] }));
        assert_eq!(dictionary_names(&words), ["kicad", "shashank"]);
        assert_eq!(join(&["S1 mini is not providing me.", "a good output."], &[]), "S1 mini is not providing me. A good output.");
        assert_eq!(join(&["like it is", "Pasting even if there is no window"], &[]), "like it is pasting even if there is no window");
        assert_eq!(join(&["when user ask to save the layout,", "It should also show"], &[]), "when user ask to save the layout, it should also show");
        assert_eq!(join(&["and then", "I'll call him on", "Monday with the", "KiCad files"], &[]), "and then I'll call him on Monday with the KiCad files");
        assert_eq!(join(&["It is showing now", "But when it was saved it was not."], &["But when it"]), "It is showing now. But when it was saved it was not.");
        assert_eq!(join(&["I know how to.", "Use this tool"], &[]), "I know how to use this tool", "false stop after a dangling word");
        assert_eq!(join(&["we need the.", "And then the board"], &[]), "we need the and then the board");
        assert_eq!(join(&["It works.", "Use this tool"], &[]), "It works. Use this tool", "a real stop stays");
        // False stops seen in the replay of recorded dictation at 850 ms / 5 s and 400 ms / 3 s.
        assert_eq!(join(&["a voice section where.", "If the user is"], &[]), "a voice section where if the user is");
        assert_eq!(join(&["put in casual mode and.", "In that, user can"], &[]), "put in casual mode and in that, user can");
        assert_eq!(join(&["deploy Sonnet agents.", "So that it completes the work"], &[]), "deploy Sonnet agents so that it completes the work");
        assert_eq!(join(&["for closing pop-up.", "With the Mr. Science"], &[]), "for closing pop-up with the Mr. Science");
        for real in [["I don't want to.", "Maybe later."], ["We have two options.", "Which one is better?"], ["It is done.", "So we ship."], ["The board works.", "The battery is new."]] {
            assert_eq!(join(&real, &[]), real.join(" "), "a real sentence end stays");
        }
        assert_eq!(join(&["we need the.", "With the board"], &["With the"]), "we need the. With the board", "a long pause is a real break");
        assert_eq!(join(&["Is it done?", "Yes."], &[]), "Is it done? Yes.");
        assert_eq!(join(&["One piece only."], &[]), "One piece only.");
    }

    #[test]
    fn a_command_said_with_pauses_is_joined_as_one_phrase() {
        let pieces = ["Search for.".to_string(), "Ha.".to_string(), "Best laptops.".to_string()];
        assert_eq!(join_command_pieces(&pieces), "Search for Ha Best laptops.");
        assert_eq!(join_command_pieces(&["Open Spotify.".to_string()]), "Open Spotify.");
    }

    #[test]
    fn any_change_to_which_engine_runs_needs_a_fresh_load() {
        let local = settings(serde_json::json!({ "Enabled": true }));
        assert_eq!(engine_key(&local), engine_key(&settings(serde_json::json!({ "Enabled": true, "MinUtteranceMs": 500 }))), "unrelated settings keep the loaded engine");
        for other in [
            serde_json::json!({ "Enabled": true, "SttProvider": 1 }),
            serde_json::json!({ "Enabled": true, "ModelFileName": "other-model" }),
            serde_json::json!({ "Enabled": true, "SttProvider": 1, "GroqFallbackToLocal": false }),
            serde_json::json!({ "Enabled": true, "Language": "de" }),
        ] {
            assert_ne!(engine_key(&local), engine_key(&settings(other.clone())), "{other}");
        }
        // A Dictionary change is Groq's prompt; the local model is not reloaded for it.
        let words = serde_json::json!([{ "Spoken": "kay cad", "Canonical": "KiCad" }]);
        let local_words = settings(serde_json::json!({ "Enabled": true, "DictationGlossary": words }));
        assert_eq!(engine_key(&local), engine_key(&local_words));
        let groq = settings(serde_json::json!({ "Enabled": true, "SttProvider": 1 }));
        let groq_words = settings(serde_json::json!({ "Enabled": true, "SttProvider": 1, "DictationGlossary": words }));
        assert_ne!(engine_key(&groq), engine_key(&groq_words));
    }
}
