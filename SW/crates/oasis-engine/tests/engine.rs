//! The whole chain with fake parts: pad bytes in → gesture rules → matched profile → action out.
//! Nothing here touches Bluetooth, the desktop or the owner's real files.

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use oasis_core::actions::{ActionBackend, MouseClick};
use oasis_core::shortcut::Chord;
use oasis_engine::{
    BridgeConfig,
    start, BackendContext, EngineConfig, EngineError, EngineEvent, EngineHandle, EngineParts,
    ForegroundProbe,
};
use oasis_platform::paths::OasisPaths;
use oasis_platform::ports::{ButtonSource, ForegroundInfo, PadInfo, PadTransport, PlatformResult, Wake};
use oasis_platform::profile_store::SettingsIntegrity;

const PROFILES: &str = r#"{
  "SchemaVersion": 2,
  "ShowOverlayLabels": false,
  "Profiles": [
    { "Name": "Default", "ContextMatch": "*", "Kind": "", "MultiConfigEnabled": false,
      "ConfigProfiles": [ { "Name": "Default", "ContextMatch": "*", "Buttons": [
        { "Type": "Shortcut", "Value": "Ctrl+1", "Label": "A", "Steps": "" },
        { "Type": "Shortcut", "Value": "Ctrl+2", "Label": "B", "Steps": "" },
        { "Type": "Shortcut", "Value": "Ctrl+3", "Label": "C", "Steps": "" } ] } ] },
    { "Name": "browser", "ContextMatch": "chrome", "Kind": "app", "MultiConfigEnabled": false,
      "ConfigProfiles": [ { "Name": "browser", "ContextMatch": "*", "Buttons": [
        { "Type": "Shortcut", "Value": "Alt+1", "Label": "A", "Steps": "" },
        { "Type": "OpenUrl", "Value": "example.com", "Label": "B", "Steps": "" },
        { "Type": "Shortcut", "Value": "", "Label": "", "Steps": "" } ] } ] }
  ]
}"#;

// ---- fakes ---------------------------------------------------------------------------------

#[derive(Default)]
struct PadState {
    connected: bool,
    inbox: VecDeque<Vec<u8>>,
}

#[derive(Clone, Default)]
struct FakePad(Arc<Mutex<PadState>>);

impl FakePad {
    fn send(&self, wire: &str) {
        self.0.lock().unwrap().inbox.push_back(wire.as_bytes().to_vec());
    }
    /// A quick tap: down and up straight away.
    fn tap(&self, button: u8) {
        self.send(&format!("BTN:{button}:D\n"));
        self.send(&format!("BTN:{button}:U\n"));
    }
}

impl PadTransport for FakePad {
    fn scan(&mut self, _: Duration) -> PlatformResult<Vec<PadInfo>> {
        Ok(vec![PadInfo { id: "100".into(), name: "OASIS_INTERFACE".into(), rssi: Some(-50) }])
    }
    fn connect(&mut self, _: &PadInfo) -> PlatformResult<()> {
        self.0.lock().unwrap().connected = true;
        Ok(())
    }
    fn disconnect(&mut self) {
        self.0.lock().unwrap().connected = false;
    }
    fn is_connected(&self) -> bool {
        self.0.lock().unwrap().connected
    }
    fn write(&mut self, _: &str) -> PlatformResult<()> {
        Ok(())
    }
    fn next_notification(&mut self, timeout: Duration) -> PlatformResult<Option<Vec<u8>>> {
        if let Some(bytes) = self.0.lock().unwrap().inbox.pop_front() {
            return Ok(Some(bytes));
        }
        thread::sleep(timeout.min(Duration::from_millis(4)));
        Ok(None)
    }
}

#[derive(Clone)]
/// (process, title, address, panel)
struct FakeForeground(Arc<Mutex<(String, String, String, String)>>);

impl ForegroundProbe for FakeForeground {
    fn set_url_profiles_exist(&mut self, _: bool) {}
    fn current(&mut self) -> PlatformResult<ForegroundInfo> {
        let (process, title, url, panel) = self.0.lock().unwrap().clone();
        // `<oasis>` stands for the OASIS window itself: the engine knows it by its own pid.
        let pid = (process == "<oasis>").then(std::process::id);
        Ok(ForegroundInfo {
            process,
            window_title: title,
            browser_url: (!url.is_empty()).then_some(url),
            pid,
            detection_confidence: if panel.is_empty() { String::new() } else { "high".into() },
            panel_id: panel,
            ..ForegroundInfo::default()
        })
    }
}

/// Records what the engine asks for.
struct Recorder(Arc<Mutex<Vec<String>>>);

impl ActionBackend for Recorder {
    fn send_shortcut(&mut self, chord: &Chord) -> bool {
        self.0.lock().unwrap().push(format!("shortcut {:?}", chord.keys));
        true
    }
    fn open_url(&mut self, url: &str) -> bool {
        self.0.lock().unwrap().push(format!("url {url}"));
        true
    }
    fn launch_browser_url(&mut self, _: &str, _: &str) -> bool {
        true
    }
    fn launch_app(&mut self, query: &str) -> bool {
        self.0.lock().unwrap().push(format!("app {query}"));
        true
    }
    fn close_app(&mut self, process: &str) -> bool {
        self.0.lock().unwrap().push(format!("close {process}"));
        true
    }
    fn window_action(&mut self, kind: &str, value: &str) -> bool {
        self.0.lock().unwrap().push(format!("window {kind} {value}"));
        true
    }
    fn run_script(&mut self, _: &str) {}
    fn mouse_click(&mut self, _: MouseClick) -> bool {
        true
    }
    fn paste_text(&mut self, text: &str) -> bool {
        self.0.lock().unwrap().push(format!("paste {text}"));
        true
    }
    fn join_meeting(&mut self) -> bool {
        false
    }
    fn system(&mut self, command: &str) -> bool {
        self.0.lock().unwrap().push(format!("system {command}"));
        true
    }
    fn pause(&mut self, _: Duration) {}
    fn run_script_allowed(&self) -> bool {
        false
    }
    fn security_log(&mut self, _: &str) {}
}

/// Writes what it is asked to do into the same log as the actions.
struct FakeSpotify(Arc<Mutex<Vec<String>>>);

impl oasis_engine::spotify::Runner for FakeSpotify {
    fn run(&self, kind: &str, value: &str) -> Result<String, String> {
        self.0.lock().unwrap().push(format!("spotify {kind} {value}"));
        if value == "boom" { Err("pretend failure".into()) } else { Ok(format!("done {value}")) }
    }
}

/// A calendar with one meeting that starts in two minutes and has a link.
struct FakeCalendar;

impl oasis_engine::meetings::Calendar for FakeCalendar {
    fn connected(&self) -> bool {
        true
    }
    fn status(&self) -> String {
        "fake".into()
    }
    fn upcoming(&self, _: i64, _: i64) -> Result<Vec<oasis_core::meetings::Meeting>, String> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        Ok(vec![oasis_core::meetings::Meeting {
            id: "m1".into(),
            title: "Standup".into(),
            start: now + 120,
            end: now + 120 + 1800,
            join_url: "https://meet.google.com/abc".into(),
            all_day: false,
            provider: "google".into(),
        }])
    }
    fn connect(&self, _: &dyn Fn(&str) -> bool) -> Result<(), String> {
        Ok(())
    }
    fn disconnect(&self) -> Result<(), String> {
        Ok(())
    }
}

/// What the engine last told the keyboard: listen, the three shortcuts, exclusive.
type Asked = (bool, [String; 3], bool);

/// The keyboard's three buttons, by hand: a queue of edges to hand over, and what the engine last asked it to do.
#[derive(Clone, Default)]
struct FakeButtons {
    queue: Arc<Mutex<Vec<(u8, bool)>>>,
    asked: Arc<Mutex<Option<Asked>>>,
    wake: Arc<Mutex<Option<Wake>>>,
}

impl FakeButtons {
    /// Edges as the keyboard would hand them over, waking the engine as a real source does.
    fn press(&self, edges: impl IntoIterator<Item = (u8, bool)>) {
        self.queue.lock().unwrap().extend(edges);
        if let Some(wake) = self.wake.lock().unwrap().clone() {
            wake();
        }
    }
}

impl ButtonSource for FakeButtons {
    fn set_wake(&mut self, wake: Wake) {
        *self.wake.lock().unwrap() = Some(wake);
    }
    fn configure(&mut self, listen: bool, chords: [String; 3], exclusive: bool) {
        *self.asked.lock().unwrap() = Some((listen, chords, exclusive));
    }
    fn drain(&mut self) -> Vec<(u8, bool)> {
        std::mem::take(&mut *self.queue.lock().unwrap())
    }
}

struct Signature(bool);

impl SettingsIntegrity for Signature {
    fn verify(&self, _: &str) -> bool {
        self.0
    }
    fn sign(&self, _: &str) -> std::io::Result<()> {
        Ok(())
    }
}

// ---- harness -------------------------------------------------------------------------------

struct Harness {
    engine: EngineHandle,
    paths: OasisPaths,
    pad: FakePad,
    keys: FakeButtons,
    foreground: FakeForeground,
    actions: Arc<Mutex<Vec<String>>>,
    events: Arc<Mutex<Vec<EngineEvent>>>,
}

fn temp_paths(name: &str, profiles: Option<&str>) -> OasisPaths {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("oasis-engine-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    if let Some(text) = profiles {
        fs::write(dir.join("profiles.json"), text).unwrap();
    }
    OasisPaths::with_roots(&dir, &dir)
}

impl Harness {
    fn start(name: &str, live: bool, signature_ok: bool) -> Harness {
        Harness::start_with(name, PROFILES, live, signature_ok)
    }

    fn start_with(name: &str, profiles: &str, live: bool, signature_ok: bool) -> Harness {
        Harness::start_full(name, profiles, live, signature_ok, None)
    }

    fn start_full(
        name: &str,
        profiles: &str,
        live: bool,
        signature_ok: bool,
        voice: Option<oasis_engine::VoiceParts>,
    ) -> Harness {
        let pad = FakePad::default();
        let keys = FakeButtons::default();
        let foreground = FakeForeground(Arc::new(Mutex::new(("notepad".into(), "Untitled".into(), String::new(), String::new()))));
        let actions = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));

        let log = Arc::clone(&actions);
        let parts = EngineParts {
            transport: pad.clone(),
            foreground: Box::new(foreground.clone()),
            quick_launch: None,
            spotify: Some(Arc::new(FakeSpotify(Arc::clone(&actions)))),
            buttons: Some(Box::new(keys.clone())),
            meetings: Some(Arc::new(oasis_engine::meetings::Meetings::new(
                Arc::new(FakeCalendar),
                {
                    let log = Arc::clone(&actions);
                    Arc::new(move |url: &str| {
                        log.lock().unwrap().push(format!("open {url}"));
                        true
                    })
                },
            ))),
            voice,
            make_backend: Box::new(move |_: BackendContext| Box::new(Recorder(Arc::clone(&log)))),
        };
        let sink = Arc::clone(&events);
        let paths = temp_paths(name, Some(profiles));
        let engine = start(
            EngineConfig {
                paths: paths.clone(),
                integrity: Some(Box::new(Signature(signature_ok))),
                live,
                bridge: profiles.contains("BrowserBridgePort").then(|| BridgeConfig {
                    token: "test-token".into(),
                    bundled_extension: vec![],
                    port: None,
                }),
                startup_setup_after: None,
                // a test named "held-…" starts with the keyboard buttons held back
                hold_keys_at_start: name.starts_with("held-"),
            },
            parts,
            move |event| sink.lock().unwrap().push(event),
        )
        .expect("the engine starts");

        let harness = Harness { engine, paths, pad, keys, foreground, actions, events };
        harness.wait(|h| h.engine.status().pad_name.is_some(), "the pad to connect");
        harness
    }

    fn wait(&self, condition: impl Fn(&Harness) -> bool, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !condition(self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn actions(&self) -> Vec<String> {
        self.actions.lock().unwrap().clone()
    }

    fn presses(&self) -> Vec<oasis_engine::Press> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| if let EngineEvent::Press(p) = e { Some(p.clone()) } else { None })
            .collect()
    }

    /// Which panel of the program has the focus (an IDE's `terminal`, …).
    fn panel(&self, panel: &str) {
        self.foreground.0.lock().unwrap().3 = panel.into();
    }

    /// Puts an app in front and waits until the engine has seen it (it looks every 100 ms; a press
    /// reads it afresh anyway, the YouTube gestures use what was seen). OASIS's own window is never
    /// shown as the app in front, so for it there is nothing to wait for.
    fn focus(&self, process: &str, title: &str, url: &str) {
        let panel = self.foreground.0.lock().unwrap().3.clone();
        *self.foreground.0.lock().unwrap() = (process.into(), title.into(), url.into(), panel);
        if process != "<oasis>" {
            self.wait(
                |h| h.engine.status().front_app.is_some_and(|f| f.process == process && f.title == title),
                "the engine to see the app in front",
            );
        }
    }

    /// Wait until `n` actions have run, then give a stray second one time to show up.
    fn expect_actions(&self, n: usize) -> Vec<String> {
        self.wait(|h| h.actions().len() >= n, "the action to run");
        thread::sleep(Duration::from_millis(300));
        self.actions()
    }
}

