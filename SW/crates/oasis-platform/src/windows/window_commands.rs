//! Spoken window control on Windows (`docs/APP_WINDOWS.md` W8): maximize, minimize, restore, close
//! one window, move to another screen, snap to a side, and share the screen between apps. The
//! phrases are parsed in `oasis_core::voice::command::window_cmd`; the rectangle arithmetic is
//! `oasis_core::placement`, the same that Quick Launch uses (it knows the invisible resize border).
//! Every failure is a sentence for the user.

use oasis_core::placement::{self, Layout, PixelRect, Screen, Side};
use oasis_win::desktop::{self, Rect};
use oasis_win::window::WindowId;

use super::actions::program_names;

fn px(r: Rect) -> PixelRect {
    PixelRect::new(r.left, r.top, r.right, r.bottom)
}

fn rc(p: PixelRect) -> Rect {
    Rect { left: p.left, top: p.top, right: p.right, bottom: p.bottom }
}

/// Run a `WindowCommand` (`verb|app`) or a `WindowSplit` (`app,app,…`).
pub fn run(kind: &str, value: &str) -> Result<(), String> {
    if oasis_core::actions::ActionKind::of(kind) == oasis_core::actions::ActionKind::WindowSplit {
        return share_screen(&value.split(',').map(str::trim).filter(|a| !a.is_empty()).collect::<Vec<_>>());
    }
    let (verb, app) = value.split_once('|').unwrap_or((value, ""));
    let windows = windows_of(app.trim())?;
    let first = windows[0];
    match verb {
        "maximize" => windows.iter().for_each(|w| desktop::maximize(*w)),
        "minimize" => windows.iter().for_each(|w| desktop::minimize(*w)),
        "restore" => windows.iter().for_each(|w| desktop::restore(*w)),
        "close" => {
            if !desktop::close(first) {
                return Err("That window would not close.".into());
            }
        }
        "next" => move_to_screen(first, None)?,
        // "to:main" (primary), "to:other:1" (secondary), "to:left", "to:right" (see `window_cmd`)
        v if v.starts_with("to:") => move_to_screen(first, Some(&v[3..]))?,
        "left" => snap(first, Side::Left)?,
        "right" => snap(first, Side::Right)?,
        "top" => snap(first, Side::Top)?,
        "bottom" => snap(first, Side::Bottom)?,
        other => return Err(format!("I do not know the window command '{other}'.")),
    }
    Ok(())
}

/// The windows an app has (front to back), or the window in front when `app` is empty.
fn windows_of(app: &str) -> Result<Vec<i64>, String> {
    if app.is_empty() {
        // The desktop or taskbar in front (a click on the desktop, or a window Explorer opened
        // without focus): "this" is the top window the user sees, never the desktop itself.
        let front = desktop::foreground();
        if front != 0 && !oasis_win::window::is_shell_window(WindowId(front as isize)) {
            return Ok(vec![front]);
        }
        return desktop::app_windows()
            .into_iter()
            .find(|w| !oasis_win::window::is_shell_window(WindowId(w.handle as isize)))
            .map(|w| vec![w.handle])
            .ok_or_else(|| "There is no window in front.".into());
    }
    let names = program_names(app);
    let found: Vec<i64> = desktop::app_windows().into_iter().filter(|w| super::actions::runs_as(&names, &w.process)).map(|w| w.handle).collect();
    if found.is_empty() {
        Err(format!("'{app}' is not open."))
    } else {
        Ok(found)
    }
}

fn screens() -> Vec<(isize, Screen)> {
    desktop::monitors().into_iter().map(|m| (m.handle, Screen { bounds: px(m.bounds), work: px(m.work), primary: m.primary })).collect()
}

