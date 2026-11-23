//! The HUD: one small transparent window that shows what the voice is doing (the listening pill,
//! with a tick and a cross while the mic is locked on) and the "Close this app?" question, all at the
//! bottom middle of the screen. It never
//! takes focus and lets clicks through, except on the tick and cross, which press button 1 or 2. The page is `ui/hud.html` and draws itself from the same
//! `engine-event` broadcast the overlay hears; Rust only places, sizes, shows and hides the window.
//!
//! It is hidden whenever nothing is up (a hidden webview costs less), and it appears on the screen the
//! mouse pointer is on.

use std::sync::atomic::{AtomicU8, Ordering};

use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, Runtime, WebviewUrl, WebviewWindowBuilder,
};

pub const LABEL: &str = "hud";
/// Everything sits about 24 px above the work area's bottom edge (the taskbar is outside it): the page
/// keeps 18 px of shadow room under the shape, and the window sits 6 px above the edge.
const PILL_LIFT: f64 = 6.0;
/// What is on screen. Kept so the window is only moved when this changes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Hidden = 0,
    Pill = 1,
    Locked = 2,
    Ready = 3,
    Close = 4,
    Learn = 5,
    Mic = 6,
    /// Why a spoken command did not run; hides itself after [`NOTE_FOR`].
    Note = 7,
}

/// How long a "did not run" note stays up.
const NOTE_FOR: std::time::Duration = std::time::Duration::from_millis(4000);
/// Counts notes, so an older note's timer does not hide a newer one.
static NOTE_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static MODE: AtomicU8 = AtomicU8::new(Mode::Hidden as u8);

/// How the pop-up is drawn: lying across (Bottom) or standing up (Left/Right, the "vertical pop-up"
/// chosen in Getting Started or on the Overlay screen). Its place is always the screen's middle.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Bottom = 0,
    Left = 1,
    Right = 2,
}
static SIDE: AtomicU8 = AtomicU8::new(Side::Bottom as u8);
/// Follow the settings. Called whenever the overlay is laid out again (load and every settings change).
pub fn set_side(side: Side) {
    SIDE.store(side as u8, Ordering::Relaxed);
}

impl Mode {
    /// Window size in logical units: the shape plus room for its shadow. At a side the pill stands up;
    /// the boxes with words in them keep their shape.
    fn size(self, side: Side) -> (f64, f64) {
        match (self, side) {
            (Mode::Pill, Side::Left | Side::Right) => return (64.0, 130.0),
            (Mode::Locked, Side::Left | Side::Right) => return (64.0, 200.0),
            _ => {}
        }
        match self {
            Mode::Pill => (130.0, 64.0),
            Mode::Locked => (200.0, 64.0),
            Mode::Ready => (260.0, 64.0),
            Mode::Close | Mode::Hidden => (480.0, 90.0),
            Mode::Learn => (340.0, 130.0),
            Mode::Mic | Mode::Note => (480.0, 90.0),
        }
    }
}

/// Create the window, hidden.
pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let (w, h) = Mode::Pill.size(Side::Bottom);
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("hud.html".into()))
        .title("OASIS HUD")
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
    window.set_ignore_cursor_events(true)
}

/// A word fixed by hand is about to join the dictionary: show the offer (the page counts down and
/// answers with `learn_answer`). A voice or close question that follows replaces it.
pub fn offer_learn<R: Runtime>(app: &AppHandle<R>, fixes: Vec<(String, String)>) {
    use tauri::Emitter;
    MODE.store(Mode::Learn as u8, Ordering::Relaxed);
    let _ = apply(app, Mode::Learn);
    let _ = app.emit_to(LABEL, "learn-offer", fixes);
}

/// The offer was answered: hide the window, unless something else has taken it over meanwhile.
pub fn end_learn<R: Runtime>(app: &AppHandle<R>) {
    if MODE
        .compare_exchange(Mode::Learn as u8, Mode::Hidden as u8, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        let _ = apply(app, Mode::Hidden);
    }
}