// ---- tests ---------------------------------------------------------------------------------

#[test]
fn a_tap_runs_the_bound_action_exactly_once() {
    let h = Harness::start("tap", true, true);
    h.pad.tap(0);
    assert_eq!(h.expect_actions(1), ["shortcut [Ctrl, Char('1')]"]);
    let press = &h.presses()[0];
    assert_eq!((press.button, press.profile.as_str(), press.kind.as_str()), (1, "Default", "Shortcut"));
    assert!(press.runs);
}

/// The bug the first real-pad run found: with the generic layout the hold-2× button ran twice.
#[test]
fn a_tap_on_every_button_runs_once() {
    let h = Harness::start("each", true, true);
    h.pad.tap(0);
    h.pad.tap(1);
    h.pad.tap(2);
    assert_eq!(
        h.expect_actions(3),
        ["shortcut [Ctrl, Char('1')]", "shortcut [Ctrl, Char('2')]", "shortcut [Ctrl, Char('3')]"]
    );
}

#[test]
fn the_profile_follows_the_app_in_front() {
    let h = Harness::start("follow", true, true);
    h.focus("chrome", "Some page - Google Chrome", "");
    h.pad.tap(0);
    h.pad.tap(1);
    assert_eq!(h.expect_actions(2), ["shortcut [Alt, Char('1')]", "url https://example.com"]);
    assert_eq!(h.presses()[0].profile, "browser");
}

#[test]
fn a_blank_button_shows_a_press_but_runs_nothing() {
    let h = Harness::start("blank", true, true);
    h.focus("chrome", "x", "");
    h.pad.tap(2);
    h.wait(|h| !h.presses().is_empty(), "the press to be reported");
    thread::sleep(Duration::from_millis(300));
    assert!(h.actions().is_empty());
    assert!(!h.presses()[0].runs);
}

#[test]
fn preview_mode_shows_presses_and_runs_nothing_until_switched_on() {
    let h = Harness::start("preview", false, true);
    h.pad.tap(0);
    h.wait(|h| h.presses().len() == 1, "the press to be reported");
    thread::sleep(Duration::from_millis(300));
    assert!(h.actions().is_empty());
    assert!(!h.presses()[0].runs);
    assert!(!h.engine.status().live);

    assert!(h.engine.set_live(true));
    h.pad.tap(0);
    assert_eq!(h.expect_actions(1), ["shortcut [Ctrl, Char('1')]"]);
    assert!(h.presses()[1].runs);
}

#[test]
fn a_bad_signature_blocks_every_action_and_refuses_to_go_live() {
    let h = Harness::start("badsig", true, false);
    assert!(!h.engine.status().integrity_ok);
    assert!(!h.engine.status().live, "live is refused at start too");
    assert!(!h.engine.set_live(true));
    h.pad.tap(0);
    thread::sleep(Duration::from_millis(500));
    assert!(h.actions().is_empty());
    // The gesture rules refuse to dispatch while the file is untrusted, so there is no press
    // to report either; the user is told why through a note.
    assert!(h.presses().is_empty());
    assert!(h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::Note { .. })));
}

#[test]
fn holding_button_3_shows_the_desktop_and_its_release_is_not_a_tap() {
    let h = Harness::start("hold", true, true);
    h.pad.send("BTN:2:D\n");
    thread::sleep(Duration::from_millis(1100));
    h.pad.send("BTN:2:U\n");
    let ran = h.expect_actions(1);
    assert_eq!(ran, ["shortcut [Win, Char('D')]"]);
}

#[test]
fn battery_and_connection_show_up_in_the_status() {
    let h = Harness::start("status", true, true);
    h.pad.send("BAT:77\n");
    h.wait(|h| h.engine.status().battery_percent == Some(77), "the battery reading");
    let status = h.engine.status();
    assert_eq!(status.pad, "Connected");
    assert_eq!(status.pad_name.as_deref(), Some("OASIS_INTERFACE"));
    assert_eq!(status.profiles_loaded, 2);
    assert!(status.running);
}

// ---- what the new window needs ----------------------------------------------------------------

#[test]
fn on_the_oasis_window_the_pad_runs_its_action_like_anywhere_else() {
    let h = Harness::start("window-pad", true, true);
    h.focus("<oasis>", "OASIS", "");
    h.pad.tap(0);
    assert_eq!(h.expect_actions(1), ["shortcut [Ctrl, Char('1')]"]);
}

#[test]
fn the_status_keeps_the_last_app_in_front_that_was_not_oasis() {
    let h = Harness::start("front", true, true);
    h.focus("chrome", "Some page - Google Chrome", "");
    h.wait(|h| h.engine.status().front_app.is_some_and(|f| f.process == "chrome"), "chrome in front");
    assert_eq!(h.engine.status().front_profile, Some(1), "the `browser` profile matches chrome");

    h.focus("<oasis>", "OASIS", "");
    thread::sleep(Duration::from_millis(1200));
    let status = h.engine.status();
    assert_eq!(status.front_app.map(|f| (f.process, f.title)), Some(("chrome".into(), "Some page - Google Chrome".into())));

    h.focus("figma", "Untitled", "");
    h.wait(|h| h.engine.status().front_app.is_some_and(|f| f.process == "figma"), "figma in front");
    assert_eq!(h.engine.status().front_profile, Some(0), "only the everywhere profile matches figma");
}

/// Found in the real window: "Make profile" for the app you were just in, made while OASIS is in
/// front, must show up as that app's profile at once, not the next time it is in front.
#[test]
fn a_profile_made_while_oasis_is_in_front_matches_the_app_you_were_just_in() {
    let h = Harness::start("front-new-profile", true, true);
    h.focus("figma", "Untitled", "");
    h.wait(|h| h.engine.status().front_app.is_some_and(|f| f.process == "figma"), "figma in front");
    assert_eq!(h.engine.status().front_profile, Some(0), "only the everywhere profile matches figma so far");

    h.focus("<oasis>", "OASIS", "");
    let made = h.engine.change(|s| s.add_profile("Figma", "figma", "app")).expect("the profile is added");
    h.wait(|h| h.engine.status().front_profile == Some(made), "the new profile to match figma");
    assert_eq!(h.engine.status().front_app.map(|f| f.process).as_deref(), Some("figma"));
}

#[test]
fn presses_are_counted_per_profile_beside_the_profiles_file() {
    let h = Harness::start("counts", true, true);
    h.pad.tap(0);
    h.pad.tap(1);
    h.expect_actions(2);
    assert_eq!(h.engine.profile_use_counts().get("Default"), Some(&2));
    let file = h.paths.profiles().with_file_name("profile_use.json");
    assert!(fs::read_to_string(file).unwrap().contains("\"Default\": 2"));
}

// ---- changing settings while running ---------------------------------------------------------

fn bind(shortcut: &str) -> impl FnOnce(&mut oasis_settings::Settings) -> Result<(), oasis_settings::edit::EditError> + '_ {
    move |settings| {
        settings.set_button(
            0,
            0,
            0,
            oasis_settings::edit::ButtonEdit {
                kind: "Shortcut".into(),
                value: shortcut.into(),
                label: "A".into(),
                steps: vec![],
            },
        )
    }
}

#[test]
fn a_saved_change_takes_effect_on_the_next_press_and_is_written_to_disk() {
    let h = Harness::start("save", true, true);
    h.pad.tap(0);
    assert_eq!(h.expect_actions(1), ["shortcut [Ctrl, Char('1')]"]);

    h.engine.change(bind("Ctrl+9")).expect("the change saves");
    h.pad.tap(0);
    assert_eq!(h.expect_actions(2)[1], "shortcut [Ctrl, Char('9')]");

    let on_disk = fs::read_to_string(h.paths.profiles()).unwrap();
    assert!(on_disk.contains("Ctrl+9") && !on_disk.contains("\"Ctrl+1\""));
    assert_eq!(fs::read_to_string(h.paths.profiles().with_file_name("profiles.json.last-good")).unwrap(), on_disk);
    assert!(h.events.lock().unwrap().contains(&EngineEvent::SettingsChanged));
}

#[test]
fn a_change_the_rules_refuse_changes_nothing_anywhere() {
    let h = Harness::start("refuse", true, true);
    let before = fs::read_to_string(h.paths.profiles()).unwrap();
    let err = h.engine.change(bind("Ctrl+Nope")).unwrap_err();
    assert!(matches!(err, oasis_engine::ChangeError::Invalid(_)), "{err}");
    assert_eq!(fs::read_to_string(h.paths.profiles()).unwrap(), before);

    h.pad.tap(0);
    assert_eq!(h.expect_actions(1), ["shortcut [Ctrl, Char('1')]"]);
}

#[test]
fn a_change_is_refused_if_another_program_edited_the_file_meanwhile() {
    let h = Harness::start("conflict", true, true);
    // The C# app saves its own change.
    let theirs = PROFILES.replace("Ctrl+1", "Ctrl+7");
    fs::write(h.paths.profiles(), &theirs).unwrap();

    let err = h.engine.change(bind("Ctrl+9")).unwrap_err();
    assert!(matches!(err, oasis_engine::ChangeError::Save(_)), "{err}");
    assert_eq!(fs::read_to_string(h.paths.profiles()).unwrap(), theirs, "their file is untouched");

    // After a reload the change goes through on top of theirs.
    h.engine.reload_settings().unwrap();
    h.engine.change(bind("Ctrl+9")).expect("saves after the reload");
}

#[test]
fn reloading_picks_up_an_edit_made_outside_and_applies_it_to_the_next_press() {
    let h = Harness::start("reload", true, true);
    fs::write(h.paths.profiles(), PROFILES.replace("Ctrl+1", "Ctrl+7")).unwrap();
    h.engine.reload_settings().unwrap();
    h.pad.tap(0);
    assert_eq!(h.expect_actions(1), ["shortcut [Ctrl, Char('7')]"]);
}

#[test]
fn an_edit_made_outside_is_noticed_and_applied_without_asking() {
    let h = Harness::start("auto-reload", true, true);
    fs::write(h.paths.profiles(), PROFILES.replace("Ctrl+1", "Ctrl+7")).unwrap();
    // The engine looks about every 1.5 s; no call to reload_settings here.
    thread::sleep(Duration::from_millis(3500));
    h.pad.tap(0);
    assert_eq!(h.expect_actions(1), ["shortcut [Ctrl, Char('7')]"]);
    h.engine.change(bind("Ctrl+9")).expect("saves on top of the reloaded file, no conflict");
}

#[test]
fn reloading_a_missing_file_says_so_and_keeps_running_with_what_it_has() {
    let h = Harness::start("reload-missing", true, true);
    fs::remove_file(h.paths.profiles()).unwrap();
    let _ = fs::remove_file(h.paths.profiles().with_file_name("profiles.json.last-good"));
    assert!(h.engine.reload_settings().is_err());
    h.pad.tap(0);
    assert_eq!(h.expect_actions(1), ["shortcut [Ctrl, Char('1')]"]);
}

#[test]
fn an_untrusted_file_cannot_be_overwritten_until_the_user_accepts_it() {
    let h = Harness::start("accept", true, false);
    assert!(matches!(h.engine.change(bind("Ctrl+9")), Err(oasis_engine::ChangeError::Save(_))));
    assert!(!h.engine.set_live(true));

    h.engine.accept_edited_settings();
    assert!(h.engine.status().integrity_ok);
    assert!(h.engine.set_live(true));
    h.engine.change(bind("Ctrl+9")).expect("saves once accepted");

    h.pad.tap(0);
    assert_eq!(h.expect_actions(1), ["shortcut [Ctrl, Char('9')]"]);
}

#[test]
fn a_profile_added_while_running_applies_to_its_app_on_the_next_press() {
    let h = Harness::start("addprofile", true, true);
    let index = h.engine.change(|s| s.add_profile("Notes", "notepad", "app")).unwrap();
    assert_eq!(index, 2);
    assert_eq!(h.engine.status().profiles_loaded, 3);

    // A new profile has blank buttons: the press is reported but nothing runs.
    h.focus("notepad", "Untitled - Notepad", "");
    h.pad.tap(0);
    h.wait(|h| h.presses().len() == 1, "the press");
    assert_eq!(h.presses()[0].profile, "Notes");
    assert!(!h.presses()[0].runs);

    h.engine
        .change(|s| {
            s.set_button(
                2,
                0,
                0,
                oasis_settings::edit::ButtonEdit {
                    kind: "Shortcut".into(),
                    value: "Ctrl+N".into(),
                    label: "NEW".into(),
                    steps: vec![],
                },
            )
        })
        .unwrap();
    h.pad.tap(0);
    assert_eq!(h.expect_actions(1), ["shortcut [Ctrl, Char('N')]"]);
}

