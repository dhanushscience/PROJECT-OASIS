//! What a pad button does. Port of the decision-making half of `Services/ActionExecutor.cs`:
//! which action types exist, what is allowed, how a multi-step automation runs and how a
//! failure is described. Every side effect (press keys, open a URL, start a program, sleep)
//! goes through [`ActionBackend`], so the rules run and are tested without touching a desktop.

pub mod app_resolver;
pub mod kind;
pub mod places;
pub mod runner;
pub mod settings_pages;
pub mod sounds;
pub mod url_policy;

use std::time::Duration;

use crate::shortcut::Chord;

pub use kind::ActionKind;
pub use runner::{run_action, run_single, run_steps, ActionOutcome, AutomationResult, StepOutcome, READY_LIMIT};
pub use url_policy::{try_allow, UrlScope};

/// One step of an `Automation` button: `Delay` or any single action.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ActionStep {
    pub kind: String,
    pub value: String,
    /// Where to go once it worked. `Auto` is the step below it, as a plain list runs.
    pub on_ok: Next,
    /// Where to go when it did not work. `Auto` stops and reports the failure.
    pub on_fail: Next,
    /// Where the automation builder drew it; nothing else reads it.
    pub pos: Option<(i32, i32)>,
}

/// Where an automation goes after a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Next {
    /// The plain-list rule: on success the step below, on failure stop and report it.
    #[default]
    Auto,
    /// End here, without a failure.
    Stop,
    /// That step (by its place in the list, from 0).
    Step(usize),
}

impl Next {
    /// As saved: nothing for `Auto`, -1 for `Stop`, the step's number otherwise.
    pub fn code(self) -> Option<i64> {
        match self {
            Next::Auto => None,
            Next::Stop => Some(-1),
            Next::Step(n) => Some(n as i64),
        }
    }

    pub fn from_code(code: Option<i64>) -> Next {
        match code {
            None => Next::Auto,
            Some(n) if n < 0 => Next::Stop,
            Some(n) => Next::Step(n as usize),
        }
    }
}

/// A configured button. `kind` is the profile's `Type` text (`Shortcut`, `LaunchApp`,
/// `OpenUrl`, `Automation`, …), compared exactly as C# did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ButtonAction {
    pub kind: String,
    pub value: String,
    pub steps: Vec<ActionStep>,
}

impl ButtonAction {
    /// The kind, read exactly like the runner does. The text in `kind` is what gets saved.
    pub fn kind(&self) -> ActionKind {
        ActionKind::of(&self.kind)
    }
}

impl ActionStep {
    /// The kind, read exactly like the runner does. The text in `kind` is what gets saved.
    pub fn kind(&self) -> ActionKind {
        ActionKind::of(&self.kind)
    }
}