/// React to an engine event: show, move or hide the window. The page reacts to the same event itself.
pub fn on_engine_event<R: Runtime>(app: &AppHandle<R>, event: &oasis_engine::EngineEvent) {
    use oasis_engine::EngineEvent::{Confirm, ConfirmEnd, MicOffer, Voice, VoiceCommand};
    let wanted = match event {
        Voice { phase } => {
            // The question stays up over the pill; voice events do not close it.
            if MODE.load(Ordering::Relaxed) == Mode::Close as u8 {
                return;
            }
            match phase.as_str() {
                "locked" => Mode::Locked,
                "listening" | "dictating" | "thinking" => {
                    // A locked recording keeps its tick and cross while it listens.
                    if phase != "thinking" && MODE.load(Ordering::Relaxed) == Mode::Locked as u8 {
                        Mode::Locked
                    } else {
                        Mode::Pill
                    }
                }
                p if p.starts_with("getting_ready:") => Mode::Ready,
                // an idle voice does not take the offer down; it ends on its own answer
                // nor the note saying why a command did not run: it goes after NOTE_FOR
                _ if matches!(MODE.load(Ordering::Relaxed), m if m == Mode::Learn as u8 || m == Mode::Mic as u8 || m == Mode::Note as u8) => return,
                _ => Mode::Hidden,
            }
        }
        Confirm { .. } => Mode::Close,
        MicOffer { .. } => Mode::Mic,
        // "Nothing is selected", "no app called …": said on screen, not only in the log. A question
        // or offer already up keeps the window.
        VoiceCommand { outcome, detail, .. } if matches!(outcome.as_str(), "no_match" | "blocked") && !detail.is_empty() => {
            if matches!(MODE.load(Ordering::Relaxed), m if m == Mode::Close as u8 || m == Mode::Mic as u8 || m == Mode::Learn as u8) {
                return;
            }
            let app = app.clone();
            let this_note = NOTE_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            std::thread::spawn(move || {
                std::thread::sleep(NOTE_FOR);
                if NOTE_COUNT.load(Ordering::Relaxed) == this_note && MODE.compare_exchange(Mode::Note as u8, Mode::Hidden as u8, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
                    let _ = apply(&app, Mode::Hidden);
                }
            });
            Mode::Note
        }
        ConfirmEnd { .. } if matches!(MODE.load(Ordering::Relaxed), m if m == Mode::Close as u8 || m == Mode::Mic as u8) => Mode::Hidden,
        _ => return,
    };
    // Placed again on every event, not only on a change: the pointer may have moved to another screen.
    MODE.store(wanted as u8, Ordering::Relaxed);
    let _ = apply(app, wanted);
}

fn apply<R: Runtime>(app: &AppHandle<R>, mode: Mode) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(LABEL) else {
        return Ok(());
    };
    // What was last put on screen: an event that changes nothing (every "listening" tick) must not
    // touch the window again, each such call restyles it and can flash the system frame.
    static SHOWN: std::sync::Mutex<Option<(u8, i32, i32, u32, u32)>> = std::sync::Mutex::new(None);
    if mode == Mode::Hidden {
        *SHOWN.lock().unwrap_or_else(|e| e.into_inner()) = None;
        return window.hide();
    }
    // Only the buttons take clicks (tick and cross, the question's Run / Cancel, the offers); everything
    // else lets them through. The window still never takes focus.
    let clicks = matches!(mode, Mode::Locked | Mode::Learn | Mode::Mic | Mode::Close);
    let mut shown = SHOWN.lock().unwrap_or_else(|e| e.into_inner());
    if shown.is_none_or(|(m, ..)| (m == Mode::Locked as u8 || m == Mode::Learn as u8 || m == Mode::Mic as u8 || m == Mode::Close as u8) != clicks) {
        window.set_ignore_cursor_events(!clicks)?;
    }
    // shortcut: the pop-up always lies across the bottom; the stored `SIDE` (vertical layout) is ignored, read `SIDE` here to stand it up again.
    let side = Side::Bottom;
    let (w, h) = mode.size(side);
    {
        use tauri::Emitter;
        let _ = app.emit_to(LABEL, "hud-side", ["bottom", "left", "right"][side as usize]);
    }
    #[cfg(windows)]
    {
        let monitors = oasis_win::desktop::monitors();
        let at = oasis_win::desktop::cursor_monitor();
        let screen = monitors
            .iter()
            .find(|m| Some(m.handle) == at)
            .or_else(|| monitors.iter().find(|m| m.primary));
        if let Some(screen) = screen {
            // The work area is in pixels; the window is sized in logical units, so find this screen's scale.
            let scale = app
                .available_monitors()?
                .iter()
                .find(|m| (m.position().x, m.position().y) == (screen.bounds.left, screen.bounds.top))
                .map_or(1.0, |m| m.scale_factor());
            let (pw, ph) = ((w * scale).round() as i32, (h * scale).round() as i32);
            let work = screen.work;
            // Every voice prompt (pill, close question, offer) sits at the bottom middle, lying across,
            // whatever side or layout the overlay has.
            let (x, y) = (
                work.left + (work.right - work.left - pw) / 2,
                work.bottom - ph - (PILL_LIFT * scale).round() as i32,
            );
            // Position, size, position: a window that crosses to a screen with another scale is resized by Windows.
            let placed = (mode as u8, x, y, pw as u32, ph as u32);
            if *shown == Some(placed) {
                return Ok(());
            }
            window.set_position(PhysicalPosition::new(x, y))?;
            window.set_size(PhysicalSize::new(pw as u32, ph as u32))?;
            window.set_position(PhysicalPosition::new(x, y))?;
            *shown = Some(placed);
        }
    }
    window.show()
}