#[test]
fn deleting_a_profile_hands_its_app_back_to_the_fallback_profile() {
    let h = Harness::start("delprofile", true, true);
    h.focus("chrome", "x", "");
    h.pad.tap(0);
    h.wait(|h| h.presses().len() == 1, "the first press");
    assert_eq!(h.presses()[0].profile, "browser");

    h.engine.change(|s| s.delete_profile(1)).unwrap();
    h.pad.tap(0);
    h.wait(|h| h.presses().len() == 2, "the second press");
    assert_eq!(h.presses()[1].profile, "Default");
    // The last profile cannot be deleted, and the refusal changes nothing.
    h.engine.change(|s| s.delete_profile(0)).unwrap_err();
    assert_eq!(h.engine.settings().profiles.len(), 1);
}

#[test]
fn moving_a_profile_changes_which_one_wins_when_two_match() {
    let h = Harness::start("moveprofile", true, true);
    h.engine.change(|s| s.add_profile("Second browser", "chrome", "app")).unwrap();
    h.focus("chrome", "x", "");
    h.pad.tap(0);
    h.wait(|h| h.presses().len() == 1, "the first press");
    assert_eq!(h.presses()[0].profile, "browser", "the earlier profile wins");

    h.engine.change(|s| s.move_profile(2, 1)).unwrap();
    h.pad.tap(0);
    h.wait(|h| h.presses().len() == 2, "the second press");
    assert_eq!(h.presses()[1].profile, "Second browser");
}

fn automation(steps: Vec<(&str, &str)>) -> oasis_settings::edit::ButtonEdit {
    oasis_settings::edit::ButtonEdit {
        kind: "Automation".into(),
        value: String::new(),
        label: "MULTI".into(),
        steps: steps
            .into_iter()
            .map(|(kind, value)| oasis_core::actions::ActionStep {
                kind: kind.into(),
                value: value.into(),
                ..Default::default()
            })
            .collect(),
    }
}

#[test]
fn a_saved_multi_step_button_runs_its_steps_in_order() {
    let h = Harness::start("steps", true, true);
    h.engine
        .change(|s| {
            s.set_button(0, 0, 0, automation(vec![("Shortcut", "Ctrl+A"), ("Delay", "5"), ("Shortcut", "Ctrl+B")]))
        })
        .unwrap();
    h.pad.tap(0);
    assert_eq!(h.expect_actions(2), ["shortcut [Ctrl, Char('A')]", "shortcut [Ctrl, Char('B')]"]);
}

#[test]
fn the_status_follows_the_number_of_profiles() {
    let h = Harness::start("count", true, true);
    assert_eq!(h.engine.status().profiles_loaded, 2);
    h.engine.change(|s| s.update_profile(1, "Renamed", "chrome", "app")).unwrap();
    assert_eq!(h.engine.settings().profiles[1].name, "Renamed");
}

#[test]
fn stopping_ends_the_loop_and_says_so() {
    let mut h = Harness::start("stop", true, true);
    h.engine.stop();
    assert!(!h.engine.status().running);
    assert!(h.events.lock().unwrap().contains(&EngineEvent::Stopped));
    h.engine.stop(); // twice is fine
}

#[test]
fn without_a_profiles_file_there_is_a_clear_error() {
    let parts = EngineParts {
        transport: FakePad::default(),
        foreground: Box::new(FakeForeground(Arc::new(Mutex::new(Default::default())))),
        voice: None,
        make_backend: Box::new(|_| Box::new(Recorder(Arc::default()))),
        quick_launch: None,
        spotify: None,
        meetings: None,
        buttons: None,
    };
    let config = EngineConfig { paths: temp_paths("nofile", None), integrity: None, live: true, bridge: None, startup_setup_after: None, hold_keys_at_start: false };
    match start(config, parts, |_| {}) {
        Err(EngineError::NoProfiles(path)) => assert!(path.ends_with("profiles.json")),
        other => panic!("expected NoProfiles, got {:?}", other.err()),
    }
}

// ---- other ways of pressing the buttons ------------------------------------------------------

#[test]
fn a_button_pressed_from_the_keyboard_runs_like_one_pressed_on_the_pad() {
    let h = Harness::start("keys-tap", true, true);
    h.keys.press([(1, true), (1, false)]);
    assert_eq!(h.expect_actions(1), ["shortcut [Ctrl, Char('2')]"]);
    let press = &h.presses()[0];
    assert_eq!((press.button, press.profile.as_str()), (2, "Default"));
}

#[test]
fn a_keyboard_button_runs_its_action_even_on_the_oasis_window() {
    let h = Harness::start("keys-in-window", true, true);
    h.focus("<oasis>", "OASIS", "");
    h.keys.press([(0, true), (0, false)]);
    assert_eq!(h.expect_actions(1), ["shortcut [Ctrl, Char('1')]"]);
}

#[test]
fn the_keyboard_is_told_what_to_listen_for_and_follows_a_saved_change() {
    let h = Harness::start("keys-config", true, true);
    h.wait(|h| h.keys.asked.lock().unwrap().is_some(), "the keyboard to be configured");
    let (listen, chords, exclusive) = h.keys.asked.lock().unwrap().clone().unwrap();
    assert!(!listen, "off until the person turns it on");
    assert_eq!(chords[0], "F1");
    assert!(exclusive);

    let mine = oasis_settings::input::InputSettings {
        hardware: true,
        keyboard: true,
        keys: ["F13".into(), "F14".into(), "F15".into()],
        exclusive: false,
    };
    h.engine.change(|s| s.set_input(&mine).map_err(oasis_settings::edit::EditError::Invalid)).unwrap();
    h.wait(|h| h.keys.asked.lock().unwrap().as_ref().is_some_and(|(on, _, _)| *on), "the keyboard to be switched on");
    let (listen, chords, exclusive) = h.keys.asked.lock().unwrap().clone().unwrap();
    assert!(listen && !exclusive);
    assert_eq!(chords, ["F13", "F14", "F15"]);
}

/// Keyboard input on, with these three keys, so a test can see which configure call it is looking at.
fn keyboard_on(h: &Harness, keys: [&str; 3]) {
    let mine = oasis_settings::input::InputSettings { keyboard: true, keys: keys.map(String::from), ..Default::default() };
    h.engine.change(|s| s.set_input(&mine).map_err(oasis_settings::edit::EditError::Invalid)).unwrap();
}

fn listening(h: &Harness, first_key: &str) -> Option<bool> {
    h.keys.asked.lock().unwrap().as_ref().filter(|(_, chords, _)| chords[0] == first_key).map(|(on, _, _)| *on)
}

#[test]
fn held_keys_stop_listening_and_come_back_when_let_go_without_changing_the_setting() {
    let h = Harness::start("keys-hold", true, true);
    keyboard_on(&h, ["F13", "F14", "F15"]);
    h.wait(|h| listening(h, "F13") == Some(true), "the keyboard to be switched on");

    h.engine.hold_keys(true);
    h.wait(|h| listening(h, "F13") == Some(false), "the keys to be held back");
    h.engine.hold_keys(false);
    h.wait(|h| listening(h, "F13") == Some(true), "the keys to be let go");
}

#[test]
fn an_engine_started_with_the_keys_held_does_not_listen_until_they_are_let_go() {
    let h = Harness::start("held-start", true, true);
    // switched on while held: the engine takes up the new keys, and still does not listen
    keyboard_on(&h, ["F16", "F17", "F18"]);
    h.wait(|h| listening(h, "F16").is_some(), "the new keys to be taken up");
    assert_eq!(listening(&h, "F16"), Some(false), "held from the start");

    h.engine.hold_keys(false);
    h.wait(|h| listening(h, "F16") == Some(true), "the keys to be let go");
}

#[test]
fn turning_the_pad_off_lets_go_of_it_and_the_keyboard_still_works() {
    let h = Harness::start("pad-off", true, true);
    let keyboard_only = oasis_settings::input::InputSettings { hardware: false, keyboard: true, ..Default::default() };
    h.engine.change(|s| s.set_input(&keyboard_only).map_err(oasis_settings::edit::EditError::Invalid)).unwrap();
    h.wait(|h| h.engine.status().pad == "Off" && h.engine.status().pad_name.is_none(), "the pad to be let go");
    h.keys.press([(0, true), (0, false)]);
    assert_eq!(h.expect_actions(1), ["shortcut [Ctrl, Char('1')]"]);
}

#[test]
fn the_engine_refuses_to_switch_every_way_off() {
    let h = Harness::start("all-off", true, true);
    let none = oasis_settings::input::InputSettings { hardware: false, keyboard: false, ..Default::default() };
    assert!(h.engine.change(|s| s.set_input(&none).map_err(oasis_settings::edit::EditError::Invalid)).is_err());
}

// ---- label peek ----------------------------------------------------------------------------

fn labels_on(profile_mode: &str) -> String {
    PROFILES
        .replace("\"ShowOverlayLabels\": false", "\"ShowOverlayLabels\": true")
        .replace("\"Name\": \"browser\",", &format!("\"Name\": \"browser\", \"LabelMode\": \"{profile_mode}\","))
}

fn shown(h: &Harness) -> Vec<(String, Vec<String>)> {
    h.events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            EngineEvent::ShowLabels { profile, labels, .. } => Some((profile.clone(), labels.clone())),
            _ => None,
        })
        .collect()
}

fn hidden(h: &Harness) -> usize {
    h.events.lock().unwrap().iter().filter(|e| matches!(e, EngineEvent::HideLabels)).count()
}

#[test]
fn the_first_tap_only_shows_the_labels_and_the_second_runs() {
    let h = Harness::start_with("peek", &labels_on("Inherit"), true, true);
    h.pad.tap(0);
    h.wait(|h| !shown(h).is_empty(), "the labels to show");
    assert_eq!(shown(&h), [("Default".to_string(), vec!["A".to_string(), "B".into(), "C".into()])]);
    thread::sleep(Duration::from_millis(400));
    assert!(h.actions().is_empty(), "the revealing tap must not run anything");

    h.pad.tap(0);
    assert_eq!(h.expect_actions(1), ["shortcut [Ctrl, Char('1')]"]);
    h.wait(|h| hidden(h) == 1, "the labels to go away once something ran");
}

#[test]
fn a_profile_can_switch_labels_off_over_the_general_setting() {
    let h = Harness::start_with("peek-off", &labels_on("Hide"), true, true);
    h.focus("chrome", "x", "");
    h.pad.tap(0);
    assert_eq!(h.expect_actions(1), ["shortcut [Alt, Char('1')]"]);
    assert!(shown(&h).is_empty());
}

#[test]
fn pad_buttons_going_down_and_up_are_reported_for_the_indicator() {
    let h = Harness::start("indicator", true, true);
    h.pad.tap(1);
    h.wait(
        |h| {
            let events = h.events.lock().unwrap();
            events.contains(&EngineEvent::PadButton { button: 2, down: true })
                && events.contains(&EngineEvent::PadButton { button: 2, down: false })
        },
        "button 2 down and up",
    );
}

// ---- the browser extension ------------------------------------------------------------------

use tungstenite::Message;

fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port()
}

/// Chrome profile with a main set and a set for "a YouTube video is playing".
fn youtube_profiles(port: u16) -> String {
    format!(
        r#"{{
  "SchemaVersion": 2, "ShowOverlayLabels": false, "BrowserBridgeEnabled": true, "BrowserBridgePort": {port},
  "YouTube": {{ "GesturesEnabled": true, "Hold2xButtonIndex": 1, "VolumeUpButtonIndex": 0,
               "VolumeDownButtonIndex": 2, "Hold2xThresholdMs": 250, "DoubleClickMs": 300 }},
  "Profiles": [
    {{ "Name": "Default", "ContextMatch": "*", "Kind": "", "MultiConfigEnabled": false,
      "ConfigProfiles": [ {{ "Name": "Default", "ContextMatch": "*", "Buttons": [
        {{ "Type": "Shortcut", "Value": "Ctrl+1", "Label": "A", "Steps": "" }},
        {{ "Type": "Shortcut", "Value": "Ctrl+2", "Label": "B", "Steps": "" }},
        {{ "Type": "Shortcut", "Value": "Ctrl+3", "Label": "C", "Steps": "" }} ] }} ] }},
    {{ "Name": "chrome", "ContextMatch": "chrome", "Kind": "app", "MultiConfigEnabled": true,
      "ConfigProfiles": [
        {{ "Name": "browser", "ContextMatch": "*", "Buttons": [
          {{ "Type": "Shortcut", "Value": "Alt+1", "Label": "A", "Steps": "" }},
          {{ "Type": "Shortcut", "Value": "Alt+2", "Label": "B", "Steps": "" }},
          {{ "Type": "Shortcut", "Value": "Alt+3", "Label": "C", "Steps": "" }} ] }},
        {{ "Name": "Watching", "ContextMatch": "", "When": "youtube.playing", "Buttons": [
          {{ "Type": "Shortcut", "Value": "K", "Label": "PAUSE", "Steps": "" }},
          {{ "Type": "Shortcut", "Value": "Shift+N", "Label": "NEXT", "Steps": "" }},
          {{ "Type": "Shortcut", "Value": "F", "Label": "FULL", "Steps": "" }} ] }} ] }}
  ]
}}"#
    )
}