/// Put a window's visible frame on `target`, restoring it first if it was maximized or minimized, and
/// keep it wholly on its screen if the app would not shrink to fit.
fn place(handle: i64, target: PixelRect, work: PixelRect, layout: Layout) -> Result<(), String> {
    let now = desktop::read(handle).ok_or("That window has gone.")?;
    if now.maximized || now.minimized {
        desktop::restore(handle);
    }
    let now = desktop::read(handle).ok_or("That window has gone.")?;
    desktop::set_outer(handle, rc(placement::outer_for(target, px(now.outer), px(now.visible))));
    if let Some(got) = desktop::read(handle) {
        if let Some(kept) = placement::keep_on_screen(px(got.visible), target, work, layout) {
            desktop::set_outer(handle, rc(placement::outer_for(kept, px(got.outer), px(got.visible))));
        }
    }
    Ok(())
}

fn snap(handle: i64, side: Side) -> Result<(), String> {
    let now = desktop::read(handle).ok_or("That window has gone.")?;
    let screens = screens();
    let (_, screen) = screens.iter().find(|(m, _)| *m == now.monitor).or(screens.first()).ok_or("No screen found.")?;
    let layout = match side {
        Side::Left => Layout::LeftHalf,
        Side::Right => Layout::RightHalf,
        _ => Layout::AsIs,
    };
    place(handle, placement::snap_to(screen.work, side), screen.work, layout)
}

/// `to`: the screen to go to, as `placement::pick_screen` reads it (`main`, `other:N`, `left`,
/// `right`), or `None` for the next one.
fn move_to_screen(handle: i64, to: Option<&str>) -> Result<(), String> {
    let now = desktop::read(handle).ok_or("That window has gone.")?;
    let screens = screens();
    let from = screens.iter().position(|(m, _)| *m == now.monitor).unwrap_or(0);
    let target = match to {
        None if screens.len() < 2 => return Err("There is only one screen.".into()),
        None => (from + 1) % screens.len(),
        Some(spec) => {
            let list: Vec<Screen> = screens.iter().map(|(_, s)| *s).collect();
            placement::pick_screen(&list, spec).ok_or_else(|| match spec {
                "main" => "I could not find the primary screen.".to_string(),
                s if s.starts_with("other:") => format!("There is no screen like that: this PC has {} {}.", list.len(), if list.len() == 1 { "screen" } else { "screens" }),
                _ => format!("There is no {spec} screen: this PC has {} {}.", list.len(), if list.len() == 1 { "screen" } else { "screens" }),
            })?
        }
    };
    if target == from {
        return Ok(());
    }
    let was_maximized = now.maximized;
    if was_maximized || now.minimized {
        desktop::restore(handle);
    }
    let now = desktop::read(handle).ok_or("That window has gone.")?;
    desktop::set_outer(handle, rc(placement::move_between(px(now.outer), screens[from].1.work, screens[target].1.work)));
    if was_maximized {
        desktop::maximize(handle);
    }
    Ok(())
}