/// True when the action does not depend on which window is in front and nothing queued after it
/// can depend on it, so it may run beside a slow "open app" instead of waiting behind it. Kept
/// narrow on purpose: anything unknown, and every automation, stays in the ordered lane.
pub fn is_focus_free(action: &ButtonAction) -> bool {
    use crate::shortcut::ChordKey::*;
    if ActionKind::is_spotify_text(&action.kind) {
        return true;
    }
    match action.kind() {
        ActionKind::Reminder | ActionKind::CancelReminders | ActionKind::Chime | ActionKind::PlaySound | ActionKind::Radio | ActionKind::Brightness | ActionKind::Volume => true,
        ActionKind::Shortcut => Chord::parse(&action.value).is_ok_and(|c| {
            !c.keys.is_empty()
                && c.keys.iter().all(|k| matches!(k, VolumeUp | VolumeDown | VolumeMute | PlayPause | NextTrack | PrevTrack | MediaStop))
        }),
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseClick {
    pub x: i32,
    pub y: i32,
    pub button: MouseButton,
}

impl MouseClick {
    /// `"120,340"` or `"120,340,right"`. Anything but `right`/`middle` is a left click.
    pub fn parse(value: &str) -> Option<MouseClick> {
        if value.trim().is_empty() {
            return None;
        }
        let parts: Vec<&str> = value.split(',').collect();
        if parts.len() < 2 {
            return None;
        }
        let x = parts[0].trim().parse().ok()?;
        let y = parts[1].trim().parse().ok()?;
        let button = match parts.get(2).map(|p| p.trim().to_lowercase()).as_deref() {
            Some("right") => MouseButton::Right,
            Some("middle") => MouseButton::Middle,
            _ => MouseButton::Left,
        };
        Some(MouseClick { x, y, button })
    }
}

/// The desktop, as the runner sees it. Each method reports whether it worked.
pub trait ActionBackend {
    fn send_shortcut(&mut self, chord: &Chord) -> bool;
    /// `url` has already passed [`try_allow`].
    fn open_url(&mut self, url: &str) -> bool;
    /// Start `browser` with `url` (already allowed). `false` if the browser is unknown.
    fn launch_browser_url(&mut self, browser: &str, url: &str) -> bool;
    /// Focus the app if it is already running, otherwise start it. `query` is a path, a
    /// command, a `shell:` location or an app name.
    fn launch_app(&mut self, query: &str) -> bool;
    /// Why the last action returned `false`, in a sentence for the user (W6): "it runs as
    /// administrator", "no window appeared". Taken once. `None`: nothing more to say.
    fn take_problem(&mut self) -> Option<String> {
        None
    }
    /// Run a script in the background. Only called when [`ActionBackend::run_script_allowed`].
    fn run_script(&mut self, script: &str);
    /// Run an AutoHotkey script in the background. Only called when
    /// [`ActionBackend::run_script_allowed`]. `false` when AutoHotkey is not installed.
    fn run_autohotkey(&mut self, _script: &str) -> bool {
        false
    }
    /// Ask the program named `process` (as listed in the running programs) to close its windows,
    /// as its own close button would. `false` when it has none open.
    fn close_app(&mut self, _process: &str) -> bool {
        false
    }
    /// A spoken window command (W8, W9): `kind` is `WindowCommand` (`verb|app`), `WindowSplit` (apps
    /// separated by commas) or `WindowLayout` (`save|name`, `restore|name`). `false` when it could not
    /// be done; [`ActionBackend::take_problem`] may say why.
    fn window_action(&mut self, _kind: &str, _value: &str) -> bool {
        false
    }
    /// Open a Quick Launch setup by its id. `false` when it is gone or nothing opened.
    fn quick_launch(&mut self, _id: &str) -> bool {
        false
    }
    /// The window in front, as a number that tells two windows apart; `None` if unknown.
    fn front_window(&mut self) -> Option<isize> {
        None
    }
    /// After a step that opens something: wait, at most `limit`, until it is ready (a new window
    /// in front that takes input), so the next step does not act too early. `false` if nothing came.
    fn wait_until_ready(&mut self, _before: Option<isize>, _limit: Duration) -> bool {
        true
    }
    fn mouse_click(&mut self, click: MouseClick) -> bool;
    fn paste_text(&mut self, text: &str) -> bool;
    fn join_meeting(&mut self) -> bool;
    /// A fixed system command: `lock`, `theme-dark` or `theme-light`. The runner sends nothing else.
    fn system(&mut self, command: &str) -> bool;
    fn pause(&mut self, duration: Duration);
    /// The "allow RunScript actions" setting.
    fn run_script_allowed(&self) -> bool;
    /// Audit trail for blocked actions.
    fn security_log(&mut self, message: &str);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_window_independent_actions_get_the_second_lane() {
        let a = |kind: &str, value: &str| ButtonAction { kind: kind.into(), value: value.into(), steps: vec![] };
        for free in [("Shortcut", "VolumeUp"), ("Shortcut", "MediaPlayPause"), ("Shortcut", "Mute"), ("SpotifyPlay", ""), ("Reminder", "timer|1|"), ("CancelReminders", "all"), ("Chime", "1"), ("Radio", "x"), ("Brightness", "50"), ("Volume", "50")] {
            assert!(is_focus_free(&a(free.0, free.1)), "{free:?}");
        }
        for ordered in [("Shortcut", "Ctrl+V"), ("Shortcut", "Ctrl+VolumeUp"), ("Shortcut", ""), ("LaunchApp", "word"), ("PasteText", "hi"), ("Automation", ""), ("Delay", "5")] {
            assert!(!is_focus_free(&a(ordered.0, ordered.1)), "{ordered:?}");
        }
    }

    #[test]
    fn mouse_clicks_parse_like_the_csharp_executor() {
        let click = |s| MouseClick::parse(s);
        assert_eq!(
            click("120,340"),
            Some(MouseClick { x: 120, y: 340, button: MouseButton::Left })
        );
        assert_eq!(click(" 5 , -7 , RIGHT ").unwrap().button, MouseButton::Right);
        assert_eq!(click("1,2,middle").unwrap().button, MouseButton::Middle);
        assert_eq!(click("1,2,banana").unwrap().button, MouseButton::Left);
        for bad in ["", "  ", "5", "a,b", "1,", ",2", "1;2"] {
            assert_eq!(click(bad), None, "{bad:?}");
        }
    }
}