const WATCH_URL: &str = "https://www.youtube.com/watch?v=abc";

type Extension = tungstenite::WebSocket<std::net::TcpStream>;

/// An extension: connects with the token and says hello; checks it was sent the settings.
fn extension(port: u16, token: &str) -> Extension {
    use tungstenite::client::IntoClientRequest;
    let stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut request = format!("ws://127.0.0.1:{port}/oasis").into_client_request().unwrap();
    request.headers_mut().insert("Origin", "chrome-extension://abc".parse().unwrap());
    let (mut socket, _) = tungstenite::client(request, stream).expect("the handshake works");
    socket.send(Message::text(format!(r#"{{"type":"hello","token":"{token}"}}"#))).unwrap();
    let first = socket.read().unwrap();
    let settings: serde_json::Value = serde_json::from_str(&first.to_string()).unwrap_or_else(|_| panic!("expected settings, got {first:?}"));
    assert_eq!(settings["type"], "oasis.settings");
    socket
}

fn start_with_extension(name: &str) -> (Harness, Extension) {
    let port = free_port();
    let h = Harness::start_with(name, &youtube_profiles(port), true, true);
    h.wait(|h| h.engine.status().bridge_status.starts_with("LISTENING"), "the bridge to listen");
    let socket = extension(port, "test-token");
    h.wait(|h| h.engine.status().bridge_clients == 1, "the extension to be counted");
    (h, socket)
}

fn read_command(socket: &mut Extension) -> String {
    let message: serde_json::Value = serde_json::from_str(&socket.read().unwrap().to_string()).unwrap();
    assert_eq!(message["type"], "oasis.command");
    message["command"].as_str().unwrap().to_string()
}

fn expect_silence(socket: &mut Extension) {
    socket.get_ref().set_read_timeout(Some(Duration::from_millis(500))).unwrap();
    assert!(socket.read().is_err(), "no command was sent");
}

fn video_playing(h: &Harness, socket: &mut Extension) {
    socket
        .send(Message::text(format!(
            r#"{{"type":"youtube.started","videoId":"abc","title":"T","url":"{WATCH_URL}","surface":"watch"}}"#
        )))
        .unwrap();
    h.wait(|h| h.engine.status().youtube == "youtube.playing", "the video to count as playing");
}

#[test]
fn the_bridge_status_and_connected_browsers_are_reported() {
    let (h, _socket) = start_with_extension("bridge-status");
    assert!(h.engine.status().bridge_status.starts_with("LISTENING · 127.0.0.1:"));
    assert_eq!(h.engine.status().bridge_clients, 1);
}

#[test]
fn a_playing_youtube_video_picks_the_youtube_set_of_buttons() {
    let (h, mut socket) = start_with_extension("yt-set");
    h.focus("chrome", "T - YouTube - Google Chrome", WATCH_URL);

    h.pad.tap(0);
    assert_eq!(h.expect_actions(1), ["shortcut [Alt, Char('1')]"], "before any video: the main set");

    video_playing(&h, &mut socket);
    h.pad.tap(1);
    h.wait(|h| h.actions().len() >= 2, "the second action");
    assert_eq!(h.actions()[1], "shortcut [Shift, Char('N')]", "now the YouTube set");
    assert_eq!(h.presses().last().unwrap().config, "Watching");
}

#[test]
fn the_youtube_set_is_not_used_when_another_site_is_in_front() {
    let (h, mut socket) = start_with_extension("yt-other");
    video_playing(&h, &mut socket);
    h.focus("chrome", "Example Domain - Google Chrome", "https://example.com/");
    h.pad.tap(0);
    assert_eq!(h.expect_actions(1), ["shortcut [Alt, Char('1')]"]);
}

#[test]
fn holding_the_2x_button_on_a_watch_page_tells_the_extension_and_releasing_restores() {
    let (h, mut socket) = start_with_extension("yt-hold");
    h.focus("chrome", "T - YouTube - Google Chrome", WATCH_URL);
    video_playing(&h, &mut socket);

    h.pad.send("BTN:1:D\n");
    assert_eq!(read_command(&mut socket), "playbackRate.hold2x");
    h.pad.send("BTN:1:U\n");
    assert_eq!(read_command(&mut socket), "playbackRate.restore");
    thread::sleep(Duration::from_millis(400));
    assert!(h.actions().is_empty(), "a hold is not a tap");
}

#[test]
fn holding_that_button_anywhere_else_does_nothing_to_the_extension() {
    let (h, mut socket) = start_with_extension("yt-hold-off");
    h.focus("code", "main.rs - Visual Studio Code", "");
    video_playing(&h, &mut socket);

    h.pad.send("BTN:1:D\n");
    thread::sleep(Duration::from_millis(600));
    h.pad.send("BTN:1:U\n");
    expect_silence(&mut socket);
}

#[test]
fn on_a_playing_video_two_taps_run_the_button_twice_and_never_change_the_volume() {
    // Double-click for volume is switched off (2026-11-18): each tap is the button's own action.
    let (h, mut socket) = start_with_extension("yt-volume");
    h.focus("chrome", "T - YouTube - Google Chrome", WATCH_URL);
    video_playing(&h, &mut socket);

    h.pad.tap(0);
    h.pad.tap(0);
    let actions = h.expect_actions(2);
    assert_eq!(actions, ["shortcut [Char('K')]", "shortcut [Char('K')]"], "{actions:?}");
}

#[test]
fn shorts_do_not_arm_the_gestures() {
    let (h, mut socket) = start_with_extension("yt-short");
    h.focus("chrome", "S - YouTube - Google Chrome", "https://www.youtube.com/shorts/x");
    socket.send(Message::text(r#"{"type":"youtube.started","videoId":"x","surface":"shorts"}"#)).unwrap();
    h.wait(|h| h.engine.status().youtube == "youtube.shorts", "the Short to be noticed");
    h.pad.send("BTN:1:D\n");
    thread::sleep(Duration::from_millis(600));
    h.pad.send("BTN:1:U\n");
    expect_silence(&mut socket);
}

#[test]
fn a_wrong_token_never_changes_what_youtube_is_doing() {
    use tungstenite::client::IntoClientRequest;
    let port = free_port();
    let h = Harness::start_with("yt-badtoken", &youtube_profiles(port), true, true);
    h.wait(|h| h.engine.status().bridge_status.starts_with("LISTENING"), "the bridge to listen");
    let stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut request = format!("ws://127.0.0.1:{port}/").into_client_request().unwrap();
    request.headers_mut().insert("Origin", "chrome-extension://abc".parse().unwrap());
    let (mut socket, _) = tungstenite::client(request, stream).unwrap();
    socket.send(Message::text(r#"{"type":"hello","token":"wrong"}"#)).unwrap();
    let _ = socket.send(Message::text(r#"{"type":"youtube.started","videoId":"abc","surface":"watch"}"#));
    thread::sleep(Duration::from_millis(500));
    assert_eq!(h.engine.status().youtube, "");
}

#[test]
fn switching_the_bridge_off_in_the_settings_stops_listening() {
    let (h, _socket) = start_with_extension("yt-off");
    h.engine
        .change(|s| {
            s.extra.insert("BrowserBridgeEnabled".into(), serde_json::Value::Bool(false));
            Ok(())
        })
        .unwrap();
    h.wait(|h| h.engine.status().bridge_status == "STOPPED", "the bridge to stop");
}

// ---- push-to-talk ---------------------------------------------------------------------------

use oasis_engine::{VoiceMic, VoiceParts};
use oasis_models::{AudioClip, ModelError, SttEngine};

#[derive(Default)]
struct MicLog {
    started: u32,
    stopped: u32,
    running: bool,
}

/// A microphone that "hears" `ms` milliseconds each time.
#[derive(Clone)]
struct FakeMic(Arc<Mutex<MicLog>>, usize);

impl VoiceMic for FakeMic {
    fn start(&mut self) -> Result<(), String> {
        let mut log = self.0.lock().unwrap();
        log.started += 1;
        log.running = true;
        Ok(())
    }
    fn stop(&mut self) -> AudioClip {
        let mut log = self.0.lock().unwrap();
        log.stopped += 1;
        log.running = false;
        AudioClip { samples: vec![0.1; self.1 * 16], sample_rate: 16_000 }
    }
}

struct FakeStt(Arc<Mutex<u32>>, String);

impl SttEngine for FakeStt {
    fn transcribe(&mut self, _: &AudioClip) -> Result<String, ModelError> {
        *self.0.lock().unwrap() += 1;
        Ok(self.1.clone())
    }
}

struct Voiced {
    h: Harness,
    mic: Arc<Mutex<MicLog>>,
    stt_calls: Arc<Mutex<u32>>,
    /// How many times the speech engine was loaded.
    loads: Arc<Mutex<u32>>,
}

fn voice_profiles(enabled: bool) -> String {
    PROFILES.replace(
        "\"ShowOverlayLabels\": false,",
        &format!(
            "\"ShowOverlayLabels\": false, \"Voice\": {{ \"Enabled\": {enabled}, \"PushToTalkMask\": [0], \"PushToTalkHoldMs\": 350, \"MinUtteranceMs\": 300 }},"
        ),
    )
}

fn start_voiced(name: &str, live: bool, enabled: bool, heard_ms: usize, says: &str) -> Voiced {
    let mic = Arc::new(Mutex::new(MicLog::default()));
    let stt_calls = Arc::new(Mutex::new(0));
    let loads = Arc::new(Mutex::new(0));
    let (mic_for, calls_for, says, loads_for) = (Arc::clone(&mic), Arc::clone(&stt_calls), says.to_string(), Arc::clone(&loads));
    let parts = VoiceParts {
        pause_media: Box::new(Vec::new),
        resume_media: Box::new(|_| {}),
        make_mic: Box::new(move |_| Box::new(FakeMic(Arc::clone(&mic_for), heard_ms))),
        model_ready: Box::new(|_| true),
        load_stt: Box::new(move |_| {
            *loads_for.lock().unwrap() += 1;
            Ok(Box::new(FakeStt(Arc::clone(&calls_for), says.clone())))
        }),
        editable_focus: Box::new(|| true),
        running_apps: Box::new(Vec::new),
        app_installed: Box::new(pretend_installed),
        on_screen: std::sync::Arc::new(Vec::new),
        app_names: Box::new(Vec::new),
    };
    let h = Harness::start_full(name, &voice_profiles(enabled), live, true, Some(parts));
    Voiced { h, mic, stt_calls, loads }
}

fn phases(h: &Harness) -> Vec<String> {
    h.events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| if let EngineEvent::Voice { phase } = e { Some(phase.clone()) } else { None })
        .collect()
}

fn transcripts(h: &Harness) -> Vec<(String, bool)> {
    h.events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| if let EngineEvent::Transcript { text, typed } = e { Some((text.clone(), *typed)) } else { None })
        .collect()
}

/// Hold the talk button long enough to count as a hold, then let go.
fn hold_to_talk(h: &Harness) {
    h.pad.send("BTN:0:D\n");
    thread::sleep(Duration::from_millis(800));
    h.pad.send("BTN:0:U\n");
}

#[test]
fn holding_the_talk_button_records_transcribes_and_types_what_was_said() {
    let v = start_voiced("ptt-types", true, true, 1500, "Hello world.");
    hold_to_talk(&v.h);
    v.h.wait(|h| h.actions().iter().any(|a| a.starts_with("paste")), "the text to be typed");
    assert_eq!(v.h.actions(), ["paste Hello world. "], "typed once, and the tap did not also run");
    assert_eq!(transcripts(&v.h), [("Hello world.".to_string(), true)]);
    let phases = phases(&v.h);
    assert!(phases.contains(&"dictating".to_string()) || phases.contains(&"listening".to_string()), "{phases:?}");
    assert_eq!(phases.last().map(String::as_str), Some("idle"), "{phases:?}");
    let log = v.mic.lock().unwrap();
    assert_eq!((log.started, log.stopped, log.running), (1, 1, false), "one recording, and the pre-roll is the recording");
}

/// The pill is up before the focus question is asked (it can take 250 ms), so the user sees at once
/// that they can talk; the decision it feeds is unchanged.
#[test]
fn the_pill_shows_before_the_focus_is_asked() {
    // The harness's event list, handed to the focus probe once the engine is up.
    let events: Arc<Mutex<Option<Arc<Mutex<Vec<EngineEvent>>>>>> = Arc::default();
    let shown_when_asked = Arc::new(Mutex::new(None));
    let (seen, asked) = (Arc::clone(&events), Arc::clone(&shown_when_asked));
    let mic = Arc::new(Mutex::new(MicLog::default()));
    let parts = VoiceParts {
        pause_media: Box::new(Vec::new),
        resume_media: Box::new(|_| {}),
        make_mic: Box::new(move |_| Box::new(FakeMic(Arc::clone(&mic), 1500))),
        model_ready: Box::new(|_| true),
        load_stt: Box::new(|_| Ok(Box::new(FakeStt(Arc::new(Mutex::new(0)), "Hello world.".into())))),
        editable_focus: Box::new(move || {
            let list = seen.lock().unwrap().clone().expect("the engine is up");
            let shown = list.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::Voice { phase } if phase == "listening"));
            asked.lock().unwrap().get_or_insert(shown);
            true
        }),
        running_apps: Box::new(Vec::new),
        app_installed: Box::new(pretend_installed),
        on_screen: std::sync::Arc::new(Vec::new),
        app_names: Box::new(Vec::new),
    };
    let h = Harness::start_full("ptt-feedback-first", &voice_profiles(true), true, true, Some(parts));
    *events.lock().unwrap() = Some(Arc::clone(&h.events));
    hold_to_talk(&h);
    h.wait(|h| h.actions().iter().any(|a| a.starts_with("paste")), "the text to be typed");
    assert_eq!(*shown_when_asked.lock().unwrap(), Some(true), "the pill came first");
}

#[test]
fn a_dictation_is_kept_with_its_recording_only_when_the_history_is_on() {
    let keeping = voice_profiles(true).replace("\"MinUtteranceMs\": 300", "\"MinUtteranceMs\": 300, \"KeepHistory\": true");
    let v = start_gate_with("history-on", &keeping, true, &[], "Hello world.");
    hold_to_talk(&v.h);
    v.h.wait(|h| h.actions().iter().any(|a| a.starts_with("paste")), "the text to be typed");
    let kept = oasis_engine::history::list(&v.h.paths.dictation_history_folder());
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!((kept[0].text.as_str(), kept[0].raw.as_str(), kept[0].has_audio), ("Hello world.", "Hello world.", true));
    assert!(kept[0].audio_ms >= 1000, "the whole recording: {} ms", kept[0].audio_ms);
    assert_eq!(&oasis_engine::history::audio(&v.h.paths.dictation_history_folder(), &kept[0].id).unwrap()[..4], b"RIFF");

    let v = start_gate_with("history-off", &voice_profiles(true), true, &[], "Hello world.");
    hold_to_talk(&v.h);
    v.h.wait(|h| h.actions().iter().any(|a| a.starts_with("paste")), "the text to be typed");
    assert!(!v.h.paths.dictation_history_folder().exists(), "nothing is kept unless the user asked");
}

#[test]
fn the_speech_model_is_loaded_at_start_not_on_the_first_recording() {
    let v = start_voiced("ptt-preload", true, true, 1500, "Hello world.");
    let started = std::time::Instant::now();
    while *v.loads.lock().unwrap() == 0 && started.elapsed() < Duration::from_secs(3) {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(*v.loads.lock().unwrap(), 1, "loaded before anyone spoke");
    assert_eq!(*v.stt_calls.lock().unwrap(), 0);
    hold_to_talk(&v.h);
    v.h.wait(|h| h.actions().iter().any(|a| a.starts_with("paste")), "the text to be typed");
    assert_eq!(*v.loads.lock().unwrap(), 1, "the recording used the loaded model");

    let off = start_voiced("ptt-preload-off", true, false, 1500, "x");
    thread::sleep(Duration::from_millis(300));
    assert_eq!(*off.loads.lock().unwrap(), 0, "voice off: nothing is loaded");
}

#[test]
fn dictation_is_cleaned_up_before_it_is_typed() {
    let v = start_voiced("ptt-cleanup", true, true, 1500, "um send it today comma then call me at five fifteen");
    hold_to_talk(&v.h);
    v.h.wait(
        |h| h.actions().iter().any(|a| a.starts_with("paste")),
        "the text to be typed",
    );
    assert_eq!(
        v.h.actions(),
        ["paste Send it today, then call me at 5:15. "]
    );
    let levels = v.h.events.lock().unwrap().iter().filter(|e| matches!(e, EngineEvent::VoiceLevel { .. })).count();
    assert!(levels > 0, "the pill's bars are fed while the mic is open");
    assert_eq!(
        transcripts(&v.h),
        [("Send it today, then call me at 5:15.".to_string(), true)]
    );

    // One timing line for the recording: durations and counts, never the words.
    // Written just after the paste is handed on, so it may land a moment after the paste is seen.
    let read_log = || fs::read_to_string(v.h.paths.voice_timing_log()).unwrap_or_default();
    v.h.wait(|_| read_log().contains(" dictate ") && read_log().contains(" cleaned "), "the timing lines");
    let log = read_log();
    assert_eq!(log.lines().count(), 3, "press, heard, cleaned: {log}");
    assert!(log.contains(" - cleaned style=Default cleaned_ms="), "cleanup time after release: {log}");
    assert!(log.lines().next().unwrap().contains(" - press cue_ms="), "the press line comes first: {log}");
    assert!(
        log.contains("dictate stt=local audio_ms=1500 ") && log.contains(" words=11"),
        "words heard, before cleanup: {log}"
    );
    assert!(!log.contains("ms=-"), "release was stamped: {log}");
    assert!(
        !log.to_lowercase().contains("today"),
        "no dictated text in the log: {log}"
    );
}

#[test]
fn a_quick_tap_on_the_talk_button_is_a_tap_not_a_recording() {
    let v = start_voiced("ptt-tap", true, true, 1500, "Should not be heard");
    v.h.pad.tap(0);
    assert_eq!(v.h.expect_actions(1), ["shortcut [Ctrl, Char('1')]"], "its normal action ran");
    thread::sleep(Duration::from_millis(500));
    assert_eq!(*v.stt_calls.lock().unwrap(), 0, "nothing was transcribed");
    let log = v.mic.lock().unwrap();
    assert!(!log.running, "the pre-roll was dropped");
    assert!(transcripts(&v.h).is_empty());
}

#[test]
fn a_recording_shorter_than_the_minimum_is_ignored() {
    let v = start_voiced("ptt-short", true, true, 100, "Should not be heard");
    hold_to_talk(&v.h);
    thread::sleep(Duration::from_millis(500));
    assert_eq!(*v.stt_calls.lock().unwrap(), 0);
    assert!(v.h.actions().iter().all(|a| !a.starts_with("paste")));
}

#[test]
fn in_preview_mode_what_was_said_is_shown_and_not_typed() {
    let v = start_voiced("ptt-preview", false, true, 1500, "Just testing.");
    hold_to_talk(&v.h);
    v.h.wait(|h| !transcripts(h).is_empty(), "the transcript");
    assert_eq!(transcripts(&v.h), [("Just testing.".to_string(), false)]);
    thread::sleep(Duration::from_millis(300));
    assert!(v.h.actions().iter().all(|a| !a.starts_with("paste")));
}

#[test]
fn with_voice_switched_off_the_talk_button_never_listens() {
    let v = start_voiced("ptt-off", true, false, 1500, "Should not be heard");
    hold_to_talk(&v.h);
    thread::sleep(Duration::from_millis(500));
    assert_eq!(v.mic.lock().unwrap().started, 0, "the microphone was never opened");
    assert_eq!(*v.stt_calls.lock().unwrap(), 0);
}

#[test]
fn silence_types_nothing() {
    let v = start_voiced("ptt-silence", true, true, 1500, "");
    hold_to_talk(&v.h);
    v.h.wait(|h| phases(h).last().map(String::as_str) == Some("idle") && *v.stt_calls.lock().unwrap() == 1, "the empty result");
    thread::sleep(Duration::from_millis(300));
    assert!(v.h.actions().iter().all(|a| !a.starts_with("paste")));
    assert!(transcripts(&v.h).is_empty());
}

#[test]
fn a_video_playing_in_the_browser_is_paused_while_talking_and_resumed_after() {
    let port = free_port();
    let mic = Arc::new(Mutex::new(MicLog::default()));
    let calls = Arc::new(Mutex::new(0));
    let (mic_for, calls_for) = (Arc::clone(&mic), Arc::clone(&calls));
    let parts = VoiceParts {
        pause_media: Box::new(Vec::new),
        resume_media: Box::new(|_| {}),
        make_mic: Box::new(move |_| Box::new(FakeMic(Arc::clone(&mic_for), 1500))),
        model_ready: Box::new(|_| true),
        load_stt: Box::new(move |_| Ok(Box::new(FakeStt(Arc::clone(&calls_for), "Next.".into())))),
        editable_focus: Box::new(|| true),
        running_apps: Box::new(Vec::new),
        app_installed: Box::new(pretend_installed),
        on_screen: std::sync::Arc::new(Vec::new),
        app_names: Box::new(Vec::new),
    };
    let profiles = youtube_profiles(port).replace(
        "\"SchemaVersion\": 2,",
        "\"SchemaVersion\": 2, \"Voice\": { \"Enabled\": true, \"PushToTalkMask\": [2], \"PushToTalkHoldMs\": 350, \"MinUtteranceMs\": 300 },",
    );
    let h = Harness::start_full("ptt-media", &profiles, true, true, Some(parts));
    h.wait(|h| h.engine.status().bridge_status.starts_with("LISTENING"), "the bridge to listen");
    let mut socket = extension(port, "test-token");
    h.wait(|h| h.engine.status().bridge_clients == 1, "the extension to be counted");
    h.focus("chrome", "T - YouTube - Google Chrome", WATCH_URL);
    video_playing(&h, &mut socket);

    h.pad.send("BTN:2:D\n");
    assert_eq!(read_command(&mut socket), "playback.pause");
    thread::sleep(Duration::from_millis(600));
    h.pad.send("BTN:2:U\n");
    assert_eq!(read_command(&mut socket), "playback.resume");
}

// ---- spoken commands ------------------------------------------------------------------------

/// The `Voice` section with extra settings (`"CaptureMode": 1, ...`).
fn command_profiles(extra: &str) -> String {
    PROFILES.replace(
        "\"ShowOverlayLabels\": false,",
        &format!(
            "\"ShowOverlayLabels\": false, \"Voice\": {{ \"Enabled\": true, \"PushToTalkMask\": [0], \"PushToTalkHoldMs\": 350, \"MinUtteranceMs\": 300, {extra} }},"
        ),
    )
}

fn start_commands(name: &str, live: bool, extra: &str, says: &str) -> Voiced {
    let mic = Arc::new(Mutex::new(MicLog::default()));
    let stt_calls = Arc::new(Mutex::new(0));
    let loads = Arc::new(Mutex::new(0));
    let (mic_for, calls_for, says, loads_for) = (Arc::clone(&mic), Arc::clone(&stt_calls), says.to_string(), Arc::clone(&loads));
    let parts = VoiceParts {
        pause_media: Box::new(Vec::new),
        resume_media: Box::new(|_| {}),
        make_mic: Box::new(move |_| Box::new(FakeMic(Arc::clone(&mic_for), 1500))),
        model_ready: Box::new(|_| true),
        load_stt: Box::new(move |_| {
            *loads_for.lock().unwrap() += 1;
            Ok(Box::new(FakeStt(Arc::clone(&calls_for), says.clone())))
        }),
        editable_focus: Box::new(|| true),
        running_apps: Box::new(Vec::new),
        app_installed: Box::new(pretend_installed),
        on_screen: std::sync::Arc::new(Vec::new),
        app_names: Box::new(Vec::new),
    };
    let h = Harness::start_full(name, &command_profiles(extra), live, true, Some(parts));
    Voiced { h, mic, stt_calls, loads }
}

fn commands(h: &Harness) -> Vec<(String, String)> {
    h.events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            EngineEvent::VoiceCommand { label, outcome, .. } => Some((label.clone(), outcome.clone())),
            _ => None,
        })
        .collect()
}

fn wait_for_command(h: &Harness) {
    h.wait(|h| !commands(h).is_empty(), "a spoken command to be handled");
}

/// Press and release a pad button (0-based), for answering a question.
fn press(h: &Harness, button: u8) {
    h.pad.send(&format!("BTN:{button}:D\n"));
    thread::sleep(Duration::from_millis(60));
    h.pad.send(&format!("BTN:{button}:U\n"));
}

const COMMAND_MODE: &str = "\"CaptureMode\": 1, \"IntentApprovalMode\": 2";

#[test]
fn a_spoken_media_command_runs_without_asking() {
    let v = start_commands("cmd-mute", true, COMMAND_MODE, "Mute.");
    hold_to_talk(&v.h);
    assert_eq!(v.h.expect_actions(1), ["shortcut [VolumeMute]"]);
    assert_eq!(commands(&v.h), [("MUTE".to_string(), "ran".to_string())]);
    assert!(transcripts(&v.h).is_empty(), "it was a command, not typing");
}

#[test]
fn a_spoken_open_launches_the_app_when_routine_commands_may_run() {
    let v = start_commands("cmd-open", true, COMMAND_MODE, "Open VS Code.");
    hold_to_talk(&v.h);
    assert_eq!(v.h.expect_actions(1), ["app code"]);
}

#[test]
fn a_new_app_asks_on_the_pad_and_button_1_runs_it_and_remembers_it() {
    let v = start_commands("cmd-ask-yes", true, "\"CaptureMode\": 1", "Open notepad.");
    hold_to_talk(&v.h);
    v.h.wait(|h| h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::Confirm { .. })), "the question");
    thread::sleep(Duration::from_millis(300));
    assert!(v.h.actions().is_empty(), "nothing runs before the answer");

    press(&v.h, 0);
    assert_eq!(v.h.expect_actions(1), ["app notepad"]);
    assert!(v.h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::ConfirmEnd { approved: true })));
    let saved = fs::read_to_string(v.h.paths.profiles()).unwrap();
    assert!(saved.contains("launchapp:notepad"), "the approval is remembered: {saved}");

    // Asked once only: the same command now runs at once.
    hold_to_talk(&v.h);
    v.h.wait(|h| h.actions().len() >= 2, "the second run");
    assert_eq!(v.h.actions()[1], "app notepad");
}