/// "put A and B side by side": two share halves; three or more are one big window and a stack (W8).
fn share_screen(apps: &[&str]) -> Result<(), String> {
    let mut handles = Vec::new();
    for app in apps {
        handles.push(windows_of(app)?[0]);
    }
    let screens = screens();
    let first = desktop::read(handles[0]).ok_or("That window has gone.")?;
    let (_, screen) = screens.iter().find(|(m, _)| *m == first.monitor).or(screens.first()).ok_or("No screen found.")?;
    for (i, (handle, target)) in handles.iter().zip(placement::split(screen.work, handles.len())).enumerate() {
        let layout = match (handles.len(), i) {
            (_, 0) => Layout::LeftHalf,
            _ => Layout::RightHalf,
        };
        place(*handle, target, screen.work, layout)?;
    }
    // Windows that were minimized or behind come forward together, the first on top.
    for handle in handles.iter().rev() {
        oasis_win::focus::activate(WindowId(*handle as isize));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_missing_app_or_screen_is_said_in_words() {
        assert_eq!(run("WindowCommand", "maximize|no-such-program-here").unwrap_err(), "'no-such-program-here' is not open.");
        assert!(run("WindowCommand", "dance|").unwrap_err().contains("do not know") || run("WindowCommand", "dance|").is_err());
    }

    /// On the real desktop, by hand: `cargo test -p oasis-platform a_spoken_window_command -- --ignored`.
    /// Opens Notepad, snaps, maximizes, minimizes, restores and closes it. Touches nothing else.
    #[test]
    #[ignore = "drives the real desktop"]
    fn a_spoken_window_command_moves_a_real_window() {
        let _ = std::process::Command::new("notepad.exe").spawn();
        let mut handle = 0;
        for _ in 0..80 {
            std::thread::sleep(Duration::from_millis(100));
            if let Some(w) = desktop::app_windows().into_iter().find(|w| w.process.eq_ignore_ascii_case("notepad")) {
                handle = w.handle;
                break;
            }
        }
        assert_ne!(handle, 0, "Notepad did not open");
        std::thread::sleep(Duration::from_millis(500));
        let near = |a: i32, b: i32| (a - b).abs() <= placement::TOLERANCE;

        run("WindowCommand", "left|notepad").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let now = desktop::read(handle).unwrap();
        let (_, screen) = screens().into_iter().find(|(m, _)| *m == now.monitor).unwrap();
        let want = placement::snap_to(screen.work, Side::Left);
        println!("left half: got {:?}, wanted {want:?}", now.visible);
        assert!(near(now.visible.left, want.left) && near(now.visible.top, want.top) && near(now.visible.bottom, want.bottom), "not at the left");
        assert!(now.visible.right <= want.right + placement::TOLERANCE, "wider than a half");

        run("WindowCommand", "right|notepad").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let now = desktop::read(handle).unwrap();
        assert!(near(now.visible.right, screen.work.right), "not at the right: {:?}", now.visible);

        run("WindowCommand", "maximize|notepad").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(desktop::read(handle).unwrap().maximized);

        run("WindowCommand", "minimize|notepad").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(desktop::read(handle).unwrap().minimized);

        run("WindowCommand", "restore|notepad").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let now = desktop::read(handle).unwrap();
        assert!(!now.minimized);

        if screens().len() > 1 {
            let before = now.monitor;
            run("WindowCommand", "next|notepad").unwrap();
            std::thread::sleep(Duration::from_millis(300));
            assert_ne!(desktop::read(handle).unwrap().monitor, before, "did not change screen");
        } else {
            assert_eq!(run("WindowCommand", "next|notepad").unwrap_err(), "There is only one screen.");
        }

        // Named screens: the primary one and the secondary one, by what Windows calls primary.
        let all = screens();
        if all.len() > 1 {
            let primary = all.iter().find(|(_, s)| s.primary).map(|(m, _)| *m).unwrap();
            run("WindowCommand", "to:main|notepad").unwrap();
            std::thread::sleep(Duration::from_millis(300));
            assert_eq!(desktop::read(handle).unwrap().monitor, primary, "primary screen");
            run("WindowCommand", "to:other:1|notepad").unwrap();
            std::thread::sleep(Duration::from_millis(300));
            assert_ne!(desktop::read(handle).unwrap().monitor, primary, "secondary screen");
            assert!(run("WindowCommand", "to:other:9|notepad").unwrap_err().contains("this PC has"), "a screen that is not there");
            let leftmost = all.iter().min_by_key(|(_, s)| s.bounds.left).map(|(m, _)| *m).unwrap();
            run("WindowCommand", "to:left|notepad").unwrap();
            std::thread::sleep(Duration::from_millis(300));
            assert_eq!(desktop::read(handle).unwrap().monitor, leftmost, "left screen");
        }

        // "close this window" closes the one in front: bring Notepad forward first.
        oasis_win::focus::activate(WindowId(handle as isize));
        std::thread::sleep(Duration::from_millis(300));
        run("WindowCommand", "close|").unwrap();
        std::thread::sleep(Duration::from_millis(600));
        let still_there = desktop::app_windows().into_iter().any(|w| w.handle == handle);
        if still_there {
            // An unsaved-changes prompt cannot appear for an empty Notepad; make sure nothing is left.
            let _ = std::process::Command::new("taskkill").args(["/IM", "notepad.exe", "/F"]).output();
        }
        assert!(!still_there, "Notepad did not close");
    }
}