#[test]
fn button_2_cancels_the_question_and_nothing_runs() {
    let v = start_commands("cmd-ask-no", true, "\"CaptureMode\": 1", "Open notepad.");
    hold_to_talk(&v.h);
    v.h.wait(|h| h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::Confirm { .. })), "the question");
    press(&v.h, 1);
    v.h.wait(|h| h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::ConfirmEnd { approved: false })), "the answer");
    thread::sleep(Duration::from_millis(300));
    assert!(v.h.actions().is_empty());
    assert!(!fs::read_to_string(v.h.paths.profiles()).unwrap().contains("launchapp:notepad"), "not remembered");
}

#[test]
fn a_dangerous_phrase_is_refused_outright() {
    let v = start_commands("cmd-blocked", true, COMMAND_MODE, "Press alt f4.");
    hold_to_talk(&v.h);
    wait_for_command(&v.h);
    assert_eq!(commands(&v.h), [("Alt+F4".to_string(), "blocked".to_string())]);
    thread::sleep(Duration::from_millis(300));
    assert!(v.h.actions().is_empty());
}

#[test]
fn a_phrase_that_starts_like_a_command_and_matches_nothing_does_nothing_and_says_so() {
    let v = start_commands("cmd-nomatch", true, COMMAND_MODE, "Open the door.");
    hold_to_talk(&v.h);
    wait_for_command(&v.h);
    assert_eq!(commands(&v.h), [(String::new(), "no_match".to_string())]);
    assert!(v.h.actions().is_empty());
}

#[test]
fn prose_said_with_no_text_field_found_is_typed_not_dropped() {
    // The page's field was never clicked, so the recording is taken as a command; it reads as prose.
    let v = start_commands("cmd-prose", true, COMMAND_MODE, "I want to write an email explaining the project.");
    hold_to_talk(&v.h);
    v.h.wait(|h| h.actions().iter().any(|a| a.starts_with("paste")), "the text to be typed");
    assert_eq!(v.h.actions(), ["paste I want to write an email explaining the project. "]);
    assert!(commands(&v.h).is_empty(), "it was text, not a command");
}

#[test]
fn in_preview_mode_a_command_is_understood_but_not_run() {
    let v = start_commands("cmd-preview", false, COMMAND_MODE, "Mute.");
    hold_to_talk(&v.h);
    wait_for_command(&v.h);
    assert_eq!(commands(&v.h), [("MUTE".to_string(), "preview".to_string())]);
    thread::sleep(Duration::from_millis(300));
    assert!(v.h.actions().is_empty());
}

#[test]
fn a_short_command_said_into_a_text_field_leaves_dictation_but_prose_is_typed() {
    let command = start_commands("cmd-escalate", true, "\"IntentApprovalMode\": 2", "Open VS Code.");
    hold_to_talk(&command.h);
    assert_eq!(command.h.expect_actions(1), ["app code"], "a short command-shaped phrase runs");

    let prose = start_commands("cmd-prose", true, "\"IntentApprovalMode\": 2", "I will see you tomorrow morning.");
    hold_to_talk(&prose.h);
    prose.h.wait(|h| h.actions().iter().any(|a| a.starts_with("paste")), "the text to be typed");
    assert_eq!(prose.h.actions(), ["paste I will see you tomorrow morning. "]);
    assert!(commands(&prose.h).is_empty());
}

#[test]
fn a_phrase_addressed_to_oasis_is_a_command_even_in_a_text_field() {
    let v = start_commands("cmd-address", true, "\"IntentApprovalMode\": 2", "Hey Oasis, mute.");
    hold_to_talk(&v.h);
    assert_eq!(v.h.expect_actions(1), ["shortcut [VolumeMute]"]);
}

#[test]
fn the_user_can_say_one_of_their_own_button_labels() {
    // The default profile's three buttons are labelled A, B and C; the browser profile's "B" opens a link.
    let v = start_commands("cmd-label", true, COMMAND_MODE, "Hey Oasis, volume up.");
    hold_to_talk(&v.h);
    assert_eq!(v.h.expect_actions(1), ["shortcut [VolumeUp]"]);
}

#[test]
fn a_command_does_not_run_when_the_settings_signature_is_bad() {
    let mic = Arc::new(Mutex::new(MicLog::default()));
    let calls = Arc::new(Mutex::new(0));
    let (mic_for, calls_for) = (Arc::clone(&mic), Arc::clone(&calls));
    let parts = VoiceParts {
        pause_media: Box::new(Vec::new),
        resume_media: Box::new(|_| {}),
        make_mic: Box::new(move |_| Box::new(FakeMic(Arc::clone(&mic_for), 1500))),
        model_ready: Box::new(|_| true),
        load_stt: Box::new(move |_| Ok(Box::new(FakeStt(Arc::clone(&calls_for), "Mute.".into())))),
        editable_focus: Box::new(|| false),
        running_apps: Box::new(Vec::new),
        app_installed: Box::new(pretend_installed),
        on_screen: std::sync::Arc::new(Vec::new),
        app_names: Box::new(Vec::new),
    };
    let h = Harness::start_full("cmd-badsig", &command_profiles(COMMAND_MODE), true, false, Some(parts));
    hold_to_talk(&h);
    thread::sleep(Duration::from_millis(1200));
    assert!(h.actions().is_empty(), "nothing runs from a file that failed its signature");
}

// ---- panels --------------------------------------------------------------------------------

/// An IDE profile with a main set, a terminal set and an agent set.
fn ide_profiles() -> String {
    r#"{
  "SchemaVersion": 2, "ShowOverlayLabels": false,
  "Profiles": [
    { "Name": "Default", "ContextMatch": "*", "Kind": "", "MultiConfigEnabled": false,
      "ConfigProfiles": [ { "Name": "Default", "ContextMatch": "*", "Buttons": [
        { "Type": "Shortcut", "Value": "Ctrl+1", "Label": "A", "Steps": "" },
        { "Type": "Shortcut", "Value": "Ctrl+2", "Label": "B", "Steps": "" },
        { "Type": "Shortcut", "Value": "Ctrl+3", "Label": "C", "Steps": "" } ] } ] },
    { "Name": "VS CODE", "ContextMatch": "code", "Kind": "app", "MultiConfigEnabled": true,
      "ConfigProfiles": [
        { "Name": "main", "ContextMatch": "*", "Buttons": [
          { "Type": "Shortcut", "Value": "Alt+1", "Label": "", "Steps": "" },
          { "Type": "Shortcut", "Value": "Alt+2", "Label": "", "Steps": "" },
          { "Type": "Shortcut", "Value": "Alt+3", "Label": "", "Steps": "" } ] },
        { "Name": "terminal", "ContextMatch": "", "PanelId": "terminal", "Buttons": [
          { "Type": "Shortcut", "Value": "Ctrl+Shift+5", "Label": "", "Steps": "" },
          { "Type": "Shortcut", "Value": "Ctrl+Shift+6", "Label": "", "Steps": "" },
          { "Type": "Shortcut", "Value": "Ctrl+Shift+7", "Label": "", "Steps": "" } ] },
        { "Name": "agent", "ContextMatch": "", "PanelId": "agent", "Buttons": [
          { "Type": "Shortcut", "Value": "Ctrl+Alt+8", "Label": "", "Steps": "" },
          { "Type": "Shortcut", "Value": "Ctrl+Alt+9", "Label": "", "Steps": "" },
          { "Type": "Shortcut", "Value": "Ctrl+Alt+0", "Label": "", "Steps": "" } ] } ] }
  ]
}"#
    .to_string()
}

#[test]
fn the_set_of_buttons_follows_which_panel_of_the_ide_has_the_focus() {
    let h = Harness::start_with("panels", &ide_profiles(), true, true);
    h.focus("code", "main.rs - Visual Studio Code", "");

    h.panel("");
    h.pad.tap(0);
    h.wait(|h| !h.actions().is_empty(), "the first action");
    h.panel("terminal");
    h.pad.tap(0);
    h.wait(|h| h.actions().len() >= 2, "the second action");
    h.panel("agent");
    h.pad.tap(0);
    h.wait(|h| h.actions().len() >= 3, "the third action");
    assert_eq!(
        h.actions(),
        ["shortcut [Alt, Char('1')]", "shortcut [Ctrl, Shift, Char('5')]", "shortcut [Ctrl, Alt, Char('8')]"],
        "main, terminal, agent"
    );
    let configs: Vec<String> = h.presses().iter().map(|p| p.config.clone()).collect();
    assert_eq!(configs, ["main", "terminal", "agent"]);
}

#[test]
fn a_panel_the_profile_has_no_set_for_uses_the_main_set() {
    let h = Harness::start_with("panels-unknown", &ide_profiles(), true, true);
    h.focus("code", "x - Visual Studio Code", "");
    h.panel("explorer");
    h.pad.tap(1);
    assert_eq!(h.expect_actions(1), ["shortcut [Alt, Char('2')]"]);
}

#[test]
fn the_panel_does_not_matter_in_another_program() {
    let h = Harness::start_with("panels-other", &ide_profiles(), true, true);
    h.focus("notepad", "Untitled", "");
    h.panel("terminal");
    h.pad.tap(0);
    assert_eq!(h.expect_actions(1), ["shortcut [Ctrl, Char('1')]"], "the default profile has no panels");
}

#[test]
fn a_spoken_timer_is_set_goes_off_once_and_chimes() {
    let v = start_commands("cmd-timer", true, COMMAND_MODE, "Set a timer for 1 second.");
    hold_to_talk(&v.h);
    v.h.wait(
        |h| h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::Note { text } if text == "Timer for 1 second")),
        "the timer to be set",
    );
    v.h.wait(
        |h| h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::Reminder { body, .. } if body == "Your timer is done.")),
        "the timer to go off",
    );
    v.h.wait(|h| h.actions.lock().unwrap().iter().any(|a| a == "system chime:4"), "the sound");
    let fired = v.h.events.lock().unwrap().iter().filter(|e| matches!(e, EngineEvent::Reminder { .. })).count();
    assert_eq!(fired, 1);
    let saved = fs::read_to_string(v.h.paths.profiles().with_file_name("reminders.txt")).unwrap_or_default();
    assert!(saved.is_empty(), "nothing left waiting: {saved:?}");
}

#[test]
fn saying_a_setup_name_opens_everything_in_it() {
    let extra = format!(
        "{COMMAND_MODE} }}, \"QuickLaunch\": [{{ \"Id\": \"p\", \"Name\": \"Project Mode\", \"Items\": [\
         {{ \"Kind\": 0, \"Target\": \"code\" }}, {{ \"Kind\": 1, \"Target\": \"https://example.com\" }}] }}], \"Zzz\": {{ "
    );
    let v = start_commands("cmd-quick-launch", true, &extra, "Open project mode.");
    hold_to_talk(&v.h);
    assert_eq!(v.h.expect_actions(2), ["app code", "url https://example.com"]);
    v.h.wait(
        |h| h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::Note { text } if text == "Project Mode: 2 of 2 ready.")),
        "the summary",
    );
}

#[test]
fn a_low_battery_is_warned_about_once() {
    let h = Harness::start("low-battery", true, true);
    let warnings = |h: &Harness| {
        h.events.lock().unwrap().iter().filter(|e| matches!(e, EngineEvent::Reminder { title, .. } if title.contains("Low battery"))).count()
    };
    h.pad.send("BAT:19\n");
    h.wait(|h| warnings(h) == 1, "the low-battery warning");
    h.pad.send("BAT:18\n");
    h.wait(|h| h.engine.status().battery_percent == Some(18), "the next reading");
    assert_eq!(warnings(&h), 1);
}

#[test]
fn the_end_of_a_video_is_mentioned_once_and_the_end_of_a_short_is_not() {
    let (h, mut socket) = start_with_extension("yt-ended");
    h.focus("chrome", "T - YouTube - Google Chrome", WATCH_URL);
    video_playing(&h, &mut socket);
    let notices = |h: &Harness| {
        h.events.lock().unwrap().iter().filter_map(|e| match e {
            EngineEvent::Notice { title, body } => Some(format!("{title}: {body}")),
            _ => None,
        }).collect::<Vec<_>>()
    };
    socket.send(Message::text(format!(r#"{{"type":"youtube.ended","videoId":"abc","title":"My talk","url":"{WATCH_URL}","surface":"watch"}}"#))).unwrap();
    h.wait(|h| !notices(h).is_empty(), "the notice");
    socket.send(Message::text(r#"{"type":"youtube.ended","videoId":"x","title":"S","url":"https://www.youtube.com/shorts/x","surface":"shorts"}"#)).unwrap();
    thread::sleep(Duration::from_millis(400));
    assert_eq!(notices(&h), ["YouTube ended: My talk"]);
}

#[test]
fn a_command_runs_silently() {
    let quiet = start_commands("cmd-quiet", true, COMMAND_MODE, "Mute.");
    hold_to_talk(&quiet.h);
    assert_eq!(quiet.h.expect_actions(1), ["shortcut [VolumeMute]"]);
    thread::sleep(Duration::from_millis(200));
    assert_eq!(quiet.h.actions(), ["shortcut [VolumeMute]"], "nothing was said");
}

#[test]
fn a_spoken_spotify_command_and_play_request_reach_spotify() {
    let pause = start_commands("cmd-spotify-pause", true, COMMAND_MODE, "Pause Spotify.");
    hold_to_talk(&pause.h);
    assert_eq!(pause.h.expect_actions(1), ["spotify SpotifyControl pause"]);
    pause.h.wait(|h| h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::Note { text } if text == "done pause")), "the answer");

    let play = start_commands("cmd-spotify-play", true, COMMAND_MODE, "Play loser on Spotify.");
    hold_to_talk(&play.h);
    assert_eq!(play.h.expect_actions(1), ["spotify SpotifyPlay loser"]);
}

#[test]
fn a_spotify_failure_is_reported_not_swallowed() {
    let v = start_commands("cmd-spotify-fail", true, COMMAND_MODE, "Play boom on Spotify.");
    hold_to_talk(&v.h);
    v.h.wait(|h| h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::ActionFailed { reason, .. } if reason == "pretend failure")), "the failure");
}

#[test]
fn a_meeting_about_to_start_is_reminded_once_and_the_reminder_can_be_answered() {
    let h = Harness::start("meeting-reminder", true, true);
    let reminders = |h: &Harness| {
        h.events.lock().unwrap().iter().filter(|e| matches!(e, EngineEvent::MeetingReminder { title, can_join: true, .. } if title == "Standup")).count()
    };
    h.wait(|h| reminders(h) == 1, "the meeting reminder");
    thread::sleep(Duration::from_millis(2500));
    assert_eq!(reminders(&h), 1, "only once while it is on screen");
    h.engine.answer_meeting_reminder("snooze").unwrap();
    h.wait(|h| h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::MeetingReminderEnd)), "the reminder to end");
    assert!(h.engine.answer_meeting_reminder("explode").is_err());
}

#[test]
fn saying_join_my_meeting_opens_the_meeting_that_is_starting() {
    let v = start_commands("cmd-join", true, COMMAND_MODE, "Join my meeting.");
    v.h.wait(|h| h.engine.meetings().is_some_and(|m| !m.meetings().is_empty()), "the calendar to load");
    hold_to_talk(&v.h);
    v.h.wait(|h| h.actions().iter().any(|a| a == "open https://meet.google.com/abc"), "the meeting link to open");
    v.h.wait(|h| h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::Note { text } if text == "Joining Standup")), "the note");
}

/// The pretend PC's programs: everything is "installed" except names with "door" or "zzq" in them.
fn pretend_installed(name: &str) -> bool {
    !name.contains("door") && !name.contains("zzq")
}

/// Voice with a text field focused or not, and these programs open: `(process, window title)`.
fn start_gate(name: &str, text_field: bool, open: &[(&str, &str)], says: &str) -> Voiced {
    start_gate_with(name, &voice_profiles(true), text_field, open, says)
}

fn start_gate_with(name: &str, profiles: &str, text_field: bool, open: &[(&str, &str)], says: &str) -> Voiced {
    let mic = Arc::new(Mutex::new(MicLog::default()));
    let stt_calls = Arc::new(Mutex::new(0));
    let loads = Arc::new(Mutex::new(0));
    let (mic_for, calls_for, says) = (Arc::clone(&mic), Arc::clone(&stt_calls), says.to_string());
    use oasis_core::voice::command::RunningApp;
    let running: Vec<RunningApp> = open.iter().map(|(p, t)| RunningApp { process: (*p).into(), title: (*t).into() }).collect();
    let parts = VoiceParts {
        pause_media: Box::new(Vec::new),
        resume_media: Box::new(|_| {}),
        make_mic: Box::new(move |_| Box::new(FakeMic(Arc::clone(&mic_for), 1500))),
        model_ready: Box::new(|_| true),
        load_stt: Box::new(move |_| Ok(Box::new(FakeStt(Arc::clone(&calls_for), says.clone())))),
        editable_focus: Box::new(move || text_field),
        running_apps: Box::new(move || running.clone()),
        app_installed: Box::new(pretend_installed),
        on_screen: std::sync::Arc::new(Vec::new),
        app_names: Box::new(Vec::new),
    };
    let h = Harness::start_full(name, profiles, true, true, Some(parts));
    Voiced { h, mic, stt_calls, loads }
}

/// Talk, then wait until the phrase was typed, run, asked about or reported.
fn say_and_settle(v: &Voiced) {
    hold_to_talk(&v.h);
    v.h.wait(|h| !h.actions().is_empty() || !commands(h).is_empty(), "the phrase to be handled");
    thread::sleep(Duration::from_millis(150));
}

#[test]
fn in_a_text_field_close_whatsapp_closes_it_when_it_is_open() {
    let v = start_gate("gate-close-open", true, &[("WhatsApp", "WhatsApp")], "Close WhatsApp.");
    say_and_settle(&v);
    // Closing a whole app asks first (W8): nothing closes before the answer.
    assert!(v.h.actions().is_empty());
    v.h.wait(|h| h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::Confirm { .. })), "the question");
    press(&v.h, 0);
    assert_eq!(v.h.expect_actions(1), ["close WhatsApp"]);
}

#[test]
fn in_a_text_field_close_whatsapp_is_typed_when_it_is_not_open() {
    let v = start_gate("gate-close-shut", true, &[("chrome", "Inbox - Google Chrome")], "Close WhatsApp.");
    say_and_settle(&v);
    assert_eq!(v.h.actions(), ["paste Close WhatsApp. "]);
}

#[test]
fn in_a_text_field_polite_or_prose_phrases_are_typed() {
    for (n, said) in ["Can you launch Chrome?", "I think you should open Chrome.", "Open the door for me please.", "Open the door."].iter().enumerate() {
        let v = start_gate(&format!("gate-typed-{n}"), true, &[], said);
        say_and_settle(&v);
        assert_eq!(v.h.actions(), [format!("paste {said} ")], "{said}");
        assert!(commands(&v.h).is_empty(), "{said}: nothing ran or was reported: {:?}", commands(&v.h));
    }
}

#[test]
fn in_a_text_field_opening_another_app_or_site_is_a_command() {
    // The owner's report (2026-11-16): cursor in Notepad, "open …" was typed. Opening asks on the pad
    // the first time (the default approval mode); nothing is typed.
    for (n, (said, label)) in [("Open Chrome.", "CHROME"), ("Open Chrome please.", "CHROME"), ("Open chat GPT.", "CHAT GPT"), ("Hey, open Chrome.", "CHROME")].iter().enumerate() {
        let v = start_gate(&format!("gate-open-{n}"), true, &[("Notepad", "Untitled - Notepad")], said);
        say_and_settle(&v);
        assert!(v.h.actions().is_empty(), "{said}: {:?}", v.h.actions());
        // The pill names the app found on this PC ("GOOGLE CHROME"), so the spoken name need only be in it.
        let got = commands(&v.h);
        assert!(got.len() == 1 && got[0].0.contains(label) && got[0].1 == "asking", "{said}: {got:?}");
    }
}

#[test]
fn in_a_text_field_an_editing_phrase_said_alone_presses_its_keys() {
    for (n, (said, pressed)) in [
        ("Copy.", vec!["shortcut [Ctrl, Char('C')]"]),
        ("Ctrl Z.", vec!["shortcut [Ctrl, Char('Z')]"]),
        ("Select line.", vec!["shortcut [Home]", "shortcut [Shift, End]"]),
    ]
    .iter()
    .enumerate()
    {
        let v = start_gate(&format!("gate-edit-{n}"), true, &[("Notepad", "Untitled - Notepad")], said);
        say_and_settle(&v);
        assert_eq!(v.h.expect_actions(pressed.len()), *pressed, "{said}: runs without asking, nothing typed");
    }
    let v = start_gate("gate-edit-sentence", true, &[("Notepad", "Untitled - Notepad")], "Copy the file.");
    say_and_settle(&v);
    assert_eq!(v.h.actions(), ["paste Copy the file. "]);
}

#[test]
fn with_editing_commands_off_a_bare_copy_is_typed() {
    let profiles = voice_profiles(true).replace("\"Enabled\": true,", "\"Enabled\": true, \"EditCommands\": false,");
    let v = start_gate_with("gate-edit-off", &profiles, true, &[("Notepad", "Untitled - Notepad")], "Copy.");
    say_and_settle(&v);
    assert_eq!(v.h.actions(), ["paste Copy. "]);
}

#[test]
fn with_no_text_field_a_polite_command_runs() {
    // Opening an app asks on the pad the first time (the default approval mode).
    let v = start_gate("gate-polite", false, &[], "Can you launch Chrome?");
    say_and_settle(&v);
    let got = commands(&v.h);
    assert!(got.len() == 1 && got[0].0.contains("CHROME") && got[0].1 == "asking", "{got:?}");
    assert!(v.h.actions().is_empty(), "nothing typed, nothing run unasked");
}

#[test]
fn with_no_text_field_an_app_that_is_not_installed_is_reported_not_launched() {
    // Four words or fewer that start like a command: reported, never launched, never typed.
    let v = start_gate("gate-door", false, &[], "Open the door.");
    say_and_settle(&v);
    assert!(v.h.actions().is_empty(), "{:?}", v.h.actions());
    assert_eq!(commands(&v.h), [(String::new(), "no_match".to_string())]);
}

#[test]
fn with_no_text_field_a_long_sentence_that_matched_nothing_is_never_typed_but_logged() {
    // VOICE_INTENT I2: with no text field, a command that matched nothing is never typed (it would
    // land in whatever window is in front). It is reported, kept on the clipboard, and logged.
    let said = "Open via zzq might primary screen and open qwx in my second screen.";
    let v = start_gate("gate-misheard", false, &[], said);
    say_and_settle(&v);
    v.h.wait(|h| commands(h).contains(&(String::new(), "no_match".to_string())), "the miss to be reported");
    thread::sleep(Duration::from_millis(300));
    assert!(v.h.actions().is_empty(), "nothing typed: {:?}", v.h.actions());
    let log = fs::read_to_string(v.h.paths.security_log()).unwrap_or_default();
    assert!(log.contains(said), "{log}");
}

#[test]
fn of_two_apps_on_two_screens_the_one_that_exists_still_opens() {
    let said = "Open Chrome in my primary screen and open Zzq in my secondary screen.";
    let v = start_commands("win-arrange-half", true, COMMAND_MODE, said);
    hold_to_talk(&v.h);
    assert_eq!(v.h.expect_actions(2), ["app chrome", "window WindowCommand to:main|chrome"]);
    let told = v.h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::Note { text } if text.contains("zzq")));
    assert!(told, "the missing app is named");
}

#[test]
fn with_no_text_field_quit_closes_an_open_program() {
    let v = start_gate("gate-quit", false, &[("Spotify", "Spotify Premium")], "Quit Spotify.");
    say_and_settle(&v);
    assert!(v.h.actions().is_empty(), "asks before closing a whole app");
    v.h.wait(|h| h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::Confirm { .. })), "the question");
    press(&v.h, 0);
    assert_eq!(v.h.expect_actions(1), ["close Spotify"]);
}

/// Voice settings with two fragments, and `extra` more fields in the Voice section.
fn fragment_profiles(extra: &str) -> String {
    voice_profiles(true).replace(
        "\"MinUtteranceMs\": 300 }",
        &format!(
            "\"MinUtteranceMs\": 300, \"Replacements\": [ {{ \"Spoken\": \"my linkedin\", \"Canonical\": \"https://linkedin.com/in/example\" }}, {{ \"Spoken\": \"my mail\", \"Canonical\": \"me@example.com\" }} ]{extra} }}"
        ),
    )
}

#[test]
fn a_fragment_said_on_its_own_types_its_text_exactly() {
    for (n, (said, field)) in [("My LinkedIn.", true), ("Insert my LinkedIn.", true), ("insert my linkedin", false)].iter().enumerate() {
        let v = start_gate_with(&format!("frag-alone-{n}"), &fragment_profiles(""), *field, &[], said);
        say_and_settle(&v);
        assert_eq!(v.h.actions(), ["paste https://linkedin.com/in/example "], "{said}, text field {field}");
    }
}

#[test]
fn a_fragment_inside_a_sentence_is_swapped_in() {
    let v = start_gate_with("frag-inside", &fragment_profiles(""), true, &[], "send it to my mail");
    say_and_settle(&v);
    assert_eq!(v.h.actions(), ["paste Send it to me@example.com. "]);
}

#[test]
fn type_with_something_that_is_no_fragment_is_still_the_type_command() {
    let v = start_gate_with("frag-type", &fragment_profiles(""), false, &[], "Type hello.");
    say_and_settle(&v);
    // The router's own TYPE command, which asks on the pad first (the default approval mode).
    assert_eq!(commands(&v.h), [("TYPE".to_string(), "asking".to_string())]);
    assert!(v.h.actions().is_empty());
}

#[test]
fn a_fragment_with_no_text_field_follows_the_typing_by_voice_setting() {
    let off = fragment_profiles(", \"AllowTypeText\": false");
    let v = start_gate_with("frag-blocked", &off, false, &[], "Insert my LinkedIn.");
    say_and_settle(&v);
    assert!(v.h.actions().is_empty(), "{:?}", v.h.actions());
    assert_eq!(commands(&v.h), [("FRAGMENT".to_string(), "blocked".to_string())]);
    // In a text field it is typing like any dictation, so it still types.
    let typed = start_gate_with("frag-field", &off, true, &[], "Insert my LinkedIn.");
    say_and_settle(&typed);
    assert_eq!(typed.h.actions(), ["paste https://linkedin.com/in/example "]);
}

/// A microphone that hears 8.5 s of talk, a pause of `.1` ms and more talk by the first look
/// during the hold, then 0.3 s more by the time the button is let go.
struct TalkingMic(bool, usize);

impl VoiceMic for TalkingMic {
    fn start(&mut self) -> Result<(), String> {
        self.0 = false;
        Ok(())
    }
    fn take(&mut self) -> AudioClip {
        let loud = |ms: usize| vec![0.3f32; ms * 16];
        let samples = if std::mem::replace(&mut self.0, true) { Vec::new() } else { [loud(8500), vec![0.0; self.1 * 16], loud(500)].concat() };
        AudioClip { samples, sample_rate: 16_000 }
    }
    fn stop(&mut self) -> AudioClip {
        AudioClip { samples: vec![0.3; 300 * 16], sample_rate: 16_000 }
    }
}

/// Says the n-th line of its script (or "Piece <n>.") for the n-th clip, and keeps each clip's
/// length in milliseconds.
struct PieceStt(Arc<Mutex<Vec<usize>>>, Vec<String>);

impl SttEngine for PieceStt {
    fn transcribe(&mut self, clip: &AudioClip) -> Result<String, ModelError> {
        let mut heard = self.0.lock().unwrap();
        heard.push(clip.samples.len() / 16);
        Ok(self.1.get(heard.len() - 1).cloned().unwrap_or_else(|| format!("Piece {}.", heard.len())))
    }
}

fn start_talking(name: &str, pause_ms: usize, pieces: &Arc<Mutex<Vec<usize>>>, script: &[&str]) -> Harness {
    let for_stt = Arc::clone(pieces);
    let script: Vec<String> = script.iter().map(|s| (*s).to_string()).collect();
    let parts = VoiceParts {
        pause_media: Box::new(Vec::new),
        resume_media: Box::new(|_| {}),
        make_mic: Box::new(move |_| Box::new(TalkingMic(false, pause_ms))),
        model_ready: Box::new(|_| true),
        load_stt: Box::new(move |_| Ok(Box::new(PieceStt(Arc::clone(&for_stt), script.clone())))),
        editable_focus: Box::new(|| true),
        running_apps: Box::new(Vec::new),
        app_installed: Box::new(pretend_installed),
        on_screen: std::sync::Arc::new(Vec::new),
        app_names: Box::new(Vec::new),
    };
    Harness::start_full(name, &voice_profiles(true), true, true, Some(parts))
}

#[test]
fn a_recording_is_decoded_in_pieces_cut_at_pauses_while_the_button_is_held() {
    let pieces = Arc::new(Mutex::new(Vec::new()));
    let h = start_talking("ptt-pieces", 1300, &pieces, &[]);
    hold_to_talk(&h);
    h.wait(|h| h.actions().iter().any(|a| a.starts_with("paste")), "the text to be typed");
    assert_eq!(h.actions(), ["paste Piece 1. Piece 2. "], "both pieces, in order, typed once");
    // Cut 0.42 s into the pause; the rest of the pause, the talk after it and the tail are piece 2.
    assert_eq!(*pieces.lock().unwrap(), [8500 + 420, 880 + 500 + 300]);
}

#[test]
fn a_long_pause_in_dictation_starts_a_new_paragraph() {
    let pieces = Arc::new(Mutex::new(Vec::new()));
    let h = start_talking("ptt-paragraph", 2500, &pieces, &["The board works. The battery is new.", "We ship on Monday."]);
    hold_to_talk(&h);
    h.wait(|h| h.actions().iter().any(|a| a.starts_with("paste")), "the text to be typed");
    assert_eq!(h.actions(), ["paste The board works. The battery is new.\n\nWe ship on Monday. "]);
}

#[test]
fn a_long_pause_alone_is_not_a_new_paragraph() {
    // Pauses are evidence, not a rule: one sentence before it is no thought to close yet.
    let pieces = Arc::new(Mutex::new(Vec::new()));
    let h = start_talking("ptt-paragraph-breath", 2500, &pieces, &[]);
    hold_to_talk(&h);
    h.wait(|h| h.actions().iter().any(|a| a.starts_with("paste")), "the text to be typed");
    assert_eq!(h.actions(), ["paste Piece 1. Piece 2. "]);
}

#[test]
fn dictation_follows_the_style_of_the_app_in_front() {
    let chat = start_gate("style-chat", true, &[], "sounds good see you there");
    chat.h.focus("WhatsApp", "WhatsApp", "");
    say_and_settle(&chat);
    assert_eq!(chat.h.actions(), ["paste Sounds good see you there "], "no closing full stop in chat");

    let code = start_gate("style-code", true, &[], "let total equals two");
    code.h.focus("cmd", "Command Prompt", "");
    say_and_settle(&code);
    assert_eq!(code.h.actions(), ["paste let total equals two "], "exact in a terminal");
}

#[test]
fn a_correction_after_a_long_pause_still_corrects() {
    let pieces = Arc::new(Mutex::new(Vec::new()));
    let h = start_talking("ptt-pause-fix", 2500, &pieces, &["Hey, I want the mail by Friday.", "No, Saturday."]);
    hold_to_talk(&h);
    h.wait(|h| h.actions().iter().any(|a| a.starts_with("paste")), "the text to be typed");
    assert_eq!(h.actions(), ["paste Hey, I want the mail by Saturday. "]);
}

#[test]
fn a_list_with_a_long_pause_before_an_item_stays_one_list() {
    let pieces = Arc::new(Mutex::new(Vec::new()));
    let script = ["So, I need 3 things to be done. One, bug should be fixed.", "Second, they should be running smoothly. Third, the response time should be very smooth."];
    let h = start_talking("ptt-pause-list", 2500, &pieces, &script);
    hold_to_talk(&h);
    h.wait(|h| h.actions().iter().any(|a| a.starts_with("paste")), "the text to be typed");
    assert_eq!(
        h.actions(),
        ["paste I need 3 things to be done:\n1. Bug should be fixed\n2. They should be running smoothly\n3. The response time should be very smooth "]
    );
}

#[test]
fn switching_the_speech_model_loads_it_in_place_and_voice_keeps_working() {
    let v = start_voiced("ptt-switch", true, true, 1500, "Hello world.");
    let loads = |v: &Voiced| *v.loads.lock().unwrap();
    let wait_for = |v: &Voiced, n: u32| {
        let started = std::time::Instant::now();
        while loads(v) < n && started.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(10));
        }
    };
    wait_for(&v, 1);
    assert_eq!(loads(&v), 1);
    // The model manager points voice at another model (the Light set, say): the engine loads it by
    // itself, with the window and the pad left alone.
    v.h.engine
        .change(|s| {
            if let Some(voice) = s.extra.get_mut("Voice").and_then(|v| v.as_object_mut()) {
                voice.insert("ModelFileName".into(), serde_json::json!("parakeet-tdt-ctc-110m"));
            }
            Ok(())
        })
        .expect("the setting saves");
    wait_for(&v, 2);
    assert_eq!(loads(&v), 2, "the new model was loaded without a restart");
    hold_to_talk(&v.h);
    v.h.wait(|h| h.actions().iter().any(|a| a.starts_with("paste")), "the text to be typed");
    assert_eq!(loads(&v), 2, "the recording used the model that was loaded ahead of it");
}

// ---- spoken window control (docs/APP_WINDOWS.md W8) ----------------------------------------------

#[test]
fn a_spoken_window_command_runs_at_once_in_command_mode() {
    for (n, (said, expected)) in [
        ("Maximize chrome.", "window WindowCommand maximize|chrome"),
        ("Minimize this window.", "window WindowCommand minimize|"),
        ("Move notepad to the other screen.", "window WindowCommand next|notepad"),
        ("Snap chrome to the left.", "window WindowCommand left|chrome"),
        ("Put chrome and notepad side by side.", "window WindowSplit chrome,notepad"),
    ]
    .iter()
    .enumerate()
    {
        let v = start_commands(&format!("win-{n}"), true, COMMAND_MODE, said);
        hold_to_talk(&v.h);
        assert_eq!(v.h.expect_actions(1), [*expected], "{said}");
    }
}

#[test]
fn closing_a_window_by_voice_asks_first_and_button_2_keeps_it_open() {
    // Owner rule (2026-11-13): every close asks.
    let v = start_commands("win-close-yes", true, COMMAND_MODE, "Close this window.");
    hold_to_talk(&v.h);
    v.h.wait(|h| h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::Confirm { .. })), "the question");
    thread::sleep(Duration::from_millis(300));
    assert!(v.h.actions().is_empty(), "nothing closes before the answer");
    press(&v.h, 0);
    assert_eq!(v.h.expect_actions(1), ["window WindowCommand close|"]);

    let v = start_commands("win-close-no", true, COMMAND_MODE, "Close this window.");
    hold_to_talk(&v.h);
    v.h.wait(|h| h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::Confirm { .. })), "the question");
    press(&v.h, 1);
    v.h.wait(|h| h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::ConfirmEnd { approved: false })), "the answer");
    thread::sleep(Duration::from_millis(300));
    assert!(v.h.actions().is_empty());
}

#[test]
fn a_window_command_said_in_a_text_field_is_typed_unless_it_is_a_short_bare_command() {
    // A long request in a text field is prose; the short bare command still acts.
    let v = start_gate("win-typed", true, &[], "Please move the meeting notes to the second screen in my document.");
    say_and_settle(&v);
    assert!(v.h.actions().iter().all(|a| a.starts_with("paste")), "{:?}", v.h.actions());
    let v = start_gate("win-bare", true, &[], "Maximize chrome.");
    say_and_settle(&v);
    assert_eq!(v.h.actions(), ["window WindowCommand maximize|chrome"]);
}

#[test]
fn a_polite_window_command_still_acts_in_a_text_field() {
    for (n, said) in ["Close the window.", "Can you close the window?", "Close the window please.", "Please maximize chrome."].iter().enumerate() {
        let v = start_gate(&format!("win-polite-{n}"), true, &[], said);
        if said.contains("maximize") {
            say_and_settle(&v);
            assert_eq!(v.h.actions(), ["window WindowCommand maximize|chrome"], "{said}");
        } else {
            hold_to_talk(&v.h);
            v.h.wait(|h| h.events.lock().unwrap().iter().any(|e| matches!(e, EngineEvent::Confirm { .. })), "the question");
            press(&v.h, 0);
            assert_eq!(v.h.expect_actions(1), ["window WindowCommand close|"], "{said}");
        }
    }
}

#[test]
fn screens_can_be_named_primary_secondary_first_second_left_or_right() {
    for (n, (said, expected)) in [
        ("Move notepad to my primary screen.", "window WindowCommand to:main|notepad"),
        ("Move notepad to the secondary screen.", "window WindowCommand to:other:1|notepad"),
        ("Send chrome to the first monitor.", "window WindowCommand to:main|chrome"),
        ("Send chrome to the second monitor.", "window WindowCommand to:other:1|chrome"),
        ("Put chrome on the left screen.", "window WindowCommand to:left|chrome"),
    ]
    .iter()
    .enumerate()
    {
        let v = start_commands(&format!("win-screen-{n}"), true, COMMAND_MODE, said);
        hold_to_talk(&v.h);
        assert_eq!(v.h.expect_actions(1), [*expected], "{said}");
    }
}

#[test]
fn two_apps_can_each_be_opened_on_their_own_screen_in_one_sentence() {
    let said = "Open Chrome in my primary screen and open Notepad in my secondary screen.";
    let v = start_commands("win-arrange", true, COMMAND_MODE, said);
    hold_to_talk(&v.h);
    assert_eq!(
        v.h.expect_actions(4),
        ["app chrome", "window WindowCommand to:main|chrome", "app notepad", "window WindowCommand to:other:1|notepad"],
    );
}

#[test]
fn open_the_chrome_window_opens_chrome() {
    let v = start_commands("win-open-the", true, COMMAND_MODE, "Open the Chrome window.");
    hold_to_talk(&v.h);
    assert_eq!(v.h.expect_actions(1), ["app chrome"]);
}
